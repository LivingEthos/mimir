import {
  AlertTriangle,
  Archive,
  Bot,
  CheckCircle2,
  ChevronRight,
  CircleDot,
  Code2,
  FileCode2,
  Files,
  Gauge,
  GitBranch,
  HardDrive,
  History,
  KeyRound,
  LayoutDashboard,
  ListChecks,
  Loader2,
  PanelRight,
  Pause,
  Palette,
  Play,
  RotateCcw,
  Search,
  Send,
  Settings,
  Share2,
  ShieldCheck,
  Square,
  TerminalSquare,
} from "lucide-react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { createStudioApiClient, readRuntimeConfig, type StudioApiClient } from "./api/client";
import { createMockSessions, mockFiles, mockSharePreview, mockStatus, runMockTurn } from "./api/mockEvents";
import type {
  ArtifactContentResponse,
  ArtifactSummary,
  CommandMetadata,
  ContextWhyResult,
  InitResult,
  ReplayPreviewResponse,
  RuntimeConfig,
  RunSummary,
  SessionCreateResponse,
  SessionEvent,
  SessionMetadata,
  SessionMessageOptions,
  SharePreviewResponse,
  TraceStatus,
  WorkspaceFileMatch,
  WorkspaceStatus,
} from "./api/types";
import {
  artifactEvents,
  contextPressure,
  latestInitResult,
  latestContext,
  latestRunsResult,
  latestTurnSummary,
  latestWhyResult,
  toArtifactSummaries,
  workspaceStatusFromEvents,
} from "./state/derive";
import {
  useStudioStore,
  type InspectorTab,
  type RouteName,
  type StudioSettings,
} from "./stores/studioStore";

const maxArtifactPreviewChars = 4_000;
type PacketActionPreviewState =
  | { kind: "replay"; data: ReplayPreviewResponse }
  | { kind: "share"; data: SharePreviewResponse };

const fallbackCommandRegistry: CommandMetadata[] = [
  { name: "/help", usage: "/help", summary: "Command registry", support: "backend", takes_input: false, enabled: true },
  { name: "/status", usage: "/status", summary: "Workspace readiness", support: "backend", takes_input: false, enabled: true },
  { name: "/init", usage: "/init", summary: "Seed project workflow files", support: "backend", takes_input: false, enabled: true },
  { name: "/doctor", usage: "/doctor", summary: "Local environment checks", support: "backend", takes_input: false, enabled: true },
  { name: "/check", usage: "/check", summary: "Source-controlled checks", support: "backend", takes_input: false, enabled: true },
  { name: "/explore", usage: "/explore <question>", summary: "Provider-free evidence search", support: "backend", takes_input: true, enabled: true },
  { name: "/context", usage: "/context <task>", summary: "Build a context packet", support: "backend", takes_input: true, enabled: true },
  { name: "/why", usage: "/why <path>", summary: "Explain context inclusion", support: "backend", takes_input: true, enabled: true },
  { name: "/runs", usage: "/runs", summary: "List local runs", support: "backend", takes_input: false, enabled: true },
  { name: "/settings", usage: "/settings", summary: "Open settings", support: "local", takes_input: false, enabled: true },
  { name: "/resume", usage: "/resume [session]", summary: "Resume a session", support: "local", takes_input: true, enabled: true },
  {
    name: "/plan",
    usage: "/plan <task>",
    summary: "Provider-backed plan mode",
    support: "planned",
    takes_input: true,
    enabled: false,
    disabled_reason: "provider-backed plan mode is not wired in Studio yet",
  },
  {
    name: "/code",
    usage: "/code <task>",
    summary: "Provider-backed edit mode",
    support: "planned",
    takes_input: true,
    enabled: false,
    disabled_reason: "provider-backed code mode needs explicit editable target UI first",
  },
  {
    name: "/share",
    usage: "/share <run>",
    summary: "Preview a redacted packet-share bundle",
    support: "local",
    takes_input: true,
    enabled: true,
    disabled_reason: null,
  },
  {
    name: "/diff",
    usage: "/diff",
    summary: "Git diff inspector",
    support: "planned",
    takes_input: false,
    enabled: false,
    disabled_reason: "diff inspection is planned for a later Studio slice",
  },
];
const preferredCommandChips = ["/help", "/status", "/init", "/context", "/why", "/runs", "/resume"];
const modes = ["Ask", "Explore", "Plan", "Code", "Review"];

export function App() {
  const [client, setClient] = useState<StudioApiClient | null>(null);
  const [runtimeReady, setRuntimeReady] = useState(false);
  const runtime = useStudioStore((state) => state.runtime);
  const settings = useStudioStore((state) => state.settings);
  const route = useStudioStore((state) => state.route);
  const connection = useStudioStore((state) => state.connection);
  const activeSession = useStudioStore((state) => state.activeSession);
  const sessions = useStudioStore((state) => state.sessions);
  const events = useStudioStore((state) => state.events);
  const status = useStudioStore((state) => state.status);
  const files = useStudioStore((state) => state.files);
  const pending = useStudioStore((state) => state.pending);
  const error = useStudioStore((state) => state.error);
  const setRuntime = useStudioStore((state) => state.setRuntime);
  const setSettings = useStudioStore((state) => state.setSettings);
  const setRoute = useStudioStore((state) => state.setRoute);
  const setConnection = useStudioStore((state) => state.setConnection);
  const setSession = useStudioStore((state) => state.setSession);
  const setSessions = useStudioStore((state) => state.setSessions);
  const setEvents = useStudioStore((state) => state.setEvents);
  const appendEvents = useStudioStore((state) => state.appendEvents);
  const setStatus = useStudioStore((state) => state.setStatus);
  const setFiles = useStudioStore((state) => state.setFiles);
  const setInspectorTab = useStudioStore((state) => state.setInspectorTab);
  const setPending = useStudioStore((state) => state.setPending);
  const setError = useStudioStore((state) => state.setError);
  const [sessionLoadingId, setSessionLoadingId] = useState<string | null>(null);
  const [packetCommandPreview, setPacketCommandPreview] =
    useState<PacketActionPreviewState | null>(null);
  const [packetCommandError, setPacketCommandError] = useState<string | null>(null);
  const sessionSequencesRef = useRef<Map<string, number>>(new Map());
  const loadRequestRef = useRef(0);
  const mockSessionsRef = useRef<SessionCreateResponse[]>([]);
  const commandRegistry = useMemo(() => mergeCommandRegistry(status?.commands), [status?.commands]);

  useEffect(() => {
    const config = readRuntimeConfig();
    setRuntime(config);
    setClient(config.mode === "api" ? createStudioApiClient(config) : null);
    setRuntimeReady(true);
  }, [setRuntime]);

  useEffect(() => {
    if (!activeSession) {
      return;
    }

    const latestSequence = events.reduce(
      (sequence, event) =>
        event.session_id === activeSession.session_id ? Math.max(sequence, event.sequence) : sequence,
      0,
    );
    const knownSequence = sessionSequencesRef.current.get(activeSession.session_id) ?? 0;
    if (latestSequence > knownSequence) {
      sessionSequencesRef.current.set(activeSession.session_id, latestSequence);
    }
  }, [activeSession, events]);

  useEffect(() => {
    setPacketCommandPreview(null);
    setPacketCommandError(null);
  }, [activeSession?.session_id]);

  useEffect(() => {
    if (!runtimeReady) {
      return;
    }

    if (runtime.mode === "mock") {
      const sessions = createMockSessions();
      const session = sessions[0];
      mockSessionsRef.current = sessions;
      sessionSequencesRef.current = new Map(
        sessions.map((item) => [item.metadata.session_id, item.events.at(-1)?.sequence ?? 0]),
      );
      setSession(session.metadata);
      setSessions(sessions.map((item) => item.metadata));
      setEvents(session.events);
      setStatus(mockStatus);
      setFiles(mockFiles);
      setConnection("mock");
      return;
    }

    if (!client) {
      return;
    }

    let cancelled = false;

    async function connectApi() {
      if (!client) {
        return;
      }

      try {
        setConnection("connecting");
        setError(null);
        const [workspace, listed, fileMatches] = await Promise.all([
          client.workspaceStatus(),
          client.listSessions(),
          client.searchFiles("").catch(() => ({ results: [] })),
        ]);
        if (cancelled) {
          return;
        }
        setStatus(workspace);
        setFiles(fileMatches.results);

        const loaded =
          listed[0] == null
            ? await client.createSession("Mimir Studio session")
            : await client.loadSession(listed[0].session_id);
        if (cancelled) {
          return;
        }

        sessionSequencesRef.current.set(
          loaded.metadata.session_id,
          loaded.events.at(-1)?.sequence ?? 0,
        );
        setSession(loaded.metadata);
        setSessions(ensureSessionListed(listed, loaded.metadata));
        setEvents(loaded.events);
      } catch (connectError) {
        if (!cancelled) {
          setConnection("error");
          setError(connectError instanceof Error ? connectError.message : "Unable to connect");
        }
      }
    }

    void connectApi();

    return () => {
      cancelled = true;
    };
  }, [
    client,
    runtimeReady,
    runtime.mode,
    setConnection,
    setError,
    setEvents,
    setFiles,
    setSession,
    setSessions,
    setStatus,
  ]);

  useEffect(() => {
    if (!runtimeReady || runtime.mode !== "api" || !client || !activeSession) {
      return;
    }

    let cancelled = false;
    let socket: WebSocket | null = null;
    let reconnectTimer: number | null = null;
    const sessionId = activeSession.session_id;

    const scheduleReconnect = () => {
      if (cancelled || reconnectTimer) {
        return;
      }

      reconnectTimer = window.setTimeout(() => {
        reconnectTimer = null;
        connectStream();
      }, 1_000);
    };

    const connectStream = () => {
      if (cancelled) {
        return;
      }

      setConnection("connecting");
      const after = sessionSequencesRef.current.get(sessionId) ?? 0;
      socket = client.openEventStream(
        sessionId,
        after,
        (event) => {
          if (event.session_id !== sessionId) {
            return;
          }
          const nextSequence = Math.max(
            sessionSequencesRef.current.get(sessionId) ?? 0,
            event.sequence,
          );
          sessionSequencesRef.current.set(sessionId, nextSequence);
          appendEvents([event]);
        },
        (status) => {
          if (cancelled) {
            return;
          }
          setConnection(status);
          if (status === "closed" || status === "error") {
            socket = null;
            scheduleReconnect();
          }
        },
      );

      if (!socket) {
        scheduleReconnect();
      }
    };

    connectStream();

    return () => {
      cancelled = true;
      if (reconnectTimer) {
        window.clearTimeout(reconnectTimer);
      }
      socket?.close();
    };
  }, [
    activeSession,
    appendEvents,
    client,
    runtime.mode,
    runtimeReady,
    setConnection,
  ]);

  useEffect(() => {
    const nextStatus = workspaceStatusFromEvents(events);
    if (nextStatus) {
      setStatus(nextStatus);
    }
  }, [events, setStatus]);

  useEffect(() => {
    const updateRoute = () => {
      setRoute(window.location.hash === "#/settings" ? "settings" : "session");
    };
    updateRoute();
    window.addEventListener("hashchange", updateRoute);
    return () => window.removeEventListener("hashchange", updateRoute);
  }, [setRoute]);

  const selectSession = useCallback(
    async (sessionId: string) => {
      if (!sessionId) {
        return;
      }

      setRoute("session");
      if (sessionId === activeSession?.session_id) {
        return;
      }

      setError(null);

      if (runtime.mode === "mock") {
        const nextSession = mockSessionsRef.current.find(
          (session) => session.metadata.session_id === sessionId,
        );
        if (!nextSession) {
          setError("Mock session is no longer available");
          return;
        }
        sessionSequencesRef.current.set(
          nextSession.metadata.session_id,
          nextSession.events.at(-1)?.sequence ?? 0,
        );
        setSession(nextSession.metadata);
        setEvents(nextSession.events);
        setConnection("mock");
        return;
      }

      if (!client) {
        setError("No API client available");
        return;
      }

      const requestId = loadRequestRef.current + 1;
      loadRequestRef.current = requestId;
      setSessionLoadingId(sessionId);
      setConnection("connecting");

      try {
        const loaded = await client.loadSession(sessionId);
        if (loadRequestRef.current !== requestId) {
          return;
        }

        sessionSequencesRef.current.set(
          loaded.metadata.session_id,
          loaded.events.at(-1)?.sequence ?? 0,
        );
        setSession(loaded.metadata);
        setSessions(ensureSessionListed(sessions, loaded.metadata));
        setEvents(loaded.events);
      } catch (loadError) {
        if (loadRequestRef.current === requestId) {
          setConnection("error");
          setError(loadError instanceof Error ? loadError.message : "Unable to load session");
        }
      } finally {
        if (loadRequestRef.current === requestId) {
          setSessionLoadingId(null);
        }
      }
    },
    [
      activeSession?.session_id,
      client,
      runtime.mode,
      sessions,
      setConnection,
      setError,
      setEvents,
      setRoute,
      setSession,
      setSessions,
    ],
  );

  const submitMessage = useCallback(
    async (message: string) => {
      const trimmed = message.trim();
      if (!trimmed || pending) {
        return;
      }

      const command = commandName(trimmed);
      const spec = commandSpec(command, commandRegistry);

      if (command === "/settings") {
        setError(null);
        setRoute("settings");
        return;
      }

      if (command === "/resume") {
        const query = resumeQueryFromCommand(trimmed);
        const target = findResumeSession(
          sessions,
          query,
          activeSession?.session_id ?? null,
        );
        setRoute("session");
        if (!target) {
          setError(query ? `No session matches "${query}"` : "No resumable sessions");
          return;
        }
        setError(null);
        await selectSession(target.session_id);
        return;
      }

      if (command === "/share") {
        const runId = shareTargetFromCommand(trimmed);
        setRoute("session");
        setInspectorTab("context");
        setPacketCommandPreview(null);
        setPacketCommandError(null);
        if (!runId) {
          const message = "/share requires a run id, for example /share run-demo";
          setPacketCommandError(message);
          setError(message);
          return;
        }

        try {
          setPending(true);
          setError(null);
          const data =
            runtime.mode === "mock"
              ? mockSharePreview(runId)
              : await clientRequired(client).fetchShare(runId);
          setPacketCommandPreview({ kind: "share", data });
        } catch (shareError) {
          const message =
            shareError instanceof Error ? shareError.message : "Unable to preview packet share";
          setPacketCommandError(message);
          setError(message);
        } finally {
          setPending(false);
        }
        return;
      }

      if (trimmed.startsWith("/") && !spec) {
        setError(`${command} is not a recognized Mimir Studio command`);
        return;
      }

      if (spec && (!spec.enabled || spec.support === "planned")) {
        setError(commandUnavailableMessage(spec));
        return;
      }

      try {
        setPending(true);
        setError(null);
        if (runtime.mode === "mock") {
          await runMockTurn(
            trimmed,
            events.at(-1)?.sequence ?? 0,
            appendEvents,
            activeSession?.session_id,
            commandSubmitOptions(settings),
          );
          if (command === "/runs") {
            setInspectorTab("artifacts");
          }
          return;
        }

        if (!client || !activeSession) {
          throw new Error("No active API session");
        }

        const response = await client.submitMessage(
          activeSession.session_id,
          trimmed,
          commandSubmitOptions(settings),
        );
        appendEvents(attachCommandResult(response.events, response.command, response.result));
        if (response.command === "runs") {
          setInspectorTab("artifacts");
        }
      } catch (submitError) {
        setError(submitError instanceof Error ? submitError.message : "Message failed");
      } finally {
        setPending(false);
      }
    },
    [
      activeSession,
      appendEvents,
      client,
      events,
      pending,
      runtime.mode,
      selectSession,
      sessions,
      settings,
      commandRegistry,
      setError,
      setInspectorTab,
      setRoute,
      setPending,
    ],
  );

  const searchFiles = useCallback(
    async (query: string) => {
      if (runtime.mode === "mock" && !query.trim()) {
        setFiles(runtime.mode === "mock" ? mockFiles : []);
        return;
      }

      if (runtime.mode === "mock") {
        const lowered = query.toLowerCase();
        setFiles(
          mockFiles.filter(
            (item) =>
              item.path.toLowerCase().includes(lowered) ||
              item.symbol?.toLowerCase().includes(lowered),
          ),
        );
        return;
      }

      if (!client) {
        return;
      }

      try {
        const response = await client.searchFiles(query);
        setFiles(response.results);
      } catch {
        setFiles([]);
      }
    },
    [client, runtime.mode, setFiles],
  );

  return (
    <div className="studio-root" data-testid="studio-shell" data-theme={settings.theme}>
      <Header
        connection={connection}
        events={events}
        runtime={runtime}
        settings={settings}
        status={status}
      />
      <main className="studio-grid">
        <LeftRail
          activeSessionId={activeSession?.session_id ?? null}
          activeRoute={route}
          files={files}
          loadingSessionId={sessionLoadingId}
          onNavigate={setRoute}
          onSessionSelect={selectSession}
          sessions={sessions}
          status={status}
        />
        {route === "settings" ? (
          <SettingsView
            runtime={runtime}
            settings={settings}
            status={status}
            onSettingsChange={setSettings}
          />
        ) : (
          <SessionView
            activeSessionId={activeSession?.session_id ?? null}
            commandRegistry={commandRegistry}
            error={error}
            events={events}
            files={files}
            onFileSearch={searchFiles}
            onResumeSession={selectSession}
            onSubmit={submitMessage}
            pending={pending}
            runtime={runtime}
            sessions={sessions}
            status={status}
          />
        )}
        <Inspector
          client={client}
          events={events}
          packetCommandError={packetCommandError}
          packetCommandPreview={packetCommandPreview}
          status={status}
          onSubmit={submitMessage}
        />
      </main>
    </div>
  );
}

