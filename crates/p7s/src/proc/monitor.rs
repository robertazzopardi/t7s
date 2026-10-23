//! Process data sources: the real system (`sysinfo`) and canned demo data.

use core::fmt::Write as _;
use std::{
    sync::Mutex,
    time::{SystemTime, UNIX_EPOCH},
};

use sysinfo::{
    Pid, Process, ProcessRefreshKind, ProcessStatus, ProcessesToUpdate, Signal,
    System, UpdateKind, Users,
};

use super::{
    ProcessRow, SystemInfo, format_cpu_time, format_size,
    signals::{SignalReport, precheck},
};

/// A point-in-time view of the system.
#[derive(Debug, Clone, Default)]
pub struct Snapshot {
    pub rows: Vec<ProcessRow>,
    pub info: SystemInfo,
}

/// Where process data comes from: the real system, or canned demo data.
///
/// Object-safe so the app can hold either source and tests can inject a fake.
pub trait ProcessSource: Send + Sync {
    /// One unfiltered sample of every process plus system totals.
    fn snapshot(&self) -> Snapshot;
    /// Full details for one process, as k9s-style describe text.
    fn detail(&self, pid: i32) -> Option<String>;
    /// Send `sig` to `pid`, after confirming it is still the same process that
    /// was selected (guards against PID reuse).
    fn signal(
        &self,
        pid: i32,
        expected_start_time: u64,
        sig: Signal,
    ) -> SignalReport;
    /// p7s's own PID, so it never signals itself.
    fn self_pid(&self) -> i32;
}

/// Mutable state behind the lock: one persistent `System` (so CPU deltas are
/// valid) plus the user table for name resolution.
struct Monitor {
    system: System,
    users: Users,
    self_pid: i32,
}

/// Real process source backed by `sysinfo`.
pub struct SystemMonitor {
    inner: Mutex<Monitor>,
}

impl SystemMonitor {
    #[must_use]
    pub fn new() -> Self {
        let mut system = System::new();
        // Prime memory + CPU so the first snapshot is not all zeros.
        system.refresh_memory();
        system.refresh_cpu_usage();
        let self_pid = sysinfo::get_current_pid().map_or(-1, pid_to_i32);
        Self {
            inner: Mutex::new(Monitor {
                system,
                users: Users::new_with_refreshed_list(),
                self_pid,
            }),
        }
    }

    /// Refresh only `pid` with full detail and hand it to `f`.
    #[allow(clippy::significant_drop_tightening)]
    fn with_process<R>(
        &self,
        pid: i32,
        kind: ProcessRefreshKind,
        f: impl FnOnce(&Monitor, &Process) -> R,
    ) -> Option<R> {
        let mut m = self.inner.lock().ok()?;
        let upid = u32::try_from(pid).ok()?;
        let spid = Pid::from_u32(upid);
        let updated = m.system.refresh_processes_specifics(
            ProcessesToUpdate::Some(&[spid]),
            false,
            kind,
        );
        if updated == 0 {
            return None;
        }
        let process = m.system.process(spid)?;
        Some(f(&m, process))
    }
}

impl Default for SystemMonitor {
    fn default() -> Self {
        Self::new()
    }
}

impl ProcessSource for SystemMonitor {
    fn snapshot(&self) -> Snapshot {
        let Ok(mut m) = self.inner.lock() else {
            return Snapshot::default();
        };
        // Least work that still fills every list column, and no per-task
        // refresh (the expensive part on Linux).
        m.system.refresh_processes_specifics(
            ProcessesToUpdate::All,
            true,
            ProcessRefreshKind::nothing()
                .with_cpu()
                .with_memory()
                .with_user(UpdateKind::OnlyIfNotSet)
                .with_cmd(UpdateKind::OnlyIfNotSet)
                .without_tasks(),
        );
        m.system.refresh_memory();
        m.system.refresh_cpu_usage();

        let total_mem = m.system.total_memory();
        let rows = m
            .system
            .processes()
            .values()
            .map(|p| process_row(&m, p, total_mem))
            .collect();
        Snapshot {
            rows,
            info: info_from(&m.system),
        }
    }

