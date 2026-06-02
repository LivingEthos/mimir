import { create } from "zustand";
import type {
  RuntimeConfig,
  SessionEvent,
  SessionMetadata,
  WorkspaceFileMatch,
  WorkspaceStatus,
} from "../api/types";

export type InspectorTab = "context" | "artifacts" | "approvals" | "provider";
export type RouteName = "session" | "settings";
export type ConnectionState = "mock" | "connecting" | "open" | "closed" | "error";
export type ApprovalPolicy = "ask" | "read-only" | "session";
export type ThemePreference = "system" | "dark" | "light";

export interface StudioSettings {
  provider: string;
  model: string;
  approvalPolicy: ApprovalPolicy;
  tokenCap: number;
  costCapUsd: number;
  theme: ThemePreference;
}

interface StudioState {
  runtime: RuntimeConfig;
  settings: StudioSettings;
  route: RouteName;
  connection: ConnectionState;
  activeSession: SessionMetadata | null;
  sessions: SessionMetadata[];
  events: SessionEvent[];
  status: WorkspaceStatus | null;
  files: WorkspaceFileMatch[];
  inspectorTab: InspectorTab;
  pending: boolean;
  error: string | null;
  setRuntime: (runtime: RuntimeConfig) => void;
  setSettings: (settings: Partial<StudioSettings>) => void;
  setRoute: (route: RouteName) => void;
  setConnection: (connection: ConnectionState) => void;
  setSession: (session: SessionMetadata | null) => void;
  setSessions: (sessions: SessionMetadata[]) => void;
  setEvents: (events: SessionEvent[]) => void;
  appendEvents: (events: SessionEvent[]) => void;
  setStatus: (status: WorkspaceStatus | null) => void;
  setFiles: (files: WorkspaceFileMatch[]) => void;
  setInspectorTab: (tab: InspectorTab) => void;
  setPending: (pending: boolean) => void;
  setError: (error: string | null) => void;
}

export const useStudioStore = create<StudioState>((set) => ({
  runtime: { mode: "mock", apiBaseUrl: "", token: null },
  settings: {
    provider: "glm",
    model: "default",
    approvalPolicy: "ask",
    tokenCap: 64000,
    costCapUsd: 2,
    theme: "system",
  },
  route: "session",
  connection: "mock",
  activeSession: null,
  sessions: [],
  events: [],
  status: null,
  files: [],
  inspectorTab: "context",
  pending: false,
  error: null,
  setRuntime: (runtime) => set({ runtime, connection: runtime.mode === "mock" ? "mock" : "connecting" }),
  setSettings: (settings) =>
    set((state) => ({ settings: { ...state.settings, ...settings } })),
  setRoute: (route) => set({ route }),
  setConnection: (connection) => set({ connection }),
  setSession: (activeSession) => set({ activeSession }),
  setSessions: (sessions) => set({ sessions }),
  setEvents: (events) => set({ events: sortEvents(dedupeEvents(events)) }),
  appendEvents: (incoming) =>
    set((state) => ({
      events: sortEvents(dedupeEvents([...state.events, ...incoming])),
    })),
  setStatus: (status) => set({ status }),
  setFiles: (files) => set({ files }),
  setInspectorTab: (inspectorTab) => set({ inspectorTab }),
  setPending: (pending) => set({ pending }),
  setError: (error) => set({ error }),
}));

function dedupeEvents(events: SessionEvent[]): SessionEvent[] {
  const order: string[] = [];
  const byKey = new Map<string, SessionEvent>();

  for (const event of events) {
    const key = event.event_id || `${event.session_id}:${event.sequence}`;
    const existing = byKey.get(key);
    if (!existing) {
      order.push(key);
      byKey.set(key, event);
      continue;
    }
    if (hasResultPayload(event) && !hasResultPayload(existing)) {
      byKey.set(key, event);
    }
  }

  return order.flatMap((key) => {
    const event = byKey.get(key);
    return event ? [event] : [];
  });
}

function sortEvents(events: SessionEvent[]): SessionEvent[] {
  return [...events].sort((left, right) => left.sequence - right.sequence);
}

function hasResultPayload(event: SessionEvent): boolean {
  return Boolean(
    event.payload &&
      typeof event.payload === "object" &&
      !Array.isArray(event.payload) &&
      "result" in event.payload,
  );
}
