use std::collections::HashSet;

use crate::keys::KeyFile;
use crate::provider::Provider;

use super::types::{ApiKey, KeySelection, KeyStatus, RoutingStrategy};

pub struct KeyManager {
    keys: Vec<ApiKey>,
    rr_cursor: usize,
}

fn is_eligible(key: &ApiKey, exclude: &HashSet<String>, now_ms: u64) -> bool {
    if !key.enabled || exclude.contains(&key.id) {
        return false;
    }
    match key.status {
        KeyStatus::Disabled => false,
        KeyStatus::Active => true,
        // A cooled key comes back on its own once its cooldown lapses.
        KeyStatus::Exhausted => key.cooldown_until_ms.is_some_and(|t| t <= now_ms),
    }
}

impl KeyManager {
    pub fn new(keys: Vec<ApiKey>) -> Self {
        KeyManager { keys, rr_cursor: 0 }
    }

    pub fn from_keyfile(kf: &KeyFile) -> Self {
        let keys = kf
            .key
            .iter()
            .map(|e| ApiKey {
                id: ApiKey::new_id(e.provider, &e.label),
                provider: e.provider,
                label: e.label.clone(),
                secret: e.secret.clone(),
                priority: e.priority,
                weight: e.weight,
                enabled: e.enabled,
                status: KeyStatus::Active,
                consecutive_errors: 0,
                cooldown_until_ms: None,
            })
            .collect();
        KeyManager::new(keys)
    }

    pub fn all(&self) -> &[ApiKey] {
        &self.keys
    }

    pub fn enabled(&self) -> Vec<&ApiKey> {
        self.keys.iter().filter(|k| k.enabled).collect()
    }

    pub fn get(&self, id: &str) -> Option<&ApiKey> {
        self.keys.iter().find(|k| k.id == id)
    }

    pub fn add(&mut self, key: ApiKey) {
        if let Some(existing) = self.keys.iter_mut().find(|k| k.id == key.id) {
            *existing = key;
        } else {
            self.keys.push(key);
        }
    }

    pub fn remove(&mut self, id: &str) -> bool {
        let before = self.keys.len();
        self.keys.retain(|k| k.id != id);
        self.keys.len() != before
    }

    pub fn eligible(&self, exclude: &HashSet<String>, now_ms: u64) -> Vec<&ApiKey> {
        self.keys
            .iter()
            .filter(|k| is_eligible(k, exclude, now_ms))
            .collect()
    }

    pub fn select(
        &mut self,
        strategy: RoutingStrategy,
        exclude: &HashSet<String>,
        now_ms: u64,
    ) -> Option<KeySelection> {
        self.select_matching(None, strategy, exclude, now_ms)
    }

    /// Same as `select`, restricted to one provider's keys so a lane only ever
    /// spends its own credentials.
    pub fn select_for(
        &mut self,
        provider: Provider,
        strategy: RoutingStrategy,
        exclude: &HashSet<String>,
        now_ms: u64,
    ) -> Option<KeySelection> {
        self.select_matching(Some(provider), strategy, exclude, now_ms)
    }

    fn select_matching(
        &mut self,
        provider: Option<Provider>,
        strategy: RoutingStrategy,
        exclude: &HashSet<String>,
        now_ms: u64,
    ) -> Option<KeySelection> {
        let mut idxs: Vec<usize> = (0..self.keys.len())
            .filter(|&i| is_eligible(&self.keys[i], exclude, now_ms))
            .filter(|&i| provider.is_none_or(|p| self.keys[i].provider == p))
            .collect();
        if idxs.is_empty() {
            return None;
        }

        let chosen = match strategy {
            RoutingStrategy::RoundRobin => {
                let pos = self.rr_cursor % idxs.len();
                self.rr_cursor = self.rr_cursor.wrapping_add(1);
                idxs[pos]
            }
            RoutingStrategy::PriorityFailover | RoutingStrategy::SessionAffinity => {
                idxs.sort_by(|&a, &b| {
                    let ka = &self.keys[a];
                    let kb = &self.keys[b];
                    ka.priority
                        .cmp(&kb.priority)
                        .then(kb.weight.cmp(&ka.weight))
                        .then(ka.id.cmp(&kb.id))
                });
                idxs[0]
            }
        };

        let key = self.keys[chosen].clone();
        let reason = format!("{} selected by {strategy:?}", key.id);
        Some(KeySelection { key, reason })
    }

    pub fn mark_exhausted(&mut self, id: &str, until_ms: u64) {
        if let Some(k) = self.keys.iter_mut().find(|k| k.id == id) {
            k.status = KeyStatus::Exhausted;
            k.cooldown_until_ms = Some(until_ms);
        }
    }

    pub fn record_success(&mut self, id: &str) {
        if let Some(k) = self.keys.iter_mut().find(|k| k.id == id) {
            k.consecutive_errors = 0;
        }
    }

    /// Whether `id` could be picked right now (enabled, not cooling, not excluded).
    pub fn is_selectable(&self, id: &str, exclude: &HashSet<String>, now_ms: u64) -> bool {
        self.get(id)
            .is_some_and(|k| is_eligible(k, exclude, now_ms))
    }

    /// Bring the pool in line with `keys.toml`: new keys join, removed keys
    /// leave, edited fields update, and each surviving key keeps its health.
    pub fn sync_from_keyfile(&mut self, kf: &KeyFile) {
        let fresh = KeyManager::from_keyfile(kf).keys;
        let old = std::mem::take(&mut self.keys);
        self.keys = fresh
            .into_iter()
            .map(|mut key| {
                if let Some(prev) = old.iter().find(|k| k.id == key.id) {
                    key.status = prev.status;
                    key.consecutive_errors = prev.consecutive_errors;
                    key.cooldown_until_ms = prev.cooldown_until_ms;
                }
                key
            })
            .collect();
    }

    pub fn record_error(&mut self, id: &str) {
        if let Some(k) = self.keys.iter_mut().find(|k| k.id == id) {
            k.consecutive_errors += 1;
        }
    }

    pub fn iter_mut(&mut self) -> impl Iterator<Item = &mut ApiKey> {
        self.keys.iter_mut()
    }
}
