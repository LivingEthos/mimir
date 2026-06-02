//! Session orchestration primitives for Mimir UI and CLI workflows.
//!
//! This crate is the first extraction step toward a shared backend for the
//! future Mimir Studio UI and the existing CLI. It owns durable session state,
//! typed append-only events, slash command parsing, and provider-free turn
//! orchestration that can be reused without shelling out to the CLI.

pub mod packet;

use std::cmp::Reverse;
use std::fmt;
use std::fs;
use std::io::{BufRead, ErrorKind, Write};

use anyhow::{anyhow, bail, Context, Result};
use camino::{Utf8Path, Utf8PathBuf};
use chrono::{DateTime, Utc};
use mimir_context::ContextBuilder;
use mimir_runs::{atomic_write, RunDir, RunId};
use mimir_schemas::ContextPacket;
use mimir_subagents::EvidenceSummary;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

const SESSION_SCHEMA_VERSION: u32 = 1;
const EVENT_SCHEMA_VERSION: u32 = 1;
const MAX_PROVIDER_RESPONSE_ARTIFACT_BYTES: usize = 256 * 1024;
const PROJECT_RULES_TEMPLATE: &str = r"# Mimir Project Rules

Keep durable repository guidance here. Mimir includes this file in context packets when it exists, subject to redaction and size limits.

<!-- MIMIR_PUBLISHED_START -->
<!-- MIMIR_PUBLISHED_END -->
";
const CHECK_NO_SECRET_MATERIAL: &str = r#"# No Provider Secrets

Source-controlled checks should catch obvious credential material before a model call.

- severity: error
- forbidden: (?i)(api[_-]?key|secret|token|password)\s*[:=]\s*['"]?[A-Za-z0-9_\-]{16,}
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

/// Seed `.mimir` project workflow files without overwriting existing files.
///
/// # Errors
/// Returns an error when directories or missing files cannot be created.
pub fn init_project_files(base: &Utf8Path) -> Result<Vec<String>> {
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

fn write_file_if_missing(path: &Utf8Path, content: &str) -> Result<bool> {
    if path.exists() {
        return Ok(false);
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, content)?;
    Ok(true)
}

/// Stable identifier for a Mimir UI/backend session.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct SessionId(String);

impl SessionId {
    /// Generate a new random session id.
    #[must_use]
    pub fn generate() -> Self {
        Self(format!("sess-{}", Uuid::new_v4()))
    }

    /// Parse an existing session id.
    ///
    /// # Errors
    /// Returns an error when the id is empty or contains path separators.
    pub fn parse(value: impl Into<String>) -> Result<Self> {
        let value = value.into();
        if !Self::is_valid(&value) {
            bail!("invalid session id");
        }
        Ok(Self(value))
    }

    /// Return whether a string is safe to use as a session directory name.
    #[must_use]
    pub fn is_valid(value: &str) -> bool {
        let Some(suffix) = value.strip_prefix("sess-") else {
            return false;
        };
        !suffix.is_empty()
            && !value.contains('/')
            && !value.contains('\\')
            && !value.contains("..")
            && !value.contains(':')
            && value
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
            && Uuid::parse_str(suffix)
                .map(|uuid| uuid.hyphenated().to_string() == suffix)
                .unwrap_or(false)
    }

    /// Borrow the session id as a string slice.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for SessionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Persisted metadata for a session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionMetadata {
    /// Schema version for the metadata file.
    pub schema_version: u32,
    /// Session id.
    pub session_id: SessionId,
    /// Human-readable title.
    pub title: String,
    /// Workspace root the session is scoped to.
    pub workspace_root: Utf8PathBuf,
    /// Creation timestamp.
    pub created_at: DateTime<Utc>,
    /// Last update timestamp.
    pub updated_at: DateTime<Utc>,
}

/// Durable session record loaded from `.mimir/sessions`.
#[derive(Debug, Clone)]
pub struct SessionRecord {
    metadata: SessionMetadata,
    root: Utf8PathBuf,
    next_sequence: u64,
}

impl SessionRecord {
    /// Return session metadata.
    #[must_use]
    pub fn metadata(&self) -> &SessionMetadata {
        &self.metadata
    }

    /// Return the session root directory.
    #[must_use]
    pub fn root(&self) -> &Utf8Path {
        &self.root
    }

    /// Return the event log path.
    #[must_use]
    pub fn events_path(&self) -> Utf8PathBuf {
        self.root.join("events.jsonl")
    }

    /// Append a typed event to the session log.
    ///
    /// # Errors
    /// Returns an error when the event cannot be serialized or written.
    pub fn append_event(&mut self, kind: SessionEventKind) -> Result<SessionEvent> {
        ensure_session_dir(&self.root)?;
        let events_path = self.events_path();
        ensure_regular_file_if_exists(&events_path, "session events file")?;
        let event = redact_session_event(
            SessionEvent {
                schema_version: EVENT_SCHEMA_VERSION,
                event_id: format!("evt-{}", Uuid::new_v4()),
                session_id: self.metadata.session_id.clone(),
                sequence: self.next_sequence,
                timestamp: Utc::now(),
                kind,
            },
            &self.metadata.workspace_root,
        )?;
        self.next_sequence = self
            .next_sequence
            .checked_add(1)
            .ok_or_else(|| anyhow!("session event sequence overflow"))?;
        self.metadata.updated_at = event.timestamp;
        write_metadata(&self.root, &self.metadata)?;

        let mut file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(events_path)?;
        serde_json::to_writer(&mut file, &event)?;
        writeln!(file)?;
        Ok(event)
    }

    /// Read all persisted events for this session.
    ///
    /// # Errors
    /// Returns an error if the event log cannot be read or parsed.
    pub fn read_events(&self) -> Result<Vec<SessionEvent>> {
        read_events(&self.events_path())
    }
}

/// Store for durable Mimir sessions.
#[derive(Debug, Clone)]
pub struct SessionStore {
    root: Utf8PathBuf,
}

impl SessionStore {
    /// Create a session store under `<workspace>/.mimir/sessions`.
    #[must_use]
    pub fn for_workspace(workspace_root: impl Into<Utf8PathBuf>) -> Self {
        let workspace_root = workspace_root.into();
        Self {
            root: workspace_root.join(".mimir").join("sessions"),
        }
    }

    /// Create a session store from an explicit `.mimir` root.
    #[must_use]
    pub fn from_mimir_root(mimir_root: impl Into<Utf8PathBuf>) -> Self {
        Self {
            root: mimir_root.into().join("sessions"),
        }
    }

    /// Return the session store root.
    #[must_use]
    pub fn root(&self) -> &Utf8Path {
        &self.root
    }

    /// Create and persist a new session.
    ///
    /// # Errors
    /// Returns an error when directories or metadata cannot be written.
    pub fn create(
        &self,
        workspace_root: impl Into<Utf8PathBuf>,
        title: impl Into<String>,
    ) -> Result<SessionRecord> {
        ensure_store_root_for_create(&self.root)?;
        let now = Utc::now();
        let metadata = SessionMetadata {
            schema_version: SESSION_SCHEMA_VERSION,
            session_id: SessionId::generate(),
            title: mimir_security::redact_secrets(&title.into()),
            workspace_root: workspace_root.into(),
            created_at: now,
            updated_at: now,
        };
        validate_session_id(&metadata.session_id)?;
        let session_root = self.root.join(metadata.session_id.as_str());
        fs::create_dir_all(&session_root)?;
        ensure_session_dir(&session_root)?;
        write_metadata(&session_root, &metadata)?;
        let mut record = SessionRecord {
            metadata,
            root: session_root,
            next_sequence: 0,
        };
        let title = record.metadata.title.clone();
        let workspace_root = record.metadata.workspace_root.clone();
        record.append_event(SessionEventKind::SessionCreated {
            title,
            workspace_root,
        })?;
        Ok(record)
    }

    /// Open an existing session by id.
    ///
    /// # Errors
    /// Returns an error if metadata or events cannot be read.
    pub fn open(&self, session_id: &SessionId) -> Result<SessionRecord> {
        ensure_store_root_for_open(&self.root)?;
        validate_session_id(session_id)?;
        let session_root = self.root.join(session_id.as_str());
        ensure_session_dir(&session_root)?;
        let metadata = read_metadata_for_session(&session_root, Some(session_id))?;
        let next_sequence = read_events(&session_root.join("events.jsonl"))?.len() as u64;
        Ok(SessionRecord {
            metadata,
            root: session_root,
            next_sequence,
        })
    }

    /// List sessions sorted by updated timestamp, newest first.
    ///
    /// # Errors
    /// Returns an error if the store directory cannot be scanned.
    pub fn list(&self) -> Result<Vec<SessionMetadata>> {
        if !ensure_store_root_for_list(&self.root)? {
            return Ok(Vec::new());
        }
        let entries = match fs::read_dir(&self.root) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(error.into()),
        };

        let mut sessions = Vec::new();
        for entry in entries {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().to_string();
            let session_id = SessionId::parse(name)?;
            let path = Utf8PathBuf::from_path_buf(entry.path())
                .map_err(|path| anyhow!("session path is not UTF-8: {}", path.display()))?;
            ensure_session_dir(&path)?;
            sessions.push(read_metadata_for_session(&path, Some(&session_id))?);
        }
        sessions.sort_by_key(|session| Reverse(session.updated_at));
        Ok(sessions)
    }
}

/// A parsed command submitted by a UI/CLI user.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionCommand {
    /// Show command help.
    Help,
    /// Show workspace and session status.
    Status,
    /// Initialize Mimir project files.
    Init,
    /// Run doctor checks.
    Doctor,
    /// Build or refresh context for a task.
    Context { task: String },
    /// Explain why a path was included or omitted.
    Why { path: String },
    /// Run read-only exploration.
    Explore { query: String },
    /// Produce an implementation plan.
    Plan { task: String },
    /// Run code mode.
    Code { task: String },
    /// Run source-controlled checks.
    Check,
    /// Show current diff.
    Diff,
    /// List previous runs.
    Runs,
    /// Resume a session.
    Resume { target: Option<String> },
    /// Share a packet or run.
    Share { target: Option<String> },
    /// Open settings.
    Settings,
    /// Plain user prompt.
    UserPrompt { message: String },
}

