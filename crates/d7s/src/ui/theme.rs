use k9tui::theme;
use ratatui::style::Style;

use crate::db::connection::Environment;

#[must_use]
pub fn env_style(env: Environment) -> Style {
    match env {
        Environment::Dev => theme::success(),
        Environment::Staging => theme::warning(),
        Environment::Prod => theme::error(),
    }
}

#[must_use]
pub fn status_up() -> Style {
    theme::success()
}

#[must_use]
pub fn status_down() -> Style {
    theme::error()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn env_styles_differ() {
        assert_ne!(
            env_style(Environment::Dev).fg,
            env_style(Environment::Prod).fg
        );
    }
}
