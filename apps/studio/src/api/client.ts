import {
  approvalActions,
  commandSupports,
  doctorStatusStates,
  sessionEventTypes,
  traceStatusStates,
  type ApprovalDecision,
  type ApprovalRequest,
  type ApiSessionEvent,
  type ArtifactRef,
  type ArtifactContentResponse,
  type ArtifactListResponse,
  type ArtifactSummary,
  type CommandMetadata,
  type RuntimeConfig,
  type ReplayPreviewResponse,
  type RunSummary,
  type SessionCreateResponse,
  type SessionEventType,
  type SessionLoadResponse,
  type SessionMessageResponse,
  type SessionMessageOptions,
  type SessionMetadata,
  type SharePreviewResponse,
  type TraceStatus,
  type WorkspaceFileMatch,
  type WorkspaceFileSearchResponse,
  type WorkspaceStatus,
} from "./types";

type ApiResponseGuard<T> = (value: unknown) => value is T;

interface ApiResponseShape<T> {
  guard: ApiResponseGuard<T>;
  label: string;
}

const sessionEventTypeSet = new Set<string>(sessionEventTypes);
const approvalActionSet = new Set<string>(approvalActions);
const commandSupportSet = new Set<string>(commandSupports);
const doctorStatusStateSet = new Set<string>(doctorStatusStates);
const traceStatusStateSet = new Set<string>(traceStatusStates);

const sessionEventPayloadGuards = {
  "session.created": isSessionCreatedPayload,
  "turn.started": isTurnStartedPayload,
  "context.build.started": isContextBuildStartedPayload,
  "context.packet.ready": isContextPacketReadyPayload,
  "context.omission.risk": isContextOmissionRiskPayload,
  "artifact.written": isArtifactWrittenPayload,
  "check.completed": isCheckCompletedPayload,
  "explore.completed": isExploreCompletedPayload,
  "doctor.completed": isDoctorCompletedPayload,
  "workspace.status.ready": isWorkspaceStatusReadyPayload,
  "approval.requested": isApprovalRequestedPayload,
  "approval.resolved": isApprovalResolvedPayload,
  "turn.completed": isTurnCompletedPayload,
  "turn.failed": isTurnFailedPayload,
} satisfies Record<SessionEventType, (value: unknown) => boolean>;

const responseShapes = {
  artifactContent: {
    guard: isArtifactContentResponse,
    label: "artifact preview response",
  },
  artifactList: {
    guard: isArtifactListResponse,
    label: "artifact list response",
  },
  commandRegistry: {
    guard: isCommandRegistry,
    label: "command registry response",
  },
  replayPreview: {
    guard: isReplayPreviewResponse,
    label: "packet replay response",
  },
  sessionCreate: {
    guard: isSessionCreateResponse,
    label: "session create response",
  },
  sessionList: {
    guard: isSessionListResponse,
    label: "session list response",
  },
  sessionLoad: {
    guard: isSessionLoadResponse,
    label: "session load response",
  },
  sessionMessage: {
    guard: isSessionMessageResponse,
    label: "message response",
  },
  sharePreview: {
    guard: isSharePreviewResponse,
    label: "packet share response",
  },
  workspaceFileSearch: {
    guard: isWorkspaceFileSearchResponse,
    label: "workspace file search response",
  },
  workspaceStatus: {
    guard: isWorkspaceStatus,
    label: "workspace status response",
  },
} satisfies Record<string, ApiResponseShape<unknown>>;

export interface StudioApiClient {
  readonly config: RuntimeConfig;
  createSession(title: string): Promise<SessionCreateResponse>;
  listSessions(): Promise<SessionMetadata[]>;
  loadSession(sessionId: string): Promise<SessionLoadResponse>;
  submitMessage(
    sessionId: string,
    message: string,
    options?: SessionMessageOptions,
  ): Promise<SessionMessageResponse>;
  workspaceStatus(): Promise<WorkspaceStatus>;
  commandRegistry(): Promise<CommandMetadata[]>;
  searchFiles(query: string): Promise<WorkspaceFileSearchResponse>;
  listArtifacts(runId: string): Promise<ArtifactListResponse>;
  fetchArtifact(runId: string, name: string): Promise<ArtifactContentResponse>;
  fetchReplay(runId: string): Promise<ReplayPreviewResponse>;
  fetchShare(runId: string): Promise<SharePreviewResponse>;
  openEventStream(
    sessionId: string,
    after: number,
    onEvent: (event: ApiSessionEvent) => void,
    onStatus: (status: "open" | "closed" | "error") => void,
  ): WebSocket | null;
}