function Header({
  connection,
  events,
  runtime,
  settings,
  status,
}: {
  connection: string;
  events: SessionEvent[];
  runtime: RuntimeConfig;
  settings: StudioSettings;
  status: WorkspaceStatus | null;
}) {
  const context = latestContext(events);
  const summary = latestTurnSummary(events);
  const pressure = contextPressure(context, settings.tokenCap);
  const branch = status?.git.branch ?? "detached";
  const dirty = status?.git.dirty ?? false;

  return (
    <header className="topbar" role="banner">
      <div className="brand-lockup">
        <div className="brand-mark" aria-hidden="true">
          <CircleDot size={18} />
        </div>
        <div>
          <div className="brand-title">Mimir Studio</div>
          <div className="microcopy">{summary}</div>
        </div>
      </div>

      <div className="mode-switch" aria-label="Mode">
        {modes.map((mode) => (
          <button className={mode === "Explore" ? "mode active" : "mode"} key={mode} type="button">
            {mode}
          </button>
        ))}
      </div>

      <div className="status-strip">
        <StatusPill icon={<GitBranch size={14} />} label={branch} tone={dirty ? "warn" : "ok"} />
        <StatusPill
          icon={<ShieldCheck size={14} />}
          label={runtime.mode === "mock" ? "mock stream" : connection}
          tone={connection === "error" ? "danger" : "ok"}
        />
        <StatusPill icon={<Bot size={14} />} label={settings.provider} tone="neutral" />
        <div className="budget-meter" aria-label="Context budget">
          <span>
            {context
              ? `${formatTokenCount(context.estimatedInputTokens)} / ${formatTokenCount(settings.tokenCap)}`
              : `0k / ${formatTokenCount(settings.tokenCap)}`}
          </span>
          <div className="meter-track">
            <div className="meter-fill" style={{ width: `${pressure}%` }} />
          </div>
        </div>
      </div>
    </header>
  );
}

function StatusPill({
  icon,
  label,
  tone,
}: {
  icon: React.ReactNode;
  label: string;
  tone: "ok" | "warn" | "danger" | "neutral";
}) {
  return (
    <span className={`status-pill ${tone}`}>
      {icon}
      <span>{label}</span>
    </span>
  );
}

function LeftRail({
  activeSessionId,
  activeRoute,
  sessions,
  status,
  files,
  loadingSessionId,
  onNavigate,
  onSessionSelect,
}: {
  activeSessionId: string | null;
  activeRoute: RouteName;
  sessions: Array<{ session_id: string; title: string; updated_at: string }>;
  status: WorkspaceStatus | null;
  files: WorkspaceFileMatch[];
  loadingSessionId: string | null;
  onNavigate: (route: RouteName) => void;
  onSessionSelect: (sessionId: string) => void;
}) {
  const navItems: Array<{ route: RouteName; label: string; icon: React.ReactNode; href: string }> = [
    { route: "session", label: "Sessions", icon: <History size={17} />, href: "#/" },
    { route: "settings", label: "Settings", icon: <Settings size={17} />, href: "#/settings" },
  ];

  return (
    <aside className="left-rail">
      <nav className="rail-nav" aria-label="Workspace">
        {navItems.map((item) => (
          <a
            className={activeRoute === item.route ? "rail-button active" : "rail-button"}
            href={item.href}
            key={item.label}
            onClick={() => onNavigate(item.route)}
          >
            {item.icon}
            <span>{item.label}</span>
          </a>
        ))}
      </nav>

      <section className="rail-section">
        <div className="rail-heading">
          <LayoutDashboard size={14} />
          <span>Readiness</span>
        </div>
        <div className="readiness-list">
          <ReadinessRow label="Git" value={status?.git.is_repo ? "repo" : "missing"} ok={Boolean(status?.git.is_repo)} />
          <ReadinessRow label="Mimir" value={status?.mimir.initialized ? "ready" : "init"} ok={Boolean(status?.mimir.initialized)} />
          <ReadinessRow label="Checks" value={String(status?.mimir.checks_loaded ?? 0)} ok={(status?.mimir.checks_loaded ?? 0) > 0} />
        </div>
      </section>

      <section className="rail-section">
        <div className="rail-heading">
          <History size={14} />
          <span>Sessions</span>
        </div>
        <div className="session-list">
          {sessions.map((session) => (
            <button
              aria-current={session.session_id === activeSessionId ? "page" : undefined}
              className={session.session_id === activeSessionId ? "session-row active" : "session-row"}
              data-testid={`session-row-${session.session_id}`}
              disabled={loadingSessionId === session.session_id}
              key={session.session_id}
              onClick={() => onSessionSelect(session.session_id)}
              type="button"
            >
              <span>{session.title}</span>
              {loadingSessionId === session.session_id ? (
                <Loader2 className="spin" size={13} />
              ) : (
                <time>{compactTime(session.updated_at)}</time>
              )}
            </button>
          ))}
        </div>
      </section>

      <section className="rail-section files-section">
        <div className="rail-heading">
          <Files size={14} />
          <span>Files</span>
        </div>
        <div className="file-list">
          {files.slice(0, 6).map((file) => (
            <div className="file-row" key={`${file.kind}:${file.path}:${file.symbol ?? ""}`}>
              <FileCode2 size={13} />
              <span>{redactForDisplay(file.symbol ?? file.path)}</span>
            </div>
          ))}
        </div>
      </section>
    </aside>
  );
}