/// Where a Studio slash command is currently handled.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CommandSupport {
    /// Executed by the local Mimir backend.
    Backend,
    /// Handled entirely in the Studio browser UI.
    Local,
    /// Visible in Studio but not wired as an executable slash command yet.
    Planned,
}

/// User-facing metadata for a built-in Studio slash command.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct CommandSpec {
    /// Slash command name, including the leading `/`.
    pub name: &'static str,
    /// Compact usage string for command palettes and help output.
    pub usage: &'static str,
    /// Short description of what the command does.
    pub summary: &'static str,
    /// Current execution surface.
    pub support: CommandSupport,
    /// Whether the command expects trailing user input.
    pub takes_input: bool,
    /// Whether Studio should submit the command as executable today.
    pub enabled: bool,
    /// Why a visible command is not executable, when disabled.
    pub disabled_reason: Option<&'static str>,
}

const BUILTIN_COMMAND_REGISTRY: &[CommandSpec] = &[
    CommandSpec {
        name: "/help",
        usage: "/help",
        summary: "Show command registry",
        support: CommandSupport::Backend,
        takes_input: false,
        enabled: true,
        disabled_reason: None,
    },
    CommandSpec {
        name: "/status",
        usage: "/status",
        summary: "Show workspace readiness",
        support: CommandSupport::Backend,
        takes_input: false,
        enabled: true,
        disabled_reason: None,
    },
    CommandSpec {
        name: "/init",
        usage: "/init",
        summary: "Seed Mimir workflow files",
        support: CommandSupport::Backend,
        takes_input: false,
        enabled: true,
        disabled_reason: None,
    },
    CommandSpec {
        name: "/doctor",
        usage: "/doctor",
        summary: "Run local environment checks",
        support: CommandSupport::Backend,
        takes_input: false,
        enabled: true,
        disabled_reason: None,
    },
    CommandSpec {
        name: "/check",
        usage: "/check",
        summary: "Run source-controlled checks",
        support: CommandSupport::Backend,
        takes_input: false,
        enabled: true,
        disabled_reason: None,
    },
    CommandSpec {
        name: "/explore",
        usage: "/explore <question>",
        summary: "Run provider-free evidence search",
        support: CommandSupport::Backend,
        takes_input: true,
        enabled: true,
        disabled_reason: None,
    },
    CommandSpec {
        name: "/context",
        usage: "/context <task>",
        summary: "Build a context packet",
        support: CommandSupport::Backend,
        takes_input: true,
        enabled: true,
        disabled_reason: None,
    },
    CommandSpec {
        name: "/why",
        usage: "/why <path>",
        summary: "Explain context inclusion or omission",
        support: CommandSupport::Backend,
        takes_input: true,
        enabled: true,
        disabled_reason: None,
    },
    CommandSpec {
        name: "/runs",
        usage: "/runs",
        summary: "List local Mimir runs",
        support: CommandSupport::Backend,
        takes_input: false,
        enabled: true,
        disabled_reason: None,
    },
    CommandSpec {
        name: "/settings",
        usage: "/settings",
        summary: "Open Studio settings",
        support: CommandSupport::Local,
        takes_input: false,
        enabled: true,
        disabled_reason: None,
    },
    CommandSpec {
        name: "/resume",
        usage: "/resume [session]",
        summary: "Resume a Studio session",
        support: CommandSupport::Local,
        takes_input: true,
        enabled: true,
        disabled_reason: None,
    },
    CommandSpec {
        name: "/plan",
        usage: "/plan <task>",
        summary: "Provider-backed planning mode",
        support: CommandSupport::Planned,
        takes_input: true,
        enabled: false,
        disabled_reason: Some("provider-backed plan mode is not wired in Studio yet"),
    },
    CommandSpec {
        name: "/code",
        usage: "/code <task>",
        summary: "Provider-backed patch mode",
        support: CommandSupport::Planned,
        takes_input: true,
        enabled: false,
        disabled_reason: Some("provider-backed code mode needs explicit editable target UI first"),
    },
    CommandSpec {
        name: "/share",
        usage: "/share <run>",
        summary: "Preview a redacted packet-share bundle",
        support: CommandSupport::Local,
        takes_input: true,
        enabled: true,
        disabled_reason: None,
    },
    CommandSpec {
        name: "/diff",
        usage: "/diff",
        summary: "Open a Git diff inspector",
        support: CommandSupport::Planned,
        takes_input: false,
        enabled: false,
        disabled_reason: Some("diff inspection is planned for a later Studio slice"),
    },
];

/// Built-in Studio slash command metadata.
#[must_use]
pub fn builtin_command_registry() -> &'static [CommandSpec] {
    BUILTIN_COMMAND_REGISTRY
}

/// Parse text entered into the session composer.
#[must_use]
pub fn parse_session_command(input: &str) -> SessionCommand {
    let trimmed = input.trim();
    let Some(command_line) = trimmed.strip_prefix('/') else {
        return SessionCommand::UserPrompt {
            message: trimmed.to_string(),
        };
    };

    let mut parts = command_line.splitn(2, char::is_whitespace);
    let command = parts.next().unwrap_or_default().to_ascii_lowercase();
    let rest = parts.next().unwrap_or_default().trim().to_string();

    match command.as_str() {
        "help" => SessionCommand::Help,
        "status" => SessionCommand::Status,
        "init" => SessionCommand::Init,
        "doctor" => SessionCommand::Doctor,
        "context" => SessionCommand::Context { task: rest },
        "why" => SessionCommand::Why { path: rest },
        "explore" => SessionCommand::Explore { query: rest },
        "plan" => SessionCommand::Plan { task: rest },
        "code" => SessionCommand::Code { task: rest },
        "check" => SessionCommand::Check,
        "diff" => SessionCommand::Diff,
        "runs" => SessionCommand::Runs,
        "resume" => SessionCommand::Resume {
            target: (!rest.is_empty()).then_some(rest),
        },
        "share" => SessionCommand::Share {
            target: (!rest.is_empty()).then_some(rest),
        },
        "settings" => SessionCommand::Settings,
        _ => SessionCommand::UserPrompt {
            message: trimmed.to_string(),
        },
    }
}

/// Append-only typed session event.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionEvent {
    /// Schema version.
    pub schema_version: u32,
    /// Event id.
    pub event_id: String,
    /// Owning session id.
    pub session_id: SessionId,
    /// Monotonic session-local sequence number.
    pub sequence: u64,
    /// Event timestamp.
    pub timestamp: DateTime<Utc>,
    /// Typed event payload.
    #[serde(flatten)]
    pub kind: SessionEventKind,
}

/// Typed event payloads used by the UI event stream.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "payload")]
pub enum SessionEventKind {
    /// A session was created.
    #[serde(rename = "session.created")]
    SessionCreated {
        /// Session title.
        title: String,
        /// Workspace root.
        workspace_root: Utf8PathBuf,
    },
    /// A turn started.
    #[serde(rename = "turn.started")]
    TurnStarted {
        /// Turn id.
        turn_id: String,
        /// User-facing command name.
        command: String,
        /// Task or prompt text.
        task: String,
    },
    /// Context packet construction started.
    #[serde(rename = "context.build.started")]
    ContextBuildStarted {
        /// Turn id.
        turn_id: String,
        /// Provider snapshot requested for the packet.
        provider: String,
        /// Model snapshot requested for the packet.
        model: String,
    },
    /// A context packet is ready.
    #[serde(rename = "context.packet.ready")]
    ContextPacketReady {
        /// Run id.
        run_id: String,
        /// Packet id.
        packet_id: String,
        /// Stable packet hash.
        packet_hash: String,
        /// Packet artifact path.
        packet_path: Utf8PathBuf,
        /// Estimated input tokens.
        estimated_input_tokens: u32,
        /// Guidance files included in the packet.
        guidance_files: Vec<String>,
        /// Likely starting files.
        likely_files: Vec<String>,
    },
    /// A risky omission was found.
    #[serde(rename = "context.omission.risk")]
    ContextOmissionRisk {
        /// Run id.
        run_id: String,
        /// Omitted path.
        path: String,
        /// Omission reason.
        reason: String,
        /// Omission risk label.
        risk: Option<String>,
    },
    /// An artifact was written.
    #[serde(rename = "artifact.written")]
    ArtifactWritten {
        /// Run id.
        run_id: String,
        /// Artifact kind.
        artifact_kind: String,
        /// Artifact path.
        path: Utf8PathBuf,
    },
    /// Source-controlled checks completed.
    #[serde(rename = "check.completed")]
    CheckCompleted {
        /// Number of checks loaded.
        checks_loaded: usize,
        /// Number of findings reported.
        findings_count: usize,
        /// Number of blocking findings.
        blocking_findings: usize,
        /// Whether the check turn passed.
        passed: bool,
    },
    /// Read-only exploration completed.
    #[serde(rename = "explore.completed")]
    ExploreCompleted {
        /// Run id.
        run_id: String,
        /// Evidence artifact path.
        evidence_path: Utf8PathBuf,
        /// Number of findings reported.
        findings_count: usize,
        /// Relevant paths found by the exploration.
        relevant_paths: Vec<String>,
        /// Exploration confidence.
        confidence: f64,
    },
    /// Doctor checks completed.
    #[serde(rename = "doctor.completed")]
    DoctorCompleted {
        /// Overall doctor status.
        status: DoctorStatus,
        /// Warning count.
        warnings: u32,
        /// Failure count.
        failures: u32,
    },
    /// Workspace status was loaded.
    #[serde(rename = "workspace.status.ready")]
    WorkspaceStatusReady {
        /// Redacted, provider-secret-free workspace status payload.
        status: serde_json::Value,
    },
    /// Explicit user approval is required before continuing.
    #[serde(rename = "approval.requested")]
    ApprovalRequested {
        /// Approval request.
        request: ApprovalRequest,
    },
    /// A pending approval was resolved.
    #[serde(rename = "approval.resolved")]
    ApprovalResolved {
        /// Approval decision.
        decision: ApprovalDecision,
    },
    /// A turn completed successfully.
    #[serde(rename = "turn.completed")]
    TurnCompleted {
        /// Turn id.
        turn_id: String,
        /// Summary for display.
        summary: String,
    },
    /// A turn failed.
    #[serde(rename = "turn.failed")]
    TurnFailed {
        /// Turn id.
        turn_id: String,
        /// Error message.
        error: String,
    },
}