let inMemoryToken: string | null = null;
let inMemoryApiBaseUrl = "";

const studioWebSocketProtocol = "mimir.studio.v1";
const studioWebSocketTokenProtocolPrefix = "mimir-token.";

export function readRuntimeConfig(): RuntimeConfig {
  const url = new URL(window.location.href);
  const tokenFromUrl = url.searchParams.get("token");
  const apiFromUrl = url.searchParams.get("api") ?? "";
  const api = loopbackApiBaseUrl(apiFromUrl);
  const mock = url.searchParams.get("mock");
  const hadApi = url.searchParams.has("api");

  if (tokenFromUrl) {
    inMemoryToken = tokenFromUrl;
    url.searchParams.delete("token");
  }
  if (hadApi) {
    inMemoryApiBaseUrl = api;
  }
  if (url.searchParams.has("api")) {
    url.searchParams.delete("api");
  }
  if (tokenFromUrl || hadApi) {
    window.history.replaceState({}, "", `${url.pathname}${url.search}${url.hash}`);
  }

  const token = tokenFromUrl ?? inMemoryToken;
  const apiBaseUrl = api || inMemoryApiBaseUrl;

  return {
    mode: mock === "1" || (!token && mock !== "0") ? "mock" : "api",
    apiBaseUrl,
    token,
  };
}

function loopbackApiBaseUrl(value: string): string {
  const trimmed = value.trim().replace(/\/$/, "");
  if (!trimmed) {
    return "";
  }

  try {
    const parsed = new URL(trimmed);
    if (parsed.protocol !== "http:" && parsed.protocol !== "https:") {
      return "";
    }
    if (!["127.0.0.1", "localhost", "::1", "[::1]"].includes(parsed.hostname)) {
      return "";
    }
    if (parsed.username || parsed.password || parsed.search || parsed.hash || parsed.pathname !== "/") {
      return "";
    }
    return parsed.origin;
  } catch {
    return "";
  }
}

function webSocketAuthProtocols(token: string): string[] {
  return [
    studioWebSocketProtocol,
    `${studioWebSocketTokenProtocolPrefix}${hexEncode(token)}`,
  ];
}

function hexEncode(value: string): string {
  return Array.from(new TextEncoder().encode(value), (byte) =>
    byte.toString(16).padStart(2, "0"),
  ).join("");
}