function ReadinessRow({ label, value, ok }: { label: string; value: string; ok: boolean }) {
  return (
    <div className="readiness-row">
      {ok ? <CheckCircle2 size={14} /> : <AlertTriangle size={14} />}
      <span>{label}</span>
      <strong>{value}</strong>
    </div>
  );
}

function SessionView({
  activeSessionId,
  commandRegistry,
  error,
  events,
  files,
  pending,
  runtime,
  sessions,
  status,
  onFileSearch,
  onResumeSession,
  onSubmit,
}: {
  activeSessionId: string | null;
  commandRegistry: CommandMetadata[];
  error: string | null;
  events: SessionEvent[];
  files: WorkspaceFileMatch[];
  pending: boolean;
  runtime: RuntimeConfig;
  sessions: SessionMetadata[];
  status: WorkspaceStatus | null;
  onFileSearch: (query: string) => void;
  onResumeSession: (sessionId: string) => Promise<void>;
  onSubmit: (message: string) => Promise<void>;
}) {
  return (
    <section className="center-pane">
      <Transcript
        commandRegistry={commandRegistry}
        events={events}
        status={status}
        onInit={() => onSubmit("/init")}
      />
      {runtime.mode === "api" && !status && error ? <ApiRecoveryPanel error={error} /> : null}
      {runtime.mode === "api" && !activeSessionId ? <NoSessionPanel /> : null}
      {error ? <div className="error-strip">{redactForDisplay(error)}</div> : null}
      <Composer
        activeSessionId={activeSessionId}
        commandRegistry={commandRegistry}
        files={files}
        onFileSearch={onFileSearch}
        onResumeSession={onResumeSession}
        onSubmit={onSubmit}
        pending={pending}
        runtime={runtime}
        sessions={sessions}
      />
    </section>
  );
}

function Transcript({
  commandRegistry,
  events,
  status,
  onInit,
}: {
  commandRegistry: CommandMetadata[];
  events: SessionEvent[];
  status: WorkspaceStatus | null;
  onInit: () => Promise<void>;
}) {
  const bottomRef = useRef<HTMLDivElement | null>(null);
  const initResult = latestInitResult(events);
  const runsResult = latestRunsResult(events);

  useEffect(() => {
    bottomRef.current?.scrollIntoView({ block: "end" });
  }, [events.length]);

  return (
    <section className="transcript" data-testid="transcript" aria-label="Transcript">
      <div className="workspace-band">
        <div>
          <span className="eyebrow">Workspace</span>
          <strong>{status?.workspace_name ?? "Mimir"}</strong>
        </div>
        <div className="workspace-stats">
          <Metric icon={<HardDrive size={15} />} label="runs" value={String(status?.mimir.runs_count ?? status?.mimir.recent_runs?.length ?? 0)} />
          <Metric icon={<ListChecks size={15} />} label="checks" value={String(status?.mimir.checks_loaded ?? 0)} />
          <Metric icon={<KeyRound size={15} />} label="providers" value={String(status?.providers.filter((item) => item.credential_detected).length ?? 0)} />
        </div>
      </div>

      <ReadinessPanel
        commands={commandRegistry}
        initResult={initResult}
        runs={runsResult?.runs ?? status?.mimir.recent_runs ?? []}
        status={status}
        onInit={onInit}
      />

      <div className="event-stack">
        {events.map((event) => (
          <EventCell commandRegistry={commandRegistry} event={event} key={event.event_id} />
        ))}
        <div ref={bottomRef} />
      </div>
    </section>
  );
}

function ApiRecoveryPanel({ error }: { error: string }) {
  return (
    <section className="empty-state api-recovery" data-testid="api-recovery-state">
      <div className="quiet-row">
        <AlertTriangle size={14} />
        <span>Local API disconnected: {redactForDisplay(error)}</span>
      </div>
      <div className="quiet-row">
        <TerminalSquare size={14} />
        <span>Start Mimir with `mimir serve --ui` or relaunch with `mimir ui`.</span>
      </div>
    </section>
  );
}

function NoSessionPanel() {
  return (
    <section className="empty-state" data-testid="no-session-state">
      <div className="quiet-row">
        <Pause size={14} />
        <span>No active Studio session is available yet.</span>
      </div>
      <div className="quiet-row">
        <History size={14} />
        <span>Reconnect the local API or create a session from the server before sending a command.</span>
      </div>
    </section>
  );
}

function Metric({ icon, label, value }: { icon: React.ReactNode; label: string; value: string }) {
  return (
    <span className="metric">
      {icon}
      <strong>{value}</strong>
      <span>{label}</span>
    </span>
  );
}

function ReadinessPanel({
  commands,
  initResult,
  runs,
  status,
  onInit,
}: {
  commands: CommandMetadata[];
  initResult: InitResult | null;
  runs: RunSummary[];
  status: WorkspaceStatus | null;
  onInit: () => Promise<void>;
}) {
  const detectedProviders = status?.providers.filter((provider) => provider.credential_detected) ?? [];
  const missingProviders = status?.providers.filter((provider) => !provider.credential_detected) ?? [];
  const initialized = Boolean(status?.mimir.initialized);
  const backendCommands = commands.filter((command) => command.support === "backend" && command.enabled);

  return (
    <section className="readiness-panel" data-testid="readiness-panel">
      <div className="readiness-panel-head">
        <div>
          <span className="eyebrow">First launch readiness</span>
          <strong>{initialized ? "Workspace ready for context work" : "Initialize Mimir to unlock packets"}</strong>
        </div>
        {!initialized ? (
          <button className="compact-action" onClick={() => void onInit()} type="button">
            <Play size={14} />
            <span>/init</span>
          </button>
        ) : null}
      </div>
      <div className="readiness-grid">
        <ReadinessMetric
          label="Git"
          value={status?.git.is_repo ? `${status.git.branch ?? "detached"} ${status.git.dirty ? "dirty" : "clean"}` : "not a repo"}
          ok={Boolean(status?.git.is_repo)}
        />
        <ReadinessMetric
          label="Mimir"
          value={`${initialized ? "initialized" : "missing"} / ${status?.mimir.config_present ? "config" : "no config"}`}
          ok={initialized && Boolean(status?.mimir.config_present)}
        />
        <ReadinessMetric
          label="Checks"
          value={`${status?.mimir.checks_loaded ?? 0} loaded`}
          ok={(status?.mimir.checks_loaded ?? 0) > 0}
        />
        <ReadinessMetric
          label="Runs"
          value={`${runs.length} recent / ${status?.mimir.sessions_count ?? 0} sessions`}
          ok={runs.length > 0}
        />
        <ReadinessMetric
          label="Commands"
          value={`${backendCommands.length} backend / ${commands.filter((command) => !command.enabled).length} planned`}
          ok={backendCommands.length > 0}
        />
      </div>
      {runs.length > 0 ? (
        <div className="run-summary-list readiness-runs">
          {runs.slice(0, 3).map((run) => (
            <div className="command-detail-row" key={run.run_id}>
              <strong>{run.run_id}</strong>
              <span>{run.artifact_count} artifacts / {run.has_context_packet ? "packet" : "no packet"}</span>
            </div>
          ))}
        </div>
      ) : null}
      <div className="provider-readiness">
        <span>Providers</span>
        <div className="provider-chip-row">
          {(status?.providers ?? []).map((provider) => (
            <span
              className={provider.credential_detected ? "provider-chip ok" : "provider-chip missing"}
              key={provider.provider}
            >
              {provider.provider}
              <small>{provider.credential_detected ? "detected" : "not detected"}</small>
            </span>
          ))}
          {status && status.providers.length === 0 ? <span className="provider-chip missing">none<small>not detected</small></span> : null}
        </div>
      </div>
      <div className="next-actions">
        {!status?.git.is_repo ? <span>Open Mimir Studio from a Git repository for branch and dirty-state checks.</span> : null}
        {!initialized ? <span>Run /init to create local workflow files without overwriting existing files.</span> : null}
        {initialized && !status?.mimir.config_present ? <span>Run /init again to seed missing config files.</span> : null}
        {missingProviders.length > 0 ? <span>Provider credentials not detected for {missingProviders.map((provider) => provider.provider).join(", ")}.</span> : null}
        {detectedProviders.length === 0 ? <span>Add provider credentials in your shell environment before provider-backed plan/code work.</span> : null}
        {initResult ? <span>{initResult.created.length > 0 ? `${initResult.created.length} files created by /init.` : "/init found existing project files."}</span> : null}
      </div>
    </section>
  );
}

function ReadinessMetric({ label, value, ok }: { label: string; value: string; ok: boolean }) {
  return (
    <div className={ok ? "readiness-metric ok" : "readiness-metric warn"}>
      {ok ? <CheckCircle2 size={14} /> : <AlertTriangle size={14} />}
      <span>{label}</span>
      <strong>{value}</strong>
    </div>
  );
}

function EventCell({
  commandRegistry,
  event,
}: {
  commandRegistry: CommandMetadata[];
  event: SessionEvent;
}) {
  const payload = event.payload as Record<string, unknown>;
  const details = eventDetails(event);

  return (
    <article className={`event-cell ${event.type.replaceAll(".", "-")}`}>
      <div className="event-gutter">
        <EventIcon type={event.type} />
      </div>
      <div className="event-body">
        <div className="event-meta">
          <span>{details.title}</span>
          <time>{compactTime(event.timestamp)}</time>
        </div>
        <div className="event-main">{redactForDisplay(details.body)}</div>
        {event.type === "context.packet.ready" ? (
          <div className="chip-row">
            {((payload.likely_files as string[] | undefined) ?? []).slice(0, 3).map((file) => (
              <span className="path-chip" key={file}>{redactForDisplay(file)}</span>
            ))}
          </div>
        ) : null}
        <EventExtras commandRegistry={commandRegistry} event={event} />
      </div>
    </article>
  );
}

function EventIcon({ type }: { type: SessionEvent["type"] }) {
  if (type.startsWith("context.")) {
    return <PanelRight size={16} />;
  }
  if (type.startsWith("turn.")) {
    return <Play size={16} />;
  }
  if (type.startsWith("check.")) {
    return <ListChecks size={16} />;
  }
  if (type.startsWith("artifact.")) {
    return <Archive size={16} />;
  }
  if (type.startsWith("approval.")) {
    return <ShieldCheck size={16} />;
  }
  return <TerminalSquare size={16} />;
}

