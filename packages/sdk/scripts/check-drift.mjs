#!/usr/bin/env node
import { readFileSync } from "node:fs";
import { spawnSync } from "node:child_process";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const repoRoot = resolve(root, "../..");
const failures = [];

const readJson = (path) => JSON.parse(readFileSync(path, "utf8"));
const readText = (path) => readFileSync(path, "utf8");
const assert = (condition, message) => {
  if (!condition) failures.push(message);
};

const generateCheck = spawnSync(process.execPath, ["scripts/generate.mjs", "--check"], {
  cwd: root,
  encoding: "utf8",
});
if (generateCheck.status !== 0) {
  failures.push(
    [
      "generated SDK schema types are stale",
      generateCheck.stdout.trim(),
      generateCheck.stderr.trim(),
    ]
      .filter(Boolean)
      .join("\n"),
  );
}

const checks = [
  ["ContextPacket", ["schema_version", "packet_id", "packet_hash", "run_id", "task_card", "mode", "included", "omitted_candidates", "budget_ledger_ref", "count_provenance"]],
  ["ProviderCapabilities", ["schema_version", "provider", "models"]],
  ["ProviderCapabilitiesList", ["schema_version", "providers"]],
  ["ExecutablePatchPlan", ["schema_version", "plan_id", "packet_id", "steps"]],
  ["PlanArtifact", ["schema_version", "run_id", "packet_id", "packet_hash", "provider", "model", "task", "steps"]],
];

for (const [name, required] of checks) {
  const schema = readJson(join(repoRoot, "schemas", `${name}.schema.json`));
  const ts = readText(join(root, `${name}.ts`));
  assert(schema.title === name, `${name}: schema title mismatch`);
  for (const field of required) {
    assert(schema.required?.includes(field), `${name}: schema no longer requires ${field}`);
    assert(ts.includes(`${field}:`) || ts.includes(`${field}?:`), `${name}.ts missing ${field}`);
  }
}

const context = readText(join(root, "ContextPacket.ts"));
assert(context.includes("task_card: {"), "ContextPacket.ts must keep schema-shaped task_card object");
assert(context.includes('complexity: "tiny" | "standard" | "complex" | "override"'), "ContextPacket.ts missing task_card complexity enum");
assert(context.includes("prompt_contract_version: number"), "ContextPacket.ts must expose numeric prompt_contract_version");
assert(context.includes('"local_estimate_only"'), "ContextPacket.ts missing local_estimate_only count provenance");

const provider = readText(join(root, "ProviderCapabilities.ts"));
const pricingBlock = provider.match(/pricing:\s*\{[\s\S]*?\n\s*\};/u)?.[0] ?? "";
assert(!pricingBlock.includes("[k: string]"), "ProviderCapabilities.ts pricing block allows unknown fields");

const executable = readText(join(root, "ExecutablePatchPlan.ts"));
assert(
  /steps:\s*\[[A-Za-z0-9_]+,\s*\.\.\.[A-Za-z0-9_]+\[\]\]/u.test(executable),
  "ExecutablePatchPlan.ts must model non-empty steps",
);

if (failures.length > 0) {
  console.error(failures.map((failure) => `- ${failure}`).join("\n"));
  process.exit(1);
}

console.log("SDK schema drift checks passed.");
