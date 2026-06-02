export type RuntimeMode = "mock" | "api";

export const sessionEventTypes = [
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
] as const;

export type SessionEventType = (typeof sessionEventTypes)[number];

export interface SessionEventBase<
  TType extends SessionEventType,
  TPayload extends Record<string, unknown>,
> {
  schema_version: number;
  event_id: string;
  session_id: string;
  sequence: number;
  timestamp: string;
  type: TType;
  payload: TPayload;
}

export type SessionEventPayload = Record<string, unknown>;

export interface SessionCreatedPayload extends SessionEventPayload {
  title: string;
  workspace_name: string;
}

export interface TurnStartedPayload extends SessionEventPayload {
  turn_id: string;
  command: string;
  task: string;
}

export interface ContextBuildStartedPayload extends SessionEventPayload {
  turn_id: string;
  provider: string;
  model: string;
}

export interface ContextPacketReadyPayload extends SessionEventPayload {
  run_id: string;
  packet_id: string;
  packet_hash: string;
  packet_path: string;
  estimated_input_tokens: number;
  guidance_files: string[];
  likely_files: string[];
}

export interface ContextOmissionRiskPayload extends SessionEventPayload {
  run_id: string;
  path: string;
  reason: string;
  risk: string | null;
}

export interface ArtifactWrittenPayload extends SessionEventPayload {
  run_id: string;
  artifact_kind: string;
  path: string;
}

export interface CheckCompletedPayload extends SessionEventPayload {
  checks_loaded: number;
  findings_count: number;
  blocking_findings: number;
  passed: boolean;
}

export interface ExploreCompletedPayload extends SessionEventPayload {
  run_id: string;
  evidence_path: string;
  findings_count: number;
  relevant_paths: string[];
  confidence: number;
}

export const doctorStatusStates = ["ok", "warnings", "failed"] as const;

export type DoctorStatusState = (typeof doctorStatusStates)[number];

export interface DoctorCompletedPayload extends SessionEventPayload {
  status: DoctorStatusState;
  warnings: number;
  failures: number;
}

export interface WorkspaceStatusReadyPayload extends SessionEventPayload {
  status: unknown;
}

export interface ArtifactRef extends SessionEventPayload {
  run_id: string;
  artifact_kind: string;
  path: string;
  sha256: string | null;
  redacted: boolean;
}

export const approvalActions = ["allow_once", "allow_for_session", "deny"] as const;

export type ApprovalAction = (typeof approvalActions)[number];

export interface ApprovalRequest extends SessionEventPayload {
  approval_id: string;
  session_id: string;
  turn_id: string;
  tool_name: string;
  reason: string;
  path: string | null;
  command: string | null;
  artifact: ArtifactRef | null;
  requested_at: string;
}

export interface ApprovalDecision extends SessionEventPayload {
  approval_id: string;
  action: ApprovalAction;
  decided_at: string;
}

export interface ApprovalRequestedPayload extends SessionEventPayload {
  request: ApprovalRequest;
}

export interface ApprovalResolvedPayload extends SessionEventPayload {
  decision: ApprovalDecision;
}

export interface TurnCompletedPayload extends SessionEventPayload {
  turn_id: string;
  summary: string;
}

export interface TurnFailedPayload extends SessionEventPayload {
  turn_id: string;
  error: string;
}

export interface SessionEventPayloads {
  "session.created": SessionCreatedPayload;
  "turn.started": TurnStartedPayload;
  "context.build.started": ContextBuildStartedPayload;
  "context.packet.ready": ContextPacketReadyPayload;
  "context.omission.risk": ContextOmissionRiskPayload;
  "artifact.written": ArtifactWrittenPayload;
  "check.completed": CheckCompletedPayload;
  "explore.completed": ExploreCompletedPayload;
  "doctor.completed": DoctorCompletedPayload;
  "workspace.status.ready": WorkspaceStatusReadyPayload;
  "approval.requested": ApprovalRequestedPayload;
  "approval.resolved": ApprovalResolvedPayload;
  "turn.completed": TurnCompletedPayload;
  "turn.failed": TurnFailedPayload;
}

export type TypedApiSessionEvent = {
  [TType in SessionEventType]: SessionEventBase<TType, SessionEventPayloads[TType]>;
}[SessionEventType];

