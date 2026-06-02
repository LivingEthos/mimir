import type {
  SharePreviewResponse,
  SessionCreateResponse,
  SessionEvent,
  SessionMetadata,
  SessionMessageOptions,
  WorkspaceFileMatch,
  WorkspaceStatus,
} from "./types";

const sessionId = "sess-mock-studio";
const artifactSessionId = "sess-mock-artifacts";

export const mockStatus: WorkspaceStatus = {
  workspace_name: "Mimir",
  git: {
    is_repo: true,
    branch: "phase6/memory-server-tui",
    dirty: true,
  },
  mimir: {
    initialized: true,
    config_present: false,
    checks_loaded: 1,
    sessions_count: 1,
    runs_count: 2,
    recent_runs: [
      {
        run_id: "run-demo",
        path: ".mimir/runs/run-demo",
        artifact_count: 3,
        has_context_packet: true,
        trace_status: { state: "recorded", redacted: true },
      },
      {
        run_id: "run-demo-explore",
        path: ".mimir/runs/run-demo-explore",
        artifact_count: 1,
        has_context_packet: false,
        trace_status: { state: "absent", redacted: false },
      },
    ],
  },
  providers: [
    { provider: "glm", models_count: 3, credential_detected: true },
    { provider: "anthropic", models_count: 4, credential_detected: true },
    { provider: "openai", models_count: 2, credential_detected: false },
    { provider: "openai-compatible", models_count: 1, credential_detected: false },
  ],
};

export const mockFiles: WorkspaceFileMatch[] = [
  { path: "crates/mimir-server/src/ui.rs", kind: "file", line: null, symbol: null },
  { path: "crates/mimir-session/src/lib.rs", kind: "file", line: null, symbol: null },
  { path: "docs/mimir-ui-product-plan.md", kind: "file", line: null, symbol: null },
  { path: "apps/studio/src/App.tsx", kind: "file", line: null, symbol: null },
  { path: "crates/mimir-server/src/ui.rs", kind: "symbol", line: 214, symbol: "ui_router" },
  {
    path: "crates/mimir-session/src/lib.rs",
    kind: "symbol",
    line: 294,
    symbol: "parse_session_command",
  },
];

export function mockSharePreview(targetRunId: string): SharePreviewResponse {
  return {
    run_id: targetRunId,
    packet_id: targetRunId === "run-demo" ? "ctx-demo" : `ctx-${targetRunId}`,
    packet_hash: "b".repeat(64),
    packet_path: `.mimir/runs/${targetRunId}/context_packet.json`,
    bundle_sha256: "c".repeat(64),
    redacted: true,
    bundle: {
      kind: "mimir.packet_share",
      run_id: targetRunId,
      packet_hash: "b".repeat(64),
      replay: { provider_request_sha256: "d".repeat(64) },
    },
  };
}

export function createMockSessions(): SessionCreateResponse[] {
  return [
    createMockSession(),
    createMockSession({
      sessionId: artifactSessionId,
      title: "Artifact review",
      minutesAgo: 38,
      variant: "artifacts",
    }),
  ];
}

