//! Built-in agent profiles. Patterns are plain data matched against the
//! rendered screen; adjust here when a CLI changes its prompts.

use std::{env, path::Path};

/// How to launch an agent and recognise its state from screen text.
pub struct AgentProfile {
    pub name: &'static str,
    pub command: &'static [&'static str],
    pub resume: Option<&'static [&'static str]>,
    pub blocked: &'static [&'static str],
    pub working: &'static [&'static str],
}

pub const PROFILES: &[AgentProfile] = &[
    AgentProfile {
        name: "claude",
        command: &["claude"],
        resume: Some(&["claude", "--continue"]),
        blocked: &["Do you want to", "❯ 1. Yes"],
        working: &["esc to interrupt"],
    },
    AgentProfile {
        name: "codex",
        command: &["codex"],
        resume: Some(&["codex", "resume", "--last"]),
        blocked: &["Allow command?"],
        working: &["esc to interrupt"],
    },
    AgentProfile {
        name: "cursor-agent",
        command: &["cursor-agent"],
        resume: Some(&["cursor-agent", "resume"]),
        blocked: &["Run this command?", "Allow"],
        working: &["ctrl+c to stop"],
    },
    AgentProfile {
        name: "opencode",
        command: &["opencode"],
        resume: Some(&["opencode", "--continue"]),
        blocked: &["Permission required"],
        working: &["esc interrupt"],
    },
    AgentProfile {
        name: "shell",
        command: &["$SHELL"],
        resume: None,
        blocked: &[],
        working: &[],
    },
];

#[must_use]
pub fn profile(name: &str) -> Option<&'static AgentProfile> {
    PROFILES.iter().find(|p| p.name == name)
}

/// Replace a literal `$SHELL` element with the user's shell.
#[must_use]
pub fn resolve_command(cmd: &[&str]) -> Vec<String> {
    cmd.iter()
        .map(|&a| {
            if a == "$SHELL" {
                env::var("SHELL")
                    .ok()
                    .filter(|s| !s.is_empty())
                    .unwrap_or_else(|| "/bin/sh".into())
            } else {
                a.into()
            }
        })
        .collect()
}

#[cfg(unix)]
fn is_exec(p: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    p.metadata()
        .is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

#[cfg(not(unix))]
fn is_exec(p: &Path) -> bool {
    p.is_file()
}

/// True if `cmd` is an executable path (when it contains `/`) or a bare
/// name found in `PATH`.
#[must_use]
pub fn on_path(cmd: &str) -> bool {
    if cmd.contains('/') {
        return is_exec(Path::new(cmd));
    }
    env::var_os("PATH")
        .is_some_and(|p| env::split_paths(&p).any(|d| is_exec(&d.join(cmd))))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_profile_has_a_command() {
        assert!(PROFILES.iter().all(|p| !p.command.is_empty()));
    }

    #[test]
    fn lookup_by_name() {
        assert_eq!(profile("claude").unwrap().name, "claude");
        assert!(profile("nope").is_none());
    }

    #[test]
    fn resolve_shell() {
        let out = resolve_command(&["$SHELL", "-l"]);
        assert_eq!(out.len(), 2);
        assert_ne!(out.first().map(String::as_str), Some("$SHELL"));
        assert_eq!(out.get(1).map(String::as_str), Some("-l"));
        assert_eq!(resolve_command(&["claude"]), vec!["claude"]);
    }

    #[cfg(unix)]
    #[test]
    fn on_path_names_and_paths() {
        use std::fs;

        assert!(on_path("sh"));
        assert!(on_path("/bin/sh"));
        assert!(!on_path("definitely-not-a-real-cmd-xyz"));
        assert!(!on_path("/nonexistent/xyz"));
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("plain");
        fs::write(&f, "x").unwrap();
        assert!(!on_path(f.to_str().unwrap()), "not executable");
    }
}
