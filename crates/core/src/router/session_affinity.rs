use std::collections::{HashMap, VecDeque};

use http::HeaderMap;

/// Session header names, in precedence order. Kept in one place so the proxy
/// and the affinity store agree on what identifies a conversation.
pub const SESSION_HEADERS: [&str; 4] = [
    "x-opencode-session",
    "x-session-id",
    "prompt-cache-key",
    "x-session-affinity",
];

const CAPACITY: usize = 4096;

#[derive(Default)]
pub struct SessionAffinity {
    map: HashMap<String, String>,
    order: VecDeque<String>,
}

impl SessionAffinity {
    pub fn new() -> Self {
        SessionAffinity::default()
    }

    pub fn extract_session_key(headers: &HeaderMap) -> Option<String> {
        for name in SESSION_HEADERS {
            if let Some(value) = headers.get(name) {
                if let Ok(s) = value.to_str() {
                    if !s.is_empty() {
                        return Some(s.to_string());
                    }
                }
            }
        }
        None
    }

    pub fn preferred(&self, session: &str) -> Option<String> {
        self.map.get(session).cloned()
    }

    pub fn set(&mut self, session: &str, key_id: &str) {
        if !self.map.contains_key(session) {
            self.order.push_back(session.to_string());
        }
        self.map.insert(session.to_string(), key_id.to_string());
        while self.order.len() > CAPACITY {
            if let Some(oldest) = self.order.pop_front() {
                self.map.remove(&oldest);
            }
        }
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }
}
