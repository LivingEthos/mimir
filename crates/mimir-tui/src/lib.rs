//! `mimir-tui` — Interactive terminal UI for Mimir.
//!
//! Built on `ratatui` and `crossterm`, the TUI provides:
//! - Budget ledger panel
//! - Included ranges panel
//! - Omitted candidates panel
//! - Provider count panel
//! - Permissions panel
//! - Diff/review panel
//!
//! # Performance target
//! Render frame < 16 ms (60 FPS).

#![warn(missing_docs)]

pub mod events;
pub mod panels;

use std::io;
use std::time::{Duration, Instant};

use crossterm::{
    event::{DisableMouseCapture, EnableMouseCapture, Event as CEvent},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::{Backend, CrosstermBackend},
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap},
    Frame, Terminal,
};
use tracing::{debug, info, warn};

use mimir_retrieval::{PackedManifest, PipelineResult, RecallGuardFlag};
use mimir_schemas::{BudgetLedger, BudgetCategory, ContextPacket};

use panels::{
    BudgetPanel, DiffPanel, IncludedPanel, OmittedPanel, PermissionsPanel, ProviderCountPanel,
};

/// Application state.
#[derive(Debug)]
pub struct App {
    /// Whether the app should exit on the next frame.
    pub should_quit: bool,
    /// Currently focused panel index.
    pub focused_panel: usize,
    /// Context packet (if loaded).
    pub packet: Option<ContextPacket>,
    /// Pipeline result (if loaded).
    pub pipeline_result: Option<PipelineResult>,
    /// Budget ledger.
    pub budget: Option<BudgetLedger>,
    /// Provider token counts.
    pub provider_counts: Vec<ProviderCount>,
    /// Permissions state.
    pub permissions: PermissionsState,
    /// Diff text lines.
    pub diff_lines: Vec<String>,
    /// Status message.
    pub status: String,
    /// Frame timing for performance monitoring.
    pub last_frame_time: Duration,
    /// Whether we are awaiting a quit confirmation after ESC.
    pub awaiting_quit_confirm: bool,
}

/// A provider count entry.
#[derive(Debug, Clone)]
pub struct ProviderCount {
    /// Provider name.
    pub name: String,
    /// Model name.
    pub model: String,
    /// Input tokens.
    pub input_tokens: u32,
    /// Output tokens.
    pub output_tokens: u32,
    /// Cache creation tokens.
    pub cache_creation_tokens: u32,
    /// Cache read tokens.
    pub cache_read_tokens: u32,
}

/// Permissions state.
#[derive(Debug, Clone, Default)]
pub struct PermissionsState {
    /// Whether editing is allowed.
    pub can_edit: bool,
    /// Whether running commands is allowed.
    pub can_run: bool,
    /// Whether network access is allowed.
    pub can_network: bool,
    /// Allowed paths.
    pub allowed_paths: Vec<String>,
    /// Blocked paths.
    pub blocked_paths: Vec<String>,
}

impl App {
    /// Create a new app with default state.
    pub fn new() -> Self {
        Self {
            should_quit: false,
            focused_panel: 0,
            packet: None,
            pipeline_result: None,
            budget: None,
            provider_counts: Vec::new(),
            permissions: PermissionsState::default(),
            diff_lines: Vec::new(),
            status: "Press ESC then q to quit | Ctrl-C exits".to_string(),
            last_frame_time: Duration::ZERO,
            awaiting_quit_confirm: false,
        }
    }

    /// Load a context packet into the app state.
    pub fn load_packet(&mut self, packet: ContextPacket) {
        self.budget = Some(BudgetLedger {
            schema_version: 1,
            run_id: packet.run_id.clone(),
            categories: vec![
                BudgetCategory {
                    name: "included".to_string(),
                    tokens: packet.estimated_input_tokens,
                    percentage: 0.0,
                },
                BudgetCategory {
                    name: "output_reserve".to_string(),
                    tokens: packet.output_reserve_tokens,
                    percentage: 0.0,
                },
                BudgetCategory {
                    name: "drift_reserve".to_string(),
                    tokens: packet.count_drift_reserve_tokens,
                    percentage: 0.0,
                },
            ],
            total_tokens: packet.cap_tokens,
        });
        self.provider_counts = vec![ProviderCount {
            name: packet.provider.clone(),
            model: packet.model.clone(),
            input_tokens: packet.estimated_input_tokens,
            output_tokens: 0,
            cache_creation_tokens: 0,
            cache_read_tokens: 0,
        }];
        self.packet = Some(packet);
    }

    /// Load a pipeline result into the app state.
    pub fn load_pipeline_result(&mut self, result: PipelineResult) {
        self.pipeline_result = Some(result);
    }

    /// Load diff text into the app state.
    pub fn load_diff(&mut self, diff: String) {
        self.diff_lines = diff.lines().map(|s| s.to_string()).collect();
    }

