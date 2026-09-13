//! A bounded log buffer per project.
//!
//! Long-running dev servers are chatty. Keeping every line would grow without limit, so the
//! buffer holds the most recent `CAPACITY` lines and drops the oldest. Each line carries a
//! monotonically increasing sequence number, which lets the frontend ask for "everything
//! after what I already have" without re-sending the whole buffer.

use serde::Serialize;
use std::collections::VecDeque;

pub const CAPACITY: usize = 1000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Stream {
    Stdout,
    Stderr,
    /// Emitted by Oracle itself, not the child process.
    System,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LogLine {
    pub seq: u64,
    pub stream: Stream,
    pub text: String,
    /// Milliseconds since the Unix epoch.
    pub at: i64,
}

#[derive(Debug, Default)]
pub struct LogRing {
    lines: VecDeque<LogLine>,
    next_seq: u64,
}

impl LogRing {
    pub fn new() -> Self {
        Self {
            lines: VecDeque::with_capacity(CAPACITY),
            next_seq: 0,
        }
    }

    pub fn push(&mut self, stream: Stream, text: impl Into<String>) -> LogLine {
        let line = LogLine {
            seq: self.next_seq,
            stream,
            text: text.into(),
            at: chrono::Utc::now().timestamp_millis(),
        };
        self.next_seq += 1;

        if self.lines.len() == CAPACITY {
            self.lines.pop_front();
        }
        self.lines.push_back(line.clone());

        line
    }

    /// Every line currently held, oldest first.
    pub fn all(&self) -> Vec<LogLine> {
        self.lines.iter().cloned().collect()
    }

    /// Lines with a sequence number at or above `seq`. Used to catch a reconnecting
    /// frontend up without resending what it already has.
    pub fn since(&self, seq: u64) -> Vec<LogLine> {
        self.lines
            .iter()
            .filter(|line| line.seq >= seq)
            .cloned()
            .collect()
    }

    pub fn clear(&mut self) {
        self.lines.clear();
    }

    pub fn len(&self) -> usize {
        self.lines.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sequence_numbers_keep_climbing_past_the_capacity() {
        let mut ring = LogRing::new();
        for i in 0..(CAPACITY + 10) {
            ring.push(Stream::Stdout, format!("line {i}"));
        }

        assert_eq!(ring.len(), CAPACITY);

        let all = ring.all();
        // The first ten lines were evicted, so the oldest survivor is line 10.
        assert_eq!(all.first().unwrap().text, "line 10");
        assert_eq!(all.first().unwrap().seq, 10);
        assert_eq!(all.last().unwrap().seq, (CAPACITY + 9) as u64);
    }

    #[test]
    fn since_returns_only_newer_lines() {
        let mut ring = LogRing::new();
        for i in 0..5 {
            ring.push(Stream::Stdout, format!("line {i}"));
        }

        let tail = ring.since(3);
        assert_eq!(tail.len(), 2);
        assert_eq!(tail[0].text, "line 3");
        assert_eq!(tail[1].text, "line 4");
    }

    #[test]
    fn since_beyond_the_end_returns_nothing() {
        let mut ring = LogRing::new();
        ring.push(Stream::Stdout, "only");

        assert!(ring.since(99).is_empty());
    }

    #[test]
    fn clearing_keeps_the_sequence_running() {
        let mut ring = LogRing::new();
        ring.push(Stream::Stdout, "before");
        ring.clear();
        let after = ring.push(Stream::System, "after");

        // Sequence numbers must never repeat, or the frontend would drop the new line as
        // something it had already seen.
        assert_eq!(after.seq, 1);
        assert_eq!(ring.len(), 1);
    }

    #[test]
    fn streams_are_preserved() {
        let mut ring = LogRing::new();
        ring.push(Stream::Stdout, "out");
        ring.push(Stream::Stderr, "err");
        ring.push(Stream::System, "sys");

        let all = ring.all();
        assert_eq!(all[0].stream, Stream::Stdout);
        assert_eq!(all[1].stream, Stream::Stderr);
        assert_eq!(all[2].stream, Stream::System);
    }
}
