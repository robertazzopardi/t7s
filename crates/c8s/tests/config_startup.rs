use std::{
    fs,
    path::PathBuf,
    process::{Command, Output, Stdio},
};

fn run_with_config(test: &str, config: &str) -> (Output, PathBuf) {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(test);
    let config_dir = dir.join("t7s");
    fs::create_dir_all(&config_dir).unwrap();
    fs::write(config_dir.join("config.toml"), config).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_c8s"))
        .env("XDG_CONFIG_HOME", &dir)
        .stdin(Stdio::null())
        .output()
        .unwrap();
    (output, dir)
}

#[test]
fn conflict_exits_nonzero_before_tui() {
    let (output, _) =
        run_with_config("c8s-conflict", "[c8s.keys]\nrestart = \"s\"\n");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(1), "{stderr}");
    assert!(stderr.contains("config.toml"), "{stderr}");
    assert!(
        stderr.contains("restart") && stderr.contains("start-stop"),
        "{stderr}"
    );
    assert!(output.stdout.is_empty(), "TUI must not have started");
}

#[test]
fn syntax_error_exits_nonzero_before_tui() {
    let (output, _) = run_with_config("c8s-syntax", "[t7s\n");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(1), "{stderr}");
    assert!(stderr.contains("config.toml"), "{stderr}");
    assert!(output.stdout.is_empty(), "TUI must not have started");
}
