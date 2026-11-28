#![cfg(unix)]
use std::{io::Read, os::unix::net::UnixListener, process::Command};

#[test]
fn notify_writes_line_to_socket() {
    let dir = tempfile::tempdir().unwrap();
    let sock = dir.path().join("h.sock");
    let listener = UnixListener::bind(&sock).unwrap();
    let status = Command::new(env!("CARGO_BIN_EXE_a4s"))
        .args(["notify", "blocked"])
        .env("A4S_SOCKET", &sock)
        .env("A4S_SESSION_ID", "7")
        .status()
        .unwrap();
    assert!(status.success());
    let (mut conn, _) = listener.accept().unwrap();
    let mut got = String::new();
    conn.read_to_string(&mut got).unwrap();
    assert_eq!(got, "7 blocked\n");
}

#[test]
fn notify_without_env_exits_zero() {
    let status = Command::new(env!("CARGO_BIN_EXE_a4s"))
        .args(["notify", "done"])
        .env_remove("A4S_SOCKET")
        .env_remove("A4S_SESSION_ID")
        .status()
        .unwrap();
    assert!(status.success());
}

#[test]
fn notify_bad_state_exits_two() {
    let status = Command::new(env!("CARGO_BIN_EXE_a4s"))
        .args(["notify", "bogus"])
        .status()
        .unwrap();
    assert_eq!(status.code(), Some(2));
}
