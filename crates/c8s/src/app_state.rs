use std::fmt::{Display, Formatter};

use crate::docker::client::DaemonHealth;

/// Top-level application state.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum AppState {
    /// Still trying to reach the Docker daemon.
    #[default]
    Connecting,
    /// Daemon unreachable — show the error and offer retry.
    ConnectError(String),
    /// Normal resource list view (containers/images/volumes/networks).
    List,
    /// Full-screen log tail for one container.
    Logs { id: String, name: String },
    /// Daemon health/info panel (c8s's single-daemon analog of a fleet health
    /// dashboard) — version, API version, OS/arch, container counts, image count.
    Info(DaemonHealth),
    /// Full-screen process table for one container.
    Processes { id: String, name: String },
    /// Full-screen `docker inspect`-style details for one container.
    Describe { name: String, text: String },
}

/// Which resource-type table the list view is currently showing.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ResourceKind {
    #[default]
    Containers,
    Images,
    Volumes,
    Networks,
}

impl Display for ResourceKind {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Containers => "Containers",
            Self::Images => "Images",
            Self::Volumes => "Volumes",
            Self::Networks => "Networks",
        })
    }
}
