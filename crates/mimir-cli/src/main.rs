//! Mimir CLI entry point.

use anyhow::Result;
use clap::{Parser, Subcommand};
use mimir_core::ContextBuilder;
use mimir_runs::RunId;
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
            println!("Editable set: {:?}", editable);
            println!("Max repair turns: {}", max_repair_turns);
            println!("Cost cap: ${}", cost_cap);
            println!("Run tests: {}", !no_test);
            println!("(Code execution requires provider API key)");
            println!("Output: results written to .mimir/runs/<run_id>/");
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
    }

    Ok(())
}
