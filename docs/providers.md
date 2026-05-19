# Providers

Mimir supports AI model providers through a unified gateway interface.

## Anthropic (Primary)

### Supported Models
| Model | Context | Output | Server Count | Prompt Cache | Streaming |
|-------|---------|--------|--------------|--------------|-----------|
| claude-sonnet-4-6 | 1M | 8K | Yes | Yes | Yes |
| claude-haiku-4-5 | 200K | 8K | Yes | Yes | Yes |

### Authentication
Set `ANTHROPIC_API_KEY` environment variable.

### Endpoints
- POST `/v1/messages` — chat completions
- POST `/v1/messages/count_tokens` — token counting

### Capabilities
- Server-side token counting (reliable)
- Prompt caching (ephemeral)
- SSE streaming
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
2. Implement `ProviderAdapter` trait
3. Add capabilities to `ProviderCapabilities` schema
4. Add error mapping
5. Write adapter contract tests

## Adapter Contract

Every provider adapter must:
- Support `count_tokens` (local or server)
- Return structured `ProviderResponse`
- Populate `CacheStatus` if caching is supported
- Redact secrets from all logged output
- Respect the gateway cap enforcement
