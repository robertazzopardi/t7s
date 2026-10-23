use core::mem::take;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use k9tui::widgets::{modal::DialogAction, navigation::TableNavigationHandler};

use crate::{
    app::App,
    app_state::AppState,
    command::suggest,
    proc::{SortKey, signals},
};

impl App {
    pub fn on_key_event(&mut self, key: KeyEvent) {
        match &self.state {
            AppState::List => self.on_key_list(key),
            AppState::Info(_) => self.on_key_info(key),
            AppState::Describe { .. } => self.on_key_describe(key),
            AppState::Signal { .. } => self.on_key_signal(key),
        }
    }

    #[allow(clippy::wildcard_enum_match_arm)]
    fn on_key_list(&mut self, key: KeyEvent) {
        if self.on_key_filter_or_quit(key) {
            return;
        }
        match (key.modifiers, key.code) {
            (_, KeyCode::Char('c')) => self.set_sort(SortKey::Cpu),
            (_, KeyCode::Char('m')) => self.set_sort(SortKey::Mem),
            (_, KeyCode::Char('p')) => self.set_sort(SortKey::Pid),
            (_, KeyCode::Char('n')) => self.set_sort(SortKey::Name),
            (_, KeyCode::Char('d') | KeyCode::Enter) => self.open_describe(),
            (_, KeyCode::Char('i')) => self.open_info(),
            (_, KeyCode::Char('K') | KeyCode::Delete) => {
                self.open_signal_picker();
            }
            (
                _,
                code @ (KeyCode::Char('j' | 'k' | 'g' | 'G')
                | KeyCode::Down
                | KeyCode::Up),
            ) => self.navigate(code),
            _ => {}
        }
    }

    fn on_key_info(&mut self, key: KeyEvent) {
        match (key.modifiers, key.code) {
            (_, KeyCode::Char('q' | 'i') | KeyCode::Esc) => {
                self.state = AppState::List;
            }
            (KeyModifiers::CONTROL, KeyCode::Char('c' | 'C')) => self.quit(),
            _ => {}
        }
    }

    #[allow(clippy::wildcard_enum_match_arm)]
    fn on_key_describe(&mut self, key: KeyEvent) {
        let total_lines = match &self.state {
            AppState::Describe { text, .. } => text.lines().count(),
            _ => return,
        };
        let max_start =
            total_lines.saturating_sub(self.describe_viewport_height);
        match (key.modifiers, key.code) {
            (_, KeyCode::Char('q') | KeyCode::Esc) => {
                self.state = AppState::List;
            }
            (KeyModifiers::CONTROL, KeyCode::Char('c' | 'C')) => self.quit(),
            (_, KeyCode::Char('k') | KeyCode::Up) => {
                self.describe_scroll = self.describe_scroll.saturating_sub(1);
            }
            (_, KeyCode::Char('j') | KeyCode::Down) => {
                self.describe_scroll =
                    (self.describe_scroll + 1).min(max_start);
            }
            (_, KeyCode::PageUp) => {
                self.describe_scroll = self.describe_scroll.saturating_sub(10);
            }
            (_, KeyCode::PageDown) => {
                self.describe_scroll =
                    (self.describe_scroll + 10).min(max_start);
            }
            (_, KeyCode::Char('g') | KeyCode::Home) => self.describe_scroll = 0,
            (_, KeyCode::Char('G') | KeyCode::End) => {
                self.describe_scroll = max_start;
            }
            _ => {}
        }
    }

    #[allow(clippy::wildcard_enum_match_arm)]
    fn on_key_signal(&mut self, key: KeyEvent) {
        if let Some(dialog) = self.confirm_dialog.as_mut() {
            match dialog.handle_key_events(key) {
                DialogAction::Submit => {
                    self.confirm_dialog = None;
                    if let Some((pid, start, sig)) = self.pending_signal.take()
                    {
                        self.send_signal(pid, start, sig);
                    }
                    self.state = AppState::List;
                }
                DialogAction::Cancel => {
                    self.confirm_dialog = None;
                    self.pending_signal = None;
                }
                DialogAction::None => {}
            }
            return;
        }

        let (pid, name, start_time, index) = match &self.state {
            AppState::Signal {
                pid,
                name,
                start_time,
                index,
            } => (*pid, name.clone(), *start_time, *index),
            _ => return,
        };
        let menu = signals::menu();
        let Some(len) = (!menu.is_empty()).then_some(menu.len()) else {
            self.state = AppState::List;
            return;
        };
        match (key.modifiers, key.code) {
            (_, KeyCode::Char('q') | KeyCode::Esc) => {
                self.state = AppState::List;
            }
            (KeyModifiers::CONTROL, KeyCode::Char('c' | 'C')) => self.quit(),
            (_, KeyCode::Char('j') | KeyCode::Down) => {
                self.set_signal_index((index + 1) % len);
            }
            (_, KeyCode::Char('k') | KeyCode::Up) => {
                self.set_signal_index(if index == 0 {
                    len - 1
                } else {
                    index - 1
                });
            }
            (_, KeyCode::Char('g') | KeyCode::Home) => self.set_signal_index(0),
            (_, KeyCode::Char('G') | KeyCode::End) => {
                self.set_signal_index(len - 1);
            }
            (_, KeyCode::Enter) => {
                if let Some(sig) = menu.get(index).copied() {
                    self.prompt_signal(pid, &name, start_time, sig);
                }
            }
            _ => {}
        }
    }

