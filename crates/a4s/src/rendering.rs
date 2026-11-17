use k9tui::{
    theme,
    widgets::{
        status_line::FOOTER_HEIGHT,
        table::DataTable,
        top_bar::{TOPBAR_HEIGHT, TopBarView},
    },
};
use ratatui::{
    Frame,
    prelude::*,
    widgets::{Block, Borders, Paragraph},
};

use crate::app::{APP_NAME, App};

impl App {
    pub fn render(&mut self, frame: &mut Frame) {
        theme::paint_background(frame);
        let layout = Layout::vertical([
            Constraint::Length(TOPBAR_HEIGHT),
            Constraint::Min(0),
            Constraint::Length(FOOTER_HEIGHT),
        ])
        .split(frame.area());
        let top_area = layout.first().copied().unwrap_or_default();
        let content_area = layout.get(1).copied().unwrap_or_default();
        let footer_area = layout.get(2).copied().unwrap_or_default();

        let n = self.sessions.model.items.len();
        let hotkeys = self.config.bar("sessions");
        let global = self.config.bar("global");
        frame.render_widget(
            TopBarView {
                summary: &format!("Sessions: {n}"),
                recent_hotkeys: &[],
                hotkeys: &hotkeys,
                global_hotkeys: &global,
                app_name: APP_NAME,
                build_info: Some(self.build_info.clone()),
            },
            top_area,
        );

        let block = Block::new()
            .borders(Borders::ALL)
            .border_style(theme::border())
            .title(format!(" Sessions [{n}] "))
            .title_alignment(Alignment::Center);
        let inner = block.inner(content_area);
        frame.render_widget(block, content_area);
        if n == 0 {
            Paragraph::new("No sessions — press c to create one")
                .style(theme::muted())
                .alignment(Alignment::Center)
                .render(inner, frame.buffer_mut());
        } else {
            frame.render_stateful_widget(
                DataTable::default(),
                inner,
                &mut self.sessions,
            );
        }

        frame.render_widget(self.status_line.clone(), footer_area);
    }
}

#[cfg(test)]
mod tests {
    use ratatui::{Terminal, backend::TestBackend, buffer::Cell};

    use super::*;

    #[test]
    fn empty_sessions_view_shows_hint() {
        let mut app = App::new();
        let mut terminal = Terminal::new(TestBackend::new(100, 24)).unwrap();
        terminal.draw(|f| app.render(f)).unwrap();
        let out: String = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(Cell::symbol)
            .collect();
        assert!(out.contains("No sessions"));
    }
}
