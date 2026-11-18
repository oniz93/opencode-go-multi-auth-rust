use std::collections::HashMap;

use crate::time::utc_day_index;

struct OrgUsage {
    day: u64,
    used: u64,
}

/// The Experiential free allowance is shared per **organisation** and resets at
/// 00:00 UTC. Bucketing by `org_id` (not by key) is what stops two keys from one
/// org each claiming the full cap.
pub struct Allowance {
    pub cap: u64,
    pub safety_margin: f64,
    orgs: HashMap<String, OrgUsage>,
}

impl Allowance {
    pub fn new(cap: u64, safety_margin: f64) -> Self {
        Allowance {
            cap,
            safety_margin,
            orgs: HashMap::new(),
        }
    }

    /// The usable budget: the cap less the safety margin, which reserves
    /// headroom so a local counter that drifts behind the org's real usage
    /// cannot trip the upstream limit into a paid fallback.
    pub fn limit(&self) -> u64 {
        let margin = self.safety_margin.clamp(0.0, 1.0);
        ((self.cap as f64) * (1.0 - margin)).floor() as u64
    }

    pub fn record(&mut self, org: &str, tokens: u64, now_ms: u64) {
        let day = utc_day_index(now_ms);
        let entry = self.orgs.entry(org.to_string()).or_insert(OrgUsage { day, used: 0 });
        if entry.day != day {
            entry.day = day;
            entry.used = 0;
        }
        entry.used = entry.used.saturating_add(tokens);
    }

    pub fn used(&self, org: &str) -> u64 {
        self.orgs.get(org).map(|e| e.used).unwrap_or(0)
    }

    pub fn budget_left(&self, org: &str) -> u64 {
        self.limit().saturating_sub(self.used(org))
    }

    pub fn is_exhausted(&self, org: &str) -> bool {
        self.budget_left(org) == 0
    }

    pub fn reset_if_new_utc_day(&mut self, now_ms: u64) {
        let day = utc_day_index(now_ms);
        for entry in self.orgs.values_mut() {
            if entry.day != day {
                entry.day = day;
                entry.used = 0;
            }
        }
    }

    /// `(org, used_today, limit)` per org, sorted by org for stable output.
    pub fn snapshot(&self) -> Vec<(String, u64, u64)> {
        let limit = self.limit();
        let mut rows: Vec<(String, u64, u64)> = self
            .orgs
            .iter()
            .map(|(org, e)| (org.clone(), e.used, limit))
            .collect();
        rows.sort_by(|a, b| a.0.cmp(&b.0));
        rows
    }
}
