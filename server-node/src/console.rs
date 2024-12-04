//! # User interface for a unix terminal
//! Renders server's internal state and latest messages in a consistent way
//! Intended to be used via the global static [`CONSOLE`] and the `log`-macro.

use crate::Server;
use std::{collections::VecDeque, fmt::Display, io::Write, sync::Mutex, time::SystemTime};
use terminal_size::{terminal_size, Height};

impl Display for Server {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let write_label = |f: &mut std::fmt::Formatter<'_>, label| write!(f, "    {label:<16} ");

        let title = format!(
            "Server {} listening on {}",
            self.config.id, self.config.address_private
        );
        let mut hline = title.chars().map(|_| '-').collect::<String>();
        hline = format!("\x1B[90m{hline}\x1B[0m");
        writeln!(f, "{hline}\n{title}\n")?;

        // Coordinator
        write_label(f, "Coordinator id")?;
        if let Some(coordinator) = self.coordinator_id {
            writeln!(f, "{coordinator}",)?;
        } else {
            writeln!(f, "Unknown",)?;
        }

        // Peers
        write_label(f, "Active peers")?;
        write!(f, "[ ")?;

        let mut ids = self.peers.iter().map(|item| item.id).collect::<Vec<_>>();
        ids.push(self.config.id);
        ids.sort();
        for (i, id) in ids.iter().enumerate() {
            if *id != self.config.id {
                write!(f, "{id}")?;
            } else {
                write!(f, "\x1B[1m{id}\x1B[0m")?;
            }

            if i != ids.len() - 1 {
                write!(f, ", ")?;
            }
        }
        writeln!(f, " ]")?;

        // Role
        write_label(f, "Current role")?;
        writeln!(f, "{:?}", self.local_role)?;

        writeln!(f, "{hline}")
    }
}

pub struct Console {
    history_len: usize,
    event_log: VecDeque<(SystemTime, String)>,
}

impl Console {
    pub fn push_event(&mut self, description: String) {
        self.event_log.truncate(self.history_len - 1);
        self.event_log.push_front((SystemTime::now(), description));
    }

    const fn new(history_len: usize) -> Self {
        Self {
            history_len,
            event_log: VecDeque::new(),
        }
    }

    fn render(&self, server: &Server) {
        // Get terminal height
        let mut lines: Option<usize> = terminal_size().map(|(_, Height(h))| h.into());
        let mut stdout = std::io::stdout().lock();

        // Clear terminal and move cursor to top left
        stdout.write_all(b"\x1B[2J\x1B[H").unwrap();

        // Format the whole server state into a string
        let server_state = format!("{server}");
        if let Some(lines) = &mut lines {
            // Subtract from remaining terminal lines available
            let remaining = lines.saturating_sub(server_state.lines().count());
            if remaining > 0 {
                stdout.write_all(server_state.as_bytes()).unwrap();
                *lines = remaining;
            } else {
                writeln!(
                    stdout,
                    "Terminal height is too low! Can't show server state."
                )
                .unwrap();
                *lines = lines.saturating_sub(2); // Probably takes two lines due to small width
            }
            *lines = lines.saturating_sub(1);
        }

        // Print log (up to the remaining terminal lines, no more)
        let lines = lines.unwrap_or(usize::MAX);
        for (time, desc) in self.event_log.iter().take(lines).rev() {
            if let Ok(duration) = time.duration_since(server.start_time) {
                write!(stdout, "\x1B[90m{duration:<9.3?}:\x1B[0m ").unwrap();
            }
            stdout.write_all(desc.as_bytes()).unwrap();
            stdout.write_all(b"\n").unwrap();
        }
    }
}

pub static CONSOLE: Mutex<Console> = const { Mutex::new(Console::new(128)) };

macro_rules! debug {
    ($($arg:tt)*) => {{
            let mut cons = crate::console::CONSOLE.lock().unwrap();
            let log = format!($($arg)*);
            for line in log.split("\n") {
                cons.push_event(format!("\x1B[90m{line}\x1B[0m"));
            }
    }};
}
macro_rules! log {
    ($($arg:tt)*) => {{
            let mut cons = crate::console::CONSOLE.lock().unwrap();
            let log = format!($($arg)*);
            for line in log.split("\n") {
                cons.push_event(line.to_string());
            }
    }};
}
macro_rules! warning {
    ($($arg:tt)*) => {{
            let mut cons = crate::console::CONSOLE.lock().unwrap();
            let log = format!($($arg)*);
            for line in log.split("\n") {
                cons.push_event(format!("\x1B[33m{line}\x1B[0m"));
            }
    }};
}
macro_rules! _error {
    ($($arg:tt)*) => {{
            let mut cons = crate::console::CONSOLE.lock().unwrap();
            let log = format!($($arg)*);
            for line in log.split("\n") {
                cons.push_event(format!("\x1B[31m{line}\x1B[0m"));
            }
    }};
}

pub(crate) use {debug, log, warning};

pub fn render(server: &Server) {
    let cons = CONSOLE.lock().unwrap();
    cons.render(server);
}