    /// Search-bar editing, `q` (clear filter, else quit), Ctrl-C and `/`.
    /// Returns true when the key was consumed.
    fn on_key_filter_or_quit(&mut self, key: KeyEvent) -> bool {
        if self.search_open {
            self.on_key_list_search(key);
            return true;
        }
        match (key.modifiers, key.code) {
            (_, KeyCode::Char('q')) if self.filter.is_empty() => {
                self.quit();
            }
            (_, KeyCode::Char('q')) => {
                self.filter.clear();
                self.reapply_filter();
            }
            (KeyModifiers::CONTROL, KeyCode::Char('c' | 'C')) => {
                self.quit();
            }
            (_, KeyCode::Char('/')) => self.search_open = true,
            (_, KeyCode::Char(':')) => {
                self.search_open = true;
                self.command_mode = true;
                self.command_text.clear();
            }
            _ => return false,
        }
        true
    }

    #[allow(clippy::wildcard_enum_match_arm)]
    fn on_key_list_search(&mut self, key: KeyEvent) {
        if self.command_mode {
            self.on_key_command(key);
            return;
        }
        match (key.modifiers, key.code) {
            (_, KeyCode::Esc) => {
                self.filter.clear();
                self.search_open = false;
                self.reapply_filter();
            }
            (_, KeyCode::Enter) => self.search_open = false,
            (_, KeyCode::Backspace) => {
                self.filter.pop();
                self.reapply_filter();
            }
            (KeyModifiers::CONTROL, KeyCode::Char('c' | 'C')) => self.quit(),
            (_, KeyCode::Char(c)) => {
                self.filter.push(c);
                self.reapply_filter();
            }
            _ => {}
        }
    }

