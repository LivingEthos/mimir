import { createHash } from "node:crypto";
import { createServer, type IncomingMessage, type ServerResponse } from "node:http";
import type { AddressInfo } from "node:net";
import { expect, test, type Locator, type Page } from "@playwright/test";
import type { SessionEvent, SessionMetadata } from "../src/api/types";

const token = "test-token";
const wsStableProtocol = "mimir.studio.v1";
const wsTokenProtocolPrefix = "mimir-token.";
const sessionId = "sess-live-token";
const secondSessionId = "sess-live-second";
const runId = "run-live-token";
const timestamp = "2026-05-27T12:00:00.000Z";
const tempWorkspacePrefix = "/tmp/mimir-live-token";
const syntheticArtifactListToken = "synthetic-artifact-token-123456";
const syntheticCreateSecret = "syntheticCreateSecret";
const syntheticLargeToken = "syntheticLargeToken123456789";
const syntheticLargeRawPrompt = "RAW_PROMPT_SHOULD_NOT_LEAK_STUDIO_LARGE";
const syntheticLargeProviderRequestBody = "PROVIDER_REQUEST_BODY_SHOULD_NOT_LEAK_STUDIO_LARGE";
const syntheticLargeProviderResponseBody = "PROVIDER_RESPONSE_BODY_SHOULD_NOT_LEAK_STUDIO_LARGE";
const syntheticLargeArtifactPath = `${tempWorkspacePrefix}/.mimir/runs/${runId}/big_log.txt`;
const syntheticMalformedShapeSecret = "syntheticMalformedShapeSecret123456";
const syntheticPreviewSecret = "syntheticPreviewSecret123";
const syntheticRunPathToken = "syntheticRunPathToken123456";
const syntheticStatusApiKey = "sk-123456789012345678901234";
const redactionPatternSecrets = [
  "AKIAIOSFODNN7EXAMPLE",
  "ghp_abcdefghijklmnopqrstuvwxyzABCDEFGHIJ",
  "xoxb-1234567890abcdef",
  "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjMifQ.signature",
  "postgres://mimir:secretpass@127.0.0.1/mimir",
  "-----BEGIN PRIVATE KEY-----\nsyntheticprivatekeymaterial\n-----END PRIVATE KEY-----",
  syntheticStatusApiKey,
];
const syntheticSecrets = [
  ...redactionPatternSecrets,
  syntheticArtifactListToken,
  syntheticCreateSecret,
  syntheticLargeArtifactPath,
  syntheticLargeProviderRequestBody,
  syntheticLargeProviderResponseBody,
  syntheticLargeRawPrompt,
  syntheticLargeToken,
  syntheticMalformedShapeSecret,
  syntheticPreviewSecret,
  syntheticRunPathToken,
];
const syntheticSecretText = redactionPatternSecrets.join(" ");

test("connects through live-token mode and fetches run artifacts", async ({ page }) => {
  test.setTimeout(60_000);

  const harness = await startUiHarness();
  const pageRequests: string[] = [];
  const pageWebSocketUrls: string[] = [];
  page.on("request", (request) => pageRequests.push(request.url()));
  page.on("websocket", (socket) => pageWebSocketUrls.push(socket.url()));

  try {
    await page.goto(`/?mock=0&token=${token}&api=${encodeURIComponent(harness.baseUrl)}`);

    await expect.poll(() => page.url().includes("token=")).toBe(false);
    await expect(page.getByTestId("studio-shell")).toBeVisible();
    await expect(page.getByRole("banner")).toContainText("Mimir Studio");
    await expect(page.locator(".runtime-label")).toContainText("api");
    await expect.poll(() => pageRequests.join("|")).toContain("/v1/workspace/files");
    await expect.poll(() => harness.requests.join("|")).toContain("/v1/workspace/files");
    await expect.poll(() => pageWebSocketUrls.some(isStudioEventStreamUrl)).toBe(true);
    await expect.poll(() => harness.webSocketRequests.length).toBeGreaterThan(0);
    expectNoWebSocketUrlTokenLeak(pageWebSocketUrls);
    expectNoWebSocketUrlTokenLeak(harness.webSocketRequests);
    expect(harness.webSocketProtocols.join("|")).toContain(wsStableProtocol);
    expect(harness.webSocketProtocols.join("|")).toContain(wsTokenProtocolPrefix);
    await expect(page.getByText("apps/studio/src/App.tsx").first()).toBeVisible();

    const composer = page.getByTestId("composer-input");
    await composer.fill("/context harden live mode with @App");
    await expect.poll(() => harness.requests.join("|")).toContain("q=App");
    await expect(page.locator(".mention-popover")).toContainText("apps/studio/src/App.tsx");
    await page.locator(".mention-popover button").first().click();
    await expect(composer).toHaveValue(/@apps\/studio\/src\/App\.tsx /);

    await page.getByTestId("composer-send").click();
    await expect(page.getByTestId("transcript")).toContainText("ctx-live-token");
    await expect(page.getByTestId("context-inspector")).toContainText(`.mimir/runs/${runId}/context_packet.json`);
    await expect(page.getByTestId("context-inspector")).toContainText("live-token omission reason");
    await page.getByRole("button", { name: /Preview replay/ }).click();
    await expect(page.getByTestId("packet-replay-preview")).toContainText("redacted replay request");
    await expectNoPrivacyLeak(page.getByTestId("packet-replay-preview"));
    await page.getByRole("button", { name: /Preview share/ }).click();
    await expect(page.getByTestId("packet-share-preview")).toContainText("mimir.packet_share");
    await expectNoPrivacyLeak(page.getByTestId("packet-share-preview"));
    const postedMessagesBeforeSlashShare = harness.messageBodies.length;
    await composer.fill(`/share ${runId}`);
    await page.getByTestId("composer-send").click();
    await expect(page.getByTestId("packet-share-preview")).toContainText("mimir.packet_share");
    await expectNoPrivacyLeak(page.getByTestId("packet-share-preview"));
    await expect.poll(() => harness.messageBodies.length).toBe(postedMessagesBeforeSlashShare);

    await page.getByRole("button", { name: /Artifacts/ }).click();
    await expect(page.getByTestId("artifact-list")).toContainText("context_packet.json");
    await expect(page.getByTestId("trace-state")).toContainText("Trace artifact recorded for this run");
    await expect(page.locator(".run-chip.active")).toContainText(runId);
    await expectNoPrivacyLeak(page.getByTestId("artifacts-inspector"));
    await expect(page.getByTestId("artifact-preview")).toContainText("schema_version");
    await expectNoPrivacyLeak(page.getByTestId("artifact-preview"));
    await page.getByRole("button", { name: /patch\.diff/ }).click();
    await expect(page.locator(".artifact-diff")).toContainText("+new line");
    await expectNoPrivacyLeak(page.getByTestId("artifact-preview"));
    await page.getByRole("button", { name: /notes\.md/ }).click();
    await expect(page.locator(".artifact-markdown")).toContainText("Review notes");
    await expectNoPrivacyLeak(page.getByTestId("artifact-preview"));
    await page.getByRole("button", { name: /trace\.spans\.jsonl/ }).click();
    await expect(page.locator(".artifact-code.trace")).toContainText("trace_id");
    await expectNoPrivacyLeak(page.getByTestId("artifact-preview"));

    await composer.fill("/runs");
    await page.getByTestId("composer-send").click();
    await expect(page.getByTestId("artifacts-inspector")).toBeVisible();
    await expect(page.locator(".run-chip.active")).toContainText(runId);
    await expectNoPrivacyLeak(page.getByTestId("artifacts-inspector"));

    await composer.fill("/resume ");
    await expect(page.getByTestId("resume-palette")).toContainText("No other sessions yet");
    await page.getByTestId("composer-send").click();
    await expect(page.locator(".error-strip")).toContainText("No resumable sessions");
    await expectNoPrivacyLeak(page.locator(".error-strip"));

    expect(JSON.stringify(harness.messageBodies)).not.toContain(token);
    await expectNoBrowserTokenLeak(page);
    await expectNoWorkspacePathLeak(page);
    await expectNoSyntheticSecrets(page.locator("body"));
    expectNoWebSocketUrlTokenLeak(pageWebSocketUrls);
    expectNoWebSocketUrlTokenLeak(harness.webSocketRequests);
  } finally {
    await harness.close();
  }
});