function eventDetails(event: SessionEvent): { title: string; body: string } {
  const payload = event.payload as Record<string, unknown>;
  switch (event.type) {
    case "session.created":
      return { title: "session", body: String(payload.title ?? "Session created") };
    case "turn.started":
      return {
        title: String(payload.command ?? "turn"),
        body: String(payload.task || "Command started"),
      };
    case "context.build.started":
      return {
        title: "context build",
        body: `${String(payload.provider ?? "provider")} / ${String(payload.model ?? "default")}`,
      };
    case "context.packet.ready":
      return {
        title: "context ready",
        body: `${String(payload.packet_id ?? "packet")} at ${Math.round(Number(payload.estimated_input_tokens ?? 0) / 1000)}k tokens`,
      };
    case "context.omission.risk":
      return {
        title: String(payload.risk ?? "omission"),
        body: `${String(payload.path ?? "")}: ${String(payload.reason ?? "")}`,
      };
    case "artifact.written":
      return {
        title: "artifact",
        body: `${String(payload.artifact_kind ?? "artifact")} ${String(payload.path ?? "")}`,
      };
    case "check.completed":
      return {
        title: "checks",
        body: `${String(payload.checks_loaded ?? 0)} loaded, ${String(payload.blocking_findings ?? 0)} blocking`,
      };
    case "doctor.completed":
      return {
        title: "doctor",
        body: `${String(payload.status ?? "unknown")} with ${String(payload.failures ?? 0)} failures`,
      };
    case "explore.completed":
      return {
        title: "explore",
        body: `${String(payload.findings_count ?? 0)} findings at ${Math.round(Number(payload.confidence ?? 0) * 100)}% confidence`,
      };
    case "workspace.status.ready":
      if (isHelpPayload(payload.status)) {
        return { title: "help", body: `${payload.status.commands.length} commands available` };
      }
      return { title: "status", body: "Workspace status loaded" };
    case "turn.completed":
      return { title: "done", body: String(payload.summary ?? "Turn completed") };
    case "turn.failed":
      return { title: "failed", body: String(payload.error ?? "Turn failed") };
    default:
      return { title: event.type, body: JSON.stringify(payload) };
  }
}

function EventExtras({
  commandRegistry,
  event,
}: {
  commandRegistry: CommandMetadata[];
  event: SessionEvent;
}) {
  const payload = event.payload as Record<string, unknown>;

  if (event.type === "workspace.status.ready") {
    const status = payload.status;
    const result = payload.result ?? status;
    if (isInitResultPayload(result)) {
      const created = result.created.length > 0 ? result.created : ["existing project files"];
      return (
        <CommandDetailList
          rows={created.slice(0, 5).map((path, index) => ({
            label: index === 0 ? "created" : "file",
            value: path,
          }))}
        />
      );
    }
    if (isRunsResultPayload(result)) {
      return (
        <CommandDetailList
          rows={result.runs.slice(0, 5).map((run) => ({
            label: run.run_id,
            value: `${run.artifact_count} artifacts / ${run.has_context_packet ? "packet" : "no packet"}`,
          }))}
        />
      );
    }
    if (isContextWhyResultPayload(result)) {
      return (
        <CommandDetailList
          rows={[
            { label: "path", value: result.path },
            { label: "status", value: result.status },
            { label: "reason", value: result.reason },
            { label: "packet", value: `${result.run_id} / ${result.packet_hash}` },
          ]}
        />
      );
    }
    if (isHelpPayload(status)) {
      const commands = mergeCommandRegistry(status.registry ?? status.commands, commandRegistry);
      return (
        <div className="command-result-grid">
          {commands.map((command) => {
            const spec = commandSpec(command.name, commands);
            return (
              <div className="command-result-row" key={command.name}>
                <strong>{command.usage || command.name}</strong>
                <span>{spec ? `${spec.support}: ${spec.summary}` : "Project command"}</span>
              </div>
            );
          })}
        </div>
      );
    }
    if (isWorkspaceStatusPayload(status)) {
      return (
        <div className="event-stat-grid">
          <span>{status.git.branch ?? "detached"}</span>
          <span>{status.git.dirty ? "dirty" : "clean"}</span>
          <span>{status.mimir.checks_loaded} checks</span>
          <span>{status.providers.filter((provider) => provider.credential_detected).length} providers</span>
        </div>
      );
    }
  }

  if (event.type === "doctor.completed") {
    const probes = doctorProbeRows(asRecord(payload.result));
    return (
      <>
        <div className="event-stat-grid">
          <span>{String(payload.status ?? "unknown")}</span>
          <span>{String(payload.warnings ?? 0)} warnings</span>
          <span>{String(payload.failures ?? 0)} failures</span>
        </div>
        {probes.length > 0 ? <CommandDetailList rows={probes} /> : null}
      </>
    );
  }

  if (event.type === "check.completed") {
    const findings = checkFindingRows(asRecord(payload.result));
    return (
      <>
        <div className="event-stat-grid">
          <span>{payload.passed ? "passed" : "blocked"}</span>
          <span>{String(payload.findings_count ?? 0)} findings</span>
          <span>{String(payload.blocking_findings ?? 0)} blocking</span>
        </div>
        {findings.length > 0 ? <CommandDetailList rows={findings.slice(0, 3)} /> : null}
      </>
    );
  }

  if (event.type === "explore.completed") {
    const paths = Array.isArray(payload.relevant_paths)
      ? payload.relevant_paths.map((item) => String(item)).slice(0, 3)
      : [];
    const findings = exploreFindingRows(asRecord(payload.result));
    return (
      <>
        <div className="chip-row">
          {paths.map((path) => (
            <span className="path-chip" key={path}>{redactForDisplay(path)}</span>
          ))}
        </div>
        {findings.length > 0 ? <CommandDetailList rows={findings.slice(0, 3)} /> : null}
      </>
    );
  }

  return null;
}

function CommandDetailList({ rows }: { rows: Array<{ label: string; value: string }> }) {
  return (
    <div className="command-detail-list">
      {rows.map((row) => (
        <div className="command-detail-row" key={`${row.label}:${row.value}`}>
          <strong>{row.label}</strong>
          <span>{redactForDisplay(row.value)}</span>
        </div>
      ))}
    </div>
  );
}

function doctorProbeRows(result: Record<string, unknown> | null): Array<{ label: string; value: string }> {
  if (!result) {
    return [];
  }

  return [
    ["config", "config"],
    ["providers", "provider_capabilities"],
    ["tokens", "token_counter"],
    ["context", "context_packet"],
    ["permissions", "permissions"],
    ["credentials", "provider_credentials"],
  ].flatMap(([label, key]) => {
    const probe = asRecord(result[key]);
    if (!probe) {
      return [];
    }
    return [{
      label,
      value: `${String(probe.status ?? "unknown")}: ${String(probe.detail ?? "")}`,
    }];
  });
}

function checkFindingRows(result: Record<string, unknown> | null): Array<{ label: string; value: string }> {
  const findings = Array.isArray(result?.findings) ? result.findings : [];
  return findings.flatMap((finding) => {
    const record = asRecord(finding);
    if (!record) {
      return [];
    }
    const paths = stringArray(record.paths);
    return [{
      label: `${String(record.severity ?? "info")} ${String(record.category ?? "check")}`,
      value: `${paths[0] ?? "*"} - ${String(record.description ?? "Finding")}`,
    }];
  });
}

function exploreFindingRows(result: Record<string, unknown> | null): Array<{ label: string; value: string }> {
  const evidence = asRecord(result?.evidence);
  return stringArray(evidence?.findings).map((finding, index) => ({
    label: `finding ${index + 1}`,
    value: finding,
  }));
}