export function createStudioApiClient(config: RuntimeConfig): StudioApiClient {
  const request = async <T>(
    path: string,
    shape: ApiResponseShape<T>,
    init: RequestInit = {},
  ): Promise<T> => {
    if (!config.token) {
      throw new Error("Missing Mimir Studio UI token");
    }

    const response = await fetch(`${config.apiBaseUrl}${path}`, {
      ...init,
      headers: {
        "Content-Type": "application/json",
        Authorization: `Bearer ${config.token}`,
        ...(init.headers ?? {}),
      },
    });

    if (!response.ok) {
      const body = await response.text();
      throw new Error(apiErrorMessage(body, response.status));
    }

    const json = await parseJsonResponse(response, shape.label);
    if (!shape.guard(json)) {
      throw unexpectedApiShapeError(shape.label);
    }
    return json;
  };

  return {
    config,
    createSession(title) {
      return request("/v1/sessions", responseShapes.sessionCreate, {
        method: "POST",
        body: JSON.stringify({ title }),
      });
    },
    listSessions() {
      return request("/v1/sessions", responseShapes.sessionList);
    },
    loadSession(sessionId) {
      return request(
        `/v1/sessions/${encodeURIComponent(sessionId)}`,
        responseShapes.sessionLoad,
      );
    },
    submitMessage(sessionId, message, options = {}) {
      const body: Record<string, string> = { message };
      const provider = options.provider?.trim();
      const model = options.model?.trim();
      if (provider) {
        body.provider = provider;
      }
      if (model) {
        body.model = model;
      }

      return request(
        `/v1/sessions/${encodeURIComponent(sessionId)}/messages`,
        responseShapes.sessionMessage,
        {
          method: "POST",
          body: JSON.stringify(body),
        },
      );
    },
    workspaceStatus() {
      return request("/v1/workspace/status", responseShapes.workspaceStatus);
    },
    commandRegistry() {
      return request("/v1/workspace/commands", responseShapes.commandRegistry);
    },
    searchFiles(query) {
      const params = new URLSearchParams({ q: query, limit: "12" });
      return request(
        `/v1/workspace/files?${params.toString()}`,
        responseShapes.workspaceFileSearch,
      );
    },
    listArtifacts(runId) {
      return request(
        `/v1/runs/${encodeURIComponent(runId)}/artifacts`,
        responseShapes.artifactList,
      );
    },
    fetchArtifact(runId, name) {
      return request(
        `/v1/runs/${encodeURIComponent(runId)}/artifacts/${encodeURIComponent(name)}`,
        responseShapes.artifactContent,
      );
    },
    fetchReplay(runId) {
      return request(
        `/v1/runs/${encodeURIComponent(runId)}/replay`,
        responseShapes.replayPreview,
      );
    },
    fetchShare(runId) {
      return request(
        `/v1/runs/${encodeURIComponent(runId)}/share`,
        responseShapes.sharePreview,
      );
    },
    openEventStream(sessionId, after, onEvent, onStatus) {
      if (!config.token) {
        return null;
      }

      const base =
        config.apiBaseUrl ||
        `${window.location.protocol}//${window.location.host}`;
      const wsBase = base.replace(/^http:/, "ws:").replace(/^https:/, "wss:");
      const params = new URLSearchParams({ after: String(after) });
      const socket = new WebSocket(
        `${wsBase}/v1/sessions/${encodeURIComponent(sessionId)}/events?${params.toString()}`,
        webSocketAuthProtocols(config.token),
      );

      socket.addEventListener("open", () => onStatus("open"));
      socket.addEventListener("close", () => onStatus("closed"));
      socket.addEventListener("error", () => onStatus("error"));
      socket.addEventListener("message", (message) => {
        try {
          const event = JSON.parse(message.data) as unknown;
          if (!isSessionEvent(event)) {
            onStatus("error");
            return;
          }
          onEvent(event);
        } catch {
          onStatus("error");
        }
      });

      return socket;
    },
  };
}

async function parseJsonResponse(response: Response, label: string): Promise<unknown> {
  try {
    return (await response.json()) as unknown;
  } catch {
    throw new Error(`Mimir API returned invalid JSON for ${label}`);
  }
}

function unexpectedApiShapeError(label: string): Error {
  return new Error(`Mimir API returned an unexpected shape for ${label}`);
}

function isSessionListResponse(value: unknown): value is SessionMetadata[] {
  return isArrayOf(value, isSessionMetadata);
}

function isSessionCreateResponse(value: unknown): value is SessionCreateResponse {
  return isSessionEnvelope(value);
}

function isSessionLoadResponse(value: unknown): value is SessionLoadResponse {
  return isSessionEnvelope(value);
}

function isSessionEnvelope(
  value: unknown,
): value is SessionCreateResponse | SessionLoadResponse {
  return (
    isRecord(value) &&
    isSessionMetadata(value.metadata) &&
    isArrayOf(value.events, isSessionEvent)
  );
}

function isSessionMessageResponse(value: unknown): value is SessionMessageResponse {
  return (
    isRecord(value) &&
    isString(value.session_id) &&
    isString(value.command) &&
    hasOwn(value, "result") &&
    isArrayOf(value.events, isSessionEvent)
  );
}

function isSessionMetadata(value: unknown): value is SessionMetadata {
  return (
    isRecord(value) &&
    isNumber(value.schema_version) &&
    isString(value.session_id) &&
    isString(value.title) &&
    isString(value.workspace_name) &&
    isString(value.created_at) &&
    isString(value.updated_at)
  );
}

function isSessionEvent(value: unknown): value is ApiSessionEvent {
  if (
    !isRecord(value) ||
    !hasOnlyKeys(value, [
      "schema_version",
      "event_id",
      "session_id",
      "sequence",
      "timestamp",
      "type",
      "payload",
    ]) ||
    !isNumber(value.schema_version) ||
    !isString(value.event_id) ||
    !isString(value.session_id) ||
    !isNumber(value.sequence) ||
    !isString(value.timestamp) ||
    !isSessionEventType(value.type)
  ) {
    return false;
  }

  return sessionEventPayloadGuards[value.type](value.payload);
}

function isSessionCreatedPayload(value: unknown): boolean {
  return (
    isRecord(value) &&
    hasOnlyKeys(value, ["title", "workspace_name"]) &&
    isString(value.title) &&
    isString(value.workspace_name)
  );
}