test("keeps API settings local and does not store the live token", async ({ page }) => {
  const harness = await startUiHarness();

  try {
    await page.goto(`/?mock=0&token=${token}&api=${encodeURIComponent(harness.baseUrl)}#/settings`);

    await expect.poll(() => page.url().includes("token=")).toBe(false);
    await expect(page.getByTestId("settings-view")).toBeVisible();
    await expect(page.getByTestId("settings-view")).toContainText("not detected");
    await page.locator('input[aria-label="Model"]').fill("m2.7");
    await page.locator('input[aria-label="Cost cap"]').fill("4");
    await page.getByRole("button", { name: "Light" }).click();
    await expect(page.getByTestId("studio-shell")).toHaveAttribute("data-theme", "light");
    await page.getByRole("link", { name: /Sessions/ }).click();
    const composer = page.getByTestId("composer-input");
    await composer.fill("/context settings follow-through");
    await page.getByTestId("composer-send").click();
    await expect.poll(() => harness.messageBodies.length).toBe(1);
    expect(harness.messageBodies[0]).toEqual({
      message: "/context settings follow-through",
      provider: "glm",
      model: "m2.7",
    });
    expect(JSON.stringify(harness.messageBodies)).not.toContain(token);
    expect(JSON.stringify(harness.messageBodies)).not.toContain("approval");
    expect(JSON.stringify(harness.messageBodies)).not.toContain("cost");
    expect(JSON.stringify(harness.messageBodies)).not.toContain("tokenCap");

    await expectNoBrowserTokenLeak(page);
    await expectNoWorkspacePathLeak(page);
  } finally {
    await harness.close();
  }
});

test("does not send the live token to non-loopback API targets", async ({ page }) => {
  const pageRequests: string[] = [];
  page.on("request", (request) => pageRequests.push(request.url()));

  await page.goto(`/?mock=0&token=${token}&api=${encodeURIComponent("https://example.invalid")}`);
  await expect.poll(() => page.url().includes("token=")).toBe(false);
  await expect.poll(() => page.url().includes("api=")).toBe(false);
  await expect(page.getByTestId("studio-shell")).toBeVisible();
  await page.waitForTimeout(300);

  expect(pageRequests.some((url) => url.startsWith("https://example.invalid"))).toBe(false);
});

test("rejects loopback API URLs with credentials paths queries or hashes", async ({ page }) => {
  const pageRequests: string[] = [];
  page.on("request", (request) => pageRequests.push(request.url()));

  const unsafeApi = "http://user:syntheticApiSecret@127.0.0.1:7777/v1?token=syntheticApiToken#secret";
  await page.goto(`/?mock=0&token=${token}&api=${encodeURIComponent(unsafeApi)}`);
  await expect.poll(() => page.url().includes("token=")).toBe(false);
  await expect.poll(() => page.url().includes("api=")).toBe(false);
  await expect(page.getByTestId("studio-shell")).toBeVisible();
  await page.waitForTimeout(300);

  expect(pageRequests.some((url) => url.startsWith("http://127.0.0.1:7777"))).toBe(false);
  await expect(page.locator("body")).not.toContainText("syntheticApiSecret");
  await expect(page.locator("body")).not.toContainText("syntheticApiToken");
  await expectNoWorkspacePathLeak(page);
});

test("shows artifact API errors and truncates large previews defensively", async ({ page }) => {
  test.slow();

  const listHarness = await startUiHarness({ artifactMode: "list-error" });
  try {
    await page.goto(`/?mock=0&token=${token}&api=${encodeURIComponent(listHarness.baseUrl)}`);
    const composer = page.getByTestId("composer-input");
    await composer.fill("/context artifact list failure");
    await page.getByTestId("composer-send").click();
    await expect(page.getByTestId("transcript")).toContainText("ctx-live-token");
    await page.getByRole("button", { name: /Artifacts/ }).click();
    await expect(page.locator(".artifact-error")).toContainText("list failed");
    await expect(page.locator(".artifact-error")).not.toContainText("synthetic-artifact-token");
  } finally {
    await listHarness.close();
  }

  const fetchHarness = await startUiHarness({ artifactMode: "fetch-error" });
  try {
    await page.goto(`/?mock=0&token=${token}&api=${encodeURIComponent(fetchHarness.baseUrl)}`);
    const composer = page.getByTestId("composer-input");
    await composer.fill("/context artifact fetch failure");
    await page.getByTestId("composer-send").click();
    await expect(page.getByTestId("transcript")).toContainText("ctx-live-token");
    await page.getByRole("button", { name: /Artifacts/ }).click();
    await expect(page.getByTestId("artifact-list")).toContainText("context_packet.json");
    await expect(page.locator(".artifact-error")).toContainText("preview");
    await expect(page.locator(".artifact-error")).toContainText("failed");
    await expect(page.locator(".artifact-error")).not.toContainText("syntheticPreviewSecret");
  } finally {
    await fetchHarness.close();
  }

  const largeHarness = await startUiHarness({ artifactMode: "large" });
  try {
    await page.goto(`/?mock=0&token=${token}&api=${encodeURIComponent(largeHarness.baseUrl)}`);
    const composer = page.getByTestId("composer-input");
    await composer.fill("/context large artifact");
    await page.getByTestId("composer-send").click();
    await expect(page.getByTestId("transcript")).toContainText("ctx-live-token");
    await page.getByRole("button", { name: /Artifacts/ }).click();
    await page.getByRole("button", { name: /big_log\.txt/ }).click();
    await expect(page.getByTestId("artifact-truncation")).toContainText("Preview truncated");
    await expectNoPrivacyLeak(page.getByTestId("artifact-preview"));
    await expect(page.getByTestId("artifact-preview")).not.toContainText(syntheticLargeToken);
    await expect(page.getByTestId("artifact-preview")).not.toContainText(syntheticLargeRawPrompt);
    await expect(page.getByTestId("artifact-preview")).not.toContainText(syntheticLargeProviderRequestBody);
    await expect(page.getByTestId("artifact-preview")).not.toContainText(syntheticLargeProviderResponseBody);
    await expect(page.getByTestId("artifact-preview")).not.toContainText(syntheticLargeArtifactPath);
  } finally {
    await largeHarness.close();
  }

  const malformedTraceHarness = await startUiHarness({ artifactMode: "malformed-trace" });
  try {
    await page.goto(`/?mock=0&token=${token}&api=${encodeURIComponent(malformedTraceHarness.baseUrl)}`);
    const composer = page.getByTestId("composer-input");
    await composer.fill("/context malformed trace preview");
    await page.getByTestId("composer-send").click();
    await expect(page.getByTestId("transcript")).toContainText("ctx-live-token");
    await page.getByRole("button", { name: /Artifacts/ }).click();
    await page.getByRole("button", { name: /trace\.spans\.jsonl/ }).click();
    await expect(page.locator(".artifact-code.trace")).toContainText("not-json trace line");
    await expectNoPrivacyLeak(page.getByTestId("artifact-preview"));
    await expect(page.getByTestId("artifact-preview")).not.toContainText(
      `${tempWorkspacePrefix}/.mimir/runs/${runId}/trace.spans.jsonl`,
    );
  } finally {
    await malformedTraceHarness.close();
  }

  const unredactedHarness = await startUiHarness({ artifactMode: "unredacted" });
  try {
    await page.goto(`/?mock=0&token=${token}&api=${encodeURIComponent(unredactedHarness.baseUrl)}`);
    const composer = page.getByTestId("composer-input");
    await composer.fill("/context unredacted artifact");
    await page.getByTestId("composer-send").click();
    await expect(page.getByTestId("transcript")).toContainText("ctx-live-token");
    await page.getByRole("button", { name: /Artifacts/ }).click();
    await expect(page.getByTestId("artifact-preview")).toContainText("Preview unavailable");
    await expectNoSyntheticSecrets(page.getByTestId("artifact-preview"));
  } finally {
    await unredactedHarness.close();
  }
});