/// Reference to a run artifact that can be shown by a UI or replayed later.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtifactRef {
    /// Owning run id.
    pub run_id: String,
    /// Artifact kind, for example `context_packet` or `patch_diff`.
    pub artifact_kind: String,
    /// Artifact path.
    pub path: Utf8PathBuf,
    /// Optional SHA-256 digest for integrity checks.
    pub sha256: Option<String>,
    /// Whether the artifact has been redacted for UI display or sharing.
    pub redacted: bool,
}

/// Permission request emitted when a turn needs explicit user approval.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApprovalRequest {
    /// Approval id.
    pub approval_id: String,
    /// Session id.
    pub session_id: SessionId,
    /// Turn id that requested approval.
    pub turn_id: String,
    /// Tool or capability requesting approval.
    pub tool_name: String,
    /// Human-readable reason.
    pub reason: String,
    /// Optional path affected by the request.
    pub path: Option<Utf8PathBuf>,
    /// Optional shell command affected by the request.
    pub command: Option<String>,
    /// Optional artifact containing more evidence, such as a diff.
    pub artifact: Option<ArtifactRef>,
    /// Request timestamp.
    pub requested_at: DateTime<Utc>,
}

/// User decision for an approval request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApprovalDecision {
    /// Approval id.
    pub approval_id: String,
    /// Decision action.
    pub action: ApprovalAction,
    /// Decision timestamp.
    pub decided_at: DateTime<Utc>,
}

/// Approval actions supported by the first UI backend contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalAction {
    /// Allow this one request.
    AllowOnce,
    /// Allow similar requests for the current session.
    AllowForSession,
    /// Deny the request.
    Deny,
}

/// Provider-free context suggestion request.
#[derive(Debug, Clone)]
pub struct ContextSuggestRequest {
    /// Task or question to map.
    pub task: String,
    /// Provider to snapshot into the packet.
    pub provider: String,
    /// Model override.
    pub model: Option<String>,
}

/// Shared request for constructing a context packet through session APIs.
#[derive(Debug, Clone)]
pub struct ContextPacketBuildRequest {
    /// Task or question to map.
    pub task: String,
    /// Mimir workflow mode, such as `ask`, `plan`, or `code`.
    pub mode: String,
    /// Provider to snapshot into the packet. When omitted, the context builder default is used.
    pub provider: Option<String>,
    /// Model to snapshot into the packet. When omitted, the context builder default is used.
    pub model: Option<String>,
    /// Workspace root used for retrieval-backed packets.
    pub workspace_root: Option<Utf8PathBuf>,
    /// Explicit `.mimir` root. Defaults to `<workspace_root>/.mimir` when persistence is enabled.
    pub mimir_root: Option<Utf8PathBuf>,
    /// Editable targets for code-mode packet construction.
    pub edit_targets: Vec<String>,
    /// Persist the packet under `.mimir/runs/<run_id>/context_packet.json` when a root is available.
    pub persist: bool,
}

/// Shared result for a context packet constructed by session APIs.
#[derive(Debug, Clone)]
pub struct ContextPacketBuildResult {
    /// Validated generated run id.
    pub run_id: RunId,
    /// Built context packet.
    pub packet: ContextPacket,
    /// Persisted packet path when `persist` was requested and a root was available.
    pub packet_path: Option<Utf8PathBuf>,
}

/// Risky omitted item reported by a context suggestion.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RiskyOmission {
    /// Omitted path.
    pub path: String,
    /// Omission reason.
    pub reason: String,
    /// Risk label.
    pub risk: Option<String>,
}

/// Result of a provider-free context suggestion turn.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextSuggestResult {
    /// Run id.
    pub run_id: String,
    /// Packet id.
    pub packet_id: String,
    /// Stable packet hash.
    pub packet_hash: String,
    /// Packet path.
    pub packet_path: Utf8PathBuf,
    /// Estimated input tokens.
    pub estimated_input_tokens: u32,
    /// Guidance files included in the packet.
    pub guidance_files: Vec<String>,
    /// Likely starting files.
    pub likely_files: Vec<String>,
    /// Risky omitted paths.
    pub risky_omissions: Vec<RiskyOmission>,
    /// Source-controlled checks loaded.
    pub checks_loaded: usize,
    /// Events emitted by this turn.
    pub events: Vec<SessionEvent>,
}

/// Result of running source-controlled checks.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckRunResult {
    /// Number of check files loaded.
    pub checks_loaded: usize,
    /// Findings reported by the checks.
    pub findings: Vec<mimir_review::Finding>,
    /// Findings that should block CI.
    pub blocking_findings: usize,
    /// Whether no blocking findings were reported.
    pub passed: bool,
    /// Events emitted by this turn.
    pub events: Vec<SessionEvent>,
}

/// Provider-free exploration request.
#[derive(Debug, Clone)]
pub struct ExploreRequest {
    /// Exploration question.
    pub query: String,
}

/// Result of a provider-free exploration turn.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExploreResult {
    /// Run id.
    pub run_id: String,
    /// Redacted evidence artifact path.
    pub evidence_path: Utf8PathBuf,
    /// Evidence collected by the read-only local subagent.
    pub evidence: EvidenceSummary,
    /// Events emitted by this turn.
    pub events: Vec<SessionEvent>,
}

/// Status label for an individual doctor probe.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DoctorProbeStatus {
    /// Probe passed.
    Ok,
    /// Probe failed.
    Fail,
    /// Probe is missing optional setup.
    Missing,
    /// Probe is optional and not configured.
    Optional,
}

impl DoctorProbeStatus {
    /// Return the stable CLI label for this status.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Fail => "fail",
            Self::Missing => "missing",
            Self::Optional => "optional",
        }
    }
}

/// One doctor probe result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DoctorProbe {
    /// Probe status.
    pub status: DoctorProbeStatus,
    /// Human-readable detail printed in parentheses by the CLI.
    pub detail: String,
}

impl DoctorProbe {
    /// Create a probe result.
    #[must_use]
    pub fn new(status: DoctorProbeStatus, detail: impl Into<String>) -> Self {
        Self {
            status,
            detail: detail.into(),
        }
    }
}

/// Overall doctor command status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DoctorStatus {
    /// All probes passed.
    Ok,
    /// No failures occurred, but one or more probes emitted warnings.
    Warnings,
    /// One or more probes failed.
    Failed,
}

impl DoctorStatus {
    /// Return the stable CLI label for this status.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Warnings => "warnings",
            Self::Failed => "failed",
        }
    }
}

/// Provider-free doctor request.
#[derive(Debug, Clone)]
pub struct DoctorRequest {
    /// Version string to report for the calling surface.
    pub version: String,
}

/// Result of the provider-free doctor command.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DoctorResult {
    /// CLI package version.
    pub version: String,
    /// Generated run id sample.
    pub run_id_sample: String,
    /// Mimir config file probe.
    pub config: DoctorProbe,
    /// Provider capability registry probe.
    pub provider_capabilities: DoctorProbe,
    /// Local token counter probe.
    pub token_counter: DoctorProbe,
    /// Context packet builder probe.
    pub context_packet: DoctorProbe,
    /// Workspace write-permission probe.
    pub permissions: DoctorProbe,
    /// Provider credential presence probe; never includes credential values.
    pub provider_credentials: DoctorProbe,
    /// Warning count.
    pub warnings: u32,
    /// Failure count.
    pub failures: u32,
    /// Overall status.
    pub status: DoctorStatus,
    /// Events emitted by this turn.
    pub events: Vec<SessionEvent>,
}

/// Shared turn runner for UI and CLI workflows.
#[derive(Debug, Clone)]
pub struct TurnRunner {
    workspace_root: Utf8PathBuf,
    mimir_root: Utf8PathBuf,
}

impl TurnRunner {
    /// Create a runner for a workspace.
    #[must_use]
    pub fn for_workspace(workspace_root: impl Into<Utf8PathBuf>) -> Self {
        let workspace_root = workspace_root.into();
        let mimir_root = workspace_root.join(".mimir");
        Self {
            workspace_root,
            mimir_root,
        }
    }

    /// Create a runner with an explicit `.mimir` root.
    #[must_use]
    pub fn new(workspace_root: impl Into<Utf8PathBuf>, mimir_root: impl Into<Utf8PathBuf>) -> Self {
        Self {
            workspace_root: workspace_root.into(),
            mimir_root: mimir_root.into(),
        }
    }

    /// Run source-controlled checks and persist session events.
    ///
    /// # Errors
    /// Returns an error when check execution or event writes fail.
    pub fn run_check(&self, session: &mut SessionRecord) -> Result<CheckRunResult> {
        let turn_id = format!("turn-{}", Uuid::new_v4());
        let mut events = Vec::new();
        emit(
            session,
            &mut events,
            SessionEventKind::TurnStarted {
                turn_id: turn_id.clone(),
                command: "check".to_string(),
                task: String::new(),
            },
        )?;

        match self.check() {
            Ok(mut result) => {
                emit(
                    session,
                    &mut events,
                    SessionEventKind::CheckCompleted {
                        checks_loaded: result.checks_loaded,
                        findings_count: result.findings.len(),
                        blocking_findings: result.blocking_findings,
                        passed: result.passed,
                    },
                )?;
                emit(
                    session,
                    &mut events,
                    SessionEventKind::TurnCompleted {
                        turn_id,
                        summary: format!(
                            "Ran {} source-controlled checks with {} blocking findings",
                            result.checks_loaded, result.blocking_findings
                        ),
                    },
                )?;
                result.events = events;
                Ok(result)
            }
            Err(error) => {
                emit(
                    session,
                    &mut events,
                    SessionEventKind::TurnFailed {
                        turn_id,
                        error: error.to_string(),
                    },
                )?;
                Err(error)
            }
        }
    }

    /// Run source-controlled checks without session event writes.
    ///
    /// # Errors
    /// Returns an error if check execution fails.
    pub fn check(&self) -> Result<CheckRunResult> {
        let checks = mimir_review::checks::load_checks(&self.workspace_root);
        let findings = mimir_review::checks::run_checks(&checks, &self.workspace_root);
        let blocking_findings = blocking_finding_count(&findings);
        Ok(CheckRunResult {
            checks_loaded: checks.len(),
            findings,
            blocking_findings,
            passed: blocking_findings == 0,
            events: Vec::new(),
        })
    }

