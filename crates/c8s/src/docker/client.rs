use alloc::collections::BTreeSet;
use core::time::Duration;
use std::process::Stdio;

use bollard::{
    Docker,
    models::{
        ContainerInspectResponse, ContainerStatsResponse, ImageInspect,
        NetworkInspect, Volume,
    },
    query_parameters::{
        InspectContainerOptions, InspectNetworkOptions, ListContainersOptions,
        ListImagesOptions, ListNetworksOptions, ListVolumesOptions,
        LogsOptions, RemoveContainerOptions, RemoveImageOptions,
        RemoveVolumeOptions, RestartContainerOptions, StatsOptions,
        StopContainerOptions, TopOptionsBuilder,
    },
};
use color_eyre::{Result, eyre::eyre};
use futures_util::{StreamExt, future::join_all};
use tokio::{
    io::{AsyncBufReadExt, BufReader},
    process::Command,
    sync::mpsc::UnboundedSender,
    time::timeout,
};

use super::{
    ContainerRow, ImageRow, NetworkRow, ProcessRow, SCAN_SCRIPT, VolumeRow,
};

/// Snapshot of daemon-level health/info — c8s's single-daemon analog of a
/// fleet-wide health dashboard (there's only ever one daemon to summarize).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DaemonHealth {
    pub server_version: String,
    pub api_version: String,
    pub os: String,
    pub arch: String,
    pub containers_running: i64,
    pub containers_paused: i64,
    pub containers_stopped: i64,
    pub images: i64,
}

/// Upper bound for the daemon health probe.
const HEALTH_TIMEOUT: Duration = Duration::from_secs(5);

/// Upper bound for one container stats sample.
const STATS_TIMEOUT: Duration = Duration::from_secs(3);

impl DaemonHealth {
    /// Lines shown in the health panel.
    #[must_use]
    pub fn lines(&self) -> Vec<String> {
        vec![
            format!("Docker version: {}", self.server_version),
            format!("API version: {}", self.api_version),
            format!("OS/Arch: {}/{}", self.os, self.arch),
            String::new(),
            format!("Containers running: {}", self.containers_running),
            format!("Containers paused: {}", self.containers_paused),
            format!("Containers stopped: {}", self.containers_stopped),
            format!("Images: {}", self.images),
        ]
    }
}

/// Thin wrapper around a bollard `Docker` handle, scoped to what c8s needs.
#[derive(Clone)]
pub struct DockerClient {
    docker: Docker,
}

impl DockerClient {
    /// Connect to the local Docker daemon over the default socket.
    pub fn connect() -> Result<Self> {
        let docker = Docker::connect_with_local_defaults()?;
        Ok(Self { docker })
    }

    /// A client pointed at a closed loopback port: every call fails fast.
    /// Lets key-handler tests reach the docker layer without a daemon, on
    /// every platform (unix sockets don't exist on Windows).
    #[cfg(test)]
    pub fn unreachable() -> Self {
        let docker = Docker::connect_with_http(
            "http://127.0.0.1:1",
            1,
            bollard::API_DEFAULT_VERSION,
        )
        .expect("lazy connect");
        Self { docker }
    }

    /// Verify the daemon is actually reachable (connect succeeds lazily otherwise).
    pub async fn ping(&self) -> Result<()> {
        self.docker.ping().await?;
        Ok(())
    }

    /// Cheap daemon-level summary (version + container counts by state), for
    /// c8s's health panel. Two lightweight API calls issued concurrently, with
    /// a hard timeout so a wedged daemon can't freeze the UI (the caller
    /// awaits this on the event loop).
    pub async fn daemon_health(&self) -> Result<DaemonHealth> {
        let (version, info) = timeout(HEALTH_TIMEOUT, async {
            tokio::try_join!(self.docker.version(), self.docker.info())
        })
        .await
        .map_err(|_| {
            eyre!("timed out after {}s", HEALTH_TIMEOUT.as_secs())
        })??;
        Ok(DaemonHealth {
            server_version: version.version.unwrap_or_default(),
            api_version: version.api_version.unwrap_or_default(),
            os: version.os.unwrap_or_default(),
            arch: version.arch.unwrap_or_default(),
            containers_running: info.containers_running.unwrap_or_default(),
            containers_paused: info.containers_paused.unwrap_or_default(),
            containers_stopped: info.containers_stopped.unwrap_or_default(),
            images: info.images.unwrap_or_default(),
        })
    }