test("shows calm API, session, packet, artifact, and trace empty states", async ({ page }) => {
  test.slow();

  const statusHarness = await startUiHarness({ statusMode: "error" });
  try {
    await page.goto(`/?mock=0&token=${token}&api=${encodeURIComponent(statusHarness.baseUrl)}`);
    await expect(page.getByTestId("api-recovery-state")).toContainText("Local API disconnected");
    await expectNoPrivacyLeak(page.getByTestId("api-recovery-state"));
  } finally {
    await statusHarness.close();
  }

  const sessionHarness = await startUiHarness({ sessionMode: "create-error" });
  try {
    await page.goto(`/?mock=0&token=${token}&api=${encodeURIComponent(sessionHarness.baseUrl)}`);
    await expect(page.getByTestId("no-session-state")).toContainText("No active Studio session");
    await expect(page.getByTestId("composer-send")).toBeDisabled();
    await expectNoWorkspacePathLeak(page);
  } finally {
    await sessionHarness.close();
  }

  const emptyHarness = await startUiHarness({
    artifactMode: "empty",
    packetPreviewMode: "unavailable",
  });
  try {
    await page.goto(`/?mock=0&token=${token}&api=${encodeURIComponent(emptyHarness.baseUrl)}`);
    const composer = page.getByTestId("composer-input");
    await composer.fill("/context unavailable previews");
    await page.getByTestId("composer-send").click();
    await expect(page.getByTestId("transcript")).toContainText("ctx-live-token");

    await page.getByRole("button", { name: /Preview replay/ }).click();
    await expect(page.locator(".artifact-error")).toContainText("packet replay/share preview is unavailable");
    await expectNoPrivacyLeak(page.locator(".artifact-error"));
    await page.getByRole("button", { name: /Preview share/ }).click();
    await expect(page.locator(".artifact-error")).toContainText("packet replay/share preview is unavailable");
    await expectNoPrivacyLeak(page.locator(".artifact-error"));
    await composer.fill(`/share ${runId}`);
    await page.getByTestId("composer-send").click();
    await expect(page.locator(".error-strip")).toContainText("packet replay/share preview is unavailable");
    await expectNoPrivacyLeak(page.locator(".error-strip"));

    await page.getByRole("button", { name: /Artifacts/ }).click();
    await expect(page.getByTestId("artifact-list")).toContainText("No artifacts for this run");
    await expect(page.getByTestId("trace-state")).toContainText("No trace recorded for this run");
    await expectNoWorkspacePathLeak(page);
  } finally {
    await emptyHarness.close();
  }
});

test("rejects malformed local API shapes without leaking response bodies", async ({ page }) => {
  test.slow();

  const statusHarness = await startUiHarness({ statusMode: "bad-shape" });
  try {
    await page.goto(`/?mock=0&token=${token}&api=${encodeURIComponent(statusHarness.baseUrl)}`);
    await expect(page.getByTestId("api-recovery-state")).toContainText(
      "unexpected shape for workspace status response",
    );
    await expectNoPrivacyLeak(page.getByTestId("api-recovery-state"));
  } finally {
    await statusHarness.close();
  }

  const sessionHarness = await startUiHarness({ sessionMode: "bad-create-shape" });
  try {
    await page.goto(`/?mock=0&token=${token}&api=${encodeURIComponent(sessionHarness.baseUrl)}`);
    await expect(page.locator(".error-strip")).toContainText(
      "unexpected shape for session create response",
    );
    await expectNoPrivacyLeak(page.locator(".error-strip"));
  } finally {
    await sessionHarness.close();
  }

  const unknownEventHarness = await startUiHarness({ sessionMode: "bad-event-type" });
  try {
    await page.goto(`/?mock=0&token=${token}&api=${encodeURIComponent(unknownEventHarness.baseUrl)}`);
    await expect(page.locator(".error-strip")).toContainText(
      "unexpected shape for session create response",
    );
    await expectNoPrivacyLeak(page.locator(".error-strip"));
  } finally {
    await unknownEventHarness.close();
  }

  const malformedEventHarness = await startUiHarness({ sessionMode: "bad-event-payload" });
  try {
    await page.goto(`/?mock=0&token=${token}&api=${encodeURIComponent(malformedEventHarness.baseUrl)}`);
    await expect(page.locator(".error-strip")).toContainText(
      "unexpected shape for session create response",
    );
    await expectNoPrivacyLeak(page.locator(".error-strip"));
  } finally {
    await malformedEventHarness.close();
  }

  const artifactHarness = await startUiHarness({ artifactMode: "bad-shape" });
  try {
    await page.goto(`/?mock=0&token=${token}&api=${encodeURIComponent(artifactHarness.baseUrl)}`);
    const composer = page.getByTestId("composer-input");
    await composer.fill("/context malformed artifact response");
    await page.getByTestId("composer-send").click();
    await expect(page.getByTestId("transcript")).toContainText("ctx-live-token");
    await page.getByRole("button", { name: /Artifacts/ }).click();
    await expect(page.locator(".artifact-error")).toContainText(
      "unexpected shape for artifact list response",
    );
    await expectNoPrivacyLeak(page.locator(".artifact-error"));
  } finally {
    await artifactHarness.close();
  }

  const previewHarness = await startUiHarness({ packetPreviewMode: "bad-shape" });
  try {
    await page.goto(`/?mock=0&token=${token}&api=${encodeURIComponent(previewHarness.baseUrl)}`);
    const composer = page.getByTestId("composer-input");
    await composer.fill("/context malformed packet preview");
    await page.getByTestId("composer-send").click();
    await expect(page.getByTestId("transcript")).toContainText("ctx-live-token");

    await page.getByRole("button", { name: /Preview replay/ }).click();
    await expect(page.locator(".artifact-error")).toContainText(
      "unexpected shape for packet replay response",
    );
    await expectNoPrivacyLeak(page.locator(".artifact-error"));

    await page.getByRole("button", { name: /Preview share/ }).click();
    await expect(page.locator(".artifact-error")).toContainText(
      "unexpected shape for packet share response",
    );
    await expectNoPrivacyLeak(page.locator(".artifact-error"));
  } finally {
    await previewHarness.close();
  }
});

