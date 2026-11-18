use ocrouter_core::events::{EventBus, EventRecord, Level};

fn rec(i: u64) -> EventRecord {
    EventRecord {
        at_ms: i,
        level: Level::Info,
        message: format!("e{i}"),
        detail: None,
    }
}

#[test]
fn ring_buffer_keeps_only_the_last_capacity() {
    let bus = EventBus::new(3);
    for i in 0..5 {
        bus.emit(rec(i));
    }
    let recent = bus.recent(10);
    assert_eq!(recent.len(), 3);
    assert_eq!(recent[0].message, "e2");
    assert_eq!(recent[2].message, "e4");
}

#[test]
fn recent_is_bounded_by_the_requested_count() {
    let bus = EventBus::new(10);
    for i in 0..5 {
        bus.emit(rec(i));
    }
    assert_eq!(bus.recent(2).len(), 2);
    assert_eq!(bus.recent(2)[1].message, "e4");
    assert_eq!(bus.recent(0).len(), 0);
}

#[test]
fn an_empty_bus_has_no_events() {
    let bus = EventBus::new(8);
    assert!(bus.recent(5).is_empty());
}

#[tokio::test]
async fn subscribers_receive_new_events() {
    let bus = EventBus::new(4);
    let mut rx = bus.subscribe();
    bus.emit(rec(7));
    let got = rx.recv().await.unwrap();
    assert_eq!(got.message, "e7");
}

#[test]
fn emitting_with_no_subscribers_does_not_panic() {
    let bus = EventBus::new(2);
    bus.emit(rec(1));
    assert_eq!(bus.recent(1).len(), 1);
}
