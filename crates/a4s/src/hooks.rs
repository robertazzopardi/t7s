//! Agent hook channel: `a4s notify <state>` writes `"<id> <state>\n"` to a
//! unix socket the running app listens on. Unix only.

use std::{
    env, fs,
    io::{self, Write},
    os::unix::net::UnixStream,
    path::{Path, PathBuf},
    process,
};

use tokio::{
    io::{AsyncBufReadExt, BufReader},
    net::UnixListener,
    sync::mpsc::UnboundedSender,
};

use crate::{
    app::BackgroundEvent, defaults::socket_name, pty::SessionId, status::Status,
};

fn parse_state(word: &str) -> Option<Status> {
    match word {
        "working" => Some(Status::Working),
        "blocked" => Some(Status::Blocked),
        "done" => Some(Status::Done),
        _ => None,
    }
}

fn parse_line(line: &str) -> Option<(SessionId, Status)> {
    let (id, state) = line.split_once(' ')?;
    Some((id.parse().ok()?, parse_state(state.trim())?))
}

/// Listening hook socket; the file is removed on drop.
pub struct HookSocket {
    path: PathBuf,
}

impl HookSocket {
    /// Bind `$TMPDIR/a4s-<pid>.sock` and forward each valid line as a
    /// `BackgroundEvent::Hook`. Needs a tokio runtime.
    ///
    /// # Errors
    /// If the socket cannot be bound.
    pub fn bind(tx: UnboundedSender<BackgroundEvent>) -> io::Result<Self> {
        let path = env::temp_dir().join(socket_name(process::id()));
        let _ = fs::remove_file(&path);
        let listener = UnixListener::bind(&path)?;
        tokio::spawn(async move {
            while let Ok((conn, _)) = listener.accept().await {
                let tx = tx.clone();
                tokio::spawn(async move {
                    let mut lines = BufReader::new(conn).lines();
                    while let Ok(Some(line)) = lines.next_line().await {
                        if let Some((id, status)) = parse_line(&line) {
                            let _ =
                                tx.send(BackgroundEvent::Hook { id, status });
                        }
                    }
                });
            }
        });
        Ok(Self { path })
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for HookSocket {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

/// Handle `a4s notify <state>`; returns the exit code. Never fails the
/// calling agent: missing env or an unreachable socket exit 0.
#[must_use]
pub fn notify(state: &str) -> i32 {
    if parse_state(state).is_none() {
        eprintln!("a4s notify: state must be working, blocked or done");
        return 2;
    }
    let (Some(sock), Some(id)) =
        (env::var_os("A4S_SOCKET"), env::var_os("A4S_SESSION_ID"))
    else {
        return 0;
    };
    if let Ok(mut s) = UnixStream::connect(sock) {
        let _ = writeln!(s, "{} {state}", id.to_string_lossy());
    }
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_lines_and_ignores_junk() {
        assert_eq!(parse_line("7 blocked"), Some((7, Status::Blocked)));
        assert_eq!(parse_line("x done"), None);
        assert_eq!(parse_line("7 bogus"), None);
        assert_eq!(parse_line("garbage"), None);
    }
}