test("blocks unredacted replay and share packet previews", async ({ page }) => {
  const harness = await startUiHarness({ packetPreviewMode: "unredacted" });

  try {
    await page.goto(`/?mock=0&token=${token}&api=${encodeURIComponent(harness.baseUrl)}`);
    const composer = page.getByTestId("composer-input");
    await composer.fill("/context inspect unredacted packet guards");
    await page.getByTestId("composer-send").click();
    await expect(page.getByTestId("transcript")).toContainText("ctx-live-token");

    await page.getByRole("button", { name: /Preview replay/ }).click();
    await expect(page.getByTestId("packet-replay-preview")).toContainText("Preview unavailable");
    await expectNoPrivacyLeak(page.getByTestId("packet-replay-preview"));

    await page.getByRole("button", { name: /Preview share/ }).click();
    await expect(page.getByTestId("packet-share-preview")).toContainText("Preview unavailable");
    await expectNoPrivacyLeak(page.getByTestId("packet-share-preview"));
  } finally {
    await harness.close();
  }
});

test("loads selected live sessions and reconnects streams with per-session cursors", async ({
  page,
}) => {
  const harness = await startUiHarness({ multiSession: true });

  try {
    await page.goto(`/?mock=0&token=${token}&api=${encodeURIComponent(harness.baseUrl)}`);

    await expect(page.getByTestId(`session-row-${sessionId}`)).toHaveClass(/active/);
    await expect(page.getByTestId("transcript")).toContainText("Live token smoke");
    await expectNoWorkspacePathLeak(page);
    await expectNoSyntheticSecrets(page.getByTestId("transcript"));
    await expect
      .poll(() => harness.webSocketRequests.join("|"))
      .toContain(`/v1/sessions/${sessionId}/events?after=1`);
    expectNoWebSocketUrlTokenLeak(harness.webSocketRequests);

    await page.getByTestId(`session-row-${secondSessionId}`).click();
    await expect(page.getByTestId(`session-row-${secondSessionId}`)).toHaveClass(/active/);
    await expect(page.getByTestId("transcript")).toContainText("Second live session ready");
    await expectNoPrivacyLeak(page.getByTestId("transcript"));
    await expect
      .poll(() => harness.requests.join("|"))
      .toContain(`GET /v1/sessions/${secondSessionId}`);
    await expect
      .poll(() => harness.webSocketRequests.join("|"))
      .toContain(`/v1/sessions/${secondSessionId}/events?after=7`);
    expectNoWebSocketUrlTokenLeak(harness.webSocketRequests);

    const composer = page.getByTestId("composer-input");
    await composer.fill("/resume ");
    await expect(page.getByTestId("resume-palette")).toContainText("Live token smoke");
    await page.getByTestId(`resume-option-${sessionId}`).click();
    await expect(page.getByTestId(`session-row-${sessionId}`)).toHaveClass(/active/);
    await expect(page.getByTestId("transcript")).toContainText("Live token smoke");
    await expectNoPrivacyLeak(page.getByTestId("transcript"));
    await expect
      .poll(() => lastMatching(harness.webSocketRequests, `/v1/sessions/${sessionId}/events`))
      .toContain(`after=1`);
  } finally {
    await harness.close();
  }
});

test("reconnects after a dropped stream with the latest cursor", async ({ page }) => {
  const harness = await startUiHarness({ disconnectAfterFirstReplay: true });

  try {
    await page.goto(`/?mock=0&token=${token}&api=${encodeURIComponent(harness.baseUrl)}`);

    await expect(page.getByTestId("transcript")).toContainText("Live token smoke");
    await expect
      .poll(() => harness.webSocketRequests.join("|"))
      .toContain(`/v1/sessions/${sessionId}/events?after=1`);
    expectNoWebSocketUrlTokenLeak(harness.webSocketRequests);

    await expect(page.getByTestId("transcript")).toContainText("stream replay");
    await expect(page.getByText("stream replay")).toHaveCount(1);
    await expect(page.getByTestId("transcript")).not.toContainText("stale session leak");
    await expectNoPrivacyLeak(page.getByTestId("transcript"));

    await expect
      .poll(() => lastMatching(harness.webSocketRequests, `/v1/sessions/${sessionId}/events`))
      .toContain("after=2");
    await expect(page.getByTestId("transcript")).toContainText("Reconnect replay complete");
  } finally {
    await harness.close();
  }
});

async function expectNoBrowserTokenLeak(page: Page): Promise<void> {
  const leakSnapshot = await page.evaluate(() => {
    const storageEntries = (storage: Storage) =>
      Array.from({ length: storage.length }, (_, index) => {
        const key = storage.key(index) ?? "";
        return [key, storage.getItem(key) ?? ""];
      });

    return {
      body: document.body.textContent ?? "",
      html: document.documentElement.innerHTML,
      localStorage: storageEntries(window.localStorage),
      sessionStorage: storageEntries(window.sessionStorage),
      url: window.location.href,
    };
  });

  expect(JSON.stringify(leakSnapshot)).not.toContain(token);
  expect(leakSnapshot.localStorage).toHaveLength(0);
  expect(leakSnapshot.sessionStorage).toHaveLength(0);
}

async function expectNoWorkspacePathLeak(page: Page): Promise<void> {
  const leakSnapshot = await page.evaluate(() => {
    const storageEntries = (storage: Storage) =>
      Array.from({ length: storage.length }, (_, index) => {
        const key = storage.key(index) ?? "";
        return [key, storage.getItem(key) ?? ""];
      });

    return {
      body: document.body.textContent ?? "",
      html: document.documentElement.innerHTML,
      localStorage: storageEntries(window.localStorage),
      sessionStorage: storageEntries(window.sessionStorage),
      url: window.location.href,
    };
  });

  expect(JSON.stringify(leakSnapshot)).not.toContain(tempWorkspacePrefix);
}

