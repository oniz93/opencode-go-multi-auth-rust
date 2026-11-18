pub const MS_PER_DAY: u64 = 86_400_000;

/// Days since the Unix epoch, UTC. The Experiential free allowance resets at
/// 00:00 UTC, so a day index is the natural window key.
pub fn utc_day_index(ms: u64) -> u64 {
    ms / MS_PER_DAY
}

/// The first instant of the next UTC day after `ms`.
pub fn utc_midnight_after(ms: u64) -> u64 {
    (utc_day_index(ms) + 1) * MS_PER_DAY
}
