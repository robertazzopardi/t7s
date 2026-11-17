//! Per-repo session manifests: `<repo>/.a4s/sessions.toml`, plus a global
//! `<state_home>/a4s/sessions.toml` for sessions outside any repo.

use std::{
    env::var_os,
    ffi::OsString,
    fs, io,
    path::{Path, PathBuf},
    process::Command,
};

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    pub name: String,
    pub agent: String,
    pub dir: PathBuf,
    pub repo: Option<PathBuf>,
}

/// On-disk shape of both file kinds; `repos` is only used by the global one.
#[derive(Default, Serialize, Deserialize)]
struct File {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    repos: Vec<PathBuf>,
    #[serde(default)]
    sessions: Vec<Entry>,
}

fn git(dir: &Path, args: &[&str]) -> Option<String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
        .filter(|s| !s.is_empty())
}

/// Top level of the git repo containing `dir`; `None` on any failure.
#[must_use]
pub fn repo_root(dir: &Path) -> Option<PathBuf> {
    git(dir, &["rev-parse", "--show-toplevel"]).map(PathBuf::from)
}

#[must_use]
pub fn manifest_path(repo: Option<&Path>, state_home: &Path) -> PathBuf {
    repo.map_or_else(
        || state_home.join("a4s").join("sessions.toml"),
        |r| r.join(".a4s").join("sessions.toml"),
    )
}

fn state_home_from(xdg: Option<OsString>, home: Option<PathBuf>) -> PathBuf {
    xdg.filter(|x| !x.is_empty()).map_or_else(
        || home.unwrap_or_default().join(".local").join("state"),
        PathBuf::from,
    )
}

#[must_use]
pub fn state_home() -> PathBuf {
    state_home_from(var_os("XDG_STATE_HOME"), var_os("HOME").map(PathBuf::from))
}

/// Add `/.a4s/` to the repo's `info/exclude` once. Silent when git is missing.
///
/// # Errors
/// If the exclude file cannot be read or written.
pub fn ensure_excluded(repo: &Path) -> io::Result<()> {
    let Some(common) = git(repo, &["rev-parse", "--git-common-dir"]) else {
        return Ok(());
    };
    let common = repo.join(common); // an absolute result replaces `repo`
    let file = common.join("info").join("exclude");
    let text = fs::read_to_string(&file).unwrap_or_default();
    if text.lines().any(|l| l.trim() == "/.a4s/") {
        return Ok(());
    }
    fs::create_dir_all(common.join("info"))?;
    let sep = if text.is_empty() || text.ends_with('\n') {
        ""
    } else {
        "\n"
    };
    fs::write(&file, format!("{text}{sep}/.a4s/\n"))
}

fn write_atomic(path: &Path, file: &File) -> io::Result<()> {
    let text = toml::to_string(file).map_err(io::Error::other)?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("toml.tmp");
    fs::write(&tmp, text)?;
    fs::rename(&tmp, path)
}

fn read(path: &Path) -> Option<File> {
    toml::from_str(&fs::read_to_string(path).ok()?).ok()
}

/// Write one file per repo plus the global file. Repos listed by the
/// previous global file are rewritten too, so a repo whose last session was
/// forgotten ends up empty rather than stale.
///
/// # Errors
/// The first write error; other files are still attempted.
pub fn save(entries: &[Entry], state_home: &Path) -> io::Result<()> {
    let global_path = manifest_path(None, state_home);
    let mut repos = read(&global_path).map(|f| f.repos).unwrap_or_default();
    for r in entries.iter().filter_map(|e| e.repo.as_ref()) {
        if !repos.contains(r) {
            repos.push(r.clone());
        }
    }
    let mut result = Ok(());
    let mut kept = Vec::new();
    for repo in repos {
        if !repo.is_dir() {
            continue; // repo gone: nothing to write, drop it from the list
        }
        let sessions = entries
            .iter()
            .filter(|e| e.repo.as_ref() == Some(&repo))
            .cloned()
            .collect();
        let r = ensure_excluded(&repo).and_then(|()| {
            write_atomic(
                &manifest_path(Some(&repo), state_home),
                &File {
                    repos: vec![],
                    sessions,
                },
            )
        });
        result = result.and(r);
        kept.push(repo);
    }
    let sessions = entries
        .iter()
        .filter(|e| e.repo.is_none())
        .cloned()
        .collect();
    let r = write_atomic(
        &global_path,
        &File {
            repos: kept,
            sessions,
        },
    );
    result.and(r)
}

