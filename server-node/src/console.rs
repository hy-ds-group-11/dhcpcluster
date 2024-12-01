//! # User interface for a unix terminal
//! Renders server's internal state and latest messages in a consistent way
//! Intended to be used via the global static [`CONSOLE`] and the `log`-macro.

use crate::Server;
use std::{collections::VecDeque, env, fmt::Display, io::Write, sync::Mutex, time::SystemTime};
use terminal_size::{terminal_size, Height};

pub struct Console {
    history_len: usize,
    event_log: VecDeque<(SystemTime, String)>,
}

impl Display for Server {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let write_label = |f: &mut std::fmt::Formatter<'_>, label| write!(f, "    {label:<16} ");

        let title = format!(
            "Server {} listening on {}",
            self.config.id, self.config.address_private
        );
        let hline = title.chars().map(|_| '-').collect::<String>();
        writeln!(f, "{title}\n{hline}")?;

        write_label(f, "Coordinator")?;
        if let Some(coordinator) = self.coordinator_id {
            writeln!(f, "{coordinator}",)?;
        } else {
            writeln!(f, "Unknown",)?;
        }
        write_label(f, "Active peers")?;
        for peer in &self.peers {
            write!(f, "{peer} ")?;
        }
        writeln!(f)?;
        write_label(f, "Current role")?;
        writeln!(f, "{:?}", self.local_role)?;

        writeln!(f, "{hline}")
    }
}

impl Console {
    pub fn push_event(&mut self, description: String) {
        if !Self::enabled() {
            println!("{}", description);
            return;
        }

        self.event_log.truncate(self.history_len - 1);
        self.event_log.push_front((SystemTime::now(), description));
    }

    fn enabled() -> bool {
        // This system is opt-in via env. variable
        env::var("SERVER_CONSOLE_UI").is_ok()
    }

    const fn new(history_len: usize) -> Self {
        Self {
            history_len,
            event_log: VecDeque::new(),
        }
    }

    fn render(&self, server: &Server) {
        if !Self::enabled() {
            return;
        }

        // Get terminal height
        let mut lines: Option<usize> = terminal_size().map(|(_, Height(h))| h.into());
        let mut stdout = std::io::stdout().lock();
        // Clear terminal
        stdout.write_all(b"\x1B[H\x1B[J").unwrap();
        // Move cursor to 0,0
        stdout.write_all(b"\x1B[0;0H").unwrap();
        // Format the whole server state into a string
        let serv_state = format!("{server}");
        if let Some(lines) = &mut lines {
            // Subtract from remaining terminal lines available
            let remaining = lines.saturating_sub(serv_state.lines().count());
            if remaining > 0 {
                stdout.write_all(serv_state.as_bytes()).unwrap();
                *lines = remaining;
            } else {
                writeln!(
                    stdout,
                    "Terminal height is too low! Can't show server state.\n"
                )
                .unwrap();
                *lines = lines.saturating_sub(2);
            }
            *lines = lines.saturating_sub(1);
        }

        // Print log (up to the remaining terminal lines, no more)
        for (time, desc) in self
            .event_log
            .iter()
            .take(lines.unwrap_or(usize::MAX))
            .rev()
        {
            if let Ok(duration) = time.duration_since(server.start_time) {
                write!(stdout, "{duration:<7.3?}: ").unwrap();
            }
            stdout.write_all(desc.as_bytes()).unwrap();
            stdout.write_all(b"\n").unwrap();
        }
    }
}

pub static CONSOLE: Mutex<Console> = const { Mutex::new(Console::new(32)) };

macro_rules! log {
    ($($arg:tt)*) => {{
        let mut cons = crate::console::CONSOLE.lock().unwrap();
        cons.push_event(format!($($arg)*));
    }};
}

pub(crate) use log;

pub fn render(server: &Server) {
    let cons = CONSOLE.lock().unwrap();
    cons.render(server);
}
