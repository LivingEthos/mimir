//! Mimir CLI entry point.

use std::collections::hash_map::DefaultHasher;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs;
use std::hash::{Hash, Hasher};
use std::io::Write;
use std::path::{Component, Path, PathBuf};

use anyhow::{anyhow, bail, Result};
use clap::{Parser, Subcommand};
use mimir_core::ContextBuilder;
use mimir_edit::{
    patch_step_paths,
    test_runner::{detect_framework, run_tests, TestFramework, TestRunResult},
    EditableSet, PatchEngine, PatchTransaction, WorktreeStatus,
};
use mimir_providers::adapters::anthropic::AnthropicAdapter;
use mimir_providers::{
    OpenAiCompatibleAdapter, ProviderDispatchAdapter, ProviderGateway, ProviderMessage,
    ProviderRequest, ProviderResponse, ResponseBlock, TokenUsage, ValidatedPacket,
    ValidatedProviderRequest,
};
use mimir_runs::{atomic_write, RunDir, RunId};
use mimir_schemas::{
    ExecutablePatchPlan, PatchEditKind, PatchEditReasoning, PatchFileEdit, PatchPlan, PatchRange,
    PatchStep,
};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tracing::info;

const MAX_PROVIDER_RESPONSE_ARTIFACT_BYTES: usize = 256 * 1024;
const MAX_PROVIDER_REQUEST_ARTIFACT_BYTES: usize = 256 * 1024;
const MAX_PATCH_PLAN_ARTIFACT_BYTES: usize = 128 * 1024;
const MAX_PATCH_RECIPE_ARTIFACT_BYTES: usize = 256 * 1024;
const MAX_TEST_RESULT_ARTIFACT_BYTES: usize = 128 * 1024;
const MAX_REPAIR_ARTIFACT_BYTES: usize = 256 * 1024;
const MAX_PATCH_DIFF_ARTIFACT_BYTES: usize = 256 * 1024;
const MAX_RECORDED_TEST_COMMANDS: usize = 32;
const MAX_RECORDED_TEST_COMMAND_CHARS: usize = 512;
const PACKET_SHARE_BUNDLE_KIND: &str = "mimir.packet_share";
const PACKET_SHARE_BUNDLE_SCHEMA_VERSION: u32 = 1;
const MAX_COMMAND_RECIPE_BYTES: u64 = 64 * 1024;
const MAX_COMMAND_RECIPE_OUTPUT_BUDGET: u32 = 32_768;
const RESERVED_COMMAND_RECIPE_NAMES: &[&str] = &[
    "fix-failing-test",
    "audit-touched-files",
    "narrow-to-symbol",
    "review-with-tests",
];
const KNOWN_COMMAND_RECIPE_TOOLS: &[&str] = &[
    "apply_patch",
    "edit_file",
    "read_file",
    "read_file_range",
    "run_command",
    "run_tests",
    "search_code",
];
const PROJECT_RULES_TEMPLATE: &str = r"# Mimir Project Rules

Keep durable repository guidance here. Mimir includes this file in context packets when it exists, subject to redaction and size limits.

<!-- MIMIR_PUBLISHED_START -->
<!-- MIMIR_PUBLISHED_END -->
";
const CHECK_NO_SECRET_MATERIAL: &str = r#"# No Provider Secrets

Source-controlled checks should catch obvious credential material before a model call.

- severity: error
- forbidden: (?i)(api[_-]?key|secret|token|password)\s*[:=]\s*['\"]?[A-Za-z0-9_\-]{16,}
- forbidden: sk-[A-Za-z0-9]{24,}
"#;
const FAST_CHECK_RECIPE: &str = r#"---
name: fast-check
description: Run the smallest local checks that cover the current edit scope.
mode: ask
retrieval:
  seeds:
    - "{{changed_path}}"
  expand:
    - imports
    - tests
allowed_tools:
  - run_command
output_budget_tokens: 2048
parameters:
  - name: changed_path
    description: Path or crate touched by the change.
    required: false
---

Use this recipe to decide which focused formatter, lint, and test commands should run before a broader validation pass.
"#;
const RELEASE_CHECK_RECIPE: &str = r"---
name: release-check
description: Run the full release handoff validation gate.
mode: ask
retrieval:
  seeds:
    - scripts/validate-production.sh
  expand:
    - config
    - tests
allowed_tools:
  - run_command
output_budget_tokens: 2048
parameters: []
---

Use this recipe before release handoff or after changes that cross multiple workstreams.
";

#[derive(Parser)]
#[command(name = "mimir")]
#[command(about = "Replayable Context for coding agents")]
#[command(version = env!("CARGO_PKG_VERSION"))]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Initialize a new Mimir project.
    Init {
        /// Project directory (default: current).
        #[arg(short, long)]
        path: Option<String>,
    },
    /// Check environment and dependencies.
    Doctor,
    /// Show version.
    Version,
    /// Provider management commands.
    Providers {
        #[command(subcommand)]
        cmd: ProvidersCmd,
    },
    /// Build or use context packets.
    Context {
        #[command(subcommand)]
        cmd: ContextCmd,
    },
    /// Plan a task (generate an implementation plan without applying).
    Plan {
        /// Task description.
        task: String,
        /// Paths the model is allowed to edit.
        #[arg(short, long)]
        editable: Vec<String>,
        /// Provider to call (`anthropic`, `glm`, `openai`, or `openai-compatible`).
        #[arg(long, default_value = "glm")]
        provider: String,
        /// Model to call. Defaults to a sensible model for the selected provider.
        #[arg(long)]
        model: Option<String>,
        /// Override provider API base URL.
        #[arg(long)]
        base_url: Option<String>,
        /// Output JSON instead of human-readable text.
        #[arg(long)]
        json: bool,
    },
    /// Execute a task (plan + apply + test + repair loop).
    Code {
        /// Task description.
        task: String,
        /// Paths the model is allowed to edit.
        #[arg(short, long)]
        editable: Vec<String>,
        /// Source-controlled command recipe from `.mimir/commands/<name>.md`.
        #[arg(long)]
        recipe: Option<String>,
        /// Recipe parameter as `key=value`; repeat for multiple parameters.
        #[arg(long = "param")]
        params: Vec<String>,
        /// Max repair turns.
        #[arg(long, default_value = "3")]
        max_repair_turns: u32,
        /// Cost cap in dollars.
        #[arg(long, default_value = "5.0")]
        cost_cap: f64,
        /// Skip tests after applying.
        #[arg(long)]
        no_test: bool,
        /// Provider to call (`anthropic`, `glm`, `openai`, or `openai-compatible`).
        #[arg(long, default_value = "glm")]
        provider: String,
        /// Model to call. Defaults to a sensible model for the selected provider.
        #[arg(long)]
        model: Option<String>,
        /// Override provider API base URL.
        #[arg(long)]
        base_url: Option<String>,
        /// Validate and persist the patch without changing the working tree.
        #[arg(long)]
        dry_run: bool,
        /// Output JSON instead of human-readable text.
        #[arg(long)]
        json: bool,
    },
    /// Review changes since a git ref.
    Review {
        /// Git ref to compare against (e.g. HEAD~1).
        #[arg(long)]
        since: Option<String>,
        /// Run committee review with multiple reviewers.
        #[arg(long)]
        committee: bool,
        /// Run source-controlled checks.
        #[arg(long)]
        checks: bool,
    },
    /// Run source-controlled checks from `.mimir/checks/*.md`.
    Check {
        /// Exit non-zero when any error or critical finding is reported.
        #[arg(long)]
        ci: bool,
        /// Output JSON instead of human-readable text.
        #[arg(long)]
        json: bool,
    },
    /// Run a read-only exploration pass and persist evidence.
    Explore {
        /// Exploration question.
        query: String,
        /// Output JSON instead of human-readable text.
        #[arg(long)]
        json: bool,
    },
    /// Run a subagent.
    Agent {
        /// Subagent name.
        name: String,
        /// Query to send.
        query: String,
        /// Show available subagents.
        #[arg(long)]
        list: bool,
    },
    /// Memory management commands (P6).
    Memory {
        #[command(subcommand)]
        cmd: MemoryCmd,
    },
    /// Start the TUI (P6).
    Tui {
        /// Path to a ContextPacket JSON file to load.
        #[arg(long)]
        packet: Option<String>,
        /// Path to a PipelineResult JSON file to load.
        #[arg(long)]
        pipeline_result: Option<String>,
        /// TCP address of a running mimir server for live refreshes.
        #[arg(long)]
        server: Option<String>,
        /// Task to send to the live server when refreshing.
        #[arg(long, requires = "server")]
        task: Option<String>,
        /// Provider to set on the live server session.
        #[arg(long, requires = "server")]
        provider: Option<String>,
        /// Model to set on the live server session.
        #[arg(long, requires = "server")]
        model: Option<String>,
        /// Existing live server session id to reuse.
        #[arg(long, requires = "server")]
        session_id: Option<String>,
        /// Automatically refresh from the live server every N milliseconds.
        #[arg(long, requires = "server")]
        refresh_ms: Option<u64>,
    },
    /// Start the JSON-RPC server (P6).
    Serve {
        /// TCP port to bind to.
        #[arg(short, long)]
        port: Option<u16>,
        /// Use stdio transport instead of TCP.
        #[arg(long, conflicts_with = "port")]
        rpc_stdio: bool,
    },
    /// Ask a question with context retrieval.
    Ask {
        /// Question to ask.
        question: String,
        /// Provider to call (`anthropic`, `glm`, `openai`, or `openai-compatible`).
        #[arg(long, default_value = "glm")]
        provider: String,
        /// Model to call. Defaults to a sensible model for the selected provider.
        #[arg(long)]
        model: Option<String>,
        /// Override provider API base URL.
        #[arg(long)]
        base_url: Option<String>,
        /// Output JSON instead of human-readable text.
        #[arg(long)]
        json: bool,
    },
    /// Packet portability commands.
    Packet {
        #[command(subcommand)]
        cmd: PacketCmd,
    },
    /// Request a cap override.
    Override {
        #[command(subcommand)]
        cmd: OverrideCmd,
    },
    /// Trace export commands.
    Trace {
        #[command(subcommand)]
        cmd: TraceCmd,
    },
    /// Eval harness commands.
    Eval {
        #[command(subcommand)]
        cmd: EvalCmd,
    },
}

#[derive(Subcommand)]
enum MemoryCmd {
    /// List memory entries.
    List {
        /// Filter by kind.
        #[arg(short, long)]
        kind: Option<String>,
        /// Filter by scope.
        #[arg(short, long)]
        scope: Option<String>,
        /// Filter by confidence.
        #[arg(short, long)]
        confidence: Option<String>,
    },
    /// Show a memory entry.
    Show {
        /// Entry ID.
        entry_id: String,
    },
    /// Explain why an entry has its score.
    Why {
        /// Entry ID.
        entry_id: String,
    },
    /// Search memory entries.
    Search {
        /// Query string.
        query: String,
    },
    /// Publish memory to project-rules.md.
    Publish,
    /// Forget (delete) a memory entry.
    Forget {
        /// Entry ID.
        entry_id: String,
    },
    /// Import sessions from other tools.
    ImportSessions {
        /// Source tool.
        #[arg(short, long)]
        from: String,
        /// Session file(s) to import.
        #[arg(value_name = "PATH")]
        paths: Vec<String>,
        /// Discover default session files for the selected tool.
        #[arg(long)]
        discover: bool,
        /// Print discovered/importable files without writing memory entries.
        #[arg(long)]
        dry_run: bool,
        /// Maximum discovered files to import.
        #[arg(long, default_value = "128")]
        max_files: usize,
        /// Include archived default locations when supported.
        #[arg(long)]
        include_archived: bool,
    },
}

#[derive(Subcommand)]
enum ProvidersCmd {
    /// Calibrate local token counter against server-side count.
    Calibrate {
        /// Model to calibrate.
        #[arg(short, long, default_value = "claude-sonnet-4-6")]
        model: String,
        /// Provider name.
        #[arg(short, long, default_value = "anthropic")]
        provider: String,
    },
    /// Show provider capabilities.
    Doctor,
}

#[derive(Subcommand)]
enum ContextCmd {
    /// Build a context packet.
    Build {
        /// Provider to snapshot into the packet.
        #[arg(long, default_value = "glm")]
        provider: String,
        /// Model to snapshot into the packet.
        #[arg(long)]
        model: Option<String>,
    },
    /// Inspect a packet.
    Inspect {
        /// Packet file path.
        path: String,
    },
    /// Show token budget.
    Budget,
    /// List omitted items.
    Omitted,
    /// Explain why a file was included or omitted.
    Why {
        /// File path to look up.
        path: String,
        /// Run ID.
        run_id: String,
    },
    /// Suggest starting context for a task without calling a provider.
    Suggest {
        /// Task or question to map.
        task: String,
        /// Provider to snapshot into the packet.
        #[arg(long, default_value = "glm")]
        provider: String,
        /// Model to snapshot into the packet.
        #[arg(long)]
        model: Option<String>,
        /// Output JSON instead of human-readable text.
        #[arg(long)]
        json: bool,
    },
    /// Call provider with packet.
    Call {
        /// Packet file path.
        path: String,
        /// Override provider API base URL.
        #[arg(long)]
        base_url: Option<String>,
        /// Stream the response.
        #[arg(long)]
        stream: bool,
        /// Enable prompt caching.
        #[arg(long)]
        cache: bool,
    },
}

#[derive(Subcommand)]
enum PacketCmd {
    /// Share a portable, sanitized packet replay bundle.
    Share {
        /// Run ID of the packet to share.
        run_id: String,
        /// Output file path (default: stdout).
        #[arg(short, long)]
        output: Option<String>,
        /// Export only the sanitized ContextPacket JSON instead of a replay bundle.
        #[arg(long)]
        packet_only: bool,
    },
    /// Replay a packet from local artifacts or a shared bundle.
    Replay {
        /// Local run ID or shared packet bundle path.
        #[arg(value_name = "RUN_ID_OR_BUNDLE")]
        target: String,
        /// Print the byte-identical redacted provider request JSON.
        #[arg(long)]
        request_json: bool,
    },
}

#[derive(Subcommand)]
enum OverrideCmd {
    /// Request a cap override.
    Request {
        /// Desired cap in tokens.
        #[arg(long)]
        cap: u32,
        /// Reason for the override.
        #[arg(long)]
        reason: String,
        /// Auto-grant after N failed attempts.
        #[arg(long, default_value = "3")]
        auto_grant_after: u32,
    },
}

#[derive(Subcommand)]
enum TraceCmd {
    /// Export a run trace.
    Export {
        /// Run ID to export.
        run_id: String,
        /// Redact sensitive data.
        #[arg(long)]
        redact: bool,
        /// Output file path.
        #[arg(short, long)]
        output: Option<String>,
    },
}

#[derive(Subcommand)]
enum EvalCmd {
    /// Run the local context recall eval dataset.
    Context {
        /// Dataset YAML path.
        #[arg(long)]
        dataset: String,
        /// Token cap to score against.
        #[arg(long, default_value = "64000")]
        cap_tokens: u32,
        /// Output file path for the EvalResult array.
        #[arg(short, long)]
        output: Option<String>,
        /// Print the EvalResult array as JSON.
        #[arg(long)]
        json: bool,
    },
}

fn default_model(provider: &str) -> &'static str {
    match provider {
        "anthropic" => "claude-sonnet-4-20250514",
        "glm" | "zai" => "glm-5.1",
        "openai" | "openai-compatible" => "gpt-4.1",
        _ => "glm-5.1",
    }
}

fn normalized_provider(provider: &str) -> String {
    match provider {
        "zai" => "glm".to_string(),
        other => other.to_string(),
    }
}

fn build_packet(
    task: &str,
    mode: &str,
    run_id: RunId,
    provider: &str,
    model: &str,
    edit_targets: Vec<String>,
) -> Result<mimir_schemas::ContextPacket> {
    ContextBuilder::new()
        .run_id(run_id)
        .task_card(task)
        .mode(mode)
        .provider(provider)
        .model(model)
        .repo_root(".")
        .edit_targets(edit_targets)
        .build()
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

#[derive(Debug, Serialize, Deserialize)]
struct SharedPacketBundle {
    schema_version: u32,
    kind: String,
    exported_at: String,
    run_id: String,
    packet_hash: String,
    provider: String,
    model: String,
    packet: mimir_schemas::ContextPacket,
    replay: SharedPacketReplay,
}

#[derive(Debug, Serialize, Deserialize)]
struct SharedPacketReplay {
    provider_request_sha256: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    user_prompt_sha256: Option<String>,
    provider_request_redacted: serde_json::Value,
}

fn redacted_json_value(value: &impl Serialize) -> Result<serde_json::Value> {
    let mut json = serde_json::to_value(value)?;
    mimir_security::redact_json_value(&mut json);
    Ok(json)
}

fn redacted_pretty_json_bytes(value: &impl Serialize) -> Result<Vec<u8>> {
    Ok(serde_json::to_vec_pretty(&redacted_json_value(value)?)?)
}

fn ensure_context_packet_safe_to_share(packet: &mimir_schemas::ContextPacket) -> Result<()> {
    let packet_json = serde_json::to_value(packet)?;
    if redact_value(packet_json.clone()) != packet_json {
        bail!("context packet contains secret-like text; refusing to create portable share bundle");
    }
    Ok(())
}

fn user_prompt_sha256(request: &serde_json::Value) -> Option<String> {
    let messages = request.get("messages")?.as_array()?;
    let user_message = messages
        .iter()
        .find(|message| message.get("role").and_then(|role| role.as_str()) == Some("user"))?;
    let content = user_message.get("content")?.as_str()?;
    Some(sha256_hex(content.as_bytes()))
}

fn shared_replay_from_request(request: &ProviderRequest) -> Result<SharedPacketReplay> {
    let provider_request_redacted = redacted_json_value(request)?;
    shared_replay_from_redacted_request(provider_request_redacted)
}

fn shared_replay_from_redacted_request(
    provider_request_redacted: serde_json::Value,
) -> Result<SharedPacketReplay> {
    let request_bytes = serde_json::to_vec_pretty(&provider_request_redacted)?;
    if request_bytes.len() > MAX_PROVIDER_REQUEST_ARTIFACT_BYTES {
        bail!(
            "provider_request artifact shared packet bundle exceeds size cap ({} bytes > {} bytes)",
            request_bytes.len(),
            MAX_PROVIDER_REQUEST_ARTIFACT_BYTES
        );
    }
    Ok(SharedPacketReplay {
        provider_request_sha256: sha256_hex(&request_bytes),
        user_prompt_sha256: user_prompt_sha256(&provider_request_redacted),
        provider_request_redacted,
    })
}

fn shared_replay_from_provider_request_artifact(
    path: &camino::Utf8PathBuf,
) -> Result<SharedPacketReplay> {
    let data = redacted_provider_request_artifact_bytes(path)?;
    let request: serde_json::Value = serde_json::from_slice(&data).map_err(|err| {
        anyhow!(
            "provider request artifact '{}' is invalid JSON: {err}",
            path
        )
    })?;
    shared_replay_from_redacted_request(request)
}

fn redacted_provider_request_artifact_bytes(path: &camino::Utf8PathBuf) -> Result<Vec<u8>> {
    let data = fs::read(path).map_err(|err| {
        anyhow!(
            "provider request artifact '{}' could not be read: {err}",
            path
        )
    })?;
    let request: serde_json::Value = serde_json::from_slice(&data).map_err(|err| {
        anyhow!(
            "provider request artifact '{}' is invalid JSON: {err}",
            path
        )
    })?;
    if redact_value(request.clone()) != request {
        bail!(
            "provider request artifact '{}' contains secret-like text; refusing to reuse it",
            path
        );
    }
    Ok(data)
}

fn build_shared_packet_bundle(
    packet: &mimir_schemas::ContextPacket,
    run_id: &str,
    replay: SharedPacketReplay,
) -> Result<SharedPacketBundle> {
    verify_packet_run_id(packet, run_id)?;
    verify_packet_replay_preconditions(packet)?;
    ensure_context_packet_safe_to_share(packet)?;
    Ok(SharedPacketBundle {
        schema_version: PACKET_SHARE_BUNDLE_SCHEMA_VERSION,
        kind: PACKET_SHARE_BUNDLE_KIND.to_string(),
        exported_at: chrono::Utc::now().to_rfc3339(),
        run_id: run_id.to_string(),
        packet_hash: packet.packet_hash.clone(),
        provider: packet.provider.clone(),
        model: packet.model.clone(),
        packet: packet.clone(),
        replay,
    })
}

fn verify_shared_packet_bundle(bundle: &SharedPacketBundle) -> Result<()> {
    if bundle.schema_version != PACKET_SHARE_BUNDLE_SCHEMA_VERSION {
        bail!(
            "unsupported shared packet bundle schema_version {}",
            bundle.schema_version
        );
    }
    if bundle.kind != PACKET_SHARE_BUNDLE_KIND {
        bail!(
            "shared packet bundle kind mismatch: expected {}, got {}",
            PACKET_SHARE_BUNDLE_KIND,
            bundle.kind
        );
    }
    verify_packet_integrity(&bundle.packet)?;
    verify_packet_run_id(&bundle.packet, &bundle.run_id)?;
    if bundle.packet_hash != bundle.packet.packet_hash {
        bail!(
            "shared packet bundle packet_hash mismatch: declared {}, packet {}",
            bundle.packet_hash,
            bundle.packet.packet_hash
        );
    }
    if bundle.provider != bundle.packet.provider || bundle.model != bundle.packet.model {
        bail!("shared packet bundle provider/model metadata mismatch");
    }
    let request_bytes = serde_json::to_vec_pretty(&bundle.replay.provider_request_redacted)?;
    let actual_request_sha256 = sha256_hex(&request_bytes);
    if bundle.replay.provider_request_sha256 != actual_request_sha256 {
        bail!(
            "shared packet bundle provider_request_sha256 mismatch: declared {}, actual {}",
            bundle.replay.provider_request_sha256,
            actual_request_sha256
        );
    }
    if let Some(expected_prompt_sha256) = &bundle.replay.user_prompt_sha256 {
        let actual_prompt_sha256 = user_prompt_sha256(&bundle.replay.provider_request_redacted)
            .ok_or_else(|| {
                anyhow!("shared packet bundle request is missing user prompt content")
            })?;
        if expected_prompt_sha256 != &actual_prompt_sha256 {
            bail!(
                "shared packet bundle user_prompt_sha256 mismatch: declared {}, actual {}",
                expected_prompt_sha256,
                actual_prompt_sha256
            );
        }
    }
    Ok(())
}

fn shared_bundle_request_bytes(bundle: &SharedPacketBundle) -> Result<Vec<u8>> {
    verify_shared_packet_bundle(bundle)?;
    Ok(serde_json::to_vec_pretty(
        &bundle.replay.provider_request_redacted,
    )?)
}

fn read_shared_packet_bundle(path: &Path) -> Result<SharedPacketBundle> {
    let data = fs::read_to_string(path).map_err(|err| {
        anyhow!(
            "shared packet bundle '{}' could not be read: {err}",
            path.display()
        )
    })?;
    let bundle: SharedPacketBundle = serde_json::from_str(&data).map_err(|err| {
        anyhow!(
            "shared packet bundle '{}' is invalid JSON: {err}",
            path.display()
        )
    })?;
    verify_shared_packet_bundle(&bundle)?;
    Ok(bundle)
}

fn write_bytes_to_output(output: Option<String>, bytes: &[u8], label: &str) -> Result<()> {
    if let Some(out) = output {
        fs::write(&out, bytes)?;
        println!("{label} written to {out}");
    } else {
        let mut stdout = std::io::stdout().lock();
        stdout.write_all(bytes)?;
        stdout.write_all(b"\n")?;
    }
    Ok(())
}

fn write_file_if_missing(path: &Path, content: &str) -> Result<bool> {
    if path.exists() {
        return Ok(false);
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, content)?;
    Ok(true)
}

fn init_project_files(dir: &str) -> Result<Vec<String>> {
    let base = Path::new(dir);
    let mimir_root = base.join(".mimir");
    fs::create_dir_all(mimir_root.join("checks"))?;
    fs::create_dir_all(mimir_root.join("commands"))?;

    let mut created = Vec::new();
    if write_file_if_missing(
        &mimir_root.join("config.yaml"),
        "version: 1\ncap_tokens: 64000\n",
    )? {
        created.push(".mimir/config.yaml".to_string());
    }
    if write_file_if_missing(&mimir_root.join("project-rules.md"), PROJECT_RULES_TEMPLATE)? {
        created.push(".mimir/project-rules.md".to_string());
    }
    if write_file_if_missing(
        &mimir_root.join("checks/no-provider-secrets.md"),
        CHECK_NO_SECRET_MATERIAL,
    )? {
        created.push(".mimir/checks/no-provider-secrets.md".to_string());
    }
    if write_file_if_missing(
        &mimir_root.join("commands/fast-check.md"),
        FAST_CHECK_RECIPE,
    )? {
        created.push(".mimir/commands/fast-check.md".to_string());
    }
    if write_file_if_missing(
        &mimir_root.join("commands/release-check.md"),
        RELEASE_CHECK_RECIPE,
    )? {
        created.push(".mimir/commands/release-check.md".to_string());
    }
    Ok(created)
}

fn check_command_output(
    checks: &[mimir_review::checks::Check],
    findings: &[mimir_review::Finding],
) -> serde_json::Value {
    let blocking = findings
        .iter()
        .filter(|finding| matches!(finding.severity.as_str(), "error" | "critical"))
        .count();
    serde_json::json!({
        "checks_loaded": checks.len(),
        "findings": findings,
        "blocking_findings": blocking,
        "passed": blocking == 0,
    })
}

fn run_check_command(ci: bool, json: bool) -> Result<()> {
    let base = camino::Utf8Path::new(".");
    let checks = mimir_review::checks::load_checks(base);
    let findings = mimir_review::checks::run_checks(&checks, base);
    let output = check_command_output(&checks, &findings);
    let passed = output
        .get("passed")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);

    if json {
        println!("{}", redacted_json_string(&output)?);
    } else {
        println!("Loaded {} source-controlled checks", checks.len());
        if findings.is_empty() {
            println!("No check findings");
        } else {
            println!("Check findings: {}", findings.len());
            for finding in &findings {
                println!(
                    "  {}: {} ({})",
                    finding.severity,
                    finding.description,
                    finding.paths.join(", ")
                );
            }
        }
    }

    if ci && !passed {
        std::process::exit(1);
    }
    Ok(())
}

