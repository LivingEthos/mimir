import { expect, test } from "@playwright/test";

test("renders a nonblank studio shell and accepts composer input", async ({ page }) => {
  await page.goto("/?mock=1");

  await expect(page.getByRole("banner")).toContainText("Mimir Studio");
  await expect(page.getByTestId("studio-shell")).toBeVisible();
  await expect(page.getByTestId("transcript")).toContainText("context");
  await expect(page.getByTestId("readiness-panel")).toContainText("First launch readiness");
  await expect(page.getByTestId("readiness-panel")).toContainText("Providers");

  const composer = page.getByTestId("composer-input");
  await composer.fill("/context add Studio smoke coverage");
  await expect(composer).toHaveValue("/context add Studio smoke coverage");

  await page.getByTestId("composer-send").click();
  await expect(page.getByTestId("transcript")).toContainText("add Studio smoke coverage");
  await expect(page.getByTestId("context-inspector")).toContainText("ctx-demo");
  await expect(page.getByTestId("context-inspector")).toContainText(".mimir/runs/run-demo/context_packet.json");
  await expect(page.getByTestId("context-inspector")).toContainText("new UI smoke coverage should track composer behavior");
  await expect(page.getByTestId("transcript")).toContainText("context completed");
  await expect(page.getByTestId("composer-send")).toHaveAttribute("aria-label", "Send");

  await composer.fill("/");
  await expect(page.getByTestId("command-palette")).toContainText("/doctor");
  await page.getByTestId("command-palette").getByRole("button", { name: /\/doctor/ }).click();
  await expect(composer).toHaveValue("/doctor");
  await expect(page.getByTestId("composer-send")).toBeEnabled();
  await page.getByTestId("composer-send").click();
  await expect(page.getByTestId("transcript")).toContainText("ok with 0 failures");
  await expect(page.getByTestId("transcript")).toContainText("doctor completed");
  await expect(page.getByTestId("composer-send")).toHaveAttribute("aria-label", "Send");

  await composer.fill("/help");
  await page.getByTestId("composer-send").click();
  await expect(page.getByTestId("transcript")).toContainText("commands available");
  await expect(page.getByTestId("transcript")).toContainText("/context <task>");
  await expect(page.getByTestId("command-palette")).not.toBeVisible();
  await expect(page.getByTestId("composer-send")).toHaveAttribute("aria-label", "Send");

  await composer.fill("/");
  await expect(page.getByTestId("command-palette")).toContainText("/init");
  await expect(page.getByTestId("command-palette")).toContainText("/plan");

  await composer.fill("/plan wire provider-backed planning");
  await page.getByTestId("composer-send").click();
  await expect(page.locator(".error-strip")).toContainText("planned but not connected");

  await composer.fill("/sh");
  await expect(page.getByTestId("command-palette")).toContainText("/share");
  await composer.fill("/share run-demo");
  await page.getByTestId("composer-send").click();
  await expect(page.getByTestId("packet-share-preview")).toContainText("mimir.packet_share");
  await expect(page.getByTestId("packet-share-preview")).toContainText("share bundle");
  await expect(page.locator(".error-strip")).not.toBeVisible();

  await composer.fill("/unknown");
  await page.getByTestId("composer-send").click();
  await expect(page.locator(".error-strip")).toContainText("not a recognized Mimir Studio command");

  await composer.fill("/resume ");
  await expect(page.getByTestId("resume-palette")).toContainText("Artifact review");
  await expect(page.getByTestId("resume-option-sess-mock-artifacts")).toHaveClass(/active/);
  await composer.press("Enter");
  await expect(page.getByTestId("session-row-sess-mock-artifacts")).toHaveClass(/active/);
  await expect(page.getByTestId("transcript")).toContainText("Artifact review ready");

  await composer.fill("/resume Studio");
  await page.getByTestId("composer-send").click();
  await expect(page.getByTestId("session-row-sess-mock-studio")).toHaveClass(/active/);
  await expect(page.getByTestId("transcript")).toContainText("Workspace status loaded");

  await composer.fill("/settings");
  await page.getByTestId("composer-send").click();
  await expect(page.getByTestId("settings-view")).toBeVisible();
  await page.locator('select[aria-label="Provider"]').selectOption("anthropic");
  await expect(page.getByRole("banner")).toContainText("anthropic");
  await page.locator('input[aria-label="Model"]').fill("m2.7");
  await page.getByTestId("settings-view").getByRole("button", { name: "Session" }).click();
  await page.locator('input[aria-label="Cost cap"]').fill("3.5");
  await page.getByRole("button", { name: "Light" }).click();
  await expect(page.getByTestId("studio-shell")).toHaveAttribute("data-theme", "light");
});