/// Global entries plus each listed repo's entries; unreadable files skipped.
#[must_use]
pub fn load(state_home: &Path) -> Vec<Entry> {
    let Some(global) = read(&manifest_path(None, state_home)) else {
        return Vec::new();
    };
    let mut out = global.sessions;
    for repo in &global.repos {
        if let Some(f) = read(&manifest_path(Some(repo), state_home)) {
            out.extend(f.sessions);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn init_repo() -> tempfile::TempDir {
        let d = tempfile::tempdir().unwrap();
        let ok = Command::new("git")
            .arg("init")
            .arg("-q")
            .arg(d.path())
            .status()
            .unwrap()
            .success();
        assert!(ok);
        d
    }

    fn entry(name: &str, dir: &Path, repo: Option<&Path>) -> Entry {
        Entry {
            name: name.into(),
            agent: "claude".into(),
            dir: dir.into(),
            repo: repo.map(Path::to_path_buf),
        }
    }

    fn tmp() -> tempfile::TempDir {
        tempfile::tempdir().unwrap()
    }

    #[test]
    fn round_trip_repo_and_global() {
        let (repo, home, other) = (init_repo(), tmp(), tmp());
        let r = repo_root(repo.path()).unwrap();
        let es = vec![entry("a", &r, Some(&r)), entry("b", other.path(), None)];
        save(&es, home.path()).unwrap();
        let mut got = load(home.path());
        got.sort_by(|x, y| x.name.cmp(&y.name));
        assert_eq!(got, es);
    }

    #[test]
    fn repo_manifest_lives_in_repo() {
        let (repo, home) = (init_repo(), tmp());
        let r = repo_root(repo.path()).unwrap();
        save(&[entry("a", &r, Some(&r))], home.path()).unwrap();
        assert!(r.join(".a4s/sessions.toml").is_file());
    }

    #[test]
    fn exclude_appended_once() {
        let repo = init_repo();
        ensure_excluded(repo.path()).unwrap();
        ensure_excluded(repo.path()).unwrap();
        let text =
            fs::read_to_string(repo.path().join(".git/info/exclude")).unwrap();
        assert_eq!(text.matches("/.a4s/").count(), 1);
        assert!(!repo.path().join(".gitignore").exists());
    }

    #[test]
    fn missing_repo_is_skipped() {
        let (repo, home, other) = (init_repo(), tmp(), tmp());
        let r = repo_root(repo.path()).unwrap();
        let es = [entry("a", &r, Some(&r)), entry("b", other.path(), None)];
        save(&es, home.path()).unwrap();
        drop(repo);
        let got = load(home.path());
        assert_eq!(got, vec![entry("b", other.path(), None)]);
    }

    #[test]
    fn forgotten_last_session_empties_repo_file() {
        let (repo, home) = (init_repo(), tmp());
        let r = repo_root(repo.path()).unwrap();
        save(&[entry("a", &r, Some(&r))], home.path()).unwrap();
        save(&[], home.path()).unwrap();
        assert_eq!(load(home.path()), vec![]);
        let f = read(&r.join(".a4s/sessions.toml")).unwrap();
        assert_eq!(f.sessions, vec![]);
    }

    #[test]
    fn state_home_prefers_xdg() {
        let home = Some(PathBuf::from("/h"));
        let got = state_home_from(Some("/x".into()), home.clone());
        assert_eq!(got, PathBuf::from("/x"));
        let got = state_home_from(Some("".into()), home.clone());
        assert_eq!(got, PathBuf::from("/h/.local/state"));
        let got = state_home_from(None, home);
        assert_eq!(got, PathBuf::from("/h/.local/state"));
    }
}
