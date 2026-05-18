//! Token counting and drift calibration.

/// Count tokens locally (fast, no network).
pub fn count_local(text: &str) -> u32 {
    // Phase 0 stub: rough word-based estimate.
    text.split_whitespace().count() as u32
}