    /// Run read-only exploration and persist session events.
    ///
    /// # Errors
    /// Returns an error when exploration, artifact writes, or event writes fail.
    pub fn run_explore(
        &self,
        session: &mut SessionRecord,
        request: ExploreRequest,
    ) -> Result<ExploreResult> {
        let turn_id = format!("turn-{}", Uuid::new_v4());
        let task = request.query.clone();
        let mut events = Vec::new();
        emit(
            session,
            &mut events,
            SessionEventKind::TurnStarted {
                turn_id: turn_id.clone(),
                command: "explore".to_string(),
                task,
            },
        )?;

        match self.explore(request) {
            Ok(mut result) => {
                emit(
                    session,
                    &mut events,
                    SessionEventKind::ArtifactWritten {
                        run_id: result.run_id.clone(),
                        artifact_kind: "explore_evidence".to_string(),
                        path: result.evidence_path.clone(),
                    },
                )?;
                emit(
                    session,
                    &mut events,
                    SessionEventKind::ExploreCompleted {
                        run_id: result.run_id.clone(),
                        evidence_path: result.evidence_path.clone(),
                        findings_count: result.evidence.findings.len(),
                        relevant_paths: result.evidence.relevant_paths.clone(),
                        confidence: result.evidence.confidence,
                    },
                )?;
                emit(
                    session,
                    &mut events,
                    SessionEventKind::TurnCompleted {
                        turn_id,
                        summary: format!(
                            "Wrote read-only exploration evidence to {}",
                            result.evidence_path
                        ),
                    },
                )?;
                result.events = events;
                Ok(result)
            }
            Err(error) => {
                emit(
                    session,
                    &mut events,
                    SessionEventKind::TurnFailed {
                        turn_id,
                        error: error.to_string(),
                    },
                )?;
                Err(error)
            }
        }
    }

    /// Run read-only exploration without session event writes.
    ///
    /// # Errors
    /// Returns an error when exploration or artifact writes fail.
    pub fn explore(&self, request: ExploreRequest) -> Result<ExploreResult> {
        let run_id = RunId::generate();
        let run_dir = RunDir::create(&self.mimir_root, &run_id)?;
        let run_id_string = run_id.to_string();
        let evidence = mimir_subagents::subagents::execute_in(
            &self.workspace_root,
            "search",
            &request.query,
            Some(&run_id_string),
        )
        .map_err(|error| anyhow!("{error}"))?;
        let evidence_path = run_dir.root().join("explore_evidence.json");
        write_redacted_json_artifact(&evidence_path, &evidence)?;

        Ok(ExploreResult {
            run_id: run_id_string,
            evidence_path,
            evidence,
            events: Vec::new(),
        })
    }

    /// Run doctor checks and persist session events.
    ///
    /// # Errors
    /// Returns an error when event writes fail.
    pub fn run_doctor(
        &self,
        session: &mut SessionRecord,
        request: DoctorRequest,
    ) -> Result<DoctorResult> {
        let turn_id = format!("turn-{}", Uuid::new_v4());
        let mut events = Vec::new();
        emit(
            session,
            &mut events,
            SessionEventKind::TurnStarted {
                turn_id: turn_id.clone(),
                command: "doctor".to_string(),
                task: String::new(),
            },
        )?;

        match self.doctor(request) {
            Ok(mut result) => {
                emit(
                    session,
                    &mut events,
                    SessionEventKind::DoctorCompleted {
                        status: result.status,
                        warnings: result.warnings,
                        failures: result.failures,
                    },
                )?;
                emit(
                    session,
                    &mut events,
                    SessionEventKind::TurnCompleted {
                        turn_id,
                        summary: format!("Doctor status: {}", result.status.as_str()),
                    },
                )?;
                result.events = events;
                Ok(result)
            }
            Err(error) => {
                emit(
                    session,
                    &mut events,
                    SessionEventKind::TurnFailed {
                        turn_id,
                        error: error.to_string(),
                    },
                )?;
                Err(error)
            }
        }
    }

    /// Run doctor checks without session event writes.
    ///
    /// # Errors
    /// Returns an error if an unexpected local IO operation fails.
    pub fn doctor(&self, request: DoctorRequest) -> Result<DoctorResult> {
        let mut warnings = 0u32;
        let mut failures = 0u32;

        let config_path = self.mimir_root.join("config.yaml");
        let config = if config_path.is_file() {
            match fs::read_to_string(&config_path)
                .map_err(anyhow::Error::from)
                .and_then(|content| {
                    serde_yaml::from_str::<serde_yaml::Value>(&content).map_err(anyhow::Error::from)
                }) {
                Ok(_) => DoctorProbe::new(DoctorProbeStatus::Ok, ".mimir/config.yaml"),
                Err(error) => {
                    failures += 1;
                    DoctorProbe::new(DoctorProbeStatus::Fail, error.to_string())
                }
            }
        } else {
            warnings += 1;
            DoctorProbe::new(DoctorProbeStatus::Missing, "run `mimir init`")
        };

        let provider_capabilities =
            match mimir_providers::capabilities::provider_capabilities_list()
                .map_err(anyhow::Error::msg)
            {
                Ok(list) if !list.providers.is_empty() => DoctorProbe::new(
                    DoctorProbeStatus::Ok,
                    format!("{} provider(s)", list.providers.len()),
                ),
                Ok(_) => {
                    failures += 1;
                    DoctorProbe::new(DoctorProbeStatus::Fail, "registry is empty")
                }
                Err(error) => {
                    failures += 1;
                    DoctorProbe::new(DoctorProbeStatus::Fail, error.to_string())
                }
            };

        let token_count = mimir_providers::count::count_local("mimir doctor token counter sanity");
        let token_counter = if token_count > 0 {
            DoctorProbe::new(DoctorProbeStatus::Ok, format!("{token_count} tokens"))
        } else {
            failures += 1;
            DoctorProbe::new(DoctorProbeStatus::Fail, "local counter returned zero")
        };

        let provider = "glm";
        let model = default_model(provider);
        let context_packet = match build_context_packet(ContextPacketBuildRequest {
            task: "Mimir doctor context sanity".to_string(),
            mode: "ask".to_string(),
            provider: Some(provider.to_string()),
            model: Some(model.to_string()),
            workspace_root: Some(self.workspace_root.clone()),
            mimir_root: None,
            edit_targets: Vec::new(),
            persist: false,
        }) {
            Ok(result) if result.packet.packet_hash.len() == 64 => {
                DoctorProbe::new(DoctorProbeStatus::Ok, result.packet.packet_id)
            }
            Ok(result) => {
                failures += 1;
                DoctorProbe::new(
                    DoctorProbeStatus::Fail,
                    format!("unexpected hash length {}", result.packet.packet_hash.len()),
                )
            }
            Err(error) => {
                failures += 1;
                DoctorProbe::new(DoctorProbeStatus::Fail, error.to_string())
            }
        };

        let permission_dir = if self.mimir_root.is_dir() {
            self.mimir_root.clone()
        } else {
            self.workspace_root.clone()
        };
        let probe_path = permission_dir.join(format!(".doctor-write-{}", RunId::generate()));
        let permissions =
            match fs::write(&probe_path, b"ok").and_then(|_| fs::remove_file(&probe_path)) {
                Ok(()) => DoctorProbe::new(DoctorProbeStatus::Ok, permission_dir.to_string()),
                Err(error) => {
                    failures += 1;
                    let _ = fs::remove_file(&probe_path);
                    DoctorProbe::new(DoctorProbeStatus::Fail, error.to_string())
                }
            };

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
        let provider_credentials = if detected_credentials.is_empty() {
            DoctorProbe::new(DoctorProbeStatus::Optional, "none detected")
        } else {
            DoctorProbe::new(
                DoctorProbeStatus::Ok,
                detected_credentials
                    .into_iter()
                    .collect::<Vec<_>>()
                    .join(", "),
            )
        };

        let status = if failures > 0 {
            DoctorStatus::Failed
        } else if warnings > 0 {
            DoctorStatus::Warnings
        } else {
            DoctorStatus::Ok
        };

        Ok(DoctorResult {
            version: request.version,
            run_id_sample: RunId::generate().to_string(),
            config,
            provider_capabilities,
            token_counter,
            context_packet,
            permissions,
            provider_credentials,
            warnings,
            failures,
            status,
            events: Vec::new(),
        })
    }

    /// Run the provider-free context suggestion workflow.
    ///
    /// # Errors
    /// Returns an error when packet construction, artifact writes, or event
    /// writes fail.
    pub fn run_context_suggest(
        &self,
        session: &mut SessionRecord,
        request: ContextSuggestRequest,
    ) -> Result<ContextSuggestResult> {
        let turn_id = format!("turn-{}", Uuid::new_v4());
        let provider = normalized_provider(&request.provider);
        let model = request
            .model
            .unwrap_or_else(|| default_model(&provider).to_string());
        let mut events = Vec::new();

        emit(
            session,
            &mut events,
            SessionEventKind::TurnStarted {
                turn_id: turn_id.clone(),
                command: "context".to_string(),
                task: request.task.clone(),
            },
        )?;
        emit(
            session,
            &mut events,
            SessionEventKind::ContextBuildStarted {
                turn_id: turn_id.clone(),
                provider: provider.clone(),
                model: model.clone(),
            },
        )?;

        match self.build_context_suggest_packet(&request.task, &provider, &model) {
            Ok(result) => {
                for omission in &result.risky_omissions {
                    emit(
                        session,
                        &mut events,
                        SessionEventKind::ContextOmissionRisk {
                            run_id: result.run_id.clone(),
                            path: omission.path.clone(),
                            reason: omission.reason.clone(),
                            risk: omission.risk.clone(),
                        },
                    )?;
                }
                emit(
                    session,
                    &mut events,
                    SessionEventKind::ContextPacketReady {
                        run_id: result.run_id.clone(),
                        packet_id: result.packet_id.clone(),
                        packet_hash: result.packet_hash.clone(),
                        packet_path: result.packet_path.clone(),
                        estimated_input_tokens: result.estimated_input_tokens,
                        guidance_files: result.guidance_files.clone(),
                        likely_files: result.likely_files.clone(),
                    },
                )?;
                emit(
                    session,
                    &mut events,
                    SessionEventKind::ArtifactWritten {
                        run_id: result.run_id.clone(),
                        artifact_kind: "context_packet".to_string(),
                        path: result.packet_path.clone(),
                    },
                )?;
                emit(
                    session,
                    &mut events,
                    SessionEventKind::TurnCompleted {
                        turn_id,
                        summary: format!(
                            "Built context packet {} with {} estimated input tokens",
                            result.packet_id, result.estimated_input_tokens
                        ),
                    },
                )?;
                Ok(ContextSuggestResult { events, ..result })
            }
            Err(error) => {
                emit(
                    session,
                    &mut events,
                    SessionEventKind::TurnFailed {
                        turn_id,
                        error: error.to_string(),
                    },
                )?;
                Err(error)
            }
        }
    }