fn packet_guidance_paths(packet: &mimir_schemas::ContextPacket) -> Vec<String> {
    packet
        .included
        .iter()
        .filter(|item| item.reason_code == "manifest_reference")
        .map(|item| item.path.clone())
        .collect()
}

fn run_context_suggest_command(
    task: String,
    provider: String,
    model: Option<String>,
    json: bool,
) -> Result<()> {
    let provider = normalized_provider(&provider);
    let model = model.unwrap_or_else(|| default_model(&provider).to_string());
    let run_id = RunId::generate();
    let mimir_root = camino::Utf8PathBuf::from(".mimir");
    let run_dir = RunDir::create(&mimir_root, &run_id)?;
    let packet = build_packet(&task, "ask", run_id.clone(), &provider, &model, Vec::new())?;
    let packet_json = serde_json::to_vec_pretty(&packet)?;
    atomic_write(&run_dir.context_packet_path(), &packet_json)?;

    let likely_files = packet
        .included
        .iter()
        .filter(|item| item.reason_code != "manifest_reference")
        .take(12)
        .map(|item| item.path.clone())
        .collect::<Vec<_>>();
    let risky_omissions = packet
        .omitted_candidates
        .iter()
        .filter(|item| item.risk.is_some())
        .take(12)
        .map(|item| {
            serde_json::json!({
                "path": item.path,
                "reason": item.reason_for_omission,
                "risk": item.risk,
            })
        })
        .collect::<Vec<_>>();
    let checks_loaded = mimir_review::checks::load_checks(camino::Utf8Path::new(".")).len();
    let output = serde_json::json!({
        "run_id": run_id.to_string(),
        "packet_id": packet.packet_id.clone(),
        "packet_path": run_dir.context_packet_path().to_string(),
        "estimated_input_tokens": packet.estimated_input_tokens,
        "guidance_files": packet_guidance_paths(&packet),
        "likely_files": likely_files.clone(),
        "risky_omissions": risky_omissions.clone(),
        "checks_loaded": checks_loaded,
        "next_steps": [
            "Inspect the packet before provider calls.",
            "Use --editable on plan/code to bound any changes.",
            "Run mimir check --ci before committing when source-controlled checks exist."
        ],
    });

    if json {
        println!("{}", redacted_json_string(&output)?);
    } else {
        println!("Suggested context for: {}", task);
        println!("Run ID: {}", run_id);
        println!("Packet path: {}", run_dir.context_packet_path());
        println!("Estimated input tokens: {}", packet.estimated_input_tokens);
        let guidance = packet_guidance_paths(&packet);
        if !guidance.is_empty() {
            println!("Guidance files:");
            for path in guidance {
                println!("  {}", path);
            }
        }
        if !likely_files.is_empty() {
            println!("Likely starting files:");
            for path in likely_files {
                println!("  {}", path);
            }
        }
        if !risky_omissions.is_empty() {
            println!("Risky omissions: {}", risky_omissions.len());
        }
        println!("Source-controlled checks loaded: {}", checks_loaded);
    }
    Ok(())
}

fn run_explore_command(query: String, json: bool) -> Result<()> {
    let run_id = RunId::generate();
    let mimir_root = camino::Utf8PathBuf::from(".mimir");
    let run_dir = RunDir::create(&mimir_root, &run_id)?;
    let evidence =
        mimir_subagents::subagents::execute("search", &query, Some(&run_id.to_string()))?;
    let artifact = run_dir.root().join("explore_evidence.json");
    write_redacted_json_artifact(&artifact, &evidence, ArtifactSizeClass::ProviderResponse)?;

    if json {
        let output = serde_json::json!({
            "run_id": run_id.to_string(),
            "evidence_path": artifact.to_string(),
            "evidence": evidence,
        });
        println!("{}", redacted_json_string(&output)?);
    } else {
        println!("Exploration run: {}", run_id);
        println!("Evidence path: {}", artifact);
        println!("Subagent: {}", evidence.subagent);
        println!("Confidence: {:.0}%", evidence.confidence * 100.0);
        for finding in &evidence.findings {
            println!("  {}", finding);
        }
        if !evidence.relevant_paths.is_empty() {
            println!("Relevant paths: {}", evidence.relevant_paths.join(", "));
        }
    }
    Ok(())
}

fn is_safe_command_recipe_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_')
}

fn split_recipe_frontmatter(content: &str) -> Result<(String, String)> {
    let normalized = content.replace("\r\n", "\n");
    let Some(rest) = normalized.strip_prefix("---\n") else {
        bail!("schema_recipe_invalid: recipe is missing YAML frontmatter");
    };
    let Some(end) = rest.find("\n---\n") else {
        bail!("schema_recipe_invalid: recipe frontmatter is not closed");
    };
    let yaml = rest[..end].to_string();
    let body = rest[end + "\n---\n".len()..].trim().to_string();
    Ok((yaml, body))
}

fn collect_template_variables(text: &str) -> Result<BTreeSet<String>> {
    let mut vars = BTreeSet::new();
    let mut rest = text;
    while let Some(start) = rest.find("{{") {
        let after_start = &rest[start + 2..];
        let Some(end) = after_start.find("}}") else {
            bail!("schema_recipe_invalid: unclosed template variable");
        };
        let name = after_start[..end].trim();
        if !is_safe_command_recipe_name(name) {
            bail!("schema_recipe_invalid: invalid template variable `{name}`");
        }
        vars.insert(name.to_string());
        rest = &after_start[end + 2..];
    }
    Ok(vars)
}

fn render_recipe_template(text: &str, values: &BTreeMap<String, String>) -> Result<String> {
    let mut rendered = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find("{{") {
        rendered.push_str(&rest[..start]);
        let after_start = &rest[start + 2..];
        let Some(end) = after_start.find("}}") else {
            bail!("schema_recipe_invalid: unclosed template variable");
        };
        let name = after_start[..end].trim();
        rendered.push_str(values.get(name).map(String::as_str).unwrap_or_default());
        rest = &after_start[end + 2..];
    }
    rendered.push_str(rest);
    Ok(rendered)
}

fn parse_recipe_params(params: &[String]) -> Result<BTreeMap<String, String>> {
    let mut parsed = BTreeMap::new();
    for param in params {
        let Some((key, value)) = param.split_once('=') else {
            bail!("schema_recipe_invalid: --param values must use key=value syntax");
        };
        let key = key.trim();
        if !is_safe_command_recipe_name(key) {
            bail!("schema_recipe_invalid: invalid recipe parameter name `{key}`");
        }
        if contains_secret_like_text(value) {
            bail!("secret_risk: recipe parameter `{key}` contains secret-like content");
        }
        if parsed.insert(key.to_string(), value.to_string()).is_some() {
            bail!("schema_recipe_invalid: duplicate recipe parameter `{key}`");
        }
    }
    Ok(parsed)
}

fn validate_command_recipe(recipe: &CommandRecipeFrontmatter, requested_name: &str) -> Result<()> {
    if !is_safe_command_recipe_name(&recipe.name) {
        bail!(
            "schema_recipe_invalid: invalid recipe name `{}`",
            recipe.name
        );
    }
    if recipe.name != requested_name {
        bail!(
            "schema_recipe_invalid: recipe frontmatter name `{}` does not match `{requested_name}`",
            recipe.name
        );
    }
    if recipe.description.trim().is_empty() || recipe.description.contains('\n') {
        bail!("schema_recipe_invalid: recipe description must be one non-empty line");
    }
    if !matches!(
        recipe.mode.as_str(),
        "ask"
            | "plan"
            | "code"
            | "review"
            | "explain"
            | "subagent_search"
            | "subagent_file_analyst"
            | "subagent_reviewer"
            | "subagent_test_summarizer"
            | "committee_specialist"
    ) {
        bail!(
            "schema_recipe_invalid: unsupported recipe mode `{}`",
            recipe.mode
        );
    }
    if recipe.retrieval.seeds.is_empty() {
        bail!("schema_recipe_invalid: recipe retrieval.seeds must not be empty");
    }
    if recipe.allowed_tools.is_empty() {
        bail!("schema_recipe_invalid: recipe allowed_tools must not be empty");
    }
    for tool in &recipe.allowed_tools {
        if !KNOWN_COMMAND_RECIPE_TOOLS.contains(&tool.as_str()) {
            bail!("schema_recipe_invalid: unknown recipe tool `{tool}`");
        }
    }
    if recipe.output_budget_tokens == 0
        || recipe.output_budget_tokens > MAX_COMMAND_RECIPE_OUTPUT_BUDGET
    {
        bail!(
            "schema_recipe_invalid: output_budget_tokens must be between 1 and {MAX_COMMAND_RECIPE_OUTPUT_BUDGET}"
        );
    }

    let mut names = BTreeSet::new();
    for parameter in &recipe.parameters {
        if !is_safe_command_recipe_name(&parameter.name) {
            bail!(
                "schema_recipe_invalid: invalid recipe parameter `{}`",
                parameter.name
            );
        }
        if parameter.description.trim().is_empty() {
            bail!(
                "schema_recipe_invalid: parameter `{}` needs a description",
                parameter.name
            );
        }
        if !names.insert(parameter.name.clone()) {
            bail!(
                "schema_recipe_invalid: duplicate recipe parameter `{}`",
                parameter.name
            );
        }
    }
    Ok(())
}

fn load_command_recipe(name: &str) -> Result<LoadedCommandRecipe> {
    if !is_safe_command_recipe_name(name) {
        bail!("schema_recipe_invalid: unsafe recipe name `{name}`");
    }
    if RESERVED_COMMAND_RECIPE_NAMES.contains(&name) {
        bail!("schema_recipe_invalid: `{name}` is reserved for built-in recipes");
    }

    let path = Path::new(".mimir")
        .join("commands")
        .join(format!("{name}.md"));
    let metadata = fs::symlink_metadata(&path).map_err(|_| {
        anyhow!(
            "schema_recipe_invalid: recipe `{name}` not found at {}",
            path.display()
        )
    })?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        bail!(
            "schema_recipe_invalid: recipe `{}` must be a regular file",
            path.display()
        );
    }
    if metadata.len() > MAX_COMMAND_RECIPE_BYTES {
        bail!(
            "schema_recipe_invalid: recipe `{}` exceeds {} bytes",
            path.display(),
            MAX_COMMAND_RECIPE_BYTES
        );
    }

    let content = fs::read_to_string(&path)?;
    if contains_secret_like_text(&content) {
        bail!(
            "secret_risk: recipe `{}` contains secret-like content",
            path.display()
        );
    }
    let (yaml, body) = split_recipe_frontmatter(&content)?;
    let frontmatter: CommandRecipeFrontmatter = serde_yaml::from_str(&yaml).map_err(|err| {
        anyhow!(
            "schema_recipe_invalid: recipe `{}` frontmatter is invalid: {err}",
            path.display()
        )
    })?;
    validate_command_recipe(&frontmatter, name)?;
    Ok(LoadedCommandRecipe {
        path: path.to_string_lossy().to_string(),
        frontmatter,
        body,
    })
}

fn render_command_recipe_execution(
    base_task: &str,
    recipe_name: &str,
    params: &[String],
    expected_mode: &str,
) -> Result<CommandRecipeExecution> {
    let recipe = load_command_recipe(recipe_name)?;
    if recipe.frontmatter.mode != expected_mode {
        bail!(
            "schema_recipe_invalid: recipe `{recipe_name}` has mode `{}` but this command requires `{expected_mode}`",
            recipe.frontmatter.mode
        );
    }
    if !recipe.frontmatter.required_evidence.is_empty() {
        bail!(
            "schema_recipe_invalid: recipe `{recipe_name}` requires evidence cards not available for direct execution: {}",
            recipe.frontmatter.required_evidence.join(", ")
        );
    }

    let values = parse_recipe_params(params)?;
    let declared = recipe
        .frontmatter
        .parameters
        .iter()
        .map(|parameter| parameter.name.clone())
        .collect::<BTreeSet<_>>();
    for key in values.keys() {
        if !declared.contains(key) {
            bail!("schema_recipe_invalid: unknown recipe parameter `{key}`");
        }
    }
    for parameter in &recipe.frontmatter.parameters {
        if parameter.required && !values.contains_key(&parameter.name) {
            bail!(
                "schema_recipe_invalid: missing required recipe parameter `{}`",
                parameter.name
            );
        }
    }

    let mut used = collect_template_variables(&recipe.body)?;
    for seed in &recipe.frontmatter.retrieval.seeds {
        used.extend(collect_template_variables(seed)?);
    }
    for name in used {
        if !declared.contains(&name) {
            bail!("schema_recipe_invalid: template variable `{name}` is not declared");
        }
    }

    let expanded_body = render_recipe_template(&recipe.body, &values)?;
    let expanded_seeds = recipe
        .frontmatter
        .retrieval
        .seeds
        .iter()
        .map(|seed| render_recipe_template(seed, &values))
        .collect::<Result<Vec<_>>>()?;
    let mut retrieval = recipe.frontmatter.retrieval.clone();
    retrieval.seeds = expanded_seeds;

    let seed_lines = retrieval
        .seeds
        .iter()
        .map(|seed| format!("- {seed}"))
        .collect::<Vec<_>>()
        .join("\n");
    let expansion_lines = if retrieval.expand.is_empty() {
        "- none".to_string()
    } else {
        retrieval
            .expand
            .iter()
            .map(|strategy| format!("- {strategy}"))
            .collect::<Vec<_>>()
            .join("\n")
    };
    let expanded_task = format!(
        "{base_task}\n\nMimir command recipe: {}\nDescription: {}\nMode: {}\nRetrieval seeds:\n{}\nRetrieval expansion:\n{}\nAllowed tools: {}\nOutput budget tokens: {}\n\nRecipe instructions:\n{}",
        recipe.frontmatter.name,
        recipe.frontmatter.description,
        recipe.frontmatter.mode,
        seed_lines,
        expansion_lines,
        recipe.frontmatter.allowed_tools.join(", "),
        recipe.frontmatter.output_budget_tokens,
        expanded_body
    );
    if contains_secret_like_text(&expanded_task) {
        bail!("secret_risk: expanded recipe task contains secret-like content");
    }

    Ok(CommandRecipeExecution {
        schema_version: 1,
        name: recipe.frontmatter.name,
        path: recipe.path,
        description: recipe.frontmatter.description,
        mode: recipe.frontmatter.mode,
        retrieval,
        allowed_tools: recipe.frontmatter.allowed_tools,
        output_budget_tokens: recipe.frontmatter.output_budget_tokens,
        required_evidence: recipe.frontmatter.required_evidence,
        parameters: values,
        expanded_body,
        expanded_task,
    })
}

fn resolve_code_task_recipe(
    task: &str,
    recipe: Option<&str>,
    params: &[String],
) -> Result<(String, Option<CommandRecipeExecution>)> {
    match recipe {
        Some(recipe_name) => {
            let execution = render_command_recipe_execution(task, recipe_name, params, "code")?;
            Ok((execution.expanded_task.clone(), Some(execution)))
        }
        None => {
            if !params.is_empty() {
                bail!("schema_recipe_invalid: --param requires --recipe");
            }
            Ok((task.to_string(), None))
        }
    }
}

fn looks_like_path(value: &str) -> bool {
    value.contains('/') || value.contains('\\') || value.ends_with(".json")
}

fn verify_packet_integrity(packet: &mimir_schemas::ContextPacket) -> Result<()> {
    let recomputed = mimir_context::hash_packet(packet);
    if packet.packet_hash != recomputed {
        bail!(
            "context packet hash mismatch: declared {}, recomputed {}",
            packet.packet_hash,
            recomputed
        );
    }
    Ok(())
}

fn verify_packet_run_id(
    packet: &mimir_schemas::ContextPacket,
    expected_run_id: &str,
) -> Result<()> {
    if packet.run_id != expected_run_id {
        bail!(
            "packet run_id mismatch: packet declares {}, but run directory is {}",
            packet.run_id,
            expected_run_id
        );
    }
    Ok(())
}

fn run_id_from_packet_path(path: &Path) -> Option<String> {
    if path.file_name()?.to_str()? != "context_packet.json" {
        return None;
    }
    let run_dir = path.parent()?;
    let runs_dir = run_dir.parent()?;
    if runs_dir.file_name()?.to_str()? != "runs" {
        return None;
    }
    Some(run_dir.file_name()?.to_str()?.to_string())
}

fn require_packet_path_run_id(packet: &mimir_schemas::ContextPacket, path: &Path) -> Result<()> {
    let expected_run_id = run_id_from_packet_path(path).ok_or_else(|| {
        anyhow!(
            "context call requires a packet at .mimir/runs/<run_id>/context_packet.json before writing run artifacts"
        )
    })?;
    verify_packet_run_id(packet, &expected_run_id)
}

