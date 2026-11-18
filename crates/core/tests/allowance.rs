use ocrouter_core::router::allowance::Allowance;
use ocrouter_core::time::utc_midnight_after;

#[test]
fn two_keys_one_org_share_one_budget() {
    let mut a = Allowance::new(1_000_000, 0.0);
    a.record("org-1", 400_000, 0);
    // a second key on the same org adds to the SAME bucket
    a.record("org-1", 300_000, 0);
    assert_eq!(a.used("org-1"), 700_000);
    assert_eq!(a.budget_left("org-1"), 300_000);
}

#[test]
fn separate_orgs_are_separate_buckets() {
    let mut a = Allowance::new(1_000_000, 0.0);
    a.record("org-1", 900_000, 0);
    a.record("org-2", 10_000, 0);
    assert_eq!(a.budget_left("org-1"), 100_000);
    assert_eq!(a.budget_left("org-2"), 990_000);
}

#[test]
fn safety_margin_reserves_headroom() {
    let mut a = Allowance::new(1_000_000, 0.05);
    assert_eq!(a.budget_left("org-1"), 950_000);
    a.record("org-1", 950_000, 0);
    assert!(a.is_exhausted("org-1"));
}

#[test]
fn budget_left_never_underflows() {
    let mut a = Allowance::new(1_000_000, 0.0);
    a.record("org-1", 5_000_000, 0);
    assert_eq!(a.budget_left("org-1"), 0);
    assert!(a.is_exhausted("org-1"));
}

#[test]
fn resets_at_the_next_utc_midnight() {
    let mut a = Allowance::new(1_000_000, 0.0);
    a.record("org-1", 999_999, 0);
    assert!(a.is_exhausted("org-1") == false);
    a.reset_if_new_utc_day(utc_midnight_after(0));
    assert_eq!(a.used("org-1"), 0);
    assert_eq!(a.budget_left("org-1"), 1_000_000);
}

#[test]
fn a_reset_does_not_leak_across_orgs_added_before_it() {
    let mut a = Allowance::new(1_000_000, 0.0);
    a.record("org-1", 500_000, 0);
    a.reset_if_new_utc_day(utc_midnight_after(0));
    assert_eq!(a.used("org-1"), 0);
    // a record after the reset lands in the new day
    a.record("org-1", 10, utc_midnight_after(0));
    assert_eq!(a.used("org-1"), 10);
}

#[test]
fn snapshot_reports_used_and_limit_per_org() {
    let mut a = Allowance::new(1_000_000, 0.05);
    a.record("org-1", 100, 0);
    let snap = a.snapshot();
    assert_eq!(snap.len(), 1);
    assert_eq!(snap[0].0, "org-1");
    assert_eq!(snap[0].1, 100);
    assert_eq!(snap[0].2, 950_000);
}