async function expectNoPrivacyLeak(locator: Locator): Promise<void> {
  const text = await locatorText(locator);
  expect(text).not.toContain(token);
  expect(text).not.toContain(tempWorkspacePrefix);
  expectNoSyntheticSecretText(text);
}

async function expectNoSyntheticSecrets(locator: Locator): Promise<void> {
  expectNoSyntheticSecretText(await locatorText(locator));
}

async function locatorText(locator: Locator): Promise<string> {
  await expect(locator.first()).toBeAttached();
  return (await locator.allTextContents()).join("\n");
}

function expectNoSyntheticSecretText(text: string): void {
  for (const secret of syntheticSecrets) {
    expect(text).not.toContain(secret);
  }
}

function expectNoWebSocketUrlTokenLeak(urls: string[]): void {
  const combinedUrls = urls.join("|");
  const studioEventUrls = urls.filter(isStudioEventStreamUrl).join("|");
  expect(combinedUrls).not.toContain(token);
  expect(studioEventUrls).not.toContain("token=");
}

function isStudioEventStreamUrl(url: string): boolean {
  return url.includes("/v1/sessions/") && url.includes("/events");
}

async function startUiHarness({
  artifactMode = "ok",
  disconnectAfterFirstReplay = false,
  multiSession = false,
  packetPreviewMode = "ok",
  sessionMode = "ok",
  statusMode = "ok",
}: {
  artifactMode?: "ok" | "list-error" | "fetch-error" | "large" | "malformed-trace" | "unredacted" | "empty" | "bad-shape";
  disconnectAfterFirstReplay?: boolean;
  multiSession?: boolean;
  packetPreviewMode?: "ok" | "unavailable" | "unredacted" | "bad-shape";
  sessionMode?: "ok" | "create-error" | "bad-create-shape" | "bad-event-type" | "bad-event-payload";
  statusMode?: "ok" | "error" | "bad-shape";
} = {}): Promise<{
  baseUrl: string;
  messageBodies: Array<Record<string, unknown>>;
  requests: string[];
  webSocketRequests: string[];
  webSocketProtocols: string[];
  close: () => Promise<void>;
}> {
  const events = [
    makeEvent(1, "session.created", {
      title: "Live token smoke",
      workspace_name: "mimir-live-token",
    }),
  ];
  const metadata: SessionMetadata = {
    schema_version: 1,
    session_id: sessionId,
    title: "Live token smoke",
    workspace_name: "mimir-live-token",
    created_at: timestamp,
    updated_at: timestamp,
  };
  const secondEvents = [
    makeEvent(1, "session.created", {
      title: "Second live session",
      workspace_name: "mimir-live-token",
    }, secondSessionId),
    makeEvent(6, "turn.started", {
      turn_id: "turn-second-status",
      command: "status",
      task: "resume previous work",
    }, secondSessionId),
    makeEvent(7, "turn.completed", {
      turn_id: "turn-second-status",
      summary: "Second live session ready",
    }, secondSessionId),
  ];
  const secondMetadata: SessionMetadata = {
    schema_version: 1,
    session_id: secondSessionId,
    title: "Second live session",
    workspace_name: "mimir-live-token",
    created_at: timestamp,
    updated_at: timestamp,
  };
  const sessionRecords = new Map<
    string,
    { metadata: SessionMetadata; events: SessionEvent[] }
  >([[sessionId, { metadata, events }]]);
  if (multiSession) {
    sessionRecords.set(secondSessionId, { metadata: secondMetadata, events: secondEvents });
  }
  const sockets = new Set<{ destroy: () => void }>();
  const messageBodies: Array<Record<string, unknown>> = [];
  const requests: string[] = [];
  const webSocketRequests: string[] = [];
  const webSocketProtocols: string[] = [];
  const streamOpenCounts = new Map<string, number>();

  const server = createServer(async (request, response) => {
    const url = new URL(request.url ?? "/", "http://127.0.0.1");
    requests.push(`${request.method} ${url.pathname}${url.search}`);

    if (request.method === "OPTIONS") {
      writeCors(response, 204);
      response.end();
      return;
    }

    if (!isAuthorized(request, url)) {
      writeJson(response, 401, { error: "missing or invalid UI token" });
      return;
    }

    if (request.method === "GET" && url.pathname === "/v1/workspace/status") {
      if (statusMode === "error") {
        writeJson(response, 503, {
          error: `${tempWorkspacePrefix}/.mimir status unavailable with api_key=${syntheticStatusApiKey}`,
        });
        return;
      }
      if (statusMode === "bad-shape") {
        writeJson(response, 200, {
          workspace_name: "mimir-live-token",
          git: "not a git status",
          leak: syntheticMalformedShapeSecret,
        });
        return;
      }
      writeJson(response, 200, {
        workspace_name: "mimir-live-token",
        git: { is_repo: true, branch: "phase6/memory-server-tui", dirty: true },
        mimir: {
          initialized: true,
          config_present: true,
          checks_loaded: 2,
          sessions_count: 1,
          runs_count: 1,
          recent_runs: [
            {
              run_id: runId,
              path: `${tempWorkspacePrefix}/.mimir/runs/${runId}?token=${syntheticRunPathToken}`,
              artifact_count: 4,
              has_context_packet: true,
              trace_status: { state: "recorded", redacted: true },
            },
          ],
        },
        providers: [{ provider: "glm", models_count: 1, credential_detected: false }],
      });
      return;
    }

    if (request.method === "GET" && url.pathname === "/v1/sessions") {
      writeJson(
        response,
        200,
        multiSession ? Array.from(sessionRecords.values()).map((record) => record.metadata) : [],
      );
      return;
    }

    if (request.method === "POST" && url.pathname === "/v1/sessions") {
      await readBody(request);
      if (sessionMode === "create-error") {
        writeJson(response, 500, {
          error: `${tempWorkspacePrefix}/.mimir session create failed with token=${syntheticCreateSecret}`,
        });
        return;
      }
      if (sessionMode === "bad-create-shape") {
        writeJson(response, 200, {
          metadata: {
            session_id: sessionId,
            title: `shape leak ${syntheticMalformedShapeSecret}`,
          },
          events: "not events",
        });
        return;
      }
      if (sessionMode === "bad-event-type") {
        writeJson(response, 200, {
          metadata,
          events: [
            rawEvent(1, "provider.called", {
              title: `unknown event leak ${syntheticMalformedShapeSecret}`,
              workspace_name: "mimir-live-token",
            }),
          ],
        });
        return;
      }
      if (sessionMode === "bad-event-payload") {
        writeJson(response, 200, {
          metadata,
          events: [
            rawEvent(1, "context.packet.ready", {
              run_id: runId,
              packet_id: "ctx-live-token",
              packet_hash: "c".repeat(64),
              packet_path: `${tempWorkspacePrefix}/.mimir/runs/${runId}/context_packet.json`,
              estimated_input_tokens: "not a number",
              guidance_files: ["AGENTS.md"],
              likely_files: [syntheticMalformedShapeSecret],
            }),
          ],
        });
        return;
      }
      writeJson(response, 200, { metadata, events });
      return;
    }

    const sessionLoadMatch = url.pathname.match(/^\/v1\/sessions\/([^/]+)$/);
    if (request.method === "GET" && sessionLoadMatch) {
      const record = sessionRecords.get(decodeURIComponent(sessionLoadMatch[1]));
      if (!record) {
        writeJson(response, 404, { error: "unknown session" });
        return;
      }
      writeJson(response, 200, { metadata: record.metadata, events: record.events });
      return;
    }

    const messageMatch = url.pathname.match(/^\/v1\/sessions\/([^/]+)\/messages$/);
    if (request.method === "POST" && messageMatch) {
      const targetSessionId = decodeURIComponent(messageMatch[1]);
      const record = sessionRecords.get(targetSessionId);
      if (!record) {
        writeJson(response, 404, { error: "unknown session" });
        return;
      }
      const body = JSON.parse(await readBody(request)) as Record<string, unknown>;
      messageBodies.push(body);
      const message = typeof body.message === "string" ? body.message : "";
      const command = message.trim().startsWith("/")
        ? message.trim().slice(1).split(/\s+/, 1)[0]
        : "context";
      if (command === "runs") {
        const sequence = nextSequence(record.events);
        const turnId = `turn-runs-${sequence}`;
        const next = [
          makeEvent(sequence, "turn.started", {
            turn_id: turnId,
            command: "runs",
            task: "",
          }, targetSessionId),
          makeEvent(sequence + 1, "workspace.status.ready", {
            status: {
              runs: [
                {
                  run_id: runId,
                  path: `${tempWorkspacePrefix}/.mimir/runs/${runId}?token=${syntheticRunPathToken}`,
                  artifact_count: 4,
                  has_context_packet: true,
                  trace_status: { state: "recorded", redacted: true },
                },
              ],
            },
          }, targetSessionId),
          makeEvent(sequence + 2, "turn.completed", {
            turn_id: turnId,
            summary: "Listed local Mimir runs",
          }, targetSessionId),
        ];
        record.events.push(...next);
        writeJson(response, 200, {
          session_id: targetSessionId,
          command,
          result: {
            runs: [
              {
                run_id: runId,
                path: `${tempWorkspacePrefix}/.mimir/runs/${runId}?token=${syntheticRunPathToken}`,
                artifact_count: 4,
                has_context_packet: true,
                trace_status: { state: "recorded", redacted: true },
              },
            ],
          },
          events: next,
        });
        return;
      }
      if (command === "why") {
        const path = message.replace(/^\/why\s*/, "").trim() || "apps/studio/src/App.tsx";
        const sequence = nextSequence(record.events);
        const result = {
          path,
          status: "included",
          reason: "included in the context packet",
          reason_code: "semantic_match",
          token_count: 320,
          run_id: runId,
          packet_id: "ctx-live-token",
          packet_hash: "c".repeat(64),
          packet_path: `${tempWorkspacePrefix}/.mimir/runs/${runId}/context_packet.json`,
          source_hash: "d".repeat(64),
        };
        const next = [
          makeEvent(sequence, "turn.started", {
            turn_id: `turn-why-${sequence}`,
            command: "why",
            task: path,
          }, targetSessionId),
          makeEvent(sequence + 1, "workspace.status.ready", {
            status: result,
          }, targetSessionId),
          makeEvent(sequence + 2, "turn.completed", {
            turn_id: `turn-why-${sequence}`,
            summary: "Context why lookup: included",
          }, targetSessionId),
        ];
        record.events.push(...next);
        writeJson(response, 200, {
          session_id: targetSessionId,
          command,
          result,
          events: next,
        });
        return;
      }
      const next = contextEvents(
        nextSequence(record.events),
        message,
        targetSessionId,
      );
      record.events.push(...next);
      writeJson(response, 200, {
        session_id: targetSessionId,
        command: "context",
        result: {},
        events: next,
      });
      return;
    }

    if (request.method === "GET" && url.pathname === `/v1/runs/${runId}/replay`) {
      if (packetPreviewMode === "unavailable") {
        writeJson(response, 404, {
          error: `${tempWorkspacePrefix}/.mimir/runs/${runId} packet replay/share preview is unavailable with api_key=${syntheticPreviewSecret}`,
        });
        return;
      }
      if (packetPreviewMode === "bad-shape") {
        writeJson(response, 200, {
          run_id: runId,
          packet_id: "ctx-live-token",
          request: { accessToken: syntheticMalformedShapeSecret },
        });
        return;
      }
      const redacted = packetPreviewMode !== "unredacted";
      writeJson(response, 200, {
        run_id: runId,
        packet_id: "ctx-live-token",
        packet_hash: "c".repeat(64),
        packet_path: `${tempWorkspacePrefix}/.mimir/runs/${runId}/context_packet.json`,
        source: "saved_artifact",
        provider_request_sha256: "f".repeat(64),
        user_prompt_sha256: "e".repeat(64),
        redacted,
        request: {
          model: "glm-5.1",
          messages: [{ role: "user", content: `redacted replay request ${syntheticSecretText}` }],
        },
      });
      return;
    }

    if (
      (request.method === "GET" || request.method === "POST") &&
      url.pathname === `/v1/runs/${runId}/share`
    ) {
      if (packetPreviewMode === "unavailable") {
        writeJson(response, 404, {
          error: `${tempWorkspacePrefix}/.mimir/runs/${runId} packet replay/share preview is unavailable with api_key=${syntheticPreviewSecret}`,
        });
        return;
      }
      if (packetPreviewMode === "bad-shape") {
        writeJson(response, 200, {
          run_id: runId,
          packet_id: "ctx-live-token",
          bundle: { accessToken: syntheticMalformedShapeSecret },
        });
        return;
      }
      const redacted = packetPreviewMode !== "unredacted";
      writeJson(response, 200, {
        run_id: runId,
        packet_id: "ctx-live-token",
        packet_hash: "c".repeat(64),
        packet_path: `${tempWorkspacePrefix}/.mimir/runs/${runId}/context_packet.json`,
        bundle_sha256: "b".repeat(64),
        redacted,
        bundle: {
          kind: "mimir.packet_share",
          run_id: runId,
          packet_hash: "c".repeat(64),
          metadata: syntheticSecretText,
          replay: { provider_request_sha256: "f".repeat(64) },
        },
      });
      return;
    }

    if (request.method === "GET" && url.pathname === "/v1/workspace/files") {
      writeJson(response, 200, {
        results: [
          { path: "apps/studio/src/App.tsx", kind: "file", line: null, symbol: null },
          { path: "apps/studio/src/api/client.ts", kind: "file", line: null, symbol: null },
        ],
      });
      return;
    }

    if (request.method === "GET" && url.pathname === `/v1/runs/${runId}/artifacts`) {
      if (artifactMode === "list-error") {
        writeJson(response, 500, {
          error: `artifact token: ${syntheticArtifactListToken} list failed`,
        });
        return;
      }
      if (artifactMode === "empty") {
        writeJson(response, 200, {
          run_id: runId,
          trace_status: { state: "absent", redacted: false },
          artifacts: [],
        });
        return;
      }
      if (artifactMode === "bad-shape") {
        writeJson(response, 200, {
          run_id: runId,
          trace_status: { state: "recorded", redacted: true },
          artifacts: [
            {
              name: "context_packet.json",
              path: `${tempWorkspacePrefix}/.mimir/runs/${runId}/context_packet.json`,
              redacted: true,
              accessToken: syntheticMalformedShapeSecret,
            },
          ],
        });
        return;
      }
      const artifacts = [
        {
          name: "context_packet.json",
          path: `.mimir/runs/${runId}/context_packet.json`,
          size_bytes: 72,
          sha256: "abc123",
          redacted: true,
        },
        {
          name: "patch.diff",
          path: `.mimir/runs/${runId}/patch.diff`,
          size_bytes: 48,
          sha256: "def456",
          redacted: true,
        },
        {
          name: "notes.md",
          path: `.mimir/runs/${runId}/notes.md`,
          size_bytes: 56,
          sha256: "ghi789",
          redacted: true,
        },
        {
          name: "trace.spans.jsonl",
          path: `.mimir/runs/${runId}/trace.spans.jsonl`,
          size_bytes: 160,
          sha256: "jkl012",
          redacted: true,
        },
      ];
      if (artifactMode === "large") {
        artifacts.push({
          name: "big_log.txt",
          path: `.mimir/runs/${runId}/big_log.txt`,
          size_bytes: 9_000,
          sha256: "mno345",
          redacted: true,
        });
      }
      writeJson(response, 200, {
        run_id: runId,
        trace_status: { state: "recorded", redacted: true },
        artifacts,
      });
      return;
    }

    if (
      request.method === "GET" &&
      url.pathname === `/v1/runs/${runId}/artifacts/context_packet.json`
    ) {
      if (artifactMode === "fetch-error") {
        writeJson(response, 500, {
          error: `preview api_key=${syntheticPreviewSecret} failed`,
        });
        return;
      }
      writeJson(response, 200, {
        name: "context_packet.json",
        path: `.mimir/runs/${runId}/context_packet.json`,
        content_type: "application/json",
        sha256: "abc123",
        redacted: artifactMode !== "unredacted",
        content: {
          schema_version: 1,
          packet_id: "ctx-live-token",
          accessToken: syntheticSecretText,
        },
      });
      return;
    }

    if (request.method === "GET" && url.pathname === `/v1/runs/${runId}/artifacts/big_log.txt`) {
      writeJson(response, 200, {
        name: "big_log.txt",
        path: `.mimir/runs/${runId}/big_log.txt`,
        content_type: "text/plain",
        sha256: "mno345",
        redacted: true,
        content:
          `${Array.from({ length: 800 }, (_, index) => `log line ${index}`).join("\n")}\n` +
          `raw_prompt=${syntheticLargeRawPrompt}\n` +
          `provider_request=${syntheticLargeProviderRequestBody}\n` +
          `provider_response=${syntheticLargeProviderResponseBody}\n` +
          `artifact_path=${syntheticLargeArtifactPath}\n` +
          `token: ${syntheticLargeToken}`,
      });
      return;
    }

    if (request.method === "GET" && url.pathname === `/v1/runs/${runId}/artifacts/patch.diff`) {
      writeJson(response, 200, {
        name: "patch.diff",
        path: `.mimir/runs/${runId}/patch.diff`,
        content_type: "text/x-diff",
        sha256: "def456",
        redacted: true,
        content: "--- a/example.txt\n+++ b/example.txt\n@@ -1 +1 @@\n-old line\n+new line\n",
      });
      return;
    }

    if (request.method === "GET" && url.pathname === `/v1/runs/${runId}/artifacts/notes.md`) {
      writeJson(response, 200, {
        name: "notes.md",
        path: `.mimir/runs/${runId}/notes.md`,
        content_type: "text/markdown",
        sha256: "ghi789",
        redacted: true,
        content: "# Review notes\n- JSON preview loads automatically\n- Diff preview remains selectable\n",
      });
      return;
    }

    if (
      request.method === "GET" &&
      url.pathname === `/v1/runs/${runId}/artifacts/trace.spans.jsonl`
    ) {
      if (artifactMode === "malformed-trace") {
        writeJson(response, 200, {
          name: "trace.spans.jsonl",
          path: `.mimir/runs/${runId}/trace.spans.jsonl`,
          content_type: "text/plain; charset=utf-8",
          sha256: "jkl012",
          redacted: true,
          content:
            `{"schema_version":1,"span_id":"a3f9b2c14e8d1c5f","name":"artifact.preview","start_us":1779131722000000,"end_us":1779131722412000,"attrs":{"api_key":"${syntheticStatusApiKey}","path":"${tempWorkspacePrefix}/src/private_trace.rs"}}\n` +
            `not-json trace line path=${tempWorkspacePrefix}/.mimir/runs/${runId}/trace.spans.jsonl api_key=${syntheticStatusApiKey}\n`,
        });
        return;
      }
      writeJson(response, 200, {
        name: "trace.spans.jsonl",
        path: `.mimir/runs/${runId}/trace.spans.jsonl`,
        content_type: "text/plain; charset=utf-8",
        sha256: "jkl012",
        redacted: true,
        content:
          '{"schema_version":1,"span_id":"a3f9b2c14e8d1c5f","trace_id":"0123456789abcdef0123456789abcdef","name":"artifact.preview","kind":"internal","start_us":1779131722000000,"end_us":1779131722412000}\\n',
      });
      return;
    }

    writeJson(response, 404, { error: `Unhandled ${request.method} ${url.pathname}` });
  });

  server.on("connection", (socket) => {
    sockets.add(socket);
    socket.on("close", () => sockets.delete(socket));
  });

  server.on("upgrade", (request, socket) => {
    const url = new URL(request.url ?? "/", "http://127.0.0.1");
    const requestedProtocols = webSocketProtocolValues(request);
    const streamMatch = url.pathname.match(/^\/v1\/sessions\/([^/]+)\/events$/);
    const targetSessionId = streamMatch ? decodeURIComponent(streamMatch[1]) : "";
    const record = sessionRecords.get(targetSessionId);
    if (!isAuthorized(request, url) || !streamMatch || !record) {
      socket.destroy();
      return;
    }

    const key = request.headers["sec-websocket-key"];
    if (typeof key !== "string") {
      socket.destroy();
      return;
    }

    const accept = createHash("sha1")
      .update(`${key}258EAFA5-E914-47DA-95CA-C5AB0DC85B11`)
      .digest("base64");
    const responseHeaders = [
      "HTTP/1.1 101 Switching Protocols",
      "Upgrade: websocket",
      "Connection: Upgrade",
      `Sec-WebSocket-Accept: ${accept}`,
    ];
    if (requestedProtocols.includes(wsStableProtocol)) {
      responseHeaders.push(`Sec-WebSocket-Protocol: ${wsStableProtocol}`);
    }
    responseHeaders.push("\r\n");
    socket.write(responseHeaders.join("\r\n"));

    webSocketRequests.push(`${url.pathname}${url.search}`);
    webSocketProtocols.push(requestedProtocols.join(","));
    const openCount = (streamOpenCounts.get(targetSessionId) ?? 0) + 1;
    streamOpenCounts.set(targetSessionId, openCount);
    const after = Number(url.searchParams.get("after") ?? 0);
    const replayEvents = [
      ...record.events,
      ...disconnectReplayEvents(disconnectAfterFirstReplay, targetSessionId, openCount),
    ].filter((item) => item.sequence > after);
    for (const event of replayEvents) {
      socket.write(encodeWebSocketText(JSON.stringify(event)));
    }
    if (disconnectAfterFirstReplay && targetSessionId === sessionId && openCount === 1) {
      setTimeout(() => socket.end(), 25);
    }
  });

  await new Promise<void>((resolve) => server.listen(0, "127.0.0.1", resolve));
  const address = server.address() as AddressInfo;

  return {
    baseUrl: `http://127.0.0.1:${address.port}`,
    messageBodies,
    requests,
    webSocketRequests,
    webSocketProtocols,
    close: () =>
      new Promise((resolve) => {
        for (const socket of sockets) {
          socket.destroy();
        }
        server.close(() => resolve());
      }),
  };
}

