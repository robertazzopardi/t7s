//! Every UI element takes its colour from the theme, so config can't miss one.
use std::{
    fs,
    path::{Path, PathBuf},
};

/// The only files allowed to name a colour.
const ALLOWED: [&str; 3] = [
    "k9tui/src/theme.rs",
    "k9tui/src/theme_source.rs",
    "k9tui/src/config.rs",
];

/// Ratatui's `Stylize` shorthand (`.red()`, `.on_red()`) also hard-codes a
/// colour.
const STYLIZE_COLOURS: [&str; 16] = [
    "black",
    "red",
    "green",
    "yellow",
    "blue",
    "magenta",
    "cyan",
    "gray",
    "dark_gray",
    "light_red",
    "light_green",
    "light_yellow",
    "light_blue",
    "light_magenta",
    "light_cyan",
    "white",
];

/// Other ways to name a colour: ratatui's `palette::tailwind` constants, an
/// aliased `Color`, and bare `Rgb`/`Indexed` after `use Color::*`.
const COLOUR_PATTERNS: [&str; 5] =
    ["Color::", "palette::", "Color as", "Rgb(", "Indexed("];

fn rust_files(dir: &Path, found: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            rust_files(&path, found);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            found.push(path);
        }
    }
}

/// Line number and text of each hard-coded colour outside test items.
fn colours_in(text: &str) -> Vec<(usize, &str)> {
    let shorthand: Vec<_> = STYLIZE_COLOURS
        .iter()
        .flat_map(|c| [format!(".{c}()"), format!(".on_{c}()")])
        .collect();
    let mut hits = Vec::new();
    let mut test_item_indent = None;
    for (number, line) in text.lines().enumerate() {
        let trimmed = line.trim_start();
        let indent = line.len() - trimmed.len();
        if let Some(item_indent) = test_item_indent {
            // rustfmt puts an item's last line at the attribute's indent.
            if indent == item_indent
                && (trimmed.starts_with('}')
                    || trimmed.ends_with(';')
                    || trimmed.ends_with(','))
            {
                test_item_indent = None;
            }
        } else if trimmed.trim_end() == "#[cfg(test)]" {
            test_item_indent = Some(indent);
        } else if COLOUR_PATTERNS.iter().any(|pattern| line.contains(pattern))
            || shorthand.iter().any(|pattern| line.contains(pattern))
        {
            hits.push((number + 1, line.trim()));
        }
    }
    hits
}

#[test]
fn checker_covers_all_colours_and_skips_only_test_items() {
    let text = "\
#[cfg(test)]
use x::Color::Red;
let a = b.gray();
#[cfg(test)]
mod tests {
    fn t() { c.red(); }
}
let d = e.on_light_blue();
self.on_key();
use ratatui::style::palette::tailwind;
use ratatui::style::Color as C;
let f = Rgb(1, 2, 3);
let g = Indexed(4);
let h = theme_source::palette(name);
";
    let lines: Vec<_> = colours_in(text).iter().map(|hit| hit.0).collect();
    assert_eq!(lines, [3, 8, 10, 11, 12, 13]);
}

#[test]
fn no_hard_coded_colours_outside_the_theme() {
    let crates = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let mut files = Vec::new();
    for entry in fs::read_dir(crates).unwrap() {
        let src = entry.unwrap().path().join("src");
        if src.is_dir() {
            rust_files(&src, &mut files);
        }
    }
    assert!(files.len() > 20, "found only {} source files", files.len());

    let mut hits = Vec::new();
    for file in files {
        let relative = file.strip_prefix(crates).unwrap();
        if ALLOWED.iter().any(|allowed| relative == Path::new(allowed)) {
            continue;
        }
        let text = fs::read_to_string(&file).unwrap();
        for (at, line) in colours_in(&text) {
            hits.push(format!("{}:{at}: {line}", relative.display()));
        }
    }
    assert!(
        hits.is_empty(),
        "{} hard-coded colours, use k9tui::theme:\n{}",
        hits.len(),
        hits.join("\n")
    );
}