    /// Load a [`ContextPacket`] from a JSON file path.
    ///
    /// # Errors
    /// Returns an error if the file cannot be read or parsed.
    pub fn load_from_file(&mut self, path: &str) -> anyhow::Result<()> {
        let data = std::fs::read_to_string(path)?;
        let packet: ContextPacket = serde_json::from_str(&data)?;
        self.load_packet(packet);
        Ok(())
    }

    /// Load a [`PipelineResult`] from a JSON file path.
    ///
    /// # Errors
    /// Returns an error if the file cannot be read or parsed.
    pub fn load_pipeline_from_file(&mut self, path: &str) -> anyhow::Result<()> {
        let data = std::fs::read_to_string(path)?;
        let result: PipelineResult = serde_json::from_str(&data)?;
        self.load_pipeline_result(result);
        Ok(())
    }

    /// Cycle focus to the next panel.
    pub fn next_panel(&mut self) {
        self.focused_panel = (self.focused_panel + 1) % 6;
    }

    /// Cycle focus to the previous panel.
    pub fn prev_panel(&mut self) {
        self.focused_panel = (self.focused_panel + 6 - 1) % 6;
    }
}

impl Default for App {
    fn default() -> Self {
        Self::new()
    }
}

/// Run the TUI event loop.
///
/// # Errors
/// Returns `io::Error` on terminal setup or event read failure.
pub fn run_tui(app: &mut App) -> io::Result<()> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let res = run_event_loop(&mut terminal, app);

    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;

    res
}

fn run_event_loop<B: Backend>(terminal: &mut Terminal<B>, app: &mut App) -> io::Result<()>
where
    io::Error: From<<B as Backend>::Error>,
{
    let mut last_tick = Instant::now();
    let tick_rate = Duration::from_millis(16); // ~60 FPS target

    while !app.should_quit {
        let frame_start = Instant::now();
        terminal.draw(|f| draw_ui(f, app))?;
        app.last_frame_time = frame_start.elapsed();

        let timeout = tick_rate.saturating_sub(last_tick.elapsed());
        if crossterm::event::poll(timeout)? {
            if let CEvent::Key(key) = crossterm::event::read()? {
                events::handle_key_event(key, app);
            }
        }

        if last_tick.elapsed() >= tick_rate {
            last_tick = Instant::now();
        }

        if app.last_frame_time > Duration::from_millis(16) {
            warn!(
                "Slow frame: {:?} (target < 16ms)",
                app.last_frame_time
            );
        }
    }
    Ok(())
}

fn draw_ui(f: &mut Frame, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .margin(1)
        .constraints([
            Constraint::Length(3), // title bar
            Constraint::Min(0),    // main area
            Constraint::Length(1), // status bar
        ])
        .split(f.area());

    let title = Paragraph::new("Mimir TUI — Context Governor")
        .style(Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD))
        .alignment(ratatui::layout::Alignment::Center)
        .block(Block::default().borders(Borders::BOTTOM));
    f.render_widget(title, chunks[0]);

    let main_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(chunks[1]);

    let left_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage(33),
            Constraint::Percentage(33),
            Constraint::Percentage(34),
        ])
        .split(main_chunks[0]);

    let right_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage(33),
            Constraint::Percentage(33),
            Constraint::Percentage(34),
        ])
        .split(main_chunks[1]);

    BudgetPanel::draw(f, left_chunks[0], app, app.focused_panel == 0);
    IncludedPanel::draw(f, left_chunks[1], app, app.focused_panel == 1);
    OmittedPanel::draw(f, left_chunks[2], app, app.focused_panel == 2);
    ProviderCountPanel::draw(f, right_chunks[0], app, app.focused_panel == 3);
    PermissionsPanel::draw(f, right_chunks[1], app, app.focused_panel == 4);
    DiffPanel::draw(f, right_chunks[2], app, app.focused_panel == 5);

    let status = Paragraph::new(Line::from(vec![
        Span::styled(
            &app.status,
            Style::default().fg(Color::Gray),
        ),
        Span::raw(" | Frame: "),
        Span::styled(
            format!("{:?}", app.last_frame_time),
            if app.last_frame_time > Duration::from_millis(16) {
                Style::default().fg(Color::Red)
            } else {
                Style::default().fg(Color::Green)
            },
        ),
    ]));
    f.render_widget(status, chunks[2]);
}

/// Draw a panel block with optional focus highlight.
pub fn draw_panel_block(title: &str, area: Rect, is_focused: bool) -> Rect {
    let block = if is_focused {
        Block::default()
            .title(format!("[{}] {}", "*", title))
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Yellow))
    } else {
        Block::default()
            .title(title.to_string())
            .borders(Borders::ALL)
    };
    let inner = block.inner(area);
    // Note: caller must render the block if they want borders visible.
    inner
}