function Composer({
  activeSessionId,
  commandRegistry,
  files,
  pending,
  runtime,
  sessions,
  onFileSearch,
  onResumeSession,
  onSubmit,
}: {
  activeSessionId: string | null;
  commandRegistry: CommandMetadata[];
  files: WorkspaceFileMatch[];
  pending: boolean;
  runtime: RuntimeConfig;
  sessions: SessionMetadata[];
  onFileSearch: (query: string) => void;
  onResumeSession: (sessionId: string) => Promise<void>;
  onSubmit: (message: string) => Promise<void>;
}) {
  const [draft, setDraft] = useState("/context scaffold the Studio shell");
  const inputRef = useRef<HTMLTextAreaElement | null>(null);
  const mention = useMemo(() => extractMention(draft), [draft]);
  const commandQuery = useMemo(() => extractCommandQuery(draft), [draft]);
  const resumeQuery = useMemo(() => extractResumeQuery(draft), [draft]);
  const commandMatches = useMemo(() => {
    if (commandQuery == null) {
      return [];
    }
    return commandRegistry.filter((command) => command.name.slice(1).startsWith(commandQuery));
  }, [commandQuery, commandRegistry]);
  const commandChips = useMemo(
    () =>
      commandRegistry
        .filter((command) => command.enabled)
        .map((command) => command.name)
        .filter((command) => preferredCommandChips.includes(command)),
    [commandRegistry],
  );
  const visibleCommandMatches = resumeQuery == null ? commandMatches : [];
  const resumeMatches = useMemo(() => {
    if (resumeQuery == null) {
      return [];
    }
    return filterResumeSessions(sessions, resumeQuery, activeSessionId).slice(0, 6);
  }, [activeSessionId, resumeQuery, sessions]);
  const [activeResumeIndex, setActiveResumeIndex] = useState(0);
  const activeResumeSession = resumeMatches[activeResumeIndex] ?? resumeMatches[0] ?? null;
  const resumeEmptyMessage = resumeQuery === "" ? "No other sessions yet" : "No matching sessions";
  const canSubmit = runtime.mode === "mock" || Boolean(activeSessionId);

  useEffect(() => {
    if (mention != null) {
      onFileSearch(mention);
    }
  }, [mention, onFileSearch]);

  useEffect(() => {
    setActiveResumeIndex(0);
  }, [resumeMatches.length, resumeQuery]);

  const submit = async () => {
    const message = draft.trim();
    if (!message || !canSubmit) {
      return;
    }
    await onSubmit(message);
    setDraft("");
  };

  const applyCommand = (commandName: string) => {
    const spec = commandSpec(commandName, commandRegistry);
    const carriedTask = draft.replace(/^\/\w+\s*/, "").trim();
    const takesInput = Boolean(spec?.takes_input);
    const nextDraft = takesInput
      ? `${commandName}${carriedTask ? ` ${carriedTask}` : " "}`
      : commandName;
    setDraft(nextDraft);
    window.requestAnimationFrame(() => inputRef.current?.focus());
  };

  const chooseResumeSession = async (sessionId: string) => {
    await onResumeSession(sessionId);
    setDraft("");
    window.requestAnimationFrame(() => inputRef.current?.focus());
  };

  const handleComposerKeyDown = (event: React.KeyboardEvent<HTMLTextAreaElement>) => {
    if (resumeQuery != null && resumeMatches.length > 0) {
      if (event.key === "ArrowDown") {
        event.preventDefault();
        setActiveResumeIndex((index) => (index + 1) % resumeMatches.length);
        return;
      }
      if (event.key === "ArrowUp") {
        event.preventDefault();
        setActiveResumeIndex(
          (index) => (index - 1 + resumeMatches.length) % resumeMatches.length,
        );
        return;
      }
      if (event.key === "Enter" && !event.shiftKey && !event.metaKey && !event.ctrlKey) {
        event.preventDefault();
        const session = resumeMatches[activeResumeIndex] ?? resumeMatches[0];
        void chooseResumeSession(session.session_id);
        return;
      }
    }

    if ((event.metaKey || event.ctrlKey) && event.key === "Enter") {
      event.preventDefault();
      void submit();
    }
  };

  return (
    <section className="composer-shell" aria-label="Composer">
      <div className="composer-toolbar">
        <div className="chip-row">
          {commandChips.map((command) => (
            <button
              className="command-chip"
              key={command}
              onClick={() => applyCommand(command)}
              type="button"
            >
              {command}
            </button>
          ))}
        </div>
        <span className="runtime-label">{runtime.mode}</span>
      </div>
      <div className="composer-input-row">
        <textarea
          aria-label="Message"
          data-testid="composer-input"
          onChange={(event) => setDraft(event.target.value)}
          onKeyDown={handleComposerKeyDown}
          placeholder="/context ..."
          ref={inputRef}
          value={draft}
        />
        <button
          aria-label={pending ? "Running" : "Send"}
          className="send-button"
          data-testid="composer-send"
          disabled={pending || !draft.trim() || !canSubmit}
          onClick={() => void submit()}
          type="button"
        >
          {pending ? <Loader2 className="spin" size={18} /> : <Send size={18} />}
        </button>
      </div>
      {visibleCommandMatches.length > 0 ? (
        <div className="command-popover" data-testid="command-palette">
          <TerminalSquare size={14} />
          <div className="command-options">
            {visibleCommandMatches.slice(0, 12).map((command) => (
              <button
                key={command.name}
                onClick={() => applyCommand(command.name)}
                type="button"
              >
                <strong>{command.name}</strong>
                <span>{command.support}: {command.summary}</span>
              </button>
            ))}
          </div>
        </div>
      ) : null}
      {resumeQuery != null ? (
        <div className="resume-popover" data-testid="resume-palette">
          <History size={14} />
          <div className="resume-options" role="listbox" aria-label="Resume sessions">
            {resumeMatches.length === 0 ? (
              <div className="resume-empty">{resumeEmptyMessage}</div>
            ) : (
              resumeMatches.map((session) => (
                <button
                  aria-selected={session.session_id === activeResumeSession?.session_id}
                  className={session.session_id === activeResumeSession?.session_id ? "active" : ""}
                  data-testid={`resume-option-${session.session_id}`}
                  key={session.session_id}
                  onClick={() => void chooseResumeSession(session.session_id)}
                  role="option"
                  type="button"
                >
                  <span className="resume-option-main">
                    <strong>{session.title}</strong>
                    <small>{session.workspace_name}</small>
                  </span>
                  <span>
                    {session.session_id === activeSessionId
                      ? "current"
                      : compactTime(session.updated_at)}
                  </span>
                </button>
              ))
            )}
          </div>
        </div>
      ) : null}
      {mention != null ? (
        <div className="mention-popover">
          <Search size={14} />
          {files.length === 0 ? (
            <div className="mention-empty">No matches</div>
          ) : (
            files.slice(0, 6).map((file) => (
              <button
                key={`${file.kind}:${file.path}:${file.symbol ?? ""}`}
                onClick={() => setDraft(replaceMention(draft, file.path))}
                type="button"
              >
                <span>{redactForDisplay(file.symbol ?? file.path)}</span>
                <small>{file.symbol ? redactForDisplay(file.path) : file.kind}</small>
              </button>
            ))
          )}
        </div>
      ) : null}
    </section>
  );
}

function Inspector({
  client,
  events,
  packetCommandError,
  packetCommandPreview,
  onSubmit,
  status,
}: {
  client: StudioApiClient | null;
  events: SessionEvent[];
  packetCommandError: string | null;
  packetCommandPreview: PacketActionPreviewState | null;
  onSubmit: (message: string) => Promise<void>;
  status: WorkspaceStatus | null;
}) {
  const tab = useStudioStore((state) => state.inspectorTab);
  const setTab = useStudioStore((state) => state.setInspectorTab);
  const context = latestContext(events);

  const tabs: Array<{ id: InspectorTab; label: string; icon: React.ReactNode }> = [
    { id: "context", label: "Context", icon: <PanelRight size={15} /> },
    { id: "artifacts", label: "Artifacts", icon: <Archive size={15} /> },
    { id: "approvals", label: "Approvals", icon: <ShieldCheck size={15} /> },
    { id: "provider", label: "Provider", icon: <Bot size={15} /> },
  ];

  return (
    <aside className="inspector">
      <div className="inspector-tabs" role="tablist">
        {tabs.map((item) => (
          <button
            className={tab === item.id ? "inspector-tab active" : "inspector-tab"}
            key={item.id}
            onClick={() => setTab(item.id)}
            type="button"
          >
            {item.icon}
            <span>{item.label}</span>
          </button>
        ))}
      </div>
      {tab === "context" ? (
        <ContextInspector
          client={client}
          context={context}
          events={events}
          packetCommandError={packetCommandError}
          packetCommandPreview={packetCommandPreview}
          onSubmit={onSubmit}
        />
      ) : null}
      {tab === "artifacts" ? <ArtifactsInspector client={client} events={events} status={status} onSubmit={onSubmit} /> : null}
      {tab === "approvals" ? <ApprovalsInspector events={events} /> : null}
      {tab === "provider" ? <ProviderInspector status={status} /> : null}
    </aside>
  );
}

function ContextInspector({
  client,
  context,
  events,
  packetCommandError,
  packetCommandPreview,
  onSubmit,
}: {
  client: StudioApiClient | null;
  context: ReturnType<typeof latestContext>;
  events: SessionEvent[];
  packetCommandError: string | null;
  packetCommandPreview: PacketActionPreviewState | null;
  onSubmit: (message: string) => Promise<void>;
}) {
  const setTab = useStudioStore((state) => state.setInspectorTab);
  const why = latestWhyResult(events);
  const [packetPreview, setPacketPreview] = useState<PacketActionPreviewState | null>(null);
  const [packetActionLoading, setPacketActionLoading] = useState<"replay" | "share" | null>(null);
  const [packetActionError, setPacketActionError] = useState<string | null>(null);

  useEffect(() => {
    if (packetCommandPreview) {
      setPacketPreview(packetCommandPreview);
      setPacketActionError(null);
    }
  }, [packetCommandPreview]);

  useEffect(() => {
    if (packetCommandError) {
      setPacketPreview(null);
      setPacketActionError(packetCommandError);
    }
  }, [packetCommandError]);

  const loadPacketAction = async (kind: "replay" | "share") => {
    if (!client || !context?.runId) {
      return;
    }
    setPacketActionLoading(kind);
    setPacketActionError(null);
    try {
      if (kind === "replay") {
        setPacketPreview({ kind, data: await client.fetchReplay(context.runId) });
      } else {
        setPacketPreview({ kind, data: await client.fetchShare(context.runId) });
      }
    } catch (error) {
      setPacketActionError(error instanceof Error ? error.message : `Unable to load ${kind}`);
    } finally {
      setPacketActionLoading(null);
    }
  };

  return (
    <section className="inspector-body" data-testid="context-inspector">
      <div className="inspector-kpis">
        <div>
          <span>Packet</span>
          <strong>{context?.packetId ?? "none"}</strong>
        </div>
        <div>
          <span>Budget</span>
          <strong>{context ? `${Math.round(context.estimatedInputTokens / 1000)}k` : "0k"}</strong>
        </div>
      </div>
      {context ? (
        <InspectorGroup icon={<Gauge size={15} />} title="Packet Identity">
          <PathRow label="run" path={context.runId || "unknown"} />
          <PathRow label="path" path={context.packetPath || "context_packet.json"} />
          {context.packetHash ? <PathRow label="hash" path={context.packetHash} /> : null}
          <div className="packet-actions">
            <button className="artifact-row" onClick={() => setTab("artifacts")} type="button">
              <Archive size={14} />
              <span>Open packet artifacts</span>
              <small>{context.runId}</small>
            </button>
            {client ? (
              <>
                <button className="artifact-row" disabled={packetActionLoading === "replay"} onClick={() => void loadPacketAction("replay")} type="button">
                  {packetActionLoading === "replay" ? <Loader2 className="spin" size={14} /> : <RotateCcw size={14} />}
                  <span>Preview replay</span>
                  <small>redacted</small>
                </button>
                <button className="artifact-row" disabled={packetActionLoading === "share"} onClick={() => void loadPacketAction("share")} type="button">
                  {packetActionLoading === "share" ? <Loader2 className="spin" size={14} /> : <Share2 size={14} />}
                  <span>Preview share</span>
                  <small>bundle</small>
                </button>
              </>
            ) : (
              <div className="quiet-row">
                <Archive size={14} />
                <span>Replay/share require the local API</span>
              </div>
            )}
          </div>
          {packetActionError ? <div className="artifact-error">{redactForDisplay(packetActionError)}</div> : null}
          {packetPreview ? <PacketActionPreview preview={packetPreview} /> : null}
        </InspectorGroup>
      ) : (
        <InspectorGroup icon={<Gauge size={15} />} title="Packet Identity">
          <div className="quiet-row">
            <Pause size={14} />
            <span>No context packet yet; replay/share become available after /context writes a run packet.</span>
          </div>
          {packetActionError ? <div className="artifact-error">{redactForDisplay(packetActionError)}</div> : null}
          {packetPreview ? <PacketActionPreview preview={packetPreview} /> : null}
        </InspectorGroup>
      )}
      <InspectorGroup icon={<Files size={15} />} title="Likely Files">
        {(context?.likelyFiles ?? []).length > 0 ? (
          (context?.likelyFiles ?? []).map((file) => (
            <button className="artifact-row" key={file} onClick={() => void onSubmit(`/why ${file}`)} type="button">
              <Files size={14} />
              <span>{redactForDisplay(file)}</span>
              <small>why</small>
            </button>
          ))
        ) : (
          <div className="quiet-row">
            <Files size={14} />
            <span>No likely files selected</span>
          </div>
        )}
      </InspectorGroup>
      <InspectorGroup icon={<ListChecks size={15} />} title="Guidance">
        {(context?.guidanceFiles ?? []).length > 0 ? (
            (context?.guidanceFiles ?? []).map((file) => <PathRow key={file} path={file} />)
        ) : (
          <div className="quiet-row">
            <ListChecks size={14} />
            <span>No guidance files included</span>
          </div>
        )}
      </InspectorGroup>
      <InspectorGroup icon={<AlertTriangle size={15} />} title="Risk">
        {(context?.riskyOmissions ?? []).length > 0 ? (
          (context?.riskyOmissions ?? []).map((item) => (
            <button className="risk-row" key={item.path} onClick={() => void onSubmit(`/why ${item.path}`)} type="button">
              <strong>{item.risk ?? "omitted"}</strong>
              <span>{redactForDisplay(item.path)}</span>
              <small>{item.reason || "No omission reason recorded"}</small>
            </button>
          ))
        ) : (
          <div className="quiet-row">
            <CheckCircle2 size={14} />
            <span>No risky omissions recorded</span>
          </div>
        )}
      </InspectorGroup>
      <InspectorGroup icon={<Search size={15} />} title="Latest Why">
        {why ? (
          <CommandDetailList
            rows={[
              { label: "path", value: why.path },
              { label: "status", value: why.status },
              { label: "reason", value: why.reason },
              { label: "packet", value: `${why.run_id} / ${why.packet_hash}` },
            ]}
          />
        ) : (
          <div className="quiet-row">
            <Search size={14} />
            <span>Select a file above or run /why &lt;path&gt;</span>
          </div>
        )}
      </InspectorGroup>
    </section>
  );
}