fn current_capability_snapshot_ref(provider: &str, model: &str) -> Result<String> {
    mimir_providers::capabilities::resolve_provider_capabilities(provider, model)
        .map(|resolved| resolved.snapshot_ref)
        .map_err(anyhow::Error::msg)
}

fn verify_capability_snapshot_ref(packet: &mimir_schemas::ContextPacket) -> Result<()> {
    let current = current_capability_snapshot_ref(&packet.provider, &packet.model)?;
    if !mimir_providers::capabilities::snapshot_refs_match(
        &current,
        &packet.capability_snapshot_ref,
    ) {
        bail!(
            "capability snapshot mismatch: packet snapshot {} does not match current {} for {}/{}",
            packet.capability_snapshot_ref,
            current,
            packet.provider,
            packet.model
        );
    }
    Ok(())
}

fn verify_packet_replay_preconditions(packet: &mimir_schemas::ContextPacket) -> Result<()> {
    verify_packet_integrity(packet)?;
    verify_capability_snapshot_ref(packet)
}

fn included_item_content(item: &mimir_schemas::IncludedItem) -> Result<String> {
    let bytes = fs::read(&item.path)
        .map_err(|err| anyhow!("included context '{}' could not be read: {err}", item.path))?;
    let actual_hash = sha256_hex(&bytes);
    if actual_hash != item.source_hash {
        bail!(
            "included context '{}' source_hash mismatch: declared {}, actual {}",
            item.path,
            item.source_hash,
            actual_hash
        );
    }
    let content = String::from_utf8(bytes)
        .map_err(|err| anyhow!("included context '{}' is not valid UTF-8: {err}", item.path))?;
    if contains_secret_like_text(&content) {
        bail!(
            "included context '{}' secret_risk: file contains secret-like content",
            item.path
        );
    }
    Ok(content)
}

fn verify_included_source_hashes(packet: &mimir_schemas::ContextPacket) -> Result<()> {
    for item in &packet.included {
        included_item_content(item)?;
    }
    Ok(())
}

fn context_prompt(packet: &mimir_schemas::ContextPacket) -> Result<String> {
    verify_packet_replay_preconditions(packet)?;

    let mut prompt = String::new();
    prompt.push_str("Task:\n");
    prompt.push_str(&packet.task_card.goal);
    if !packet.task_card.acceptance_criteria.is_empty() {
        prompt.push_str("\nAcceptance criteria:\n");
        for criterion in &packet.task_card.acceptance_criteria {
            prompt.push_str(&format!("- {criterion}\n"));
        }
    }
    prompt.push_str("\n\nContext packet:\n");
    prompt.push_str(&format!(
        "packet_id={} packet_hash={} run_id={}\n",
        packet.packet_id, packet.packet_hash, packet.run_id
    ));

    if !packet.included.is_empty() {
        prompt.push_str("\nIncluded context:\n");
        for item in &packet.included {
            let content = included_item_content(item)?;
            prompt.push_str(&format!(
                "\n--- {} ({}; {}; {} tokens) ---\n{}\n",
                item.path, item.candidate_kind, item.reason_code, item.tokens, content
            ));
        }
    }

    if !packet.omitted_candidates.is_empty() {
        prompt.push_str("\nOmitted candidates:\n");
        for item in &packet.omitted_candidates {
            prompt.push_str(&format!(
                "- {} omitted because {} ({} tokens)\n",
                item.path, item.reason_for_omission, item.estimated_tokens
            ));
        }
    }

    Ok(prompt)
}

fn context_packet_summary(packet: &mimir_schemas::ContextPacket) -> Result<String> {
    verify_packet_replay_preconditions(packet)?;

    let mut prompt = String::new();
    prompt.push_str("Task:\n");
    prompt.push_str(&packet.task_card.goal);
    prompt.push_str("\n\nContext packet metadata:\n");
    prompt.push_str(&format!(
        "packet_id={} packet_hash={} run_id={}\n",
        packet.packet_id, packet.packet_hash, packet.run_id
    ));

    if !packet.included.is_empty() {
        prompt.push_str("\nIncluded context from original packet:\n");
        for item in &packet.included {
            prompt.push_str(&format!(
                "- {} ({}; {}; {} tokens; source_hash={})\n",
                item.path, item.candidate_kind, item.reason_code, item.tokens, item.source_hash
            ));
        }
    }

    if !packet.omitted_candidates.is_empty() {
        prompt.push_str("\nOmitted candidates:\n");
        for item in &packet.omitted_candidates {
            prompt.push_str(&format!(
                "- {} omitted because {} ({} tokens)\n",
                item.path, item.reason_for_omission, item.estimated_tokens
            ));
        }
    }

    Ok(prompt)
}

fn provider_extra(
    packet: &mimir_schemas::ContextPacket,
) -> Option<HashMap<String, serde_json::Value>> {
    let mut extra = HashMap::new();
    if matches!(packet.provider.as_str(), "glm" | "zai") {
        extra.insert(
            "thinking".to_string(),
            serde_json::json!({ "type": "disabled" }),
        );
    }
    if extra.is_empty() {
        None
    } else {
        Some(extra)
    }
}

fn request_from_packet_with_prompt(
    packet: &mimir_schemas::ContextPacket,
    system: &str,
    user_prompt: String,
    max_tokens: u32,
) -> ProviderRequest {
    ProviderRequest {
        model: packet.model.clone(),
        system: Some(system.to_string()),
        messages: vec![ProviderMessage {
            role: "user".to_string(),
            content: user_prompt,
        }],
        tools: None,
        max_tokens: Some(max_tokens.min(packet.output_reserve_tokens)),
        temperature: Some(0.0),
        stream: Some(false),
        stop_sequences: None,
        extra: provider_extra(packet),
    }
}

fn request_from_packet(
    packet: &mimir_schemas::ContextPacket,
    stream: bool,
) -> Result<ProviderRequest> {
    let mut request = request_from_packet_with_prompt(
        packet,
        "You are Mimir, a careful coding-agent assistant. Answer directly and use the supplied replayable context when it is relevant.",
        context_prompt(packet)?,
        packet.output_reserve_tokens,
    );
    request.stream = Some(stream);
    Ok(request)
}

fn validate_packet_for_dispatch(
    packet: &mimir_schemas::ContextPacket,
    request: ProviderRequest,
) -> Result<(ProviderGateway, ValidatedProviderRequest)> {
    let resolved = mimir_providers::capabilities::resolve_provider_capabilities(
        &packet.provider,
        &packet.model,
    )
    .map_err(anyhow::Error::msg)?;
    let gateway = ProviderGateway::new(resolved.capabilities)
        .with_capability_snapshot_ref(resolved.snapshot_ref);
    let actual_input_tokens = packet
        .estimated_input_tokens
        .max(request_input_tokens(&request));
    let validated = gateway
        .prepare_request(
            ValidatedPacket {
                provider: packet.provider.clone(),
                model: packet.model.clone(),
                capability_snapshot_ref: packet.capability_snapshot_ref.clone(),
                estimated_input_tokens: actual_input_tokens,
                output_reserve_tokens: packet.output_reserve_tokens,
                count_drift_reserve_tokens: packet.count_drift_reserve_tokens,
            },
            request,
        )
        .map_err(|message| anyhow!(message))?;
    Ok((gateway, validated))
}

fn request_input_tokens(request: &ProviderRequest) -> u32 {
    let messages = request
        .messages
        .iter()
        .map(|message| (message.role.clone(), message.content.clone()))
        .collect::<Vec<_>>();
    let mut total =
        mimir_providers::count::count_request_local(request.system.as_deref(), &messages);
    if let Some(tools) = &request.tools {
        if let Ok(tools_json) = serde_json::to_string(tools) {
            total = total.saturating_add(mimir_providers::count::count_local(&tools_json));
        }
    }
    total
}

fn call_provider_with_request(
    packet: &mimir_schemas::ContextPacket,
    request: ProviderRequest,
    stream: bool,
    cache: bool,
    base_url: Option<String>,
) -> Result<ProviderResponse> {
    if stream {
        bail!("streaming provider output is not wired yet; rerun without --stream");
    }
    if cache {
        bail!("prompt-cache controls are not wired yet; rerun without --cache");
    }
    verify_packet_replay_preconditions(packet)?;
    let (gateway, validated_request) = validate_packet_for_dispatch(packet, request)?;

    let runtime = tokio::runtime::Runtime::new()?;
    match packet.provider.as_str() {
        "anthropic" => {
            let mut adapter = AnthropicAdapter::new().map_err(|err| anyhow!(err.to_string()))?;
            if let Some(url) = base_url {
                adapter = adapter.with_base_url(url);
            }
            runtime
                .block_on(
                    gateway.dispatch(ProviderDispatchAdapter::from(&adapter), validated_request),
                )
                .map_err(|err| anyhow!(err.to_string()))
        }
        "glm" | "zai" => {
            let mut adapter =
                OpenAiCompatibleAdapter::glm_from_env_with_model(packet.model.clone())
                    .map_err(|err| anyhow!(err.to_string()))?;
            if let Some(url) = base_url {
                adapter = adapter.with_base_url(url);
            }
            runtime
                .block_on(
                    gateway.dispatch(ProviderDispatchAdapter::from(&adapter), validated_request),
                )
                .map_err(|err| anyhow!(err.to_string()))
        }
        "openai" | "openai-compatible" => {
            let mut adapter = OpenAiCompatibleAdapter::generic_from_env(
                packet.provider.clone(),
                packet.model.clone(),
            )
            .map_err(|err| anyhow!(err.to_string()))?;
            if let Some(url) = base_url {
                adapter = adapter.with_base_url(url);
            }
            runtime
                .block_on(
                    gateway.dispatch(ProviderDispatchAdapter::from(&adapter), validated_request),
                )
                .map_err(|err| anyhow!(err.to_string()))
        }
        other => {
            let resolved =
                mimir_providers::capabilities::resolve_provider_capabilities(other, &packet.model)
                    .map_err(anyhow::Error::msg)?;
            if !resolved.registry_backed {
                bail!("unsupported provider '{}'", other);
            }
            let mut adapter = OpenAiCompatibleAdapter::generic_from_env(
                packet.provider.clone(),
                packet.model.clone(),
            )
            .map_err(|err| anyhow!(err.to_string()))?;
            if let Some(url) = base_url {
                adapter = adapter.with_base_url(url);
            }
            runtime
                .block_on(
                    gateway.dispatch(ProviderDispatchAdapter::from(&adapter), validated_request),
                )
                .map_err(|err| anyhow!(err.to_string()))
        }
    }
}