function isTurnStartedPayload(value: unknown): boolean {
  return (
    isRecord(value) &&
    hasOnlyKeys(value, ["turn_id", "command", "task"]) &&
    isString(value.turn_id) &&
    isString(value.command) &&
    isString(value.task)
  );
}

function isContextBuildStartedPayload(value: unknown): boolean {
  return (
    isRecord(value) &&
    hasOnlyKeys(value, ["turn_id", "provider", "model"]) &&
    isString(value.turn_id) &&
    isString(value.provider) &&
    isString(value.model)
  );
}

function isContextPacketReadyPayload(value: unknown): boolean {
  return (
    isRecord(value) &&
    hasOnlyKeys(value, [
      "run_id",
      "packet_id",
      "packet_hash",
      "packet_path",
      "estimated_input_tokens",
      "guidance_files",
      "likely_files",
    ]) &&
    isString(value.run_id) &&
    isString(value.packet_id) &&
    isString(value.packet_hash) &&
    isString(value.packet_path) &&
    isNumber(value.estimated_input_tokens) &&
    isArrayOf(value.guidance_files, isString) &&
    isArrayOf(value.likely_files, isString)
  );
}

function isContextOmissionRiskPayload(value: unknown): boolean {
  return (
    isRecord(value) &&
    hasOnlyKeys(value, ["run_id", "path", "reason", "risk"]) &&
    isString(value.run_id) &&
    isString(value.path) &&
    isString(value.reason) &&
    isNullableString(value.risk)
  );
}

function isArtifactWrittenPayload(value: unknown): boolean {
  return (
    isRecord(value) &&
    hasOnlyKeys(value, ["run_id", "artifact_kind", "path"]) &&
    isString(value.run_id) &&
    isString(value.artifact_kind) &&
    isString(value.path)
  );
}

function isCheckCompletedPayload(value: unknown): boolean {
  return (
    isRecord(value) &&
    hasOnlyKeys(value, ["checks_loaded", "findings_count", "blocking_findings", "passed"]) &&
    isNumber(value.checks_loaded) &&
    isNumber(value.findings_count) &&
    isNumber(value.blocking_findings) &&
    isBoolean(value.passed)
  );
}

function isExploreCompletedPayload(value: unknown): boolean {
  return (
    isRecord(value) &&
    hasOnlyKeys(value, [
      "run_id",
      "evidence_path",
      "findings_count",
      "relevant_paths",
      "confidence",
    ]) &&
    isString(value.run_id) &&
    isString(value.evidence_path) &&
    isNumber(value.findings_count) &&
    isArrayOf(value.relevant_paths, isString) &&
    isNumber(value.confidence)
  );
}

function isDoctorCompletedPayload(value: unknown): boolean {
  return (
    isRecord(value) &&
    hasOnlyKeys(value, ["status", "warnings", "failures"]) &&
    isString(value.status) &&
    doctorStatusStateSet.has(value.status) &&
    isNumber(value.warnings) &&
    isNumber(value.failures)
  );
}

function isWorkspaceStatusReadyPayload(value: unknown): boolean {
  return (
    isRecord(value) &&
    hasOnlyKeys(value, ["status"]) &&
    hasOwn(value, "status")
  );
}

function isApprovalRequestedPayload(value: unknown): boolean {
  return (
    isRecord(value) &&
    hasOnlyKeys(value, ["request"]) &&
    isApprovalRequest(value.request)
  );
}

function isApprovalResolvedPayload(value: unknown): boolean {
  return (
    isRecord(value) &&
    hasOnlyKeys(value, ["decision"]) &&
    isApprovalDecision(value.decision)
  );
}

function isTurnCompletedPayload(value: unknown): boolean {
  return (
    isRecord(value) &&
    hasOnlyKeys(value, ["turn_id", "summary"]) &&
    isString(value.turn_id) &&
    isString(value.summary)
  );
}

function isTurnFailedPayload(value: unknown): boolean {
  return (
    isRecord(value) &&
    hasOnlyKeys(value, ["turn_id", "error"]) &&
    isString(value.turn_id) &&
    isString(value.error)
  );
}

