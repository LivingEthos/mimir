//! Criterion benchmark for the TUI render path.
//!
//! Benchmarks one full frame draw of a representative [`App`] via
//! [`mimir_tui::draw_test_frame`], which drives the same private `draw_ui`
//! layout/widget path used by the live terminal loop but renders into an
//! off-screen `ratatui::backend::TestBackend` at a fixed 120x40 size. This is
//! the per-frame cost the live loop pays at up to 60 FPS.
//!
//! All inputs are local and synthetic (an in-memory `ContextPacket`,
//! `PipelineResult`, and diff text assembled in-process); there are no
//! provider, network, terminal, or filesystem calls. Target: render frame
//! < 16ms (60 FPS).

use criterion::{black_box, criterion_group, criterion_main, Criterion};
use mimir_retrieval::{PackedItem, PackedManifest, PipelineResult, Range, RecallGuardFlag};
use mimir_schemas::{ContextPacket, ContextRange, IncludedItem, OmittedCandidate, TaskCard};
use mimir_tui::{App, PermissionsState, ProviderCount};

/// Fixed render area: a wide-ish, tall-ish terminal exercising every panel.
const FRAME_WIDTH: u16 = 120;
const FRAME_HEIGHT: u16 = 40;

/// Build a `ContextPacket` populated with enough included and omitted items to
/// give the included/omitted panels realistic, multi-line content to lay out.
fn representative_packet() -> ContextPacket {
    let included = (0..24)
        .map(|i| IncludedItem {
            path: format!("crates/mimir-core/src/module_{i}.rs"),
            ranges: vec![
                ContextRange {
                    start: i * 10 + 1,
                    end: i * 10 + 40,
                },
                ContextRange {
                    start: i * 10 + 80,
                    end: i * 10 + 95,
                },
            ],
            candidate_kind: "range".to_string(),
            reason_code: "direct_user_mention".to_string(),
            tokens: 128 + i,
            source_hash: "a".repeat(64),
            trust_level: "trusted".to_string(),
            editable: true,
        })
        .collect();

    let omitted_candidates = (0..18)
        .map(|i| OmittedCandidate {
            schema_version: 1,
            path: format!("tests/module_{i}_test.rs"),
            ranges: vec![ContextRange {
                start: 1,
                end: i + 20,
            }],
            candidate_kind: "test".to_string(),
            reason_code: "budget_limit".to_string(),
            score: 0.55,
            features: serde_json::json!({ "kind": "test", "idx": i }),
            estimated_tokens: 80 + i,
            discovered_by: vec!["test_map".to_string()],
            source_hash: None,
            reason_for_omission: "budget_limit".to_string(),
            risk: Some("medium".to_string()),
            what_would_trigger_inclusion: "more budget".to_string(),
        })
        .collect();

    ContextPacket {
        schema_version: 1,
        packet_id: "pkt-bench-001".to_string(),
        packet_hash: "0".repeat(64),
        run_id: "20260101-000000-00000001".to_string(),
        task_card: TaskCard {
            goal: "Render a fully populated TUI frame".to_string(),
            acceptance_criteria: Vec::new(),
            likely_files: Vec::new(),
            risk_level: None,
            expected_test_command: None,
            unknowns: Vec::new(),
            need_for_large_context: None,
            complexity: "tiny".to_string(),
        },
        mode: "ask".to_string(),
        cap_tokens: 64000,
        target_tokens: 32000,
        output_reserve_tokens: 4096,
        count_drift_reserve_tokens: 1024,
        provider: "anthropic".to_string(),
        model: "claude-sonnet-4-20250514".to_string(),
        capability_snapshot_ref: "cap-001".to_string(),
        prompt_contract_version: 1,
        included,
        omitted_candidates,
        tool_schemas: vec![],
        evidence_cards: vec![],
        memory_entries: vec![],
        budget_ledger_ref: ".mimir/runs/20260101-000000-00000001/budget_ledger.json".to_string(),
        estimated_input_tokens: 12_000,
        count_provenance: "local_estimate_only".to_string(),
        created_at: "2025-01-01T00:00:00Z".to_string(),
        authoritative_input_tokens: None,
        recall_guard_flags: vec![],
    }
}

/// Build a `PipelineResult` so the included/omitted panels render from the
/// pipeline path (preferred over the raw packet) with guard flags present.
fn representative_pipeline() -> PipelineResult {
    let included = (0..24)
        .map(|i| PackedItem {
            path: format!("crates/mimir-core/src/module_{i}.rs"),
            ranges: vec![Range {
                start: i * 10 + 1,
                end: i * 10 + 40,
            }],
            estimated_tokens: 128 + i,
            score: 0.9,
            reason_code: "direct_match".to_string(),
        })
        .collect();

    PipelineResult {
        manifest: PackedManifest {
            included,
            omitted: vec![],
            total_tokens: 12_000,
        },
        sufficiency_ok: true,
        sufficiency_warnings: vec!["coverage below target on tests/".to_string()],
        recall_guard_flags: vec![RecallGuardFlag {
            category: "missing_test".to_string(),
            description: "No tests found for changed module".to_string(),
            paths: vec!["crates/mimir-core/src/module_0.rs".to_string()],
        }],
    }
}

/// Assemble a representative, fully populated `App` exercising every panel.
fn representative_app() -> App {
    let mut app = App::new();
    app.load_packet(representative_packet());
    app.load_pipeline_result(representative_pipeline());

    let diff = (0..60)
        .map(|i| {
            if i % 3 == 0 {
                format!("+ added line {i} in crates/mimir-core/src/module.rs")
            } else if i % 3 == 1 {
                format!("- removed line {i}")
            } else {
                format!("  context line {i}")
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    app.load_diff(diff);

    app.provider_counts = (0..4)
        .map(|i| ProviderCount {
            name: format!("provider-{i}"),
            model: format!("model-{i}"),
            input_tokens: 1000 + i,
            output_tokens: 200 + i,
            cache_creation_tokens: 50 + i,
            cache_read_tokens: 25 + i,
        })
        .collect();

    app.permissions = PermissionsState {
        can_edit: true,
        can_run: false,
        can_network: true,
        allowed_paths: vec!["crates/".to_string(), "tests/".to_string()],
        blocked_paths: vec![".mimir/secrets/".to_string()],
    };

    app.focused_panel = 2;
    app.status = "Rendering benchmark frame".to_string();
    app
}

fn bench_render_frame(c: &mut Criterion) {
    let app = representative_app();
    c.bench_function("render_frame_120x40", |b| {
        b.iter(|| {
            black_box(draw_one_frame(black_box(&app)));
        });
    });
}

/// Render exactly one full frame into an off-screen buffer.
fn draw_one_frame(app: &App) -> ratatui::buffer::Buffer {
    mimir_tui::draw_test_frame(app, FRAME_WIDTH, FRAME_HEIGHT)
}

criterion_group!(benches, bench_render_frame);
criterion_main!(benches);