function disconnectReplayEvents(
  enabled: boolean,
  targetSessionId: string,
  openCount: number,
): SessionEvent[] {
  if (!enabled || targetSessionId !== sessionId) {
    return [];
  }

  if (openCount === 1) {
    const replayed = makeEvent(
      2,
      "turn.started",
      { turn_id: "turn-stream-replay", command: "status", task: "stream replay" },
      sessionId,
    );
    return [
      replayed,
      replayed,
      makeEvent(
        98,
        "turn.completed",
        {
          turn_id: "turn-stale-session",
          summary: `stale session leak ${tempWorkspacePrefix} api_key=${syntheticPreviewSecret}`,
        },
        secondSessionId,
      ),
    ];
  }

  if (openCount === 2) {
    return [
      makeEvent(3, "turn.completed", {
        turn_id: "turn-stream-replay",
        summary: "Reconnect replay complete",
      }, sessionId),
    ];
  }

  return [];
}

function contextEvents(
  startSequence: number,
  task: string,
  targetSessionId = sessionId,
): SessionEvent[] {
  const turnId = `turn-context-${startSequence}`;
  return [
    makeEvent(startSequence, "turn.started", {
      turn_id: turnId,
      command: "context",
      task,
    }, targetSessionId),
    makeEvent(
      startSequence + 1,
      "context.build.started",
      { turn_id: turnId, provider: "glm", model: "default" },
      targetSessionId,
    ),
    makeEvent(startSequence + 2, "context.packet.ready", {
      run_id: runId,
      packet_id: "ctx-live-token",
      packet_hash: "c".repeat(64),
      packet_path: `${tempWorkspacePrefix}/.mimir/runs/${runId}/context_packet.json`,
      estimated_input_tokens: 42_120,
      guidance_files: ["AGENTS.md"],
      likely_files: ["apps/studio/src/App.tsx"],
    }, targetSessionId),
    makeEvent(startSequence + 3, "context.omission.risk", {
      run_id: runId,
      path: "apps/studio/tests/studio-live-token.spec.ts",
      reason: "live-token omission reason",
      risk: "test_missing",
    }, targetSessionId),
    makeEvent(startSequence + 4, "artifact.written", {
      run_id: runId,
      artifact_kind: "context_packet",
      path: `${tempWorkspacePrefix}/.mimir/runs/${runId}/context_packet.json`,
    }, targetSessionId),
    makeEvent(startSequence + 5, "turn.completed", {
      turn_id: turnId,
      summary: "Context ready",
    }, targetSessionId),
  ];
}

