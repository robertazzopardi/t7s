//! Per-repo session manifests: `<repo>/.a4s/sessions.toml`, plus a global
//! `<state_home>/a4s/sessions.toml` for sessions outside any repo.

use std::{
    env::var_os,
    ffi::OsString,
    fs::{self, OpenOptions},
    io::{self, Write},
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
    let text = match fs::read_to_string(&file) {
        Ok(t) => t,
        Err(e) if e.kind() == io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(e), // never overwrite what we cannot read
    };
    if text.lines().any(|l| l.trim() == "/.a4s/") {
        return Ok(());
    }
    fs::create_dir_all(common.join("info"))?;
    let sep = if text.is_empty() || text.ends_with('\n') {
        ""
    } else {
        "\n"
    };
    OpenOptions::new()
        .append(true)
        .create(true)
        .open(&file)?
        .write_all(format!("{sep}/.a4s/\n").as_bytes())
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

/// Missing file: `None`. Unreadable or corrupt file: moved aside to a
/// `.bak` (never clobbering an earlier one), a warning is pushed, `None`.
fn read(path: &Path, warnings: &mut Vec<String>) -> Option<File> {
    let text = match fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return None,
        Err(e) => {
            warnings.push(format!("cannot read {}: {e}", path.display()));
            return None;
        }
    };
    match toml::from_str(&text) {
        Ok(f) => Some(f),
        Err(e) => {
            let mut bak = path.with_extension("toml.bak");
            let mut n = 1;
            while bak.exists() {
                bak = path.with_extension(format!("toml.bak.{n}"));
                n += 1;
            }
            let moved = match fs::rename(path, &bak) {
                Ok(()) => format!("saved as {}", bak.display()),
                Err(re) => format!("backup failed: {re}"),
            };
            warnings.push(format!(
                "corrupt manifest {} ({e}); {moved}",
                path.display()
            ));
            None
        }
    }
}

/// Write one file per repo plus the global file. Repos listed by the
/// previous global file are rewritten too, so a repo whose last session was
/// forgotten ends up empty rather than stale.
///
/// # Errors
/// The first write error; other files are still attempted.
pub fn save(entries: &[Entry], state_home: &Path) -> io::Result<()> {
    let global_path = manifest_path(None, state_home);
    let mut ignored = Vec::new(); // load already reported these
    let mut repos = read(&global_path, &mut ignored)
        .map(|f| f.repos)
        .unwrap_or_default();
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
        // Back up a corrupt repo file before it is replaced.
        let _ = read(&manifest_path(Some(&repo), state_home), &mut ignored);
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

/// Global entries plus each listed repo's entries, and warnings for files
/// that were corrupt or unreadable (corrupt ones are backed up).
#[must_use]
pub fn load(state_home: &Path) -> (Vec<Entry>, Vec<String>) {
    let mut warnings = Vec::new();
    let Some(global) = read(&manifest_path(None, state_home), &mut warnings)
    else {
        return (Vec::new(), warnings);
    };
    let mut out = global.sessions;
    for repo in &global.repos {
        if let Some(f) =
            read(&manifest_path(Some(repo), state_home), &mut warnings)
        {
            out.extend(f.sessions);
        }
    }
    (out, warnings)
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
        let mut got = load(home.path()).0;
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
        let got = load(home.path()).0;
        assert_eq!(got, vec![entry("b", other.path(), None)]);
    }

    #[test]
    fn forgotten_last_session_empties_repo_file() {
        let (repo, home) = (init_repo(), tmp());
        let r = repo_root(repo.path()).unwrap();
        save(&[entry("a", &r, Some(&r))], home.path()).unwrap();
        save(&[], home.path()).unwrap();
        assert_eq!(load(home.path()).0, vec![]);
        let f = read(&r.join(".a4s/sessions.toml"), &mut vec![]).unwrap();
        assert_eq!(f.sessions, vec![]);
    }

    #[test]
    fn corrupt_global_is_backed_up_not_destroyed() {
        let (home, other) = (tmp(), tmp());
        let path = manifest_path(None, home.path());
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, "not [valid toml").unwrap();
        let (got, warnings) = load(home.path());
        assert_eq!(got, vec![]);
        assert_eq!(warnings.len(), 1);
        let bak = path.with_extension("toml.bak");
        assert_eq!(fs::read_to_string(&bak).unwrap(), "not [valid toml");
        save(&[entry("b", other.path(), None)], home.path()).unwrap();
        assert_eq!(fs::read_to_string(&bak).unwrap(), "not [valid toml");
        assert_eq!(load(home.path()).0.len(), 1);
    }

    #[test]
    fn corrupt_repo_file_is_backed_up() {
        let (repo, home) = (init_repo(), tmp());
        let r = repo_root(repo.path()).unwrap();
        save(&[entry("a", &r, Some(&r))], home.path()).unwrap();
        let path = manifest_path(Some(&r), home.path());
        fs::write(&path, "garbage = [").unwrap();
        let (got, warnings) = load(home.path());
        assert_eq!((got, warnings.len()), (vec![], 1));
        let bak = path.with_extension("toml.bak");
        assert_eq!(fs::read_to_string(&bak).unwrap(), "garbage = [");
        // A second corrupt file must not clobber the first backup.
        fs::write(&path, "again = [").unwrap();
        let _ = load(home.path());
        assert_eq!(fs::read_to_string(&bak).unwrap(), "garbage = [");
        assert!(path.with_extension("toml.bak.1").exists());
    }

    #[test]
    fn exclude_keeps_existing_line_without_newline() {
        let repo = init_repo();
        let file = repo.path().join(".git/info/exclude");
        fs::write(&file, "target").unwrap();
        ensure_excluded(repo.path()).unwrap();
        assert_eq!(fs::read_to_string(&file).unwrap(), "target\n/.a4s/\n");
    }

    #[test]
    fn unreadable_exclude_is_not_truncated() {
        let repo = init_repo();
        let file = repo.path().join(".git/info/exclude");
        fs::write(&file, [0xff, 0xfe, b'\n']).unwrap();
        assert!(ensure_excluded(repo.path()).is_err());
        assert_eq!(fs::read(&file).unwrap(), [0xff, 0xfe, b'\n']);
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
