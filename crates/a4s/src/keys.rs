//! Encode crossterm key and paste events as bytes for an agent's PTY, and
//! track the kitty keyboard protocol mode the agent asks for.
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

const ESC: u8 = 0x1b;

/// `1 + (shift 1 | alt 2 | ctrl 4)`, the CSI modifier parameter.
const fn mod_param(m: KeyModifiers) -> u8 {
    let mut v = 0;
    if m.contains(KeyModifiers::SHIFT) {
        v |= 1;
    }
    if m.contains(KeyModifiers::ALT) {
        v |= 2;
    }
    if m.contains(KeyModifiers::CONTROL) {
        v |= 4;
    }
    v + 1
}

/// Keys sent as `CSI <num> ~` (final `~`) or `CSI 1 <final>` / `SS3 <final>`.
const fn nav(code: KeyCode) -> Option<(u8, char)> {
    Some(match code {
        KeyCode::Up => (1, 'A'),
        KeyCode::Down => (1, 'B'),
        KeyCode::Right => (1, 'C'),
        KeyCode::Left => (1, 'D'),
        KeyCode::Home => (1, 'H'),
        KeyCode::End => (1, 'F'),
        KeyCode::Insert => (2, '~'),
        KeyCode::Delete => (3, '~'),
        KeyCode::PageUp => (5, '~'),
        KeyCode::PageDown => (6, '~'),
        KeyCode::F(1) => (1, 'P'),
        KeyCode::F(2) => (1, 'Q'),
        KeyCode::F(3) => (1, 'R'),
        KeyCode::F(4) => (1, 'S'),
        KeyCode::F(5) => (15, '~'),
        KeyCode::F(6) => (17, '~'),
        KeyCode::F(7) => (18, '~'),
        KeyCode::F(8) => (19, '~'),
        KeyCode::F(9) => (20, '~'),
        KeyCode::F(10) => (21, '~'),
        KeyCode::F(11) => (23, '~'),
        KeyCode::F(12) => (24, '~'),
        KeyCode::F(_)
        | KeyCode::Backspace
        | KeyCode::Enter
        | KeyCode::Tab
        | KeyCode::BackTab
        | KeyCode::Char(_)
        | KeyCode::Null
        | KeyCode::Esc
        | KeyCode::CapsLock
        | KeyCode::ScrollLock
        | KeyCode::NumLock
        | KeyCode::PrintScreen
        | KeyCode::Pause
        | KeyCode::Menu
        | KeyCode::KeypadBegin
        | KeyCode::Media(_)
        | KeyCode::Modifier(_) => return None,
    })
}

/// Codepoint for keys that get `CSI <code> ; <mods> u` under kitty.
fn csi_u_code(code: KeyCode) -> Option<u32> {
    match code {
        KeyCode::Char(c) => Some(c.to_lowercase().next().unwrap_or(c) as u32),
        KeyCode::Enter => Some(13),
        KeyCode::Tab | KeyCode::BackTab => Some(9),
        KeyCode::Backspace => Some(127),
        KeyCode::Esc => Some(27),
        KeyCode::Left
        | KeyCode::Right
        | KeyCode::Up
        | KeyCode::Down
        | KeyCode::Home
        | KeyCode::End
        | KeyCode::PageUp
        | KeyCode::PageDown
        | KeyCode::Delete
        | KeyCode::Insert
        | KeyCode::F(_)
        | KeyCode::Null
        | KeyCode::CapsLock
        | KeyCode::ScrollLock
        | KeyCode::NumLock
        | KeyCode::PrintScreen
        | KeyCode::Pause
        | KeyCode::Menu
        | KeyCode::KeypadBegin
        | KeyCode::Media(_)
        | KeyCode::Modifier(_) => None,
    }
}

/// Kitty form for modified keys; `None` means fall back to legacy bytes.
fn kitty(key: KeyEvent) -> Option<Vec<u8>> {
    let mut m = key.modifiers;
    if matches!(key.code, KeyCode::Char(_))
        && !m.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
    {
        // Shift on plain text is already in the char; keep it with Ctrl/Alt.
        m.remove(KeyModifiers::SHIFT);
    } else if key.code == KeyCode::BackTab {
        m.insert(KeyModifiers::SHIFT);
    }
    let mods = mod_param(m);
    if mods == 1 {
        return None;
    }
    if let Some(code) = csi_u_code(key.code) {
        return Some(format!("\x1b[{code};{mods}u").into_bytes());
    }
    xterm_modified(key.code, mods)
}