function PacketActionPreview({
  preview,
}: {
  preview: { kind: "replay"; data: ReplayPreviewResponse } | { kind: "share"; data: SharePreviewResponse };
}) {
  if (!preview.data.redacted) {
    return (
      <section className="artifact-preview packet-preview" data-testid={`packet-${preview.kind}-preview`}>
        <div className="quiet-row">
          <AlertTriangle size={14} />
          <span>Preview unavailable because this payload is not marked redacted</span>
        </div>
      </section>
    );
  }
  const content = preview.kind === "replay" ? preview.data.request : preview.data.bundle;
  const { text, truncated } = formatArtifactContent(content);
  const digest = preview.kind === "replay" ? preview.data.provider_request_sha256 : preview.data.bundle_sha256;
  const sourceLabel = preview.kind === "replay" ? replaySourceLabel(preview.data.source) : "share bundle";

  return (
    <section className="artifact-preview packet-preview" data-testid={`packet-${preview.kind}-preview`}>
      <div className="artifact-preview-head">
        <strong>{preview.kind === "replay" ? "Replay request" : "Share bundle"}</strong>
        <small>{preview.data.redacted ? `redacted / ${sourceLabel}` : sourceLabel}</small>
      </div>
      <PathRow label="run" path={preview.data.run_id} />
      <PathRow label="path" path={preview.data.packet_path} />
      <PathRow label="packet" path={preview.data.packet_hash} />
      <PathRow label="sha256" path={digest} />
      <pre className="artifact-code json">{text}</pre>
      {truncated ? <ArtifactTruncationNotice /> : null}
    </section>
  );
}

function ArtifactsInspector({
  client,
  events,
  onSubmit,
  status,
}: {
  client: StudioApiClient | null;
  events: SessionEvent[];
  onSubmit: (message: string) => Promise<void>;
  status: WorkspaceStatus | null;
}) {
  const context = latestContext(events);
  const runsResult = latestRunsResult(events);
  const artifactItems = useMemo(() => artifactEvents(events), [events]);
  const fallbackArtifacts = useMemo(() => toArtifactSummaries(artifactItems), [artifactItems]);
  const runSummaries = useMemo(
    () => runsResult?.runs ?? status?.mimir.recent_runs ?? [],
    [runsResult?.runs, status?.mimir.recent_runs],
  );
  const runIds = useMemo(() => {
    const ids = [
      context?.runId,
      ...artifactItems.map((item) => item.runId),
      ...runSummaries.map((run) => run.run_id),
    ].filter(
      (id): id is string => Boolean(id),
    );
    return Array.from(new Set(ids));
  }, [artifactItems, context?.runId, runSummaries]);
  const [activeRunId, setActiveRunId] = useState("");
  const [artifacts, setArtifacts] = useState<ArtifactSummary[]>(fallbackArtifacts);
  const [selected, setSelected] = useState<ArtifactSummary | null>(null);
  const [preview, setPreview] = useState<ArtifactContentResponse | null>(null);
  const [listLoading, setListLoading] = useState(false);
  const [previewLoading, setPreviewLoading] = useState(false);
  const [listError, setListError] = useState<string | null>(null);
  const [previewError, setPreviewError] = useState<string | null>(null);
  const activeRunSummary = runSummaries.find((run) => run.run_id === activeRunId);
  const [traceStatus, setTraceStatus] = useState<TraceStatus | null>(
    activeRunSummary?.trace_status ?? null,
  );
  const hasTraceArtifact = traceStatus?.state === "recorded";

  useEffect(() => {
    if (runIds.length === 0) {
      setActiveRunId("");
      return;
    }
    if (!activeRunId || !runIds.includes(activeRunId)) {
      setActiveRunId(runIds[0]);
    }
  }, [activeRunId, runIds]);

  useEffect(() => {
    let cancelled = false;
    setPreview(null);
    setPreviewError(null);
    setListError(null);

    if (!activeRunId) {
      setListLoading(false);
      setArtifacts(fallbackArtifacts);
      setSelected(null);
      setTraceStatus(null);
      return () => {
        cancelled = true;
      };
    }

    setTraceStatus(activeRunSummary?.trace_status ?? null);

    if (!client) {
      setListLoading(false);
      setArtifacts(fallbackArtifacts);
      setSelected(fallbackArtifacts[0] ?? null);
      return () => {
        cancelled = true;
      };
    }

    setListLoading(true);
    client
      .listArtifacts(activeRunId)
      .then((response) => {
        if (cancelled) {
          return;
        }
        setArtifacts(response.artifacts);
        setSelected(response.artifacts[0] ?? null);
        setTraceStatus(response.trace_status);
      })
      .catch((error: unknown) => {
        if (cancelled) {
          return;
        }
        setArtifacts(fallbackArtifacts);
        setSelected(fallbackArtifacts[0] ?? null);
        setTraceStatus(activeRunSummary?.trace_status ?? null);
        setListError(error instanceof Error ? error.message : "Unable to load artifacts");
      })
      .finally(() => {
        if (!cancelled) {
          setListLoading(false);
        }
      });

    return () => {
      cancelled = true;
    };
  }, [activeRunId, activeRunSummary?.trace_status, client, fallbackArtifacts]);

  useEffect(() => {
    let cancelled = false;
    setPreview(null);
    setPreviewError(null);

    if (!selected || !client || !activeRunId) {
      setPreviewLoading(false);
      return () => {
        cancelled = true;
      };
    }

    setPreviewLoading(true);
    client
      .fetchArtifact(activeRunId, selected.name)
      .then((response) => {
        if (!cancelled) {
          setPreview(response);
        }
      })
      .catch((error: unknown) => {
        if (!cancelled) {
          setPreviewError(error instanceof Error ? error.message : "Unable to fetch artifact");
        }
      })
      .finally(() => {
        if (!cancelled) {
          setPreviewLoading(false);
        }
      });

    return () => {
      cancelled = true;
    };
  }, [activeRunId, client, selected]);

  const openArtifact = useCallback(
    (artifact: ArtifactSummary) => {
      setSelected({ ...artifact });
    },
    [],
  );

  return (
    <section className="inspector-body" data-testid="artifacts-inspector">
      <InspectorGroup icon={<History size={15} />} title="Runs">
        {runIds.length === 0 ? (
          <>
            <div className="quiet-row">
              <Pause size={14} />
              <span>No runs yet</span>
            </div>
            <button className="artifact-row" onClick={() => void onSubmit("/context inspect this workspace")} type="button">
              <PanelRight size={14} />
              <span>Build a context packet</span>
              <small>/context</small>
            </button>
          </>
        ) : (
          <div className="run-picker">
            {runIds.map((runId) => (
              <button
                className={runId === activeRunId ? "run-chip active" : "run-chip"}
                key={runId}
                onClick={() => setActiveRunId(runId)}
                type="button"
              >
                {runId}
              </button>
            ))}
          </div>
        )}
        {runSummaries.length > 0 ? (
          <div className="run-summary-list">
            {runSummaries.slice(0, 4).map((run) => (
              <div className="command-detail-row" key={run.run_id}>
                <strong>{run.run_id}</strong>
                <span>{run.artifact_count} artifacts / {run.has_context_packet ? "packet" : "no packet"}</span>
              </div>
            ))}
          </div>
        ) : null}
      </InspectorGroup>
      <InspectorGroup icon={<Archive size={15} />} title="Redacted Artifacts">
        <div data-testid="artifact-list">
          {listLoading ? (
            <div className="quiet-row" data-testid="artifact-list-loading">
              <Loader2 className="spin" size={14} />
              <span>Loading artifacts</span>
            </div>
          ) : artifacts.length === 0 ? (
            <>
              <div className="quiet-row">
                <Archive size={14} />
                <span>{listError ? "Artifact list unavailable" : "No artifacts for this run"}</span>
              </div>
              <button className="artifact-row" onClick={() => void onSubmit("/runs")} type="button">
                <History size={14} />
                <span>Refresh runs</span>
                <small>/runs</small>
              </button>
            </>
          ) : (
            artifacts.map((artifact) => (
              <button
                className={
                  selected?.name === artifact.name ? "artifact-row active" : "artifact-row"
                }
                key={artifact.path}
                onClick={() => openArtifact(artifact)}
                type="button"
              >
                <Code2 size={14} />
                <span>{artifact.name}</span>
                <small>{artifact.redacted ? "redacted" : formatBytes(artifact.size_bytes)}</small>
              </button>
            ))
          )}
        </div>
      </InspectorGroup>
      {listError ? <div className="artifact-error">{redactForDisplay(listError)}</div> : null}
      {previewError ? <div className="artifact-error">{redactForDisplay(previewError)}</div> : null}
      {activeRunId ? (
        <InspectorGroup icon={<TerminalSquare size={15} />} title="Trace">
          {hasTraceArtifact ? (
            <div className="quiet-row" data-testid="trace-state">
              <CheckCircle2 size={14} />
              <span>Trace artifact recorded for this run</span>
            </div>
          ) : traceStatus?.state === "unavailable" ? (
            <div className="quiet-row" data-testid="trace-state">
              <AlertTriangle size={14} />
              <span>Trace metadata unavailable for this run</span>
            </div>
          ) : (
            <div className="quiet-row" data-testid="trace-state">
              <Pause size={14} />
              <span>No trace recorded for this run</span>
            </div>
          )}
        </InspectorGroup>
      ) : null}
      {previewLoading ? (
        <section className="artifact-preview empty" data-testid="artifact-preview">
          <div className="quiet-row" data-testid="artifact-preview-loading">
            <Loader2 className="spin" size={14} />
            <span>Loading preview</span>
          </div>
        </section>
      ) : preview ? (
        <section className="artifact-preview" data-testid="artifact-preview">
          <div className="artifact-preview-head">
            <strong>{preview.name}</strong>
            <small>{artifactPreviewMode(preview)}</small>
          </div>
          <ArtifactPreviewContent artifact={preview} />
        </section>
      ) : selected && !client ? (
        <section className="artifact-preview empty" data-testid="artifact-preview">
          <div className="quiet-row">
            <Archive size={14} />
            <span>Preview available in API mode</span>
          </div>
        </section>
      ) : selected ? (
        <section className="artifact-preview empty" data-testid="artifact-preview">
          <div className="quiet-row">
            <RotateCcw size={14} />
            <span>Select this artifact again to retry preview</span>
          </div>
        </section>
      ) : null}
    </section>
  );
}

