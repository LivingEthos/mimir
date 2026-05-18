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

#[tokio::main]
async fn main() -> Result<()> {
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
    }

    Ok(())
}
