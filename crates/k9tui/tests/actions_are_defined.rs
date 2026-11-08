//! Actions are defined in TOML and wired in Rust, so the two can't drift.
use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
};

use toml::Table;

fn crates_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .to_path_buf()
}

fn rust_files(dir: &Path, found: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            rust_files(&path, found);
        } else if path.extension().is_some_and(|ext| ext == "rs")
            && !path.ends_with("default_bars_test.rs")
        {
            found.push(path);
        }
    }
}

/// A crate's Rust source, without its test modules.
fn source(name: &str) -> String {
    let mut files = Vec::new();
    rust_files(&crates_dir().join(name).join("src"), &mut files);
    files
        .iter()
        .map(|file| {
            let text = fs::read_to_string(file).unwrap();
            text.split("#[cfg(test)]\nmod ")
                .next()
                .unwrap_or_default()
                .to_owned()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn table(name: &str, level: &str, field: &str) -> BTreeSet<String> {
    let path = crates_dir().join(name).join("src/config.toml");
    let file: Table = fs::read_to_string(path).unwrap().parse().unwrap();
    file.get(level)
        .and_then(|value| value.get(field))
        .and_then(toml::Value::as_table)
        .map(|keys| keys.keys().cloned().collect())
        .unwrap_or_default()
}

fn actions(app: &str) -> BTreeSet<String> {
    let mut all = table("k9tui", "t7s", "keys");
    all.extend(table(app, app, "keys"));
    all
}

fn quoted(text: &str) -> Vec<&str> {
    text.split('"').skip(1).step_by(2).collect()
}

/// `(screen, action)` for each `fires(...)` and `action` for each label call.
fn uses(source: &str) -> (Vec<&str>, Vec<&str>) {
    let mut screens = Vec::new();
    let mut names = Vec::new();
    for call in source.split("fires(").skip(1) {
        let args = call.split(')').next().unwrap_or_default();
        // Closures forward a variable, and `fn fires` declares parameters.
        if args.contains("action") {
            screens.extend(quoted(args).first());
            continue;
        }
        let found = quoted(args);
        match found.as_slice() {
            [action] => names.push(*action),
            [screen, action, ..] => {
                screens.push(*screen);
                names.push(*action);
            }
            [] => {}
        }
    }
    for needle in [".label(\"", ".labels(\"", "keymap.get(\""] {
        for call in source.split(needle).skip(1) {
            names.extend(call.split('"').next());
        }
    }
    (screens, names)
}

#[test]
fn every_action_used_in_rust_is_defined() {
    for app in ["c8s", "d7s"] {
        let defined = actions(app);
        let text = source(app);
        let (_, names) = uses(&text);
        assert!(names.len() > 10, "{app}: scan found only {names:?}");
        for name in names {
            assert!(defined.contains(name), "{app}: '{name}' is not defined");
        }
    }
}

#[test]
fn every_screen_used_in_rust_is_defined() {
    for app in ["c8s", "d7s"] {
        let defined = table(app, app, "screens");
        let text = source(app);
        let (screens, _) = uses(&text);
        assert_ne!(screens.len(), 0);
        for screen in screens {
            assert!(defined.contains(screen), "{app}: no screen '{screen}'");
        }
        for call in text.split(".bar(\"").skip(1) {
            let screen = call.split('"').next().unwrap_or_default();
            assert!(defined.contains(screen), "{app}: no bar '{screen}'");
        }
    }
}

#[test]
fn every_defined_action_is_used() {
    let k9tui = source("k9tui");
    for app in ["c8s", "d7s"] {
        let text = format!("{}{k9tui}", source(app));
        for action in table(app, app, "keys") {
            assert!(
                text.contains(&format!("\"{action}\"")),
                "{app}: '{action}' has no use in Rust"
            );
        }
    }
    let apps = format!("{}{}{k9tui}", source("c8s"), source("d7s"));
    for action in table("k9tui", "t7s", "keys") {
        assert!(
            apps.contains(&format!("\"{action}\"")),
            "'{action}' has no use in Rust"
        );
    }
}
