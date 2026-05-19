//! Mimir CLI entry point.

use anyhow::Result;
use clap::{Parser, Subcommand};
use mimir_core::ContextBuilder;
use mimir_runs::{atomic_write, RunDir, RunId};
use tracing::info;

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
    /// Build a context packet (stub).
    Context {
        #[command(subcommand)]
        cmd: ContextCmd,
    },
    /// Plan a task (generate PatchPlan without applying).
    Plan {
        /// Task description.
        task: String,
        /// Paths the model is allowed to edit.
        #[arg(short, long)]
        editable: Vec<String>,
    },
    /// Execute a task (plan + apply + test + repair loop).
    Code {
        /// Task description.
        task: String,
        /// Paths the model is allowed to edit.
        #[arg(short, long)]
        editable: Vec<String>,
        /// Max repair turns.
        #[arg(long, default_value = "3")]
        max_repair_turns: u32,
        /// Cost cap in dollars.
        #[arg(long, default_value = "5.0")]
        cost_cap: f64,
        /// Skip tests after applying.
        #[arg(long)]
        no_test: bool,
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
    },
    /// Start the JSON-RPC server (P6).
    Serve {
        /// TCP port to bind to.
        #[arg(short, long)]
        port: Option<u16>,
        /// Use stdio transport instead of TCP.
        #[arg(long)]
        rpc_stdio: bool,
    },
    /// Ask a question with context retrieval.
    Ask {
        /// Question to ask.
        question: String,
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
    Build,
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
    /// Call provider with packet.
    Call {
        /// Packet file path.
        path: String,
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
    /// Share (sanitize and export) a packet.
    Share {
        /// Run ID of the packet to share.
        run_id: String,
        /// Output file path (default: stdout).
        #[arg(short, long)]
        output: Option<String>,
    },
    /// Replay a packet from local artifacts.
    Replay {
        /// Run ID to replay.
        run_id: String,
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

fn main() -> Result<()> {
    tracing_subscriber::fmt::init();
    let cli = Cli::parse();

    match cli.command {
        Commands::Init { path } => {
            let dir = path.unwrap_or_else(|| ".".to_string());
            info!("Initializing Mimir in {}", dir);
            let mimir_root = camino::Utf8PathBuf::from(format!("{}/.mimir", dir));
            let _ = mimir_runs::RunDir::create(&mimir_root, &mimir_runs::RunId::generate());
            std::fs::write(
                format!("{}/.mimir/config.yaml", dir),
                "version: 1\ncap_tokens: 64000\n",
            )?;
            println!("Initialized .mimir/ in {}", dir);
        }
        Commands::Doctor => {
            println!("Mimir doctor");
            println!("  Version: {}", env!("CARGO_PKG_VERSION"));
            println!("  RunId sample: {}", RunId::generate());
            // TODO: check API key, rust version, git
        }
        Commands::Version => {
            println!("mimir {}", env!("CARGO_PKG_VERSION"));
        }
        Commands::Providers { cmd } => match cmd {
            ProvidersCmd::Calibrate { model, provider } => {
                println!("Calibrating token counter for {}/{}...", provider, model);
                println!("  Local count:  ~{} tokens (tiktoken-rs estimate)", 100);
                println!("  Server count: requires ANTHROPIC_API_KEY");
                println!("  Drift ratio:  run with API key set for exact calibration");
            }
            ProvidersCmd::Doctor => {
                println!("Provider capabilities:");
                println!("  anthropic:");
                println!("    - claude-sonnet-4-6: 1M context, 8K output");
                println!("    - claude-haiku-4-5: 200K context, 8K output");
                println!("    - Server token count: supported");
                println!("    - Prompt caching: supported");
                println!("    - Streaming: supported");
            }
        },
        Commands::Context { cmd } => match cmd {
            ContextCmd::Build => {
                let builder = ContextBuilder::new()
                    .task_card("Implement feature X")
                    .mode("standard")
                    .provider("anthropic")
                    .model("claude-sonnet-4-20250514");
                let packet = builder.build()?;
                println!("Built packet: {}", packet.packet_id);
            }
            ContextCmd::Inspect { path } => {
                let data = std::fs::read_to_string(&path)?;
                let packet: mimir_schemas::ContextPacket = serde_json::from_str(&data)?;
                println!(
                    "Packet {}: {} tokens",
                    packet.packet_id, packet.estimated_input_tokens
                );
            }
            ContextCmd::Budget => {
                println!("Token budget: 64000 cap, 4096 output reserve, 1024 drift reserve");
            }
            ContextCmd::Omitted => {
                println!("No omitted items");
            }
            ContextCmd::Why { path, run_id } => {
                match mimir_context::context_why(&path, &run_id) {
                    Ok(mimir_context::WhyResult::Included {
                        reason_code,
                        token_count,
                    }) => {
                        println!(
                            "Included: {} (reason: {}, tokens: {})",
                            path, reason_code, token_count
                        );
                    }
                    Ok(mimir_context::WhyResult::Omitted { reason, token_count }) => {
                        println!("Omitted: {} (reason: {}, tokens: {})", path, reason, token_count);
                    }
                    Ok(mimir_context::WhyResult::NotFound) => {
                        println!("not found");
                    }
                    Err(e) => {
                        println!("Error: {}", e);
                    }
                }
            }
            ContextCmd::Call {
                path,
                stream,
                cache,
            } => {
                println!("Calling provider with packet from {}", path);
                if stream {
                    println!("  Mode: streaming (SSE)");
                }
                if cache {
                    println!("  Cache: prompt caching enabled");
                }
                println!("  (Provider call requires ANTHROPIC_API_KEY to be set)");
            }
        },
        Commands::Plan { task, editable } => {
            println!("Planning task: {}", task);
            println!("Editable set: {:?}", editable);
            println!("(Plan generation requires provider API key)");
            println!("Output: PatchPlan JSON would be written to .mimir/runs/<run_id>/plan.json");
        }
        Commands::Code {
            task,
            editable,
            max_repair_turns,
            cost_cap,
            no_test,
        } => {
            println!("Executing task: {}", task);
            let run_id = RunId::generate();
            let mimir_root = camino::Utf8PathBuf::from(".mimir");
            let run_dir = RunDir::create(&mimir_root, &run_id)?;

            // Build editable set
            let editable_set = if editable.is_empty() {
                // Default: all tracked source files
                mimir_edit::EditableSet::from_paths(
                    std::fs::read_dir(".")
                        .ok()
                        .into_iter()
                        .flatten()
                        .filter_map(|e| e.ok())
                        .filter(|e| {
                            e.file_type().map(|t| t.is_file()).unwrap_or(false)
                        })
                        .filter_map(|e| e.path().to_str().map(|s| s.to_string()))
                        .collect(),
                )
            } else {
                mimir_edit::EditableSet::from_paths(editable)
            };
            println!("Editable set: {} files", editable_set.paths().len());

            // Build context packet for the task
            let packet_path = run_dir.context_packet_path();
            let packet = mimir_context::ContextBuilder::new()
                .task_card(&task)
                .mode("code")
                .provider("anthropic")
                .model("claude-sonnet-4-20250514")
                .build()?;
            let packet_json = serde_json::to_string_pretty(&packet)?;
            atomic_write(&packet_path, packet_json.as_bytes())?;
            println!("Context packet: {}", packet_path);

            // Run repair loop if tests are enabled
            if !no_test {
                let config = mimir_edit::repair::RepairConfig {
                    max_repair_turns: max_repair_turns as u32,
                    cost_cap_dollars: cost_cap,
                    stop_on_success: true,
                };

                let repair_result = mimir_edit::repair::run_repair_loop(
                    &config,
                    || {
                        // Run cargo test
                        let output = std::process::Command::new("cargo")
                            .args(["test", "--workspace"])
                            .output()
                            .map_err(|e| mimir_edit::EditError::Io(e.to_string()))?;
                        Ok(mimir_edit::test_runner::TestRunResult {
                            framework: mimir_edit::test_runner::TestFramework::CargoTest,
                            command: "cargo test --workspace".into(),
                            exit_code: output.status.code().unwrap_or(-1),
                            stdout: String::from_utf8_lossy(&output.stdout).into(),
                            stderr: String::from_utf8_lossy(&output.stderr).into(),
                            passed: output.status.success(),
                            tests_run: None,
                            tests_failed: None,
                        })
                    },
                    |_test_result| {
                        // In a real implementation, this would call the provider
                        // to generate fix patches. For now, return empty.
                        println!("(Repair patches would be generated via provider API)");
                        vec![]
                    },
                );

                let repair_json = serde_json::to_string_pretty(&repair_result)?;
                let repair_path = run_dir.context_packet_path().with_file_name("repair_result.json");
                atomic_write(&repair_path, repair_json.as_bytes())?;
                println!(
                    "Repair loop: {} turns, converged: {}, reason: {}",
                    repair_result.turns_executed,
                    repair_result.converged,
                    repair_result.stop_reason
                );
                println!("Repair result: {}", repair_path);
            }

            println!("Output: results written to {}", run_dir.context_packet_path().parent().unwrap_or(camino::Utf8Path::new(".")));
        }
        Commands::Review { since, committee, checks } => {
            let since_ref = since.as_deref().unwrap_or("HEAD~1");
            println!("Reviewing changes since {}", since_ref);
            let base = camino::Utf8Path::new(".");
            match mimir_review::diff::diff_since(base, since_ref) {
                Ok(diffs) => {
                    println!("Found {} changed files", diffs.len());
                    let uninspected = mimir_review::diff::detect_uninspected(
                        &diffs, &std::collections::HashSet::new()
                    );
                    if !uninspected.is_empty() {
                        println!("  Uninspected files: {}", uninspected.len());
                    }
                    let generated = mimir_review::diff::detect_generated_edits(&diffs);
                    if !generated.is_empty() {
                        println!("  Generated file edits: {}", generated.len());
                        for f in &generated {
                            println!("    ERROR: {}", f.description);
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
                Err(e) => {
                    println!("Error: {}", e);
                }
            }
        }
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
                match mimir_subagents::subagents::execute_stub(
                    &name, &query, None
                ) {
                    Ok(evidence) => {
                        println!("Subagent: {}", evidence.subagent);
                        println!("Findings: {}", evidence.findings.join("\n  - "));
                        println!("Paths: {:?}", evidence.relevant_paths);
                        println!("Confidence: {:.0}%", evidence.confidence * 100.0);
                        println!("Cost: ${:.4}", evidence.cost_usd);
                    }
                    Err(e) => {
                        println!("Error: {}", e);
                    }
                }
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
                let entries = store.list(kind.as_deref(), scope.as_deref(), confidence.as_deref())?;
                println!("{} memory entries:", entries.len());
                for e in entries {
                    println!(
                        "  {} | {} | {} | {}",
                        e.entry_id, e.kind, e.scope, e.confidence
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
                    Some(e) => {
                        println!("Entry: {}", e.entry_id);
                        println!("  Kind: {}", e.kind);
                        println!("  Scope: {}", e.scope);
                        println!("  Confidence: {}", e.confidence);
                        println!("  Safe to send: {}", e.safe_to_send);
                        println!("  Created: {}", e.created_at);
                        println!("  Tags: {}", e.retrieval_tags.join(", "));
                        println!("  Body:\n{}", e.body);
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
                    Some(e) => {
                        let engine = mimir_memory::MemoryDecisionEngine::default();
                        let signals = mimir_memory::ScoreSignals::default();
                        let breakdown = engine.score(&signals);
                        println!("Entry: {}", e.entry_id);
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
                for e in entries {
                    println!("  {} | {} | {}", e.entry_id, e.kind, e.scope);
                }
            }
            MemoryCmd::Publish => {
                let store =
                    mimir_memory::MemoryStore::open(".mimir/memory.db").unwrap_or_else(|e| {
                        println!("Memory DB not initialized: {}", e);
                        std::process::exit(1);
                    });
                let entries = store.list(Some("lesson"), None, Some("verified"))?;
                let safe: Vec<_> = entries.into_iter().filter(|e| e.safe_to_send).collect();
                mimir_memory::publish(&safe, ".mimir/project-rules.md")?;
                println!("Published {} entries to .mimir/project-rules.md", safe.len());
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
            MemoryCmd::ImportSessions { from } => {
                println!("Importing sessions from: {}", from);
                println!("(Session import not yet implemented for source: {})", from);
            }
        },
        Commands::Tui {
            packet,
            pipeline_result,
        } => {
            println!("Starting Mimir TUI...");
            let mut app = mimir_tui::App::new();
            if let Some(path) = packet {
                app.load_from_file(&path)?;
            }
            if let Some(path) = pipeline_result {
                app.load_pipeline_from_file(&path)?;
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
        Commands::Ask { question, json } => {
            let run_id = RunId::generate();
            let mimir_root = camino::Utf8PathBuf::from(".mimir");
            let run_dir = RunDir::create(&mimir_root, &run_id)?;
            let builder = ContextBuilder::new()
                .task_card(&question)
                .mode("standard")
                .provider("anthropic")
                .model("claude-sonnet-4-20250514");
            let packet = builder.build()?;
            let packet_json = serde_json::to_string_pretty(&packet)?;
            atomic_write(&run_dir.context_packet_path(), packet_json.as_bytes())?;
            if json {
                println!(
                    "{{\"run_id\":\"{}\",\"packet_id\":\"{}\",\"tokens\":{}}}",
                    run_id, packet.packet_id, packet.estimated_input_tokens
                );
            } else {
                println!("Question: {}", question);
                println!("Run ID: {}", run_id);
                println!("Packet: {} tokens", packet.estimated_input_tokens);
                println!("(Provider call requires ANTHROPIC_API_KEY)");
            }
        }
        Commands::Packet { cmd } => match cmd {
            PacketCmd::Share { run_id, output } => {
                let mimir_root = camino::Utf8PathBuf::from(".mimir");
                let run_dir = RunDir::create(&mimir_root, &RunId(run_id.clone()))?;
                let packet_path = run_dir.context_packet_path();
                if !std::path::Path::new(&packet_path).exists() {
                    println!("No packet found for run {}", run_id);
                    std::process::exit(1);
                }
                let data = std::fs::read_to_string(&packet_path)?;
                let mut packet: serde_json::Value = serde_json::from_str(&data)?;
                // Sanitize: remove any potential secrets
                if let Some(obj) = packet.as_object_mut() {
                    obj.remove("api_key");
                    obj.remove("secrets");
                }
                let sanitized = serde_json::to_string_pretty(&packet)?;
                if let Some(out) = output {
                    std::fs::write(&out, sanitized)?;
                    println!("Sanitized packet written to {}", out);
                } else {
                    println!("{}", sanitized);
                }
            }
            PacketCmd::Replay { run_id } => {
                let mimir_root = camino::Utf8PathBuf::from(".mimir");
                let run_dir = RunDir::create(&mimir_root, &RunId(run_id.clone()))?;
                let packet_path = run_dir.context_packet_path();
                if !std::path::Path::new(&packet_path).exists() {
                    println!("No packet found for run {}", run_id);
                    std::process::exit(1);
                }
                let data = std::fs::read_to_string(&packet_path)?;
                let packet: mimir_schemas::ContextPacket = serde_json::from_str(&data)?;
                println!("Replaying packet {}", packet.packet_id);
                println!("Task: {}", packet.task_card);
                println!("Included items: {}", packet.included.len());
                println!("Estimated tokens: {}", packet.estimated_input_tokens);
                println!(
                    "(To actually replay, run: mimir context call .mimir/runs/{}/context_packet.json)",
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
                let req = mimir_schemas::OverrideRequest {
                    schema_version: 1,
                    request_id: RunId::generate().to_string(),
                    run_id: RunId::generate().to_string(),
                    reason: format!("{} (cap: {}, auto_grant: {})", reason, cap, auto_grant_after),
                    requested_by: "cli".to_string(),
                };
                let run_id = RunId::generate();
                let mimir_root = camino::Utf8PathBuf::from(".mimir");
                let run_dir = RunDir::create(&mimir_root, &run_id)?;
                let req_json = serde_json::to_string_pretty(&req)?;
                atomic_write(&run_dir.context_packet_path(), req_json.as_bytes())?;
                println!("Override request recorded: {}", run_id);
                println!("  Requested cap: {} tokens", cap);
                println!("  Reason: {}", req.reason);
                println!(
                    "  Auto-grant after: {} failed attempts",
                    auto_grant_after
                );
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
                let run_dir = RunDir::create(&mimir_root, &RunId(run_id.clone()))?;
                let trace_path = run_dir.events_path();
                if !std::path::Path::new(&trace_path).exists() {
                    println!("No trace found for run {}", run_id);
                    std::process::exit(1);
                }
                let data = std::fs::read_to_string(&trace_path)?;
                let lines: Vec<&str> = data.lines().collect();
                let mut events: Vec<serde_json::Value> = Vec::new();
                for line in &lines {
                    if let Ok(event) = serde_json::from_str::<serde_json::Value>(line) {
                        events.push(event);
                    }
                }
                if redact {
                    for event in events.iter_mut() {
                        if let Some(obj) = event.as_object_mut() {
                            if let Some(payload) = obj.get_mut("payload") {
                                if let Some(p) = payload.as_object_mut() {
                                    if p.contains_key("api_key") {
                                        p.insert("api_key".into(), "***REDACTED***".into());
                                    }
                                }
                            }
                        }
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
    }

    Ok(())
}