    pub async fn list_containers(&self) -> Result<Vec<ContainerRow>> {
        let options = ListContainersOptions {
            all: true,
            ..Default::default()
        };
        let summaries = self.docker.list_containers(Some(options)).await?;
        let mut rows: Vec<ContainerRow> =
            summaries.iter().map(ContainerRow::from_summary).collect();
        // Stats only exist for running containers. Sampled concurrently, each
        // with a timeout; failures just leave the cells blank.
        join_all(rows.iter_mut().filter(|r| r.status == "running").map(
            |row| async move {
                if let Some(stats) = self.container_stats(&row.id).await {
                    row.apply_stats(&stats);
                }
            },
        ))
        .await;
        Ok(rows)
    }

    /// One non-streaming stats sample (the daemon fills `precpu_stats`).
    async fn container_stats(
        &self,
        id: &str,
    ) -> Option<ContainerStatsResponse> {
        let options = StatsOptions {
            stream: false,
            one_shot: false,
        };
        timeout(STATS_TIMEOUT, self.docker.stats(id, Some(options)).next())
            .await
            .ok()?
            .and_then(Result::ok)
    }

    /// Processes of a container with their output targets, found by running
    /// [`SCAN_SCRIPT`] via the `docker` CLI (same as the exec-shell key).
    /// Falls back to `docker top` when the container has no usable `sh`.
    pub async fn processes(&self, id: &str) -> Result<Vec<ProcessRow>> {
        let Some(mut rows) = Self::scan(id, None).await else {
            return self.top(id).await;
        };
        // The kernel hides another user's `/proc/<pid>/fd` (no ptrace cap),
        // e.g. postgres (uid 70) under the default root exec. Rescan as each
        // owner of a process whose fds came back empty.
        let owners: BTreeSet<String> = rows
            .iter()
            .filter(|r| r.stdout.is_empty() && !r.user.is_empty())
            .map(|r| r.user.clone())
            .collect();
        let scans =
            join_all(owners.iter().map(|uid| Self::scan(id, Some(uid)))).await;
        for seen in scans.into_iter().flatten() {
            for s in seen.into_iter().filter(|s| !s.stdout.is_empty()) {
                if let Some(r) = rows.iter_mut().find(|r| r.pid == s.pid) {
                    r.stdout = s.stdout;
                    r.stderr = s.stderr;
                    r.files = s.files;
                }
            }
        }
        Ok(rows)
    }

    /// Run [`SCAN_SCRIPT`] in the container, optionally as `user`.
    async fn scan(id: &str, user: Option<&str>) -> Option<Vec<ProcessRow>> {
        let mut cmd = Command::new("docker");
        cmd.arg("exec");
        if let Some(user) = user {
            cmd.args(["-u", user]);
        }
        let out = cmd
            .args([id, "sh", "-c", SCAN_SCRIPT])
            .output()
            .await
            .ok()?;
        let rows = ProcessRow::from_scan(&String::from_utf8_lossy(&out.stdout));
        (out.status.success() && !rows.is_empty()).then_some(rows)
    }

    /// Check a file inside the container is readable; the error is the
    /// tool's own message (e.g. `head: can't open '/x': No such file`).
    pub async fn probe_file(&self, id: &str, path: &str) -> Result<(), String> {
        let out = Command::new("docker")
            .args(["exec", id, "head", "-c", "0", path])
            .output()
            .await
            .map_err(|e| format!("docker exec failed: {e}"))?;
        if out.status.success() {
            return Ok(());
        }
        let msg = String::from_utf8_lossy(&out.stderr);
        let msg = msg.trim();
        Err(if msg.is_empty() {
            format!("{path} unreadable")
        } else {
            msg.to_string()
        })
    }