    /// `:` command bar editing: Enter runs, Esc cancels, Tab completes.
    #[allow(clippy::wildcard_enum_match_arm)]
    fn on_key_command(&mut self, key: KeyEvent) {
        match (key.modifiers, key.code) {
            (KeyModifiers::CONTROL, KeyCode::Char('c' | 'C')) => self.quit(),
            (_, KeyCode::Esc) => self.close_command_bar(),
            (_, KeyCode::Enter) => {
                let text = take(&mut self.command_text);
                self.close_command_bar();
                self.run_command(&text);
            }
            (_, KeyCode::Tab) => {
                if let Some(s) = suggest(&self.command_text) {
                    self.command_text = s.to_string();
                }
            }
            (_, KeyCode::Backspace) => {
                self.command_text.pop();
            }
            (m, KeyCode::Char(c))
                if !m.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                self.command_text.push(c);
            }
            _ => {}
        }
    }

    fn navigate(&mut self, key: KeyCode) {
        TableNavigationHandler::navigate_table(
            &self.processes.model,
            &mut self.processes.view,
            key,
        );
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use ratatui::{Terminal, backend::TestBackend, buffer::Cell};

    use super::*;
    use crate::{
        app::App,
        proc::{
            ProcessRow,
            monitor::{DemoSource, demo_snapshot},
        },
    };

    fn app() -> App {
        let mut app = App::new(Arc::new(DemoSource::new()));
        app.apply_snapshot(demo_snapshot());
        app
    }

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn screen(app: &mut App) -> String {
        let mut terminal = Terminal::new(TestBackend::new(120, 30)).unwrap();
        terminal.draw(|f| app.render(f)).unwrap();
        let buf = terminal.backend().buffer();
        buf.content
            .chunks(usize::from(buf.area.width))
            .map(|row| row.iter().map(Cell::symbol).collect::<String>())
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn sort_keys_reorder_and_toggle_direction() {
        let mut app = app();
        assert_eq!(app.sort_key, SortKey::Cpu);
        assert!(app.sort_desc, "cpu defaults to descending");

        app.on_key_event(key(KeyCode::Char('p')));
        assert_eq!(app.sort_key, SortKey::Pid);
        assert!(!app.sort_desc, "pid defaults to ascending");
        // Rows are ascending by pid.
        let pids: Vec<i32> =
            app.processes.model.items.iter().map(|r| r.pid).collect();
        assert_eq!(pids, vec![1, 82, 320, 1024, 1025, 2048, 4096, 5000]);

        app.on_key_event(key(KeyCode::Char('p')));
        assert!(app.sort_desc, "pressing the active key flips direction");
    }

    #[test]
    fn selection_follows_pid_across_refresh() {
        let mut app = app();
        // Select the highest-CPU row (pid 4096) in the default cpu-desc view.
        assert_eq!(app.selected_row().map(|r| r.pid), Some(4096));

        let mut snap = demo_snapshot();
        snap.rows.reverse();
        snap.rows.push(ProcessRow {
            pid: 6000,
            name: "extra".to_string(),
            ..ProcessRow::default()
        });
        app.apply_snapshot(snap);

        assert_eq!(
            app.selected_row().map(|r| r.pid),
            Some(4096),
            "cursor must stay on the same PID, not the same row index"
        );
    }

    #[test]
    fn navigation_wraps() {
        let mut app = app();
        app.on_key_event(key(KeyCode::Char('p')));
        app.processes.view.state.select(Some(0));
        app.on_key_event(key(KeyCode::Char('k')));
        assert_eq!(
            app.processes.view.state.selected(),
            Some(app.processes.model.items.len() - 1)
        );
    }

    #[test]
    fn describe_opens_and_closes() {
        let mut app = app();
        app.on_key_event(key(KeyCode::Char('d')));
        assert!(matches!(&app.state, AppState::Describe { .. }));
        assert!(screen(&mut app).contains("Describe:"));
        app.on_key_event(key(KeyCode::Esc));
        assert_eq!(app.state, AppState::List);
    }

    #[test]
    fn info_opens_and_closes() {
        let mut app = app();
        app.on_key_event(key(KeyCode::Char('i')));
        assert!(matches!(&app.state, AppState::Info(_)));
        let out = screen(&mut app);
        assert!(out.contains(" System "), "{out}");
        assert!(out.contains("Load avg:"), "{out}");
        app.on_key_event(key(KeyCode::Char('q')));
        assert_eq!(app.state, AppState::List);
    }

    #[test]
    fn signal_flow_confirms_and_reports() {
        let mut app = app();
        app.on_key_event(key(KeyCode::Char('K')));
        assert!(matches!(&app.state, AppState::Signal { .. }));
        assert!(screen(&mut app).contains("Signal:"));

        app.on_key_event(key(KeyCode::Enter));
        assert!(app.confirm_dialog.is_some(), "picker opens a confirm");
        assert!(app.pending_signal.is_some());

        // Default is No: move to Yes, then Enter sends.
        app.on_key_event(key(KeyCode::Left));
        app.on_key_event(key(KeyCode::Enter));
        assert!(app.confirm_dialog.is_none());
        assert!(app.pending_signal.is_none());
        assert_eq!(app.state, AppState::List);
        assert!(screen(&mut app).contains("Sent"));
    }

    #[test]
    fn signal_picker_default_no_does_not_send() {
        let mut app = app();
        app.on_key_event(key(KeyCode::Delete));
        app.on_key_event(key(KeyCode::Enter)); // open confirm
        app.on_key_event(key(KeyCode::Enter)); // default No
        assert!(app.pending_signal.is_none());
        let out = screen(&mut app);
        assert!(!out.contains("Sent"), "{out}");
    }

    #[test]
    fn signal_picker_esc_cancels_back_to_list() {
        let mut app = app();
        app.on_key_event(key(KeyCode::Char('K')));
        app.on_key_event(key(KeyCode::Esc));
        assert_eq!(app.state, AppState::List);
        assert!(app.confirm_dialog.is_none());
    }

    #[test]
    fn filter_narrows_and_q_clears_before_quit() {
        let mut app = app();
        app.running = true;
        app.on_key_event(key(KeyCode::Char('/')));
        for c in "nginx".chars() {
            app.on_key_event(key(KeyCode::Char(c)));
        }
        assert_eq!(app.filter, "nginx");
        assert!(
            app.processes
                .model
                .items
                .iter()
                .all(|r| r.cmdline.to_lowercase().contains("nginx"))
        );
        app.on_key_event(key(KeyCode::Enter));
        app.on_key_event(key(KeyCode::Char('q')));
        assert!(app.running, "q clears the filter instead of quitting");
        assert_eq!(app.filter, "");
        assert_eq!(app.processes.model.items.len(), 8);
    }

    #[test]
    fn command_bar_resolves_sort_and_quit() {
        let mut app = app();
        app.running = true;
        app.on_key_event(key(KeyCode::Char(':')));
        for c in "mem".chars() {
            app.on_key_event(key(KeyCode::Char(c)));
        }
        app.on_key_event(key(KeyCode::Enter));
        assert_eq!(app.sort_key, SortKey::Mem);

        app.on_key_event(key(KeyCode::Char(':')));
        app.on_key_event(key(KeyCode::Char('q')));
        app.on_key_event(key(KeyCode::Enter));
        assert!(!app.running);
    }
}