    fn detail(&self, pid: i32) -> Option<String> {
        self.with_process(
            pid,
            ProcessRefreshKind::everything().without_tasks(),
            describe_text,
        )
    }

    fn signal(
        &self,
        pid: i32,
        expected_start_time: u64,
        sig: Signal,
    ) -> SignalReport {
        let self_pid = self.self_pid();
        // Refresh just this PID so the start time is current: if the original
        // process exited and its PID was reused, the start time will differ.
        let found =
            self.with_process(pid, ProcessRefreshKind::nothing(), |_m, p| {
                p.start_time()
            });
        let report = precheck(pid, self_pid, found, expected_start_time);
        if report != SignalReport::Ready {
            return report;
        }
        let Some(report) =
            self.with_process(pid, ProcessRefreshKind::nothing(), |_m, p| {
                match p.kill_with(sig) {
                    None => SignalReport::Unsupported,
                    Some(true) => SignalReport::Sent,
                    Some(false) => SignalReport::Denied,
                }
            })
        else {
            return SignalReport::Gone;
        };
        report
    }

    fn self_pid(&self) -> i32 {
        self.inner.lock().map_or(-1, |m| m.self_pid)
    }
}

fn pid_to_i32(pid: Pid) -> i32 {
    i32::try_from(pid.as_u32()).unwrap_or(i32::MAX)
}

fn user_name(m: &Monitor, p: &Process) -> String {
    p.user_id()
        .and_then(|uid| m.users.get_user_by_id(uid))
        .map_or_else(String::new, |u| u.name().to_string())
}

fn process_row(m: &Monitor, p: &Process, total_mem: u64) -> ProcessRow {
    let rss = p.memory();
    let mem = percent_of(rss, total_mem);
    ProcessRow {
        pid: pid_to_i32(p.pid()),
        ppid: p.parent().map(pid_to_i32),
        user: user_name(m, p),
        cpu: p.cpu_usage(),
        mem,
        rss,
        vsz: p.virtual_memory(),
        state: state_char(p.status()),
        cpu_time_ms: p.accumulated_cpu_time(),
        name: p.name().to_string_lossy().into_owned(),
        cmdline: cmdline(p),
        start_time: p.start_time(),
    }
}

#[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)]
fn percent_of(part: u64, whole: u64) -> f32 {
    if whole == 0 {
        0.0
    } else {
        (part as f64 / whole as f64 * 100.0) as f32
    }
}

fn cmdline(p: &Process) -> String {
    p.cmd()
        .iter()
        .map(|a| a.to_string_lossy())
        .collect::<Vec<_>>()
        .join(" ")
}

const fn state_char(status: ProcessStatus) -> char {
    match status {
        ProcessStatus::Run => 'R',
        ProcessStatus::Sleep => 'S',
        ProcessStatus::Idle => 'I',
        ProcessStatus::Zombie => 'Z',
        ProcessStatus::Stop | ProcessStatus::Suspended => 'T',
        ProcessStatus::Tracing => 't',
        ProcessStatus::Dead => 'X',
        ProcessStatus::Wakekill => 'K',
        ProcessStatus::Waking => 'W',
        ProcessStatus::Parked => 'P',
        ProcessStatus::LockBlocked => 'L',
        ProcessStatus::UninterruptibleDiskSleep => 'D',
        ProcessStatus::Unknown(_) => '?',
    }
}