    /// Build a provider-free context suggestion without session event writes.
    ///
    /// # Errors
    /// Returns an error when packet construction or artifact writes fail.
    pub fn suggest_context(&self, request: ContextSuggestRequest) -> Result<ContextSuggestResult> {
        let provider = normalized_provider(&request.provider);
        let model = request
            .model
            .unwrap_or_else(|| default_model(&provider).to_string());
        self.build_context_suggest_packet(&request.task, &provider, &model)
    }

    fn build_context_suggest_packet(
        &self,
        task: &str,
        provider: &str,
        model: &str,
    ) -> Result<ContextSuggestResult> {
        let built = build_context_packet(ContextPacketBuildRequest {
            task: task.to_string(),
            mode: "ask".to_string(),
            provider: Some(provider.to_string()),
            model: Some(model.to_string()),
            workspace_root: Some(self.workspace_root.clone()),
            mimir_root: Some(self.mimir_root.clone()),
            edit_targets: Vec::new(),
            persist: true,
        })?;
        let packet_path = built
            .packet_path
            .clone()
            .context("persisted context packet missing path")?;

        let likely_files = likely_packet_files(&built.packet);
        let risky_omissions = risky_packet_omissions(&built.packet);
        let checks_loaded = mimir_review::checks::load_checks(&self.workspace_root).len();

        Ok(ContextSuggestResult {
            run_id: built.run_id.to_string(),
            packet_id: built.packet.packet_id.clone(),
            packet_hash: built.packet.packet_hash.clone(),
            packet_path,
            estimated_input_tokens: built.packet.estimated_input_tokens,
            guidance_files: packet_guidance_paths(&built.packet),
            likely_files,
            risky_omissions,
            checks_loaded,
            events: Vec::new(),
        })
    }
}

/// Return the default model for a provider.
#[must_use]
pub fn default_model(provider: &str) -> &'static str {
    match provider {
        "anthropic" => "claude-sonnet-4-20250514",
        "glm" | "zai" => "glm-5.1",
        "openai" | "openai-compatible" => "gpt-4.1",
        _ => "glm-5.1",
    }
}

/// Normalize provider aliases.
#[must_use]
pub fn normalized_provider(provider: &str) -> String {
    match provider {
        "zai" => "glm".to_string(),
        other => other.to_string(),
    }
}

/// Build a context packet through the shared session API.
///
/// When `request.persist` is true and either `mimir_root` or `workspace_root` is
/// available, the packet is written through `RunDir` so replay/share callers can
/// load it from `.mimir/runs/<run_id>/context_packet.json`.
///
/// # Errors
/// Returns an error when packet construction, run-id validation, or artifact
/// persistence fails.
pub fn build_context_packet(
    request: ContextPacketBuildRequest,
) -> Result<ContextPacketBuildResult> {
    let run_id = RunId::generate();
    let trace_start_us = current_unix_micros();
    let task = request.task.clone();
    let mode = request.mode.clone();
    let provider = request.provider.clone();
    let model = request.model.clone();
    let mut builder = ContextBuilder::new()
        .run_id(run_id.clone())
        .task_card(task.clone())
        .mode(mode.clone())
        .edit_targets(request.edit_targets);

    if let Some(provider) = &provider {
        builder = builder.provider(provider);
    }
    if let Some(model) = &model {
        builder = builder.model(model);
    }
    if let Some(workspace_root) = &request.workspace_root {
        builder = builder.repo_root(workspace_root.as_std_path().to_path_buf());
    }

    let packet = builder.build()?;
    let parsed_run_id = RunId::parse(packet.run_id.clone())?;
    if parsed_run_id.as_str() != run_id.as_str() {
        bail!(
            "context builder returned mismatched run id: expected {}, got {}",
            run_id,
            packet.run_id
        );
    }

    let packet_path = if request.persist {
        let mimir_root = request.mimir_root.or_else(|| {
            request
                .workspace_root
                .as_ref()
                .map(|root| root.join(".mimir"))
        });
        if let Some(mimir_root) = mimir_root {
            let run_dir = RunDir::create(&mimir_root, &run_id)?;
            let packet_json = serde_json::to_vec_pretty(&packet)?;
            let packet_path = run_dir.context_packet_path();
            atomic_write(&packet_path, &packet_json)?;
            let trace_end_us = current_unix_micros().max(trace_start_us);
            run_dir.append_trace_span(&mimir_telemetry::TraceSpan {
                schema_version: 1,
                span_id: new_trace_span_id(),
                trace_id: Some(new_trace_id()),
                parent_id: None,
                name: "mimir.context.build".to_string(),
                kind: Some(mimir_telemetry::TraceSpanKind::Internal),
                start_us: trace_start_us,
                end_us: trace_end_us,
                attrs: Some(serde_json::json!({
                    "run_id": packet.run_id.clone(),
                    "packet_id": packet.packet_id.clone(),
                    "provider": provider,
                    "model": model,
                    "status": "ok",
                    "estimated_input_tokens": packet.estimated_input_tokens,
                })),
                events: None,
                status: Some(mimir_telemetry::TraceSpanStatus {
                    code: Some(mimir_telemetry::TraceSpanStatusCode::Ok),
                    message: None,
                }),
            })?;
            Some(packet_path)
        } else {
            None
        }
    } else {
        None
    };

    Ok(ContextPacketBuildResult {
        run_id,
        packet,
        packet_path,
    })
}

fn current_unix_micros() -> u64 {
    Utc::now().timestamp_micros().try_into().unwrap_or(0)
}

fn new_trace_id() -> String {
    Uuid::new_v4().simple().to_string()
}

fn new_trace_span_id() -> String {
    let id = new_trace_id();
    id[..16].to_string()
}

fn blocking_finding_count(findings: &[mimir_review::Finding]) -> usize {
    findings
        .iter()
        .filter(|finding| matches!(finding.severity.as_str(), "error" | "critical"))
        .count()
}

fn write_redacted_json_artifact(path: &Utf8PathBuf, value: &impl Serialize) -> Result<()> {
    let mut json = serde_json::to_value(value)?;
    mimir_security::redact_json_value(&mut json);
    let bytes = serde_json::to_vec_pretty(&json)?;
    if bytes.len() > MAX_PROVIDER_RESPONSE_ARTIFACT_BYTES {
        bail!(
            "provider_response artifact {} exceeds size cap ({} bytes > {} bytes)",
            path,
            bytes.len(),
            MAX_PROVIDER_RESPONSE_ARTIFACT_BYTES
        );
    }
    atomic_write(path, &bytes)?;
    Ok(())
}

fn emit(
    session: &mut SessionRecord,
    events: &mut Vec<SessionEvent>,
    kind: SessionEventKind,
) -> Result<()> {
    let event = session.append_event(kind)?;
    events.push(event);
    Ok(())
}

fn packet_guidance_paths(packet: &ContextPacket) -> Vec<String> {
    packet
        .included
        .iter()
        .filter(|item| item.reason_code == "manifest_reference")
        .map(|item| item.path.clone())
        .collect()
}

fn likely_packet_files(packet: &ContextPacket) -> Vec<String> {
    packet
        .included
        .iter()
        .filter(|item| item.reason_code != "manifest_reference")
        .take(12)
        .map(|item| item.path.clone())
        .collect()
}

fn risky_packet_omissions(packet: &ContextPacket) -> Vec<RiskyOmission> {
    packet
        .omitted_candidates
        .iter()
        .filter(|item| item.risk.is_some())
        .take(12)
        .map(|item| RiskyOmission {
            path: item.path.clone(),
            reason: item.reason_for_omission.clone(),
            risk: item.risk.clone(),
        })
        .collect()
}

fn write_metadata(root: &Utf8Path, metadata: &SessionMetadata) -> Result<()> {
    ensure_session_dir(root)?;
    validate_session_id(&metadata.session_id)?;
    let metadata_path = root.join("session.json");
    ensure_regular_file_if_exists(&metadata_path, "session metadata file")?;
    let mut metadata = metadata.clone();
    metadata.title = mimir_security::redact_secrets(&metadata.title);
    let bytes = serde_json::to_vec_pretty(&metadata)?;
    atomic_write(&metadata_path, &bytes)?;
    Ok(())
}

fn read_metadata_for_session(
    root: &Utf8Path,
    expected_session_id: Option<&SessionId>,
) -> Result<SessionMetadata> {
    ensure_session_dir(root)?;
    let path = root.join("session.json");
    ensure_regular_file(&path, "session metadata file")?;
    let bytes = fs::read(&path).with_context(|| format!("failed to read {path}"))?;
    let mut metadata: SessionMetadata =
        serde_json::from_slice(&bytes).with_context(|| format!("failed to parse {path}"))?;
    validate_session_id(&metadata.session_id)?;
    if let Some(expected_session_id) = expected_session_id {
        if metadata.session_id != *expected_session_id {
            bail!("session metadata id does not match session directory");
        }
    }
    metadata.title = mimir_security::redact_secrets(&metadata.title);
    Ok(metadata)
}

fn read_events(path: &Utf8Path) -> Result<Vec<SessionEvent>> {
    if let Some(root) = path.parent() {
        ensure_session_dir(root)?;
    }
    ensure_regular_file_if_exists(path, "session events file")?;
    let file = match fs::File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error.into()),
    };

    std::io::BufReader::new(file)
        .lines()
        .enumerate()
        .map(|(index, line)| {
            let line = line?;
            serde_json::from_str(&line)
                .with_context(|| format!("failed to parse event line {}", index + 1))
        })
        .collect()
}

fn redact_session_event(event: SessionEvent, workspace_root: &Utf8Path) -> Result<SessionEvent> {
    let mut value = serde_json::to_value(event)?;
    mimir_security::redact_json_value(&mut value);
    sanitize_session_event_paths(&mut value, workspace_root);
    serde_json::from_value(value).context("failed to deserialize redacted session event")
}

