//! In-memory log ring buffer served to the desktop app, plus tracing setup.

use std::collections::VecDeque;
use std::sync::Mutex;

use splitwire_core::ipc::{LogEntry, LogLevel, LogSource};
use splitwire_core::now_ms;
use tracing_subscriber::EnvFilter;

const CAPACITY: usize = 2000;
/// Most entries returned by one `GetLogs`.
const PAGE: usize = 500;

pub struct LogBuffer {
    inner: Mutex<Inner>,
}

struct Inner {
    entries: VecDeque<LogEntry>,
    next_seq: u64,
}

impl LogBuffer {
    pub fn new() -> Self {
        Self { inner: Mutex::new(Inner { entries: VecDeque::with_capacity(CAPACITY), next_seq: 1 }) }
    }

    pub fn push(&self, level: LogLevel, source: LogSource, message: impl Into<String>) -> u64 {
        let message = message.into();
        match (source, level) {
            (LogSource::Service, LogLevel::Error) => tracing::error!("{message}"),
            (LogSource::Service, LogLevel::Warn) => tracing::warn!("{message}"),
            (LogSource::Service, _) => tracing::info!("{message}"),
            (LogSource::Engine, LogLevel::Error | LogLevel::Warn) => tracing::warn!(target: "sing-box", "{message}"),
            (LogSource::Engine, _) => tracing::debug!(target: "sing-box", "{message}"),
        }

        let mut inner = self.inner.lock().unwrap();
        let seq = inner.next_seq;
        inner.next_seq += 1;
        if inner.entries.len() == CAPACITY {
            inner.entries.pop_front();
        }
        inner.entries.push_back(LogEntry { seq, time: now_ms(), level, source, message });
        seq
    }

    pub fn info(&self, message: impl Into<String>) {
        self.push(LogLevel::Info, LogSource::Service, message);
    }

    pub fn warn(&self, message: impl Into<String>) {
        self.push(LogLevel::Warn, LogSource::Service, message);
    }

    pub fn error(&self, message: impl Into<String>) {
        self.push(LogLevel::Error, LogSource::Service, message);
    }

    pub fn since(&self, after: u64) -> Vec<LogEntry> {
        let inner = self.inner.lock().unwrap();
        inner.entries.iter().filter(|e| e.seq > after).take(PAGE).cloned().collect()
    }

    /// The most recent engine error logged after `after`, for failure messages.
    pub fn last_engine_error(&self, after: u64) -> Option<String> {
        let inner = self.inner.lock().unwrap();
        inner
            .entries
            .iter()
            .rev()
            .take_while(|e| e.seq > after)
            .find(|e| e.source == LogSource::Engine && e.level == LogLevel::Error)
            .map(|e| e.message.clone())
    }

    pub fn last_seq(&self) -> u64 {
        self.inner.lock().unwrap().next_seq - 1
    }
}

/// One parsed line of sing-box output.
pub struct EngineLine {
    pub level: LogLevel,
    pub message: String,
    /// Belongs to a single connection (`[id 12ms] ...`). These are noisy.
    pub per_connection: bool,
}

/// Parses sing-box's `LEVEL[0000] message` output (timestamps and colors off).
pub fn parse_engine_line(line: &str) -> EngineLine {
    let line = line.trim_end();
    let (level, message) = match line.split_once('[') {
        Some((prefix, rest)) if !prefix.is_empty() && prefix.chars().all(|c| c.is_ascii_uppercase()) => {
            let level = match prefix {
                "TRACE" | "DEBUG" => LogLevel::Debug,
                "INFO" => LogLevel::Info,
                "WARN" => LogLevel::Warn,
                _ => LogLevel::Error,
            };
            let message = rest.split_once("] ").map(|(_, m)| m).unwrap_or(rest);
            (level, message)
        }
        _ => (LogLevel::Info, line),
    };
    let per_connection = message.starts_with('[')
        && message.split_once("] ").is_some_and(|(tag, _)| tag.ends_with("ms"));
    EngineLine { level, message: message.to_string(), per_connection }
}

pub fn init_console_tracing() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::fmt().with_env_filter(filter).with_target(false).init();
}

/// Logs to daily files in `%ProgramData%\Splitwire\logs`, keeping a week.
pub fn init_service_tracing() -> Option<tracing_appender::non_blocking::WorkerGuard> {
    let dir = crate::paths::service_log_dir();
    std::fs::create_dir_all(&dir).ok()?;
    let appender = tracing_appender::rolling::Builder::new()
        .rotation(tracing_appender::rolling::Rotation::DAILY)
        .filename_prefix("service")
        .filename_suffix("log")
        .max_log_files(7)
        .build(&dir)
        .ok()?;
    let (writer, guard) = tracing_appender::non_blocking(appender);
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::fmt().with_env_filter(filter).with_ansi(false).with_writer(writer).init();
    Some(guard)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_engine_lines() {
        let line = parse_engine_line("INFO[0000] sing-box started (0.12s)");
        assert_eq!(line.level, LogLevel::Info);
        assert_eq!(line.message, "sing-box started (0.12s)");
        assert!(!line.per_connection);

        let line = parse_engine_line(
            "INFO[0012] [3816420147 0ms] inbound/tun[tun-in]: inbound connection from 198.18.0.1:52344",
        );
        assert!(line.per_connection);
        assert!(line.message.starts_with("[3816420147 0ms]"));

        let line = parse_engine_line("FATAL[0000] start service: open tun: access denied");
        assert_eq!(line.level, LogLevel::Error);
        assert_eq!(line.message, "start service: open tun: access denied");

        let line = parse_engine_line("plain output");
        assert_eq!(line.level, LogLevel::Info);
        assert_eq!(line.message, "plain output");
    }

    #[test]
    fn buffer_pages_and_finds_errors() {
        let logs = LogBuffer::new();
        let start = logs.last_seq();
        logs.push(LogLevel::Error, LogSource::Engine, "boom");
        logs.info("hello");
        assert_eq!(logs.since(start).len(), 2);
        assert_eq!(logs.last_engine_error(start).as_deref(), Some("boom"));
        assert_eq!(logs.last_engine_error(logs.last_seq()), None);
    }
}
