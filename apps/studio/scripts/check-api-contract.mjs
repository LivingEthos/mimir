#!/usr/bin/env node
import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const repoRoot = resolve(root, "../..");
const failures = [];

const readText = (path) => readFileSync(path, "utf8");
const assert = (condition, message) => {
  if (!condition) {
    failures.push(message);
  }
};

const tsTypes = readText(resolve(root, "src/api/types.ts"));
const apiClient = readText(resolve(root, "src/api/client.ts"));
const appSource = readText(resolve(root, "src/App.tsx"));
const sessionRust = readText(resolve(repoRoot, "crates/mimir-session/src/lib.rs"));
const uiRust = readText(resolve(repoRoot, "crates/mimir-server/src/ui.rs"));

function blockAfter(source, marker) {
  const start = source.indexOf(marker);
  if (start < 0) {
    failures.push(`missing block marker ${marker}`);
    return "";
  }
  const open = source.indexOf("{", start);
  if (open < 0) {
    failures.push(`missing opening brace for ${marker}`);
    return "";
  }

  let depth = 0;
  let inString = false;
  let escaped = false;
  for (let index = open; index < source.length; index += 1) {
    const char = source[index];
    if (inString) {
      escaped = char === "\\" && !escaped;
      if (char === "\"" && !escaped) {
        inString = false;
      }
      if (char !== "\\") {
        escaped = false;
      }
      continue;
    }
    if (char === "\"") {
      inString = true;
      continue;
    }
    if (char === "{") {
      depth += 1;
    } else if (char === "}") {
      depth -= 1;
      if (depth === 0) {
        return source.slice(open + 1, index);
      }
    }
  }

  failures.push(`missing closing brace for ${marker}`);
  return "";
}

function literalsFromTsConst(name) {
  const match = tsTypes.match(new RegExp(String.raw`export const ${name} = \[([\s\S]*?)\] as const;`, "u"));
  if (!match) {
    failures.push(`missing TS const ${name}`);
    return [];
  }
  return Array.from(match[1].matchAll(/"([^"]+)"/gu), (literal) => literal[1]).sort();
}

function quotedFieldsFromTsInterface(name) {
  return Array.from(
    blockAfter(tsTypes, `export interface ${name}`).matchAll(/^\s*"([^"]+)"\s*:/gmu),
    (field) => field[1],
  ).sort();
}

function fieldsFromTsInterface(name) {
  return Array.from(
    blockAfter(tsTypes, `export interface ${name}`).matchAll(
      /^\s*(?:"([^"]+)"|([A-Za-z_][A-Za-z0-9_]*))\??\s*:/gmu,
    ),
    (field) => field[1] ?? field[2],
  ).sort();
}

function optionalFieldsFromTsInterface(name) {
  return Array.from(
    blockAfter(tsTypes, `export interface ${name}`).matchAll(
      /^\s*(?:"([^"]+)"|([A-Za-z_][A-Za-z0-9_]*))\?\s*:/gmu,
    ),
    (field) => field[1] ?? field[2],
  ).sort();
}

function entriesFromTsInterface(name) {
  return Object.fromEntries(
    Array.from(
      blockAfter(tsTypes, `export interface ${name}`).matchAll(
        /^\s*"([^"]+)"\s*:\s*([A-Za-z_][A-Za-z0-9_]*)\s*;/gmu,
      ),
      (field) => [field[1], field[2]],
    ),
  );
}

function quotedKeysFromObject(source, marker) {
  return Array.from(
    blockAfter(source, marker).matchAll(/^\s*"([^"]+)"\s*:/gmu),
    (field) => field[1],
  ).sort();
}

function entriesFromObject(source, marker) {
  return Object.fromEntries(
    Array.from(
      blockAfter(source, marker).matchAll(/^\s*"([^"]+)"\s*:\s*([A-Za-z_][A-Za-z0-9_]*)/gmu),
      (field) => [field[1], field[2]],
    ),
  );
}

function blockFromOpeningBrace(source, open, marker) {
  let depth = 0;
  let inString = false;
  let escaped = false;
  for (let index = open; index < source.length; index += 1) {
    const char = source[index];
    if (inString) {
      escaped = char === "\\" && !escaped;
      if (char === "\"" && !escaped) {
        inString = false;
      }
      if (char !== "\\") {
        escaped = false;
      }
      continue;
    }
    if (char === "\"") {
      inString = true;
      continue;
    }
    if (char === "{") {
      depth += 1;
    } else if (char === "}") {
      depth -= 1;
      if (depth === 0) {
        return source.slice(open + 1, index);
      }
    }
  }

  failures.push(`missing closing brace for ${marker}`);
  return "";
}