export function createMockSession({
  sessionId: targetSessionId = sessionId,
  title = "Studio scaffold",
  minutesAgo = 12,
  variant = "context",
}: {
  sessionId?: string;
  title?: string;
  minutesAgo?: number;
  variant?: "context" | "artifacts";
} = {}): SessionCreateResponse {
  const metadata: SessionMetadata = {
    schema_version: 1,
    session_id: targetSessionId,
    title,
    workspace_name: mockStatus.workspace_name,
    created_at: new Date(Date.now() - 1000 * 60 * minutesAgo).toISOString(),
    updated_at: new Date(Date.now() - 1000 * 60 * Math.max(1, Math.floor(minutesAgo / 3))).toISOString(),
  };

  if (variant === "artifacts") {
    return {
      metadata,
      events: [
        event(0, "session.created", {
          title: metadata.title,
          workspace_name: metadata.workspace_name,
        }, targetSessionId),
        event(1, "turn.started", {
          turn_id: "turn-demo-artifacts",
          command: "runs",
          task: "inspect recent artifacts",
        }, targetSessionId),
        event(2, "artifact.written", {
          run_id: "run-demo-explore",
          artifact_kind: "explore_evidence",
          path: ".mimir/runs/run-demo-explore/explore_evidence.json",
        }, targetSessionId),
        event(3, "explore.completed", {
          run_id: "run-demo-explore",
          evidence_path: ".mimir/runs/run-demo-explore/explore_evidence.json",
          findings_count: 3,
          relevant_paths: ["apps/studio/src/App.tsx", "apps/studio/src/api/client.ts"],
          confidence: 0.82,
        }, targetSessionId),
        event(4, "turn.completed", {
          turn_id: "turn-demo-artifacts",
          summary: "Artifact review ready",
        }, targetSessionId),
      ],
    };
  }

  return {
    metadata,
    events: [
      event(0, "session.created", {
        title: metadata.title,
        workspace_name: metadata.workspace_name,
      }, targetSessionId),
      event(1, "turn.started", {
        turn_id: "turn-demo-status",
        command: "status",
        task: "",
      }, targetSessionId),
      event(2, "workspace.status.ready", { status: mockStatus }, targetSessionId),
      event(3, "turn.completed", {
        turn_id: "turn-demo-status",
        summary: "Workspace status loaded",
      }, targetSessionId),
      event(4, "turn.started", {
        turn_id: "turn-demo-context",
        command: "context",
        task: "scaffold Mimir Studio UI",
      }, targetSessionId),
      event(5, "context.build.started", {
        turn_id: "turn-demo-context",
        provider: "glm",
        model: "default",
      }, targetSessionId),
      event(6, "context.packet.ready", {
        run_id: "run-demo",
        packet_id: "ctx-demo",
        packet_hash: "a".repeat(64),
        packet_path: ".mimir/runs/run-demo/context_packet.json",
        estimated_input_tokens: 42112,
        guidance_files: ["AGENTS.md", "docs/HANDOFF.md"],
        likely_files: ["crates/mimir-server/src/ui.rs", "crates/mimir-session/src/lib.rs"],
      }, targetSessionId),
      event(7, "context.omission.risk", {
        run_id: "run-demo",
        path: "packages/sdk/index.d.ts",
        reason: "schema mirror may need future event type exports",
        risk: "schema_missing",
      }, targetSessionId),
      event(8, "artifact.written", {
        run_id: "run-demo",
        artifact_kind: "context_packet",
        path: ".mimir/runs/run-demo/context_packet.json",
      }, targetSessionId),
      event(9, "turn.completed", {
        turn_id: "turn-demo-context",
        summary: "Context packet ctx-demo is ready",
      }, targetSessionId),
    ],
  };
}