function ApprovalsInspector({ events }: { events: SessionEvent[] }) {
  const approvals = events.filter((event) => event.type.startsWith("approval."));
  return (
    <section className="inspector-body">
      <InspectorGroup icon={<ShieldCheck size={15} />} title="Approval Queue">
        {approvals.length === 0 ? (
          <div className="quiet-row">
            <Pause size={14} />
            <span>No pending requests</span>
          </div>
        ) : (
          approvals.map((event) => <PathRow key={event.event_id} path={event.type} />)
        )}
      </InspectorGroup>
    </section>
  );
}

function ProviderInspector({ status }: { status: WorkspaceStatus | null }) {
  return (
    <section className="inspector-body">
      <InspectorGroup icon={<Bot size={15} />} title="Providers">
        {(status?.providers ?? []).map((provider) => (
          <div className="provider-row" key={provider.provider}>
            <span>{provider.provider}</span>
            <small>{provider.models_count} models</small>
            {provider.credential_detected ? <CheckCircle2 size={14} /> : <Square size={14} />}
          </div>
        ))}
      </InspectorGroup>
    </section>
  );
}

function InspectorGroup({
  children,
  icon,
  title,
}: {
  children: React.ReactNode;
  icon: React.ReactNode;
  title: string;
}) {
  return (
    <section className="inspector-group">
      <div className="inspector-heading">
        {icon}
        <span>{title}</span>
      </div>
      <div className="inspector-list">{children}</div>
    </section>
  );
}

function PathRow({ path, label }: { path: string; label?: string }) {
  const displayPath = redactForDisplay(path);
  return (
    <div className="path-row">
      {label ? <small>{label}</small> : <ChevronRight size={13} />}
      <span>{displayPath}</span>
    </div>
  );
}

function SettingsView({
  runtime,
  settings,
  status,
  onSettingsChange,
}: {
  runtime: RuntimeConfig;
  settings: StudioSettings;
  status: WorkspaceStatus | null;
  onSettingsChange: (settings: Partial<StudioSettings>) => void;
}) {
  const providerOptions = useMemo(() => {
    const names = (status?.providers ?? []).map((provider) => provider.provider);
    return Array.from(new Set([settings.provider, ...names].filter(Boolean)));
  }, [settings.provider, status?.providers]);
  const selectedProvider = (status?.providers ?? []).find(
    (provider) => provider.provider === settings.provider,
  );

  return (
    <section className="settings-view" data-testid="settings-view">
      <div className="settings-header">
        <Settings size={20} />
        <div>
          <h1>Settings</h1>
          <p>{runtime.mode === "mock" ? "Mock stream" : "Local API"}</p>
        </div>
      </div>
      <div className="settings-grid">
        <SettingsGroup icon={<Bot size={16} />} title="Provider">
          <label className="setting-control">
            <span>Provider</span>
            <select
              aria-label="Provider"
              onChange={(event) => onSettingsChange({ provider: event.target.value })}
              value={settings.provider}
            >
              {providerOptions.map((provider) => (
                <option key={provider} value={provider}>
                  {provider}
                </option>
              ))}
            </select>
          </label>
          <label className="setting-control">
            <span>Model</span>
            <input
              aria-label="Model"
              maxLength={80}
              onChange={(event) => onSettingsChange({ model: event.target.value })}
              spellCheck={false}
              value={settings.model}
            />
          </label>
          <SettingLine
            label="Credential"
            value={selectedProvider?.credential_detected ? "detected" : "not detected"}
          />
        </SettingsGroup>

        <SettingsGroup icon={<ShieldCheck size={16} />} title="Approvals">
          <SegmentedControl
            label="Policy"
            onChange={(approvalPolicy) => onSettingsChange({ approvalPolicy })}
            options={[
              { label: "Ask", value: "ask" },
              { label: "Read-only", value: "read-only" },
              { label: "Session", value: "session" },
            ]}
            value={settings.approvalPolicy}
          />
        </SettingsGroup>

        <SettingsGroup icon={<Gauge size={16} />} title="Caps">
          <label className="setting-control cap-control">
            <span>Token cap</span>
            <div className="cap-row">
              <input
                aria-label="Token cap slider"
                max={128000}
                min={16000}
                onChange={(event) =>
                  onSettingsChange({ tokenCap: boundedNumber(event.target.value, 16000, 128000) })
                }
                step={4000}
                type="range"
                value={settings.tokenCap}
              />
              <input
                aria-label="Token cap"
                max={128000}
                min={16000}
                onChange={(event) =>
                  onSettingsChange({ tokenCap: boundedNumber(event.target.value, 16000, 128000) })
                }
                step={1000}
                type="number"
                value={settings.tokenCap}
              />
            </div>
          </label>
          <label className="setting-control">
            <span>Cost cap</span>
            <input
              aria-label="Cost cap"
              min={0}
              onChange={(event) =>
                onSettingsChange({ costCapUsd: boundedNumber(event.target.value, 0, 100) })
              }
              step={0.5}
              type="number"
              value={settings.costCapUsd}
            />
          </label>
        </SettingsGroup>

        <SettingsGroup icon={<Palette size={16} />} title="Theme">
          <SegmentedControl
            label="Theme"
            onChange={(theme) => onSettingsChange({ theme })}
            options={[
              { label: "System", value: "system" },
              { label: "Dark", value: "dark" },
              { label: "Light", value: "light" },
            ]}
            value={settings.theme}
          />
        </SettingsGroup>

        <SettingsGroup icon={<HardDrive size={16} />} title="Runtime">
          <SettingLine label="API base" value={runtime.apiBaseUrl || "same origin"} />
          <SettingLine label="UI token" value={runtime.token ? "in memory" : "not present"} />
          <SettingLine label="Workspace" value={status?.workspace_name ?? "unknown"} />
          <SettingLine label="Branch" value={status?.git.branch ?? "unknown"} />
        </SettingsGroup>
      </div>
    </section>
  );
}

function SettingsGroup({
  children,
  icon,
  title,
}: {
  children: React.ReactNode;
  icon: React.ReactNode;
  title: string;
}) {
  return (
    <section className="settings-panel">
      <div className="settings-panel-heading">
        {icon}
        <span>{title}</span>
      </div>
      <div className="settings-panel-body">{children}</div>
    </section>
  );
}

function SegmentedControl<TValue extends string>({
  label,
  options,
  value,
  onChange,
}: {
  label: string;
  options: Array<{ label: string; value: TValue }>;
  value: TValue;
  onChange: (value: TValue) => void;
}) {
  return (
    <div className="segmented-setting">
      <span>{label}</span>
      <div className="segmented-control" role="group" aria-label={label}>
        {options.map((option) => (
          <button
            className={option.value === value ? "active" : ""}
            key={option.value}
            onClick={() => onChange(option.value)}
            type="button"
          >
            {option.label}
          </button>
        ))}
      </div>
    </div>
  );
}

function SettingLine({ label, value }: { label: string; value: string }) {
  return (
    <div className="setting-line">
      <span>{label}</span>
      <strong>{value}</strong>
    </div>
  );
}

function commandName(value: string): string {
  return value.trim().split(/\s+/, 1)[0].toLowerCase();
}

function resumeQueryFromCommand(value: string): string {
  return value.replace(/^\/resume\b/i, "").trim();
}

function shareTargetFromCommand(value: string): string {
  return value.replace(/^\/share\b/i, "").trim();
}

function clientRequired(client: StudioApiClient | null): StudioApiClient {
  if (!client) {
    throw new Error("Packet share preview requires the local API");
  }
  return client;
}

function extractCommandQuery(value: string): string | null {
  const match = value.match(/^\/([a-z]*)$/i);
  return match?.[1].toLowerCase() ?? null;
}

function extractResumeQuery(value: string): string | null {
  const match = value.match(/^\/resume(?:\s+(.*))?$/i);
  return match ? (match[1] ?? "") : null;
}

function extractMention(value: string): string | null {
  const match = value.match(/@([A-Za-z0-9_./-]*)$/);
  return match?.[1] ?? null;
}

function replaceMention(value: string, path: string): string {
  return value.replace(/@[A-Za-z0-9_./-]*$/, `@${path} `);
}

function compactTime(value: string): string {
  const date = new Date(value);
  if (Number.isNaN(date.valueOf())) {
    return "";
  }
  return date.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
}

function formatBytes(value: number): string {
  if (!Number.isFinite(value) || value <= 0) {
    return "0 B";
  }
  if (value < 1024) {
    return `${value} B`;
  }
  return `${Math.round(value / 1024)} KB`;
}

function formatTokenCount(value: number): string {
  if (!Number.isFinite(value) || value <= 0) {
    return "0k";
  }
  return `${Math.round(value / 1000)}k`;
}

function asRecord(value: unknown): Record<string, unknown> | null {
  return value && typeof value === "object" && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : null;
}

function stringArray(value: unknown): string[] {
  return Array.isArray(value) ? value.map((item) => String(item)) : [];
}

function boundedNumber(value: string, min: number, max: number): number {
  const next = Number(value);
  if (!Number.isFinite(next)) {
    return min;
  }
  return Math.min(max, Math.max(min, next));
}

function commandSubmitOptions(settings: StudioSettings): SessionMessageOptions {
  const provider = settings.provider.trim();
  const model = settings.model.trim();
  return {
    ...(provider ? { provider } : {}),
    ...(model ? { model } : {}),
  };
}

function attachCommandResult(
  events: SessionEvent[],
  command: string,
  result: unknown,
): SessionEvent[] {
  const resultRecord = asRecord(result);
  if (!resultRecord) {
    return events;
  }

  const targetTypes = commandResultEventTypes(command);
  let attached = false;
  return events.map((event) => {
    if (attached || !targetTypes.includes(event.type)) {
      return event;
    }
    attached = true;
    return {
      ...event,
      payload: {
        ...(asRecord(event.payload) ?? {}),
        result: resultRecord,
      },
    };
  });
}

function commandResultEventTypes(command: string): SessionEvent["type"][] {
  switch (command) {
    case "doctor":
      return ["doctor.completed"];
    case "check":
      return ["check.completed"];
    case "explore":
      return ["explore.completed"];
    case "context":
      return ["context.packet.ready"];
    case "status":
    case "help":
    case "init":
    case "why":
    case "runs":
      return ["workspace.status.ready"];
    default:
      return [];
  }
}

function filterResumeSessions(
  sessions: SessionMetadata[],
  query: string,
  activeSessionId: string | null,
): SessionMetadata[] {
  const normalized = normalizeSessionQuery(query);
  const matches = normalized
    ? sessions.filter((session) => sessionMatchesQuery(session, normalized))
    : sessions.filter((session) => session.session_id !== activeSessionId);

  return [...matches].sort((left, right) => {
    const leftActive = left.session_id === activeSessionId ? 1 : 0;
    const rightActive = right.session_id === activeSessionId ? 1 : 0;
    if (leftActive !== rightActive) {
      return leftActive - rightActive;
    }
    return Date.parse(right.updated_at) - Date.parse(left.updated_at);
  });
}

