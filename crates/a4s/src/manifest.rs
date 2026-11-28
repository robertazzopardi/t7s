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
    /// Seconds since `UNIX_EPOCH`. Absent in manifests written before this field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created: Option<u64>,
    /// Seconds since `UNIX_EPOCH`; `None` while the session is still running.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ended: Option<u64>,
    /// Exit code of an ended session; absent means 0.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<u32>,
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

/// Missing file: `Ok(None)`. Unreadable or corrupt file: moved aside to a
/// `.bak` (never clobbering an earlier one) with a warning, `Ok(None)`. If the
/// move fails the file stays put and the result is `Err(())`: callers must
/// not write that path.
#[allow(clippy::result_unit_err)]
fn read(path: &Path, warnings: &mut Vec<String>) -> Result<Option<File>, ()> {
    let problem = match fs::read_to_string(path) {
        Ok(text) => match toml::from_str(&text) {
            Ok(f) => return Ok(Some(f)),
            Err(e) => e.to_string(),
        },
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => e.to_string(),
    };
    let mut bak = path.with_extension("toml.bak");
    let mut n = 1;
    while bak.exists() {
        bak = path.with_extension(format!("toml.bak.{n}"));
        n += 1;
    }
    let (moved, result) = match fs::rename(path, &bak) {
        Ok(()) => (format!("saved as {}", bak.display()), Ok(None)),
        Err(re) => (format!("backup failed, left untouched: {re}"), Err(())),
    };
    warnings.push(format!(
        "bad manifest {} ({problem}); {moved}",
        path.display()
    ));
    result
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
    let Ok(old) = read(&global_path, &mut ignored) else {
        return Err(io::Error::other("global manifest is unreadable"));
    };
    let mut repos = old.map(|f| f.repos).unwrap_or_default();
    for r in entries.iter().filter_map(|e| e.repo.as_ref()) {
        if !repos.contains(r) {
            repos.push(r.clone());
        }
    }
    let mut result = Ok(());
    let mut kept = Vec::new();
    for repo in repos {
        if !repo.is_dir() {
            // Unmounted or gone for now: keep it listed so its file loads later.
            kept.push(repo);
            continue;
        }
        let sessions = entries
            .iter()
            .filter(|e| e.repo.as_ref() == Some(&repo))
            .cloned()
            .collect();
        // Back up a corrupt repo file before it is replaced; never write a
        // file that could not be backed up.
        if read(&manifest_path(Some(&repo), state_home), &mut ignored).is_err()
        {
            result = result.and_then(|()| {
                Err(io::Error::other("repo manifest is unreadable"))
            });
            kept.push(repo);
            continue;
        }
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

/// What `load` found.
#[derive(Default)]
pub struct Loaded {
    pub entries: Vec<Entry>,
    pub warnings: Vec<String>,
    /// Repos whose manifest was read (or is absent) this run.
    pub repos: Vec<PathBuf>,
}

/// Global entries plus each listed repo's entries, and warnings for files
/// that were corrupt or unreadable (those are backed up).
#[must_use]
pub fn load(state_home: &Path) -> Loaded {
    let mut out = Loaded::default();
    let Ok(Some(global)) =
        read(&manifest_path(None, state_home), &mut out.warnings)
    else {
        return out;
    };
    out.entries = global.sessions;
    for repo in global.repos {
        if let Some(entries) = load_repo(&repo, state_home, &mut out.warnings) {
            out.entries.extend(entries);
            out.repos.push(repo);
        }
    }
    out
}

/// One repo's entries; `None` if its file is bad and could not be backed up.
#[must_use]
pub fn load_repo(
    repo: &Path,
    state_home: &Path,
    warnings: &mut Vec<String>,
) -> Option<Vec<Entry>> {
    let file = read(&manifest_path(Some(repo), state_home), warnings).ok()?;
    Some(file.map(|f| f.sessions).unwrap_or_default())
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
            created: None,
            ended: None,
            exit_code: None,
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
        let mut got = load(home.path()).entries;
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
        let got = load(home.path()).entries;
        assert_eq!(got, vec![entry("b", other.path(), None)]);
    }

    #[test]
    fn missing_repo_dir_stays_listed() {
        let (home, other) = (tmp(), tmp());
        let gone = other.path().join("unmounted");
        save(&[entry("a", &gone, Some(&gone))], home.path()).unwrap();
        let global = read(&manifest_path(None, home.path()), &mut vec![])
            .unwrap()
            .unwrap();
        assert_eq!(global.repos, vec![gone]);
        let loaded = load(home.path());
        assert_eq!(loaded.warnings, Vec::<String>::new());
        assert_eq!(loaded.entries, vec![]);
    }

    #[test]
    fn forgotten_last_session_empties_repo_file() {
        let (repo, home) = (init_repo(), tmp());
        let r = repo_root(repo.path()).unwrap();
        save(&[entry("a", &r, Some(&r))], home.path()).unwrap();
        save(&[], home.path()).unwrap();
        assert_eq!(load(home.path()).entries, vec![]);
        let f = read(&r.join(".a4s/sessions.toml"), &mut vec![])
            .unwrap()
            .unwrap();
        assert_eq!(f.sessions, vec![]);
    }

    #[test]
    fn corrupt_global_is_backed_up_not_destroyed() {
        let (home, other) = (tmp(), tmp());
        let path = manifest_path(None, home.path());
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, "not [valid toml").unwrap();
        let l = load(home.path());
        assert_eq!(l.entries, vec![]);
        assert_eq!(l.warnings.len(), 1);
        let bak = path.with_extension("toml.bak");
        assert_eq!(fs::read_to_string(&bak).unwrap(), "not [valid toml");
        save(&[entry("b", other.path(), None)], home.path()).unwrap();
        assert_eq!(fs::read_to_string(&bak).unwrap(), "not [valid toml");
        assert_eq!(load(home.path()).entries.len(), 1);
    }

    #[test]
    fn non_utf8_global_is_backed_up() {
        let (home, other) = (tmp(), tmp());
        let path = manifest_path(None, home.path());
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, [0xff, 0xfe]).unwrap();
        let l = load(home.path());
        assert_eq!((l.entries, l.warnings.len()), (vec![], 1));
        let bak = path.with_extension("toml.bak");
        assert_eq!(fs::read(&bak).unwrap(), [0xff, 0xfe]);
        save(&[entry("b", other.path(), None)], home.path()).unwrap();
        assert_eq!(fs::read(&bak).unwrap(), [0xff, 0xfe]);
    }

    #[test]
    fn corrupt_repo_file_is_backed_up() {
        let (repo, home) = (init_repo(), tmp());
        let r = repo_root(repo.path()).unwrap();
        save(&[entry("a", &r, Some(&r))], home.path()).unwrap();
        let path = manifest_path(Some(&r), home.path());
        fs::write(&path, "garbage = [").unwrap();
        let l = load(home.path());
        assert_eq!((l.entries, l.warnings.len()), (vec![], 1));
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
    fn old_manifest_without_times_loads() {
        let home = tmp();
        let path = manifest_path(None, home.path());
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(
            &path,
            "[[sessions]]\nname = \"a\"\nagent = \"claude\"\ndir = \"/x\"\n",
        )
        .unwrap();
        let l = load(home.path());
        assert_eq!(l.warnings, Vec::<String>::new());
        assert_eq!(
            l.entries,
            vec![Entry {
                name: "a".into(),
                agent: "claude".into(),
                dir: "/x".into(),
                repo: None,
                created: None,
                ended: None,
                exit_code: None,
            }]
        );
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