function makeEvent<TType extends SessionEvent["type"]>(
  sequence: number,
  type: TType,
  payload: Extract<SessionEvent, { type: TType }>["payload"],
  targetSessionId = sessionId,
): Extract<SessionEvent, { type: TType }> {
  return {
    schema_version: 1,
    event_id: `${targetSessionId}-evt-${sequence}`,
    session_id: targetSessionId,
    sequence,
    timestamp,
    type,
    payload,
  } as Extract<SessionEvent, { type: TType }>;
}

function rawEvent(
  sequence: number,
  type: string,
  payload: Record<string, unknown>,
): Record<string, unknown> {
  return {
    schema_version: 1,
    event_id: `${sessionId}-evt-${sequence}`,
    session_id: sessionId,
    sequence,
    timestamp,
    type,
    payload,
  };
}

function lastMatching(values: string[], needle: string): string {
  return [...values].reverse().find((value) => value.includes(needle)) ?? "";
}

function nextSequence(events: SessionEvent[]): number {
  return Math.max(0, ...events.map((event) => event.sequence)) + 1;
}

function isAuthorized(request: IncomingMessage, url: URL): boolean {
  return (
    request.headers.authorization === `Bearer ${token}` ||
    request.headers["x-mimir-token"] === token ||
    url.searchParams.get("token") === token ||
    webSocketProtocolToken(request) === token
  );
}