export async function runMockTurn(
  message: string,
  afterSequence: number,
  onEvents: (events: SessionEvent[]) => void,
  targetSessionId = sessionId,
  options: SessionMessageOptions = {},
): Promise<void> {
  const command = message.trim().startsWith("/") ? message.trim().slice(1).split(/\s+/, 1)[0] : "prompt";
  const task = message.replace(/^\/\w+\s*/, "").trim();
  const provider = options.provider?.trim() || "glm";
  const model = options.model?.trim() || "default";
  const turnId = `turn-${randomId()}`;
  let sequence = afterSequence + 1;
  let shouldComplete = true;

  const emit = async (events: SessionEvent[]) => {
    onEvents(events);
    await delay(140);
  };

  await emit([
    event(sequence++, "turn.started", {
      turn_id: turnId,
      command,
      task,
    }, targetSessionId),
  ]);

  if (command === "context" || command === "prompt") {
    await emit([
      event(sequence++, "context.build.started", {
        turn_id: turnId,
        provider,
        model,
      }, targetSessionId),
    ]);
    await emit([
      event(sequence++, "context.packet.ready", {
        run_id: "run-demo",
        packet_id: "ctx-demo",
        packet_hash: "b".repeat(64),
        packet_path: ".mimir/runs/run-demo/context_packet.json",
        estimated_input_tokens: 43820,
        guidance_files: ["AGENTS.md", "docs/HANDOFF.md", "docs/mimir-ui-product-plan.md"],
        likely_files: ["apps/studio/src/App.tsx", "crates/mimir-server/src/ui.rs"],
      }, targetSessionId),
      event(sequence++, "context.omission.risk", {
        run_id: "run-demo",
        path: "apps/studio/tests/studio-smoke.spec.ts",
        reason: "new UI smoke coverage should track composer behavior",
        risk: "test_missing",
      }, targetSessionId),
    ]);
  } else if (command === "status") {
    await emit([event(sequence++, "workspace.status.ready", { status: mockStatus }, targetSessionId)]);
  } else if (command === "check") {
    await emit([
      event(sequence++, "check.completed", {
        checks_loaded: 1,
        findings_count: 0,
        blocking_findings: 0,
        passed: true,
      }, targetSessionId),
    ]);
  } else if (command === "doctor") {
    await emit([
      event(sequence++, "doctor.completed", {
        status: "ok",
        warnings: 0,
        failures: 0,
      }, targetSessionId),
    ]);
  } else if (command === "init") {
    await emit([
      event(sequence++, "workspace.status.ready", {
        status: mockStatus,
        result: {
          created: [".mimir/config.yaml", ".mimir/project-rules.md"],
          status: mockStatus,
        },
      }, targetSessionId),
    ]);
  } else if (command === "why") {
    await emit([
      event(sequence++, "workspace.status.ready", {
        status: {
          path: task || "apps/studio/src/App.tsx",
          status: "included",
          reason: "included in the context packet",
          reason_code: "semantic_match",
          run_id: "run-demo",
          packet_id: "ctx-demo",
          packet_hash: "b".repeat(64),
          packet_path: ".mimir/runs/run-demo/context_packet.json",
        },
      }, targetSessionId),
    ]);
  } else if (command === "runs") {
    await emit([
      event(sequence++, "workspace.status.ready", {
        status: {
          runs: mockStatus.mimir.recent_runs,
        },
      }, targetSessionId),
    ]);
  } else if (command === "explore") {
    await emit([
      event(sequence++, "artifact.written", {
        run_id: "run-demo-explore",
        artifact_kind: "explore_evidence",
        path: ".mimir/runs/run-demo-explore/explore_evidence.json",
      }, targetSessionId),
      event(sequence++, "explore.completed", {
        run_id: "run-demo-explore",
        evidence_path: ".mimir/runs/run-demo-explore/explore_evidence.json",
        findings_count: 3,
        relevant_paths: ["apps/studio/src/App.tsx", "crates/mimir-server/src/ui.rs"],
        confidence: 0.82,
      }, targetSessionId),
    ]);
  } else if (command === "help") {
    await emit([
      event(sequence++, "workspace.status.ready", {
        status: {
          commands: [
            "/help",
            "/status",
            "/init",
            "/doctor",
            "/check",
            "/explore <question>",
            "/context <task>",
            "/why <path>",
            "/runs",
          ],
        },
      }, targetSessionId),
    ]);
  } else {
    shouldComplete = false;
    await emit([
      event(sequence++, "turn.failed", {
        turn_id: turnId,
        error: `${command} is not available in mock mode`,
      }, targetSessionId),
    ]);
  }

  if (shouldComplete) {
    await emit([
      event(sequence, "turn.completed", {
        turn_id: turnId,
        summary: `${command} completed`,
      }, targetSessionId),
    ]);
  }
}

function event(
  typeSequence: number,
  type: SessionEvent["type"],
  payload: SessionEvent["payload"],
  targetSessionId = sessionId,
): SessionEvent {
  return {
    schema_version: 1,
    event_id: `${targetSessionId}-evt-${typeSequence}-${randomId()}`,
    session_id: targetSessionId,
    sequence: typeSequence,
    timestamp: new Date().toISOString(),
    type,
    payload,
  };
}

function randomId(): string {
  return globalThis.crypto?.randomUUID?.().slice(0, 8) ?? Math.random().toString(16).slice(2, 10);
}

function delay(ms: number): Promise<void> {
  return new Promise((resolve) => window.setTimeout(resolve, ms));
}
