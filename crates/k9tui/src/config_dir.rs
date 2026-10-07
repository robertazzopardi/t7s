use std::{
    io::{Error, ErrorKind},
    path::{Path, PathBuf},
};

/// Config files bigger than this are refused (a real skin/keymap is < 1 KiB).
const MAX_CONFIG_BYTES: u64 = 256 * 1024;

/// Resolve `~/.config/<app>`, the same on every platform (k9s-style),
/// rather than deferring to OS-specific config-dir conventions.
#[must_use]
pub fn config_dir(app: &str) -> Option<PathBuf> {
    directories::BaseDirs::new().map(|dirs| config_dir_in(dirs.home_dir(), app))
}

/// `<home>/.config/<app>`; split out so tests can inject a temp home.
#[must_use]
pub fn config_dir_in(home: &Path, app: &str) -> PathBuf {
    home.join(".config").join(app)
}

/// Read a config file as UTF-8 text with a size cap and any leading BOM
/// removed. `NotFound` is passed through so callers can treat it as "no
/// config".
///
/// # Errors
/// I/O errors, non-UTF-8 content, or a file over the size cap.
pub fn read_config(path: &Path) -> Result<String, Error> {
    if std::fs::metadata(path)?.len() > MAX_CONFIG_BYTES {
        return Err(Error::new(
            ErrorKind::InvalidData,
            format!("file larger than {MAX_CONFIG_BYTES} bytes"),
        ));
    }
    let text = std::fs::read_to_string(path)?;
    Ok(text.strip_prefix('\u{feff}').unwrap_or(&text).to_string())
}

/// One status-bar line for any number of warnings (full list goes to stderr
/// on exit), so a bad config can't flood the footer.
#[must_use]
pub fn summarize_warnings(warnings: &[String]) -> Option<String> {
    let (first, rest) = warnings.split_first()?;
    Some(if rest.is_empty() {
        format!("Config: {first}")
    } else {
        format!("Config: {first} (+{} more, see stderr on exit)", rest.len())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_dir_in_is_under_the_given_home() {
        let home = std::env::temp_dir().join("k9tui-fake-home");
        assert_eq!(
            config_dir_in(&home, "d7s"),
            home.join(".config").join("d7s")
        );
    }

    #[test]
    fn read_config_strips_bom_and_caps_size_and_rejects_dirs_and_binary() {
        let dir = std::env::temp_dir()
            .join(format!("k9tui-readcfg-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("a.yml");
        std::fs::write(&f, "\u{feff}a: b").unwrap();
        assert_eq!(read_config(&f).unwrap(), "a: b");
        std::fs::write(&f, vec![b'#'; 300 * 1024]).unwrap();
        assert_eq!(read_config(&f).unwrap_err().kind(), ErrorKind::InvalidData);
        std::fs::write(&f, [0xff, 0xfe, 0x00, 0x80]).unwrap();
        assert!(read_config(&f).is_err());
        assert!(read_config(&dir).is_err(), "directory is not a file");
        assert_eq!(
            read_config(&dir.join("missing")).unwrap_err().kind(),
            ErrorKind::NotFound
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn summary_is_one_line() {
        assert_eq!(summarize_warnings(&[]), None);
        let one = vec!["a".to_string()];
        assert_eq!(summarize_warnings(&one).as_deref(), Some("Config: a"));
        let many: Vec<String> = (0..50).map(|i| i.to_string()).collect();
        let s = summarize_warnings(&many).unwrap();
        assert!(s.contains("+49 more") && !s.contains("; "), "{s}");
    }
}
