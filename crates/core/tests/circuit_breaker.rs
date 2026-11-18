use ocrouter_core::router::circuit_breaker::{CircuitBreaker, CircuitState};

#[test]
fn opens_only_after_threshold_consecutive_failures() {
    let mut cb = CircuitBreaker::new(3, 60_000);
    assert_eq!(cb.record_failure("k", 0), CircuitState::Closed);
    assert_eq!(cb.record_failure("k", 0), CircuitState::Closed);
    assert_eq!(cb.record_failure("k", 0), CircuitState::Open);
    assert!(!cb.is_available("k", 0));
}

#[test]
fn success_resets_the_counter() {
    let mut cb = CircuitBreaker::new(3, 60_000);
    cb.record_failure("k", 0);
    cb.record_failure("k", 0);
    cb.record_success("k");
    assert_eq!(cb.record_failure("k", 0), CircuitState::Closed);
}

#[test]
fn half_opens_after_open_ms() {
    let mut cb = CircuitBreaker::new(1, 10_000);
    cb.record_failure("k", 0);
    assert!(!cb.is_available("k", 5_000));
    assert!(cb.is_available("k", 10_000));
    assert_eq!(cb.state_of("k", 10_000), CircuitState::HalfOpen);
}

#[test]
fn one_bad_key_does_not_open_another() {
    let mut cb = CircuitBreaker::new(2, 60_000);
    cb.record_failure("a", 0);
    cb.record_failure("a", 0);
    assert!(!cb.is_available("a", 0));
    assert!(cb.is_available("b", 0));
    assert_eq!(cb.state_of("b", 0), CircuitState::Closed);
}

#[test]
fn an_unknown_key_is_available() {
    let cb = CircuitBreaker::new(3, 60_000);
    assert!(cb.is_available("never-seen", 0));
    assert_eq!(cb.state_of("never-seen", 0), CircuitState::Closed);
}
