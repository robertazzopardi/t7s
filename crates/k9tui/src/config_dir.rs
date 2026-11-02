use std::path::PathBuf;

/// Resolve `~/.config/<app>`, the same on every platform (k9s-style),
/// rather than deferring to OS-specific config-dir conventions.
#[must_use]
pub fn config_dir(app: &str) -> Option<PathBuf> {
    directories::BaseDirs::new()
        .map(|dirs| dirs.home_dir().join(".config").join(app))
}
