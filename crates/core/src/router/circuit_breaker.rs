use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CircuitState {
    Closed,
    Open,
    HalfOpen,
}

struct Entry {
    consecutive_failures: u32,
    opened_at_ms: Option<u64>,
}

pub struct CircuitBreaker {
    threshold: u32,
    open_ms: u64,
    entries: HashMap<String, Entry>,
}

impl CircuitBreaker {
    pub fn new(threshold: u32, open_ms: u64) -> Self {
        CircuitBreaker {
            threshold: threshold.max(1),
            open_ms,
            entries: HashMap::new(),
        }
    }

    pub fn state_of(&self, id: &str, now_ms: u64) -> CircuitState {
        match self.entries.get(id).and_then(|e| e.opened_at_ms) {
            None => CircuitState::Closed,
            Some(opened) if now_ms.saturating_sub(opened) >= self.open_ms => CircuitState::HalfOpen,
            Some(_) => CircuitState::Open,
        }
    }

    pub fn is_available(&self, id: &str, now_ms: u64) -> bool {
        self.state_of(id, now_ms) != CircuitState::Open
    }

    pub fn record_failure(&mut self, id: &str, now_ms: u64) -> CircuitState {
        let entry = self.entries.entry(id.to_string()).or_insert(Entry {
            consecutive_failures: 0,
            opened_at_ms: None,
        });
        entry.consecutive_failures += 1;
        if entry.opened_at_ms.is_none() && entry.consecutive_failures >= self.threshold {
            entry.opened_at_ms = Some(now_ms);
        }
        self.state_of(id, now_ms)
    }

    pub fn record_success(&mut self, id: &str) {
        if let Some(entry) = self.entries.get_mut(id) {
            entry.consecutive_failures = 0;
            entry.opened_at_ms = None;
        }
    }

    pub fn consecutive_failures(&self, id: &str) -> u32 {
        self.entries.get(id).map(|e| e.consecutive_failures).unwrap_or(0)
    }
}