fn info_from(system: &System) -> SystemInfo {
    let load = System::load_average();
    SystemInfo {
        os_name: System::long_os_version()
            .or_else(System::os_version)
            .unwrap_or_else(|| System::name().unwrap_or_default()),
        kernel: System::kernel_version().unwrap_or_default(),
        host: System::host_name().unwrap_or_default(),
        uptime_secs: System::uptime(),
        load_one: load.one,
        load_five: load.five,
        load_fifteen: load.fifteen,
        cpu_count: system.cpus().len(),
        total_mem: system.total_memory(),
        used_mem: system.used_memory(),
        total_swap: system.total_swap(),
        used_swap: system.used_swap(),
    }
}

fn describe_text(m: &Monitor, p: &Process) -> String {
    let mut out = String::new();
    let mut line = |label: &str, value: &str| {
        let _ = writeln!(out, "{label:<12}{value}");
    };
    line("PID:", &p.pid().to_string());
    line("Name:", &p.name().to_string_lossy());
    line("User:", &user_name(m, p));
    line(
        "Parent:",
        &p.parent()
            .map_or_else(|| "-".to_string(), |x| x.to_string()),
    );
    line("Status:", &p.status().to_string());
    line("Started:", &format_started(p.start_time()));
    line("CPU time:", &format_cpu_time(p.accumulated_cpu_time()));
    line("Memory:", &format_size(p.memory()));
    line("Virtual:", &format_size(p.virtual_memory()));
    if let Some(exe) = p.exe() {
        line("Exe:", &exe.to_string_lossy());
    }
    if let Some(cwd) = p.cwd() {
        line("Cwd:", &cwd.to_string_lossy());
    }
    if let Some(root) = p.root() {
        line("Root:", &root.to_string_lossy());
    }

    out.push_str("\nCommand:\n  ");
    out.push_str(&cmdline(p));

    out.push_str("\n\nEnvironment:\n");
    if p.environ().is_empty() {
        out.push_str("  (none)\n");
    }
    for e in p.environ() {
        let _ = writeln!(out, "  {}", e.to_string_lossy());
    }
    out
}

/// Render a Unix start time as "how long ago" it started (d/h/m).
fn format_started(secs: u64) -> String {
    let elapsed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
        .saturating_sub(secs);
    format!("{} ago", super::format_uptime(elapsed))
}

/// Canned process data for `--demo` recordings and tests: never touches the
/// real system, so no hostnames, usernames or paths can leak into a demo.
pub struct DemoSource {
    self_pid: i32,
}

impl DemoSource {
    #[must_use]
    pub const fn new() -> Self {
        Self { self_pid: 5000 }
    }
}

impl Default for DemoSource {
    fn default() -> Self {
        Self::new()
    }
}

impl ProcessSource for DemoSource {
    fn snapshot(&self) -> Snapshot {
        demo_snapshot()
    }

    fn detail(&self, pid: i32) -> Option<String> {
        let row = demo_snapshot().rows.into_iter().find(|r| r.pid == pid)?;
        Some(format!(
            "PID:        {}\nName:       {}\nUser:       {}\nStatus:     \
             Sleep\nMemory:     {}\nVirtual:    {}\n\nCommand:\n  {}\n\n\
             Environment:\n  (demo)\n",
            row.pid,
            row.name,
            row.user,
            format_size(row.vsz),
            format_size(row.rss),
            row.command(),
        ))
    }

    fn signal(
        &self,
        _pid: i32,
        _expected_start_time: u64,
        _sig: Signal,
    ) -> SignalReport {
        // Demo mode never signals a real process.
        SignalReport::Sent
    }

    fn self_pid(&self) -> i32 {
        self.self_pid
    }
}