function webSocketProtocolToken(request: IncomingMessage): string | null {
  const tokenProtocol = webSocketProtocolValues(request).find((value) =>
    value.startsWith(wsTokenProtocolPrefix),
  );
  if (!tokenProtocol) {
    return null;
  }
  return hexDecode(tokenProtocol.slice(wsTokenProtocolPrefix.length));
}

function webSocketProtocolValues(request: IncomingMessage): string[] {
  const header = request.headers["sec-websocket-protocol"];
  const values = Array.isArray(header) ? header : header ? [header] : [];
  return values
    .flatMap((value) => value.split(","))
    .map((value) => value.trim())
    .filter(Boolean);
}

function hexDecode(value: string): string | null {
  if (!/^(?:[a-f0-9]{2})+$/i.test(value)) {
    return null;
  }

  return Buffer.from(value, "hex").toString("utf8");
}

function writeJson(response: ServerResponse, status: number, body: unknown): void {
  writeCors(response, status);
  response.setHeader("Content-Type", "application/json");
  response.end(JSON.stringify(body));
}

function writeCors(response: ServerResponse, status: number): void {
  response.statusCode = status;
  response.setHeader("Access-Control-Allow-Origin", "*");
  response.setHeader("Access-Control-Allow-Headers", "authorization,content-type,x-mimir-token");
  response.setHeader("Access-Control-Allow-Methods", "GET,POST,OPTIONS");
}

async function readBody(request: IncomingMessage): Promise<string> {
  let body = "";
  for await (const chunk of request) {
    body += chunk;
  }
  return body;
}

function encodeWebSocketText(value: string): Buffer {
  const payload = Buffer.from(value);
  if (payload.length < 126) {
    return Buffer.concat([Buffer.from([0x81, payload.length]), payload]);
  }
  return Buffer.concat([Buffer.from([0x81, 126, payload.length >> 8, payload.length & 0xff]), payload]);
}
