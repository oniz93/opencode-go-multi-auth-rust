pub mod config;
pub mod events;
pub mod keys;
pub mod models;
pub mod provider;
pub mod proxy;
pub mod router;
pub mod state;
pub mod time;

pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

#[test]
fn version_is_non_empty() {
    assert!(!crate::version().is_empty());
}
