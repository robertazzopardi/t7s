use std::{
    fs,
    path::PathBuf,
    process::{Command, Output, Stdio},
};

fn run_with_config(test: &str, config: &str) -> Output {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(test);
    let config_dir = dir.join("t7s");
    fs::create_dir_all(&config_dir).unwrap();
    fs::write(config_dir.join("config.toml"), config).unwrap();
    Command::new(env!("CARGO_BIN_EXE_d7s"))
        .env("XDG_CONFIG_HOME", &dir)
        .stdin(Stdio::null())
        .output()
        .unwrap()
}

#[test]
fn conflict_exits_nonzero_before_tui() {
    let output =
        run_with_config("d7s-conflict", "[t7s.keys]\nsearch = \"x\"\n");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(1), "{stderr}");
    assert!(stderr.contains("config.toml"), "{stderr}");
    assert!(
        stderr.contains("search") && stderr.contains("export"),
        "{stderr}"
    );
    assert!(output.stdout.is_empty(), "TUI must not have started");
}

#[test]
fn syntax_error_exits_nonzero_before_tui() {
    let output = run_with_config("d7s-syntax", "[t7s\n");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(1), "{stderr}");
    assert!(stderr.contains("config.toml"), "{stderr}");
    assert!(output.stdout.is_empty(), "TUI must not have started");
}