    /// Stream `tail -F` of a file inside the container into `tx`.
    // shortcut: aborting this drops the local `docker exec`; the remote
    // `tail` exits on its next write (SIGPIPE), not immediately.
    pub async fn tail_file(
        &self,
        id: &str,
        path: &str,
        tx: UnboundedSender<String>,
    ) {
        let child = Command::new("docker")
            .args(["exec", id, "tail", "-n", "200", "-F", path])
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn();
        let Ok(mut child) = child else {
            let _ = tx.send("[failed to run docker exec]".to_string());
            return;
        };
        let Some(stdout) = child.stdout.take() else {
            return;
        };
        let mut lines = BufReader::new(stdout).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            if tx.send(line).is_err() {
                return;
            }
        }
    }

    /// Processes running in a container (`docker top`, full command lines).
    pub async fn top(&self, id: &str) -> Result<Vec<ProcessRow>> {
        let options = TopOptionsBuilder::default().ps_args("aux").build();
        let top = self.docker.top_processes(id, Some(options)).await?;
        Ok(ProcessRow::from_top(
            &top.titles.unwrap_or_default(),
            &top.processes.unwrap_or_default(),
        ))
    }

    pub async fn start(&self, id: &str) -> Result<()> {
        self.docker.start_container(id, None).await?;
        Ok(())
    }

    pub async fn stop(&self, id: &str) -> Result<()> {
        self.docker
            .stop_container(id, None::<StopContainerOptions>)
            .await?;
        Ok(())
    }

    pub async fn restart(&self, id: &str) -> Result<()> {
        self.docker
            .restart_container(id, None::<RestartContainerOptions>)
            .await?;
        Ok(())
    }

    pub async fn remove(&self, id: &str) -> Result<()> {
        self.docker
            .remove_container(
                id,
                Some(RemoveContainerOptions {
                    force: true,
                    ..Default::default()
                }),
            )
            .await?;
        Ok(())
    }

    pub async fn list_images(&self) -> Result<Vec<ImageRow>> {
        let options = ListImagesOptions {
            all: false,
            ..Default::default()
        };
        let summaries = self.docker.list_images(Some(options)).await?;
        Ok(summaries.iter().map(ImageRow::from_summary).collect())
    }

    pub async fn remove_image(&self, id: &str) -> Result<()> {
        self.docker
            .remove_image(
                id,
                Some(RemoveImageOptions {
                    force: true,
                    ..Default::default()
                }),
                None,
            )
            .await?;
        Ok(())
    }

    pub async fn list_volumes(&self) -> Result<Vec<VolumeRow>> {
        let response =
            self.docker.list_volumes(None::<ListVolumesOptions>).await?;
        Ok(response
            .volumes
            .unwrap_or_default()
            .iter()
            .map(VolumeRow::from_volume)
            .collect())
    }

    pub async fn remove_volume(&self, name: &str) -> Result<()> {
        self.docker
            .remove_volume(name, Some(RemoveVolumeOptions { force: true }))
            .await?;
        Ok(())
    }

    pub async fn list_networks(&self) -> Result<Vec<NetworkRow>> {
        let networks = self
            .docker
            .list_networks(None::<ListNetworksOptions>)
            .await?;
        Ok(networks.iter().map(NetworkRow::from_network).collect())
    }

    pub async fn remove_network(&self, name: &str) -> Result<()> {
        self.docker.remove_network(name).await?;
        Ok(())
    }

    /// Fetch full inspect details for `id` (id, image, status, ports, mounts, env, created time).
    pub async fn inspect(&self, id: &str) -> Result<ContainerInspectResponse> {
        Ok(self
            .docker
            .inspect_container(id, None::<InspectContainerOptions>)
            .await?)
    }

    pub async fn inspect_image(&self, id: &str) -> Result<ImageInspect> {
        Ok(self.docker.inspect_image(id).await?)
    }

    pub async fn inspect_volume(&self, name: &str) -> Result<Volume> {
        Ok(self.docker.inspect_volume(name).await?)
    }

    pub async fn inspect_network(&self, id: &str) -> Result<NetworkInspect> {
        Ok(self
            .docker
            .inspect_network(id, None::<InspectNetworkOptions>)
            .await?)
    }

    /// Stream log lines for `id` into `tx` until the stream ends or the
    /// receiver is dropped (e.g. the user left the log view).
    pub async fn tail_logs(&self, id: &str, tx: UnboundedSender<String>) {
        let options = LogsOptions {
            follow: true,
            stdout: true,
            stderr: true,
            tail: "200".to_string(),
            ..Default::default()
        };
        let mut stream = self.docker.logs(id, Some(options));
        while let Some(chunk) = stream.next().await {
            let line = match chunk {
                Ok(output) => output.to_string(),
                Err(e) => format!("[log stream error: {e}]"),
            };
            for line in line.lines() {
                if tx.send(line.to_string()).is_err() {
                    return;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn health_lines_format_every_field() {
        let h = DaemonHealth {
            server_version: "28.0.1".into(),
            api_version: "1.48".into(),
            os: "linux".into(),
            arch: "arm64".into(),
            containers_running: 3,
            containers_paused: 1,
            containers_stopped: 7,
            images: 12,
        };
        assert_eq!(
            h.lines(),
            vec![
                "Docker version: 28.0.1",
                "API version: 1.48",
                "OS/Arch: linux/arm64",
                "",
                "Containers running: 3",
                "Containers paused: 1",
                "Containers stopped: 7",
                "Images: 12",
            ]
        );
    }

    #[tokio::test]
    async fn daemon_health_fails_fast_when_unreachable() {
        let res = DockerClient::unreachable().daemon_health().await;
        assert!(res.is_err());
    }
}
