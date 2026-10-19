use core::mem::take;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use k9tui::{
    theme,
    widgets::{
        modal::{ConfirmDialog, DialogAction},
        navigation::TableNavigationHandler,
    },
};
use ratatui::{Terminal, backend::Backend, style::Style};

use crate::{
    app::App,
    app_state::{AppState, ResourceKind},
    command::{Command, resolve, suggest},
    error::Result,
    ui::widgets::hotkeys::{LIST_HOTKEYS, RESOURCE_HOTKEYS},
};

impl App {
    pub async fn on_key_event<B: Backend>(
        &mut self,
        key: KeyEvent,
        terminal: &mut Terminal<B>,
    ) -> Result<()> {
        match &self.state {
            AppState::ConnectError(_) => self.on_key_connect_error(key).await,
            AppState::Connecting => Ok(()),
            AppState::List => self.on_key_list(key, terminal).await,
            AppState::Logs { .. } => {
                self.on_key_logs(key);
                Ok(())
            }
            AppState::Info(_) => {
                self.on_key_info(key);
                Ok(())
            }
            AppState::Describe { .. } => {
                self.on_key_describe(key);
                Ok(())
            }
            AppState::Processes { .. } => {
                self.on_key_processes(key);
                Ok(())
            }
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
    fn on_key_processes(&mut self, key: KeyEvent) {
        match (key.modifiers, key.code) {
            (_, KeyCode::Char('q') | KeyCode::Esc) => self.close_processes(),
            (KeyModifiers::CONTROL, KeyCode::Char('c' | 'C')) => self.quit(),
            (_, KeyCode::Char('l')) => self.open_process_output(),
            (_, KeyCode::Char('j')) => self.navigate_processes(KeyCode::Down),
            (_, KeyCode::Char('k')) => self.navigate_processes(KeyCode::Up),
            (
                _,
                code @ (KeyCode::Down | KeyCode::Up | KeyCode::Char('g' | 'G')),
            ) => {
                self.navigate_processes(code);
            }
            _ => {}
        }
    }

    fn navigate_processes(&mut self, key: KeyCode) {
        TableNavigationHandler::navigate_table(
            &self.processes.model,
            &mut self.processes.view,
            key,
        );
    }

    fn on_key_describe(&mut self, key: KeyEvent) {
        let AppState::Describe { text, .. } = &self.state else {
            return;
        };
        let total_lines = text.lines().count();
        let max_start =
            total_lines.saturating_sub(self.describe_viewport_height);
        match (key.modifiers, key.code) {
            (_, KeyCode::Char('q') | KeyCode::Esc) => self.close_describe(),
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

    async fn on_key_connect_error(&mut self, key: KeyEvent) -> Result<()> {
        match (key.modifiers, key.code) {
            (_, KeyCode::Char('r') | KeyCode::Enter) => {
                self.connect();
            }
            (_, KeyCode::Char('q'))
            | (KeyModifiers::CONTROL, KeyCode::Char('c' | 'C')) => {
                self.quit();
            }
            _ => {}
        }
        Ok(())
    }

    #[allow(clippy::wildcard_enum_match_arm)]
    fn on_key_logs(&mut self, key: KeyEvent) {
        if self.log_search_open {
            match key.code {
                KeyCode::Esc => {
                    self.log_filter.clear();
                    self.log_search_open = false;
                }
                KeyCode::Enter => self.log_search_open = false,
                KeyCode::Backspace => {
                    self.log_filter.pop();
                }
                KeyCode::Char(c) => self.log_filter.push(c),
                _ => {}
            }
            self.log_scroll = 0;
            self.log_follow = false;
            return;
        }

        let max_start =
            self.log_view_len.saturating_sub(self.log_viewport_height);
        match (key.modifiers, key.code) {
            (_, KeyCode::Char('q') | KeyCode::Esc)
                if self.log_filter.is_empty() =>
            {
                self.close_logs();
            }
            (_, KeyCode::Char('q') | KeyCode::Esc) => self.log_filter.clear(),
            (_, KeyCode::Char('/')) => self.log_search_open = true,
            (KeyModifiers::NONE, KeyCode::Char('d')) => {
                self.log_dedup = self.log_dedup.next();
            }
            (KeyModifiers::CONTROL, KeyCode::Char('c' | 'C')) => self.quit(),
            (_, KeyCode::Char('k') | KeyCode::Up) => {
                self.log_follow = false;
                self.log_scroll = self.log_scroll.saturating_sub(1);
            }
            (_, KeyCode::Char('j') | KeyCode::Down) => {
                self.log_scroll = (self.log_scroll + 1).min(max_start);
                self.log_follow = self.log_scroll >= max_start;
            }
            (KeyModifiers::CONTROL, KeyCode::Char('u'))
            | (_, KeyCode::PageUp) => {
                self.log_follow = false;
                self.log_scroll = self.log_scroll.saturating_sub(10);
            }
            (KeyModifiers::CONTROL, KeyCode::Char('d'))
            | (_, KeyCode::PageDown) => {
                self.log_scroll = (self.log_scroll + 10).min(max_start);
                self.log_follow = self.log_scroll >= max_start;
            }
            (_, KeyCode::Char('g') | KeyCode::Home) => {
                self.log_follow = false;
                self.log_scroll = 0;
            }
            (_, KeyCode::Char('G') | KeyCode::End) => {
                self.log_follow = true;
            }
            _ => {}
        }
    }

    #[allow(clippy::wildcard_enum_match_arm)]
    fn on_key_list_search(&mut self, key: KeyEvent) {
        if self.command_mode {
            self.on_key_command(key);
            return;
        }
        match (key.modifiers, key.code) {
            (_, KeyCode::Esc) => {
                self.list_filter.clear();
                self.list_search_open = false;
                self.reapply_list_filter();
            }
            (_, KeyCode::Enter) => self.list_search_open = false,
            (_, KeyCode::Backspace) => {
                self.list_filter.pop();
                self.reapply_list_filter();
            }
            (KeyModifiers::CONTROL, KeyCode::Char('c' | 'C')) => self.quit(),
            (_, KeyCode::Char(c)) => {
                self.list_filter.push(c);
                self.reapply_list_filter();
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

    fn close_command_bar(&mut self) {
        self.command_mode = false;
        self.list_search_open = false;
        self.command_text.clear();
    }

    fn run_command(&mut self, text: &str) {
        match resolve(text) {
            Command::Empty => {}
            Command::View(kind) => self.switch_view(kind),
            Command::Quit => self.quit(),
            Command::Unknown => {
                self.set_status(format!("Unknown command: {}", text.trim()));
            }
        }
    }

    /// Search-bar editing, `q` (clear filter, else quit), Ctrl-C and `/`.
    /// Returns true when the key was consumed. Terminal-free so it is testable.
    fn on_key_list_filter_or_quit(&mut self, key: KeyEvent) -> bool {
        if self.list_search_open {
            self.on_key_list_search(key);
            return true;
        }
        match (key.modifiers, key.code) {
            (_, KeyCode::Char('q')) if self.list_filter.is_empty() => {
                self.quit();
            }
            (_, KeyCode::Char('q')) => {
                self.list_filter.clear();
                self.reapply_list_filter();
            }
            (KeyModifiers::CONTROL, KeyCode::Char('c' | 'C')) => {
                self.quit();
            }
            (_, KeyCode::Char('/')) => self.list_search_open = true,
            (_, KeyCode::Char(':')) => {
                self.list_search_open = true;
                self.command_mode = true;
                self.command_text.clear();
            }
            _ => return false,
        }
        true
    }

    #[allow(clippy::wildcard_enum_match_arm)]
    #[allow(clippy::too_many_lines)] // flat key-dispatch match
    async fn on_key_list<B: Backend>(
        &mut self,
        key: KeyEvent,
        terminal: &mut Terminal<B>,
    ) -> Result<()> {
        if let Some(dialog) = self.confirm_dialog.as_mut() {
            match dialog.handle_key_events(key) {
                DialogAction::Submit => {
                    self.confirm_dialog = None;
                    if let Some((kind, id)) = self.pending_remove.take() {
                        self.remove_resource(kind, &id);
                    }
                }
                DialogAction::Cancel => {
                    self.confirm_dialog = None;
                    self.pending_remove = None;
                }
                DialogAction::None => {}
            }
            return Ok(());
        }

        if self.on_key_list_filter_or_quit(key) {
            return Ok(());
        }

        match (key.modifiers, key.code) {
            (_, KeyCode::Char('1')) => {
                self.switch_view(ResourceKind::Containers);
            }
            (_, KeyCode::Char('2')) => self.switch_view(ResourceKind::Images),
            (_, KeyCode::Char('3')) => self.switch_view(ResourceKind::Volumes),
            (_, KeyCode::Char('4')) => self.switch_view(ResourceKind::Networks),
            (_, KeyCode::Char('j') | KeyCode::Down) => {
                self.navigate(KeyCode::Down);
            }
            (_, KeyCode::Char('k') | KeyCode::Up) => self.navigate(KeyCode::Up),
            (_, KeyCode::Char('g')) => self.navigate(KeyCode::Char('g')),
            (_, KeyCode::Char('G')) => self.navigate(KeyCode::Char('G')),
            (_, KeyCode::Char('a'))
                if self.view == ResourceKind::Containers =>
            {
                self.running_only = !self.running_only;
                self.reapply_list_filter();
            }
            (_, KeyCode::Char('s' | 'S'))
                if self.view == ResourceKind::Containers =>
            {
                if let Some(row) = self.selected_container().cloned() {
                    if row.is_running() {
                        self.stop_container(&row.id);
                    } else {
                        self.start_container(&row.id);
                    }
                }
            }
            (_, KeyCode::Char('r'))
                if self.view == ResourceKind::Containers =>
            {
                if let Some(row) = self.selected_container().cloned() {
                    self.restart_container(&row.id);
                }
            }
            (_, KeyCode::Char('p'))
                if self.view == ResourceKind::Containers =>
            {
                if let Some(row) = self.selected_container().cloned() {
                    self.open_processes(&row.id, &row.name);
                }
            }
            (_, KeyCode::Char('l'))
                if self.view == ResourceKind::Containers =>
            {
                if let Some(row) = self.selected_container().cloned() {
                    self.open_logs(&row.id, &row.name);
                }
            }
            (_, KeyCode::Char('d') | KeyCode::Enter) => {
                self.describe_selected();
            }
            (_, KeyCode::Char('e'))
                if self.view == ResourceKind::Containers =>
            {
                if let Some(row) = self.selected_container().cloned() {
                    self.exec_shell(terminal, &row.id)?;
                }
            }
            (_, KeyCode::Char('i')) => {
                self.open_daemon_health();
            }
            (_, KeyCode::Char('D') | KeyCode::Delete) => {
                self.prompt_remove_selected();
            }
            _ => {}
        }
        Ok(())
    }

    /// Switch the active list view. Selection state per resource kind is
    /// preserved on its own `TableDataState`, so no reset is needed here.
    fn switch_view(&mut self, kind: ResourceKind) {
        self.view = kind;
        self.hotkeys = match kind {
            ResourceKind::Containers => LIST_HOTKEYS.to_vec(),
            ResourceKind::Images
            | ResourceKind::Volumes
            | ResourceKind::Networks => RESOURCE_HOTKEYS.to_vec(),
        };
    }

    fn navigate(&mut self, key: KeyCode) {
        match self.view {
            ResourceKind::Containers => TableNavigationHandler::navigate_table(
                &self.containers.model,
                &mut self.containers.view,
                key,
            ),
            ResourceKind::Images => TableNavigationHandler::navigate_table(
                &self.images.model,
                &mut self.images.view,
                key,
            ),
            ResourceKind::Volumes => TableNavigationHandler::navigate_table(
                &self.volumes.model,
                &mut self.volumes.view,
                key,
            ),
            ResourceKind::Networks => TableNavigationHandler::navigate_table(
                &self.networks.model,
                &mut self.networks.view,
                key,
            ),
        }
    }

    /// `d`/`Enter`: describe whatever is selected in the active view.
    fn describe_selected(&mut self) {
        let Some((id, name)) = self.selected_target() else {
            return;
        };
        self.open_describe(self.view, &id, &name);
    }

    /// (docker id-or-name, display label) of the selection in the active view.
    fn selected_target(&self) -> Option<(String, String)> {
        match self.view {
            ResourceKind::Containers => self
                .selected_container()
                .map(|r| (r.id.clone(), r.name.clone())),
            ResourceKind::Images => self
                .selected_image()
                .map(|r| (r.id.clone(), r.repo_tags.clone())),
            ResourceKind::Volumes => self
                .selected_volume()
                .map(|r| (r.name.clone(), r.name.clone())),
            ResourceKind::Networks => self
                .selected_network()
                .map(|r| (r.id.clone(), r.name.clone())),
        }
    }

    /// Open the remove-confirmation dialog for the selection, if any.
    fn prompt_remove_selected(&mut self) {
        let kind = self.view;
        let Some((id, label)) = self.selected_target() else {
            return;
        };

        self.pending_remove = Some((kind, id));
        self.confirm_dialog = Some(ConfirmDialog::new(
            format!(" Remove {}? ", singular(kind)),
            format!(
                "Remove {} '{label}'?\nThis action cannot be undone.",
                singular(kind)
            ),
            modal_border(),
            1, // default to No (k9s-style)
        ));
    }

    fn start_container(&mut self, id: &str) {
        use crate::{app::BackgroundEvent, in_flight::container_key};

        let key = container_key(id);
        if !self.in_flight.try_begin(key.clone()) {
            self.set_status("Already busy with this container");
            return;
        }
        let Some(docker) = self.docker.clone() else {
            self.in_flight.end(&key);
            return;
        };
        self.set_status(format!("Starting {id}…"));
        let tx = self.bg_tx.clone();
        let id = id.to_string();
        tokio::spawn(async move {
            let event = match docker.start(&id).await {
                Ok(()) => BackgroundEvent::ActionDone {
                    key,
                    message: "Container started".into(),
                },
                Err(e) => BackgroundEvent::ActionFailed {
                    key,
                    message: format!("Start failed: {e}"),
                },
            };
            let _ = tx.send(event);
        });
    }

    fn stop_container(&mut self, id: &str) {
        use crate::{app::BackgroundEvent, in_flight::container_key};

        let key = container_key(id);
        if !self.in_flight.try_begin(key.clone()) {
            self.set_status("Already busy with this container");
            return;
        }
        let Some(docker) = self.docker.clone() else {
            self.in_flight.end(&key);
            return;
        };
        self.set_status(format!("Stopping {id}…"));
        let tx = self.bg_tx.clone();
        let id = id.to_string();
        tokio::spawn(async move {
            let event = match docker.stop(&id).await {
                Ok(()) => BackgroundEvent::ActionDone {
                    key,
                    message: "Container stopped".into(),
                },
                Err(e) => BackgroundEvent::ActionFailed {
                    key,
                    message: format!("Stop failed: {e}"),
                },
            };
            let _ = tx.send(event);
        });
    }

    fn restart_container(&mut self, id: &str) {
        use crate::{app::BackgroundEvent, in_flight::container_key};

        let key = container_key(id);
        if !self.in_flight.try_begin(key.clone()) {
            self.set_status("Already busy with this container");
            return;
        }
        let Some(docker) = self.docker.clone() else {
            self.in_flight.end(&key);
            return;
        };
        self.set_status(format!("Restarting {id}…"));
        let tx = self.bg_tx.clone();
        let id = id.to_string();
        tokio::spawn(async move {
            let event = match docker.restart(&id).await {
                Ok(()) => BackgroundEvent::ActionDone {
                    key,
                    message: "Container restarted".into(),
                },
                Err(e) => BackgroundEvent::ActionFailed {
                    key,
                    message: format!("Restart failed: {e}"),
                },
            };
            let _ = tx.send(event);
        });
    }

    fn remove_resource(&mut self, kind: ResourceKind, id: &str) {
        use crate::{app::BackgroundEvent, in_flight::resource_key};

        let key = resource_key(kind, id);
        if !self.in_flight.try_begin(key.clone()) {
            self.set_status(format!(
                "Already busy with this {}",
                singular(kind).to_lowercase()
            ));
            return;
        }
        let Some(docker) = self.docker.clone() else {
            self.in_flight.end(&key);
            return;
        };
        self.set_status(format!("Removing {id}…"));
        let tx = self.bg_tx.clone();
        let id = id.to_string();
        tokio::spawn(async move {
            let result = match kind {
                ResourceKind::Containers => docker.remove(&id).await,
                ResourceKind::Images => docker.remove_image(&id).await,
                ResourceKind::Volumes => docker.remove_volume(&id).await,
                ResourceKind::Networks => docker.remove_network(&id).await,
            };
            let event = match result {
                Ok(()) => BackgroundEvent::ActionDone {
                    key,
                    message: format!("{} removed", singular(kind)),
                },
                Err(e) => BackgroundEvent::ActionFailed {
                    key,
                    message: format!("Remove failed: {e}"),
                },
            };
            let _ = tx.send(event);
        });
    }
}

fn modal_border() -> Style {
    theme::border()
}

/// Human-readable singular label for a resource kind, for status/dialog text.
const fn singular(kind: ResourceKind) -> &'static str {
    match kind {
        ResourceKind::Containers => "Container",
        ResourceKind::Images => "Image",
        ResourceKind::Volumes => "Volume",
        ResourceKind::Networks => "Network",
    }
}

#[cfg(test)]
mod tests {
    use k9tui::widgets::table::TableDataState;
    use ratatui::{backend::TestBackend, buffer::Cell};

    use super::*;
    use crate::{
        docker::{
            ContainerRow, ImageRow, NetworkRow, VolumeRow,
            client::{DaemonHealth, DockerClient},
        },
        ui::widgets::hotkeys::RESOURCE_HOTKEYS,
    };

    type Term = Terminal<TestBackend>;

    fn term() -> Term {
        Terminal::new(TestBackend::new(120, 30)).unwrap()
    }

    /// App with one row selected in every view and a daemon that is always
    /// unreachable, so docker calls fail fast and surface in the status line.
    fn app() -> App {
        let mut app = App::new();
        app.state = AppState::List;
        app.docker = Some(DockerClient::unreachable());
        app.containers = TableDataState::new(vec![ContainerRow {
            id: "c1".into(),
            name: "web".into(),
            ..Default::default()
        }]);
        app.images = TableDataState::new(vec![ImageRow {
            id: "i1".into(),
            repo_tags: "nginx:latest".into(),
            ..Default::default()
        }]);
        app.volumes = TableDataState::new(vec![VolumeRow {
            name: "data".into(),
            ..Default::default()
        }]);
        app.networks = TableDataState::new(vec![NetworkRow {
            id: "n1".into(),
            name: "backend".into(),
            ..Default::default()
        }]);
        app.containers.view.state.select(Some(0));
        app.images.view.state.select(Some(0));
        app.volumes.view.state.select(Some(0));
        app.networks.view.state.select(Some(0));
        app
    }

    /// Real terminals report `D` as Char('D') + SHIFT.
    fn shift_d() -> KeyEvent {
        KeyEvent::new(KeyCode::Char('D'), KeyModifiers::SHIFT)
    }

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    async fn flush_background(app: &mut App) {
        for _ in 0..32 {
            tokio::task::yield_now().await;
            app.drain_background_events();
        }
    }

    fn screen(app: &mut App, term: &mut Term) -> String {
        term.draw(|f| app.render(f)).unwrap();
        let buf = term.backend().buffer();
        buf.content
            .chunks(usize::from(buf.area.width))
            .map(|row| row.iter().map(Cell::symbol).collect::<String>())
            .collect::<Vec<_>>()
            .join("\n")
    }

    const KINDS: [ResourceKind; 4] = [
        ResourceKind::Containers,
        ResourceKind::Images,
        ResourceKind::Volumes,
        ResourceKind::Networks,
    ];

    #[tokio::test]
    async fn i_failure_stays_on_list_with_status_in_every_view() {
        for kind in KINDS {
            let mut app = app();
            let mut t = term();
            app.switch_view(kind);
            app.on_key_event(key(KeyCode::Char('i')), &mut t)
                .await
                .unwrap();
            flush_background(&mut app).await;
            assert_eq!(app.state, AppState::List, "{kind:?}");
            let out = screen(&mut app, &mut t);
            assert!(out.contains("Daemon info failed"), "{kind:?}: {out}");
        }
    }

    #[tokio::test]
    async fn info_panel_closes_on_q_esc_i_and_ctrl_c_quits() {
        for code in [KeyCode::Char('q'), KeyCode::Esc, KeyCode::Char('i')] {
            let mut app = app();
            let mut t = term();
            app.running = true;
            app.state = AppState::Info(DaemonHealth::default());
            app.on_key_event(key(code), &mut t).await.unwrap();
            assert_eq!(app.state, AppState::List, "{code:?}");
            assert!(app.running, "{code:?} must not quit");
        }
        let mut app = app();
        let mut t = term();
        app.running = true;
        app.state = AppState::Info(DaemonHealth::default());
        app.on_key_event(
            KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL),
            &mut t,
        )
        .await
        .unwrap();
        assert!(!app.running);
    }

    #[tokio::test]
    async fn d_and_enter_describe_every_kind() {
        for kind in KINDS {
            for code in [KeyCode::Char('d'), KeyCode::Enter] {
                let mut app = app();
                let mut t = term();
                app.switch_view(kind);
                app.on_key_event(key(code), &mut t).await.unwrap();
                flush_background(&mut app).await;
                // Reached the inspect call (daemon unreachable) rather than
                // the old "only available for containers" refusal.
                let out = screen(&mut app, &mut t);
                assert!(out.contains("Inspect failed"), "{kind:?}: {out}");
                assert!(!out.contains("only available"), "{kind:?}");
            }
        }
    }

    #[tokio::test]
    async fn shift_d_opens_confirm_for_every_kind_and_enter_removes() {
        for kind in KINDS {
            let mut app = app();
            let mut t = term();
            app.switch_view(kind);
            app.on_key_event(shift_d(), &mut t).await.unwrap();
            assert!(app.confirm_dialog.is_some(), "{kind:?}");
            assert_eq!(app.pending_remove.as_ref().map(|p| p.0), Some(kind));
            assert_eq!(app.state, AppState::List, "D must not describe");

            // Default is No: move to Yes, then Enter attempts the removal.
            app.on_key_event(key(KeyCode::Left), &mut t).await.unwrap();
            app.on_key_event(key(KeyCode::Enter), &mut t).await.unwrap();
            assert!(app.confirm_dialog.is_none());
            assert!(app.pending_remove.is_none());
            flush_background(&mut app).await;
            let out = screen(&mut app, &mut t);
            assert!(out.contains("Remove failed"), "{kind:?}: {out}");
        }
    }

    #[tokio::test]
    async fn enter_on_default_no_does_not_remove() {
        let mut app = app();
        let mut t = term();
        app.on_key_event(shift_d(), &mut t).await.unwrap();
        app.on_key_event(key(KeyCode::Enter), &mut t).await.unwrap();
        assert!(app.confirm_dialog.is_none() && app.pending_remove.is_none());
        let out = screen(&mut app, &mut t);
        assert!(!out.contains("Remove failed"), "{out}");
    }

    #[tokio::test]
    async fn pending_remove_targets_selected_id() {
        let mut app = app();
        let mut t = term();
        app.switch_view(ResourceKind::Volumes);
        app.on_key_event(key(KeyCode::Delete), &mut t)
            .await
            .unwrap();
        assert_eq!(
            app.pending_remove,
            Some((ResourceKind::Volumes, "data".to_string()))
        );
        app.on_key_event(key(KeyCode::Esc), &mut t).await.unwrap();
        assert!(app.confirm_dialog.is_none() && app.pending_remove.is_none());
    }

    #[tokio::test]
    async fn esc_cancels_confirm_without_removing() {
        let mut app = app();
        let mut t = term();
        app.on_key_event(shift_d(), &mut t).await.unwrap();
        app.on_key_event(key(KeyCode::Esc), &mut t).await.unwrap();
        let out = screen(&mut app, &mut t);
        assert!(!out.contains("Remove failed"));
        assert!(app.pending_remove.is_none());
    }

    #[tokio::test]
    async fn confirm_dialog_renders_in_buffer() {
        let mut app = app();
        let mut t = term();
        app.switch_view(ResourceKind::Networks);
        app.on_key_event(shift_d(), &mut t).await.unwrap();
        let out = screen(&mut app, &mut t);
        assert!(out.contains("Remove Network?"), "{out}");
        assert!(out.contains("Remove Network 'backend'?"));
        assert!(out.contains("Yes") && out.contains("No"));
    }

    #[tokio::test]
    async fn describe_view_renders_and_scrolls() {
        let mut app = app();
        let mut t = term();
        let text = (0..100)
            .map(|i| format!("line-{i}"))
            .collect::<Vec<_>>()
            .join("\n");
        app.state = AppState::Describe {
            name: "web".into(),
            text,
        };
        let out = screen(&mut app, &mut t);
        assert!(out.contains("Describe: web"));
        assert!(out.contains("line-0"));
        assert!(!out.contains("line-99"));

        app.on_key_event(key(KeyCode::Char('G')), &mut t)
            .await
            .unwrap();
        let out = screen(&mut app, &mut t);
        assert!(out.contains("line-99"));
        assert!(!out.contains("line-0\n") || app.describe_scroll > 0);

        app.on_key_event(key(KeyCode::Esc), &mut t).await.unwrap();
        assert_eq!(app.state, AppState::List);
    }

    async fn type_cmd(app: &mut App, t: &mut Term, s: &str) {
        for c in s.chars() {
            app.on_key_event(key(KeyCode::Char(c)), t).await.unwrap();
        }
    }

    #[tokio::test]
    async fn colon_bar_types_completes_runs_and_switches_view() {
        let mut app = app();
        let mut t = term();
        app.on_key_event(key(KeyCode::Char(':')), &mut t)
            .await
            .unwrap();
        assert!(app.command_mode && app.list_search_open);
        type_cmd(&mut app, &mut t, "im").await;
        assert_eq!(app.command_text, "im");
        assert_eq!(app.list_filter, "", "typing must not filter rows");
        let out = screen(&mut app, &mut t);
        assert!(out.contains(" Command "), "{out}");
        assert!(out.contains(":images"), "dim ghost completion: {out}");
        app.on_key_event(key(KeyCode::Tab), &mut t).await.unwrap();
        assert_eq!(app.command_text, "images");
        app.on_key_event(key(KeyCode::Enter), &mut t).await.unwrap();
        assert!(!app.command_mode && !app.list_search_open);
        assert_eq!(app.view, ResourceKind::Images);
        assert_eq!(app.hotkeys.len(), RESOURCE_HOTKEYS.len());
    }

    #[tokio::test]
    async fn colon_aliases_unknown_quit_esc_and_ctrl_c() {
        let mut app = app();
        let mut t = term();
        for (cmd, kind) in [
            ("v", ResourceKind::Volumes),
            ("n", ResourceKind::Networks),
            ("c", ResourceKind::Containers),
        ] {
            app.on_key_event(key(KeyCode::Char(':')), &mut t)
                .await
                .unwrap();
            type_cmd(&mut app, &mut t, cmd).await;
            app.on_key_event(key(KeyCode::Enter), &mut t).await.unwrap();
            assert_eq!(app.view, kind, "{cmd}");
        }
        app.on_key_event(key(KeyCode::Char(':')), &mut t)
            .await
            .unwrap();
        type_cmd(&mut app, &mut t, "zz").await;
        app.on_key_event(key(KeyCode::Enter), &mut t).await.unwrap();
        assert_eq!(app.view, ResourceKind::Containers);
        assert!(screen(&mut app, &mut t).contains("Unknown command: zz"));

        app.on_key_event(key(KeyCode::Char(':')), &mut t)
            .await
            .unwrap();
        type_cmd(&mut app, &mut t, "images").await;
        app.on_key_event(key(KeyCode::Esc), &mut t).await.unwrap();
        assert!(!app.command_mode && !app.list_search_open);
        assert_eq!(app.view, ResourceKind::Containers);

        app.running = true;
        app.on_key_event(key(KeyCode::Char(':')), &mut t)
            .await
            .unwrap();
        app.on_key_event(
            KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL),
            &mut t,
        )
        .await
        .unwrap();
        assert!(!app.running);

        app.on_key_event(key(KeyCode::Esc), &mut t).await.unwrap();
        app.running = true;
        app.on_key_event(key(KeyCode::Char(':')), &mut t)
            .await
            .unwrap();
        type_cmd(&mut app, &mut t, "quit").await;
        app.on_key_event(key(KeyCode::Enter), &mut t).await.unwrap();
        assert!(!app.running);
    }

    #[tokio::test]
    async fn colon_is_text_in_search_and_inert_in_other_views() {
        let mut app = app();
        let mut t = term();
        app.on_key_event(key(KeyCode::Char('/')), &mut t)
            .await
            .unwrap();
        app.on_key_event(key(KeyCode::Char(':')), &mut t)
            .await
            .unwrap();
        assert!(!app.command_mode);
        assert_eq!(app.list_filter, ":");

        let mut app = self::app();
        app.state = AppState::Describe {
            name: "web".into(),
            text: "x".into(),
        };
        app.on_key_event(key(KeyCode::Char(':')), &mut t)
            .await
            .unwrap();
        assert!(!app.command_mode && !app.list_search_open);
    }

    fn ctrl(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
    }

    async fn colon(app: &mut App, t: &mut Term, cmd: &str) {
        app.on_key_event(key(KeyCode::Char(':')), t).await.unwrap();
        type_cmd(app, t, cmd).await;
        app.on_key_event(key(KeyCode::Enter), t).await.unwrap();
    }

    #[tokio::test]
    async fn colon_whitespace_case_and_unicode_input() {
        let mut app = app();
        let mut t = term();
        colon(&mut app, &mut t, "  IMAGES  ").await;
        assert_eq!(app.view, ResourceKind::Images);
        // Empty and whitespace-only just close the bar, no status noise.
        for cmd in ["", "   "] {
            colon(&mut app, &mut t, cmd).await;
            assert!(!app.command_mode && !app.list_search_open, "{cmd:?}");
            assert_eq!(app.view, ResourceKind::Images);
            assert!(!screen(&mut app, &mut t).contains("Unknown command"));
        }
        // Multibyte / emoji input: Unknown, Backspace removes a whole char.
        colon(&mut app, &mut t, "🦀é").await;
        // (wide glyphs render as glyph + filler cell, so only match the prefix)
        assert!(screen(&mut app, &mut t).contains("Unknown command: 🦀"));
        app.on_key_event(key(KeyCode::Char(':')), &mut t)
            .await
            .unwrap();
        type_cmd(&mut app, &mut t, "🦀é").await;
        app.on_key_event(key(KeyCode::Backspace), &mut t)
            .await
            .unwrap();
        assert_eq!(app.command_text, "🦀");
        app.on_key_event(key(KeyCode::Backspace), &mut t)
            .await
            .unwrap();
        app.on_key_event(key(KeyCode::Backspace), &mut t)
            .await
            .unwrap();
        assert_eq!(app.command_text, "");
        let _ = screen(&mut app, &mut t);
    }

    #[tokio::test]
    async fn colon_bar_ignores_ctrl_chars_and_keeps_filter_untouched() {
        let mut app = app();
        let mut t = term();
        // Applied row filter survives a cancelled command and a `/` after.
        app.on_key_event(key(KeyCode::Char('/')), &mut t)
            .await
            .unwrap();
        type_cmd(&mut app, &mut t, "web").await;
        app.on_key_event(key(KeyCode::Enter), &mut t).await.unwrap();
        assert_eq!(app.list_filter, "web");
        app.on_key_event(key(KeyCode::Char(':')), &mut t)
            .await
            .unwrap();
        type_cmd(&mut app, &mut t, "zz").await;
        app.on_key_event(ctrl('w'), &mut t).await.unwrap();
        app.on_key_event(
            KeyEvent::new(KeyCode::Char('x'), KeyModifiers::ALT),
            &mut t,
        )
        .await
        .unwrap();
        assert_eq!(app.command_text, "zz", "Ctrl/Alt chords are not text");
        assert_eq!(app.list_filter, "web", "typing a command never filters");
        app.on_key_event(key(KeyCode::Esc), &mut t).await.unwrap();
        assert_eq!(app.list_filter, "web", "Esc must not clear the filter");
        assert!(!app.command_mode && app.command_text.is_empty());
        app.on_key_event(key(KeyCode::Char('/')), &mut t)
            .await
            .unwrap();
        assert!(app.list_search_open && !app.command_mode);
        type_cmd(&mut app, &mut t, "x").await;
        assert_eq!(app.list_filter, "webx", "`/` is a normal filter again");
    }

    #[tokio::test]
    async fn colon_q_in_bar_is_text_and_inert_with_dialog_or_in_logs() {
        let mut app = app();
        let mut t = term();
        app.running = true;
        app.on_key_event(key(KeyCode::Char(':')), &mut t)
            .await
            .unwrap();
        type_cmd(&mut app, &mut t, "q").await;
        assert!(app.running, "q is typed, not a quit key, until Enter");
        app.on_key_event(key(KeyCode::Esc), &mut t).await.unwrap();

        // Confirm dialog open: `:` must not open the bar.
        app.on_key_event(shift_d(), &mut t).await.unwrap();
        assert!(app.confirm_dialog.is_some());
        app.on_key_event(key(KeyCode::Char(':')), &mut t)
            .await
            .unwrap();
        assert!(!app.command_mode && !app.list_search_open);
        assert!(app.confirm_dialog.is_some());
        app.on_key_event(key(KeyCode::Esc), &mut t).await.unwrap();

        // Log view + its search bar: `:` is just text.
        app.state = AppState::Logs {
            id: "c1".into(),
            name: "web".into(),
        };
        app.on_key_event(key(KeyCode::Char(':')), &mut t)
            .await
            .unwrap();
        assert!(!app.command_mode && !app.list_search_open);
        app.on_key_event(key(KeyCode::Char('/')), &mut t)
            .await
            .unwrap();
        app.on_key_event(key(KeyCode::Char(':')), &mut t)
            .await
            .unwrap();
        assert_eq!(app.log_filter, ":");
        assert!(!app.command_mode);
    }

    #[tokio::test]
    async fn colon_bar_renders_at_tiny_and_normal_sizes_with_long_text() {
        let mut app = app();
        let mut t = term();
        app.on_key_event(key(KeyCode::Char(':')), &mut t)
            .await
            .unwrap();
        type_cmd(&mut app, &mut t, "im").await;
        for (w, h) in [(20, 5), (80, 24), (1, 1), (3, 3), (10, 4)] {
            let mut small = Terminal::new(TestBackend::new(w, h)).unwrap();
            small.draw(|f| app.render(f)).unwrap();
        }
        type_cmd(&mut app, &mut t, &"x".repeat(200)).await;
        let mut small = Terminal::new(TestBackend::new(20, 5)).unwrap();
        small.draw(|f| app.render(f)).unwrap();
        // 120-col hotkey bar still lists the new hint.
        app.on_key_event(key(KeyCode::Esc), &mut t).await.unwrap();
        let out = screen(&mut app, &mut t);
        assert!(out.contains("command"), "{out}");
    }

    #[tokio::test]
    async fn remove_refuses_when_resource_in_flight() {
        let mut app = App::new();
        app.docker = Some(DockerClient::unreachable());
        let key = crate::in_flight::container_key("x");
        assert!(app.in_flight.try_begin(key.clone()));
        app.remove_resource(ResourceKind::Containers, "x");
        assert!(app.in_flight.contains(&key));
    }

    #[tokio::test]
    async fn restart_refuses_when_container_in_flight() {
        let mut app = App::new();
        app.docker = Some(DockerClient::unreachable());
        let id = "abc123";
        assert!(app.in_flight.try_begin(crate::in_flight::container_key(id)));
        app.restart_container(id);
        assert!(app.in_flight.contains(&crate::in_flight::container_key(id)));
    }

    #[tokio::test]
    async fn action_done_clears_in_flight_and_sets_status() {
        use crate::app::BackgroundEvent;

        let mut app = App::new();
        let key = crate::in_flight::container_key("c1");
        assert!(app.in_flight.try_begin(key.clone()));
        app.bg_tx
            .send(BackgroundEvent::ActionDone {
                key: key.clone(),
                message: "Restarted".into(),
            })
            .unwrap();
        app.drain_background_events();
        assert!(!app.in_flight.contains(&key));
    }
}

#[cfg(test)]
mod filter_tests {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    use crate::{
        app::App,
        docker::{ContainerRow, ImageRow, NetworkRow, VolumeRow},
    };

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn press(app: &mut App, code: KeyCode) -> bool {
        app.on_key_list_filter_or_quit(key(code))
    }

    fn type_str(app: &mut App, s: &str) {
        for c in s.chars() {
            press(app, KeyCode::Char(c));
        }
    }

    #[test]
    fn running_only_hides_stopped_containers() {
        let mut app = app();
        for (row, status) in
            app.containers_all.iter_mut().zip(["running", "exited"])
        {
            row.status = status.into();
        }
        app.running_only = true;
        app.reapply_list_filter();
        assert_eq!(app.containers.model.items.len(), 1);
        app.running_only = false;
        app.reapply_list_filter();
        assert_eq!(app.containers.model.items.len(), 2);
    }

    fn app() -> App {
        let mut app = App::new();
        app.running = true;
        app.containers_all = vec![
            ContainerRow {
                name: "Web".into(),
                image: "nginx".into(),
                ..Default::default()
            },
            ContainerRow {
                name: "db".into(),
                image: "postgres".into(),
                ..Default::default()
            },
        ];
        app.images_all = vec![ImageRow {
            repo_tags: "nginx:latest".into(),
            ..Default::default()
        }];
        app.volumes_all = vec![
            VolumeRow {
                name: "data".into(),
                ..Default::default()
            },
            VolumeRow {
                name: "cache".into(),
                driver: "NGINX-driver".into(),
                ..Default::default()
            },
        ];
        app.networks_all = vec![NetworkRow {
            name: "bridge".into(),
            scope: "local".into(),
            ..Default::default()
        }];
        app.reapply_list_filter();
        app
    }

    #[test]
    fn slash_opens_search_and_chars_append() {
        let mut app = app();
        assert!(press(&mut app, KeyCode::Char('/')));
        assert!(app.list_search_open);
        type_str(&mut app, "ng");
        assert_eq!(app.list_filter, "ng");
        // 'q' while typing is text, not quit/clear.
        press(&mut app, KeyCode::Char('q'));
        assert_eq!(app.list_filter, "ngq");
        assert!(app.running);
    }

    #[test]
    fn backspace_pops() {
        let mut app = app();
        press(&mut app, KeyCode::Char('/'));
        type_str(&mut app, "abc");
        press(&mut app, KeyCode::Backspace);
        assert_eq!(app.list_filter, "ab");
    }

    #[test]
    fn enter_closes_keeping_filter() {
        let mut app = app();
        press(&mut app, KeyCode::Char('/'));
        type_str(&mut app, "nginx");
        press(&mut app, KeyCode::Enter);
        assert!(!app.list_search_open);
        assert_eq!(app.list_filter, "nginx");
        assert_eq!(app.containers.model.items.len(), 1);
    }

    #[test]
    fn esc_clears_and_closes() {
        let mut app = app();
        press(&mut app, KeyCode::Char('/'));
        type_str(&mut app, "nginx");
        press(&mut app, KeyCode::Esc);
        assert!(!app.list_search_open);
        assert_eq!(app.list_filter, "");
        assert_eq!(app.containers.model.items.len(), 2);
    }

    #[test]
    fn ctrl_c_while_filtering_quits_without_appending() {
        let mut app = app();
        press(&mut app, KeyCode::Char('/'));
        type_str(&mut app, "ab");
        let ctrl_c = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);
        assert!(app.on_key_list_filter_or_quit(ctrl_c));
        assert!(!app.running);
        assert_eq!(app.list_filter, "ab");
    }

    #[test]
    fn ctrl_c_outside_search_quits() {
        let mut app = app();
        let ctrl_c = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);
        assert!(app.on_key_list_filter_or_quit(ctrl_c));
        assert!(!app.running);
    }

    #[test]
    fn q_with_filter_clears_instead_of_quitting() {
        let mut app = app();
        press(&mut app, KeyCode::Char('/'));
        type_str(&mut app, "nginx");
        press(&mut app, KeyCode::Enter);
        press(&mut app, KeyCode::Char('q'));
        assert!(app.running);
        assert_eq!(app.list_filter, "");
        assert_eq!(app.containers.model.items.len(), 2);
    }

    #[test]
    fn q_without_filter_quits() {
        let mut app = app();
        assert!(press(&mut app, KeyCode::Char('q')));
        assert!(!app.running);
    }

    #[test]
    fn other_keys_are_not_consumed() {
        let mut app = app();
        assert!(!press(&mut app, KeyCode::Char('j')));
        assert!(app.running);
    }

    #[test]
    fn filter_applies_to_all_lists_case_insensitive_any_column() {
        let mut app = app();
        press(&mut app, KeyCode::Char('/'));
        type_str(&mut app, "NgInX");
        // containers: image column; images: tags; volumes: driver column;
        // networks: no match.
        assert_eq!(app.containers.model.items.len(), 1);
        assert_eq!(app.images.model.items.len(), 1);
        assert_eq!(app.volumes.model.items.len(), 1);
        assert_eq!(app.networks.model.items.len(), 0);
        // Backspacing to empty restores everything.
        for _ in 0..5 {
            press(&mut app, KeyCode::Backspace);
        }
        assert_eq!(app.containers.model.items.len(), 2);
        assert_eq!(app.volumes.model.items.len(), 2);
        assert_eq!(app.networks.model.items.len(), 1);
    }
}
