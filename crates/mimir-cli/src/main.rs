//! Mimir CLI entry point.

use anyhow::Result;
use clap::{Parser, Subcommand};
use mimir_core::ContextBuilder;
use mimir_runs::RunId;
use tracing::{info};

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
    /// Build a context packet (stub).
    Context {
        #[command(subcommand)]
        cmd: ContextCmd,
    },
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
    /// Call provider with packet.
    Call {
        /// Packet file path.
        path: String,
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
            // Use mimir-runs to create the run directory structure properly
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
                println!("Packet {}: {} tokens", packet.packet_id, packet.estimated_input_tokens);
            }
            ContextCmd::Budget => {
                println!("Token budget: 64000 cap, 4096 output reserve, 1024 drift reserve");
            }
            ContextCmd::Omitted => {
                println!("No omitted items");
            }
            ContextCmd::Call { path } => {
                println!("Calling provider with packet from {}", path);
                println!("(Provider call requires ANTHROPIC_API_KEY to be set)");
            }
        },
    }

    Ok(())
}