fn response_text(response: &ProviderResponse) -> String {
    response
        .content
        .iter()
        .filter_map(|block| match block {
            ResponseBlock::Text { text } => Some(text.as_str()),
            ResponseBlock::ToolUse { .. } => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn write_provider_artifacts(run_dir: &RunDir, response: &ProviderResponse) -> Result<()> {
    write_redacted_json_artifact(
        &run_dir.root().join("response.json"),
        response,
        ArtifactSizeClass::ProviderResponse,
    )?;
    append_redacted_event(
        run_dir,
        &serde_json::json!({
            "event_type": "provider_response",
            "timestamp": chrono::Utc::now().to_rfc3339(),
            "model": response.model,
            "usage": response.usage,
            "stop_reason": response.stop_reason,
        }),
    )?;
    Ok(())
}

fn write_provider_request_artifact(run_dir: &RunDir, request: &ProviderRequest) -> Result<()> {
    write_redacted_json_artifact(
        &run_dir.root().join("provider_request.redacted.json"),
        request,
        ArtifactSizeClass::ProviderRequest,
    )
}

#[derive(Debug, Clone, Copy)]
enum ArtifactSizeClass {
    ProviderRequest,
    ProviderResponse,
    PatchPlan,
    PatchRecipe,
    TestResult,
    Repair,
    PatchDiff,
}

impl ArtifactSizeClass {
    fn label(self) -> &'static str {
        match self {
            ArtifactSizeClass::ProviderRequest => "provider_request",
            ArtifactSizeClass::ProviderResponse => "provider_response",
            ArtifactSizeClass::PatchPlan => "patch_plan",
            ArtifactSizeClass::PatchRecipe => "patch_recipe",
            ArtifactSizeClass::TestResult => "test_result",
            ArtifactSizeClass::Repair => "repair",
            ArtifactSizeClass::PatchDiff => "patch_diff",
        }
    }

    fn max_bytes(self) -> usize {
        match self {
            ArtifactSizeClass::ProviderRequest => MAX_PROVIDER_REQUEST_ARTIFACT_BYTES,
            ArtifactSizeClass::ProviderResponse => MAX_PROVIDER_RESPONSE_ARTIFACT_BYTES,
            ArtifactSizeClass::PatchPlan => MAX_PATCH_PLAN_ARTIFACT_BYTES,
            ArtifactSizeClass::PatchRecipe => MAX_PATCH_RECIPE_ARTIFACT_BYTES,
            ArtifactSizeClass::TestResult => MAX_TEST_RESULT_ARTIFACT_BYTES,
            ArtifactSizeClass::Repair => MAX_REPAIR_ARTIFACT_BYTES,
            ArtifactSizeClass::PatchDiff => MAX_PATCH_DIFF_ARTIFACT_BYTES,
        }
    }
}

fn write_redacted_json_artifact(
    path: &camino::Utf8PathBuf,
    value: &impl Serialize,
    size_class: ArtifactSizeClass,
) -> Result<()> {
    let mut json = serde_json::to_value(value)?;
    mimir_security::redact_json_value(&mut json);
    let bytes = serde_json::to_vec_pretty(&json)?;
    write_artifact_bytes(path, &bytes, size_class)
}

fn write_redacted_bytes_artifact(
    path: &camino::Utf8PathBuf,
    bytes: &[u8],
    size_class: ArtifactSizeClass,
) -> Result<()> {
    let redacted = mimir_security::redact_secrets(&String::from_utf8_lossy(bytes));
    write_artifact_bytes(path, redacted.as_bytes(), size_class)
}

fn write_artifact_bytes(
    path: &camino::Utf8PathBuf,
    bytes: &[u8],
    size_class: ArtifactSizeClass,
) -> Result<()> {
    let max_bytes = size_class.max_bytes();
    if bytes.len() > max_bytes {
        bail!(
            "{} artifact {} exceeds size cap ({} bytes > {} bytes)",
            size_class.label(),
            path,
            bytes.len(),
            max_bytes
        );
    }
    atomic_write(path, bytes)?;
    Ok(())
}

fn write_redacted_json(path: &camino::Utf8PathBuf, value: &impl Serialize) -> Result<()> {
    let mut json = serde_json::to_value(value)?;
    mimir_security::redact_json_value(&mut json);
    let bytes = serde_json::to_vec_pretty(&json)?;
    atomic_write(path, &bytes)?;
    Ok(())
}

fn redacted_json_string(value: &impl Serialize) -> Result<String> {
    let mut json = serde_json::to_value(value)?;
    mimir_security::redact_json_value(&mut json);
    Ok(serde_json::to_string_pretty(&json)?)
}

fn append_redacted_event(run_dir: &RunDir, event: &serde_json::Value) -> Result<()> {
    let mut event = event.clone();
    mimir_security::redact_json_value(&mut event);
    run_dir.append_event(&event)?;
    Ok(())
}

fn contains_secret_like_text(text: &str) -> bool {
    mimir_security::redact_secrets(text) != text
}

fn redacted_message(message: impl ToString) -> String {
    mimir_security::redact_secrets(&message.to_string())
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
struct PlanModelOutput {
    #[serde(default)]
    steps: Vec<String>,
    #[serde(default)]
    risks: Vec<String>,
    #[serde(default)]
    files_likely_affected: Vec<String>,
    #[serde(default)]
    tests_to_run: Vec<String>,
    #[serde(default)]
    assumptions: Vec<String>,
}

#[derive(Debug, Serialize)]
struct PlanArtifact {
    schema_version: u32,
    run_id: String,
    packet_id: String,
    packet_hash: String,
    provider: String,
    model: String,
    task: String,
    editable_target_set: Vec<String>,
    steps: Vec<String>,
    risks: Vec<String>,
    files_likely_affected: Vec<String>,
    tests_to_run: Vec<String>,
    assumptions: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    raw_response: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct CodeModelOutput {
    #[serde(default)]
    patch_plan: Option<PatchPlan>,
    #[serde(default)]
    patch_recipe: Option<ProviderPatchRecipe>,
    #[serde(default)]
    tests_to_run: Vec<String>,
    #[serde(default)]
    notes: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct LegacyCodeModelOutput {
    #[serde(default)]
    patch_plan: Option<ProviderPatchRecipe>,
    #[serde(default)]
    patch_recipe: Option<ProviderPatchRecipe>,
    #[serde(default)]
    tests_to_run: Vec<String>,
    #[serde(default)]
    notes: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProviderPatchRecipe {
    schema_version: u32,
    plan_id: String,
    #[serde(default)]
    packet_id: Option<String>,
    #[serde(default)]
    steps: Vec<PatchStep>,
}

#[derive(Debug, Serialize)]
struct PatchReport {
    schema_version: u32,
    run_id: String,
    packet_id: String,
    plan_id: String,
    dry_run: bool,
    applied: bool,
    affected_files: Vec<String>,
    tests_to_run: Vec<String>,
    max_repair_turns: u32,
    cost_cap: f64,
    test_policy: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    test_result_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    test_passed: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    test_exit_code: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    repair: Option<RepairLoopSummary>,
    #[serde(skip_serializing_if = "Option::is_none")]
    notes: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    rejected: Option<String>,
}

#[derive(Debug, Serialize)]
struct RepairLoopSummary {
    attempted: bool,
    converged: bool,
    turns_executed: u32,
    total_cost: f64,
    stop_reason: String,
    turns: Vec<RepairTurnReport>,
}

#[derive(Debug, Serialize)]
struct RepairTurnReport {
    turn: u32,
    cost: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    patch_plan_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    patch_recipe_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    patch_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    test_result_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    test_passed: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    test_exit_code: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    rejected: Option<String>,
}

struct CodeTestOutcome {
    policy: String,
    path: Option<String>,
    result: Option<TestRunResult>,
}

struct RepairRunContext<'a> {
    run_dir: &'a RunDir,
    base: &'a camino::Utf8Path,
    options: &'a CodeCommandOptions,
    packet: &'a mimir_schemas::ContextPacket,
    editable_set: &'a EditableSet,
    suggested_commands: &'a [String],
    initial_spend: f64,
}

struct RepairLoopOutcome {
    summary: RepairLoopSummary,
    test_policy: String,
    test_result_path: Option<String>,
    test_result: Option<TestRunResult>,
    test_failure: Option<String>,
}

#[derive(Debug, Serialize)]
struct TestRunArtifact {
    schema_version: u32,
    policy: String,
    provider_suggested_commands: Vec<String>,
    skipped_provider_commands: Vec<SkippedProviderCommand>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    repair_suggested_commands: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    skipped_repair_suggested_commands: Vec<SkippedProviderCommand>,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<TestRunResult>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

#[derive(Debug, Serialize)]
struct SkippedProviderCommand {
    command: String,
    reason: String,
}

struct CodeCommandOptions {
    task: String,
    editable: Vec<String>,
    recipe: Option<String>,
    params: Vec<String>,
    max_repair_turns: u32,
    cost_cap: f64,
    no_test: bool,
    provider: String,
    model: Option<String>,
    base_url: Option<String>,
    dry_run: bool,
    json: bool,
}

struct PatchRejection {
    affected_files: Vec<String>,
    tests_to_run: Vec<String>,
    notes: Option<String>,
    reason: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct CommandRecipeFrontmatter {
    name: String,
    description: String,
    mode: String,
    retrieval: CommandRecipeRetrieval,
    allowed_tools: Vec<String>,
    output_budget_tokens: u32,
    #[serde(default)]
    required_evidence: Vec<String>,
    parameters: Vec<CommandRecipeParameter>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct CommandRecipeRetrieval {
    seeds: Vec<String>,
    #[serde(default)]
    expand: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct CommandRecipeParameter {
    name: String,
    description: String,
    #[serde(default)]
    required: bool,
}

#[derive(Debug, Clone)]
struct LoadedCommandRecipe {
    path: String,
    frontmatter: CommandRecipeFrontmatter,
    body: String,
}

#[derive(Debug, Clone, Serialize)]
struct CommandRecipeExecution {
    schema_version: u32,
    name: String,
    path: String,
    description: String,
    mode: String,
    retrieval: CommandRecipeRetrieval,
    allowed_tools: Vec<String>,
    output_budget_tokens: u32,
    required_evidence: Vec<String>,
    parameters: BTreeMap<String, String>,
    expanded_body: String,
    expanded_task: String,
}

fn plan_request_from_packet(packet: &mimir_schemas::ContextPacket) -> Result<ProviderRequest> {
    let prompt = format!(
        "Generate a production implementation plan for the task using the replayable context below.\n\nReturn only JSON with this shape:\n{{\"steps\":[\"concise ordered step\"],\"risks\":[\"risk\"],\"files_likely_affected\":[\"relative/path\"],\"tests_to_run\":[\"command\"],\"assumptions\":[\"assumption\"]}}\n\nKeep steps concise and actionable. Do not include markdown.\n\n{}",
        context_prompt(packet)?
    );
    Ok(request_from_packet_with_prompt(
        packet,
        "You are Mimir Planner. Produce concise, structured, replayable implementation plans and call out uncertainty explicitly.",
        prompt,
        4096,
    ))
}

fn code_request_from_packet(
    packet: &mimir_schemas::ContextPacket,
    editable: &[String],
    max_repair_turns: u32,
    cost_cap: f64,
    output_budget_tokens: Option<u32>,
) -> Result<ProviderRequest> {
    let editable_json = serde_json::to_string(editable).unwrap_or_else(|_| "[]".to_string());
    let prompt = format!(
        "Propose a safe patch for the task using the replayable context below.\n\nEditable target set: {editable_json}\nCurrent packet_id: {}\nMax repair turns budget: {max_repair_turns}\nCost cap USD: {cost_cap}\n\nReturn only JSON with this exact wrapper shape:\n{{\"patch_plan\":{{\"schema_version\":1,\"plan_id\":\"plan-id\",\"packet_id\":\"{}\",\"files_to_edit\":[{{\"path\":\"relative/path\",\"edit_kind\":\"unified_diff\"}}],\"editable_target_set\":{editable_json},\"reasoning_per_edit\":[{{\"path\":\"relative/path\",\"rationale\":\"why this edit is needed\"}}],\"tests_to_run\":[\"safe verification command suggestion\"],\"risks\":[]}},\"patch_recipe\":{{\"schema_version\":1,\"plan_id\":\"plan-id\",\"packet_id\":\"{}\",\"steps\":[{{\"action\":\"unified_diff\",\"path\":\"relative/path\",\"diff\":\"--- a/relative/path\\n+++ b/relative/path\\n@@ ...\"}}]}},\"notes\":\"optional note\"}}\n\nThe patch_plan must match Mimir PatchPlan schema and bind to the current packet_id. The patch_recipe is the executable patch payload. All paths must be relative to the repo root and inside the editable target set. Allowed patch_recipe actions are line_range, unified_diff, whole_file, create, delete, and move as defined by Mimir PatchStep. Prefer unified_diff for existing files. If no safe patch is possible, return patch_plan metadata without patch_recipe so Mimir fails closed. Do not include markdown.\n\n{}",
        packet.packet_id,
        packet.packet_id,
        packet.packet_id,
        context_prompt(packet)?
    );
    Ok(request_from_packet_with_prompt(
        packet,
        "You are Mimir Code. Produce only safe, minimal PatchPlan JSON. Never propose edits outside the editable target set.",
        prompt,
        output_budget_tokens.unwrap_or(packet.output_reserve_tokens),
    ))
}

fn parse_json_from_response<T>(text: &str) -> Result<T>
where
    T: DeserializeOwned,
{
    let trimmed = text.trim();
    if let Ok(parsed) = serde_json::from_str(trimmed) {
        return Ok(parsed);
    }

    if let Some(block) = extract_fenced_block(trimmed, &["json"]) {
        if let Ok(parsed) = serde_json::from_str(block.trim()) {
            return Ok(parsed);
        }
    }

    if let (Some(start), Some(end)) = (trimmed.find('{'), trimmed.rfind('}')) {
        if start < end {
            let candidate = &trimmed[start..=end];
            if let Ok(parsed) = serde_json::from_str(candidate) {
                return Ok(parsed);
            }
        }
    }

    bail!("provider response did not contain valid JSON")
}

fn extract_fenced_block(text: &str, preferred_labels: &[&str]) -> Option<String> {
    let mut blocks = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find("```") {
        rest = &rest[start + 3..];
        let Some(newline) = rest.find('\n') else {
            break;
        };
        let label = rest[..newline].trim().to_ascii_lowercase();
        let after_label = &rest[newline + 1..];
        let Some(end) = after_label.find("```") else {
            break;
        };
        blocks.push((label, after_label[..end].trim().to_string()));
        rest = &after_label[end + 3..];
    }

    for preferred in preferred_labels {
        if let Some((_, body)) = blocks.iter().find(|(label, _)| label.contains(preferred)) {
            return Some(body.clone());
        }
    }

    blocks.into_iter().next().map(|(_, body)| body)
}

fn plan_artifact_from_response(
    packet: &mimir_schemas::ContextPacket,
    editable: &[String],
    response_text: &str,
) -> PlanArtifact {
    let parsed = parse_json_from_response::<PlanModelOutput>(response_text).ok();
    let parsed_ok = parsed.is_some();
    let fallback_steps = fallback_plan_steps(response_text);
    let output = parsed.unwrap_or_else(|| PlanModelOutput {
        steps: fallback_steps,
        risks: Vec::new(),
        files_likely_affected: editable.to_vec(),
        tests_to_run: Vec::new(),
        assumptions: vec![
            "Provider returned prose instead of structured JSON; preserved in plan.md".to_string(),
        ],
    });

    PlanArtifact {
        schema_version: 1,
        run_id: packet.run_id.clone(),
        packet_id: packet.packet_id.clone(),
        packet_hash: packet.packet_hash.clone(),
        provider: packet.provider.clone(),
        model: packet.model.clone(),
        task: packet.task_card.goal.clone(),
        editable_target_set: editable.to_vec(),
        steps: if output.steps.is_empty() {
            fallback_plan_steps(response_text)
        } else {
            output.steps
        },
        risks: output.risks,
        files_likely_affected: if output.files_likely_affected.is_empty() {
            editable.to_vec()
        } else {
            output.files_likely_affected
        },
        tests_to_run: output.tests_to_run,
        assumptions: output.assumptions,
        raw_response: (!parsed_ok).then(|| mimir_security::redact_secrets(response_text)),
    }
}

fn redact_plan_artifact(plan: &mut PlanArtifact) {
    plan.task = mimir_security::redact_secrets(&plan.task);
    for value in &mut plan.editable_target_set {
        *value = mimir_security::redact_secrets(value);
    }
    for value in &mut plan.steps {
        *value = mimir_security::redact_secrets(value);
    }
    for value in &mut plan.risks {
        *value = mimir_security::redact_secrets(value);
    }
    for value in &mut plan.files_likely_affected {
        *value = mimir_security::redact_secrets(value);
    }
    for value in &mut plan.tests_to_run {
        *value = mimir_security::redact_secrets(value);
    }
    for value in &mut plan.assumptions {
        *value = mimir_security::redact_secrets(value);
    }
    if let Some(raw_response) = &mut plan.raw_response {
        *raw_response = mimir_security::redact_secrets(raw_response);
    }
}

fn fallback_plan_steps(text: &str) -> Vec<String> {
    let steps: Vec<String> = text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(strip_bullet_prefix)
        .take(12)
        .collect();
    if steps.is_empty() {
        vec!["Review provider response in plan.md before implementation".to_string()]
    } else {
        steps
    }
}

fn strip_bullet_prefix(line: &str) -> String {
    line.trim_start_matches(['-', '*', ' '])
        .trim_start_matches(|ch: char| ch.is_ascii_digit() || ch == '.')
        .trim()
        .to_string()
}

fn render_plan_markdown(plan: &PlanArtifact) -> String {
    let mut markdown = format!(
        "# Mimir Plan\n\nRun ID: {}\nPacket: {}\nProvider: {}/{}\n\n## Task\n{}\n\n## Steps\n",
        plan.run_id, plan.packet_id, plan.provider, plan.model, plan.task
    );
    for (index, step) in plan.steps.iter().enumerate() {
        markdown.push_str(&format!("{}. {}\n", index + 1, step));
    }
    push_markdown_list(&mut markdown, "Risks", &plan.risks);
    push_markdown_list(
        &mut markdown,
        "Files Likely Affected",
        &plan.files_likely_affected,
    );
    push_markdown_list(&mut markdown, "Tests To Run", &plan.tests_to_run);
    push_markdown_list(&mut markdown, "Assumptions", &plan.assumptions);
    markdown
}

fn push_markdown_list(markdown: &mut String, heading: &str, items: &[String]) {
    markdown.push_str(&format!("\n## {heading}\n"));
    if items.is_empty() {
        markdown.push_str("- None recorded\n");
    } else {
        for item in items {
            markdown.push_str(&format!("- {item}\n"));
        }
    }
}

fn parse_patch_plan_response(
    text: &str,
    packet: &mimir_schemas::ContextPacket,
    editable: &[String],
) -> Result<(PatchPlan, ExecutablePatchPlan, Option<String>)> {
    if let Ok(wrapper) = parse_json_from_response::<CodeModelOutput>(text) {
        if wrapper.patch_plan.is_some() || wrapper.patch_recipe.is_some() {
            let recipe_input = wrapper.patch_recipe.ok_or_else(|| {
                anyhow!("provider returned PatchPlan metadata without executable patch_recipe")
            })?;
            let recipe = strict_patch_recipe_from_provider_input(recipe_input, packet)?;
            let mut plan = wrapper
                .patch_plan
                .unwrap_or_else(|| patch_plan_from_recipe(packet, editable, &recipe, &[]));
            merge_tests_to_plan(&mut plan, wrapper.tests_to_run);
            return Ok((plan, recipe, wrapper.notes));
        }
    }

    if let Ok(wrapper) = parse_json_from_response::<LegacyCodeModelOutput>(text) {
        if let Some(recipe_input) = wrapper.patch_recipe.or(wrapper.patch_plan) {
            let recipe = strict_patch_recipe_from_provider_input(recipe_input, packet)?;
            let plan = patch_plan_from_recipe(packet, editable, &recipe, &wrapper.tests_to_run);
            return Ok((plan, recipe, wrapper.notes));
        }
    }

    if let Ok(recipe_input) = parse_json_from_response::<ProviderPatchRecipe>(text) {
        let recipe = strict_patch_recipe_from_provider_input(recipe_input, packet)?;
        let plan = patch_plan_from_recipe(packet, editable, &recipe, &[]);
        return Ok((plan, recipe, None));
    }

    if parse_json_from_response::<PatchPlan>(text).is_ok() {
        bail!("provider returned PatchPlan metadata without executable patch_recipe");
    }

    if parse_json_from_response::<serde_json::Value>(text).is_ok() {
        bail!("provider response JSON was not a schema-valid executable patch recipe");
    }

    if let Some(diff) = extract_diff_response(text) {
        let recipe = patch_plan_from_unified_diff(&diff, &packet.run_id, &packet.packet_id)?;
        ensure_patch_recipe_has_steps(&recipe)?;
        let plan = patch_plan_from_recipe(packet, editable, &recipe, &[]);
        return Ok((plan, recipe, None));
    }

    bail!("provider response did not contain a non-empty executable patch_recipe JSON object or unified diff")
}

fn strict_patch_recipe_from_provider_input(
    recipe: ProviderPatchRecipe,
    packet: &mimir_schemas::ContextPacket,
) -> Result<ExecutablePatchPlan> {
    let recipe = ExecutablePatchPlan {
        schema_version: recipe.schema_version,
        plan_id: recipe.plan_id,
        packet_id: recipe.packet_id.unwrap_or_else(|| packet.packet_id.clone()),
        steps: recipe.steps,
    };
    ensure_patch_recipe_has_steps(&recipe)?;
    Ok(recipe)
}

fn ensure_patch_recipe_has_steps(recipe: &ExecutablePatchPlan) -> Result<()> {
    if recipe.steps.is_empty() {
        bail!("provider returned an empty patch recipe; no files were changed")
    }
    Ok(())
}

fn extract_diff_response(text: &str) -> Option<String> {
    if let Some(block) = extract_fenced_block(text, &["diff", "patch"]) {
        if block.contains("@@") && block.contains("--- ") && block.contains("+++ ") {
            return Some(block);
        }
    }
    let trimmed = text.trim();
    (trimmed.contains("@@") && trimmed.contains("--- ") && trimmed.contains("+++ "))
        .then(|| trimmed.to_string())
}

fn patch_plan_from_unified_diff(
    diff: &str,
    run_id: &str,
    packet_id: &str,
) -> Result<ExecutablePatchPlan> {
    let lines: Vec<&str> = diff.lines().collect();
    let mut steps = Vec::new();
    let mut index = 0;

    while index + 1 < lines.len() {
        if !lines[index].starts_with("--- ") || !lines[index + 1].starts_with("+++ ") {
            index += 1;
            continue;
        }

        let old_path = clean_diff_path(lines[index].trim_start_matches("--- ").trim());
        let new_path = clean_diff_path(lines[index + 1].trim_start_matches("+++ ").trim());
        let start = index;
        index += 2;
        while index < lines.len() && !lines[index].starts_with("--- ") {
            index += 1;
        }
        let mut chunk = lines[start..index].join("\n");
        chunk.push('\n');

        match (old_path, new_path) {
            (None, Some(path)) => steps.push(PatchStep::Create {
                path,
                content: added_lines_from_diff(&chunk),
            }),
            (Some(path), None) => steps.push(PatchStep::Delete { path }),
            (_, Some(path)) => steps.push(PatchStep::UnifiedDiff { path, diff: chunk }),
            (None, None) => bail!("diff file header did not contain a target path"),
        }
    }

    Ok(ExecutablePatchPlan {
        schema_version: 1,
        plan_id: format!("plan-{run_id}"),
        packet_id: packet_id.to_string(),
        steps,
    })
}

fn patch_plan_from_recipe(
    packet: &mimir_schemas::ContextPacket,
    editable: &[String],
    recipe: &ExecutablePatchPlan,
    extra_tests: &[String],
) -> PatchPlan {
    let mut seen_files = BTreeSet::new();
    let mut files_to_edit = Vec::new();
    for step in &recipe.steps {
        for edit in patch_step_file_edits(step) {
            if seen_files.insert(edit.path.clone()) {
                files_to_edit.push(edit);
            }
        }
    }

    let reasoning_per_edit = files_to_edit
        .iter()
        .map(|edit| PatchEditReasoning {
            path: edit.path.clone(),
            rationale: "Generated from executable patch recipe.".to_string(),
            disproving_evidence: None,
        })
        .collect();

    PatchPlan {
        schema_version: 1,
        plan_id: recipe.plan_id.clone(),
        packet_id: packet.packet_id.clone(),
        files_to_edit,
        editable_target_set: editable.to_vec(),
        reasoning_per_edit,
        tests_to_run: dedupe_strings(extra_tests.iter().cloned()),
        risks: Vec::new(),
        what_needs_more_context: Vec::new(),
    }
}

fn patch_step_file_edits(step: &PatchStep) -> Vec<PatchFileEdit> {
    match step {
        PatchStep::LineRange {
            path,
            start_line,
            end_line,
            ..
        } => vec![PatchFileEdit {
            path: path.clone(),
            edit_kind: PatchEditKind::LineRangeReplace,
            ranges: vec![PatchRange {
                start: Some(*start_line as u32),
                end: Some(*end_line as u32),
            }],
            expected_new_content_hash: None,
        }],
        PatchStep::UnifiedDiff { path, .. } => vec![PatchFileEdit {
            path: path.clone(),
            edit_kind: PatchEditKind::UnifiedDiff,
            ranges: Vec::new(),
            expected_new_content_hash: None,
        }],
        PatchStep::WholeFile { path, .. }
        | PatchStep::Create { path, .. }
        | PatchStep::Delete { path } => {
            vec![PatchFileEdit {
                path: path.clone(),
                edit_kind: PatchEditKind::WholeFileRewrite,
                ranges: Vec::new(),
                expected_new_content_hash: None,
            }]
        }
        PatchStep::Move { from, to } => vec![
            PatchFileEdit {
                path: from.clone(),
                edit_kind: PatchEditKind::WholeFileRewrite,
                ranges: Vec::new(),
                expected_new_content_hash: None,
            },
            PatchFileEdit {
                path: to.clone(),
                edit_kind: PatchEditKind::WholeFileRewrite,
                ranges: Vec::new(),
                expected_new_content_hash: None,
            },
        ],
    }
}

fn merge_tests_to_plan(plan: &mut PatchPlan, tests: Vec<String>) {
    plan.tests_to_run = dedupe_strings(plan.tests_to_run.iter().cloned().chain(tests));
}

fn dedupe_strings(values: impl IntoIterator<Item = String>) -> Vec<String> {
    let mut seen = BTreeSet::new();
    let mut output = Vec::new();
    for value in values {
        if seen.insert(value.clone()) {
            output.push(value);
        }
    }
    output
}

fn validate_patch_plan_binding(
    plan: &PatchPlan,
    recipe: &ExecutablePatchPlan,
    packet: &mimir_schemas::ContextPacket,
    editable: &[String],
) -> Result<()> {
    if plan.schema_version != 1 || recipe.schema_version != 1 {
        bail!("patch plan schema_version must be 1");
    }
    if plan.files_to_edit.is_empty() {
        bail!("patch plan files_to_edit must list every executable patch target");
    }
    if plan.packet_id != packet.packet_id {
        bail!(
            "patch plan packet_id mismatch: expected {}, got {}",
            packet.packet_id,
            plan.packet_id
        );
    }
    if recipe.packet_id != packet.packet_id {
        bail!(
            "patch recipe packet_id mismatch: expected {}, got {}",
            packet.packet_id,
            recipe.packet_id
        );
    }
    if plan.plan_id != recipe.plan_id {
        bail!(
            "patch plan_id mismatch: metadata plan_id {} does not match recipe plan_id {}",
            plan.plan_id,
            recipe.plan_id
        );
    }

    let requested_editable: BTreeSet<&str> = editable.iter().map(String::as_str).collect();
    let declared_editable: BTreeSet<&str> = plan
        .editable_target_set
        .iter()
        .map(String::as_str)
        .collect();
    for path in &plan.editable_target_set {
        if !requested_editable.contains(path.as_str()) {
            bail!("patch plan editable_target_set includes path outside requested editable set: {path}");
        }
    }

    let declared_files: BTreeSet<&str> = plan
        .files_to_edit
        .iter()
        .map(|edit| edit.path.as_str())
        .collect();
    for edit in &plan.files_to_edit {
        if !declared_editable.contains(edit.path.as_str()) {
            bail!(
                "patch plan file {} is outside editable_target_set",
                edit.path
            );
        }
    }

    for step in &recipe.steps {
        validate_patch_step_headers(step)?;
        for path in patch_step_paths(step) {
            if !declared_editable.contains(path) {
                bail!("patch recipe step path {path} is outside patch plan editable_target_set");
            }
            if !declared_files.is_empty() && !declared_files.contains(path) {
                bail!("patch recipe step path {path} is missing from patch plan files_to_edit");
            }
        }
    }

    Ok(())
}

fn validate_patch_step_headers(step: &PatchStep) -> Result<()> {
    if let PatchStep::UnifiedDiff { path, diff } = step {
        validate_unified_diff_headers(path, diff)?;
    }
    Ok(())
}

fn validate_unified_diff_headers(path: &str, diff: &str) -> Result<()> {
    let mut saw_header = false;
    let lines: Vec<&str> = diff.lines().collect();
    for line in &lines {
        let lower = line.trim_start().to_ascii_lowercase();
        if lower.starts_with("binary files ")
            || lower == "git binary patch"
            || lower.starts_with("new file mode ")
            || lower.starts_with("deleted file mode ")
            || lower.starts_with("old mode ")
            || lower.starts_with("new mode ")
            || lower.starts_with("similarity index ")
            || lower.starts_with("rename from ")
            || lower.starts_with("rename to ")
            || lower.starts_with("subproject commit ")
        {
            bail!("unsupported unsafe unified diff metadata: {}", line.trim());
        }
    }

    let mut index = 0;
    while index < lines.len() {
        if lines[index].trim().is_empty() {
            index += 1;
            continue;
        }
        if !is_diff_file_header_at(&lines, index) {
            bail!(
                "unified diff step for {path} has unexpected line outside a file header: {}",
                lines[index].trim()
            );
        }
        saw_header = true;
        let old_path = clean_diff_path(lines[index].trim_start_matches("--- ").trim());
        let new_path = clean_diff_path(lines[index + 1].trim_start_matches("+++ ").trim());
        for header_path in [old_path, new_path].into_iter().flatten() {
            if header_path != path {
                bail!("unified diff header path {header_path} does not match step path {path}");
            }
        }

        index += 2;
        let mut saw_hunk = false;
        while index < lines.len() && !is_diff_file_header_at(&lines, index) {
            if !lines[index].starts_with("@@") {
                bail!(
                    "unified diff step for {path} has unexpected line before hunk header: {}",
                    lines[index].trim()
                );
            }
            saw_hunk = true;
            let (expected_old, expected_new) = parse_unified_hunk_counts(lines[index])?;
            index += 1;
            let mut old_count = 0usize;
            let mut new_count = 0usize;
            while index < lines.len() {
                let counts_complete = old_count == expected_old && new_count == expected_new;
                if counts_complete {
                    if lines[index].starts_with("\\ ") {
                        index += 1;
                        continue;
                    }
                    if lines[index].starts_with("@@") || is_diff_file_header_at(&lines, index) {
                        break;
                    }
                    bail!(
                        "unified diff hunk for {path} has extra line after declared range: {}",
                        lines[index].trim()
                    );
                }
                if lines[index].starts_with("\\ ") {
                    index += 1;
                    continue;
                }
                if lines[index].starts_with("@@") || is_diff_file_header_at(&lines, index) {
                    break;
                }
                match lines[index].as_bytes().first() {
                    Some(b' ') => {
                        old_count += 1;
                        new_count += 1;
                    }
                    Some(b'-') => old_count += 1,
                    Some(b'+') => new_count += 1,
                    _ => {
                        bail!(
                            "unified diff hunk line for {path} must start with space, '+', '-', or '\\': {}",
                            lines[index].trim()
                        );
                    }
                }
                if old_count > expected_old || new_count > expected_new {
                    bail!(
                        "unified diff hunk for {path} exceeds declared line counts: expected -{expected_old} +{expected_new}, saw -{old_count} +{new_count}"
                    );
                }
                index += 1;
            }
            if old_count != expected_old || new_count != expected_new {
                bail!(
                    "unified diff hunk for {path} has mismatched line counts: expected -{expected_old} +{expected_new}, saw -{old_count} +{new_count}"
                );
            }
        }
        if !saw_hunk {
            bail!("unified diff step for {path} is missing a hunk header");
        }
    }

    if !saw_header {
        bail!("unified diff step for {path} is missing ---/+++ file headers");
    }

    Ok(())
}

fn is_diff_file_header_at(lines: &[&str], index: usize) -> bool {
    index + 1 < lines.len()
        && lines[index].starts_with("--- ")
        && lines[index + 1].starts_with("+++ ")
}

fn parse_unified_hunk_counts(header: &str) -> Result<(usize, usize)> {
    let Some(rest) = header.strip_prefix("@@ ") else {
        bail!("invalid unified diff hunk header: {header}");
    };
    let Some(end) = rest.find(" @@") else {
        bail!("invalid unified diff hunk header: {header}");
    };
    let mut parts = rest[..end].split_whitespace();
    let old_range = parts
        .next()
        .ok_or_else(|| anyhow!("invalid unified diff hunk header: {header}"))?;
    let new_range = parts
        .next()
        .ok_or_else(|| anyhow!("invalid unified diff hunk header: {header}"))?;
    if !old_range.starts_with('-') || !new_range.starts_with('+') {
        bail!("invalid unified diff hunk header: {header}");
    }
    Ok((
        parse_unified_range_count(&old_range[1..], header)?,
        parse_unified_range_count(&new_range[1..], header)?,
    ))
}

fn parse_unified_range_count(range: &str, header: &str) -> Result<usize> {
    let mut parts = range.split(',');
    let start = parts
        .next()
        .ok_or_else(|| anyhow!("invalid unified diff hunk header: {header}"))?;
    start
        .parse::<usize>()
        .map_err(|_| anyhow!("invalid unified diff hunk header: {header}"))?;
    let count = parts
        .next()
        .map(|value| {
            value
                .parse::<usize>()
                .map_err(|_| anyhow!("invalid unified diff hunk header: {header}"))
        })
        .transpose()?
        .unwrap_or(1);
    if parts.next().is_some() {
        bail!("invalid unified diff hunk header: {header}");
    }
    Ok(count)
}

fn clean_diff_path(raw: &str) -> Option<String> {
    let token = raw.split_whitespace().next()?;
    if token == "/dev/null" {
        return None;
    }
    Some(
        token
            .trim_matches('"')
            .strip_prefix("a/")
            .or_else(|| token.trim_matches('"').strip_prefix("b/"))
            .unwrap_or_else(|| token.trim_matches('"'))
            .to_string(),
    )
}

fn added_lines_from_diff(diff: &str) -> String {
    let mut content = String::new();
    for line in diff.lines() {
        if line.starts_with('+') && !line.starts_with("+++") {
            content.push_str(&line[1..]);
            content.push('\n');
        }
    }
    content
}

fn affected_files(plan: &ExecutablePatchPlan) -> Vec<String> {
    let mut files = BTreeSet::new();
    for step in &plan.steps {
        for path in patch_step_paths(step) {
            files.insert(path.to_string());
        }
    }
    files.into_iter().collect()
}

fn render_patch_artifact(plan: &ExecutablePatchPlan) -> String {
    let mut text = String::new();
    for step in &plan.steps {
        match step {
            PatchStep::UnifiedDiff { diff, .. } => {
                text.push_str(diff);
                if !text.ends_with('\n') {
                    text.push('\n');
                }
            }
            PatchStep::Create { path, content } => {
                text.push_str(&format!(
                    "--- /dev/null\n+++ b/{path}\n@@ -0,0 +1,{} @@\n",
                    content.lines().count()
                ));
                for line in content.lines() {
                    text.push_str(&format!("+{line}\n"));
                }
            }
            PatchStep::Delete { path } => {
                text.push_str(&format!("--- a/{path}\n+++ /dev/null\n"));
            }
            other => {
                text.push_str(&format!(
                    "# Non-unified Mimir patch step:\n{}\n",
                    serde_json::to_string_pretty(other).unwrap_or_else(|_| "{}".to_string())
                ));
            }
        }
    }
    text
}

fn apply_patch_plan_to_base(
    plan: &ExecutablePatchPlan,
    base: &camino::Utf8Path,
    editable_set: &EditableSet,
) -> std::result::Result<(), mimir_edit::EditError> {
    for step in &plan.steps {
        PatchEngine::apply(step, base, editable_set)?;
    }
    Ok(())
}

fn apply_patch_plan_with_guards(
    plan: &ExecutablePatchPlan,
    base: &camino::Utf8Path,
    editable_set: &EditableSet,
    run_id: &RunId,
    backup_namespace: &str,
    files: &[String],
    allowed_dirty: &BTreeSet<String>,
) -> Result<()> {
    ensure_targets_not_dirty(base, files, allowed_dirty)?;
    let backup_dir = format!(".mimir/runs/{run_id}/backups/{backup_namespace}");

    for file in files {
        ensure_no_symlink_components(base, file)?;
    }

    let mut transaction = PatchTransaction::new(base, editable_set, backup_dir)
        .map_err(|error| anyhow!(error.to_string()))?;
    transaction
        .apply_all(&plan.steps)
        .map_err(|error| anyhow!(error.to_string()))?;
    transaction
        .commit()
        .map_err(|error| anyhow!(error.to_string()))?;

    Ok(())
}

fn ensure_targets_not_dirty(
    base: &camino::Utf8Path,
    files: &[String],
    allowed_dirty: &BTreeSet<String>,
) -> Result<()> {
    let status = WorktreeStatus::check(base);
    if !status.is_git_repo {
        return Ok(());
    }
    if let Some(error) = status.status_error {
        bail!("dirty_worktree: unable to inspect git status safely: {error}");
    }

    let allowed_dirty = allowed_dirty
        .iter()
        .filter_map(|path| normalized_relative_string(path))
        .collect::<BTreeSet<_>>();
    let dirty: Vec<String> = files
        .iter()
        .filter_map(|path| normalized_relative_string(path))
        .filter(|path| status.is_file_dirty(path) && !allowed_dirty.contains(path))
        .collect();
    if dirty.is_empty() {
        Ok(())
    } else {
        bail!(
            "dirty_worktree: refusing to modify uncommitted target file(s): {}",
            dirty.into_iter().collect::<Vec<_>>().join(", ")
        )
    }
}

fn validate_patch_plan_dry_run(
    plan: &ExecutablePatchPlan,
    base: &camino::Utf8Path,
    editable: &[String],
    run_id: &str,
) -> Result<()> {
    let temp_root =
        std::env::temp_dir().join(format!("mimir-dry-run-{run_id}-{}", std::process::id()));
    if temp_root.exists() {
        fs::remove_dir_all(&temp_root)?;
    }
    fs::create_dir_all(&temp_root)?;
    copy_editable_files(base, &temp_root, editable)?;
    let temp_utf8 = camino::Utf8PathBuf::from_path_buf(temp_root.clone())
        .map_err(|path| anyhow!("temporary dry-run path is not UTF-8: {}", path.display()))?;
    let editable_set = EditableSet::from_paths(editable.to_vec());
    let result = apply_patch_plan_to_base(plan, &temp_utf8, &editable_set)
        .map_err(|err| anyhow!(err.to_string()));
    let cleanup = fs::remove_dir_all(&temp_root).map_err(anyhow::Error::from);
    result.and(cleanup)
}

fn run_code_tests_named(
    run_dir: &RunDir,
    base: &camino::Utf8Path,
    options: &CodeCommandOptions,
    suggested_commands: &[String],
    repair_suggested_commands: &[String],
    artifact_name: &str,
) -> Result<CodeTestOutcome> {
    if options.no_test {
        return Ok(CodeTestOutcome {
            policy: "skipped_by_flag".to_string(),
            path: None,
            result: None,
        });
    }
    if options.dry_run {
        return Ok(CodeTestOutcome {
            policy: "skipped_dry_run".to_string(),
            path: None,
            result: None,
        });
    }

    let provider_suggested_commands = capped_recorded_commands(suggested_commands);
    let repair_suggested_commands = capped_recorded_commands(repair_suggested_commands);
    let skipped_provider_commands = skipped_commands(
        &provider_suggested_commands,
        "provider-suggested test commands are recorded but not executed automatically",
    );
    let skipped_repair_suggested_commands = skipped_commands(
        &repair_suggested_commands,
        "repair-suggested test commands are recorded distinctly but not executed automatically",
    );

    let detected = detect_framework(base);
    let (policy, result, error) = if matches!(
        detected,
        TestFramework::Vitest | TestFramework::Jest | TestFramework::Mocha
    ) {
        (
            "skipped_unsafe_test_framework".to_string(),
            None,
            Some("detected JavaScript test framework would use npx; not auto-running".to_string()),
        )
    } else if let Some(reason) = suspicious_test_hook_reason(base, detected) {
        (
            "skipped_suspicious_test_hooks".to_string(),
            None,
            Some(reason),
        )
    } else {
        match run_tests(base, Some(detected)) {
            Ok(result) => (
                "auto_detected".to_string(),
                Some(sanitize_test_result(result)),
                None,
            ),
            Err(error) => (
                "auto_detect_failed".to_string(),
                None,
                Some(redacted_message(error)),
            ),
        }
    };

    let artifact = TestRunArtifact {
        schema_version: 1,
        policy: policy.clone(),
        provider_suggested_commands,
        skipped_provider_commands,
        repair_suggested_commands,
        skipped_repair_suggested_commands,
        result: result.clone(),
        error,
    };
    let path = run_dir.root().join(artifact_name);
    write_redacted_json_artifact(&path, &artifact, ArtifactSizeClass::TestResult)?;
    append_redacted_event(
        run_dir,
        &serde_json::json!({
            "event_type": "test_completed",
            "timestamp": chrono::Utc::now().to_rfc3339(),
            "policy": policy,
            "passed": result.as_ref().map(|result| result.passed),
            "exit_code": result.as_ref().map(|result| result.exit_code),
            "artifact": path.to_string(),
        }),
    )?;

    Ok(CodeTestOutcome {
        policy: artifact.policy,
        path: Some(path.to_string()),
        result,
    })
}

fn capped_recorded_commands(commands: &[String]) -> Vec<String> {
    let mut output = commands
        .iter()
        .take(MAX_RECORDED_TEST_COMMANDS)
        .map(|command| {
            redacted_message(truncate_for_artifact(
                command,
                MAX_RECORDED_TEST_COMMAND_CHARS,
            ))
        })
        .collect::<Vec<_>>();
    let omitted = commands.len().saturating_sub(MAX_RECORDED_TEST_COMMANDS);
    if omitted > 0 {
        output.push(format!(
            "... {omitted} additional command(s) omitted by artifact size cap ..."
        ));
    }
    output
}

fn skipped_commands(commands: &[String], reason: &str) -> Vec<SkippedProviderCommand> {
    commands
        .iter()
        .cloned()
        .map(|command| SkippedProviderCommand {
            command,
            reason: reason.to_string(),
        })
        .collect()
}

fn suspicious_test_hook_reason(
    base: &camino::Utf8Path,
    framework: TestFramework,
) -> Option<String> {
    match framework {
        TestFramework::CargoTest => {
            for hook in ["build.rs", ".cargo/config", ".cargo/config.toml"] {
                if base.join(hook).exists() {
                    return Some(format!(
                        "detected Cargo hook/config `{hook}`; not auto-running tests without explicit opt-in"
                    ));
                }
            }
            if let Some(path) = find_suspicious_file(base.as_std_path(), |path| {
                path.file_name().is_some_and(|name| name == "build.rs")
                    || path.file_name().is_some_and(|name| name == "Cargo.toml")
                        && cargo_manifest_declares_build_script(path)
            }) {
                return Some(format!(
                    "detected Cargo build hook `{}`; not auto-running tests without explicit opt-in",
                    display_relative_path(base.as_std_path(), &path)
                ));
            }
            None
        }
        TestFramework::Pytest => {
            if base.join("conftest.py").exists() {
                return Some(
                    "detected Pytest conftest.py; not auto-running tests without explicit opt-in"
                        .to_string(),
                );
            }
            for config in ["pyproject.toml", "pytest.ini", "setup.cfg", "tox.ini"] {
                let path = base.join(config);
                let Ok(content) = fs::read_to_string(&path) else {
                    continue;
                };
                if content.contains("pytest_plugins") {
                    return Some(format!(
                        "detected Pytest plugin config in `{config}`; not auto-running tests without explicit opt-in"
                    ));
                }
            }
            if let Some(path) = find_suspicious_file(base.as_std_path(), |path| {
                let Some(file_name) = path.file_name().and_then(|name| name.to_str()) else {
                    return false;
                };
                if file_name == "conftest.py" {
                    return true;
                }
                matches!(
                    file_name,
                    "pyproject.toml" | "pytest.ini" | "setup.cfg" | "tox.ini"
                ) && fs::read_to_string(path)
                    .map(|content| content.contains("pytest_plugins"))
                    .unwrap_or(false)
            }) {
                return Some(format!(
                    "detected Pytest hook/plugin `{}`; not auto-running tests without explicit opt-in",
                    display_relative_path(base.as_std_path(), &path)
                ));
            }
            None
        }
        TestFramework::Vitest
        | TestFramework::Jest
        | TestFramework::Mocha
        | TestFramework::Unknown => None,
    }
}

fn find_suspicious_file(
    root: &Path,
    mut is_suspicious: impl FnMut(&Path) -> bool,
) -> Option<PathBuf> {
    fn visit(dir: &Path, is_suspicious: &mut impl FnMut(&Path) -> bool) -> Option<PathBuf> {
        let entries = fs::read_dir(dir).ok()?;
        for entry in entries.flatten() {
            let path = entry.path();
            let file_name = entry.file_name();
            let name = file_name.to_string_lossy();
            if path.is_dir() {
                if matches!(
                    name.as_ref(),
                    ".git" | ".mimir" | "target" | "node_modules" | ".venv" | "venv"
                ) {
                    continue;
                }
                if let Some(found) = visit(&path, is_suspicious) {
                    return Some(found);
                }
            } else if is_suspicious(&path) {
                return Some(path);
            }
        }
        None
    }

    visit(root, &mut is_suspicious)
}

fn cargo_manifest_declares_build_script(path: &Path) -> bool {
    fs::read_to_string(path)
        .map(|content| {
            content
                .lines()
                .map(str::trim)
                .any(|line| line.starts_with("build") && line.contains('='))
        })
        .unwrap_or(false)
}

fn display_relative_path(base: &Path, path: &Path) -> String {
    path.strip_prefix(base)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

fn sanitize_test_result(mut result: TestRunResult) -> TestRunResult {
    result.command = redacted_message(result.command);
    result.stdout = redacted_message(truncate_for_artifact(&result.stdout, 20_000));
    result.stderr = redacted_message(truncate_for_artifact(&result.stderr, 10_000));
    result
}

fn truncate_for_artifact(value: &str, max_chars: usize) -> String {
    let truncated = value.chars().take(max_chars).collect::<String>();
    if truncated.len() == value.len() {
        value.to_string()
    } else {
        format!("{truncated}\n... truncated ...")
    }
}

fn test_failure_reason(result: Option<&TestRunResult>) -> Option<String> {
    result.filter(|result| !result.passed).map(|result| {
        redacted_message(format!(
            "tests_failed: auto-detected tests exited with status {}",
            result.exit_code
        ))
    })
}

fn run_code_repair_loop(
    ctx: RepairRunContext<'_>,
    initial_test: TestRunResult,
    run_owned_files: &mut BTreeSet<String>,
    run_owned_hashes: &mut BTreeMap<String, Option<u64>>,
) -> Result<RepairLoopOutcome> {
    let mut turns = Vec::new();
    let mut total_cost = ctx.initial_spend;
    let mut current_test = initial_test;

    append_redacted_event(
        ctx.run_dir,
        &serde_json::json!({
            "event_type": "repair_started",
            "timestamp": chrono::Utc::now().to_rfc3339(),
            "max_repair_turns": ctx.options.max_repair_turns,
            "cost_cap": ctx.options.cost_cap,
            "initial_spend": ctx.initial_spend,
        }),
    )?;

    if ctx.options.max_repair_turns == 0 {
        return Ok(repair_outcome(
            turns,
            total_cost,
            "max_repair_turns_reached",
            false,
            "auto_detected".to_string(),
            None,
            Some(current_test.clone()),
            test_failure_reason(Some(&current_test)),
        ));
    }

    for turn in 1..=ctx.options.max_repair_turns {
        if let Err(error) = ensure_run_owned_files_unchanged(ctx.base, run_owned_hashes) {
            let reason = redacted_message(format!("repair_run_owned_files_changed: {error}"));
            turns.push(RepairTurnReport {
                turn,
                cost: 0.0,
                patch_plan_path: None,
                patch_recipe_path: None,
                patch_path: None,
                test_result_path: None,
                test_passed: Some(false),
                test_exit_code: Some(current_test.exit_code),
                rejected: Some(reason.clone()),
            });
            append_redacted_event(
                ctx.run_dir,
                &serde_json::json!({
                    "event_type": "repair_run_owned_files_changed",
                    "timestamp": chrono::Utc::now().to_rfc3339(),
                    "turn": turn,
                    "reason": &reason,
                }),
            )?;
            return Ok(repair_outcome(
                turns,
                total_cost,
                reason.clone(),
                false,
                "auto_detected".to_string(),
                None,
                Some(current_test),
                Some(reason),
            ));
        }
        let request = repair_request_from_failure(
            ctx.packet,
            &ctx.options.editable,
            ctx.base,
            turn,
            ctx.options.max_repair_turns,
            ctx.options.cost_cap,
            total_cost,
            &current_test,
        )?;
        let preflight_cost =
            estimate_repair_request_cost(&ctx.options.provider, &ctx.packet.model, &request);
        if let Some(reason) =
            repair_cost_preflight_rejection(total_cost, preflight_cost, ctx.options.cost_cap)
        {
            let reason = redacted_message(reason);
            turns.push(RepairTurnReport {
                turn,
                cost: 0.0,
                patch_plan_path: None,
                patch_recipe_path: None,
                patch_path: None,
                test_result_path: None,
                test_passed: Some(false),
                test_exit_code: Some(current_test.exit_code),
                rejected: Some(reason.clone()),
            });
            append_redacted_event(
                ctx.run_dir,
                &serde_json::json!({
                    "event_type": "repair_cost_cap_preflight_exceeded",
                    "timestamp": chrono::Utc::now().to_rfc3339(),
                    "turn": turn,
                    "estimated_request_cost": preflight_cost,
                    "total_cost": total_cost,
                    "cost_cap": ctx.options.cost_cap,
                    "reason": &reason,
                }),
            )?;
            return Ok(repair_outcome(
                turns,
                total_cost,
                reason.clone(),
                false,
                "auto_detected".to_string(),
                None,
                Some(current_test),
                Some(reason),
            ));
        }
        let request_artifact = format!("repair_request.redacted.turn-{turn}.json");
        write_redacted_json_artifact(
            &ctx.run_dir.root().join(&request_artifact),
            &request,
            ArtifactSizeClass::Repair,
        )?;

        let response = match call_provider_with_request(
            ctx.packet,
            request,
            false,
            false,
            ctx.options.base_url.clone(),
        ) {
            Ok(response) => response,
            Err(error) => {
                let reason = redacted_message(format!("repair_provider_error: {error}"));
                turns.push(RepairTurnReport {
                    turn,
                    cost: 0.0,
                    patch_plan_path: None,
                    patch_recipe_path: None,
                    patch_path: None,
                    test_result_path: None,
                    test_passed: Some(false),
                    test_exit_code: Some(current_test.exit_code),
                    rejected: Some(reason.clone()),
                });
                return Ok(repair_outcome(
                    turns,
                    total_cost,
                    reason.clone(),
                    false,
                    "auto_detected".to_string(),
                    None,
                    Some(current_test),
                    Some(reason),
                ));
            }
        };

        let response_artifact = format!("repair_response.turn-{turn}.json");
        write_redacted_json_artifact(
            &ctx.run_dir.root().join(&response_artifact),
            &response,
            ArtifactSizeClass::ProviderResponse,
        )?;
        append_redacted_event(
            ctx.run_dir,
            &serde_json::json!({
                "event_type": "repair_provider_response",
                "timestamp": chrono::Utc::now().to_rfc3339(),
                "turn": turn,
                "model": &response.model,
                "usage": &response.usage,
                "stop_reason": &response.stop_reason,
                "artifact": response_artifact,
            }),
        )?;

        let turn_cost =
            estimate_provider_cost(&ctx.options.provider, &response.model, &response.usage);
        if total_cost + turn_cost > ctx.options.cost_cap {
            total_cost += turn_cost;
            let reason = redacted_message(format!(
                "repair_cost_cap_exceeded: estimated repair cost ${total_cost:.6} exceeds cap ${:.6}",
                ctx.options.cost_cap
            ));
            turns.push(RepairTurnReport {
                turn,
                cost: turn_cost,
                patch_plan_path: None,
                patch_recipe_path: None,
                patch_path: None,
                test_result_path: None,
                test_passed: Some(false),
                test_exit_code: Some(current_test.exit_code),
                rejected: Some(reason.clone()),
            });
            return Ok(repair_outcome(
                turns,
                total_cost,
                reason.clone(),
                false,
                "auto_detected".to_string(),
                None,
                Some(current_test),
                Some(reason),
            ));
        }
        total_cost += turn_cost;

        let text = response_text(&response);
        let parsed = parse_patch_plan_response(&text, ctx.packet, &ctx.options.editable);
        let (patch_plan, patch_recipe, _notes) = match parsed {
            Ok(parsed) => parsed,
            Err(error) => {
                let reason = redacted_message(format!("repair_patch_malformed: {error}"));
                turns.push(RepairTurnReport {
                    turn,
                    cost: turn_cost,
                    patch_plan_path: None,
                    patch_recipe_path: None,
                    patch_path: None,
                    test_result_path: None,
                    test_passed: Some(false),
                    test_exit_code: Some(current_test.exit_code),
                    rejected: Some(reason.clone()),
                });
                return Ok(repair_outcome(
                    turns,
                    total_cost,
                    reason.clone(),
                    false,
                    "auto_detected".to_string(),
                    None,
                    Some(current_test),
                    Some(reason),
                ));
            }
        };
        let repair_tests_to_run = patch_plan.tests_to_run.clone();

        let patch_plan_artifact = format!("repair_patch_plan.turn-{turn}.json");
        let patch_recipe_artifact = format!("repair_patch_recipe.turn-{turn}.json");
        let patch_artifact = format!("repair_patch.turn-{turn}.diff");
        write_redacted_json_artifact(
            &ctx.run_dir.root().join(&patch_plan_artifact),
            &patch_plan,
            ArtifactSizeClass::PatchPlan,
        )?;
        write_redacted_json_artifact(
            &ctx.run_dir.root().join(&patch_recipe_artifact),
            &patch_recipe,
            ArtifactSizeClass::PatchRecipe,
        )?;
        let patch_text = render_patch_artifact(&patch_recipe);
        write_redacted_bytes_artifact(
            &ctx.run_dir.root().join(&patch_artifact),
            patch_text.as_bytes(),
            ArtifactSizeClass::PatchDiff,
        )?;

        let files = affected_files(&patch_recipe);
        let safety_result = if contains_secret_like_text(&patch_text) {
            Err(anyhow!(
                "provider repair patch contained secret-like text and was not applied"
            ))
        } else {
            validate_patch_plan_binding(
                &patch_plan,
                &patch_recipe,
                ctx.packet,
                &ctx.options.editable,
            )
            .and_then(|()| {
                validate_patch_plan_dry_run(
                    &patch_recipe,
                    ctx.base,
                    &ctx.options.editable,
                    &format!("{}-repair-{turn}", ctx.packet.run_id),
                )
            })
        };

        if let Err(error) = safety_result {
            let reason = redacted_message(format!("repair_patch_rejected: {error}"));
            turns.push(RepairTurnReport {
                turn,
                cost: turn_cost,
                patch_plan_path: Some(patch_plan_artifact),
                patch_recipe_path: Some(patch_recipe_artifact),
                patch_path: Some(patch_artifact),
                test_result_path: None,
                test_passed: Some(false),
                test_exit_code: Some(current_test.exit_code),
                rejected: Some(reason.clone()),
            });
            append_redacted_event(
                ctx.run_dir,
                &serde_json::json!({
                    "event_type": "repair_patch_rejected",
                    "timestamp": chrono::Utc::now().to_rfc3339(),
                    "turn": turn,
                    "reason": &reason,
                }),
            )?;
            return Ok(repair_outcome(
                turns,
                total_cost,
                reason.clone(),
                false,
                "auto_detected".to_string(),
                None,
                Some(current_test),
                Some(reason),
            ));
        }

        if let Err(error) = ensure_run_owned_files_unchanged(ctx.base, run_owned_hashes) {
            let reason = redacted_message(format!(
                "repair_run_owned_files_changed_before_apply: {error}"
            ));
            turns.push(RepairTurnReport {
                turn,
                cost: turn_cost,
                patch_plan_path: Some(patch_plan_artifact),
                patch_recipe_path: Some(patch_recipe_artifact),
                patch_path: Some(patch_artifact),
                test_result_path: None,
                test_passed: Some(false),
                test_exit_code: Some(current_test.exit_code),
                rejected: Some(reason.clone()),
            });
            append_redacted_event(
                ctx.run_dir,
                &serde_json::json!({
                    "event_type": "repair_run_owned_files_changed",
                    "timestamp": chrono::Utc::now().to_rfc3339(),
                    "turn": turn,
                    "reason": &reason,
                }),
            )?;
            return Ok(repair_outcome(
                turns,
                total_cost,
                reason.clone(),
                false,
                "auto_detected".to_string(),
                None,
                Some(current_test),
                Some(reason),
            ));
        }

        if let Err(error) = apply_patch_plan_with_guards(
            &patch_recipe,
            ctx.base,
            ctx.editable_set,
            &RunId(ctx.packet.run_id.clone()),
            &format!("repair-{turn}"),
            &files,
            run_owned_files,
        ) {
            let reason = redacted_message(format!("repair_patch_apply_failed: {error}"));
            turns.push(RepairTurnReport {
                turn,
                cost: turn_cost,
                patch_plan_path: Some(patch_plan_artifact),
                patch_recipe_path: Some(patch_recipe_artifact),
                patch_path: Some(patch_artifact),
                test_result_path: None,
                test_passed: Some(false),
                test_exit_code: Some(current_test.exit_code),
                rejected: Some(reason.clone()),
            });
            return Ok(repair_outcome(
                turns,
                total_cost,
                reason.clone(),
                false,
                "auto_detected".to_string(),
                None,
                Some(current_test),
                Some(reason),
            ));
        }
        run_owned_files.extend(files.iter().cloned());
        *run_owned_hashes = snapshot_file_hashes(ctx.base, run_owned_files)?;

        append_redacted_event(
            ctx.run_dir,
            &serde_json::json!({
                "event_type": "repair_patch_applied",
                "timestamp": chrono::Utc::now().to_rfc3339(),
                "turn": turn,
                "affected_files": files,
            }),
        )?;

        let test_artifact = format!("test_result.turn-{turn}.json");
        let outcome = run_code_tests_named(
            ctx.run_dir,
            ctx.base,
            ctx.options,
            ctx.suggested_commands,
            &repair_tests_to_run,
            &test_artifact,
        )?;
        let turn_report = RepairTurnReport {
            turn,
            cost: turn_cost,
            patch_plan_path: Some(patch_plan_artifact),
            patch_recipe_path: Some(patch_recipe_artifact),
            patch_path: Some(patch_artifact),
            test_result_path: outcome.path.clone(),
            test_passed: outcome.result.as_ref().map(|result| result.passed),
            test_exit_code: outcome.result.as_ref().map(|result| result.exit_code),
            rejected: None,
        };

        if let Some(result) = outcome.result {
            let failure = test_failure_reason(Some(&result));
            turns.push(turn_report);
            if failure.is_none() {
                return Ok(repair_outcome(
                    turns,
                    total_cost,
                    "tests_passed",
                    true,
                    outcome.policy,
                    outcome.path,
                    Some(result),
                    None,
                ));
            }
            current_test = result;
        } else {
            let reason = redacted_message("repair_tests_did_not_run");
            turns.push(RepairTurnReport {
                rejected: Some(reason.clone()),
                ..turn_report
            });
            return Ok(repair_outcome(
                turns,
                total_cost,
                reason.clone(),
                false,
                outcome.policy,
                outcome.path,
                None,
                Some(reason),
            ));
        }
    }

    let reason = redacted_message("max_repair_turns_reached");
    Ok(repair_outcome(
        turns,
        total_cost,
        reason.clone(),
        false,
        "auto_detected".to_string(),
        None,
        Some(current_test.clone()),
        test_failure_reason(Some(&current_test)).or(Some(reason)),
    ))
}

#[allow(clippy::too_many_arguments)]
fn repair_outcome(
    turns: Vec<RepairTurnReport>,
    total_cost: f64,
    stop_reason: impl Into<String>,
    converged: bool,
    test_policy: String,
    test_result_path: Option<String>,
    test_result: Option<TestRunResult>,
    test_failure: Option<String>,
) -> RepairLoopOutcome {
    RepairLoopOutcome {
        summary: RepairLoopSummary {
            attempted: true,
            converged,
            turns_executed: turns.len() as u32,
            total_cost,
            stop_reason: stop_reason.into(),
            turns,
        },
        test_policy,
        test_result_path,
        test_result,
        test_failure,
    }
}

#[allow(clippy::too_many_arguments)]
fn repair_request_from_failure(
    packet: &mimir_schemas::ContextPacket,
    editable: &[String],
    base: &camino::Utf8Path,
    turn: u32,
    max_repair_turns: u32,
    cost_cap: f64,
    spent: f64,
    test_result: &TestRunResult,
) -> Result<ProviderRequest> {
    let editable_json = serde_json::to_string(editable).unwrap_or_else(|_| "[]".to_string());
    let test_json = redacted_json_string(test_result)?;
    let current_files = current_editable_snapshot(base, editable)?;
    let prompt = format!(
        "Repair the previously applied Mimir patch. Tests failed after patch application.\n\nEditable target set: {editable_json}\nCurrent packet_id: {}\nRepair turn: {turn}/{max_repair_turns}\nRepair cost spent USD: {spent:.6}\nRepair cost cap USD: {cost_cap:.6}\n\nCurrent editable file snapshot after the failed patch:\n{current_files}\n\nFailing test result JSON:\n{test_json}\n\nReturn only JSON with this wrapper shape: {{\"patch_plan\":{{...PatchPlan metadata bound to packet_id {}}},\"patch_recipe\":{{\"schema_version\":1,\"plan_id\":\"repair-plan-id\",\"packet_id\":\"{}\",\"steps\":[...]}}}}. The patch_recipe must contain only the minimal repair edits. All paths must remain inside the editable target set. Do not include provider API keys, secrets, markdown, shell commands, binary diffs, mode changes, renames, or submodule metadata.\n\n{}",
        packet.packet_id,
        packet.packet_id,
        packet.packet_id,
        context_packet_summary(packet)?
    );
    Ok(request_from_packet_with_prompt(
        packet,
        "You are Mimir Repair. Produce only safe, minimal repair PatchPlan JSON. Never propose edits outside the editable target set.",
        prompt,
        packet.output_reserve_tokens,
    ))
}

fn current_editable_snapshot(base: &camino::Utf8Path, editable: &[String]) -> Result<String> {
    let mut snapshot = String::new();
    for path in editable {
        let Some(_) = safe_relative_path(path) else {
            continue;
        };
        ensure_no_symlink_components(base, path)?;
        let full_path = base.join(path);
        if !full_path.is_file() {
            continue;
        }
        let content = fs::read_to_string(&full_path)
            .map_err(|error| anyhow!("read current editable file {path}: {error}"))?;
        snapshot.push_str(&format!(
            "\n--- {path} ---\n{}\n",
            redacted_message(truncate_for_artifact(&content, 20_000))
        ));
    }
    if snapshot.is_empty() {
        Ok("No current editable file content was available.".to_string())
    } else {
        Ok(snapshot)
    }
}

fn snapshot_file_hashes(
    base: &camino::Utf8Path,
    files: &BTreeSet<String>,
) -> Result<BTreeMap<String, Option<u64>>> {
    let mut hashes = BTreeMap::new();
    for file in files {
        let path = base.join(file);
        let hash = if path.is_file() {
            Some(stable_byte_hash(&fs::read(&path)?))
        } else {
            None
        };
        hashes.insert(file.clone(), hash);
    }
    Ok(hashes)
}

fn ensure_run_owned_files_unchanged(
    base: &camino::Utf8Path,
    expected_hashes: &BTreeMap<String, Option<u64>>,
) -> Result<()> {
    for (file, expected) in expected_hashes {
        let path = base.join(file);
        let current = if path.is_file() {
            Some(stable_byte_hash(&fs::read(&path)?))
        } else {
            None
        };
        if current != *expected {
            bail!("dirty_worktree: refusing repair because run-owned file changed after Mimir wrote it: {file}");
        }
    }
    Ok(())
}

fn stable_byte_hash(bytes: &[u8]) -> u64 {
    let mut hasher = DefaultHasher::new();
    bytes.hash(&mut hasher);
    hasher.finish()
}

fn estimate_provider_cost(provider: &str, model: &str, usage: &TokenUsage) -> f64 {
    let provider = normalized_provider(provider);
    let model = model.to_ascii_lowercase();
    let registry_pricing = mimir_providers::capabilities::CapabilityRegistry::bundled()
        .ok()
        .and_then(|registry| registry.model(&provider, &model).cloned())
        .map(|capabilities| {
            (
                capabilities.pricing.input_per_million,
                capabilities.pricing.output_per_million,
            )
        });
    let (input_per_million, output_per_million) =
        registry_pricing.unwrap_or_else(|| match provider.as_str() {
            "anthropic" if model.contains("opus") => (15.0, 75.0),
            "anthropic" if model.contains("sonnet") => (3.0, 15.0),
            "anthropic" if model.contains("haiku") => (1.0, 5.0),
            "anthropic" => (1.0, 5.0),
            "glm" | "zai" => (0.15, 0.60),
            _ => (0.50, 2.00),
        });
    let input = f64::from(usage.input_tokens) * input_per_million / 1_000_000.0;
    let output = f64::from(usage.output_tokens) * output_per_million / 1_000_000.0;
    let cost = input + output;
    if cost > 0.0 {
        cost.max(0.000_001)
    } else {
        0.0
    }
}

fn estimate_repair_request_cost(provider: &str, model: &str, request: &ProviderRequest) -> f64 {
    let usage = TokenUsage {
        input_tokens: request_input_tokens(request),
        output_tokens: request.max_tokens.unwrap_or_default(),
        cache_creation_input_tokens: None,
        cache_read_input_tokens: None,
    };
    estimate_provider_cost(provider, model, &usage)
}

fn repair_cost_preflight_rejection(
    spent: f64,
    estimated_turn_cost: f64,
    cost_cap: f64,
) -> Option<String> {
    (spent + estimated_turn_cost > cost_cap).then(|| {
        format!(
            "repair_cost_cap_preflight_exceeded: estimated repair request cost ${estimated_turn_cost:.6} plus spent ${spent:.6} exceeds cap ${cost_cap:.6}"
        )
    })
}

fn initial_cost_preflight_rejection(estimated_cost: f64, cost_cap: f64) -> Option<String> {
    (estimated_cost > cost_cap).then(|| {
        format!(
            "initial_cost_cap_preflight_exceeded: estimated provider request cost ${estimated_cost:.6} exceeds cap ${cost_cap:.6}"
        )
    })
}

fn provider_response_cost_rejection(actual_cost: f64, cost_cap: f64) -> Option<String> {
    (actual_cost > cost_cap).then(|| {
        format!(
            "initial_cost_cap_exceeded: provider response cost ${actual_cost:.6} exceeds cap ${cost_cap:.6}"
        )
    })
}

fn copy_editable_files(
    base: &camino::Utf8Path,
    temp_root: &std::path::Path,
    editable: &[String],
) -> Result<()> {
    for path in editable {
        let Some(relative) = safe_relative_path(path) else {
            continue;
        };
        ensure_no_symlink_components(base, path)?;
        let source = base.join(path);
        if !source.is_file() {
            continue;
        }
        let destination = temp_root.join(&relative);
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::copy(&source, destination)?;
    }
    Ok(())
}

fn safe_relative_path(path: &str) -> Option<PathBuf> {
    if path.is_empty() || path.contains('\\') {
        return None;
    }
    let candidate = std::path::Path::new(path);
    if candidate.is_absolute() {
        return None;
    }
    let mut normalized = PathBuf::new();
    for component in candidate.components() {
        match component {
            Component::Normal(part) => normalized.push(part),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => return None,
        }
    }
    (!normalized.as_os_str().is_empty()).then_some(normalized)
}

fn normalized_relative_string(path: &str) -> Option<String> {
    safe_relative_path(path).map(|path| path.to_string_lossy().replace('\\', "/"))
}

fn ensure_no_symlink_components(base: &camino::Utf8Path, path: &str) -> Result<()> {
    let Some(relative) = safe_relative_path(path) else {
        bail!("file_not_editable: {path} is not a safe relative path");
    };

    let mut candidate = base.as_std_path().to_path_buf();
    for component in relative.components() {
        candidate.push(component.as_os_str());
        match fs::symlink_metadata(&candidate) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                bail!("file_not_editable: refusing symlink-backed editable path {path}");
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => break,
            Err(error) => bail!("io_error: stat {path}: {error}"),
        }
    }

    Ok(())
}

fn write_patch_report(run_dir: &RunDir, report: &PatchReport) -> Result<()> {
    write_redacted_json(&run_dir.root().join("patch_report.json"), report)
}

fn reject_patch_plan(
    run_dir: &RunDir,
    packet: &mimir_schemas::ContextPacket,
    patch_plan: &PatchPlan,
    options: &CodeCommandOptions,
    rejection: PatchRejection,
) -> Result<()> {
    let report = PatchReport {
        schema_version: 1,
        run_id: packet.run_id.clone(),
        packet_id: packet.packet_id.clone(),
        plan_id: patch_plan.plan_id.clone(),
        dry_run: options.dry_run,
        applied: false,
        affected_files: rejection.affected_files,
        tests_to_run: rejection.tests_to_run,
        max_repair_turns: options.max_repair_turns,
        cost_cap: options.cost_cap,
        test_policy: if options.no_test {
            "skipped_by_flag".to_string()
        } else {
            "not_run_patch_rejected".to_string()
        },
        test_result_path: None,
        test_passed: None,
        test_exit_code: None,
        repair: None,
        notes: rejection.notes,
        rejected: Some(rejection.reason.clone()),
    };
    write_patch_report(run_dir, &report)?;
    append_redacted_event(
        run_dir,
        &serde_json::json!({
            "event_type": "patch_rejected",
            "timestamp": chrono::Utc::now().to_rfc3339(),
            "reason": rejection.reason,
        }),
    )?;
    Ok(())
}

fn run_plan_command(
    task: String,
    editable: Vec<String>,
    provider: String,
    model: Option<String>,
    base_url: Option<String>,
    json: bool,
) -> Result<()> {
    let provider = normalized_provider(&provider);
    let model = model.unwrap_or_else(|| default_model(&provider).to_string());
    let run_id = RunId::generate();
    let mimir_root = camino::Utf8PathBuf::from(".mimir");
    let run_dir = RunDir::create(&mimir_root, &run_id)?;
    let packet = build_packet(
        &task,
        "plan",
        run_id.clone(),
        &provider,
        &model,
        editable.clone(),
    )?;
    let packet_json = serde_json::to_vec_pretty(&packet)?;
    atomic_write(&run_dir.context_packet_path(), &packet_json)?;

    let request = plan_request_from_packet(&packet)?;
    write_provider_request_artifact(&run_dir, &request)?;
    let response = call_provider_with_request(&packet, request, false, false, base_url)?;
    write_provider_artifacts(&run_dir, &response)?;

    let text = response_text(&response);
    let mut plan = plan_artifact_from_response(&packet, &editable, &text);
    redact_plan_artifact(&mut plan);
    let markdown = render_plan_markdown(&plan);
    atomic_write(&run_dir.root().join("plan.md"), markdown.as_bytes())?;
    write_redacted_json(&run_dir.root().join("plan.json"), &plan)?;
    append_redacted_event(
        &run_dir,
        &serde_json::json!({
            "event_type": "plan_generated",
            "timestamp": chrono::Utc::now().to_rfc3339(),
            "step_count": plan.steps.len(),
            "risk_count": plan.risks.len(),
        }),
    )?;

    if json {
        let plan_path = run_dir.root().join("plan.md").to_string();
        let plan_json_path = run_dir.root().join("plan.json").to_string();
        let output = serde_json::json!({
            "run_id": run_id.to_string(),
            "packet_id": packet.packet_id,
            "packet_hash": packet.packet_hash,
            "plan_path": plan_path,
            "plan_json_path": plan_json_path,
            "steps": plan.steps,
            "risks": plan.risks,
            "files_likely_affected": plan.files_likely_affected,
            "tests_to_run": plan.tests_to_run,
        });
        println!("{}", redacted_json_string(&output)?);
    } else {
        println!("Run ID: {run_id}");
        println!("Plan: {}", run_dir.root().join("plan.md"));
        println!("Steps: {}", plan.steps.len());
        for (index, step) in plan.steps.iter().enumerate() {
            println!("  {}. {}", index + 1, step);
        }
        if !plan.files_likely_affected.is_empty() {
            println!(
                "Files likely affected: {}",
                plan.files_likely_affected.join(", ")
            );
        }
        if !plan.tests_to_run.is_empty() {
            let suggested = plan
                .tests_to_run
                .iter()
                .map(redacted_message)
                .collect::<Vec<_>>()
                .join("; ");
            println!("Suggested tests: {suggested}");
        }
    }

    Ok(())
}

fn run_code_command(options: CodeCommandOptions) -> Result<()> {
    if options.editable.is_empty() {
        bail!("mimir code requires at least one --editable path so patch safety can be enforced");
    }

    let (task, command_recipe) =
        resolve_code_task_recipe(&options.task, options.recipe.as_deref(), &options.params)?;
    let provider = normalized_provider(&options.provider);
    let model = options
        .model
        .clone()
        .unwrap_or_else(|| default_model(&provider).to_string());
    let run_id = RunId::generate();
    let mimir_root = camino::Utf8PathBuf::from(".mimir");
    let run_dir = RunDir::create(&mimir_root, &run_id)?;
    if let Some(recipe) = &command_recipe {
        write_redacted_json_artifact(
            &run_dir.root().join("command_recipe.json"),
            recipe,
            ArtifactSizeClass::ProviderRequest,
        )?;
    }
    let packet = build_packet(
        &task,
        "code",
        run_id.clone(),
        &provider,
        &model,
        options.editable.clone(),
    )?;
    let packet_json = serde_json::to_vec_pretty(&packet)?;
    atomic_write(&run_dir.context_packet_path(), &packet_json)?;

    let request = code_request_from_packet(
        &packet,
        &options.editable,
        options.max_repair_turns,
        options.cost_cap,
        command_recipe
            .as_ref()
            .map(|recipe| recipe.output_budget_tokens),
    )?;
    let estimated_initial_cost = estimate_repair_request_cost(&provider, &model, &request);
    if let Some(reason) = initial_cost_preflight_rejection(estimated_initial_cost, options.cost_cap)
    {
        let reason = redacted_message(reason);
        let report = PatchReport {
            schema_version: 1,
            run_id: run_id.to_string(),
            packet_id: packet.packet_id.clone(),
            plan_id: "provider-call-not-started".to_string(),
            dry_run: options.dry_run,
            applied: false,
            affected_files: Vec::new(),
            tests_to_run: Vec::new(),
            max_repair_turns: options.max_repair_turns,
            cost_cap: options.cost_cap,
            test_policy: "not_run_cost_cap".to_string(),
            test_result_path: None,
            test_passed: None,
            test_exit_code: None,
            repair: None,
            notes: None,
            rejected: Some(reason.clone()),
        };
        write_patch_report(&run_dir, &report)?;
        append_redacted_event(
            &run_dir,
            &serde_json::json!({
                "event_type": "cost_cap_aborted",
                "timestamp": chrono::Utc::now().to_rfc3339(),
                "estimated_cost": estimated_initial_cost,
                "cost_cap": options.cost_cap,
                "reason": &reason,
            }),
        )?;
        if options.json {
            println!("{}", redacted_json_string(&report)?);
        }
        bail!("{reason}");
    }
    write_provider_request_artifact(&run_dir, &request)?;
    let response =
        call_provider_with_request(&packet, request, false, false, options.base_url.clone())?;
    write_provider_artifacts(&run_dir, &response)?;
    let initial_cost = estimate_provider_cost(&provider, &response.model, &response.usage);
    if let Some(reason) = provider_response_cost_rejection(initial_cost, options.cost_cap) {
        let reason = redacted_message(reason);
        let report = PatchReport {
            schema_version: 1,
            run_id: run_id.to_string(),
            packet_id: packet.packet_id.clone(),
            plan_id: "provider-response-cost-cap".to_string(),
            dry_run: options.dry_run,
            applied: false,
            affected_files: Vec::new(),
            tests_to_run: Vec::new(),
            max_repair_turns: options.max_repair_turns,
            cost_cap: options.cost_cap,
            test_policy: "not_run_cost_cap".to_string(),
            test_result_path: None,
            test_passed: None,
            test_exit_code: None,
            repair: None,
            notes: None,
            rejected: Some(reason.clone()),
        };
        write_patch_report(&run_dir, &report)?;
        append_redacted_event(
            &run_dir,
            &serde_json::json!({
                "event_type": "cost_cap_aborted",
                "timestamp": chrono::Utc::now().to_rfc3339(),
                "actual_cost": initial_cost,
                "cost_cap": options.cost_cap,
                "reason": &reason,
            }),
        )?;
        if options.json {
            println!("{}", redacted_json_string(&report)?);
        }
        bail!("{reason}");
    }

    let text = response_text(&response);
    let (patch_plan, patch_recipe, notes) =
        match parse_patch_plan_response(&text, &packet, &options.editable) {
            Ok(parsed) => parsed,
            Err(error) => {
                let reason = redacted_message(error);
                let report = PatchReport {
                    schema_version: 1,
                    run_id: run_id.to_string(),
                    packet_id: packet.packet_id.clone(),
                    plan_id: "unparsed-provider-response".to_string(),
                    dry_run: options.dry_run,
                    applied: false,
                    affected_files: Vec::new(),
                    tests_to_run: Vec::new(),
                    max_repair_turns: options.max_repair_turns,
                    cost_cap: options.cost_cap,
                    test_policy: "not_run".to_string(),
                    test_result_path: None,
                    test_passed: None,
                    test_exit_code: None,
                    repair: None,
                    notes: None,
                    rejected: Some(reason.clone()),
                };
                write_patch_report(&run_dir, &report)?;
                bail!("patch rejected by provider response validation: {reason}");
            }
        };
    let tests_to_run = patch_plan.tests_to_run.clone();
    write_redacted_json_artifact(
        &run_dir.root().join("patch_plan.json"),
        &patch_plan,
        ArtifactSizeClass::PatchPlan,
    )?;
    write_redacted_json_artifact(
        &run_dir.root().join("patch_recipe.json"),
        &patch_recipe,
        ArtifactSizeClass::PatchRecipe,
    )?;
    let patch_text = render_patch_artifact(&patch_recipe);
    let patch_artifact = mimir_security::redact_secrets(&patch_text);
    write_artifact_bytes(
        &run_dir.root().join("patch.diff"),
        patch_artifact.as_bytes(),
        ArtifactSizeClass::PatchDiff,
    )?;

    let files = affected_files(&patch_recipe);
    if contains_secret_like_text(&patch_text) {
        let reason = "provider patch contained secret-like text and was not applied".to_string();
        reject_patch_plan(
            &run_dir,
            &packet,
            &patch_plan,
            &options,
            PatchRejection {
                affected_files: files,
                tests_to_run,
                notes,
                reason: reason.clone(),
            },
        )?;
        bail!("patch rejected by safety validation: {reason}");
    }

    if let Err(error) =
        validate_patch_plan_binding(&patch_plan, &patch_recipe, &packet, &options.editable)
    {
        let reason = redacted_message(error);
        reject_patch_plan(
            &run_dir,
            &packet,
            &patch_plan,
            &options,
            PatchRejection {
                affected_files: files,
                tests_to_run,
                notes,
                reason: reason.clone(),
            },
        )?;
        bail!("patch rejected by safety validation: {reason}");
    }

    let editable_set = EditableSet::from_paths(options.editable.clone());
    let base = camino::Utf8Path::new(".");
    let mut run_owned_files = BTreeSet::new();
    let mut run_owned_hashes = BTreeMap::new();
    let apply_result =
        validate_patch_plan_dry_run(&patch_recipe, base, &options.editable, &run_id.to_string())
            .and_then(|()| {
                if options.dry_run {
                    Ok(())
                } else {
                    apply_patch_plan_with_guards(
                        &patch_recipe,
                        base,
                        &editable_set,
                        &run_id,
                        "initial",
                        &files,
                        &BTreeSet::new(),
                    )
                }
            });

    if let Err(error) = apply_result {
        let reason = redacted_message(error);
        reject_patch_plan(
            &run_dir,
            &packet,
            &patch_plan,
            &options,
            PatchRejection {
                affected_files: files,
                tests_to_run,
                notes,
                reason: reason.clone(),
            },
        )?;
        bail!("patch rejected by safety validation: {reason}");
    }
    if !options.dry_run {
        run_owned_files.extend(files.iter().cloned());
        run_owned_hashes = snapshot_file_hashes(base, &run_owned_files)?;
    }

    let initial_test = run_code_tests_named(
        &run_dir,
        base,
        &options,
        &tests_to_run,
        &[],
        "test_result.json",
    )?;
    let mut test_policy = initial_test.policy;
    let mut test_result_path = initial_test.path;
    let mut test_result = initial_test.result;
    let mut test_failure = test_failure_reason(test_result.as_ref());
    let mut repair_summary = None;

    if test_failure.is_some() && !options.no_test && !options.dry_run {
        if let Some(failed_test) = test_result.clone() {
            let repair = run_code_repair_loop(
                RepairRunContext {
                    run_dir: &run_dir,
                    base,
                    options: &options,
                    packet: &packet,
                    editable_set: &editable_set,
                    suggested_commands: &tests_to_run,
                    initial_spend: initial_cost,
                },
                failed_test,
                &mut run_owned_files,
                &mut run_owned_hashes,
            )?;
            test_policy = repair.test_policy;
            test_result_path = repair.test_result_path;
            test_result = repair.test_result;
            test_failure = repair.test_failure;
            write_redacted_json_artifact(
                &run_dir.root().join("repair_summary.json"),
                &repair.summary,
                ArtifactSizeClass::Repair,
            )?;
            repair_summary = Some(repair.summary);
        }
    }

    let report = PatchReport {
        schema_version: 1,
        run_id: run_id.to_string(),
        packet_id: packet.packet_id.clone(),
        plan_id: patch_plan.plan_id.clone(),
        dry_run: options.dry_run,
        applied: !options.dry_run,
        affected_files: files.clone(),
        tests_to_run: tests_to_run.clone(),
        max_repair_turns: options.max_repair_turns,
        cost_cap: options.cost_cap,
        test_policy,
        test_result_path,
        test_passed: test_result.as_ref().map(|result| result.passed),
        test_exit_code: test_result.as_ref().map(|result| result.exit_code),
        repair: repair_summary,
        notes,
        rejected: test_failure.clone(),
    };
    write_patch_report(&run_dir, &report)?;
    append_redacted_event(
        &run_dir,
        &serde_json::json!({
            "event_type": if options.dry_run { "patch_dry_run" } else { "patch_applied" },
            "timestamp": chrono::Utc::now().to_rfc3339(),
            "affected_files": files,
        }),
    )?;

    if let Some(reason) = &test_failure {
        append_redacted_event(
            &run_dir,
            &serde_json::json!({
                "event_type": "patch_tests_failed",
                "timestamp": chrono::Utc::now().to_rfc3339(),
                "reason": reason,
            }),
        )?;
    }

    if options.json {
        println!("{}", redacted_json_string(&report)?);
    } else {
        println!("Run ID: {run_id}");
        println!("Patch: {}", run_dir.root().join("patch.diff"));
        println!("Patch report: {}", run_dir.root().join("patch_report.json"));
        println!(
            "Status: {}",
            if options.dry_run {
                "validated dry run"
            } else {
                "applied"
            }
        );
        println!("Affected files: {}", report.affected_files.join(", "));
        if !report.tests_to_run.is_empty() {
            let suggested = report
                .tests_to_run
                .iter()
                .map(redacted_message)
                .collect::<Vec<_>>()
                .join("; ");
            println!("Suggested tests: {suggested}");
        }
    }

    if let Some(reason) = test_failure {
        bail!("{reason}");
    }

    Ok(())
}

fn redact_value(mut value: serde_json::Value) -> serde_json::Value {
    mimir_security::redact_json_value(&mut value);
    value
}

fn run_eval_context_command(
    dataset: String,
    cap_tokens: u32,
    output: Option<String>,
    json: bool,
) -> Result<()> {
    let run = mimir_eval::run_context_dataset(&dataset, cap_tokens)?;
    let results_json = serde_json::to_vec_pretty(&run.results)?;
    let output_path = if let Some(path) = output {
        PathBuf::from(path)
    } else {
        let stamp = chrono::Utc::now().format("%Y%m%d-%H%M%S").to_string();
        PathBuf::from(".mimir")
            .join("evals")
            .join(format!("{}-{stamp}", run.summary.dataset_id))
            .join("eval_results.json")
    };
    if let Some(parent) = output_path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&output_path, &results_json)?;

    let summary_path = output_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join("summary.json");
    fs::write(&summary_path, serde_json::to_vec_pretty(&run.summary)?)?;

    if json {
        let mut stdout = std::io::stdout().lock();
        stdout.write_all(&results_json)?;
        stdout.write_all(b"\n")?;
    } else {
        println!("Context eval dataset: {}", run.summary.dataset_id);
        println!("Cases: {}", run.summary.case_count);
        println!(
            "Passed: {}/{}",
            run.summary.passed_count, run.summary.case_count
        );
        println!("Mean file recall: {:.3}", run.summary.mean_file_recall);
        println!("Mean precision: {:.3}", run.summary.mean_precision);
        println!(
            "Mode 4 beats mode 0: {}",
            run.summary.mode4_outperforms_mode0
        );
        println!("Cap compliance: {}", run.summary.cap_compliance);
        println!("Input tokens: {}", run.summary.tokens_in_total);
        println!("Results: {}", output_path.display());
        println!("Summary: {}", summary_path.display());
    }

    if !run.summary.cap_compliance {
        bail!(
            "context eval failed: {}/{} cases passed; cap_compliance={}",
            run.summary.passed_count,
            run.summary.case_count,
            run.summary.cap_compliance
        );
    }

    Ok(())
}

fn run_doctor_command() -> Result<()> {
    println!("Mimir doctor");
    println!("  Version: ok ({})", env!("CARGO_PKG_VERSION"));
    println!("  RunId: ok ({})", RunId::generate());

    let mut warnings = 0u32;
    let mut failures = 0u32;

    let config_path = std::path::Path::new(".mimir/config.yaml");
    if config_path.is_file() {
        match std::fs::read_to_string(config_path)
            .map_err(anyhow::Error::from)
            .and_then(|content| {
                serde_yaml::from_str::<serde_yaml::Value>(&content).map_err(anyhow::Error::from)
            }) {
            Ok(_) => println!("  Config: ok (.mimir/config.yaml)"),
            Err(err) => {
                failures += 1;
                println!("  Config: fail ({err})");
            }
        }
    } else {
        warnings += 1;
        println!("  Config: missing (run `mimir init`)");
    }

    match mimir_providers::capabilities::provider_capabilities_list().map_err(anyhow::Error::msg) {
        Ok(list) if !list.providers.is_empty() => {
            println!(
                "  Provider capabilities: ok ({} provider(s))",
                list.providers.len()
            );
        }
        Ok(_) => {
            failures += 1;
            println!("  Provider capabilities: fail (registry is empty)");
        }
        Err(err) => {
            failures += 1;
            println!("  Provider capabilities: fail ({err})");
        }
    }

    let token_count = mimir_providers::count::count_local("mimir doctor token counter sanity");
    if token_count > 0 {
        println!("  Token counter: ok ({token_count} tokens)");
    } else {
        failures += 1;
        println!("  Token counter: fail (local counter returned zero)");
    }

    let run_id = RunId::generate();
    let provider = "glm";
    let model = default_model(provider);
    match build_packet(
        "Mimir doctor context sanity",
        "ask",
        run_id,
        provider,
        model,
        Vec::new(),
    ) {
        Ok(packet) if packet.packet_hash.len() == 64 => {
            println!("  Context packet: ok ({})", packet.packet_id);
        }
        Ok(packet) => {
            failures += 1;
            println!(
                "  Context packet: fail (unexpected hash length {})",
                packet.packet_hash.len()
            );
        }
        Err(err) => {
            failures += 1;
            println!("  Context packet: fail ({err})");
        }
    }

    let permission_dir = if std::path::Path::new(".mimir").is_dir() {
        std::path::Path::new(".mimir")
    } else {
        std::path::Path::new(".")
    };
    let probe_path = permission_dir.join(format!(".doctor-write-{}", RunId::generate()));
    match std::fs::write(&probe_path, b"ok").and_then(|_| std::fs::remove_file(&probe_path)) {
        Ok(_) => println!("  Permissions: ok ({})", permission_dir.display()),
        Err(err) => {
            failures += 1;
            let _ = std::fs::remove_file(&probe_path);
            println!("  Permissions: fail ({err})");
        }
    }

    let detected_credentials = [
        ("ANTHROPIC_API_KEY", "anthropic"),
        ("GLM_API_KEY", "glm"),
        ("ZAI_API_KEY", "glm"),
        ("OPENAI_API_KEY", "openai"),
    ]
    .into_iter()
    .filter_map(|(env_var, provider)| {
        std::env::var(env_var)
            .ok()
            .filter(|value| !value.trim().is_empty())
            .map(|_| provider)
    })
    .collect::<std::collections::BTreeSet<_>>();
    if detected_credentials.is_empty() {
        println!("  Provider credentials: optional (none detected)");
    } else {
        println!(
            "  Provider credentials: ok ({})",
            detected_credentials
                .into_iter()
                .collect::<Vec<_>>()
                .join(", ")
        );
    }

    if failures > 0 {
        println!("Doctor status: failed");
        bail!("mimir doctor found {failures} failure(s) and {warnings} warning(s)");
    }
    if warnings > 0 {
        println!("Doctor status: warnings");
    } else {
        println!("Doctor status: ok");
    }
    Ok(())
}

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .init();
    let cli = Cli::parse();

    match cli.command {
        Commands::Init { path } => {
            let dir = path.unwrap_or_else(|| ".".to_string());
            info!("Initializing Mimir in {}", dir);
            let mimir_root = camino::Utf8PathBuf::from(format!("{}/.mimir", dir));
            let _ = RunDir::create(&mimir_root, &RunId::generate());
            let created = init_project_files(&dir)?;
            println!("Initialized .mimir/ in {}", dir);
            if !created.is_empty() {
                println!("Created workflow files:");
                for path in created {
                    println!("  {}", path);
                }
            }
        }
        Commands::Doctor => {
            run_doctor_command()?;
        }
        Commands::Version => {
            println!("mimir {}", env!("CARGO_PKG_VERSION"));
        }
        Commands::Providers { cmd } => match cmd {
            ProvidersCmd::Calibrate { model, provider } => {
                println!("Calibrating token counter for {}/{}...", provider, model);
                println!("  Local count: run `mimir context build --provider {} --model {}` to snapshot a packet", provider, model);
                println!("  Server count: supported only by adapters that expose a count endpoint");
            }
            ProvidersCmd::Doctor => {
                let list = mimir_providers::capabilities::provider_capabilities_list()
                    .map_err(anyhow::Error::msg)?;
                println!("Provider capabilities: ok");
                for provider in list.providers {
                    let snapshot = current_capability_snapshot_ref(
                        &provider.provider,
                        provider
                            .models
                            .keys()
                            .min()
                            .map(String::as_str)
                            .unwrap_or(default_model(&provider.provider)),
                    )
                    .unwrap_or_else(|_| "unknown".to_string());
                    println!(
                        "  {}: {} model(s), snapshot {}",
                        provider.provider,
                        provider.models.len(),
                        snapshot
                    );
                    let mut models = provider.models.keys().collect::<Vec<_>>();
                    models.sort();
                    for model in models {
                        let caps = provider.models.get(model).expect("model key present");
                        println!(
                            "    {}: input {}, output reserve {}, max output {}",
                            model,
                            caps.max_input_tokens,
                            caps.output_reserve_tokens,
                            caps.max_output_tokens
                        );
                    }
                }
            }
        },
        Commands::Context { cmd } => match cmd {
            ContextCmd::Build { provider, model } => {
                let provider = normalized_provider(&provider);
                let model = model.unwrap_or_else(|| default_model(&provider).to_string());
                let run_id = RunId::generate();
                let mimir_root = camino::Utf8PathBuf::from(".mimir");
                let run_dir = RunDir::create(&mimir_root, &run_id)?;
                let packet = build_packet(
                    "Build a replayable context packet for the current repository",
                    "ask",
                    run_id.clone(),
                    &provider,
                    &model,
                    Vec::new(),
                )?;
                let packet_json = serde_json::to_vec_pretty(&packet)?;
                atomic_write(&run_dir.context_packet_path(), &packet_json)?;
                println!("Built packet: {}", packet.packet_id);
                println!("Run ID: {}", run_id);
                println!("Packet path: {}", run_dir.context_packet_path());
                println!("Estimated input tokens: {}", packet.estimated_input_tokens);
            }
            ContextCmd::Inspect { path } => {
                let data = std::fs::read_to_string(&path)?;
                let packet: mimir_schemas::ContextPacket = serde_json::from_str(&data)?;
                println!(
                    "Packet {}: {} tokens, hash {}",
                    packet.packet_id, packet.estimated_input_tokens, packet.packet_hash
                );
            }
            ContextCmd::Budget => {
                println!("Token budget: 64000 cap, 4096 output reserve, 512 drift reserve");
            }
            ContextCmd::Omitted => {
                println!("Omitted items are recorded per run in context_packet.json");
            }
            ContextCmd::Why { path, run_id } => match mimir_context::context_why(&path, &run_id) {
                Ok(mimir_context::WhyResult::Included {
                    reason_code,
                    token_count,
                }) => println!(
                    "Included: {} (reason: {}, tokens: {})",
                    path, reason_code, token_count
                ),
                Ok(mimir_context::WhyResult::Omitted {
                    reason,
                    token_count,
                }) => {
                    println!(
                        "Omitted: {} (reason: {}, tokens: {})",
                        path, reason, token_count
                    );
                }
                Ok(mimir_context::WhyResult::NotFound) => println!("not found"),
                Err(error) => println!("Error: {}", error),
            },
            ContextCmd::Suggest {
                task,
                provider,
                model,
                json,
            } => run_context_suggest_command(task, provider, model, json)?,
            ContextCmd::Call {
                path,
                base_url,
                stream,
                cache,
            } => {
                let data = std::fs::read_to_string(&path)?;
                let packet: mimir_schemas::ContextPacket = serde_json::from_str(&data)?;
                require_packet_path_run_id(&packet, Path::new(&path))?;
                let request = request_from_packet(&packet, stream)?;
                let run_dir = RunDir::open(
                    &camino::Utf8PathBuf::from(".mimir"),
                    &RunId(packet.run_id.clone()),
                )
                .ok();
                if let Some(run_dir) = &run_dir {
                    write_provider_request_artifact(run_dir, &request)?;
                }
                let response =
                    call_provider_with_request(&packet, request, stream, cache, base_url)?;
                if let Some(run_dir) = &run_dir {
                    write_provider_artifacts(run_dir, &response)?;
                }
                println!("{}", response_text(&response));
            }
        },
        Commands::Plan {
            task,
            editable,
            provider,
            model,
            base_url,
            json,
        } => run_plan_command(task, editable, provider, model, base_url, json)?,
        Commands::Code {
            task,
            editable,
            recipe,
            params,
            max_repair_turns,
            cost_cap,
            no_test,
            provider,
            model,
            base_url,
            dry_run,
            json,
        } => run_code_command(CodeCommandOptions {
            task,
            editable,
            recipe,
            params,
            max_repair_turns,
            cost_cap,
            no_test,
            provider,
            model,
            base_url,
            dry_run,
            json,
        })?,
        Commands::Review {
            since,
            committee,
            checks,
        } => {
            let since_ref = since.as_deref().unwrap_or("HEAD~1");
            println!("Reviewing changes since {}", since_ref);
            let base = camino::Utf8Path::new(".");
            match mimir_review::diff::diff_since(base, since_ref) {
                Ok(diffs) => {
                    println!("Found {} changed files", diffs.len());
                    let uninspected = mimir_review::diff::detect_uninspected(
                        &diffs,
                        &std::collections::HashSet::new(),
                    );
                    if !uninspected.is_empty() {
                        println!("  Uninspected files: {}", uninspected.len());
                    }
                    let generated = mimir_review::diff::detect_generated_edits(&diffs);
                    if !generated.is_empty() {
                        println!("  Generated file edits: {}", generated.len());
                        for finding in &generated {
                            println!("    ERROR: {}", finding.description);
                        }
                    }
                    if committee {
                        println!("  Committee review mode enabled");
                    }
                    if checks {
                        let checks = mimir_review::checks::load_checks(base);
                        println!("  Loaded {} source-controlled checks", checks.len());
                        let findings = mimir_review::checks::run_checks(&checks, base);
                        if !findings.is_empty() {
                            println!("  Check failures: {}", findings.len());
                        }
                    }
                }
                Err(error) => println!("Error: {}", error),
            }
        }
        Commands::Check { ci, json } => run_check_command(ci, json)?,
        Commands::Explore { query, json } => run_explore_command(query, json)?,
        Commands::Agent { name, query, list } => {
            if list {
                let registry = mimir_subagents::subagents::SubagentRegistry::new();
                println!("Available subagents:");
                for agent in registry.list() {
                    println!(
                        "  {:20} | {:10} | {}",
                        agent.name, agent.cost_tier, agent.description
                    );
                }
            } else {
                println!("Running subagent '{}' with query: {}", name, query);
                let evidence = mimir_subagents::subagents::execute(&name, &query, None)?;
                println!("Subagent: {}", evidence.subagent);
                println!("Findings: {}", evidence.findings.join("\n  - "));
                println!("Paths: {:?}", evidence.relevant_paths);
                println!("Confidence: {:.0}%", evidence.confidence * 100.0);
                println!("Cost: ${:.4}", evidence.cost_usd);
            }
        }
        Commands::Memory { cmd } => match cmd {
            MemoryCmd::List {
                kind,
                scope,
                confidence,
            } => {
                let store =
                    mimir_memory::MemoryStore::open(".mimir/memory.db").unwrap_or_else(|e| {
                        println!("Memory DB not initialized: {}", e);
                        std::process::exit(1);
                    });
                let entries =
                    store.list(kind.as_deref(), scope.as_deref(), confidence.as_deref())?;
                println!("{} memory entries:", entries.len());
                for entry in entries {
                    println!(
                        "  {} | {} | {} | {}",
                        entry.entry_id, entry.kind, entry.scope, entry.confidence
                    );
                }
            }
            MemoryCmd::Show { entry_id } => {
                let store =
                    mimir_memory::MemoryStore::open(".mimir/memory.db").unwrap_or_else(|e| {
                        println!("Memory DB not initialized: {}", e);
                        std::process::exit(1);
                    });
                match store.get(&entry_id)? {
                    Some(entry) => {
                        println!("Entry: {}", entry.entry_id);
                        println!("  Kind: {}", entry.kind);
                        println!("  Scope: {}", entry.scope);
                        println!("  Confidence: {}", entry.confidence);
                        println!("  Safe to send: {}", entry.safe_to_send);
                        println!("  Created: {}", entry.created_at);
                        println!("  Tags: {}", entry.retrieval_tags.join(", "));
                        println!("  Body:\n{}", entry.body);
                    }
                    None => println!("Entry not found: {}", entry_id),
                }
            }
            MemoryCmd::Why { entry_id } => {
                let store =
                    mimir_memory::MemoryStore::open(".mimir/memory.db").unwrap_or_else(|e| {
                        println!("Memory DB not initialized: {}", e);
                        std::process::exit(1);
                    });
                match store.get(&entry_id)? {
                    Some(entry) => {
                        let engine = mimir_memory::MemoryDecisionEngine::default();
                        let signals = mimir_memory::ScoreSignals::default();
                        let breakdown = engine.score(&signals);
                        println!("Entry: {}", entry.entry_id);
                        println!("  Promotion score: {:.2}", breakdown.total);
                        println!("  Breakdown: {:?}", breakdown);
                    }
                    None => println!("Entry not found: {}", entry_id),
                }
            }
            MemoryCmd::Search { query } => {
                let store =
                    mimir_memory::MemoryStore::open(".mimir/memory.db").unwrap_or_else(|e| {
                        println!("Memory DB not initialized: {}", e);
                        std::process::exit(1);
                    });
                let entries = store.search(&query)?;
                println!("{} results for '{}'", entries.len(), query);
                for entry in entries {
                    println!("  {} | {} | {}", entry.entry_id, entry.kind, entry.scope);
                }
            }
            MemoryCmd::Publish => {
                let store =
                    mimir_memory::MemoryStore::open(".mimir/memory.db").unwrap_or_else(|e| {
                        println!("Memory DB not initialized: {}", e);
                        std::process::exit(1);
                    });
                let entries = store.list(None, None, Some("validated"))?;
                let safe: Vec<_> = entries
                    .into_iter()
                    .filter(|entry| entry.safe_to_send)
                    .collect();
                mimir_memory::publish(&safe, ".mimir/project-rules.md")?;
                println!(
                    "Published {} entries to .mimir/project-rules.md",
                    safe.len()
                );
            }
            MemoryCmd::Forget { entry_id } => {
                let store =
                    mimir_memory::MemoryStore::open(".mimir/memory.db").unwrap_or_else(|e| {
                        println!("Memory DB not initialized: {}", e);
                        std::process::exit(1);
                    });
                store.delete(&entry_id)?;
                println!("Forgot entry: {}", entry_id);
            }
            MemoryCmd::ImportSessions {
                from,
                paths,
                discover,
                dry_run,
                max_files,
                include_archived,
            } => {
                let importer = mimir_memory::importer_for(&from).ok_or_else(|| {
                    anyhow!(
                        "unsupported session source '{}'; expected one of: aider, claude-code, codex, opencode",
                        from
                    )
                })?;

                if paths.is_empty() && !discover {
                    bail!("mimir memory import-sessions requires PATH arguments or --discover");
                }

                let mut import_paths = paths.into_iter().map(PathBuf::from).collect::<Vec<_>>();
                if discover {
                    let mut roots = mimir_memory::DiscoveryRoots::from_env();
                    roots.max_files = max_files;
                    roots.include_archived = include_archived;
                    let discovered = mimir_memory::discover_sessions(&from, &roots)?;
                    for session in discovered {
                        import_paths.push(session.path);
                    }
                }
                import_paths.sort();
                import_paths.dedup();

                if dry_run {
                    println!(
                        "Discovered {} session file(s) from {}",
                        import_paths.len(),
                        from
                    );
                    for path in import_paths {
                        println!("  {}", path.display());
                    }
                    return Ok(());
                }

                std::fs::create_dir_all(".mimir")?;
                let store = mimir_memory::MemoryStore::open(".mimir/memory.db")?;
                let mut imported = 0usize;

                for path in import_paths {
                    let path_display = path.display().to_string();
                    let entries = importer.import(&path_display)?;
                    let count = entries.len();
                    for entry in entries {
                        store.insert(&entry)?;
                    }
                    println!("Imported {} entries from {}", count, path_display);
                    imported += count;
                }

                println!(
                    "Imported {} total session entries from {} into .mimir/memory.db",
                    imported, from
                );
            }
        },
        Commands::Tui {
            packet,
            pipeline_result,
            server,
            task,
            provider,
            model,
            session_id,
            refresh_ms,
        } => {
            println!("Starting Mimir TUI...");
            let mut app = mimir_tui::App::new();
            if let Some(path) = packet {
                app.load_from_file(&path)?;
            }
            if let Some(path) = pipeline_result {
                app.load_pipeline_from_file(&path)?;
            }
            if let Some(address) = server {
                let task = task.ok_or_else(|| {
                    anyhow!("mimir tui --server requires --task for live refresh")
                })?;
                if provider.is_some() != model.is_some() {
                    bail!("mimir tui --server requires --provider and --model to be set together");
                }
                let mut config = mimir_tui::server_client::LiveServerConfig::new(address, task);
                config.provider = provider;
                config.model = model;
                config.session_id = session_id;
                config.refresh_interval = refresh_ms.map(std::time::Duration::from_millis);
                config.workspace_root = Some(std::env::current_dir()?);
                app.set_live_server(config);
                app.request_live_refresh();
            }
            mimir_tui::run_tui(&mut app)?;
        }
        Commands::Serve { port, rpc_stdio } => {
            let rt = tokio::runtime::Runtime::new()?;
            let config = mimir_server::ServerConfig {
                tcp_bind: port.map(|p| format!("127.0.0.1:{}", p)),
                stdio: rpc_stdio,
            };
            rt.block_on(mimir_server::run_server(config))?;
        }
        Commands::Ask {
            question,
            provider,
            model,
            base_url,
            json,
        } => {
            let provider = normalized_provider(&provider);
            let model = model.unwrap_or_else(|| default_model(&provider).to_string());
            let run_id = RunId::generate();
            let mimir_root = camino::Utf8PathBuf::from(".mimir");
            let run_dir = RunDir::create(&mimir_root, &run_id)?;
            let packet = build_packet(
                &question,
                "ask",
                run_id.clone(),
                &provider,
                &model,
                Vec::new(),
            )?;
            let packet_json = serde_json::to_vec_pretty(&packet)?;
            atomic_write(&run_dir.context_packet_path(), &packet_json)?;
            let request = request_from_packet(&packet, false)?;
            write_provider_request_artifact(&run_dir, &request)?;
            let response = call_provider_with_request(&packet, request, false, false, base_url)?;
            write_provider_artifacts(&run_dir, &response)?;
            if json {
                let output = serde_json::json!({
                    "run_id": run_id.to_string(),
                    "packet_id": packet.packet_id,
                    "packet_hash": packet.packet_hash,
                    "tokens": packet.estimated_input_tokens,
                    "response": response_text(&response),
                    "usage": response.usage,
                    "model": response.model,
                });
                println!("{}", redacted_json_string(&output)?);
            } else {
                println!("Run ID: {}", run_id);
                println!("Packet: {} tokens", packet.estimated_input_tokens);
                println!("{}", response_text(&response));
            }
        }
        Commands::Packet { cmd } => match cmd {
            PacketCmd::Share {
                run_id,
                output,
                packet_only,
            } => {
                let mimir_root = camino::Utf8PathBuf::from(".mimir");
                let run_dir = match RunDir::open(&mimir_root, &RunId(run_id.clone())) {
                    Ok(run_dir) => run_dir,
                    Err(_) => {
                        println!("No packet found for run {}", run_id);
                        std::process::exit(1);
                    }
                };
                let packet_path = run_dir.context_packet_path();
                if !std::path::Path::new(&packet_path).exists() {
                    println!("No packet found for run {}", run_id);
                    std::process::exit(1);
                }
                let data = std::fs::read_to_string(&packet_path)?;
                let packet: mimir_schemas::ContextPacket = serde_json::from_str(&data)?;
                verify_packet_integrity(&packet)?;
                verify_packet_run_id(&packet, &run_id)?;
                verify_included_source_hashes(&packet)?;
                if packet_only {
                    let sanitized = redacted_pretty_json_bytes(&packet)?;
                    write_bytes_to_output(output, &sanitized, "Sanitized packet")?;
                } else {
                    let provider_request_path =
                        run_dir.root().join("provider_request.redacted.json");
                    let replay = if std::path::Path::new(&provider_request_path).exists() {
                        shared_replay_from_provider_request_artifact(&provider_request_path)?
                    } else {
                        let request = request_from_packet(&packet, false)?;
                        shared_replay_from_request(&request)?
                    };
                    let bundle = build_shared_packet_bundle(&packet, &run_id, replay)?;
                    let bundle_json = redacted_pretty_json_bytes(&bundle)?;
                    write_bytes_to_output(output, &bundle_json, "Shared packet bundle")?;
                }
            }
            PacketCmd::Replay {
                target,
                request_json,
            } => {
                let target_path = std::path::Path::new(&target);
                if target_path.exists() || looks_like_path(&target) {
                    let bundle = read_shared_packet_bundle(target_path)?;
                    if request_json {
                        let bytes = shared_bundle_request_bytes(&bundle)?;
                        let mut stdout = std::io::stdout().lock();
                        stdout.write_all(&bytes)?;
                    } else {
                        println!("Replaying shared packet {}", bundle.packet.packet_id);
                        println!("Run ID: {}", bundle.run_id);
                        println!("Task: {}", bundle.packet.task_card.goal);
                        println!("Included items: {}", bundle.packet.included.len());
                        println!("Estimated tokens: {}", bundle.packet.estimated_input_tokens);
                        println!(
                            "Provider request sha256: {}",
                            bundle.replay.provider_request_sha256
                        );
                        if let Some(prompt_sha256) = bundle.replay.user_prompt_sha256.as_deref() {
                            println!("User prompt sha256: {}", prompt_sha256);
                        }
                        println!(
                            "To inspect the replay request: mimir packet replay {} --request-json",
                            target
                        );
                    }
                    return Ok(());
                }
                let run_id = target;
                let mimir_root = camino::Utf8PathBuf::from(".mimir");
                let run_dir = match RunDir::open(&mimir_root, &RunId(run_id.clone())) {
                    Ok(run_dir) => run_dir,
                    Err(_) => {
                        println!("No packet found for run {}", run_id);
                        std::process::exit(1);
                    }
                };
                let packet_path = run_dir.context_packet_path();
                if !std::path::Path::new(&packet_path).exists() {
                    println!("No packet found for run {}", run_id);
                    std::process::exit(1);
                }
                let data = std::fs::read_to_string(&packet_path)?;
                let packet: mimir_schemas::ContextPacket = serde_json::from_str(&data)?;
                verify_packet_run_id(&packet, &run_id)?;
                verify_packet_replay_preconditions(&packet)?;
                verify_included_source_hashes(&packet)?;
                if request_json {
                    let provider_request_path =
                        run_dir.root().join("provider_request.redacted.json");
                    let request_json = if std::path::Path::new(&provider_request_path).exists() {
                        redacted_provider_request_artifact_bytes(&provider_request_path)?
                    } else {
                        let request = request_from_packet(&packet, false)?;
                        redacted_pretty_json_bytes(&request)?
                    };
                    let mut stdout = std::io::stdout().lock();
                    stdout.write_all(&request_json)?;
                    return Ok(());
                }
                println!("Replaying packet {}", packet.packet_id);
                println!("Task: {}", packet.task_card.goal);
                println!("Included items: {}", packet.included.len());
                println!("Estimated tokens: {}", packet.estimated_input_tokens);
                println!(
                    "To replay provider dispatch: mimir context call .mimir/runs/{}/context_packet.json",
                    run_id
                );
            }
        },
        Commands::Override { cmd } => match cmd {
            OverrideCmd::Request {
                cap,
                reason,
                auto_grant_after,
            } => {
                let run_id = RunId::generate();
                let req = mimir_schemas::OverrideRequest {
                    schema_version: 1,
                    request_id: RunId::generate().to_string(),
                    run_id: run_id.to_string(),
                    reason: format!(
                        "{} (cap: {}, auto_grant: {})",
                        reason, cap, auto_grant_after
                    ),
                    requested_by: "cli".to_string(),
                };
                let mimir_root = camino::Utf8PathBuf::from(".mimir");
                let run_dir = RunDir::create(&mimir_root, &run_id)?;
                let req_json = serde_json::to_vec_pretty(&req)?;
                atomic_write(&run_dir.root().join("override_request.json"), &req_json)?;
                println!("Override request recorded: {}", run_id);
                println!("  Requested cap: {} tokens", cap);
                println!("  Reason: {}", req.reason);
                println!("  Auto-grant after: {} failed attempts", auto_grant_after);
                println!("  Status: pending approval");
            }
        },
        Commands::Trace { cmd } => match cmd {
            TraceCmd::Export {
                run_id,
                redact,
                output,
            } => {
                let mimir_root = camino::Utf8PathBuf::from(".mimir");
                let run_dir = match RunDir::open(&mimir_root, &RunId(run_id.clone())) {
                    Ok(run_dir) => run_dir,
                    Err(_) => {
                        println!("No trace found for run {}", run_id);
                        std::process::exit(1);
                    }
                };
                let trace_path = run_dir.events_path();
                if !std::path::Path::new(&trace_path).exists() {
                    println!("No trace found for run {}", run_id);
                    std::process::exit(1);
                }
                let data = std::fs::read_to_string(&trace_path)?;
                let mut events: Vec<serde_json::Value> = data
                    .lines()
                    .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
                    .collect();
                if redact {
                    for event in &mut events {
                        mimir_security::redact_json_value(event);
                    }
                }
                let export = serde_json::json!({
                    "run_id": run_id,
                    "event_count": events.len(),
                    "events": events,
                });
                let export_json = serde_json::to_string_pretty(&export)?;
                if let Some(out) = output {
                    std::fs::write(&out, export_json)?;
                    println!("Trace exported to {}", out);
                } else {
                    println!("{}", export_json);
                }
            }
        },
        Commands::Eval { cmd } => match cmd {
            EvalCmd::Context {
                dataset,
                cap_tokens,
                output,
                json,
            } => run_eval_context_command(dataset, cap_tokens, output, json)?,
        },
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_max_tokens_uses_packet_reserve_without_legacy_8k_clamp() {
        let packet = ContextBuilder::new()
            .provider("anthropic")
            .model("claude-sonnet-4-6")
            .build()
            .unwrap();

        let request =
            request_from_packet_with_prompt(&packet, "system", "prompt".to_string(), 64_000);

        assert_eq!(request.max_tokens, Some(64_000));
    }

    #[test]
    fn estimate_provider_cost_uses_registry_pricing() {
        let usage = TokenUsage {
            input_tokens: 1_000_000,
            output_tokens: 1_000_000,
            cache_creation_input_tokens: None,
            cache_read_input_tokens: None,
        };

        let haiku = estimate_provider_cost("anthropic", "claude-haiku-4-5", &usage);
        let sonnet = estimate_provider_cost("anthropic", "claude-sonnet-4-6", &usage);

        assert_eq!(haiku, 6.0);
        assert_eq!(sonnet, 18.0);
    }

    #[test]
    fn oversized_json_artifacts_fail_closed_without_partial_writes_or_secret_leaks() {
        let dir = tempfile::TempDir::new().unwrap();
        let root = camino::Utf8PathBuf::from_path_buf(dir.path().to_path_buf()).unwrap();
        let leaked = "OPENAI_API_KEY=sk-12345678901234567890";

        for (name, class) in [
            (
                "provider_request.redacted.json",
                ArtifactSizeClass::ProviderRequest,
            ),
            ("response.json", ArtifactSizeClass::ProviderResponse),
            ("patch_plan.json", ArtifactSizeClass::PatchPlan),
            ("patch_recipe.json", ArtifactSizeClass::PatchRecipe),
            (
                "repair_request.redacted.turn-1.json",
                ArtifactSizeClass::Repair,
            ),
            (
                "repair_response.turn-1.json",
                ArtifactSizeClass::ProviderResponse,
            ),
            (
                "repair_patch_plan.turn-1.json",
                ArtifactSizeClass::PatchPlan,
            ),
            (
                "repair_patch_recipe.turn-1.json",
                ArtifactSizeClass::PatchRecipe,
            ),
            ("repair_summary.json", ArtifactSizeClass::Repair),
            ("test_result.json", ArtifactSizeClass::TestResult),
        ] {
            let path = root.join(name);
            let artifact = serde_json::json!({
                "payload": "x".repeat(class.max_bytes() + 1),
                "api_key": leaked,
            });

            let error = write_redacted_json_artifact(&path, &artifact, class)
                .unwrap_err()
                .to_string();

            assert!(error.contains("exceeds size cap"), "{error}");
            assert!(!error.contains(leaked), "{name} leaked secret-like content");
            assert!(!path.exists(), "{name} left a partial artifact");
        }
    }

    #[test]
    fn oversized_diff_artifacts_fail_closed_without_partial_writes_or_secret_leaks() {
        let dir = tempfile::TempDir::new().unwrap();
        let root = camino::Utf8PathBuf::from_path_buf(dir.path().to_path_buf()).unwrap();
        let leaked = "OPENAI_API_KEY=sk-12345678901234567890";

        for (name, class) in [
            ("patch.diff", ArtifactSizeClass::PatchDiff),
            ("repair_patch.turn-1.diff", ArtifactSizeClass::PatchDiff),
        ] {
            let path = root.join(name);
            let payload = format!("{leaked}\n{}", "x".repeat(class.max_bytes() + 1));

            let error = write_redacted_bytes_artifact(&path, payload.as_bytes(), class)
                .unwrap_err()
                .to_string();

            assert!(error.contains("exceeds size cap"), "{error}");
            assert!(!error.contains(leaked), "{name} leaked secret-like content");
            assert!(!path.exists(), "{name} left a partial artifact");
        }
    }
}
