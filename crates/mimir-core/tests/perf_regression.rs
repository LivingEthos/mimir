//! Deterministic, CI-safe performance-baseline guard.
//!
//! This test does NOT run any benchmark and does NOT depend on wall-clock
//! timing, so it can never flake. It loads the committed `bench/baselines.json`
//! and asserts structural invariants:
//!
//! 1. The file parses and contains every expected metric key.
//! 2. Every recorded baseline value is `<=` its target from
//!    `17-PERFORMANCE-TARGETS.md` (mirrored in the `targets` block of the
//!    baseline file). This makes it impossible for a committed baseline to
//!    silently claim a worse-than-target number.
//! 3. The required `ci_gates` fields are present and sane.
//!
//! The timing comparison against a *fresh* `cargo bench` run lives in
//! `scripts/check-perf-regression.sh`, which is allowed to be slow. This test
//! is the fast, deterministic counterpart that gates CI on the committed data.

use serde_json::Value;
use std::path::PathBuf;

/// Every metric key that must exist in `bench/baselines.json`, paired with the
/// performance target it must not exceed. Targets are taken verbatim from
/// `docs/17-PERFORMANCE-TARGETS.md` (root: `17-PERFORMANCE-TARGETS.md`),
/// converted to the unit implied by the key suffix (`_ms` or `_s`).
const METRIC_TARGETS: &[(&str, f64)] = &[
    ("mimir_init_ms", 100.0),
    ("cold_startup_warm_cache_ms", 200.0),
    ("cold_startup_no_cache_ms", 2000.0),
    ("packet_build_small_ms", 500.0),
    ("repo_index_10k_cold_s", 30.0),
    ("repo_index_incremental_s", 2.0),
    ("token_count_local_ms", 50.0),
    ("tui_render_frame_ms", 16.0),
    ("doctor_command_s", 2.0),
];

/// Resolve the path to `bench/baselines.json` from the workspace root.
///
/// `CARGO_MANIFEST_DIR` points at `crates/mimir-core` during the test build, so
/// the workspace root is two directories up.
fn baselines_path() -> PathBuf {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest_dir
        .parent()
        .and_then(|p| p.parent())
        .expect("workspace root is two levels above crates/mimir-core")
        .join("bench")
        .join("baselines.json")
}

fn load_baselines() -> Value {
    let path = baselines_path();
    let raw = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("failed to read {}: {e}", path.display()));
    serde_json::from_str(&raw)
        .unwrap_or_else(|e| panic!("{} is not valid JSON: {e}", path.display()))
}

#[test]
fn baselines_file_parses_and_has_expected_structure() {
    let root = load_baselines();

    // Provenance fields required by the harness contract.
    assert_eq!(
        root.get("generated_from").and_then(Value::as_str),
        Some("cargo bench"),
        "baselines.json must record generated_from = \"cargo bench\""
    );
    assert!(
        root.get("generated_commit")
            .and_then(Value::as_str)
            .is_some_and(|s| !s.is_empty()),
        "baselines.json must record a non-empty generated_commit (git short hash)"
    );

    let metrics = root
        .get("metrics")
        .and_then(Value::as_object)
        .expect("baselines.json must contain a `metrics` object");

    for (key, _) in METRIC_TARGETS {
        let v = metrics
            .get(*key)
            .unwrap_or_else(|| panic!("metrics is missing required key `{key}`"));
        let n = v
            .as_f64()
            .unwrap_or_else(|| panic!("metric `{key}` must be a number, got {v}"));
        assert!(
            n.is_finite() && n >= 0.0,
            "metric `{key}` must be a finite, non-negative number, got {n}"
        );
    }
}

#[test]
fn ci_gate_fields_present_and_sane() {
    let root = load_baselines();
    let gates = root
        .get("ci_gates")
        .and_then(Value::as_object)
        .expect("baselines.json must contain a `ci_gates` object");

    for key in [
        "test_timeout_multiplier",
        "eval_timeout_multiplier",
        "bench_regression_threshold_percent",
    ] {
        let v = gates
            .get(key)
            .unwrap_or_else(|| panic!("ci_gates is missing required key `{key}`"));
        let n = v
            .as_f64()
            .unwrap_or_else(|| panic!("ci_gate `{key}` must be a number, got {v}"));
        assert!(
            n > 0.0,
            "ci_gate `{key}` must be strictly positive, got {n}"
        );
    }
}

#[test]
fn every_baseline_is_within_its_target() {
    let root = load_baselines();
    let metrics = root
        .get("metrics")
        .and_then(Value::as_object)
        .expect("baselines.json must contain a `metrics` object");

    for (key, target) in METRIC_TARGETS {
        let measured = metrics
            .get(*key)
            .and_then(Value::as_f64)
            .unwrap_or_else(|| panic!("metric `{key}` missing or not numeric"));
        assert!(
            measured <= *target,
            "PERF REGRESSION: baseline `{key}` = {measured} exceeds its target {target}. \
             A committed baseline may never claim a worse-than-target number; \
             investigate and fix the hot path or file an ADR proposing a new target."
        );
    }
}

#[test]
fn recorded_targets_match_the_spec() {
    // If the baseline file carries its own `targets` block, it must agree with
    // the spec values mirrored in METRIC_TARGETS. This catches drift between
    // the JSON's self-described targets and 17-PERFORMANCE-TARGETS.md.
    let root = load_baselines();
    let Some(targets) = root.get("targets").and_then(Value::as_object) else {
        // The targets block is optional; absence is not a failure.
        return;
    };
    for (key, target) in METRIC_TARGETS {
        if let Some(v) = targets.get(*key) {
            let recorded = v
                .as_f64()
                .unwrap_or_else(|| panic!("target `{key}` must be numeric, got {v}"));
            assert!(
                (recorded - *target).abs() < f64::EPSILON,
                "target drift: baselines.json records target `{key}` = {recorded} \
                 but 17-PERFORMANCE-TARGETS.md says {target}"
            );
        }
    }
}
