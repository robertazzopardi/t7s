//! A child process on a PTY, with its output parsed into a vt100 screen.
use core::error::Error;
use std::{
    io::{self, Read, Write},
    path::Path,
    sync::{Arc, Mutex, MutexGuard},
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
    writer: Box<dyn Write + Send>,
    parser: Arc<Mutex<vt100::Parser>>,
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
                format!("{prog} not found on PATH"),
            ));
        }
        let pair = native_pty_system()
            .openpty(size(rows, cols))
            .map_err(to_io)?;
        let mut builder = CommandBuilder::new(prog);
        builder.args(args);
        builder.cwd(cwd);
        for (k, v) in env {
            builder.env(k, v);
        }
        let child = pair.slave.spawn_command(builder).map_err(to_io)?;
        drop(pair.slave);
        let mut reader = pair.master.try_clone_reader().map_err(to_io)?;
        let writer = pair.master.take_writer().map_err(to_io)?;
        let parser = Arc::new(Mutex::new(vt100::Parser::new(
            rows,
            cols,
            SCROLLBACK_LINES,
        )));

        let kitty = Arc::new(Mutex::new(KittyMode::default()));
        let thread_parser = Arc::clone(&parser);
        let thread_kitty = Arc::clone(&kitty);
        thread::spawn(move || {
            let mut buf = [0u8; 8192];
            while let Ok(n @ 1..) = reader.read(&mut buf) {
                if let (Ok(mut p), Some(chunk)) =
                    (thread_parser.lock(), buf.get(..n))
                {
                    p.process(chunk);
                }
                if let (Ok(mut k), Some(chunk)) =
                    (thread_kitty.lock(), buf.get(..n))
                {
                    k.feed(chunk);
                }
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

    /// # Errors
    /// If the PTY write fails (e.g. the child is gone).
    pub fn write(&mut self, bytes: &[u8]) -> io::Result<()> {
        self.writer.write_all(bytes)?;
        self.writer.flush()
    }

    /// # Errors
    /// If the PTY resize fails.
    pub fn resize(&mut self, rows: u16, cols: u16) -> io::Result<()> {
        self.master.resize(size(rows, cols)).map_err(to_io)?;
        self.screen().screen_mut().set_size(rows, cols);
        Ok(())
    }

    /// # Panics
    /// If the reader thread panicked while holding the lock.
    pub fn screen(&self) -> MutexGuard<'_, vt100::Parser> {
        self.parser.lock().expect("parser lock poisoned")
    }

    /// Whether the child enabled the kitty keyboard protocol.
    #[must_use]
    pub fn kitty_keys(&self) -> bool {
        self.kitty.lock().is_ok_and(|k| k.enabled())
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