fn sanitize_session_event_paths(value: &mut Value, workspace_root: &Utf8Path) {
    match value {
        Value::Object(map) => {
            let keys = map.keys().cloned().collect::<Vec<_>>();
            for key in keys {
                let Some(child) = map.get_mut(&key) else {
                    continue;
                };
                if session_event_path_key(&key) {
                    if let Some(path) = child.as_str() {
                        *child = Value::String(session_event_display_path(workspace_root, path));
                        continue;
                    }
                }
                sanitize_session_event_paths(child, workspace_root);
            }
        }
        Value::Array(items) => {
            for item in items {
                sanitize_session_event_paths(item, workspace_root);
            }
        }
        Value::String(text) => {
            let sanitized = sanitize_session_event_string(workspace_root, text);
            if sanitized != *text {
                *text = sanitized;
            }
        }
        Value::Null | Value::Bool(_) | Value::Number(_) => {}
    }
}

fn session_event_path_key(key: &str) -> bool {
    let lower = key.to_ascii_lowercase();
    lower == "path"
        || lower == "workspace_root"
        || lower == "packet_path"
        || lower == "evidence_path"
        || lower.ends_with("_path")
        || lower.ends_with("_paths")
}

fn session_event_display_path(workspace_root: &Utf8Path, value: &str) -> String {
    let redacted = mimir_security::redact_secrets(value);
    let candidate = Utf8Path::new(&redacted);
    if candidate.is_absolute() {
        if let Ok(relative) = candidate.strip_prefix(workspace_root) {
            return if relative.as_str().is_empty() {
                workspace_display_name(workspace_root)
            } else {
                relative.as_str().replace('\\', "/")
            };
        }
        for prefix in session_event_workspace_prefixes(workspace_root) {
            if redacted == prefix {
                return workspace_display_name(workspace_root);
            }
            for separator in ['/', '\\'] {
                let prefix_with_separator = format!("{prefix}{separator}");
                if let Some(relative) = redacted.strip_prefix(&prefix_with_separator) {
                    return relative.replace('\\', "/");
                }
            }
        }
        return candidate
            .file_name()
            .map(|name| format!("[external]/{name}"))
            .unwrap_or_else(|| "[external path]".to_string());
    }
    sanitize_session_event_string(workspace_root, &redacted)
}

fn sanitize_session_event_string(workspace_root: &Utf8Path, value: &str) -> String {
    let mut sanitized = mimir_security::redact_secrets(value);
    let display_name = workspace_display_name(workspace_root);
    for prefix in session_event_workspace_prefixes(workspace_root) {
        let slash_prefix = format!("{prefix}/");
        let backslash_prefix = format!("{prefix}\\");
        sanitized = sanitized.replace(&slash_prefix, "");
        sanitized = sanitized.replace(&backslash_prefix, "");
        sanitized = sanitized.replace(&prefix, &display_name);
    }
    sanitized
}

fn session_event_workspace_prefixes(workspace_root: &Utf8Path) -> Vec<String> {
    let mut prefixes = vec![workspace_root.as_str().to_string()];
    if let Ok(canonical) = canonical_utf8(workspace_root) {
        let canonical = canonical.as_str().to_string();
        if !prefixes.contains(&canonical) {
            prefixes.push(canonical);
        }
    }
    prefixes
}

fn workspace_display_name(workspace_root: &Utf8Path) -> String {
    workspace_root
        .file_name()
        .filter(|name| !name.is_empty())
        .unwrap_or("workspace")
        .to_string()
}

fn validate_session_id(session_id: &SessionId) -> Result<()> {
    if SessionId::is_valid(session_id.as_str()) {
        Ok(())
    } else {
        bail!("invalid session id")
    }
}

fn ensure_store_root_for_create(root: &Utf8Path) -> Result<()> {
    let mimir_root = mimir_root_for_sessions(root)?;
    fs::create_dir_all(&mimir_root)?;
    ensure_regular_dir(&mimir_root, "mimir root")?;
    fs::create_dir_all(root)?;
    ensure_store_root_for_open(root)
}

fn ensure_store_root_for_open(root: &Utf8Path) -> Result<()> {
    let mimir_root = mimir_root_for_sessions(root)?;
    ensure_regular_dir(&mimir_root, "mimir root")?;
    ensure_regular_dir(root, "sessions root")?;
    ensure_contained(&mimir_root, root, "sessions root escapes mimir root")
}

fn ensure_store_root_for_list(root: &Utf8Path) -> Result<bool> {
    let mimir_root = mimir_root_for_sessions(root)?;
    match fs::symlink_metadata(&mimir_root) {
        Ok(_) => ensure_regular_dir(&mimir_root, "mimir root")?,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error.into()),
    }
    match fs::symlink_metadata(root) {
        Ok(_) => {
            ensure_regular_dir(root, "sessions root")?;
            ensure_contained(&mimir_root, root, "sessions root escapes mimir root")?;
            Ok(true)
        }
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error.into()),
    }
}

fn ensure_session_dir(root: &Utf8Path) -> Result<()> {
    let sessions_root = root
        .parent()
        .ok_or_else(|| anyhow!("session directory has no parent"))?;
    let mimir_root = mimir_root_for_sessions(sessions_root)?;
    ensure_regular_dir(&mimir_root, "mimir root")?;
    ensure_regular_dir(sessions_root, "sessions root")?;
    ensure_regular_dir(root, "session directory")?;
    ensure_contained(
        &mimir_root,
        sessions_root,
        "sessions root escapes mimir root",
    )?;
    ensure_contained(
        sessions_root,
        root,
        "session directory escapes sessions root",
    )
}

fn mimir_root_for_sessions(sessions_root: &Utf8Path) -> Result<Utf8PathBuf> {
    sessions_root
        .parent()
        .map(Utf8Path::to_path_buf)
        .ok_or_else(|| anyhow!("sessions root has no parent"))
}

fn ensure_regular_dir(path: &Utf8Path, label: &str) -> Result<()> {
    let metadata = fs::symlink_metadata(path).map_err(|error| {
        if error.kind() == ErrorKind::NotFound {
            anyhow!("{label} not found")
        } else {
            anyhow!("{error}")
        }
    })?;
    let file_type = metadata.file_type();
    if file_type.is_symlink() {
        bail!("{label} must not be a symlink");
    }
    if !file_type.is_dir() {
        bail!("{label} is not a directory");
    }
    Ok(())
}

fn ensure_regular_file_if_exists(path: &Utf8Path, label: &str) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => ensure_regular_file_type(metadata.file_type(), label),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

fn ensure_regular_file(path: &Utf8Path, label: &str) -> Result<()> {
    let metadata = fs::symlink_metadata(path).map_err(|error| {
        if error.kind() == ErrorKind::NotFound {
            anyhow!("{label} not found")
        } else {
            anyhow!("{error}")
        }
    })?;
    ensure_regular_file_type(metadata.file_type(), label)
}

fn ensure_regular_file_type(file_type: fs::FileType, label: &str) -> Result<()> {
    if file_type.is_symlink() {
        bail!("{label} must not be a symlink");
    }
    if !file_type.is_file() {
        bail!("{label} is not a regular file");
    }
    Ok(())
}

fn ensure_contained(root: &Utf8Path, child: &Utf8Path, message: &str) -> Result<()> {
    let canonical_root = canonical_utf8(root)?;
    let canonical_child = canonical_utf8(child)?;
    if canonical_child.starts_with(&canonical_root) {
        Ok(())
    } else {
        bail!("{message}")
    }
}

