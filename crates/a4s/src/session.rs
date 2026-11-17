use k9tui::widgets::table::TableData;

const COLUMNS: [&str; 6] = ["#", "ST", "NAME", "AGENT", "DIR", "AGE"];

/// One row in the sessions table. Task 6 extends this.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SessionRow {
    pub index: String,
    pub state: String,
    pub name: String,
    pub agent: String,
    pub dir: String,
    pub age: String,
}

impl TableData for SessionRow {
    fn title() -> &'static str {
        "Sessions"
    }

    fn ref_array(&self) -> Vec<String> {
        vec![
            self.index.clone(),
            self.state.clone(),
            self.name.clone(),
            self.agent.clone(),
            self.dir.clone(),
            self.age.clone(),
        ]
    }

    fn num_columns(&self) -> usize {
        COLUMNS.len()
    }

    fn cols() -> Vec<&'static str> {
        COLUMNS.to_vec()
    }
}
