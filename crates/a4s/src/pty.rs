//! A child process on a PTY, with its output parsed into a vt100 screen.
use core::error::Error;
use std::{
    io::{self, Read, Write},
    panic::{AssertUnwindSafe, catch_unwind},
    path::Path,
    sync::{Arc, Mutex, MutexGuard, PoisonError, mpsc},
    thread,
    time::Duration,
};

use portable_pty::{
    Child, CommandBuilder, MasterPty, PtySize, native_pty_system,
};

use crate::{agent::on_path, defaults::SCROLLBACK_LINES, keys::KittyMode};

pub type SessionId = u64;

pub struct Pty {
    child: Box<dyn Child + Send + Sync>,
    master: Box<dyn MasterPty + Send>,
    /// Bytes for the writer thread, so a child that stops reading never
    /// blocks the UI thread.
    writer: mpsc::Sender<Vec<u8>>,
    parser: Arc<Mutex<vt100::Parser<Replies>>>,
    kitty: Arc<Mutex<KittyMode>>,
    exit: Option<u32>,
}

const fn size(rows: u16, cols: u16) -> PtySize {
    PtySize {
        rows,
        cols,
        pixel_width: 0,
        pixel_height: 0,
    }
}

// portable-pty returns anyhow errors; keep an inner io::Error (e.g. NotFound) intact.
fn to_io(e: impl Into<Box<dyn Error + Send + Sync>>) -> io::Error {
    match e.into().downcast::<io::Error>() {
        Ok(e) => *e,
        Err(e) => io::Error::other(e),
    }
}

/// Answers the terminal queries agents block on (cursor position, device
/// attributes, kitty keyboard flags) by writing the reply to the PTY.
pub struct Replies {
    tx: mpsc::Sender<Vec<u8>>,
    kitty: Arc<Mutex<KittyMode>>,
}

impl vt100::Callbacks for Replies {
    fn unhandled_csi(
        &mut self,
        screen: &mut vt100::Screen,
        i1: Option<u8>,
        i2: Option<u8>,
        params: &[&[u16]],
        c: char,
    ) {
        let first = params.first().and_then(|p| p.first()).copied();
        let reply = match (i1, i2, c, first) {
            (None, None, 'n', Some(6)) => {
                let (row, col) = screen.cursor_position();
                format!("\x1b[{};{}R", row + 1, col + 1)
            }
            (None, None, 'c', None | Some(0)) => "\x1b[?1;2c".to_string(),
            (Some(b'?'), None, 'u', _) => {
                let flags = self
                    .kitty
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .flags();
                format!("\x1b[?{flags}u")
            }
            _ => return,
        };
        let _ = self.tx.send(reply.into_bytes());
    }
}

/// Swap in a fresh parser of the same size, after a panic left the old one
/// half-updated. The screen content is lost; the replies keep working.
fn reset_parser(p: &mut vt100::Parser<Replies>) {
    let (rows, cols) = p.screen().size();
    let replies = Replies {
        tx: p.callbacks().tx.clone(),
        kitty: Arc::clone(&p.callbacks().kitty),
    };
    *p = vt100::Parser::new_with_callbacks(
        rows,
        cols,
        SCROLLBACK_LINES,
        replies,
    );
}

