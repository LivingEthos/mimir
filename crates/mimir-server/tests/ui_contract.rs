//! Focused UI/API contract tests for Mimir Studio DTOs.

#![allow(dead_code)]

use futures::StreamExt as _;
use mimir_runs::RunId;
use mimir_server::{run_ui_server, UiServerConfig};
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{collections::BTreeSet, time::Duration};
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::header::{ORIGIN as WS_ORIGIN, SEC_WEBSOCKET_PROTOCOL};

const MIMIR_STUDIO_WS_PROTOCOL: &str = "mimir.studio.v1";
const TOKEN: &str = "test-token";
const SECRET: &str = "sk-123456789012345678901234";

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SessionMetadataDto {
    schema_version: u32,
    session_id: String,
    title: String,
    workspace_name: String,
    created_at: String,
    updated_at: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SessionEventDto {
    schema_version: u32,
    event_id: String,
    session_id: String,
    sequence: u64,
    timestamp: String,
    #[serde(rename = "type")]
    event_type: String,
    payload: Value,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SessionCreatedPayloadDto {
    title: String,
    workspace_name: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct TurnStartedPayloadDto {
    turn_id: String,
    command: String,
    task: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ContextBuildStartedPayloadDto {
    turn_id: String,
    provider: String,
    model: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ContextPacketReadyPayloadDto {
    run_id: String,
    packet_id: String,
    packet_hash: String,
    packet_path: String,
    estimated_input_tokens: u32,
    guidance_files: Vec<String>,
    likely_files: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ContextOmissionRiskPayloadDto {
    run_id: String,
    path: String,
    reason: String,
    risk: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ArtifactWrittenPayloadDto {
    run_id: String,
    artifact_kind: String,
    path: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CheckCompletedPayloadDto {
    checks_loaded: usize,
    findings_count: usize,
    blocking_findings: usize,
    passed: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ExploreCompletedPayloadDto {
    run_id: String,
    evidence_path: String,
    findings_count: usize,
    relevant_paths: Vec<String>,
    confidence: f64,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct DoctorCompletedPayloadDto {
    status: String,
    warnings: u32,
    failures: u32,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct WorkspaceStatusReadyPayloadDto {
    status: Value,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ArtifactRefDto {
    run_id: String,
    artifact_kind: String,
    path: String,
    sha256: Option<String>,
    redacted: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ApprovalRequestDto {
    approval_id: String,
    session_id: String,
    turn_id: String,
    tool_name: String,
    reason: String,
    path: Option<String>,
    command: Option<String>,
    artifact: Option<ArtifactRefDto>,
    requested_at: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ApprovalDecisionDto {
    approval_id: String,
    action: String,
    decided_at: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ApprovalRequestedPayloadDto {
    request: ApprovalRequestDto,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ApprovalResolvedPayloadDto {
    decision: ApprovalDecisionDto,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct TurnCompletedPayloadDto {
    turn_id: String,
    summary: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct TurnFailedPayloadDto {
    turn_id: String,
    error: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SessionCreateResponseDto {
    metadata: SessionMetadataDto,
    events: Vec<SessionEventDto>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SessionLoadResponseDto {
    metadata: SessionMetadataDto,
    events: Vec<SessionEventDto>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct WorkspaceStatusDto {
    workspace_name: String,
    git: GitStatusDto,
    mimir: MimirStatusDto,
    providers: Vec<ProviderStatusDto>,
    commands: Vec<CommandMetadataDto>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct GitStatusDto {
    is_repo: bool,
    branch: Option<String>,
    dirty: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct MimirStatusDto {
    initialized: bool,
    config_present: bool,
    checks_loaded: usize,
    sessions_count: usize,
    runs_count: usize,
    recent_runs: Vec<RunSummaryDto>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProviderStatusDto {
    provider: String,
    models_count: usize,
    credential_detected: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CommandMetadataDto {
    name: String,
    usage: String,
    summary: String,
    support: String,
    takes_input: bool,
    enabled: bool,
    disabled_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RunSummaryDto {
    run_id: String,
    path: String,
    artifact_count: usize,
    has_context_packet: bool,
    trace_status: TraceStatusDto,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct TraceStatusDto {
    state: String,
    redacted: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ArtifactListResponseDto {
    run_id: String,
    trace_status: TraceStatusDto,
    artifacts: Vec<ArtifactSummaryDto>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ArtifactSummaryDto {
    name: String,
    path: String,
    size_bytes: u64,
    sha256: String,
    checksum_basis: String,
    redacted: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ArtifactContentResponseDto {
    name: String,
    path: String,
    content_type: String,
    sha256: String,
    checksum_basis: String,
    redacted: bool,
    content: Value,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct TraceSpanPreviewDto {
    schema_version: u32,
    span_id: String,
    trace_id: Option<String>,
    parent_id: Option<String>,
    name: String,
    kind: Option<String>,
    start_us: u64,
    end_us: u64,
    attrs: Option<Value>,
    events: Option<Vec<TraceSpanEventPreviewDto>>,
    status: Option<TraceSpanStatusPreviewDto>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct TraceSpanEventPreviewDto {
    at_us: u64,
    name: String,
    attrs: Option<Value>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct TraceSpanStatusPreviewDto {
    code: Option<String>,
    message: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ReplayPreviewResponseDto {
    run_id: String,
    packet_id: String,
    packet_hash: String,
    packet_path: String,
    source: String,
    provider_request_sha256: String,
    user_prompt_sha256: Option<String>,
    redacted: bool,
    request: Value,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct SharePreviewResponseDto {
    run_id: String,
    packet_id: String,
    packet_hash: String,
    packet_path: String,
    bundle_sha256: String,
    redacted: bool,
    bundle: SharedPacketBundleDto,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct SharedPacketBundleDto {
    schema_version: u32,
    kind: String,
    exported_at: String,
    run_id: String,
    packet_hash: String,
    provider: String,
    model: String,
    packet: Value,
    replay: SharedPacketReplayDto,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct SharedPacketReplayDto {
    provider_request_sha256: String,
    user_prompt_sha256: Option<String>,
    provider_request_redacted: Value,
}

async fn spawn_ui_server(
    dir: &tempfile::TempDir,
) -> (String, tokio::task::JoinHandle<anyhow::Result<()>>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let workspace_root =
        camino::Utf8PathBuf::from_path_buf(std::fs::canonicalize(dir.path()).unwrap())
            .expect("utf8 temp dir");
    let config = UiServerConfig::new(workspace_root, Some(TOKEN.to_string()));
    let handle = tokio::spawn(async move { run_ui_server(listener, config).await });
    (format!("http://{addr}"), handle)
}

fn websocket_event_request(
    base_url: &str,
    session_id: &str,
    after: u64,
) -> tokio_tungstenite::tungstenite::http::Request<()> {
    let ws_url = format!(
        "{}/v1/sessions/{session_id}/events?after={after}",
        base_url.replace("http://", "ws://")
    );
    let mut request = ws_url.into_client_request().unwrap();
    request.headers_mut().insert(
        SEC_WEBSOCKET_PROTOCOL,
        format!(
            "{MIMIR_STUDIO_WS_PROTOCOL}, {}",
            websocket_token_protocol(TOKEN)
        )
        .parse()
        .unwrap(),
    );
    request
        .headers_mut()
        .insert(WS_ORIGIN, base_url.parse().unwrap());
    request
}

fn websocket_token_protocol(token: &str) -> String {
    let encoded = token
        .as_bytes()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    format!("mimir-token.{encoded}")
}

fn workspace_prefixes(dir: &tempfile::TempDir) -> Vec<String> {
    let mut prefixes = vec![dir.path().to_string_lossy().to_string()];
    if let Ok(canonical) = std::fs::canonicalize(dir.path()) {
        let canonical = canonical.to_string_lossy().to_string();
        if !prefixes.contains(&canonical) {
            prefixes.push(canonical);
        }
    }
    for prefix in prefixes.clone() {
        if let Some(without_private) = prefix.strip_prefix("/private") {
            let without_private = without_private.to_string();
            if !prefixes.contains(&without_private) {
                prefixes.push(without_private);
            }
        } else if prefix.starts_with("/var/") {
            let with_private = format!("/private{prefix}");
            if !prefixes.contains(&with_private) {
                prefixes.push(with_private);
            }
        }
    }
    prefixes
}

fn assert_text_omits_workspace_prefix(text: &str, dir: &tempfile::TempDir) {
    for prefix in workspace_prefixes(dir) {
        assert!(
            !text.contains(&prefix),
            "UI payload leaked workspace prefix {prefix}: {text}"
        );
    }
}

fn assert_json_omits_workspace_prefix(value: &Value, dir: &tempfile::TempDir) {
    assert_text_omits_workspace_prefix(&serde_json::to_string(value).unwrap(), dir);
}

fn assert_json_has_no_key(value: &Value, key: &str) {
    match value {
        Value::Object(map) => {
            assert!(!map.contains_key(key), "unexpected key {key} in {value}");
            for child in map.values() {
                assert_json_has_no_key(child, key);
            }
        }
        Value::Array(items) => {
            for item in items {
                assert_json_has_no_key(item, key);
            }
        }
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {}
    }
}

fn assert_public_text_is_safe(text: &str, dir: &tempfile::TempDir) {
    assert!(!text.contains(SECRET), "{text}");
    assert!(!text.contains(TOKEN), "{text}");
    assert_text_omits_workspace_prefix(text, dir);
}

fn assert_public_text_omits_needles(text: &str, dir: &tempfile::TempDir, needles: &[String]) {
    assert_public_text_is_safe(text, dir);
    for (index, needle) in needles.iter().enumerate() {
        assert!(
            !text.contains(needle),
            "public response leaked private preview context #{index}: {text}"
        );
    }
}

fn assert_session_event_payload_contract(event: &SessionEventDto) {
    session_event_payload_contract_result(event).unwrap_or_else(|error| {
        panic!("{} payload failed DTO contract: {error}", event.event_type)
    });
}

fn session_event_payload_contract_result(event: &SessionEventDto) -> Result<(), String> {
    match event.event_type.as_str() {
        "session.created" => parse_payload::<SessionCreatedPayloadDto>(event),
        "turn.started" => parse_payload::<TurnStartedPayloadDto>(event),
        "context.build.started" => parse_payload::<ContextBuildStartedPayloadDto>(event),
        "context.packet.ready" => parse_payload::<ContextPacketReadyPayloadDto>(event),
        "context.omission.risk" => parse_payload::<ContextOmissionRiskPayloadDto>(event),
        "artifact.written" => parse_payload::<ArtifactWrittenPayloadDto>(event),
        "check.completed" => parse_payload::<CheckCompletedPayloadDto>(event),
        "explore.completed" => parse_payload::<ExploreCompletedPayloadDto>(event),
        "doctor.completed" => parse_payload::<DoctorCompletedPayloadDto>(event),
        "workspace.status.ready" => parse_payload::<WorkspaceStatusReadyPayloadDto>(event),
        "approval.requested" => parse_payload::<ApprovalRequestedPayloadDto>(event),
        "approval.resolved" => parse_payload::<ApprovalResolvedPayloadDto>(event),
        "turn.completed" => parse_payload::<TurnCompletedPayloadDto>(event),
        "turn.failed" => parse_payload::<TurnFailedPayloadDto>(event),
        _ => Err(format!("unhandled session event type {}", event.event_type)),
    }
}

fn parse_payload<T>(event: &SessionEventDto) -> Result<(), String>
where
    T: for<'de> Deserialize<'de>,
{
    serde_json::from_value::<T>(event.payload.clone())
        .map(|_| ())
        .map_err(|error| error.to_string())
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

fn write_replayable_run(dir: &tempfile::TempDir, run_id: &str) -> serde_json::Value {
    std::fs::create_dir_all(dir.path().join("src")).unwrap();
    std::fs::write(
        dir.path().join("src/replay_target.rs"),
        "pub fn replay_target_unique() -> &'static str { \"stable\" }\n",
    )
    .unwrap();
    let packet = mimir_context::ContextBuilder::new()
        .run_id(RunId(run_id.to_string()))
        .task_card("replay_target_unique")
        .provider("glm")
        .model("glm-5.1")
        .repo_root(dir.path())
        .build()
        .unwrap();
    assert!(
        packet
            .included
            .iter()
            .any(|item| item.path == "src/replay_target.rs"),
        "test packet should include replay_target.rs"
    );
    let run_dir = dir.path().join(".mimir/runs").join(run_id);
    std::fs::create_dir_all(&run_dir).unwrap();
    std::fs::write(
        run_dir.join("context_packet.json"),
        serde_json::to_vec_pretty(&packet).unwrap(),
    )
    .unwrap();
    let request = serde_json::json!({
        "model": "glm-5.1",
        "messages": [
            {"role": "user", "content": "redacted replay request for replay_target_unique"}
        ],
        "stream": false
    });
    std::fs::write(
        run_dir.join("provider_request.redacted.json"),
        serde_json::to_vec_pretty(&request).unwrap(),
    )
    .unwrap();
    request
}

#[test]
fn session_event_payload_contract_rejects_unknown_or_missing_fields() {
    let malformed_context_ready = SessionEventDto {
        schema_version: 1,
        event_id: "evt-contract".to_string(),
        session_id: "sess-contract".to_string(),
        sequence: 1,
        timestamp: "2026-01-01T00:00:00Z".to_string(),
        event_type: "context.packet.ready".to_string(),
        payload: serde_json::json!({
            "run_id": "20260101-120000-abcdef31",
            "packet_id": "pkt-contract",
            "packet_hash": "a".repeat(64),
            "packet_path": ".mimir/runs/20260101-120000-abcdef31/context_packet.json",
            "estimated_input_tokens": 1234,
            "guidance_files": ["AGENTS.md"],
            "likely_files": ["src/lib.rs"],
            "unexpected": true
        }),
    };
    assert!(session_event_payload_contract_result(&malformed_context_ready).is_err());

    let malformed_turn_failed = SessionEventDto {
        schema_version: 1,
        event_id: "evt-failed".to_string(),
        session_id: "sess-contract".to_string(),
        sequence: 2,
        timestamp: "2026-01-01T00:00:01Z".to_string(),
        event_type: "turn.failed".to_string(),
        payload: serde_json::json!({"error": "missing turn id"}),
    };
    assert!(session_event_payload_contract_result(&malformed_turn_failed).is_err());

    let malformed_approval_requested = SessionEventDto {
        schema_version: 1,
        event_id: "evt-approval".to_string(),
        session_id: "sess-contract".to_string(),
        sequence: 3,
        timestamp: "2026-01-01T00:00:02Z".to_string(),
        event_type: "approval.requested".to_string(),
        payload: serde_json::json!({
            "request": {
                "approval_id": "approval-contract",
                "session_id": "sess-contract"
            }
        }),
    };
    assert!(session_event_payload_contract_result(&malformed_approval_requested).is_err());
}

#[tokio::test]
async fn session_http_ws_and_persisted_dtos_are_display_safe() {
    let dir = tempfile::TempDir::new().unwrap();
    std::fs::create_dir_all(dir.path().join("src")).unwrap();
    std::fs::write(dir.path().join("src/lib.rs"), "pub fn ui_contract() {}\n").unwrap();
    let (base_url, handle) = spawn_ui_server(&dir).await;
    let client = reqwest::Client::new();

    let created_value: Value = client
        .post(format!("{base_url}/v1/sessions"))
        .bearer_auth(TOKEN)
        .json(&serde_json::json!({"title": format!("Secret title {SECRET}")}))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    let created: SessionCreateResponseDto = serde_json::from_value(created_value.clone()).unwrap();
    assert_eq!(created.metadata.schema_version, 1);
    assert!(created.metadata.session_id.starts_with("sess-"));
    assert!(!created.metadata.title.contains(SECRET));
    assert!(created.metadata.title.contains("REDACTED"));
    assert!(!created.metadata.workspace_name.is_empty());
    assert_eq!(created.events.len(), 1);
    assert_eq!(created.events[0].event_type, "session.created");
    assert_session_event_payload_contract(&created.events[0]);
    assert_json_has_no_key(&created_value, "workspace_root");
    assert_json_omits_workspace_prefix(&created_value, &dir);

    let session_id = mimir_session::SessionId::parse(created.metadata.session_id.clone()).unwrap();
    let store_root =
        camino::Utf8PathBuf::from_path_buf(dir.path().to_path_buf()).expect("utf8 temp dir");
    let mut session = mimir_session::SessionStore::for_workspace(store_root)
        .open(&session_id)
        .unwrap();
    let workspace_root = session.metadata().workspace_root.clone();
    let run_id = "20260101-120000-abcdef31";
    let packet_path = workspace_root.join(format!(".mimir/runs/{run_id}/context_packet.json"));
    let evidence_path = workspace_root.join(format!(".mimir/runs/{run_id}/explore_evidence.json"));
    let source_path = workspace_root.join("src/lib.rs");
    session
        .append_event(mimir_session::SessionEventKind::TurnStarted {
            turn_id: "turn-contract".to_string(),
            command: "context".to_string(),
            task: format!("inspect {source_path} with api_key={SECRET}"),
        })
        .unwrap();
    session
        .append_event(mimir_session::SessionEventKind::ContextBuildStarted {
            turn_id: "turn-contract".to_string(),
            provider: "glm".to_string(),
            model: "glm-5.1".to_string(),
        })
        .unwrap();
    session
        .append_event(mimir_session::SessionEventKind::ContextPacketReady {
            run_id: run_id.to_string(),
            packet_id: "pkt-contract".to_string(),
            packet_hash: "a".repeat(64),
            packet_path: packet_path.clone(),
            estimated_input_tokens: 1234,
            guidance_files: vec![workspace_root.join("AGENTS.md").to_string()],
            likely_files: vec![source_path.to_string()],
        })
        .unwrap();
    session
        .append_event(mimir_session::SessionEventKind::ContextOmissionRisk {
            run_id: run_id.to_string(),
            path: source_path.to_string(),
            reason: format!("risk near {source_path} with token={SECRET}"),
            risk: Some("test_missing".to_string()),
        })
        .unwrap();
    session
        .append_event(mimir_session::SessionEventKind::ArtifactWritten {
            run_id: run_id.to_string(),
            artifact_kind: "context_packet".to_string(),
            path: packet_path.clone(),
        })
        .unwrap();
    let approval_request: mimir_session::ApprovalRequest =
        serde_json::from_value(serde_json::json!({
            "approval_id": "approval-contract",
            "session_id": created.metadata.session_id,
            "turn_id": "turn-contract",
            "tool_name": "run_command",
            "reason": format!("needs approval near {source_path} with TOKEN={SECRET}"),
            "path": source_path,
            "command": format!("cat {source_path}"),
            "artifact": {
                "run_id": run_id,
                "artifact_kind": "context_packet",
                "path": packet_path,
                "sha256": null,
                "redacted": true
            },
            "requested_at": "2026-01-01T00:00:02Z"
        }))
        .unwrap();
    session
        .append_event(mimir_session::SessionEventKind::ApprovalRequested {
            request: approval_request,
        })
        .unwrap();
    let approval_decision: mimir_session::ApprovalDecision =
        serde_json::from_value(serde_json::json!({
            "approval_id": "approval-contract",
            "action": "allow_once",
            "decided_at": "2026-01-01T00:00:03Z"
        }))
        .unwrap();
    session
        .append_event(mimir_session::SessionEventKind::ApprovalResolved {
            decision: approval_decision,
        })
        .unwrap();
    session
        .append_event(mimir_session::SessionEventKind::ExploreCompleted {
            run_id: run_id.to_string(),
            evidence_path,
            findings_count: 1,
            relevant_paths: vec![source_path.to_string()],
            confidence: 0.9,
        })
        .unwrap();
    session
        .append_event(mimir_session::SessionEventKind::WorkspaceStatusReady {
            status: serde_json::json!({
                "workspace_root": workspace_root,
                "source_path": source_path,
                "nested": {"artifact_path": packet_path},
                "message": format!("checked {source_path} with TOKEN={SECRET}"),
            }),
        })
        .unwrap();
    session
        .append_event(mimir_session::SessionEventKind::CheckCompleted {
            checks_loaded: 2,
            findings_count: 1,
            blocking_findings: 0,
            passed: true,
        })
        .unwrap();
    session
        .append_event(mimir_session::SessionEventKind::DoctorCompleted {
            status: mimir_session::DoctorStatus::Warnings,
            warnings: 1,
            failures: 0,
        })
        .unwrap();
    session
        .append_event(mimir_session::SessionEventKind::TurnCompleted {
            turn_id: "turn-contract".to_string(),
            summary: "contract turn completed".to_string(),
        })
        .unwrap();
    session
        .append_event(mimir_session::SessionEventKind::TurnFailed {
            turn_id: "turn-contract".to_string(),
            error: format!("failed at {source_path} with TOKEN={SECRET}"),
        })
        .unwrap();

    let persisted = std::fs::read_to_string(
        dir.path()
            .join(".mimir/sessions")
            .join(created.metadata.session_id.as_str())
            .join("events.jsonl"),
    )
    .unwrap();
    assert_public_text_is_safe(&persisted, &dir);
    assert!(persisted.contains("src/lib.rs"));
    assert!(persisted.contains(".mimir/runs/20260101-120000-abcdef31/context_packet.json"));

    let listed_value: Value = client
        .get(format!("{base_url}/v1/sessions"))
        .bearer_auth(TOKEN)
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    let listed: Vec<SessionMetadataDto> = serde_json::from_value(listed_value.clone()).unwrap();
    assert_eq!(listed[0].session_id, created.metadata.session_id);
    assert_json_has_no_key(&listed_value, "workspace_root");
    assert_json_omits_workspace_prefix(&listed_value, &dir);

    let loaded_value: Value = client
        .get(format!(
            "{base_url}/v1/sessions/{}",
            created.metadata.session_id
        ))
        .bearer_auth(TOKEN)
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    let loaded: SessionLoadResponseDto = serde_json::from_value(loaded_value.clone()).unwrap();
    assert_eq!(loaded.metadata.session_id, created.metadata.session_id);
    assert!(loaded.events.len() >= 14);
    for event in &loaded.events {
        assert_session_event_payload_contract(event);
    }
    let loaded_event_types = loaded
        .events
        .iter()
        .map(|event| event.event_type.as_str())
        .collect::<BTreeSet<_>>();
    let expected_event_types = [
        "session.created",
        "turn.started",
        "context.build.started",
        "context.packet.ready",
        "context.omission.risk",
        "artifact.written",
        "check.completed",
        "explore.completed",
        "doctor.completed",
        "workspace.status.ready",
        "approval.requested",
        "approval.resolved",
        "turn.completed",
        "turn.failed",
    ]
    .into_iter()
    .collect::<BTreeSet<_>>();
    assert!(
        expected_event_types.is_subset(&loaded_event_types),
        "missing event contract coverage: {loaded_event_types:?}"
    );
    assert_json_has_no_key(&loaded_value, "workspace_root");
    assert_json_omits_workspace_prefix(&loaded_value, &dir);
    let context_ready = loaded
        .events
        .iter()
        .find(|event| event.event_type == "context.packet.ready")
        .expect("context packet event");
    assert_eq!(
        context_ready.payload["packet_path"],
        ".mimir/runs/20260101-120000-abcdef31/context_packet.json"
    );
    assert_eq!(context_ready.payload["likely_files"][0], "src/lib.rs");

    let mut status: WorkspaceStatusDto = client
        .get(format!("{base_url}/v1/workspace/status"))
        .bearer_auth(TOKEN)
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(status.mimir.sessions_count, 1);
    assert!(!status.workspace_name.is_empty());
    assert!(status
        .commands
        .iter()
        .any(|command| command.name == "/share"));
    status
        .commands
        .sort_by(|left, right| left.name.cmp(&right.name));

    let forbidden = client
        .post(format!("{base_url}/v1/sessions"))
        .bearer_auth(TOKEN)
        .json(&serde_json::json!({"workspace_root": "/tmp"}))
        .send()
        .await
        .unwrap();
    assert_eq!(forbidden.status(), StatusCode::FORBIDDEN);
    let forbidden_body = forbidden.text().await.unwrap();
    assert_public_text_is_safe(&forbidden_body, &dir);

    let (mut socket, _) = connect_async(websocket_event_request(
        &base_url,
        &created.metadata.session_id,
        0,
    ))
    .await
    .unwrap();
    let mut ws_events = Vec::new();
    while ws_events.len() < 13 {
        let message = tokio::time::timeout(Duration::from_secs(2), socket.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        if let tokio_tungstenite::tungstenite::Message::Text(text) = message {
            assert_public_text_is_safe(&text, &dir);
            let value: Value = serde_json::from_str(&text).unwrap();
            assert_json_has_no_key(&value, "workspace_root");
            let event = serde_json::from_value::<SessionEventDto>(value).unwrap();
            assert_session_event_payload_contract(&event);
            ws_events.push(event);
        }
    }
    assert!(ws_events
        .iter()
        .any(|event| event.event_type == "context.packet.ready"));
    assert!(ws_events
        .iter()
        .any(|event| event.event_type == "approval.requested"));
    assert!(ws_events
        .iter()
        .any(|event| event.event_type == "approval.resolved"));
    assert!(ws_events
        .iter()
        .any(|event| event.event_type == "turn.failed"));
    socket.close(None).await.unwrap();

    handle.abort();
}

#[tokio::test]
async fn trace_status_uses_trace_artifacts_not_events_fallbacks() {
    let dir = tempfile::TempDir::new().unwrap();
    let events_only_run_id = "20260101-120000-abcdef41";
    let events_only_run = dir.path().join(".mimir/runs").join(events_only_run_id);
    std::fs::create_dir_all(&events_only_run).unwrap();
    std::fs::write(
        events_only_run.join("context_packet.json"),
        br#"{"ok":true}"#,
    )
    .unwrap();
    std::fs::write(
        events_only_run.join("events.jsonl"),
        br#"{"event_type":"provider_response","trace_id":"not-a-trace-artifact"}"#,
    )
    .unwrap();

    #[cfg(unix)]
    let symlink_run_id = "20260101-120000-abcdef42";
    #[cfg(unix)]
    {
        let symlink_run = dir.path().join(".mimir/runs").join(symlink_run_id);
        std::fs::create_dir_all(&symlink_run).unwrap();
        let symlink_target = dir.path().join("trace-target.jsonl");
        std::fs::write(
            &symlink_target,
            format!(
                r#"{{"schema_version":1,"span_id":"0123456789abcdef","name":"trace","start_us":1,"end_us":2,"attrs":{{"raw_prompt":"RAW_PROMPT_SHOULD_NOT_LEAK_SYMLINK","provider_request":"PROVIDER_REQUEST_BODY_SHOULD_NOT_LEAK_SYMLINK","provider_response":"PROVIDER_RESPONSE_BODY_SHOULD_NOT_LEAK_SYMLINK","api_key":"{SECRET}"}}}}"#
            ),
        )
        .unwrap();
        std::os::unix::fs::symlink(&symlink_target, symlink_run.join("trace.spans.jsonl")).unwrap();
    }

    let (base_url, handle) = spawn_ui_server(&dir).await;
    let client = reqwest::Client::new();
    let status: WorkspaceStatusDto = client
        .get(format!("{base_url}/v1/workspace/status"))
        .bearer_auth(TOKEN)
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    let events_only_run = status
        .mimir
        .recent_runs
        .iter()
        .find(|run| run.run_id == events_only_run_id)
        .expect("events-only run");
    assert_eq!(events_only_run.trace_status.state, "absent");
    assert!(!events_only_run.trace_status.redacted);

    let artifacts: ArtifactListResponseDto = client
        .get(format!(
            "{base_url}/v1/runs/{events_only_run_id}/artifacts?token={TOKEN}"
        ))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(artifacts.trace_status.state, "absent");
    assert!(artifacts
        .artifacts
        .iter()
        .any(|artifact| artifact.name == "events.jsonl"));

    #[cfg(unix)]
    {
        let symlink_run = status
            .mimir
            .recent_runs
            .iter()
            .find(|run| run.run_id == symlink_run_id)
            .expect("symlink trace run");
        assert_eq!(symlink_run.trace_status.state, "unavailable");
        assert!(!symlink_run.trace_status.redacted);

        let artifacts_response = client
            .get(format!(
                "{base_url}/v1/runs/{symlink_run_id}/artifacts?token={TOKEN}"
            ))
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap();
        let artifacts_text = artifacts_response.text().await.unwrap();
        assert_public_text_is_safe(&artifacts_text, &dir);
        let artifacts: ArtifactListResponseDto = serde_json::from_str(&artifacts_text).unwrap();
        assert_eq!(artifacts.trace_status.state, "unavailable");
        assert!(!artifacts.trace_status.redacted);
        assert!(artifacts
            .artifacts
            .iter()
            .all(|artifact| artifact.name != "trace.spans.jsonl"));

        let fetch_response = client
            .get(format!(
                "{base_url}/v1/runs/{symlink_run_id}/artifacts/trace.spans.jsonl?token={TOKEN}"
            ))
            .send()
            .await
            .unwrap();
        assert_eq!(fetch_response.status(), StatusCode::FORBIDDEN);
        let fetch_text = fetch_response.text().await.unwrap();
        assert!(fetch_text.contains("artifact path escapes run directory"));
        assert_public_text_omits_needles(
            &fetch_text,
            &dir,
            &[
                "RAW_PROMPT_SHOULD_NOT_LEAK_SYMLINK".to_string(),
                "PROVIDER_REQUEST_BODY_SHOULD_NOT_LEAK_SYMLINK".to_string(),
                "PROVIDER_RESPONSE_BODY_SHOULD_NOT_LEAK_SYMLINK".to_string(),
                format!(".mimir/runs/{symlink_run_id}/trace.spans.jsonl"),
            ],
        );
    }

    handle.abort();
}

#[tokio::test]
async fn trace_artifact_previews_redact_malformed_jsonl_and_oversized_redacted_output() {
    let dir = tempfile::TempDir::new().unwrap();
    let run_id = "20260101-120000-abcdef43";
    let run_dir = dir.path().join(".mimir/runs").join(run_id);
    std::fs::create_dir_all(&run_dir).unwrap();
    std::fs::create_dir_all(dir.path().join("src")).unwrap();
    let workspace_root = std::fs::canonicalize(dir.path()).unwrap();
    let source_path = workspace_root.join("src/private_trace.rs");
    std::fs::write(&source_path, "pub fn private_trace() {}\n").unwrap();

    let trace_path = run_dir.join("trace.spans.jsonl");
    let trace_content_path = workspace_root
        .join(".mimir/runs")
        .join(run_id)
        .join("trace.spans.jsonl");
    let malformed_trace = format!(
        "{{\"schema_version\":1,\"span_id\":\"0123456789abcdef\",\"name\":\"trace.preview\",\"start_us\":1,\"end_us\":2,\"attrs\":{{\"api_key\":\"{SECRET}\",\"path\":\"{}\"}}}}\nnot-json trace line api_key={SECRET} path={}\n",
        source_path.to_string_lossy(),
        trace_content_path.to_string_lossy()
    );
    std::fs::write(&trace_path, malformed_trace.as_bytes()).unwrap();

    let oversized_name = "expanded_preview.json";
    let oversized_path = run_dir.join(oversized_name);
    let oversized_content_path = workspace_root
        .join(".mimir/runs")
        .join(run_id)
        .join(oversized_name);
    let raw_prompt = "RAW_PROMPT_SHOULD_NOT_LEAK_OVERSIZED";
    let provider_request_body = "PROVIDER_REQUEST_BODY_SHOULD_NOT_LEAK_OVERSIZED";
    let provider_response_body = "PROVIDER_RESPONSE_BODY_SHOULD_NOT_LEAK_OVERSIZED";
    let numbers = std::iter::repeat_n("0", 120_000)
        .collect::<Vec<_>>()
        .join(",");
    let compact_json = format!(
        r#"{{"items":[{numbers}],"raw_prompt":"{raw_prompt}","provider_request_body":"{provider_request_body}","provider_response_body":"{provider_response_body}","artifact_path":"{}","api_key":"{SECRET}"}}"#,
        oversized_content_path.to_string_lossy()
    );
    assert!(
        compact_json.len() as u64 <= 512 * 1024,
        "raw artifact must fit under the fetch cap to exercise the redacted preview cap"
    );
    std::fs::write(&oversized_path, compact_json.as_bytes()).unwrap();

    let private_needles = vec![
        raw_prompt.to_string(),
        provider_request_body.to_string(),
        provider_response_body.to_string(),
        source_path.to_string_lossy().to_string(),
        trace_path.to_string_lossy().to_string(),
        trace_content_path.to_string_lossy().to_string(),
        oversized_path.to_string_lossy().to_string(),
        oversized_content_path.to_string_lossy().to_string(),
        format!(".mimir/runs/{run_id}/{oversized_name}"),
    ];

    let (base_url, handle) = spawn_ui_server(&dir).await;
    let client = reqwest::Client::new();

    let status_response = client
        .get(format!("{base_url}/v1/workspace/status"))
        .bearer_auth(TOKEN)
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap();
    let status_text = status_response.text().await.unwrap();
    assert_public_text_omits_needles(&status_text, &dir, &private_needles);
    let status: WorkspaceStatusDto = serde_json::from_str(&status_text).unwrap();
    let run_summary = status
        .mimir
        .recent_runs
        .iter()
        .find(|run| run.run_id == run_id)
        .expect("status summary for redacted trace run");
    assert_eq!(run_summary.path, format!(".mimir/runs/{run_id}"));
    assert_eq!(run_summary.trace_status.state, "recorded");
    assert!(run_summary.trace_status.redacted);

    let artifacts_response = client
        .get(format!(
            "{base_url}/v1/runs/{run_id}/artifacts?token={TOKEN}"
        ))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap();
    let artifacts_text = artifacts_response.text().await.unwrap();
    assert_public_text_omits_needles(&artifacts_text, &dir, &private_needles);
    let artifacts: ArtifactListResponseDto = serde_json::from_str(&artifacts_text).unwrap();
    assert_eq!(artifacts.trace_status.state, "recorded");
    assert!(artifacts.trace_status.redacted);
    assert!(artifacts
        .artifacts
        .iter()
        .any(|artifact| artifact.name == "trace.spans.jsonl"));
    assert!(artifacts
        .artifacts
        .iter()
        .all(|artifact| artifact.name != oversized_name));

    let trace_response_text = client
        .get(format!(
            "{base_url}/v1/runs/{run_id}/artifacts/trace.spans.jsonl?token={TOKEN}"
        ))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .text()
        .await
        .unwrap();
    assert_public_text_omits_needles(&trace_response_text, &dir, &private_needles);
    let trace_response: ArtifactContentResponseDto =
        serde_json::from_str(&trace_response_text).unwrap();
    assert_eq!(trace_response.content_type, "text/plain; charset=utf-8");
    assert_eq!(trace_response.checksum_basis, "redacted_preview");
    assert!(trace_response.redacted);
    let trace_text = trace_response.content.as_str().unwrap();
    assert!(trace_text.contains("not-json trace line"));
    assert!(trace_text.contains("REDACTED"));

    let oversized_response = client
        .get(format!(
            "{base_url}/v1/runs/{run_id}/artifacts/{oversized_name}?token={TOKEN}"
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(oversized_response.status(), StatusCode::PAYLOAD_TOO_LARGE);
    let oversized_text = oversized_response.text().await.unwrap();
    assert!(oversized_text.contains("artifact preview exceeds UI fetch size cap"));
    assert_public_text_omits_needles(&oversized_text, &dir, &private_needles);

    handle.abort();
}

#[tokio::test]
async fn run_trace_replay_and_share_dtos_are_redacted_and_workspace_relative() {
    let dir = tempfile::TempDir::new().unwrap();
    let run_id = "20260101-120000-abcdef32";
    let request = write_replayable_run(&dir, run_id);
    let request_bytes = serde_json::to_vec_pretty(&request).unwrap();
    let workspace_root = std::fs::canonicalize(dir.path()).unwrap();
    let trace_path = workspace_root
        .join("src/replay_target.rs")
        .to_string_lossy()
        .to_string();
    let trace_span = serde_json::json!({
        "schema_version": 1,
        "span_id": "a3f9b2c14e8d1c5f",
        "trace_id": "a3f9b2c14e8d1c5f6b7a8e9d0c1b2a3f",
        "parent_id": null,
        "name": "context.build",
        "kind": "internal",
        "start_us": 1_779_131_722_000_000_u64,
        "end_us": 1_779_131_722_412_000_u64,
        "attrs": {
            "api_key": SECRET,
            "path": trace_path,
            "source_path": trace_path,
            "run_id": run_id
        },
        "events": [{
            "at_us": 1_779_131_722_190_000_u64,
            "name": "retrieval.completed",
            "attrs": {
                "source_path": trace_path,
                "note": format!("read {trace_path}")
            }
        }],
        "status": {
            "code": "ok",
            "message": format!("finished {trace_path} using {SECRET}")
        }
    });
    let trace_done = serde_json::json!({
        "schema_version": 1,
        "span_id": "b3f9b2c14e8d1c5f",
        "trace_id": "a3f9b2c14e8d1c5f6b7a8e9d0c1b2a3f",
        "parent_id": "a3f9b2c14e8d1c5f",
        "name": "context.done",
        "kind": "internal",
        "start_us": 1_779_131_722_412_000_u64,
        "end_us": 1_779_131_722_413_000_u64,
        "attrs": {
            "path": trace_path
        },
        "status": {
            "code": "ok",
            "message": null
        }
    });
    let trace = format!(
        "{}\n{}\n",
        serde_json::to_string(&trace_span).unwrap(),
        serde_json::to_string(&trace_done).unwrap()
    );
    std::fs::write(
        dir.path()
            .join(".mimir/runs")
            .join(run_id)
            .join("trace.spans.jsonl"),
        trace.as_bytes(),
    )
    .unwrap();
    let (base_url, handle) = spawn_ui_server(&dir).await;
    let client = reqwest::Client::new();

    let status: WorkspaceStatusDto = client
        .get(format!("{base_url}/v1/workspace/status"))
        .bearer_auth(TOKEN)
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    let run = status
        .mimir
        .recent_runs
        .iter()
        .find(|run| run.run_id == run_id)
        .expect("recent run");
    assert_eq!(run.path, format!(".mimir/runs/{run_id}"));
    assert!(run.artifact_count >= 3);
    assert!(run.has_context_packet);
    assert_eq!(run.trace_status.state, "recorded");
    assert!(run.trace_status.redacted);

    let artifacts: ArtifactListResponseDto = client
        .get(format!(
            "{base_url}/v1/runs/{run_id}/artifacts?token={TOKEN}"
        ))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(artifacts.run_id, run_id);
    assert_eq!(artifacts.trace_status.state, "recorded");
    assert!(artifacts.trace_status.redacted);
    let trace_summary = artifacts
        .artifacts
        .iter()
        .find(|artifact| artifact.name == "trace.spans.jsonl")
        .expect("trace artifact");
    assert_eq!(
        trace_summary.path,
        format!(".mimir/runs/{run_id}/trace.spans.jsonl")
    );
    assert_eq!(trace_summary.checksum_basis, "redacted_preview");
    assert!(trace_summary.redacted);
    assert_ne!(trace_summary.sha256, sha256_hex(trace.as_bytes()));

    let trace_content: ArtifactContentResponseDto = client
        .get(format!(
            "{base_url}/v1/runs/{run_id}/artifacts/trace.spans.jsonl?token={TOKEN}"
        ))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(trace_content.name, "trace.spans.jsonl");
    assert_eq!(trace_content.path, trace_summary.path);
    assert_eq!(trace_content.content_type, "text/plain; charset=utf-8");
    assert_eq!(trace_content.checksum_basis, "redacted_preview");
    assert!(trace_content.redacted);
    assert_eq!(trace_content.sha256, trace_summary.sha256);
    let trace_text = trace_content.content.as_str().unwrap();
    assert!(!trace_text.contains(SECRET));
    assert!(trace_text.contains("REDACTED"));
    assert_text_omits_workspace_prefix(trace_text, &dir);
    let trace_lines = trace_text
        .lines()
        .map(|line| serde_json::from_str::<TraceSpanPreviewDto>(line).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(trace_lines.len(), 2);
    assert_eq!(trace_lines[0].schema_version, 1);
    assert_eq!(trace_lines[0].span_id, "a3f9b2c14e8d1c5f");
    assert_eq!(trace_lines[0].name, "context.build");
    assert_eq!(trace_lines[0].kind.as_deref(), Some("internal"));
    assert_eq!(trace_lines[0].start_us, 1_779_131_722_000_000);
    assert_eq!(trace_lines[0].end_us, 1_779_131_722_412_000);
    let attrs = trace_lines[0].attrs.as_ref().unwrap();
    assert_eq!(attrs["path"], "src/replay_target.rs");
    assert_eq!(attrs["source_path"], "src/replay_target.rs");
    assert_eq!(attrs["api_key"], "<REDACTED:SECRET_FIELD>");
    let event = &trace_lines[0].events.as_ref().unwrap()[0];
    assert_eq!(event.name, "retrieval.completed");
    assert_eq!(
        event.attrs.as_ref().unwrap()["source_path"],
        "src/replay_target.rs"
    );
    assert!(!event.attrs.as_ref().unwrap()["note"]
        .as_str()
        .unwrap()
        .contains(&trace_path));
    assert_eq!(
        trace_lines[0].status.as_ref().unwrap().code.as_deref(),
        Some("ok")
    );

    let replay: ReplayPreviewResponseDto = client
        .get(format!("{base_url}/v1/runs/{run_id}/replay?token={TOKEN}"))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(replay.run_id, run_id);
    assert_eq!(
        replay.packet_path,
        format!(".mimir/runs/{run_id}/context_packet.json")
    );
    assert_eq!(replay.source, "saved_artifact");
    assert_eq!(replay.provider_request_sha256, sha256_hex(&request_bytes));
    assert_eq!(
        replay.user_prompt_sha256.as_deref(),
        Some(sha256_hex(b"redacted replay request for replay_target_unique").as_str())
    );
    assert!(replay.redacted);
    assert_eq!(
        replay.request["messages"][0]["content"],
        "redacted replay request for replay_target_unique"
    );
    let replay_text = serde_json::to_string(&replay.request).unwrap();
    assert!(!replay_text.contains(SECRET));

    let share_get: SharePreviewResponseDto = client
        .get(format!("{base_url}/v1/runs/{run_id}/share?token={TOKEN}"))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    let share_post: SharePreviewResponseDto = client
        .post(format!("{base_url}/v1/runs/{run_id}/share"))
        .bearer_auth(TOKEN)
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    for share in [&share_get, &share_post] {
        assert_eq!(share.run_id, run_id);
        assert_eq!(
            share.packet_path,
            format!(".mimir/runs/{run_id}/context_packet.json")
        );
        assert!(share.redacted);
        assert_eq!(share.bundle.kind, "mimir.packet_share");
        assert_eq!(share.bundle.run_id, run_id);
        assert_eq!(share.bundle.packet_hash, share.packet_hash);
        assert_eq!(
            share.bundle.replay.provider_request_sha256,
            sha256_hex(&request_bytes)
        );
        assert_eq!(
            share.bundle.replay.user_prompt_sha256,
            replay.user_prompt_sha256
        );
        let share_text = serde_json::to_string(share).unwrap();
        assert!(!share_text.contains(SECRET));
        assert!(!share_text.contains(TOKEN));
        assert_text_omits_workspace_prefix(&share_text, &dir);
    }

    let secret_name_error = client
        .get(format!(
            "{base_url}/v1/runs/{run_id}/artifacts/ghp_abcdefghijklmnopqrstuvwxyzABCDEFGHIJ.json?token={TOKEN}"
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(secret_name_error.status(), StatusCode::BAD_REQUEST);
    let secret_name_body = secret_name_error.text().await.unwrap();
    assert_public_text_is_safe(&secret_name_body, &dir);

    handle.abort();
}