/// xterm form `CSI <num> ; <mods> <final>` for a modified nav or F key.
fn xterm_modified(code: KeyCode, mods: u8) -> Option<Vec<u8>> {
    let (num, fin) = nav(code)?;
    Some(format!("\x1b[{num};{mods}{fin}").into_bytes())
}

const fn ctrl_punct(c: char) -> Option<u8> {
    match c {
        ' ' | '@' => Some(0x00),
        '[' => Some(0x1b),
        // Legacy terminals report 0x1C..0x1F as Ctrl+4..7.
        '\\' | '4' => Some(0x1c),
        ']' => Some(0x1d),
        '^' | '6' => Some(0x1e),
        '_' | '/' | '7' => Some(0x1f),
        _ => None,
    }
}

fn legacy(key: KeyEvent, app_cursor: bool) -> Vec<u8> {
    let mods = mod_param(key.modifiers);
    if mods != 1
        && let Some(bytes) = xterm_modified(key.code, mods)
    {
        return bytes;
    }
    let mut out = Vec::new();
    if key.modifiers.contains(KeyModifiers::ALT) {
        out.push(ESC);
    }
    match key.code {
        KeyCode::Char(c) => {
            let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
            if ctrl && c.is_ascii_alphabetic() {
                out.push(c.to_ascii_lowercase() as u8 - b'a' + 1);
            } else if let (true, Some(b)) = (ctrl, ctrl_punct(c)) {
                out.push(b);
            } else {
                let mut b = [0; 4];
                out.extend_from_slice(c.encode_utf8(&mut b).as_bytes());
            }
        }
        KeyCode::Enter => out.push(b'\r'),
        KeyCode::Tab => out.push(b'\t'),
        KeyCode::BackTab => out.extend_from_slice(b"\x1b[Z"),
        KeyCode::Backspace => out.push(0x7f),
        KeyCode::Esc => out.push(ESC),
        KeyCode::Left
        | KeyCode::Right
        | KeyCode::Up
        | KeyCode::Down
        | KeyCode::Home
        | KeyCode::End
        | KeyCode::PageUp
        | KeyCode::PageDown
        | KeyCode::Delete
        | KeyCode::Insert
        | KeyCode::F(_) => {
            if let Some((num, fin)) = nav(key.code) {
                let cursor_key = matches!(
                    key.code,
                    KeyCode::Up
                        | KeyCode::Down
                        | KeyCode::Left
                        | KeyCode::Right
                );
                if fin == '~' {
                    out.extend_from_slice(format!("\x1b[{num}~").as_bytes());
                } else if matches!(key.code, KeyCode::F(_))
                    || (cursor_key && app_cursor)
                {
                    out.extend_from_slice(format!("\x1bO{fin}").as_bytes());
                } else {
                    out.extend_from_slice(format!("\x1b[{fin}").as_bytes());
                }
            }
        }
        KeyCode::Null
        | KeyCode::CapsLock
        | KeyCode::ScrollLock
        | KeyCode::NumLock
        | KeyCode::PrintScreen
        | KeyCode::Pause
        | KeyCode::Menu
        | KeyCode::KeypadBegin
        | KeyCode::Media(_)
        | KeyCode::Modifier(_) => {}
    }
    out
}

/// Bytes to write to the PTY for `key`. `kind`/`state` are ignored.
#[must_use]
pub fn encode(key: KeyEvent, kitty_on: bool, app_cursor: bool) -> Vec<u8> {
    if kitty_on && let Some(bytes) = kitty(key) {
        return bytes;
    }
    legacy(key, app_cursor)
}

/// Wrap `text` in bracketed-paste markers only if the child enabled them.
#[must_use]
pub fn encode_paste(text: &str, bracketed: bool) -> Vec<u8> {
    if bracketed {
        let mut v = b"\x1b[200~".to_vec();
        v.extend_from_slice(text.as_bytes());
        v.extend_from_slice(b"\x1b[201~");
        v
    } else {
        text.as_bytes().to_vec()
    }
}

const STACK_CAP: usize = 16;
const CARRY_CAP: usize = 32;

/// Tracks `CSI > f u` (push), `CSI < n u` (pop) and `CSI = f ; m u` (set)
/// from the child's output. Enabled when the top-of-stack flags are non-zero.
#[derive(Debug, Default)]
pub struct KittyMode {
    stack: Vec<u32>,
    /// Partial escape sequence split across chunks.
    carry: Vec<u8>,
}

