//! Process row model, formatting, sorting and table wiring.

pub mod monitor;
pub mod signals;

use core::cmp::Ordering;

use k9tui::widgets::table::TableData;

/// One row in the process table.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ProcessRow {
    pub pid: i32,
    pub ppid: Option<i32>,
    pub user: String,
    /// CPU usage in percent of a single core (may exceed 100 on a multi-core
    /// machine, exactly like `top`).
    pub cpu: f32,
    /// Resident memory as a percent of total RAM.
    pub mem: f32,
    /// Resident set size, bytes.
    pub rss: u64,
    /// Virtual memory size, bytes.
    pub vsz: u64,
    /// Single-letter state code (R, S, Z, ...).
    pub state: char,
    /// Accumulated CPU time, milliseconds.
    pub cpu_time_ms: u64,
    pub name: String,
    pub cmdline: String,
    /// Start time, seconds since the epoch; used to detect PID reuse.
    pub start_time: u64,
}

impl ProcessRow {
    /// Text for the COMMAND column: the full command line, else the name.
    #[must_use]
    pub fn command(&self) -> String {
        if self.cmdline.trim().is_empty() {
            self.name.clone()
        } else {
            self.cmdline.clone()
        }
    }
}

/// Format a byte count in binary units (`1.5M`), like htop.
#[must_use]
#[allow(clippy::cast_precision_loss)]
pub fn format_size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "K", "M", "G", "T"];
    if bytes < 1024 {
        return format!("{bytes}B");
    }
    let mut size = bytes as f64;
    let mut unit = 0;
    while size >= 1024.0 && unit + 1 < UNITS.len() {
        size /= 1024.0;
        unit += 1;
    }
    let suffix = UNITS.get(unit).copied().unwrap_or("B");
    format!("{size:.1}{suffix}")
}

/// Format accumulated CPU milliseconds as `h:mm:ss` (or `m:ss` under an hour).
#[must_use]
pub fn format_cpu_time(ms: u64) -> String {
    let total = ms / 1000;
    let (h, m, s) = (total / 3600, (total % 3600) / 60, total % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}

/// Format an uptime in seconds as `Nd Nh Nm` (dropping leading zero units).
#[must_use]
pub fn format_uptime(secs: u64) -> String {
    let (d, h, m) = (secs / 86_400, (secs % 86_400) / 3600, (secs % 3600) / 60);
    if d > 0 {
        format!("{d}d {h}h {m}m")
    } else if h > 0 {
        format!("{h}h {m}m")
    } else {
        format!("{m}m")
    }
}

/// Percent value with one decimal, as shown in the CPU/MEM columns.
#[must_use]
pub fn format_percent(value: f32) -> String {
    format!("{value:.1}")
}

/// System-wide totals for the `i` panel.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SystemInfo {
    pub os_name: String,
    pub kernel: String,
    pub host: String,
    pub uptime_secs: u64,
    pub load_one: f64,
    pub load_five: f64,
    pub load_fifteen: f64,
    pub cpu_count: usize,
    pub total_mem: u64,
    pub used_mem: u64,
    pub total_swap: u64,
    pub used_swap: u64,
}

impl SystemInfo {
    /// Lines shown in the system panel, in `Label: value` form.
    #[must_use]
    pub fn lines(&self) -> Vec<String> {
        vec![
            format!("OS:         {}", self.os_name),
            format!("Kernel:     {}", self.kernel),
            format!("Host:       {}", self.host),
            format!("Uptime:     {}", format_uptime(self.uptime_secs)),
            format!(
                "Load avg:   {:.2} {:.2} {:.2}",
                self.load_one, self.load_five, self.load_fifteen
            ),
            format!("CPUs:       {}", self.cpu_count),
            format!(
                "Memory:     {} / {}",
                format_size(self.used_mem),
                format_size(self.total_mem)
            ),
            format!(
                "Swap:       {} / {}",
                format_size(self.used_swap),
                format_size(self.total_swap)
            ),
        ]
    }
}

/// Which column the process table is sorted by.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SortKey {
    #[default]
    Cpu,
    Mem,
    Pid,
    Name,
}

impl SortKey {
    /// Lower-case label used in the table title and `:` commands.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Cpu => "cpu",
            Self::Mem => "mem",
            Self::Pid => "pid",
            Self::Name => "name",
        }
    }

    /// Direction used the first time this key is selected (busiest first for
    /// cpu/memory, natural order for pid/name, like `top`/`htop`).
    #[must_use]
    pub const fn starts_descending(self) -> bool {
        matches!(self, Self::Cpu | Self::Mem)
    }

    fn compare(self, a: &ProcessRow, b: &ProcessRow) -> Ordering {
        match self {
            Self::Cpu => a.cpu.partial_cmp(&b.cpu).unwrap_or(Ordering::Equal),
            Self::Mem => a.rss.cmp(&b.rss),
            Self::Pid => a.pid.cmp(&b.pid),
            Self::Name => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
        }
    }
}

/// Sort `rows` in place by `key`. Ties always break by ascending PID so the
/// order is stable between refreshes and the cursor does not jump.
pub fn sort_rows(rows: &mut [ProcessRow], key: SortKey, descending: bool) {
    rows.sort_by(|a, b| {
        let ord = key.compare(a, b);
        let ord = if descending { ord.reverse() } else { ord };
        ord.then_with(|| a.pid.cmp(&b.pid))
    });
}