function findResumeSession(
  sessions: SessionMetadata[],
  query: string,
  activeSessionId: string | null,
): SessionMetadata | null {
  const matches = filterResumeSessions(sessions, query, activeSessionId);
  return matches[0] ?? null;
}

function sessionMatchesQuery(session: SessionMetadata, query: string): boolean {
  return (
    normalizeSessionQuery(session.title).includes(query) ||
    normalizeSessionQuery(session.session_id).includes(query)
  );
}

function normalizeSessionQuery(value: string): string {
  return value.trim().toLowerCase();
}

function formatArtifactContent(value: unknown): { text: string; truncated: boolean } {
  const text = redactForDisplay(
    typeof value === "string" ? value : (JSON.stringify(value, null, 2) ?? ""),
  );
  return text.length > maxArtifactPreviewChars
    ? { text: `${text.slice(0, maxArtifactPreviewChars)}\n...`, truncated: true }
    : { text, truncated: false };
}

function ArtifactPreviewContent({ artifact }: { artifact: ArtifactContentResponse }) {
  if (!artifact.redacted) {
    return (
      <div className="quiet-row">
        <AlertTriangle size={14} />
        <span>Preview unavailable because this artifact is not marked redacted</span>
      </div>
    );
  }
  const { text, truncated } = formatArtifactContent(artifact.content);
  const mode = artifactPreviewMode(artifact);

  if (mode === "markdown") {
    return (
      <>
        <MarkdownPreview text={text} />
        {truncated ? <ArtifactTruncationNotice /> : null}
      </>
    );
  }

  if (mode === "diff") {
    return (
      <>
        <DiffPreview text={text} />
        {truncated ? <ArtifactTruncationNotice /> : null}
      </>
    );
  }

  return (
    <>
      <pre className={`artifact-code ${mode}`}>{text}</pre>
      {truncated ? <ArtifactTruncationNotice /> : null}
    </>
  );
}

function ArtifactTruncationNotice() {
  return (
    <div className="artifact-truncation" data-testid="artifact-truncation">
      Preview truncated at {formatBytes(maxArtifactPreviewChars)}.
    </div>
  );
}

function MarkdownPreview({ text }: { text: string }) {
  return (
    <div className="artifact-markdown">
      {text.split("\n").slice(0, 120).map((line, index) => {
        if (line.startsWith("### ")) {
          return <h4 key={index}>{line.slice(4)}</h4>;
        }
        if (line.startsWith("## ")) {
          return <h3 key={index}>{line.slice(3)}</h3>;
        }
        if (line.startsWith("# ")) {
          return <h2 key={index}>{line.slice(2)}</h2>;
        }
        if (line.startsWith("- ")) {
          return <p className="artifact-list-line" key={index}>{line}</p>;
        }
        if (!line.trim()) {
          return <div className="artifact-blank-line" key={index} />;
        }
        return <p key={index}>{line}</p>;
      })}
    </div>
  );
}

function DiffPreview({ text }: { text: string }) {
  return (
    <pre className="artifact-diff">
      {text.split("\n").slice(0, 240).map((line, index) => (
        <span className={diffLineClass(line)} key={index}>{line || " "}</span>
      ))}
    </pre>
  );
}

function diffLineClass(line: string): string {
  if (line.startsWith("+++ ") || line.startsWith("--- ")) {
    return "diff-file";
  }
  if (line.startsWith("@@")) {
    return "diff-hunk";
  }
  if (line.startsWith("+")) {
    return "diff-add";
  }
  if (line.startsWith("-")) {
    return "diff-del";
  }
  return "diff-context";
}

function artifactPreviewMode(artifact: ArtifactContentResponse): "json" | "markdown" | "diff" | "trace" | "text" {
  const name = artifact.name.toLowerCase();
  const contentType = artifact.content_type.toLowerCase();
  if (name.endsWith(".json") || contentType.includes("json")) {
    return name.includes("trace") ? "trace" : "json";
  }
  if (name.endsWith(".md") || name.endsWith(".markdown") || contentType.includes("markdown")) {
    return "markdown";
  }
  if (name.endsWith(".diff") || name.endsWith(".patch") || contentType.includes("diff")) {
    return "diff";
  }
  if (name.includes("trace")) {
    return "trace";
  }
  return "text";
}

function replaySourceLabel(source: string): string {
  return source === "saved_artifact" ? "saved artifact" : "reconstructed";
}

function commandSpec(
  commandName: string,
  registry: CommandMetadata[] = fallbackCommandRegistry,
): CommandMetadata | undefined {
  const baseName = commandName.trim().split(/\s+/, 1)[0];
  return registry.find((command) => command.name === baseName);
}

function commandUnavailableMessage(command: CommandMetadata): string {
  if (command.support === "planned") {
    return `${command.name} is planned but not connected to the backend yet${
      command.disabled_reason ? `: ${command.disabled_reason}` : ""
    }`;
  }
  if (command.support === "local") {
    return `${command.name} is handled locally by Mimir Studio`;
  }
  return `${command.name} is not available yet${
    command.disabled_reason ? `: ${command.disabled_reason}` : ""
  }`;
}

function mergeCommandRegistry(
  backend: unknown,
  base: CommandMetadata[] = fallbackCommandRegistry,
): CommandMetadata[] {
  const byName = new Map(base.map((command) => [command.name, command]));
  if (Array.isArray(backend)) {
    for (const item of backend) {
      const command = normalizeCommandMetadata(item);
      if (command) {
        byName.set(command.name, { ...byName.get(command.name), ...command });
      }
    }
  }
  return Array.from(byName.values());
}

function normalizeCommandMetadata(value: unknown): CommandMetadata | null {
  if (typeof value === "string") {
    const fallback = commandSpec(value);
    return fallback ?? {
      name: value.split(/\s+/, 1)[0],
      usage: value,
      summary: "Project command",
      support: "backend",
      takes_input: value.includes("<") || value.includes("["),
      enabled: true,
      disabled_reason: null,
    };
  }
  if (!value || typeof value !== "object") {
    return null;
  }
  const item = value as Partial<CommandMetadata> & { takesInput?: unknown };
  if (!item.name || typeof item.name !== "string" || !item.name.startsWith("/")) {
    return null;
  }
  const support =
    item.support === "backend" || item.support === "local" || item.support === "planned"
      ? item.support
      : "planned";
  return {
    name: item.name,
    usage: typeof item.usage === "string" ? item.usage : item.name,
    summary: typeof item.summary === "string" ? item.summary : "Project command",
    support,
    takes_input:
      typeof item.takes_input === "boolean"
        ? item.takes_input
        : Boolean(item.takesInput),
    enabled: typeof item.enabled === "boolean" ? item.enabled : support !== "planned",
    disabled_reason:
      typeof item.disabled_reason === "string" ? item.disabled_reason : null,
  };
}

function ensureSessionListed(
  sessions: SessionMetadata[],
  metadata: SessionMetadata,
): SessionMetadata[] {
  const existingIndex = sessions.findIndex(
    (session) => session.session_id === metadata.session_id,
  );
  if (existingIndex === -1) {
    return [metadata, ...sessions];
  }
  return sessions.map((session) =>
    session.session_id === metadata.session_id ? metadata : session,
  );
}

function isHelpPayload(
  value: unknown,
): value is { commands: Array<CommandMetadata | string>; registry?: CommandMetadata[] } {
  if (!value || typeof value !== "object" || !("commands" in value)) {
    return false;
  }
  const commands = (value as { commands?: unknown }).commands;
  return (
    Array.isArray(commands) &&
    commands.every(
      (command) => typeof command === "string" || normalizeCommandMetadata(command) != null,
    )
  );
}

function isWorkspaceStatusPayload(value: unknown): value is WorkspaceStatus {
  if (!value || typeof value !== "object") {
    return false;
  }
  const candidate = value as Partial<WorkspaceStatus>;
  return Boolean(candidate.git && candidate.mimir && Array.isArray(candidate.providers));
}

function isInitResultPayload(value: unknown): value is InitResult {
  if (!value || typeof value !== "object") {
    return false;
  }
  const candidate = value as Partial<InitResult>;
  return Array.isArray(candidate.created) && isWorkspaceStatusPayload(candidate.status);
}

function isRunsResultPayload(value: unknown): value is { runs: RunSummary[] } {
  if (!value || typeof value !== "object") {
    return false;
  }
  const candidate = value as { runs?: unknown };
  return Array.isArray(candidate.runs) && candidate.runs.every(isRunSummaryPayload);
}

function isContextWhyResultPayload(value: unknown): value is ContextWhyResult {
  if (!value || typeof value !== "object") {
    return false;
  }
  const candidate = value as Partial<ContextWhyResult>;
  return Boolean(candidate.path && candidate.status && candidate.reason && candidate.run_id && candidate.packet_hash);
}

function isRunSummaryPayload(value: unknown): value is RunSummary {
  if (!value || typeof value !== "object") {
    return false;
  }
  const candidate = value as Partial<RunSummary>;
  return Boolean(candidate.run_id && candidate.path && typeof candidate.artifact_count === "number");
}

function redactForDisplay(value: string): string {
  return value
    .replace(/\/[^"'\s<>]*\/(\.mimir\/runs\/[^"'\s<>]+)/g, "$1")
    .replace(/\/(?:private\/)?tmp\/[^"'\s<>]+/g, "[redacted:path]")
    .replace(/AKIA[0-9A-Z]{16}/g, "[redacted:aws]")
    .replace(/AIza[0-9A-Za-z\-_]{35}/g, "[redacted:gcp]")
    .replace(/sk-ant-[A-Za-z0-9-]+/g, "[redacted:anthropic]")
    .replace(/sk-[A-Za-z0-9]{24,}/g, "[redacted:key]")
    .replace(/(?:sk|pk)_(?:live|test)_[A-Za-z0-9]{24}/g, "[redacted:stripe]")
    .replace(/ghp_[A-Za-z0-9]{36}/g, "[redacted:github]")
    .replace(/github_pat_[A-Za-z0-9_]+/g, "[redacted:github]")
    .replace(/xox[baprs]-[0-9A-Za-z]+/g, "[redacted:slack]")
    .replace(/Bearer\s+[A-Za-z0-9_.-]{8,}/gi, "Bearer [redacted]")
    .replace(/\bui-[A-Za-z0-9-]{16,}/g, "ui-[redacted]")
    .replace(/eyJ[A-Za-z0-9_-]*\.eyJ[A-Za-z0-9_-]*(?:\.[A-Za-z0-9_-]*)?/g, "[redacted:jwt]")
    .replace(/(postgres|mysql|mongodb):\/\/[^:\s]+:[^@\s]+@/gi, "$1://[redacted]@")
    .replace(/-----BEGIN (?:RSA |EC |DSA |OPENSSH )?PRIVATE KEY-----[\s\S]*?-----END (?:RSA |EC |DSA |OPENSSH )?PRIVATE KEY-----/g, "[redacted:private-key]")
    .replace(/\b[A-Z_]*(?:KEY|SECRET|TOKEN)=[^\s]+/g, "[redacted:env]")
    .replace(/(api[_-]?key|accessToken|refreshToken|sessionToken|token|secret|password|setCookie)(["'\s:=]+)[A-Za-z0-9_.:/@%+-]{8,}/gi, "$1$2[redacted]");
}