impl KittyMode {
    pub fn feed(&mut self, bytes: &[u8]) {
        for &b in bytes {
            if b == ESC {
                self.carry.clear();
                self.carry.push(b);
                continue;
            }
            if self.carry.is_empty() {
                continue;
            }
            self.carry.push(b);
            let ok = if self.carry.len() == 2 {
                b == b'['
            } else {
                matches!(b, 0x20..=0x7e)
            };
            if !ok || self.carry.len() > CARRY_CAP {
                self.carry.clear();
            } else if self.carry.len() > 2 && (0x40..=0x7e).contains(&b) {
                if b == b'u' {
                    self.apply();
                }
                self.carry.clear();
            }
        }
    }

    fn apply(&mut self) {
        let body = self
            .carry
            .get(2..self.carry.len().saturating_sub(1))
            .unwrap_or(&[]);
        let Some((&kind, rest)) = body.split_first() else {
            return;
        };
        let text = String::from_utf8_lossy(rest).into_owned();
        let mut parts = text.split(';');
        let first = parts.next().and_then(|s| s.parse::<u32>().ok());
        match kind {
            b'>' => {
                if self.stack.len() == STACK_CAP {
                    self.stack.remove(0);
                }
                self.stack.push(first.unwrap_or(0));
            }
            b'<' => {
                let n = first.unwrap_or(1).max(1) as usize;
                self.stack.truncate(self.stack.len().saturating_sub(n));
            }
            b'=' => {
                let flags = first.unwrap_or(0);
                let mode = parts.next().and_then(|s| s.parse().ok());
                if self.stack.is_empty() {
                    self.stack.push(0);
                }
                if let Some(top) = self.stack.last_mut() {
                    match mode.unwrap_or(1) {
                        2 => *top |= flags,
                        3 => *top &= !flags,
                        _ => *top = flags,
                    }
                }
            }
            _ => {}
        }
    }

    /// Top-of-stack flags, as reported to a `CSI ? u` query.
    #[must_use]
    pub fn flags(&self) -> u32 {
        self.stack.last().copied().unwrap_or(0)
    }

