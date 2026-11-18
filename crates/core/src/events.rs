use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Level {
    Info,
    Warn,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventRecord {
    pub at_ms: u64,
    pub level: Level,
    pub message: String,
    pub detail: Option<serde_json::Value>,
}

/// A bounded in-memory record of routing events, plus a live broadcast for
/// subscribers (the dashboard's SSE stream). `Clone` shares the same buffer.
#[derive(Clone)]
pub struct EventBus {
    buffer: Arc<Mutex<VecDeque<EventRecord>>>,
    capacity: usize,
    tx: broadcast::Sender<EventRecord>,
}

impl EventBus {
    pub fn new(capacity: usize) -> Self {
        let (tx, _rx) = broadcast::channel(capacity.max(1) * 4);
        EventBus {
            buffer: Arc::new(Mutex::new(VecDeque::new())),
            capacity,
            tx,
        }
    }

    pub fn emit(&self, record: EventRecord) {
        {
            let mut buffer = self.buffer.lock().expect("event buffer poisoned");
            buffer.push_back(record.clone());
            while buffer.len() > self.capacity {
                buffer.pop_front();
            }
        }
        // A send error only means nobody is subscribed right now.
        let _ = self.tx.send(record);
    }

    pub fn recent(&self, n: usize) -> Vec<EventRecord> {
        let buffer = self.buffer.lock().expect("event buffer poisoned");
        let start = buffer.len().saturating_sub(n);
        buffer.iter().skip(start).cloned().collect()
    }

    pub fn subscribe(&self) -> broadcast::Receiver<EventRecord> {
        self.tx.subscribe()
    }
}