function isApprovalRequest(value: unknown): value is ApprovalRequest {
  return (
    isRecord(value) &&
    hasOnlyKeys(value, [
      "approval_id",
      "session_id",
      "turn_id",
      "tool_name",
      "reason",
      "path",
      "command",
      "artifact",
      "requested_at",
    ]) &&
    isString(value.approval_id) &&
    isString(value.session_id) &&
    isString(value.turn_id) &&
    isString(value.tool_name) &&
    isString(value.reason) &&
    isNullableString(value.path) &&
    isNullableString(value.command) &&
    (value.artifact === null || isArtifactRef(value.artifact)) &&
    isString(value.requested_at)
  );
}

function isApprovalDecision(value: unknown): value is ApprovalDecision {
  return (
    isRecord(value) &&
    hasOnlyKeys(value, ["approval_id", "action", "decided_at"]) &&
    isString(value.approval_id) &&
    isString(value.action) &&
    approvalActionSet.has(value.action) &&
    isString(value.decided_at)
  );
}

function isArtifactRef(value: unknown): value is ArtifactRef {
  return (
    isRecord(value) &&
    hasOnlyKeys(value, ["run_id", "artifact_kind", "path", "sha256", "redacted"]) &&
    isString(value.run_id) &&
    isString(value.artifact_kind) &&
    isString(value.path) &&
    isNullableString(value.sha256) &&
    isBoolean(value.redacted)
  );
}

function isSessionEventType(value: unknown): value is SessionEventType {
  return isString(value) && sessionEventTypeSet.has(value);
}

function isWorkspaceStatus(value: unknown): value is WorkspaceStatus {
  if (!isRecord(value)) {
    return false;
  }

  const commands = value.commands;
  return (
    isString(value.workspace_name) &&
    isGitStatus(value.git) &&
    isMimirStatus(value.mimir) &&
    isArrayOf(value.providers, isProviderStatus) &&
    (commands === undefined || isCommandRegistry(commands))
  );
}

function isGitStatus(value: unknown): value is WorkspaceStatus["git"] {
  return (
    isRecord(value) &&
    isBoolean(value.is_repo) &&
    isNullableString(value.branch) &&
    isBoolean(value.dirty)
  );
}

function isMimirStatus(value: unknown): value is WorkspaceStatus["mimir"] {
  return (
    isRecord(value) &&
    isBoolean(value.initialized) &&
    isBoolean(value.config_present) &&
    isNumber(value.checks_loaded) &&
    isNumber(value.sessions_count) &&
    isNumber(value.runs_count) &&
    isArrayOf(value.recent_runs, isRunSummary)
  );
}

function isProviderStatus(
  value: unknown,
): value is WorkspaceStatus["providers"][number] {
  return (
    isRecord(value) &&
    isString(value.provider) &&
    isNumber(value.models_count) &&
    isBoolean(value.credential_detected)
  );
}

function isCommandRegistry(value: unknown): value is CommandMetadata[] {
  return isArrayOf(value, isCommandMetadata);
}

function isCommandMetadata(value: unknown): value is CommandMetadata {
  return (
    isRecord(value) &&
    isString(value.name) &&
    isString(value.usage) &&
    isString(value.summary) &&
    isCommandSupport(value.support) &&
    isBoolean(value.takes_input) &&
    isBoolean(value.enabled) &&
    (value.disabled_reason === undefined || isNullableString(value.disabled_reason))
  );
}

function isCommandSupport(value: unknown): value is CommandMetadata["support"] {
  return isString(value) && commandSupportSet.has(value);
}

function isWorkspaceFileSearchResponse(
  value: unknown,
): value is WorkspaceFileSearchResponse {
  return isRecord(value) && isArrayOf(value.results, isWorkspaceFileMatch);
}

function isWorkspaceFileMatch(value: unknown): value is WorkspaceFileMatch {
  return (
    isRecord(value) &&
    isString(value.path) &&
    isString(value.kind) &&
    isNullableNumber(value.line) &&
    isNullableString(value.symbol)
  );
}

function isArtifactListResponse(value: unknown): value is ArtifactListResponse {
  return (
    isRecord(value) &&
    isString(value.run_id) &&
    isTraceStatus(value.trace_status) &&
    isArrayOf(value.artifacts, isArtifactSummary)
  );
}

function isArtifactSummary(value: unknown): value is ArtifactSummary {
  return (
    isRecord(value) &&
    isString(value.name) &&
    isString(value.path) &&
    isNumber(value.size_bytes) &&
    isString(value.sha256) &&
    isOptionalString(value.checksum_basis) &&
    isBoolean(value.redacted)
  );
}

