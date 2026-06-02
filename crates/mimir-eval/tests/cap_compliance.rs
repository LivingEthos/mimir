//! Cap compliance 100% gate.
//!
//! Runs the full 15-case `context-recall-v1` fixture dataset across every mode
//! present in the fixture (0, 2, 3, 4, 5) through the eval harness's public API
//! ([`run_context_dataset`]) and asserts cap compliance for **every** built
//! packet:
//!
//! 1. The authoritative input token count (`estimated_input_tokens`, surfaced as
//!    `metrics.tokens_in_total`) stays at or below the cap.
//! 2. The gateway-required budget (`estimated_input_tokens + output_reserve +
//!    count_drift_reserve`) also stays at or below the cap.
//!
//! The dataset's `allowed_caps_to_test` is `[64000]`, matching
//! [`TokenPolicy::default().cap_tokens`]. This test is provider-free: the harness
//! never dispatches to a provider, so no network/provider calls occur.

use std::path::{Path, PathBuf};

use mimir_context::TokenPolicy;
use mimir_eval::{load_context_dataset, run_context_dataset};

/// The single cap declared by the fixture's `allowed_caps_to_test`.
const CAP_TOKENS: u32 = 64_000;

/// Every eval-mode label the fixture exercises (modes 0, 2, 3, 4, 5).
const EXPECTED_MODES: [&str; 5] = [
    "0-baseline",
    "2-rank",
    "3-rank-recall",
    "4-rank-recall-subagents",
    "5-full",
];

fn fixture_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/context-recall-v1.yaml")
}

/// Sanity-check the fixture contract this gate relies on: 15 cases, and the only
/// cap any case asks to be tested at is the policy default (64000).
#[test]
fn fixture_declares_single_default_cap() {
    let dataset = load_context_dataset(fixture_path()).expect("fixture dataset must load");
    assert_eq!(dataset.cases.len(), 15, "fixture must hold all 15 cases");
    assert_eq!(
        TokenPolicy::default().cap_tokens,
        CAP_TOKENS,
        "policy default cap must match the cap this gate enforces",
    );
    for case in &dataset.cases {
        assert_eq!(
            case.allowed_caps_to_test,
            vec![CAP_TOKENS],
            "case {} must only request the default cap",
            case.id,
        );
    }
}

/// The 100% gate: run all 15 fixtures across modes 0/2/3/4/5 and assert that
/// NO single packet, in ANY mode, exceeds the cap — neither on the authoritative
/// input count alone nor on the gateway budget (input + reserves).
#[test]
fn every_packet_in_every_mode_respects_cap() {
    let policy = TokenPolicy::default();
    let output_reserve = policy.output_reserve_tokens;
    let count_drift_reserve = policy.count_drift_reserve_tokens;

    let run = run_context_dataset(fixture_path(), CAP_TOKENS)
        .expect("cap-compliance dataset run must succeed without provider calls");

    // All 15 packets must have been built.
    assert_eq!(
        run.results.len(),
        15,
        "every fixture case must produce exactly one packet",
    );

    // Per-packet, per-mode cap enforcement. This loop FAILS if any single packet
    // in any mode exceeds the cap.
    for result in &run.results {
        let estimated_input_tokens = result.metrics.tokens_in_total;
        assert_eq!(
            result.cap_tokens, CAP_TOKENS,
            "case {} ran at unexpected cap",
            result.case_id,
        );

        // (1) Authoritative input token count must fit under the cap on its own.
        assert!(
            estimated_input_tokens <= CAP_TOKENS,
            "case {} (mode {}) authoritative input tokens {} exceed cap {}",
            result.case_id,
            result.mode,
            estimated_input_tokens,
            CAP_TOKENS,
        );

        // (2) Gateway budget: input + output reserve + count-drift reserve must
        //     also fit. Reconstruct the inequality independently of the harness
        //     flag using the policy's reserves so the assertion proves the bound.
        let gateway_total = estimated_input_tokens
            .saturating_add(output_reserve)
            .saturating_add(count_drift_reserve);
        assert!(
            gateway_total <= CAP_TOKENS,
            "case {} (mode {}) gateway budget {} (= {} input + {} output_reserve + {} count_drift_reserve) exceeds cap {}",
            result.case_id,
            result.mode,
            gateway_total,
            estimated_input_tokens,
            output_reserve,
            count_drift_reserve,
            CAP_TOKENS,
        );

        // The harness's own per-packet cap_compliance flag (which encodes the
        // same reserve-inclusive inequality) must agree with our reconstruction.
        assert!(
            result.metrics.cap_compliance,
            "case {} (mode {}) harness reported cap non-compliance",
            result.case_id, result.mode,
        );
        assert_eq!(
            result.metrics.cap_compliance,
            gateway_total <= CAP_TOKENS,
            "case {} (mode {}) harness cap_compliance disagrees with reconstructed bound",
            result.case_id,
            result.mode,
        );
    }

    // Aggregate gate: the harness summary must report 100% cap compliance.
    assert!(
        run.summary.cap_compliance,
        "summary cap_compliance must be true when every packet fits the cap",
    );

    // Every mode the fixture exercises (0, 2, 3, 4, 5) must be represented, so the
    // per-packet assertions above genuinely covered all five modes.
    let observed: std::collections::BTreeSet<&str> =
        run.results.iter().map(|r| r.mode.as_str()).collect();
    for mode in EXPECTED_MODES {
        assert!(
            observed.contains(mode),
            "mode {mode} must be exercised by the fixture; observed modes: {observed:?}",
        );
    }
}
