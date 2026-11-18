use serde::{Deserialize, Serialize};

use crate::config::Config;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Provider {
    #[serde(rename = "opencode-go")]
    OpenCodeGo,
    #[serde(rename = "opencode-console")]
    OpenCodeConsole,
    #[serde(rename = "experiential")]
    Experiential,
}

impl Provider {
    pub fn as_str(&self) -> &'static str {
        match self {
            Provider::OpenCodeGo => "opencode-go",
            Provider::OpenCodeConsole => "opencode-console",
            Provider::Experiential => "experiential",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthStyle {
    BearerAndApiKey,
    BearerOnly,
}

#[derive(Debug, Clone)]
pub struct Upstream {
    pub provider: Provider,
    pub base_url: String,
    pub model_list_path: &'static str,
    pub auth: AuthStyle,
}

pub fn upstream_for(provider: Provider, cfg: &Config) -> Upstream {
    let (base_url, auth) = match provider {
        Provider::OpenCodeGo => (cfg.go_base_url.clone(), AuthStyle::BearerAndApiKey),
        Provider::OpenCodeConsole => (cfg.oc_base_url.clone(), AuthStyle::BearerAndApiKey),
        Provider::Experiential => (cfg.el_base_url.clone(), AuthStyle::BearerOnly),
    };
    Upstream {
        provider,
        base_url,
        model_list_path: "/v1/models",
        auth,
    }
}