function isArtifactContentResponse(value: unknown): value is ArtifactContentResponse {
  return (
    isRecord(value) &&
    isString(value.name) &&
    isString(value.path) &&
    isString(value.content_type) &&
    isString(value.sha256) &&
    isOptionalString(value.checksum_basis) &&
    isBoolean(value.redacted) &&
    hasOwn(value, "content")
  );
}

function isReplayPreviewResponse(value: unknown): value is ReplayPreviewResponse {
  return (
    isRecord(value) &&
    isString(value.run_id) &&
    isString(value.packet_id) &&
    isString(value.packet_hash) &&
    isString(value.packet_path) &&
    isString(value.source) &&
    isString(value.provider_request_sha256) &&
    (value.user_prompt_sha256 === undefined || isNullableString(value.user_prompt_sha256)) &&
    isBoolean(value.redacted) &&
    hasOwn(value, "request")
  );
}

function isSharePreviewResponse(value: unknown): value is SharePreviewResponse {
  return (
    isRecord(value) &&
    isString(value.run_id) &&
    isString(value.packet_id) &&
    isString(value.packet_hash) &&
    isString(value.packet_path) &&
    isString(value.bundle_sha256) &&
    isBoolean(value.redacted) &&
    hasOwn(value, "bundle")
  );
}

function isRunSummary(value: unknown): value is RunSummary {
  return (
    isRecord(value) &&
    isString(value.run_id) &&
    isString(value.path) &&
    isNumber(value.artifact_count) &&
    isBoolean(value.has_context_packet) &&
    isTraceStatus(value.trace_status)
  );
}

function isTraceStatus(value: unknown): value is TraceStatus {
  return (
    isRecord(value) &&
    isString(value.state) &&
    traceStatusStateSet.has(value.state) &&
    isBoolean(value.redacted)
  );
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function hasOwn(value: Record<string, unknown>, key: string): boolean {
  return Object.prototype.hasOwnProperty.call(value, key);
}

function hasOnlyKeys(value: Record<string, unknown>, allowedKeys: readonly string[]): boolean {
  return Object.keys(value).every((key) => allowedKeys.includes(key));
}

function isArrayOf<T>(value: unknown, guard: (item: unknown) => item is T): value is T[] {
  return Array.isArray(value) && value.every(guard);
}

function isString(value: unknown): value is string {
  return typeof value === "string";
}

function isOptionalString(value: unknown): value is string | undefined {
  return value === undefined || isString(value);
}

function isNullableString(value: unknown): value is string | null {
  return value === null || isString(value);
}

function isNumber(value: unknown): value is number {
  return typeof value === "number" && Number.isFinite(value);
}

function isNullableNumber(value: unknown): value is number | null {
  return value === null || isNumber(value);
}

function isBoolean(value: unknown): value is boolean {
  return typeof value === "boolean";
}

function apiErrorMessage(body: string, status: number): string {
  if (!body) {
    return `Mimir API request failed with ${status}`;
  }
  try {
    const parsed = JSON.parse(body) as { error?: unknown };
    if (typeof parsed.error === "string" && parsed.error.trim()) {
      return safeApiErrorText(parsed.error);
    }
  } catch {
    // Fall back to the raw response body below.
  }
  return safeApiErrorText(body);
}

function safeApiErrorText(value: string): string {
  const redacted = value
    .replace(/\/[^"'\s<>]*\/(\.mimir\/runs\/[^"'\s<>]+)/g, "$1")
    .replace(/\/(?:private\/)?tmp\/[^"'\s<>]+/g, "[redacted:path]")
    .replace(/sk-[A-Za-z0-9]{24,}/g, "[redacted:key]")
    .replace(/ghp_[A-Za-z0-9]{36}/g, "[redacted:github]")
    .replace(/xox[baprs]-[0-9A-Za-z]+/g, "[redacted:slack]")
    .replace(/Bearer\s+[A-Za-z0-9_.-]{8,}/gi, "Bearer [redacted]")
    .replace(/\bui-[A-Za-z0-9-]{16,}/g, "ui-[redacted]")
    .replace(/\b[A-Z_]*(?:KEY|SECRET|TOKEN)=[^\s]+/g, "[redacted:env]")
    .replace(/(api[_-]?key|accessToken|refreshToken|sessionToken|token|secret|password)(["'\s:=]+)[A-Za-z0-9_.:/@%+-]{8,}/gi, "$1$2[redacted]");
  return redacted.length > 500 ? `${redacted.slice(0, 500)}...` : redacted;
}