export type UntypedApiSessionEvent = SessionEventBase<SessionEventType, SessionEventPayload>;

export type ApiSessionEvent = TypedApiSessionEvent | UntypedApiSessionEvent;

export type ClientLocalPayload = SessionEventPayload & {
  result?: unknown;
};

export type ClientLocalSessionEvent = {
  [TType in SessionEventType]: SessionEventBase<TType, ClientLocalPayload>;
}[SessionEventType];

export type SessionEvent = ApiSessionEvent | ClientLocalSessionEvent;

export interface SessionMetadata {
  schema_version: number;
  session_id: string;
  title: string;
  workspace_name: string;
  created_at: string;
  updated_at: string;
}

export interface SessionCreateResponse {
  metadata: SessionMetadata;
  events: ApiSessionEvent[];
}

export interface SessionLoadResponse {
  metadata: SessionMetadata;
  events: ApiSessionEvent[];
}

export interface SessionMessageResponse {
  session_id: string;
  command: string;
  result: unknown;
  events: ApiSessionEvent[];
}

export interface SessionMessageOptions {
  provider?: string;
  model?: string;
}

export interface WorkspaceStatus {
  workspace_name: string;
  git: {
    is_repo: boolean;
    branch: string | null;
    dirty: boolean;
  };
  mimir: {
    initialized: boolean;
    config_present: boolean;
    checks_loaded: number;
    sessions_count: number;
    runs_count: number;
    recent_runs: RunSummary[];
  };
  providers: Array<{
    provider: string;
    models_count: number;
    credential_detected: boolean;
  }>;
  commands?: CommandMetadata[];
}

export const commandSupports = ["backend", "local", "planned"] as const;

export type CommandSupport = (typeof commandSupports)[number];

export interface CommandMetadata {
  name: string;
  usage: string;
  summary: string;
  support: CommandSupport;
  takes_input: boolean;
  enabled: boolean;
  disabled_reason?: string | null;
}

export const traceStatusStates = ["absent", "recorded", "unavailable"] as const;

export type TraceStatusState = (typeof traceStatusStates)[number];

export interface TraceStatus {
  state: TraceStatusState;
  redacted: boolean;
}

export interface RunSummary {
  run_id: string;
  path: string;
  artifact_count: number;
  has_context_packet: boolean;
  trace_status: TraceStatus;
}

export interface InitResult {
  created: string[];
  status: WorkspaceStatus;
}

export interface RunsResult {
  runs: RunSummary[];
}

export interface ContextWhyResult {
  path: string;
  status: "included" | "omitted" | "not_found" | string;
  reason: string;
  reason_code?: string;
  token_count?: number;
  run_id: string;
  packet_id: string;
  packet_hash: string;
  packet_path: string;
  source_hash?: string | null;
}

export interface WorkspaceFileMatch {
  path: string;
  kind: "file" | "symbol" | string;
  line: number | null;
  symbol: string | null;
}

export interface WorkspaceFileSearchResponse {
  results: WorkspaceFileMatch[];
}

export interface ArtifactSummary {
  name: string;
  path: string;
  size_bytes: number;
  sha256: string;
  checksum_basis?: "redacted_preview" | string;
  redacted: boolean;
}

export interface ArtifactListResponse {
  run_id: string;
  trace_status: TraceStatus;
  artifacts: ArtifactSummary[];
}

export interface ArtifactContentResponse {
  name: string;
  path: string;
  content_type: string;
  sha256: string;
  checksum_basis?: "redacted_preview" | string;
  redacted: boolean;
  content: unknown;
}

export interface ReplayPreviewResponse {
  run_id: string;
  packet_id: string;
  packet_hash: string;
  packet_path: string;
  source: "saved_artifact" | "reconstructed" | string;
  provider_request_sha256: string;
  user_prompt_sha256?: string | null;
  redacted: boolean;
  request: unknown;
}

export interface SharePreviewResponse {
  run_id: string;
  packet_id: string;
  packet_hash: string;
  packet_path: string;
  bundle_sha256: string;
  redacted: boolean;
  bundle: unknown;
}

export interface RuntimeConfig {
  mode: RuntimeMode;
  apiBaseUrl: string;
  token: string | null;
}
