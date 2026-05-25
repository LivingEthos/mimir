# Providers

Mimir supports AI model providers through a unified gateway interface. Provider keys are environment-only: pass them via shell environment variables and do not write them into Mimir config, prompts, or artifacts.

## GLM / ZAI / OpenAI-Compatible

`mimir plan` and `mimir code` can call OpenAI-compatible chat-completions endpoints. The CLI defaults to `--provider glm`; use `--provider openai-compatible` for a generic compatible endpoint. `mimir providers doctor` validates bundled and configured local provider capability YAML, checks gateway invariants, and lists registry-backed models.

```bash
export GLM_API_KEY=...
export GLM_BASE_URL=https://api.z.ai/api/coding/paas/v4
mimir plan --provider glm --model glm-5.1 --editable src/lib.rs "Plan the change"

export OPENAI_API_KEY=...
export OPENAI_BASE_URL=https://api.example.com/v1
mimir code --provider openai-compatible --model my-model --editable src/lib.rs --dry-run "Implement the change"
```

GLM/ZAI credentials and endpoint configuration are intentionally separate from generic OpenAI-compatible credentials: `--provider glm` reads only `GLM_API_KEY` or `ZAI_API_KEY` plus optional `GLM_BASE_URL`/`GLM_MODEL`, while `--provider openai-compatible` reads `OPENAI_API_KEY`, `OPENAI_BASE_URL`, and `OPENAI_MODEL`.

Bundled provider YAML is the canonical source for registry-backed providers. Capability validation reserves headroom for input tokens, output reserve, and Mimir's local-count drift reserve before a model can be loaded. Registry-backed snapshots are used before generated dynamic capabilities; GLM/ZAI and generic OpenAI-compatible models use conservative generated capabilities only when no bundled or configured local YAML exists for that provider.

Local OpenAI-compatible provider capabilities may be loaded from a YAML file path, but credentials remain environment-only and must never be placed in the YAML. Point Mimir at the local capability file with `MIMIR_PROVIDER_CAPABILITIES_PATH=/path/to/provider.yaml`, or add a path-only entry to `.mimir/config.yaml`:

```yaml
provider_capabilities_path: provider-capabilities.yaml
```

The local YAML must be only a single `ProviderCapabilities` document: `schema_version`, `provider`, and `models`. Do not wrap it in a `providers` array; that plural shape is only for provider-list responses. Unknown fields, malformed YAML, duplicate providers, and invalid token/pricing invariants fail closed instead of falling back to generated capabilities. Provider-list APIs such as `workspace/providers` and LSP `workspace/executeCommand` with `workspace/providers` return the plural `ProviderCapabilitiesList` wrapper with top-level `schema_version` and `providers`.

For a local OpenAI-compatible provider, keep the YAML capability-only and pass credentials through `OPENAI_API_KEY`:

```bash
export MIMIR_PROVIDER_CAPABILITIES_PATH=/path/to/local-provider.yaml
export OPENAI_API_KEY=...
mimir plan --provider local-openai --model local-model --base-url http://127.0.0.1:8080/v1 --editable src/lib.rs "Plan the change"
```

`mimir plan` writes a schema-backed `PlanArtifact` implementation-plan artifact (`plan.json`). This is intentionally separate from code-mode `PatchPlan` metadata. `mimir code` requires at least one `--editable` path. Code-mode providers return a `patch_plan` metadata object plus a `patch_recipe` executable object. Persisted `patch_recipe.json` is strict: it always includes `packet_id` and a non-empty `steps` array. The CLI still accepts legacy provider payloads without `packet_id`, binds them to the current packet, and only then writes artifacts. Mimir validates packet binding, editable targets, diff headers, secret-like content, and a dry-run preflight before applying. Actual apply uses a transaction that preflights paths, backs up existing files, and rolls back earlier steps when a later step fails. Provider-suggested test commands are recorded but not executed automatically; safe local Cargo/Pytest tests may auto-run, while JavaScript frameworks that would use `npx` and Cargo/Pytest projects with local build/test hooks are skipped. Auto-run test subprocesses receive a sanitized environment: provider credentials and generic key/token/secret/password/credential variables are stripped before tests execute.

Run artifacts are written under `.mimir/runs/<run-id>/`, including `context_packet.json`, `provider_request.redacted.json`, `response.json`, `patch_plan.json`, `patch_recipe.json`, `patch.diff`, `test_result*.json`, and `patch_report.json`. Repair turns, when needed, write `repair_request.redacted.turn-N.json`, `repair_response.turn-N.json`, `repair_patch_plan.turn-N.json`, `repair_patch_recipe.turn-N.json`, and `repair_patch.turn-N.diff`; a standalone `repair_summary.json` is also written when a repair loop runs.

Detected test failures fail closed after artifacts are written. The current production policy keeps the failed patch in the worktree for user inspection instead of rolling it back after tests fail. Repair turns may add follow-up patches; if repair exhausts its turn/cost budget or is rejected, the final worktree reflects the last safely applied patch and `patch_report.json` records `rejected` with the test/repair reason.

## Anthropic (Primary)

### Supported Models
| Model | Context | Output | Server Count | Prompt Cache | CLI Streaming |
|-------|---------|--------|--------------|--------------|-----------|
| claude-sonnet-4-6 | 1M | 64K | Yes | Yes | Not yet wired |
| claude-sonnet-4-20250514 | 1M | 64K | Yes | Yes | Not yet wired |
| claude-haiku-4-5 | 200K | 64K | Yes | Yes | Not yet wired |

### Authentication
Set `ANTHROPIC_API_KEY` environment variable.

### Endpoints
- POST `/v1/messages` — chat completions
- POST `/v1/messages/count_tokens` — token counting

### Capabilities
- Server-side token counting (reliable)
- Prompt caching (ephemeral)
- SSE streaming exists in lower-level provider code, but CLI `--stream` currently fails closed until the dispatch path is wired.
- Tool use

### Error Mapping
| Anthropic Error | Mimir Code | Retryable |
|-----------------|------------|-----------|
| invalid_request_error | provider_invalid_request | No |
| authentication_error | provider_unauthorized | No |
| permission_error | provider_forbidden | No |
| not_found_error | provider_not_found | No |
| request_too_large | provider_request_too_large | No |
| rate_limit_error | provider_rate_limited | Yes |
| overloaded_error | provider_overloaded | Yes |
| api_error | provider_internal_error | Yes |

## Adding a Provider

1. Create adapter in `crates/mimir-providers/src/adapters/`
2. Implement `ProviderAdapter` metadata/counting only; provider generation dispatch must stay gateway-owned
3. Add capabilities to `ProviderCapabilities` schema or a local single-provider YAML
4. Add error mapping
5. Write adapter contract tests

## Adapter Contract

Every provider adapter must:
- Support `count_tokens` (local or server)
- Return structured `ProviderResponse`
- Report token usage and cache-related metadata when supported
- Redact secrets from all logged output
- Dispatch only through `ProviderGateway`; external crates must not be able to call adapter generation directly
- Respect gateway cap and capability-snapshot enforcement
