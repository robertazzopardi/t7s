use std::{
    io::ErrorKind,
    path::Path,
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

use a4s::pty::Pty;

fn spawn(cmd: &[&str]) -> Pty {
    let cmd: Vec<String> = cmd.iter().map(|s| (*s).to_owned()).collect();
    Pty::spawn(&cmd, Path::new("."), &[], 24, 80, || {}).expect("spawn")
}

fn wait_for<T>(mut f: impl FnMut() -> Option<T>) -> T {
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        if let Some(v) = f() {
            return v;
        }
        assert!(Instant::now() < deadline, "timed out");
        thread::sleep(Duration::from_millis(10));
    }
}

fn screen_text(p: &Pty) -> String {
    p.screen().screen().contents()
}

#[test]
fn echo_reaches_screen() {
    let p = spawn(&["sh", "-c", "echo hello"]);
    wait_for(|| screen_text(&p).contains("hello").then_some(()));
}

#[test]
fn input_reaches_child() {
    let mut p = spawn(&["sh", "-c", "read x; echo got:$x"]);
    p.write(b"abc\r").unwrap();
    wait_for(|| screen_text(&p).contains("got:abc").then_some(()));
}

#[test]
fn exit_is_reported() {
    let mut p = spawn(&["sh", "-c", "exit 3"]);
    assert_eq!(wait_for(|| p.try_exit()), 3);
}

#[test]
fn resize_changes_screen_size() {
    let mut p = spawn(&["sh", "-c", "sleep 5"]);
    p.resize(10, 40).unwrap();
    assert_eq!(p.screen().screen().size(), (10, 40));
}

#[test]
fn drop_kills_child() {
    let p = spawn(&["sh", "-c", "echo $$; sleep 30"]);
    let pid = wait_for(|| {
        screen_text(&p)
            .lines()
            .next()
            .and_then(|l| l.trim().parse::<u32>().ok())
    });
    drop(p);
    let alive = || {
        Command::new("kill")
            .args(["-0", &pid.to_string()])
            .stderr(Stdio::null())
            .status()
            .unwrap()
            .success()
    };
    wait_for(|| (!alive()).then_some(()));
}

#[test]
fn write_to_child_that_never_reads_does_not_block() {
    let mut p = spawn(&["sh", "-c", "sleep 30"]);
    let start = Instant::now();
    p.write(&vec![b'x'; 256 * 1024]).unwrap();
    p.write(b"more").unwrap();
    assert!(start.elapsed() < Duration::from_secs(1));
}

#[test]
fn cursor_position_query_is_answered() {
    // Raw mode so `dd` gets the `ESC[1;1R` reply without a newline.
    let p = spawn(&[
        "sh",
        "-c",
        "stty raw -echo; printf '\\033[6n'; dd bs=1 count=6 >/dev/null 2>&1; \
         stty sane; echo got-cpr",
    ]);
    wait_for(|| screen_text(&p).contains("got-cpr").then_some(()));
}

#[test]
fn missing_binary_is_not_found() {
    let cmd = vec!["a4s-no-such-cmd".to_owned()];
    let Err(e) = Pty::spawn(&cmd, Path::new("."), &[], 24, 80, || {}) else {
        panic!("expected error");
    };
    assert_eq!(e.kind(), ErrorKind::NotFound);
}
