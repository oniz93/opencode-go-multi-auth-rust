use http::HeaderMap;

use crate::models::{Catalog, Lane};
use crate::provider::Provider;
use crate::router::allowance::Allowance;
use crate::router::circuit_breaker::CircuitBreaker;
use crate::router::key_manager::KeyManager;

use super::quota::{is_quota_error, resolve_cooldown_ms};
use super::usage::TokenUsage;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LaneChoice {
    Experiential,
    Go,
    Console,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Attempt {
    Retry,
    Stop,
}

/// Pick the lane for a model. A model Experiential serves prefers EL (free
/// first) and falls back to Go when the org's allowance is spent or EL is
/// unusable. `el_org: None` means "EL is not usable" (no key, or refused).
pub fn choose_lane(
    model: &str,
    catalog: &Catalog,
    allowance: &Allowance,
    el_org: Option<&str>,
    allow_paid: bool,
) -> Option<LaneChoice> {
    match catalog.lane_for(model)? {
        Lane::Experiential => {
            if let Some(org) = el_org {
                // `allow_paid` deliberately lets paid overflow happen once the
                // free allowance is spent; the owner opts in.
                if allow_paid || !allowance.is_exhausted(org) {
                    return Some(LaneChoice::Experiential);
                }
            }
            // The free allowance is gone: retry the same id on Go when it exists.
            if catalog.has_go_fallback(model) {
                Some(LaneChoice::Go)
            } else {
                None
            }
        }
        Lane::Go => Some(LaneChoice::Go),
        Lane::Console => Some(LaneChoice::Console),
    }
}

pub struct ResponseSignal<'a> {
    pub status: u16,
    pub headers: &'a HeaderMap,
    pub body_peek: &'a str,
    pub key_id: &'a str,
    pub provider: Provider,
    pub org: Option<&'a str>,
    pub usage: Option<TokenUsage>,
    pub now_ms: u64,
    pub fallback_cooldown_ms: u64,
    /// `false` when this same body already produced a 5xx on another key, so a
    /// bad request cannot evict every healthy key.
    pub count_breaker_failure: bool,
    /// Route chains move on at any 429, not just a recognised quota wall.
    pub advance_on_429: bool,
}

pub fn classify(
    signal: ResponseSignal,
    breaker: &mut CircuitBreaker,
    keys: &mut KeyManager,
    allowance: &mut Allowance,
) -> Attempt {
    let quota = is_quota_error(signal.status, signal.headers, signal.body_peek);
    if quota || (signal.status == 429 && signal.advance_on_429) {
        // shortcut: a plain rate limit with no reset hint parks the key for 60s; make it configurable if that proves wrong.
        let fallback = if quota {
            signal.fallback_cooldown_ms
        } else {
            60_000
        };
        let cooldown =
            resolve_cooldown_ms(signal.headers, signal.body_peek, signal.now_ms, fallback);
        keys.mark_exhausted(signal.key_id, signal.now_ms.saturating_add(cooldown));
        return Attempt::Retry;
    }

    // The proxy owns the credential, so a 401 is this key's fault (revoked or
    // mistyped): sideline it and fail over instead of failing every request.
    if signal.status == 401 {
        keys.mark_exhausted(
            signal.key_id,
            signal.now_ms.saturating_add(signal.fallback_cooldown_ms),
        );
        keys.record_error(signal.key_id);
        return Attempt::Retry;
    }

    if signal.status >= 500 {
        if signal.count_breaker_failure {
            breaker.record_failure(signal.key_id, signal.now_ms);
            keys.record_error(signal.key_id);
        }
        return Attempt::Retry;
    }

    breaker.record_success(signal.key_id);
    keys.record_success(signal.key_id);
    if signal.provider == Provider::Experiential {
        if let (Some(org), Some(usage)) = (signal.org, signal.usage) {
            allowance.record(org, usage.total(), signal.now_ms);
        }
    }
    Attempt::Stop
}
