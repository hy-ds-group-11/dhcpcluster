//! # User interface for a unix terminal
//! Renders server's internal state and latest messages in a consistent way
//! Intended to be used via the global static [`CONSOLE`] and the macros defined in this crate.

use std::{
    collections::VecDeque,
    io::{IsTerminal, Write},
    sync::Mutex,
    time::SystemTime,
};
use terminal_size::{terminal_size, Height};

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

    fn render(&self, start_time: SystemTime, state: &str) {
        let mut stdout = std::io::stdout().lock();

        // Get terminal height
        let mut lines: Option<usize> = terminal_size().map(|(_, Height(h))| h.into());

        // Clear terminal and move cursor to top left
        stdout.write_all(b"\x1B[2J\x1B[H").unwrap();

        if let Some(lines) = &mut lines {
            // Subtract from remaining terminal lines available
            let remaining = lines.saturating_sub(state.lines().count());
            if remaining > 0 {
                stdout.write_all(state.as_bytes()).unwrap();
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
            if let Ok(duration) = time.duration_since(start_time) {
                write!(stdout, "\x1B[90m{duration:<9.3?}:\x1B[0m ").unwrap();
            }
            stdout.write_all(desc.as_bytes()).unwrap();
            stdout.write_all(b"\n").unwrap();
        }
    }
}

pub static CONSOLE: Mutex<Console> = const { Mutex::new(Console::new(1024)) };

pub fn render(start_time: SystemTime, state: &str) {
    if std::io::stdout().is_terminal() {
        let cons = CONSOLE.lock().unwrap();
        cons.render(start_time, state);
    }
    // Otherwise do nothing, because [`log_str`] already prints the event as is
}

pub fn log_str(event: &str, style: &str) {
    if std::io::stdout().is_terminal() {
        let mut cons = crate::console::CONSOLE.lock().unwrap();
        for line in event.split("\n") {
            // TODO: Encode messages to a struct with a style-enum and apply escape sequences in [`render`]
            cons.push_event(format!("\x1B{style}{line}\x1B[0m"));
        }
    } else {
        // Don't write control characters, just output lines as is
        println!("{event}");
    }
}

macro_rules! debug {
    ($($arg:tt)*) => {{
            if std::env::var("SERVER_VERBOSE").is_ok() {
                crate::console::log_str(&format!($($arg)*), "[90m");
            }
    }};
}

macro_rules! error {
    ($($arg:tt)*) => {{
            crate::console::log_str(&format!($($arg)*), "[31m");
    }};
}

macro_rules! log {
    ($($arg:tt)*) => {{
            crate::console::log_str(&format!($($arg)*), "[0m");
    }};
}

macro_rules! warning {
    ($($arg:tt)*) => {{
            crate::console::log_str(&format!($($arg)*), "[33m");
    }};
}

pub(crate) use {debug, error, log, warning};
