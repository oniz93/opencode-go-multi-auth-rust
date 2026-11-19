use std::collections::{BTreeMap, HashSet};

use super::catalog::{Lane, ModelInfo};

/// The client-facing model catalog: one entry per model id, plus the knowledge
/// of which ids also exist on the Go lane so a request can be retried there
/// once the Experiential allowance is spent.
pub struct Catalog {
    pub models: Vec<ModelInfo>,
    go_ids: HashSet<String>,
}

impl Catalog {
    /// Experiential entries win a shared id (they are free-first); the Go
    /// entry's existence is remembered as a fallback target.
    pub fn merged(go: Vec<ModelInfo>, el: Vec<ModelInfo>) -> Catalog {
        let go_ids: HashSet<String> = go.iter().map(|m| m.id.clone()).collect();
        let mut by_id: BTreeMap<String, ModelInfo> = BTreeMap::new();
        for model in go {
            by_id.insert(model.id.clone(), model);
        }
        for model in el {
            by_id.insert(model.id.clone(), model);
        }
        Catalog {
            models: by_id.into_values().collect(),
            go_ids,
        }
    }

    /// Add Zen (Console) models. They only fill ids Go and Experiential do not
    /// serve, so a shared id keeps routing to those lanes.
    pub fn with_console(mut self, console: Vec<ModelInfo>) -> Catalog {
        for model in console {
            if self.get(&model.id).is_none() {
                self.models.push(model);
            }
        }
        self.models.sort_by(|a, b| a.id.cmp(&b.id));
        self
    }

    pub fn lane_for(&self, model_id: &str) -> Option<Lane> {
        self.models
            .iter()
            .find(|m| m.id == model_id)
            .map(|m| m.lane)
    }

    pub fn has_go_fallback(&self, model_id: &str) -> bool {
        self.go_ids.contains(model_id)
    }

    pub fn get(&self, model_id: &str) -> Option<&ModelInfo> {
        self.models.iter().find(|m| m.id == model_id)
    }
}
