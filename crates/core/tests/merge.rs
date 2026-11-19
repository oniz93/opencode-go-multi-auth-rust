use ocrouter_core::models::merge::Catalog;
use ocrouter_core::models::{Lane, ModelInfo};
use ocrouter_core::provider::Provider;

fn m(id: &str, lane: Lane) -> ModelInfo {
    let provider = match lane {
        Lane::Go => Provider::OpenCodeGo,
        Lane::Experiential => Provider::Experiential,
        Lane::Console => Provider::OpenCodeConsole,
    };
    ModelInfo {
        id: id.to_string(),
        name: id.to_string(),
        lane,
        provider,
        context_limit: None,
        output_limit: None,
        input_price: None,
        output_price: None,
        supports_tools: true,
        modalities_in: vec!["text".to_string()],
    }
}

#[test]
fn a_model_on_both_lanes_prefers_el_but_is_still_known_to_go() {
    let go = vec![m("deepseek-v4.1-flash", Lane::Go)];
    let el = vec![m("deepseek-v4.1-flash", Lane::Experiential)];
    let c = Catalog::merged(go, el);
    assert_eq!(c.lane_for("deepseek-v4.1-flash"), Some(Lane::Experiential));
    assert_eq!(c.models.len(), 1);
    assert!(c.has_go_fallback("deepseek-v4.1-flash"));
}

#[test]
fn an_el_only_model_resolves_to_el_with_no_go_fallback() {
    let c = Catalog::merged(vec![m("kimi-k3", Lane::Go)], vec![m("clef", Lane::Experiential)]);
    assert_eq!(c.lane_for("clef"), Some(Lane::Experiential));
    assert!(!c.has_go_fallback("clef"));
    assert_eq!(c.lane_for("kimi-k3"), Some(Lane::Go));
    assert!(c.has_go_fallback("kimi-k3"));
}

#[test]
fn an_unknown_model_resolves_to_none() {
    let c = Catalog::merged(vec![], vec![]);
    assert_eq!(c.lane_for("nope"), None);
    assert!(!c.has_go_fallback("nope"));
}

#[test]
fn dedupe_keeps_one_entry_per_id() {
    let go = vec![m("a", Lane::Go), m("b", Lane::Go)];
    let el = vec![m("b", Lane::Experiential), m("c", Lane::Experiential)];
    let c = Catalog::merged(go, el);
    let mut ids: Vec<&str> = c.models.iter().map(|m| m.id.as_str()).collect();
    ids.sort();
    assert_eq!(ids, vec!["a", "b", "c"]);
    // the surviving "b" is the Experiential entry
    let b = c.models.iter().find(|m| m.id == "b").unwrap();
    assert_eq!(b.lane, Lane::Experiential);
    assert!(c.has_go_fallback("b"));
}

#[test]
fn models_are_sorted_by_id_for_stable_output() {
    let c = Catalog::merged(vec![m("z", Lane::Go), m("a", Lane::Go)], vec![]);
    let ids: Vec<&str> = c.models.iter().map(|m| m.id.as_str()).collect();
    assert_eq!(ids, vec!["a", "z"]);
}

#[test]
fn zen_models_fill_only_ids_go_and_el_do_not_serve() {
    let catalog = Catalog::merged(
        vec![m("deepseek-v4.1-flash", Lane::Go)],
        vec![m("deepseek-v4.1-flash-1017", Lane::Experiential)],
    )
    .with_console(vec![
        m("deepseek-v4.1-flash", Lane::Console),
        m("deepseek-v4.1-flash-free", Lane::Console),
    ]);
    assert_eq!(catalog.models.len(), 3);
    assert_eq!(catalog.lane_for("deepseek-v4.1-flash"), Some(Lane::Go));
    assert_eq!(catalog.lane_for("deepseek-v4.1-flash-free"), Some(Lane::Console));
    let ids: Vec<&str> = catalog.models.iter().map(|m| m.id.as_str()).collect();
    assert!(ids.windows(2).all(|w| w[0] <= w[1]), "kept sorted: {ids:?}");
}
