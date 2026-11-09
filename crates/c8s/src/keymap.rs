use k9tui::config::AppSpec;

/// `c8s` definitions live in `config.toml`.
pub const SPEC: AppSpec = AppSpec {
    name: "c8s",
    config: include_str!("config.toml"),
};

#[cfg(test)]
mod tests {
    use k9tui::config::resolve;

    use super::*;

    #[test]
    fn builtin_files_validate() {
        resolve(None, &SPEC, "test").unwrap();
    }

    #[test]
    fn conflict_on_logs_screen_is_fatal() {
        let errors = resolve(Some("[c8s.keys]\ndedup = \"j\"\n"), &SPEC, "c")
            .unwrap_err();
        assert!(errors.join("\n").contains("dedup"));
    }

    /// Every example a user can copy from the README must start the app.
    #[test]
    fn readme_examples_are_valid() {
        let readme = include_str!("../README.md");
        let examples: Vec<_> = readme
            .split("```toml\n")
            .skip(1)
            .filter_map(|rest| rest.split("```").next())
            .collect();
        assert_ne!(examples.len(), 0);
        for example in examples {
            resolve(Some(example), &SPEC, "README").unwrap();
        }
    }
}