    #[must_use]
    pub fn enabled(&self) -> bool {
        self.flags() != 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn k(code: KeyCode, m: KeyModifiers) -> KeyEvent {
        KeyEvent::new(code, m)
    }

    #[test]
    fn legacy_encodings() {
        let n = KeyModifiers::NONE;
        let cases: Vec<(KeyEvent, bool, Vec<u8>)> = vec![
            (k(KeyCode::Char('a'), n), false, b"a".to_vec()),
            (k(KeyCode::Char('c'), KeyModifiers::CONTROL), false, vec![3]),
            (
                k(KeyCode::Char('x'), KeyModifiers::ALT),
                false,
                b"\x1bx".to_vec(),
            ),
            (
                k(KeyCode::Char('['), KeyModifiers::CONTROL),
                false,
                vec![0x1b],
            ),
            (
                k(KeyCode::Char('\\'), KeyModifiers::CONTROL),
                false,
                vec![0x1c],
            ),
            (
                k(KeyCode::Char(']'), KeyModifiers::CONTROL),
                false,
                vec![0x1d],
            ),
            (
                k(KeyCode::Char('^'), KeyModifiers::CONTROL),
                false,
                vec![0x1e],
            ),
            (
                k(KeyCode::Char('_'), KeyModifiers::CONTROL),
                false,
                vec![0x1f],
            ),
            (k(KeyCode::Char('@'), KeyModifiers::CONTROL), false, vec![0]),
            (k(KeyCode::Up, n), false, b"\x1b[A".to_vec()),
            (k(KeyCode::Up, n), true, b"\x1bOA".to_vec()),
            (k(KeyCode::Enter, n), false, b"\r".to_vec()),
            (k(KeyCode::F(5), n), false, b"\x1b[15~".to_vec()),
            (k(KeyCode::F(1), n), false, b"\x1bOP".to_vec()),
            (k(KeyCode::Home, n), true, b"\x1b[H".to_vec()),
            (k(KeyCode::Delete, n), true, b"\x1b[3~".to_vec()),
            (
                k(KeyCode::BackTab, KeyModifiers::SHIFT),
                false,
                b"\x1b[Z".to_vec(),
            ),
            (k(KeyCode::Char('é'), n), false, "é".as_bytes().to_vec()),
            (
                k(KeyCode::Char('/'), KeyModifiers::CONTROL),
                false,
                vec![0x1f],
            ),
            (
                k(KeyCode::Left, KeyModifiers::CONTROL),
                false,
                b"\x1b[1;5D".to_vec(),
            ),
            (
                k(KeyCode::Left, KeyModifiers::CONTROL),
                true,
                b"\x1b[1;5D".to_vec(),
            ),
            (
                k(KeyCode::Up, KeyModifiers::SHIFT),
                false,
                b"\x1b[1;2A".to_vec(),
            ),
            (
                k(KeyCode::Left, KeyModifiers::ALT),
                false,
                b"\x1b[1;3D".to_vec(),
            ),
            (
                k(KeyCode::Delete, KeyModifiers::CONTROL),
                false,
                b"\x1b[3;5~".to_vec(),
            ),
            (
                k(KeyCode::F(5), KeyModifiers::CONTROL),
                false,
                b"\x1b[15;5~".to_vec(),
            ),
            (
                k(KeyCode::F(1), KeyModifiers::SHIFT),
                false,
                b"\x1b[1;2P".to_vec(),
            ),
            (
                k(KeyCode::F(4), KeyModifiers::CONTROL),
                false,
                b"\x1b[1;5S".to_vec(),
            ),
        ];
        for (key, app, want) in cases {
            assert_eq!(encode(key, false, app), want, "{key:?}");
        }
    }

    #[test]
    fn legacy_ctrl_digits_are_punct_bytes() {
        let ctrl = |c| k(KeyCode::Char(c), KeyModifiers::CONTROL);
        assert_eq!(encode(ctrl('4'), false, false), vec![0x1c]);
        assert_eq!(encode(ctrl('6'), false, false), vec![0x1e]);
        assert_eq!(encode(ctrl('7'), false, false), vec![0x1f]);
    }

    #[test]
    fn kitty_shift_enter_is_distinct() {
        let e = k(KeyCode::Enter, KeyModifiers::SHIFT);
        assert_eq!(encode(e, true, false), b"\x1b[13;2u");
        assert_eq!(encode(e, false, false), b"\r");
    }

    #[test]
    fn kitty_other_forms() {
        let n = KeyModifiers::NONE;
        assert_eq!(encode(k(KeyCode::Enter, n), true, false), b"\r");
        assert_eq!(encode(k(KeyCode::Char('a'), n), true, false), b"a");
        assert_eq!(
            encode(k(KeyCode::Char('c'), KeyModifiers::CONTROL), true, false),
            b"\x1b[99;5u"
        );
        assert_eq!(
            encode(
                k(
                    KeyCode::Char('c'),
                    KeyModifiers::CONTROL | KeyModifiers::SHIFT
                ),
                true,
                false
            ),
            b"\x1b[99;6u"
        );
        assert_eq!(
            encode(k(KeyCode::Up, KeyModifiers::CONTROL), true, false),
            b"\x1b[1;5A"
        );
        assert_eq!(
            encode(k(KeyCode::Char('A'), KeyModifiers::SHIFT), true, false),
            b"A"
        );
    }

    #[test]
    fn paste_wrapping() {
        assert_eq!(encode_paste("a\nb", true), b"\x1b[200~a\nb\x1b[201~");
        assert_eq!(encode_paste("a\nb", false), b"a\nb");
    }

    #[test]
    fn kitty_mode_push_pop_and_split_chunks() {
        let mut m = KittyMode::default();
        m.feed(b"\x1b[>1");
        assert!(!m.enabled());
        m.feed(b"u");
        assert!(m.enabled());
        m.feed(b"\x1b[<u");
        assert!(!m.enabled());
        m.feed(b"\x1b[>1u\x1b[=0;1u");
        assert!(!m.enabled());
        m.feed(b"\x1b[=1;1u");
        assert!(m.enabled());
    }

    #[test]
    fn kitty_mode_bounded() {
        let mut m = KittyMode::default();
        m.feed(b"\x1b[>1");
        m.feed(&[b'0'; 1000]);
        m.feed(b"u");
        assert!(!m.enabled(), "garbage run must not complete a sequence");
        m.feed(b"\x1b[>1u");
        assert!(m.enabled(), "valid sequence after garbage still parses");
        m.feed(b"\x1b[<u");
        for _ in 0..100 {
            m.feed(b"\x1b[>1u");
        }
        assert_eq!(m.stack.len(), STACK_CAP);
    }
}