const COLUMNS: [&str; 9] = [
    "PID", "PPID", "USER", "CPU%", "MEM%", "RSS", "STAT", "TIME", "COMMAND",
];
const COMMAND_COL: usize = 8;

impl TableData for ProcessRow {
    fn title() -> &'static str {
        "Processes"
    }

    fn ref_array(&self) -> Vec<String> {
        vec![
            self.pid.to_string(),
            self.ppid.map_or_else(String::new, |p| p.to_string()),
            self.user.clone(),
            format_percent(self.cpu),
            format_percent(self.mem),
            format_size(self.rss),
            self.state.to_string(),
            format_cpu_time(self.cpu_time_ms),
            self.command(),
        ]
    }

    fn num_columns(&self) -> usize {
        COLUMNS.len()
    }

    fn cols() -> Vec<&'static str> {
        COLUMNS.to_vec()
    }

    fn fill_column() -> Option<usize> {
        Some(COMMAND_COL)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(pid: i32, name: &str, cpu: f32, rss: u64) -> ProcessRow {
        ProcessRow {
            pid,
            name: name.to_string(),
            cpu,
            rss,
            ..ProcessRow::default()
        }
    }

    #[test]
    fn formats_sizes_in_binary_units() {
        assert_eq!(format_size(0), "0B");
        assert_eq!(format_size(512), "512B");
        assert_eq!(format_size(1536), "1.5K");
        assert_eq!(format_size(1024 * 1024), "1.0M");
        assert_eq!(format_size(3 * 1024 * 1024 * 1024), "3.0G");
    }

    #[test]
    fn formats_cpu_time() {
        assert_eq!(format_cpu_time(0), "0:00");
        assert_eq!(format_cpu_time(65_000), "1:05");
        assert_eq!(format_cpu_time(3_725_000), "1:02:05");
    }

    #[test]
    fn formats_uptime() {
        assert_eq!(format_uptime(90), "1m");
        assert_eq!(format_uptime(3 * 3600 + 120), "3h 2m");
        assert_eq!(format_uptime(2 * 86_400 + 5 * 3600), "2d 5h 0m");
    }

    #[test]
    fn command_falls_back_to_name_when_cmdline_empty() {
        let mut r = row(1, "kthreadd", 0.0, 0);
        assert_eq!(r.command(), "kthreadd");
        r.cmdline = "nginx -g daemon off;".to_string();
        assert_eq!(r.command(), "nginx -g daemon off;");
    }

    #[test]
    fn sorts_by_cpu_descending_with_pid_tiebreak() {
        let mut rows = vec![
            row(3, "a", 10.0, 0),
            row(1, "b", 50.0, 0),
            row(2, "c", 10.0, 0),
        ];
        sort_rows(&mut rows, SortKey::Cpu, true);
        let pids: Vec<i32> = rows.iter().map(|r| r.pid).collect();
        assert_eq!(pids, vec![1, 2, 3]);

        sort_rows(&mut rows, SortKey::Cpu, false);
        let pids: Vec<i32> = rows.iter().map(|r| r.pid).collect();
        assert_eq!(pids, vec![2, 3, 1]);
    }

    #[test]
    fn sorts_by_name_case_insensitively() {
        let mut rows = vec![
            row(1, "Zsh", 0.0, 0),
            row(2, "bash", 0.0, 0),
            row(3, "Nginx", 0.0, 0),
        ];
        sort_rows(&mut rows, SortKey::Name, false);
        let names: Vec<&str> = rows.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, vec!["bash", "Nginx", "Zsh"]);
    }

    #[test]
    fn table_data_maps_columns_in_order() {
        let r = ProcessRow {
            pid: 42,
            ppid: Some(1),
            user: "root".into(),
            cpu: 12.3,
            mem: 1.5,
            rss: 1536,
            vsz: 4096,
            state: 'R',
            cpu_time_ms: 65_000,
            name: "nginx".into(),
            cmdline: "nginx -g daemon off;".into(),
            start_time: 5,
        };
        assert_eq!(ProcessRow::cols(), COLUMNS.to_vec());
        assert_eq!(r.num_columns(), 9);
        assert_eq!(
            r.ref_array(),
            vec![
                "42".to_string(),
                "1".to_string(),
                "root".to_string(),
                "12.3".to_string(),
                "1.5".to_string(),
                "1.5K".to_string(),
                "R".to_string(),
                "1:05".to_string(),
                "nginx -g daemon off;".to_string(),
            ]
        );
        assert_eq!(ProcessRow::fill_column(), Some(COMMAND_COL));
    }

    #[test]
    fn system_info_lines_include_totals() {
        let info = SystemInfo {
            os_name: "Linux".into(),
            kernel: "6.9.0".into(),
            host: "demo-host".into(),
            uptime_secs: 90_000,
            load_one: 0.5,
            load_five: 0.4,
            load_fifteen: 0.3,
            cpu_count: 8,
            total_mem: 16 * 1024 * 1024 * 1024,
            used_mem: 8 * 1024 * 1024 * 1024,
            total_swap: 1024 * 1024 * 1024,
            used_swap: 0,
        };
        let text = info.lines().join("\n");
        for want in [
            "OS:         Linux",
            "Kernel:     6.9.0",
            "Host:       demo-host",
            "Uptime:     1d 1h 0m",
            "Load avg:   0.50 0.40 0.30",
            "CPUs:       8",
            "Memory:     8.0G / 16.0G",
            "Swap:       0B / 1.0G",
        ] {
            assert!(text.contains(want), "missing {want:?} in\n{text}");
        }
    }
}