/// Deterministic fake snapshot used by `--demo` and by tests.
#[must_use]
pub fn demo_snapshot() -> Snapshot {
    let rows = vec![
        demo_row(1, None, "root", 'S', 0.0, 4_096_000, "init"),
        demo_row(
            82,
            Some(1),
            "root",
            'S',
            0.0,
            12_000_000,
            "systemd-journald",
        ),
        demo_row(320, Some(1), "root", 'I', 0.1, 0, "kworker/u8:2"),
        demo_row(
            1024,
            Some(1),
            "root",
            'S',
            0.3,
            24_000_000,
            "nginx: master process nginx",
        ),
        demo_row(
            1025,
            Some(1024),
            "www-data",
            'R',
            12.4,
            64_000_000,
            "nginx: worker process",
        ),
        demo_row(
            2048,
            Some(1),
            "postgres",
            'S',
            1.1,
            128_000_000,
            "postgres: checkpointer",
        ),
        demo_row(
            4096,
            Some(1),
            "deploy",
            'R',
            42.7,
            256_000_000,
            "node server.js",
        ),
        demo_row(5000, Some(1), "deploy", 'R', 1.2, 8_000_000, "p7s --demo"),
    ];
    let info = SystemInfo {
        os_name: "Linux (demo)".to_string(),
        kernel: "6.9.0-demo".to_string(),
        host: "demo-host".to_string(),
        uptime_secs: 3 * 86_400 + 4 * 3600 + 12 * 60,
        load_one: 0.42,
        load_five: 0.51,
        load_fifteen: 0.60,
        cpu_count: 8,
        total_mem: 16 * 1024 * 1024 * 1024,
        used_mem: 6 * 1024 * 1024 * 1024,
        total_swap: 8 * 1024 * 1024 * 1024,
        used_swap: 0,
    };
    Snapshot { rows, info }
}

fn demo_row(
    pid: i32,
    parent: Option<i32>,
    user: &str,
    state: char,
    cpu: f32,
    rss: u64,
    cmd: &str,
) -> ProcessRow {
    let name = cmd.split_whitespace().next().unwrap_or(cmd).to_string();
    ProcessRow {
        pid,
        ppid: parent,
        user: user.to_string(),
        cpu,
        mem: percent_of(rss, 16 * 1024 * 1024 * 1024),
        rss,
        vsz: rss.saturating_mul(4),
        state,
        cpu_time_ms: u64::try_from(pid).unwrap_or(0).saturating_mul(1_000),
        name,
        cmdline: cmd.to_string(),
        start_time: 1_700_000_000,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::proc::{SortKey, sort_rows};

    #[test]
    fn demo_snapshot_is_populated_and_sortedable() {
        let snap = demo_snapshot();
        assert!(snap.rows.len() >= 6);
        assert_eq!(snap.info.cpu_count, 8);
        assert_ne!(snap.info.os_name.as_str(), "");
        let mut rows = snap.rows;
        sort_rows(&mut rows, SortKey::Cpu, true);
        // Highest CPU first.
        assert_eq!(rows.first().map(|r| r.pid), Some(4096));
    }

    #[test]
    fn demo_detail_found_for_known_pid_only() {
        let src = DemoSource::new();
        assert!(src.detail(1024).is_some());
        assert!(src.detail(999_999).is_none());
    }

    #[test]
    fn demo_signal_never_touches_the_system() {
        assert_eq!(
            DemoSource::new().signal(1, 0, Signal::Kill),
            SignalReport::Sent
        );
    }

    #[test]
    fn state_char_maps_common_states() {
        assert_eq!(state_char(ProcessStatus::Run), 'R');
        assert_eq!(state_char(ProcessStatus::Sleep), 'S');
        assert_eq!(state_char(ProcessStatus::Zombie), 'Z');
        assert_eq!(state_char(ProcessStatus::Unknown(1)), '?');
    }

    #[test]
    fn real_monitor_takes_a_snapshot() {
        // Smoke test: must not panic on any supported platform.
        let monitor = SystemMonitor::new();
        let self_pid = monitor.self_pid();
        let snap = monitor.snapshot();
        // p7s itself is always in the list.
        assert!(snap.rows.iter().any(|r| r.pid == self_pid));
        assert!(snap.info.cpu_count >= 1);
    }
}
