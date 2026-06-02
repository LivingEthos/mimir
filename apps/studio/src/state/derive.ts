import type {
  ArtifactSummary,
  ContextWhyResult,
  InitResult,
  RunSummary,
  RunsResult,
  SessionEvent,
  TraceStatus,
  WorkspaceStatus,
} from "../api/types";

export interface ContextSnapshot {
  runId: string;
  packetId: string;
  packetHash: string;
  packetPath: string;
  estimatedInputTokens: number;
  guidanceFiles: string[];
  likelyFiles: string[];
  riskyOmissions: Array<{
    path: string;
    reason: string;
    risk: string | null;
  }>;
}

export interface ArtifactEvent {
  runId: string;
  kind: string;
  path: string;
}

export function latestContext(events: SessionEvent[]): ContextSnapshot | null {
  const ready = [...events].reverse().find((event) => event.type === "context.packet.ready");
  if (!ready) {
    return null;
  }

  const payload = ready.payload as Record<string, unknown>;
  const runId = String(payload.run_id ?? "");

  return {
    runId,
    packetId: String(payload.packet_id ?? ""),
    packetHash: String(payload.packet_hash ?? ""),
    packetPath: String(payload.packet_path ?? ""),
    estimatedInputTokens: Number(payload.estimated_input_tokens ?? 0),
    guidanceFiles: asStringArray(payload.guidance_files),
    likelyFiles: asStringArray(payload.likely_files),
    riskyOmissions: events
      .filter((event) => event.type === "context.omission.risk")
      .map((event) => event.payload as Record<string, unknown>)
      .filter((item) => String(item.run_id ?? "") === runId)
      .map((item) => ({
        path: String(item.path ?? ""),
        reason: String(item.reason ?? ""),
        risk: item.risk == null ? null : String(item.risk),
      })),
  };
}

export function artifactEvents(events: SessionEvent[]): ArtifactEvent[] {
  return events
    .filter((event) => event.type === "artifact.written")
    .map((event) => {
      const payload = event.payload as Record<string, unknown>;
      return {
        runId: String(payload.run_id ?? ""),
        kind: String(payload.artifact_kind ?? ""),
        path: String(payload.path ?? ""),
      };
    });
}

export function workspaceStatusFromEvents(events: SessionEvent[]): WorkspaceStatus | null {
  for (const event of [...events].reverse()) {
    if (event.type !== "workspace.status.ready") {
      continue;
    }

    const payload = event.payload as { status?: unknown };
    if (isWorkspaceStatus(payload.status)) {
      return payload.status;
    }
  }

  return null;
}

export function latestInitResult(events: SessionEvent[]): InitResult | null {
  return latestCommandResult(events, "init", isInitResult);
}

export function latestRunsResult(events: SessionEvent[]): RunsResult | null {
  return latestCommandResult(events, "runs", isRunsResult);
}

export function latestWhyResult(events: SessionEvent[]): ContextWhyResult | null {
  return latestCommandResult(events, "why", isContextWhyResult);
}

export function latestTurnSummary(events: SessionEvent[]): string {
  const terminal = [...events]
    .reverse()
    .find((event) => event.type === "turn.failed" || event.type === "turn.completed");
  if (terminal) {
    const payload = terminal.payload as Record<string, unknown>;
    return terminal.type === "turn.failed"
      ? String(payload.error ?? "Turn failed")
      : String(payload.summary ?? "Turn completed");
  }

  return "Ready";
}

export function contextPressure(context: ContextSnapshot | null, tokenCap = 64_000): number {
  if (!context) {
    return 0;
  }

  return Math.min(100, Math.round((context.estimatedInputTokens / Math.max(1, tokenCap)) * 100));
}

export function toArtifactSummaries(items: ArtifactEvent[]): ArtifactSummary[] {
  return items.map((item) => ({
    name: item.path.split("/").at(-1) ?? item.kind,
    path: item.path,
    size_bytes: 0,
    sha256: "",
    redacted: true,
  }));
}

function asStringArray(value: unknown): string[] {
  return Array.isArray(value) ? value.map((item) => String(item)) : [];
}

function isWorkspaceStatus(value: unknown): value is WorkspaceStatus {
  if (!value || typeof value !== "object") {
    return false;
  }
  const candidate = value as Partial<WorkspaceStatus>;
  return Boolean(candidate.workspace_name && candidate.git && candidate.mimir && candidate.providers);
}

function latestCommandResult<T>(
  events: SessionEvent[],
  command: string,
  guard: (value: unknown) => value is T,
): T | null {
  for (const event of [...events].reverse()) {
    if (event.type !== "workspace.status.ready") {
      continue;
    }
    const payload = event.payload as { result?: unknown; status?: unknown };
    const result = payload.result ?? payload.status;
    if (guard(result) && latestStartedCommand(events, event.sequence) === command) {
      return result;
    }
  }
  return null;
}

function latestStartedCommand(events: SessionEvent[], beforeSequence: number): string | null {
  for (const event of [...events].reverse()) {
    if (event.sequence >= beforeSequence || event.type !== "turn.started") {
      continue;
    }
    const payload = event.payload as Record<string, unknown>;
    return String(payload.command ?? "");
  }
  return null;
}

function isInitResult(value: unknown): value is InitResult {
  if (!value || typeof value !== "object") {
    return false;
  }
  const candidate = value as Partial<InitResult>;
  return Array.isArray(candidate.created) && isWorkspaceStatus(candidate.status);
}

function isRunsResult(value: unknown): value is RunsResult {
  if (!value || typeof value !== "object") {
    return false;
  }
  const candidate = value as Partial<RunsResult>;
  return Array.isArray(candidate.runs) && candidate.runs.every(isRunSummary);
}

function isContextWhyResult(value: unknown): value is ContextWhyResult {
  if (!value || typeof value !== "object") {
    return false;
  }
  const candidate = value as Partial<ContextWhyResult>;
  return Boolean(candidate.path && candidate.status && candidate.run_id && candidate.packet_hash);
}

function isRunSummary(value: unknown): value is RunSummary {
  if (!value || typeof value !== "object") {
    return false;
  }
  const candidate = value as Partial<RunSummary>;
  return Boolean(candidate.run_id && candidate.path && isTraceStatus(candidate.trace_status));
}

function isTraceStatus(value: unknown): value is TraceStatus {
  if (!value || typeof value !== "object") {
    return false;
  }
  const candidate = value as Partial<TraceStatus>;
  return (
    (candidate.state === "absent" ||
      candidate.state === "recorded" ||
      candidate.state === "unavailable") &&
    typeof candidate.redacted === "boolean"
  );
}