fn canonical_utf8(path: &Utf8Path) -> Result<Utf8PathBuf> {
    Utf8PathBuf::from_path_buf(fs::canonicalize(path)?)
        .map_err(|path| anyhow!("path is not UTF-8: {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_workspace() -> (tempfile::TempDir, Utf8PathBuf) {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = Utf8PathBuf::from_path_buf(dir.path().to_path_buf()).expect("utf8 temp path");
        fs::create_dir_all(root.join("src")).expect("src dir");
        fs::write(
            root.join("src/lib.rs"),
            "pub fn session_marker() -> &'static str { \"mimir\" }\n",
        )
        .expect("source file");
        fs::create_dir_all(root.join(".mimir/checks")).expect("checks dir");
        fs::write(
            root.join(".mimir/checks/no-provider-secrets.md"),
            "# No Provider Secrets\n\n- severity: error\n- forbidden: (?i)api[_-]?key\n",
        )
        .expect("check file");
        (dir, root)
    }

    fn workspace_prefixes(root: &Utf8Path) -> Vec<String> {
        let mut prefixes = vec![root.as_str().to_string()];
        if let Ok(canonical) = std::fs::canonicalize(root) {
            let canonical = canonical.to_string_lossy().to_string();
            if !prefixes.contains(&canonical) {
                prefixes.push(canonical);
            }
        }
        prefixes
    }

    fn assert_omits_workspace_prefix(text: &str, root: &Utf8Path) {
        for prefix in workspace_prefixes(root) {
            assert!(
                !text.contains(&prefix),
                "session event leaked workspace prefix {prefix}: {text}"
            );
        }
    }

    fn fixed_session_id() -> SessionId {
        SessionId::parse("sess-550e8400-e29b-41d4-a716-446655440000").expect("fixed session id")
    }

    fn second_session_id() -> SessionId {
        SessionId::parse("sess-6f9619ff-8b86-d011-b42d-00cf4fc964ff").expect("second session id")
    }

    fn session_metadata_for(root: &Utf8Path, session_id: SessionId) -> SessionMetadata {
        let now = Utc::now();
        SessionMetadata {
            schema_version: SESSION_SCHEMA_VERSION,
            session_id,
            title: "Session".to_string(),
            workspace_root: root.to_path_buf(),
            created_at: now,
            updated_at: now,
        }
    }

    fn write_test_metadata(session_root: &Utf8Path, metadata: &SessionMetadata) {
        fs::write(
            session_root.join("session.json"),
            serde_json::to_vec_pretty(metadata).expect("metadata json"),
        )
        .expect("write metadata");
    }

    #[cfg(unix)]
    fn symlink_path(original: impl AsRef<std::path::Path>, link: impl AsRef<std::path::Path>) {
        std::os::unix::fs::symlink(original, link).expect("symlink");
    }

    #[test]
    fn parse_session_command_handles_slash_and_plain_input() {
        assert_eq!(parse_session_command("/help"), SessionCommand::Help);
        assert_eq!(parse_session_command("/HELP"), SessionCommand::Help);
        assert!(builtin_command_registry().iter().any(|command| {
            command.name == "/share" && command.support == CommandSupport::Local && command.enabled
        }));
        assert_eq!(
            parse_session_command("/context find session marker"),
            SessionCommand::Context {
                task: "find session marker".to_string()
            }
        );
        assert_eq!(
            parse_session_command("please inspect the repo"),
            SessionCommand::UserPrompt {
                message: "please inspect the repo".to_string()
            }
        );
    }

    #[test]
    fn session_id_parse_accepts_generated_uuid_format() {
        let generated = SessionId::generate();
        assert!(SessionId::parse(generated.as_str()).is_ok());
        assert!(SessionId::parse("sess-550e8400-e29b-41d4-a716-446655440000").is_ok());
    }

    #[test]
    fn session_id_parse_rejects_unsafe_or_non_generated_values() {
        for value in [
            "",
            "sess-",
            "sess-1234",
            "sess-not-a-uuid",
            "sess-550E8400-E29B-41D4-A716-446655440000",
            "../sess-550e8400-e29b-41d4-a716-446655440000",
            "sess-550e8400-e29b-41d4-a716-446655440000/escape",
            r"sess-550e8400-e29b-41d4-a716-446655440000\escape",
            "sess-550e8400-e29b-41d4-a716-446655440000..",
            "%2e%2e",
            "/tmp/sess-550e8400-e29b-41d4-a716-446655440000",
        ] {
            assert!(SessionId::parse(value).is_err(), "{value}");
        }
    }

    #[test]
    fn init_project_files_seeds_without_overwriting() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = Utf8PathBuf::from_path_buf(dir.path().to_path_buf()).expect("utf8 temp path");

        let created = init_project_files(&root).expect("init project files");
        assert_eq!(
            created,
            vec![
                ".mimir/config.yaml",
                ".mimir/project-rules.md",
                ".mimir/checks/no-provider-secrets.md",
                ".mimir/commands/fast-check.md",
                ".mimir/commands/release-check.md",
            ]
        );

        fs::write(root.join(".mimir/project-rules.md"), "custom rules\n").expect("custom rules");
        let second = init_project_files(&root).expect("second init");
        assert!(second.is_empty());
        assert_eq!(
            fs::read_to_string(root.join(".mimir/project-rules.md")).expect("read rules"),
            "custom rules\n"
        );
    }

    #[test]
    fn session_store_creates_lists_and_reopens_sessions() {
        let (_dir, root) = temp_workspace();
        let store = SessionStore::for_workspace(root.clone());
        let secret = "sk-123456789012345678901234";
        let session = store
            .create(root.clone(), format!("Test session {secret}"))
            .expect("create session");
        let events = session.read_events().expect("read events");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].sequence, 0);
        assert!(matches!(
            events[0].kind,
            SessionEventKind::SessionCreated { .. }
        ));

        let sessions = store.list().expect("list sessions");
        assert_eq!(sessions.len(), 1);
        assert!(!sessions[0].title.contains(secret));
        assert!(sessions[0].title.contains("REDACTED"));
        let metadata_text = fs::read_to_string(session.root().join("session.json")).unwrap();
        assert!(!metadata_text.contains(secret));

        let reopened = store
            .open(&sessions[0].session_id)
            .expect("open created session");
        assert_eq!(reopened.read_events().expect("read events").len(), 1);
    }

    #[test]
    fn session_events_are_redacted_before_persistence() {
        let (_dir, root) = temp_workspace();
        let store = SessionStore::for_workspace(root.clone());
        let mut session = store
            .create(root.clone(), "Secret event")
            .expect("create session");
        let secret = "sk-123456789012345678901234";

        let event = session
            .append_event(SessionEventKind::TurnStarted {
                turn_id: "turn-secret".to_string(),
                command: "context".to_string(),
                task: format!("inspect api_key={secret}"),
            })
            .expect("append event");
        let event_json = serde_json::to_string(&event).expect("serialize event");
        assert!(!event_json.contains(secret));

        let persisted = fs::read_to_string(session.events_path()).expect("read events");
        assert!(!persisted.contains(secret));
        assert!(persisted.contains("[REDACTED]") || persisted.contains("<REDACTED:"));
        assert_omits_workspace_prefix(&persisted, &root);

        let events = session.read_events().expect("read redacted events");
        let events_json = serde_json::to_string(&events).expect("serialize events");
        assert!(!events_json.contains(secret));
        assert_omits_workspace_prefix(&events_json, &root);
    }

    #[test]
    fn session_events_store_display_safe_paths_before_persistence() {
        let (_dir, root) = temp_workspace();
        let store = SessionStore::for_workspace(root.clone());
        let mut session = store
            .create(root.clone(), "Path privacy")
            .expect("create session");
        let packet_path = root.join(".mimir/runs/20260101-120000-abcdef12/context_packet.json");
        let evidence_path = root.join(".mimir/runs/20260101-120000-abcdef13/explore_evidence.json");
        let source_path = root.join("src/lib.rs");
        let secret = "sk-123456789012345678901234";

        session
            .append_event(SessionEventKind::ContextPacketReady {
                run_id: "20260101-120000-abcdef12".to_string(),
                packet_id: "pkt-path-privacy".to_string(),
                packet_hash: "a".repeat(64),
                packet_path: packet_path.clone(),
                estimated_input_tokens: 1200,
                guidance_files: vec![root.join("AGENTS.md").to_string()],
                likely_files: vec![source_path.to_string()],
            })
            .expect("context ready");
        session
            .append_event(SessionEventKind::ArtifactWritten {
                run_id: "20260101-120000-abcdef12".to_string(),
                artifact_kind: "context_packet".to_string(),
                path: packet_path.clone(),
            })
            .expect("artifact event");
        session
            .append_event(SessionEventKind::ExploreCompleted {
                run_id: "20260101-120000-abcdef13".to_string(),
                evidence_path,
                findings_count: 1,
                relevant_paths: vec![source_path.to_string()],
                confidence: 0.9,
            })
            .expect("explore event");
        session
            .append_event(SessionEventKind::DoctorCompleted {
                status: DoctorStatus::Ok,
                warnings: 0,
                failures: 0,
            })
            .expect("doctor event");
        session
            .append_event(SessionEventKind::WorkspaceStatusReady {
                status: serde_json::json!({
                    "workspace_root": root,
                    "recent_runs": [
                        {
                            "path": packet_path.parent().expect("run dir"),
                            "artifact_path": packet_path,
                        }
                    ],
                }),
            })
            .expect("workspace status");
        session
            .append_event(SessionEventKind::TurnFailed {
                turn_id: "turn-path-failure".to_string(),
                error: format!("failed reading {} with api_key={secret}", source_path),
            })
            .expect("failed event");

        let persisted = fs::read_to_string(session.events_path()).expect("read events");
        assert_omits_workspace_prefix(&persisted, &root);
        assert!(!persisted.contains(secret));
        assert!(persisted.contains(".mimir/runs/20260101-120000-abcdef12/context_packet.json"));
        assert!(persisted.contains("src/lib.rs"));

        let events = session.read_events().expect("read events");
        assert!(events.iter().any(|event| {
            matches!(
                &event.kind,
                SessionEventKind::SessionCreated { workspace_root, .. }
                    if !workspace_root.is_absolute()
            )
        }));
        assert!(events.iter().any(|event| {
            matches!(
                &event.kind,
                SessionEventKind::ContextPacketReady { packet_path, .. }
                    if packet_path == Utf8Path::new(".mimir/runs/20260101-120000-abcdef12/context_packet.json")
            )
        }));
        assert!(events.iter().any(|event| {
            matches!(
                &event.kind,
                SessionEventKind::ExploreCompleted { evidence_path, .. }
                    if evidence_path == Utf8Path::new(".mimir/runs/20260101-120000-abcdef13/explore_evidence.json")
            )
        }));
    }

    #[test]
    fn session_store_open_does_not_create_missing_session() {
        let (_dir, root) = temp_workspace();
        let store = SessionStore::for_workspace(root.clone());
        fs::create_dir_all(root.join(".mimir/sessions")).expect("sessions root");
        let session_id = fixed_session_id();
        let missing = root.join(".mimir/sessions").join(session_id.as_str());

        let error = store
            .open(&session_id)
            .expect_err("missing session must fail closed")
            .to_string();

        assert!(error.contains("not found"), "{error}");
        assert!(!missing.exists());
    }

    #[test]
    fn session_store_list_rejects_invalid_session_id_directory() {
        let (_dir, root) = temp_workspace();
        let sessions_root = root.join(".mimir/sessions");
        fs::create_dir_all(sessions_root.join("sess-1234")).expect("invalid session dir");
        let store = SessionStore::for_workspace(root);

        let error = store
            .list()
            .expect_err("invalid session directory must fail closed")
            .to_string();

        assert!(error.contains("invalid session id"), "{error}");
    }

    #[test]
    fn session_store_open_and_list_reject_metadata_session_id_mismatch() {
        let (_dir, root) = temp_workspace();
        let sessions_root = root.join(".mimir/sessions");
        let session_id = fixed_session_id();
        let session_root = sessions_root.join(session_id.as_str());
        fs::create_dir_all(&session_root).expect("session root");
        write_test_metadata(
            &session_root,
            &session_metadata_for(&root, second_session_id()),
        );
        fs::write(session_root.join("events.jsonl"), "").expect("events");
        let store = SessionStore::for_workspace(root);

        let open_error = store
            .open(&session_id)
            .expect_err("mismatched metadata must not open")
            .to_string();
        assert!(open_error.contains("does not match"), "{open_error}");

        let list_error = store
            .list()
            .expect_err("mismatched metadata must not list")
            .to_string();
        assert!(list_error.contains("does not match"), "{list_error}");
    }

    #[test]
    fn session_store_list_surfaces_metadata_parse_errors() {
        let (_dir, root) = temp_workspace();
        let sessions_root = root.join(".mimir/sessions");
        let session_id = fixed_session_id();
        let session_root = sessions_root.join(session_id.as_str());
        fs::create_dir_all(&session_root).expect("session root");
        fs::write(session_root.join("session.json"), "{not valid json").expect("bad metadata");
        let store = SessionStore::for_workspace(root);

        let error = store
            .list()
            .expect_err("malformed metadata must fail closed")
            .to_string();

        assert!(error.contains("failed to parse"), "{error}");
    }

    #[cfg(unix)]
    #[test]
    fn session_store_create_rejects_symlinked_sessions_root() {
        let (_dir, root) = temp_workspace();
        let outside = tempfile::tempdir().expect("outside tempdir");
        fs::create_dir_all(root.join(".mimir")).expect("mimir root");
        symlink_path(outside.path(), root.join(".mimir/sessions"));
        let store = SessionStore::for_workspace(root.clone());

        let error = store
            .create(root, "Symlinked sessions root")
            .expect_err("symlinked sessions root must fail closed")
            .to_string();

        assert!(
            error.contains("sessions root must not be a symlink"),
            "{error}"
        );
        assert_eq!(
            fs::read_dir(outside.path())
                .expect("outside listing")
                .count(),
            0
        );
    }

    #[cfg(unix)]
    #[test]
    fn session_store_open_and_list_reject_symlinked_session_dir_escape() {
        let (_dir, root) = temp_workspace();
        let outside = tempfile::tempdir().expect("outside tempdir");
        let sessions_root = root.join(".mimir/sessions");
        fs::create_dir_all(&sessions_root).expect("sessions root");
        let session_id = fixed_session_id();
        let outside_session =
            Utf8PathBuf::from_path_buf(outside.path().join("escaped")).expect("utf8 outside");
        fs::create_dir_all(&outside_session).expect("outside session");
        write_test_metadata(
            &outside_session,
            &session_metadata_for(&root, session_id.clone()),
        );
        fs::write(outside_session.join("events.jsonl"), "").expect("outside events");
        symlink_path(&outside_session, sessions_root.join(session_id.as_str()));
        let store = SessionStore::for_workspace(root);

        let open_error = store
            .open(&session_id)
            .expect_err("symlinked session dir must not open")
            .to_string();
        assert!(
            open_error.contains("session directory must not be a symlink"),
            "{open_error}"
        );

        let list_error = store
            .list()
            .expect_err("symlinked session dir must not list")
            .to_string();
        assert!(
            list_error.contains("session directory must not be a symlink"),
            "{list_error}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn session_store_open_rejects_symlinked_session_json() {
        let (_dir, root) = temp_workspace();
        let store = SessionStore::for_workspace(root.clone());
        let session = store
            .create(root.clone(), "Metadata symlink")
            .expect("create session");
        let outside = root.join("outside-session.json");
        fs::write(&outside, "{}").expect("outside metadata target");
        fs::remove_file(session.root().join("session.json")).expect("remove metadata");
        symlink_path(&outside, session.root().join("session.json"));

        let error = store
            .open(&session.metadata().session_id)
            .expect_err("symlinked metadata must not open")
            .to_string();

        assert!(
            error.contains("session metadata file must not be a symlink"),
            "{error}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn session_record_read_events_rejects_symlinked_events_jsonl() {
        let (_dir, root) = temp_workspace();
        let store = SessionStore::for_workspace(root.clone());
        let session = store
            .create(root.clone(), "Events symlink")
            .expect("create session");
        let outside = root.join("outside-events.jsonl");
        fs::write(&outside, "").expect("outside event target");
        fs::remove_file(session.events_path()).expect("remove events");
        symlink_path(&outside, session.events_path());

        let error = session
            .read_events()
            .expect_err("symlinked events must not read")
            .to_string();

        assert!(
            error.contains("session events file must not be a symlink"),
            "{error}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn session_record_append_event_rejects_symlinked_events_jsonl_and_preserves_target() {
        let (_dir, root) = temp_workspace();
        let store = SessionStore::for_workspace(root.clone());
        let mut session = store
            .create(root.clone(), "Append symlink")
            .expect("create session");
        let outside = root.join("outside-events.jsonl");
        fs::write(&outside, "outside\n").expect("outside event target");
        fs::remove_file(session.events_path()).expect("remove events");
        symlink_path(&outside, session.events_path());

        let error = session
            .append_event(SessionEventKind::TurnStarted {
                turn_id: "turn-symlink".to_string(),
                command: "context".to_string(),
                task: "must fail closed".to_string(),
            })
            .expect_err("symlinked events must not append")
            .to_string();

        assert!(
            error.contains("session events file must not be a symlink"),
            "{error}"
        );
        assert_eq!(
            fs::read_to_string(outside).expect("outside target"),
            "outside\n"
        );
    }

    #[test]
    fn shared_context_packet_builder_persists_replayable_packet() {
        let (_dir, root) = temp_workspace();
        let result = build_context_packet(ContextPacketBuildRequest {
            task: "find session_marker".to_string(),
            mode: "ask".to_string(),
            provider: Some("glm".to_string()),
            model: Some("glm-5.1".to_string()),
            workspace_root: Some(root.clone()),
            mimir_root: None,
            edit_targets: Vec::new(),
            persist: true,
        })
        .expect("build context packet");

        assert!(RunId::parse(result.packet.run_id.clone()).is_ok());
        assert_eq!(result.packet.run_id, result.run_id.to_string());
        assert_eq!(result.packet.provider, "glm");
        assert_eq!(result.packet.model, "glm-5.1");
        assert_eq!(
            result.packet.budget_ledger_ref,
            format!(".mimir/runs/{}/budget_ledger.json", result.run_id)
        );
        let packet_path = result.packet_path.expect("persisted packet path");
        assert_eq!(
            packet_path,
            root.join(".mimir/runs")
                .join(result.run_id.as_str())
                .join("context_packet.json")
        );
        let persisted: ContextPacket =
            serde_json::from_slice(&fs::read(&packet_path).expect("read persisted packet"))
                .expect("parse persisted packet");
        assert_eq!(persisted.packet_hash, result.packet.packet_hash);
        assert_eq!(persisted.run_id, result.packet.run_id);
        let trace_path = root
            .join(".mimir/runs")
            .join(result.run_id.as_str())
            .join("trace.spans.jsonl");
        let trace = fs::read_to_string(trace_path).expect("read trace spans");
        assert!(trace.contains("\"schema_version\":1"));
        assert!(trace.contains("mimir.context.build"));
    }

    #[test]
    fn context_suggest_turn_builds_packet_and_events_without_provider_call() {
        let (_dir, root) = temp_workspace();
        let store = SessionStore::for_workspace(root.clone());
        let mut session = store
            .create(root.clone(), "Context turn")
            .expect("create session");
        let runner = TurnRunner::for_workspace(root);

        let result = runner
            .run_context_suggest(
                &mut session,
                ContextSuggestRequest {
                    task: "find session_marker".to_string(),
                    provider: "glm".to_string(),
                    model: None,
                },
            )
            .expect("context suggest");

        assert!(result.packet_path.exists());
        assert!(result.packet_id.starts_with("pkt-"));
        assert_eq!(result.checks_loaded, 1);
        assert!(result
            .events
            .iter()
            .any(|event| matches!(event.kind, SessionEventKind::ContextPacketReady { .. })));
        assert!(session
            .read_events()
            .expect("session events")
            .iter()
            .any(|event| matches!(event.kind, SessionEventKind::TurnCompleted { .. })));
    }

    #[test]
    fn check_turn_reports_findings_and_events() {
        let (_dir, root) = temp_workspace();
        fs::write(
            root.join(".mimir/checks/no-todos.md"),
            "# No TODOs\n\n- severity: error\n- forbidden: TODO\n",
        )
        .expect("todo check");
        fs::write(root.join("src/lib.rs"), "// TODO: tighten this\n").expect("source file");
        let store = SessionStore::for_workspace(root.clone());
        let mut session = store
            .create(root.clone(), "Check turn")
            .expect("create session");
        let runner = TurnRunner::for_workspace(root);

        let result = runner.run_check(&mut session).expect("check turn");

        assert_eq!(result.checks_loaded, 2);
        assert_eq!(result.blocking_findings, 1);
        assert!(!result.passed);
        assert!(result
            .events
            .iter()
            .any(|event| matches!(event.kind, SessionEventKind::CheckCompleted { .. })));
        assert!(session
            .read_events()
            .expect("session events")
            .iter()
            .any(|event| matches!(event.kind, SessionEventKind::TurnCompleted { .. })));
    }

    #[test]
    fn explore_turn_writes_redacted_evidence_and_events() {
        let (_dir, root) = temp_workspace();
        let store = SessionStore::for_workspace(root.clone());
        let mut session = store
            .create(root.clone(), "Explore turn")
            .expect("create session");
        let runner = TurnRunner::for_workspace(root);

        let result = runner
            .run_explore(
                &mut session,
                ExploreRequest {
                    query: "where is session_marker defined".to_string(),
                },
            )
            .expect("explore turn");

        assert!(result.evidence_path.exists());
        assert_eq!(
            result.evidence.parent_run_id.as_deref(),
            Some(result.run_id.as_str())
        );
        assert!(result
            .evidence
            .relevant_paths
            .iter()
            .any(|path| path == "src/lib.rs"));
        assert!(result
            .events
            .iter()
            .any(|event| matches!(event.kind, SessionEventKind::ArtifactWritten { .. })));
        assert!(result
            .events
            .iter()
            .any(|event| matches!(event.kind, SessionEventKind::ExploreCompleted { .. })));
    }

    #[test]
    fn doctor_turn_reports_status_and_events_without_provider_call() {
        let (_dir, root) = temp_workspace();
        fs::write(
            root.join(".mimir/config.yaml"),
            "version: 1\ncap_tokens: 64000\n",
        )
        .expect("config");
        let store = SessionStore::for_workspace(root.clone());
        let mut session = store
            .create(root.clone(), "Doctor turn")
            .expect("create session");
        let runner = TurnRunner::for_workspace(root);

        let result = runner
            .run_doctor(
                &mut session,
                DoctorRequest {
                    version: "test-version".to_string(),
                },
            )
            .expect("doctor turn");

        assert_eq!(result.version, "test-version");
        assert_eq!(result.config.status, DoctorProbeStatus::Ok);
        assert_eq!(result.failures, 0);
        assert_eq!(result.status, DoctorStatus::Ok);
        assert!(result
            .events
            .iter()
            .any(|event| matches!(event.kind, SessionEventKind::DoctorCompleted { .. })));
    }
}