function serdeRenamesFromRustEnum(source, enumName) {
  return Array.from(
    blockAfter(source, `pub enum ${enumName}`).matchAll(/#\[serde\(rename = "([^"]+)"\)\]/gu),
    (rename) => rename[1],
  ).sort();
}

function snakeCase(value) {
  return value.replace(/([a-z0-9])([A-Z])/gu, "$1_$2").toLowerCase();
}

function snakeVariantsFromRustEnum(source, enumName) {
  const body = blockAfter(source, `enum ${enumName}`);
  return Array.from(body.matchAll(/^\s*([A-Z][A-Za-z0-9]*)\s*,/gmu), (variant) =>
    snakeCase(variant[1]),
  ).sort();
}

function fieldsFromRustStruct(source, structName) {
  return Array.from(
    blockAfter(source, `pub struct ${structName}`).matchAll(/^\s*pub\s+([a-z][A-Za-z0-9_]*)\s*:/gmu),
    (field) => field[1],
  ).sort();
}

function sessionEventPayloadFieldsFromRust(source) {
  const body = blockAfter(source, "pub enum SessionEventKind");
  const entries = [];
  const variants = body.matchAll(
    /#\[serde\(rename = "([^"]+)"\)\]\s*([A-Z][A-Za-z0-9]*)\s*\{/gmu,
  );
  for (const variant of variants) {
    const open = body.indexOf("{", variant.index);
    const fields = Array.from(
      blockFromOpeningBrace(body, open, variant[1]).matchAll(/^\s*([a-z][A-Za-z0-9_]*)\s*:/gmu),
      (field) => field[1],
    ).sort();
    entries.push([variant[1], fields]);
  }
  return Object.fromEntries(entries);
}

function fieldsFromGuardFunction(name) {
  const body = blockAfter(apiClient, `function ${name}(`);
  const match = body.match(/hasOnlyKeys\(\s*value\s*,\s*\[([\s\S]*?)\]\s*\)/u);
  if (!match) {
    failures.push(`missing hasOnlyKeys field list in ${name}`);
    return [];
  }
  return Array.from(match[1].matchAll(/"([^"]+)"/gu), (field) => field[1]).sort();
}

function assertSameSet(label, left, right) {
  assert(
    JSON.stringify(left) === JSON.stringify(right),
    `${label} drifted\n  left:  ${left.join(", ")}\n  right: ${right.join(", ")}`,
  );
}

function assertFields(source, marker, fields) {
  const body = blockAfter(source, marker);
  for (const field of fields) {
    assert(new RegExp(`\\b${field}\\??:`, "u").test(body), `${marker} missing field ${field}`);
  }
  return body;
}

assertSameSet(
  "Session event type contract",
  serdeRenamesFromRustEnum(sessionRust, "SessionEventKind"),
  literalsFromTsConst("sessionEventTypes"),
);
assertSameSet(
  "Session event payload interface contract",
  literalsFromTsConst("sessionEventTypes"),
  quotedFieldsFromTsInterface("SessionEventPayloads"),
);
assertSameSet(
  "Session event payload guard contract",
  literalsFromTsConst("sessionEventTypes"),
  quotedKeysFromObject(apiClient, "const sessionEventPayloadGuards ="),
);
assertSameSet(
  "Command support contract",
  snakeVariantsFromRustEnum(sessionRust, "CommandSupport"),
  literalsFromTsConst("commandSupports"),
);
assertSameSet(
  "Trace status contract",
  snakeVariantsFromRustEnum(uiRust, "TraceStatusState"),
  literalsFromTsConst("traceStatusStates"),
);
assertSameSet(
  "Approval action contract",
  snakeVariantsFromRustEnum(sessionRust, "ApprovalAction"),
  literalsFromTsConst("approvalActions"),
);

const sessionEventTypesFromTs = literalsFromTsConst("sessionEventTypes");
const sessionEventPayloadInterfaces = entriesFromTsInterface("SessionEventPayloads");
const sessionEventPayloadGuardFunctions = entriesFromObject(apiClient, "const sessionEventPayloadGuards =");
const sessionEventRustPayloadFields = sessionEventPayloadFieldsFromRust(sessionRust);
const studioEventPayloadFieldOverrides = {
  "session.created": ["title", "workspace_name"],
};

for (const eventType of sessionEventTypesFromTs) {
  const interfaceName = sessionEventPayloadInterfaces[eventType];
  const guardName = sessionEventPayloadGuardFunctions[eventType];
  assert(Boolean(interfaceName), `${eventType} missing TS payload interface mapping`);
  assert(Boolean(guardName), `${eventType} missing runtime payload guard mapping`);
  if (!interfaceName || !guardName) {
    continue;
  }

  const tsFields = fieldsFromTsInterface(interfaceName);
  const guardFields = fieldsFromGuardFunction(guardName);
  const rustFields = studioEventPayloadFieldOverrides[eventType] ?? sessionEventRustPayloadFields[eventType];
  assert(Boolean(rustFields), `${eventType} missing Rust SessionEventKind payload`);
  assertSameSet(`${eventType} TS payload/runtime guard fields`, tsFields, guardFields);
  assertSameSet(`${eventType} Rust/Studio payload DTO fields`, rustFields ?? [], tsFields);
}

for (const [rustName, tsName, guardName] of [
  ["ArtifactRef", "ArtifactRef", "isArtifactRef"],
  ["ApprovalRequest", "ApprovalRequest", "isApprovalRequest"],
  ["ApprovalDecision", "ApprovalDecision", "isApprovalDecision"],
]) {
  const tsFields = fieldsFromTsInterface(tsName);
  assertSameSet(`${tsName} Rust/TS fields`, fieldsFromRustStruct(sessionRust, rustName), tsFields);
  assertSameSet(`${tsName} TS/runtime guard fields`, tsFields, fieldsFromGuardFunction(guardName));
  assertSameSet(`${tsName} API DTO optional fields`, [], optionalFieldsFromTsInterface(tsName));
}

const sessionMetadata = assertFields(tsTypes, "export interface SessionMetadata", [
  "schema_version",
  "session_id",
  "title",
  "workspace_name",
  "created_at",
  "updated_at",
]);
assert(!sessionMetadata.includes("workspace_root"), "SessionMetadata must not expose workspace_root");

assertFields(tsTypes, "export interface SessionEventBase", [
  "schema_version",
  "event_id",
  "session_id",
  "sequence",
  "timestamp",
  "type",
  "payload",
]);
for (const responseName of [
  "SessionCreateResponse",
  "SessionLoadResponse",
  "SessionMessageResponse",
]) {
  const response = blockAfter(tsTypes, `export interface ${responseName}`);
  assert(
    response.includes("events: ApiSessionEvent[];"),
    `${responseName} must expose API DTO events, not client-enriched events`,
  );
  assert(
    !response.includes("events: SessionEvent[];"),
    `${responseName} must keep client-local enrichment out of API DTOs`,
  );
}
assertFields(tsTypes, "export interface WorkspaceStatus {", [
  "workspace_name",
  "git",
  "mimir",
  "providers",
  "commands",
]);
assertFields(tsTypes, "export interface CommandMetadata", [
  "name",
  "usage",
  "summary",
  "support",
  "takes_input",
  "enabled",
  "disabled_reason",
]);
assertFields(tsTypes, "export interface RunSummary", [
  "run_id",
  "path",
  "artifact_count",
  "has_context_packet",
  "trace_status",
]);
const traceStatus = assertFields(tsTypes, "export interface TraceStatus", ["state", "redacted"]);
assert(!traceStatus.includes("path"), "TraceStatus must remain path-free");
assert(!traceStatus.includes("artifact_name"), "TraceStatus must remain artifact-name-free");
assertFields(tsTypes, "export interface ArtifactListResponse", [
  "run_id",
  "trace_status",
  "artifacts",
]);
assertFields(tsTypes, "export interface ArtifactSummary", [
  "name",
  "path",
  "size_bytes",
  "sha256",
  "checksum_basis",
  "redacted",
]);
assertFields(tsTypes, "export interface ReplayPreviewResponse", [
  "run_id",
  "packet_id",
  "packet_hash",
  "packet_path",
  "source",
  "provider_request_sha256",
  "redacted",
  "request",
]);
assertFields(tsTypes, "export interface SharePreviewResponse", [
  "run_id",
  "packet_id",
  "packet_hash",
  "packet_path",
  "bundle_sha256",
  "redacted",
  "bundle",
]);

assertFields(uiRust, "struct RunSummary", ["run_id", "path", "artifact_count", "has_context_packet", "trace_status"]);
assertFields(uiRust, "struct ArtifactListResponse", ["run_id", "trace_status", "artifacts"]);
const rustTraceStatus = assertFields(uiRust, "struct TraceStatus", ["state", "redacted"]);
assert(!rustTraceStatus.includes("path"), "Rust TraceStatus must remain path-free");
assert(
  uiRust.includes('const TRACE_STATUS_ARTIFACTS: &[&str] = &["trace.spans.jsonl", "trace.json"];'),
  "Rust trace status must prefer first-class trace span artifacts instead of events.jsonl fallback",
);

assert(
  !appSource.includes('artifacts.some((artifact) => artifact.name.toLowerCase().includes("trace"))'),
  "Studio trace state must use trace_status instead of artifact-name scanning",
);

if (failures.length > 0) {
  console.error(failures.map((failure) => `- ${failure}`).join("\n"));
  process.exit(1);
}

console.log("Studio API contract checks passed.");