impl Pty {
    /// `on_output` runs on the reader thread after each chunk and once more at EOF.
    ///
    /// # Errors
    /// `NotFound` if the program is not on PATH, otherwise any PTY or spawn failure.
    pub fn spawn(
        cmd: &[String],
        cwd: &Path,
        env: &[(String, String)],
        rows: u16,
        cols: u16,
        on_output: impl Fn() + Send + 'static,
    ) -> io::Result<Self> {
        let (prog, args) = cmd.split_first().ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, "empty command")
        })?;
        // portable-pty reports a missing binary as a plain string error.
        if !on_path(prog) {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!("`{prog}` not found on PATH"),
            ));
        }
        let pair = native_pty_system()
            .openpty(size(rows, cols))
            .map_err(to_io)?;
        let mut builder = CommandBuilder::new(prog);
        builder.args(args);
        builder.cwd(cwd);
        // Output is parsed by vt100, not the outer terminal, so advertise that.
        builder.env("TERM", "xterm-256color");
        builder.env_remove("TERM_PROGRAM");
        builder.env_remove("TERM_PROGRAM_VERSION");
        for (k, v) in env {
            builder.env(k, v);
        }
        let child = pair.slave.spawn_command(builder).map_err(to_io)?;
        drop(pair.slave);
        let mut reader = pair.master.try_clone_reader().map_err(to_io)?;
        let mut pty_writer = pair.master.take_writer().map_err(to_io)?;
        let (writer, rx) = mpsc::channel::<Vec<u8>>();
        thread::spawn(move || {
            while let Ok(bytes) = rx.recv() {
                if pty_writer
                    .write_all(&bytes)
                    .and_then(|()| pty_writer.flush())
                    .is_err()
                {
                    break;
                }
            }
        });

        let kitty = Arc::new(Mutex::new(KittyMode::default()));
        let replies = Replies {
            tx: writer.clone(),
            kitty: Arc::clone(&kitty),
        };
        let parser = Arc::new(Mutex::new(vt100::Parser::new_with_callbacks(
            rows,
            cols,
            SCROLLBACK_LINES,
            replies,
        )));
        let thread_parser = Arc::clone(&parser);
        let thread_kitty = Arc::clone(&kitty);
        thread::spawn(move || {
            let mut buf = [0u8; 8192];
            while let Ok(n @ 1..) = reader.read(&mut buf) {
                let chunk = buf.get(..n).unwrap_or_default();
                // Kitty first, so a flags query in this chunk sees a push.
                thread_kitty
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .feed(chunk);
                let mut p = thread_parser
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner);
                // A vt100 bug must not take the app (and every agent) down.
                // Keep draining the PTY either way, or the child blocks on write.
                if catch_unwind(AssertUnwindSafe(|| p.process(chunk))).is_err()
                {
                    reset_parser(&mut p);
                }
                drop(p);
                on_output();
            }
            on_output();
        });

        Ok(Self {
            child,
            master: pair.master,
            writer,
            parser,
            kitty,
            exit: None,
        })
    }

    /// Queue bytes for the child; never blocks.
    ///
    /// # Errors
    /// If the writer thread has stopped (a PTY write failed).
    pub fn write(&mut self, bytes: &[u8]) -> io::Result<()> {
        self.writer
            .send(bytes.to_vec())
            .map_err(|_| io::Error::from(io::ErrorKind::BrokenPipe))
    }

    /// # Errors
    /// If the PTY resize fails.
    pub fn resize(&mut self, rows: u16, cols: u16) -> io::Result<()> {
        self.master.resize(size(rows, cols)).map_err(to_io)?;
        self.screen().screen_mut().set_size(rows, cols);
        Ok(())
    }

    /// The parser, even if a reader thread panicked while holding it.
    pub fn screen(&self) -> MutexGuard<'_, vt100::Parser<Replies>> {
        self.parser.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Whether the child enabled the kitty keyboard protocol.
    #[must_use]
    pub fn kitty_keys(&self) -> bool {
        self.kitty
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .enabled()
    }

    /// Exit code once the child has exited.
    pub fn try_exit(&mut self) -> Option<u32> {
        if self.exit.is_none() {
            self.exit =
                self.child.try_wait().ok().flatten().map(|s| s.exit_code());
        }
        self.exit
    }
}

impl Drop for Pty {
    fn drop(&mut self) {
        if self.try_exit().is_some() {
            return;
        }
        let _ = self.child.kill();
        // Reap, but never block on a child that ignores SIGHUP.
        for _ in 0..20 {
            if self.try_exit().is_some() {
                break;
            }
            thread::sleep(Duration::from_millis(10));
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use super::*;

    #[test]
    fn queries_get_replies() {
        let (tx, rx) = mpsc::channel();
        let kitty = Arc::new(Mutex::new(KittyMode::default()));
        let replies = Replies {
            tx,
            kitty: Arc::clone(&kitty),
        };
        let mut p = vt100::Parser::new_with_callbacks(24, 80, 0, replies);
        p.process(b"ab\x1b[6n\x1b[c\x1b[?u");
        kitty
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .feed(b"\x1b[>3u");
        p.process(b"\x1b[?u");
        let got: Vec<Vec<u8>> = rx.try_iter().collect();
        assert_eq!(
            got,
            [
                b"\x1b[1;3R".to_vec(),
                b"\x1b[?1;2c".to_vec(),
                b"\x1b[?0u".to_vec(),
                b"\x1b[?3u".to_vec(),
            ]
        );
    }

    #[test]
    fn reset_keeps_size_and_replies() {
        let (tx, rx) = mpsc::channel();
        let kitty = Arc::new(Mutex::new(KittyMode::default()));
        let replies = Replies {
            tx,
            kitty: Arc::clone(&kitty),
        };
        let mut p = vt100::Parser::new_with_callbacks(10, 40, 0, replies);
        p.process(b"junk");
        reset_parser(&mut p);
        assert_eq!(p.screen().size(), (10, 40));
        assert_eq!(p.screen().contents(), "");
        p.process(b"\x1b[6n");
        assert_eq!(rx.try_iter().collect::<Vec<_>>(), [b"\x1b[1;1R".to_vec()]);
    }

    #[test]
    fn child_sees_xterm_term_and_no_term_program() {
        let pty = Pty::spawn(
            &[
                "/bin/sh".into(),
                "-c".into(),
                "echo \"$TERM|${TERM_PROGRAM-unset}\"".into(),
            ],
            Path::new("/"),
            &[],
            24,
            80,
            || {},
        )
        .unwrap();
        let want = "xterm-256color|unset";
        let deadline = Instant::now() + Duration::from_secs(5);
        while !pty.screen().screen().contents().contains(want) {
            assert!(Instant::now() < deadline, "no output");
            thread::sleep(Duration::from_millis(10));
        }
    }
}
