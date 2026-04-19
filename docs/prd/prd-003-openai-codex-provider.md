# PRD-003: OpenAI Codex Subscription Provider

- **Status:** Implemented
- **Version:** v0.3.0
- **Scope:** `themion-core` (new `client_codex` module, `CodexAuth` data type, agent client abstraction); `themion-cli` (auth store IO, login flow in TUI, config provider arm); workspace `Cargo.toml`; docs
- **Author:** Tasanakorn (design) + Claude Code (PRD authoring)
- **Date:** 2026-04-18

> **Implementation note:** The landed implementation follows the overall provider/auth/client design in this PRD, but the login path currently centers on the device flow implemented in `crates/themion-cli/src/login_codex.rs` and invoked from `tui.rs`. Treat the code and current docs as the source of truth where they differ from the originally proposed local-listen-first flow.

## 1. Goals

- Let users authenticate against their existing ChatGPT / OpenAI Codex subscription via a `/login codex` command in the TUI; no API key, no separate billing.
- Drive completions through the OpenAI Responses API at `https://chatgpt.com/backend-api/codex/responses` using the OAuth access token issued during login.
- Persist `{ access, refresh, expires, account_id }` to `~/.config/themion/auth.json` so subsequent process launches reuse the session without re-authenticating.
- Refresh the access token transparently before any request whose token has expired (or is about to), and rewrite `auth.json` with the fresh values.
- Default the provider's model alias `gpt-5.4` to the upstream model id `gpt-5.4`, while leaving the alias overridable in config.

## 2. Non-goals

- No support for the standard OpenAI platform API key flow (`api.openai.com/v1/chat/completions`). The Responses-API endpoint and the subscription auth path are the only OpenAI integration this PRD covers.
- No WebSocket transport. The Responses API supports a websocket variant; this PRD is SSE-only, matching the existing `chat_completion_stream` model.
- No interactive prompt during the Device Code polling loop; the TUI shows a single entry with the `verification_uri` and `user_code` and blocks `agent_busy` until the poll resolves or times out.
- No central credential rotation, multi-account switching, or per-profile auth files. One global `auth.json` shared across all profiles that name `provider = "openai-codex"`.
- No migration of existing OpenRouter or llamacpp profiles; both continue working unchanged.

## 3. Background & Motivation

themion currently speaks one wire format: OpenAI Chat Completions, served by OpenRouter (paid API key) or a local llamacpp instance (no auth). Users with an existing ChatGPT subscription have already paid for Codex-class capacity but cannot use it from themion — there is no API-key path to the subscription endpoint, and the Responses API exposed at `chatgpt.com/backend-api/codex/responses` requires an OAuth bearer minted via the same PKCE flow the official Codex CLI uses.

Adding a Codex subscription provider lets these users point themion at the model they're already paying for, with a one-time browser login. It also forces themion's client layer to grow a second backend, which has been deferred since the project began — the `OpenRouterClient` type alias was always a placeholder.

### 3.1 Current state

- `crates/themion-core/src/client.rs` defines `ChatClient` (alias `OpenRouterClient`) which speaks Chat Completions only. `chat_completion_stream` posts to `{base_url}/chat/completions` with `messages`, parses `data: {"choices":[{"delta":…}]}` SSE frames, and returns `(ResponseMessage, Option<Usage>)`. The streaming types `StreamChunkData`, `StreamChoice`, `StreamDelta`, `StreamToolCallDelta`, `StreamFunctionDelta` are all bound to the Chat Completions wire shape.
- `crates/themion-core/src/agent.rs` holds `client: OpenRouterClient` as a concrete type. The sole call site in `run_loop` (lines 209–220) calls `self.client.chat_completion_stream(model, messages, tools, on_chunk)` and consumes the returned `(ResponseMessage, Usage)` tuple. There is no trait, no enum, no `dyn`.
- `crates/themion-cli/src/config.rs` `resolve_profile` matches on `provider.as_str()` with arms for `"llamacpp"` and a fallback that defaults to `openrouter`. The `api_key`-required guard at lines 157–167 only fires for `provider == "openrouter"`.
- `crates/themion-cli/src/tui.rs` `handle_command` is a synchronous `fn(&mut self, &str) -> Vec<String>`; it cannot perform `.await` calls. Async work uses the existing `AppEvent` pattern: spawn a `tokio::task`, send `AppEvent` variants back through `app_tx`. The agent run path in `submit_input` is the canonical example. `agent_busy` already serializes one async op at a time.
- `~/.config/themion/auth.json` does not exist, has no loader, no struct, no writer.
- Workspace deps already present: tokio, reqwest, serde, serde_json, anyhow, toml, rusqlite, uuid, ratatui, crossterm, tui-textarea, tokio-stream, dirs (cli-only). No SHA-2, base64, JWT, or browser-launcher crates.

## 4. Design

### 4.1 Client abstraction

`themion-core` introduces a `ChatBackend` trait with a single async method matching today's `chat_completion_stream` signature, and `Agent.client` becomes `Box<dyn ChatBackend + Send + Sync>`:

```rust
#[async_trait::async_trait]
pub trait ChatBackend: Send + Sync {
    async fn chat_completion_stream(
        &self,
        model: &str,
        messages: &[Message],
        tools: &Value,
        on_chunk: Box<dyn FnMut(String) + Send + 'static>,
    ) -> Result<(ResponseMessage, Option<Usage>)>;
}
```

`ChatClient` and the new `CodexClient` both `impl ChatBackend`. The wider `(ResponseMessage, Option<Usage>)` return tuple is preserved verbatim — `Agent::run_loop` already unpacks it that way. The `on_chunk` parameter is widened from `impl FnMut(String)` to `Box<dyn FnMut(String) + Send + 'static>` because `async_trait` rewrites methods into futures whose lifetimes don't admit non-`'static` borrowed closures cleanly. The `'static` bound is the explicit consequence: the existing call site in `agent.rs` (lines 214–218) must `Box::new` an owned closure, which it already does in spirit (it captures `event_tx.clone()` by move).

`async_trait = "0.1"` is added as a workspace dependency. Dynamic dispatch is acceptable here: there is exactly one `chat_completion_stream` call per LLM round, dwarfed by the network round-trip cost.

**Alternative considered:** an enum `ClientEnum::OpenAi(ChatClient) | Codex(CodexClient)` with a hand-written dispatch method. Rejected: every new backend (gemini, anthropic, ollama) would have to edit the enum and the dispatch arm; a trait keeps `themion-core` open to extension without core edits.

**Alternative considered:** make `Agent` generic over `B: ChatBackend`. Rejected: `Agent` is owned by `App.agents: Vec<AgentHandle>` in the TUI, and a generic field would force the TUI to either pick one backend at compile time or maintain its own enum. The trait-object cost is one v-table call per LLM round.

### 4.2 Auth data type and `auth.json`

The `CodexAuth` struct lives in `themion-core` (new module `crates/themion-core/src/auth.rs`) so that both the client (which needs to pass refreshed tokens to a save callback) and the CLI (which owns the disk path) speak the same type:

```rust
// in themion-core
#[derive(Serialize, Deserialize, Clone)]
pub struct CodexAuth {
    pub access_token: String,
    pub refresh_token: String,
    pub expires_at: i64,         // unix epoch seconds
    pub account_id: String,      // chatgpt_account_id JWT claim
}

impl CodexAuth {
    pub fn is_expired(&self, skew_secs: i64) -> bool;  // pure logic, no IO
}
```

The CLI module `crates/themion-cli/src/auth_store.rs` owns IO only:

```rust
// in themion-cli
pub fn auth_path() -> Option<PathBuf>;                  // dirs::config_dir().join("themion/auth.json")
pub fn load() -> Result<Option<CodexAuth>>;
pub fn save(auth: &CodexAuth) -> Result<()>;            // atomic write + chmod 0600 on Unix
```

`save` writes atomically (`tempfile` + `rename`) and chmods to `0600` on Unix. `load` returns `Ok(None)` when the file is absent, `Err` only on corrupt JSON or IO failure. The lifetime is "global per host" — one file shared by every themion profile that uses Codex.

Splitting the type from the IO is the smallest change that makes the §4.4 callback `Fn(&CodexAuth) -> Result<()>` resolvable from the CLI side: the CLI passes `|a| auth_store::save(a)`.

**Alternative considered:** stash the tokens inside `config.toml` next to `api_key`. Rejected: the access token is short-lived and rewritten on every refresh; mixing it with hand-edited config invites diff churn and accidental commits to dotfile repos. A separate file with stricter permissions is cleaner.

**Alternative considered:** OS keychain (`keyring` crate). Rejected: pulls a platform-specific dependency surface (libsecret on Linux, Keychain on macOS, Credential Manager on Windows) for marginal benefit on a developer-tools CLI; a `0600` JSON file matches what the official Codex CLI does and what users already trust for `~/.aws/credentials`, `~/.npmrc`, etc.

**Alternative considered:** keep `CodexAuth` in `themion-cli` only and pass individual `(access, refresh, expires_at, account_id)` strings into the save callback. Rejected: every future field added to the auth payload would require a callback-signature change; a shared struct localized to `themion-core` is more durable.

### 4.3 Config: new provider arm

`ProfileConfig` is unchanged. `resolve_profile` gains a new arm:

```rust
"openai-codex" => {
    let base_url = profile.base_url.clone().filter(|s| !s.is_empty())
        .unwrap_or_else(|| CODEX_DEFAULT_BASE_URL.to_string());
    let model = std::env::var("CODEX_MODEL").ok().filter(|s| !s.is_empty())
        .or_else(|| profile.model.clone().filter(|s| !s.is_empty()))
        .unwrap_or_else(|| CODEX_DEFAULT_MODEL.to_string());
    (base_url, None, model)  // api_key always None for codex
}
```

Constants:

- `CODEX_DEFAULT_BASE_URL = "https://chatgpt.com/backend-api/codex"`
- `CODEX_DEFAULT_MODEL    = "gpt-5.4"`

The `api_key`-required guard at `config.rs:157–167` is widened to skip when `provider == "openai-codex"`. No `api_key` env var is honored for this provider — the credential lives entirely in `auth.json`.

The auto-generated `CONFIG_TEMPLATE` gains a commented example:

```toml
# [profile.codex]
# provider = "openai-codex"
# # model defaults to "gpt-5.4" → gpt-5.4
# # log in once with: /login codex
```

Model alias mapping (`gpt-5.4` → `gpt-5.4`) lives inside `CodexClient::resolve_model_alias`, not in config — the alias is a property of the upstream API and would otherwise force every user to know the underlying SKU.

### 4.4 `CodexClient`

A new module `crates/themion-core/src/client_codex.rs`:

```rust
pub struct CodexClient {
    http: reqwest::Client,
    base_url: String,                          // default https://chatgpt.com/backend-api/codex
    auth: Arc<tokio::sync::RwLock<CodexAuth>>,
    auth_writer: Box<dyn Fn(&CodexAuth) -> Result<()> + Send + Sync>,
}
```

`CodexClient::new(base_url, initial_auth, on_save)` takes the loaded `CodexAuth`, the persistence callback (the CLI passes a closure that calls `auth_store::save`), and the base URL. `themion-core` stays disk-agnostic; the closure is the seam.

`impl ChatBackend for CodexClient::chat_completion_stream`:

1. `ensure_fresh_token().await` — acquires read lock, checks `auth.is_expired(60)`, drops the read lock; on stale, takes write lock, re-checks (double-check), POSTs `application/x-www-form-urlencoded` body `grant_type=refresh_token&refresh_token=…&client_id=…` to `https://auth.openai.com/oauth/token`, replaces the inner state, calls `auth_writer` to persist.
2. Resolve the model alias: `model_id = resolve_model_alias(model)` (single-arm match: `"gpt-5.4" => "gpt-5.4"`, otherwise pass-through).
3. Translate `&[Message]` → Responses-API `input` array (see §4.5).
4. POST to `{base_url}/responses` with body `{"model": model_id, "instructions": system_prompt, "input": [...], "tools": [...], "stream": true}`. Required headers: `Authorization: Bearer {access}`, `chatgpt-account-id: {account_id}`, `originator: pi`, `OpenAI-Beta: responses=experimental`, `Content-Type: application/json`, `Accept: text/event-stream`.
5. Parse SSE frames in the named-event format (see §4.6), assemble a `ResponseMessage` and `Option<Usage>` from the `response.completed` payload, return.

System-prompt handling: `Agent` currently prepends a `role="system"` `Message` to the slice it sends. `CodexClient` extracts that first system message into the Responses-API `instructions` field and drops it from `input`; subsequent system-role messages (the windowing hint from PRD-002) are translated as `{"type": "message", "role": "developer", "content": [...]}` items inside `input`, since `instructions` accepts only one string.

**Constants verified against `badlogic/pi-mono` source** (`packages/ai/src/providers/openai-codex-responses.ts` and `packages/ai/src/utils/oauth/openai-codex.ts`): `client_id = app_EMoamEEZ73f0CkXaXp7hrann`, `originator: pi`, `OpenAI-Beta: responses=experimental` (SSE path), `chatgpt-account-id` (lowercase key), JWT namespace `https://api.openai.com/auth` → field `chatgpt_account_id`. No further cross-check required.

**Alternative considered:** keep the Chat Completions wire shape and translate at the HTTP boundary inside `chatgpt.com/backend-api`. Rejected: that endpoint does not accept Chat Completions input; the `/responses` path is the only one the subscription auth permits.

### 4.5 Message translation (Chat Completions → Responses API)

| Chat Completions `Message`                              | Responses API `input` item                                                                                                                                                                          |
| ------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `role="system"` (first only)                            | hoisted into top-level `instructions: String`                                                                                                                                                       |
| `role="system"` (subsequent)                            | `{"type": "message", "role": "developer", "content": [{"type": "input_text", "text": …}]}`                                                                                                          |
| `role="user", content=Some(s)`                          | `{"type": "message", "role": "user", "content": [{"type": "input_text", "text": s}]}`                                                                                                               |
| `role="assistant", content=Some(s), tool_calls=None`    | `{"type": "message", "role": "assistant", "content": [{"type": "output_text", "text": s}]}`                                                                                                         |
| `role="assistant", tool_calls=Some(calls)`              | one `{"type": "function_call", "call_id": tc.id, "name": tc.function.name, "arguments": tc.function.arguments}` item per `ToolCall`; preceding `output_text` item only if `content` is non-empty   |
| `role="tool", content=Some(out), tool_call_id=Some(id)` | `{"type": "function_call_output", "call_id": id, "output": out}`                                                                                                                                    |

Tools in the request body are translated from the existing OpenAI function-tool schema (`{type: "function", function: {name, description, parameters}}`) to the Responses-API flat shape (`{type: "function", name, description, parameters}`) — this is a flatten of the inner `function` object; no other rewrite. `tool_definitions()` in `tools.rs` is unchanged; the translation happens inside `CodexClient`.

### 4.6 SSE parsing

Responses-API SSE uses **named events** (`event: response.output_text.delta\ndata: {…}\n\n`) rather than the unnamed `data:` frames Chat Completions emits. `CodexClient` parses both `event:` and `data:` fields per frame. Events handled:

| Event name                                  | Action                                                                                                                                                                            |
| ------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `response.output_text.delta`                | `data.delta` is appended to `content` accumulator and forwarded to `on_chunk`                                                                                                     |
| `response.function_call_arguments.delta`    | `data.delta` is appended to the in-progress tool call's `arguments` buffer keyed by `data.item_id`                                                                                |
| `response.output_item.added`                | when `data.item.type == "function_call"`, allocate a new tool-call slot indexed by `data.item.id` with `name = data.item.name`, `id = data.item.call_id`                          |
| `response.completed`                        | extract `usage` (`input_tokens`, `output_tokens`, `input_tokens_details.cached_tokens`) into `Usage`; mark stream done                                                            |
| `response.failed` / `error` / `[DONE]`      | terminate the loop; `response.failed` raises `anyhow::bail!` with the included error message                                                                                      |
| any other event                             | ignored (e.g. `response.created`, `response.output_text.done`)                                                                                                                    |

Parser scaffolding mirrors `chat_completion_stream`: drain `\n`-terminated lines from a byte buffer, accumulate `event:` and `data:` fields until a blank line dispatches the assembled frame. The new types live in `client_codex.rs` (`ResponsesEvent`, `ResponsesUsage`, `ResponsesItem`, etc.); none are shared with the Chat Completions parser.

### 4.7 OAuth login flow

A new module `crates/themion-cli/src/login_codex.rs` exposes the device-flow implementation used by the TUI today:

```rust
pub async fn start_device_flow() -> Result<(
    DeviceCodeInfo,
    Pin<Box<dyn Future<Output = Result<CodexAuth>> + Send + 'static>>,
)>;
```

The current implementation obtains a device code, prompts the user to open the verification URI and enter the code, then polls until authorization completes and exchanges the resulting authorization code for tokens.

**Alternative considered:** a local-listen PKCE flow with automatic fallback to device flow. Rejected for the current implementation: the simpler device-flow-first path landed and is the current source of truth in code.

### 4.8 TUI `/login codex` command

`handle_command` gains an arm for `/login codex`. Because the function is sync, it cannot run the OAuth flow inline; it instead:

1. Sets `self.agent_busy = true` and pushes an `Entry::Assistant("logging in to OpenAI Codex…")` line.
2. Spawns a `tokio::task` that starts the device flow, emits a prompt event carrying `verification_uri` and `user_code`, then dispatches `AppEvent::LoginComplete(Result<CodexAuth>)` back into the main loop when polling finishes.
3. Returns `vec![]` from `handle_command` so the user sees only the placeholder line; the result line is appended later when the event arrives.

The `agent_busy` guard prevents a user from triggering a second login or sending a chat message while the OAuth flow is in flight.

### 4.9 Wiring the Codex client into `build_agent`

`build_agent` in `tui.rs` constructs a `ChatClient` for standard providers and a `CodexClient` when `session.provider == "openai-codex"`.

`build_agent` is fallible and returns a clear error when the Codex profile is selected without a valid `auth.json`.

Print mode (`main.rs` non-TUI branch) does **not** trigger the login flow; if a user invokes print mode against a Codex profile without a live `auth.json`, the run errors out with the same "run /login codex first" message.

## 5. Changes by Component

| File                                                  | Change                                                                                                                                                                                                                                                                                                                                                                  |
| ----------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `Cargo.toml` (workspace)                              | Add `async-trait = "0.1"`, `sha2 = "0.10"`, `base64 = "0.22"`, `open = "5"` to `[workspace.dependencies]`.                                                                                                                                                                                                                                                              |
| `crates/themion-core/Cargo.toml`                      | Pull in `async-trait`. (`reqwest`, `serde_json`, `anyhow` already present.)                                                                                                                                                                                                                                                                                             |
| `crates/themion-core/src/lib.rs`                      | Add `pub mod auth;` and `pub mod client_codex;`; re-export `ChatBackend` and `CodexAuth`.                                                                                                                                                                                                                                                                               |
| `crates/themion-core/src/auth.rs` (new)               | `CodexAuth` struct (Serialize/Deserialize/Clone) plus `is_expired(skew_secs)`. Pure data; no IO.                                                                                                                                                                                                                                                                        |
| `crates/themion-core/src/client.rs`                   | Define `pub trait ChatBackend` with `chat_completion_stream`; `impl ChatBackend for ChatClient` wrapping the existing inherent method. Inherent method retained for backward compat. The trait method takes `on_chunk: Box<dyn FnMut(String) + Send + 'static>` — note the boxing and `'static` bound that `async_trait` requires.                                       |
| `crates/themion-core/src/client_codex.rs` (new)       | `CodexClient` struct, `impl ChatBackend`, message translation (§4.5), SSE parser (§4.6), token-refresh helper, model-alias resolver. The `auth_writer: Box<dyn Fn(&CodexAuth) -> Result<()> + Send + Sync>` field is the seam to the CLI's disk writer.                                                                                                                  |
| `crates/themion-core/src/agent.rs`                    | Change `client: OpenRouterClient` to `client: Box<dyn ChatBackend + Send + Sync>`; update all four constructors (`new`, `new_verbose`, `new_with_events`, `new_with_db`) to take the boxed trait object. The `chat_completion_stream` call site (lines 214–218) wraps its closure in `Box::new(...)` to satisfy the trait's `Box<dyn FnMut(String) + Send + 'static>`.   |
| `crates/themion-cli/Cargo.toml`                       | Pull in `sha2`, `base64`, `open`; existing `tokio`, `reqwest`, `serde_json`, `dirs`, `uuid` cover the rest.                                                                                                                                                                                                                                                             |
| `crates/themion-cli/src/auth_store.rs` (new)          | `auth_path()`, `load() -> Result<Option<CodexAuth>>`, `save(&CodexAuth) -> Result<()>` (atomic write + 0600 chmod on Unix). Re-exports the `CodexAuth` from `themion-core` for convenience.                                                                                                                                                                              |
| `crates/themion-cli/src/login_codex.rs` (new)         | Device-flow-based Codex login helper that returns a `DeviceCodeInfo` prompt and a polling future for final token acquisition.                                                                                                                                                                                                                                            |
| `crates/themion-cli/src/config.rs`                    | Add `"openai-codex"` arm to `resolve_profile`; widen the `api_key`-required guard to skip when `provider == "openai-codex"`; add codex constants and the commented example to `CONFIG_TEMPLATE`.                                                                                                                                                                        |
| `crates/themion-cli/src/tui.rs`                       | Add `/login codex` arm in `handle_command`; add `LoginPrompt` and `LoginComplete` event handling; wire `build_agent` to choose between `ChatClient` and `CodexClient`; rebuild the interactive agent on successful login.                                                                                            |
| `crates/themion-cli/src/main.rs`                      | Update print-mode `Agent::new_with_db` call to pass `Box::new(ChatClient::…)`; if `cfg.provider == "openai-codex"`, load `CodexAuth` (or error out with the `/login codex` hint); same `Box::new` wrapping for `CodexClient`.                                                                                                                                           |
| `docs/architecture.md`                                | Add a "Providers" subsection covering `ChatBackend`, the OpenRouter / llamacpp / Codex matrix, and the `auth.json` location. Note the SSE-format divergence between Chat Completions and Responses API.                                                                                                                                                                 |
| `docs/README.md`                                      | Add the PRD-003 row to the PRD table.                                                                                                                                                                                                                                                                                                                                   |

## 6. Edge Cases

- **`auth.json` missing when a Codex profile is selected at startup**: `build_agent` fails fast with the `/login codex` hint.
- **`auth.json` corrupt or malformed JSON**: `auth_store::load` returns `Err`; startup or profile switch fails cleanly.
- **Refresh token rejected by `/oauth/token`** (revoked, expired, account suspended): `chat_completion_stream` returns an error containing the OAuth error body; the TUI surfaces it as a normal turn failure. The user re-runs `/login codex`.
- **Clock skew** between client and server pushes a still-valid token into "expired": the 60-second skew window in `is_expired` covers small drift; on larger drift, refresh succeeds anyway because the refresh path is server-validated.
- **Concurrent agents share one `CodexClient`** (future multi-agent case from PRD-002): the `Arc<RwLock<CodexAuth>>` serializes refresh, so two agents that both notice expiry race only on the write lock; whichever wins refreshes, the loser sees a fresh token on its second read. `auth_writer` is called once per refresh.
- **Responses API returns a non-streaming error frame mid-stream** (`response.failed`): the parser breaks the loop and returns `anyhow::bail!` with the error string from the event payload. The agent loop treats this exactly like any other client error.
- **Model alias resolved to an upstream id the account can't access**: the Responses API returns HTTP 403 with an explanatory body; `chat_completion_stream` propagates it. The user overrides `model = …` in their profile.
- **Switching from a Codex profile back to an OpenRouter profile mid-session via `/config profile use`**: `build_agent` is re-invoked and constructs a fresh `ChatClient`; the `CodexClient` is dropped along with the old `Agent`. `auth.json` is untouched.

## 7. Migration

Existing OpenRouter and llamacpp users see no behavior change: their profiles' `provider` strings unchanged; `Agent::new_with_db` still accepts a client (now boxed); `ChatClient` retains its inherent `chat_completion_stream` method as well as the new trait impl, so any external embedder that called the inherent method continues to work.

The agent's `client` field type changes from concrete `OpenRouterClient` to `Box<dyn ChatBackend + Send + Sync>`. This is a breaking change for any external consumer of `themion-core` — there are none beyond `themion-cli` today. All four `Agent::new*` constructors widen their first parameter symmetrically.

First-time Codex users follow this path:

1. Run themion (any version after upgrade) — the OpenRouter or llamacpp default still works.
2. In TUI, run `/login codex` and complete the browser flow.

`/login codex` creates the `codex` profile automatically, switches to it, and rebuilds the agent in one step. No manual config editing or `/config profile use` required for first-time setup. `/config profile use codex` still works normally to switch back to the Codex profile in later sessions.

`auth.json` survives across themion versions; the schema is intentionally minimal so future additions can be `Option<…>` with `serde(default)`.

## 8. Testing

| Step                                                                                                                | Verify                                                                                                                                                                              |
| ------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Run `cargo build` after the trait refactor with no Codex profile configured                                         | OpenRouter and llamacpp profiles still build and run; one-turn REPL against OpenRouter returns an answer; no compile warnings about unused trait imports.                            |
| In TUI with no `auth.json`, run `/login codex`                                                                      | TUI shows the verification URL and user code, then on success writes `auth.json`, switches to codex profile, and allows Codex chats immediately. |
| Re-run `/login codex` while a previous login is still in flight                                                     | Second invocation is rejected by the `agent_busy` guard with a busy message.                                                               |
| With a valid `auth.json`, set `[profile.codex] provider = "openai-codex"` and run `/config profile use codex`        | Status bar shows `codex` profile and `gpt-5.4` model; one-turn chat completes and stats line shows non-zero token counts.                                  |
| Manually edit `auth.json` to set `expires_at` to one second in the past, then send a chat                            | Token refresh fires before the request; `auth.json` `access_token` and `expires_at` are rewritten on disk; chat completes normally.                                                  |
| Manually corrupt `auth.json` (write `not json`), restart themion targeting the codex profile                        | Startup prints a clear error naming `auth.json` and the `/login codex` hint; process exits non-zero; no panic.                                                                      |
| Delete `auth.json` entirely and run `cargo run -p themion-cli -- "hello"` (print mode) against the codex profile     | Print mode exits non-zero with a stderr line `no codex auth; run /login codex first`; nothing is written to stdout.                                                                  |
| Send a chat that triggers a tool call (e.g. "list files in this dir") through the Codex client                       | TUI shows the tool call and completion; the assistant continues with a coherent reply citing the tool output.                                                                            |
| Unit test: SSE parser fed a synthetic byte stream containing `response.output_text.delta` × 3, `response.completed`  | `chat_completion_stream` returns content equal to the concatenated deltas, `Usage` populated from the completed event.                                                              |
| Unit test: `CodexAuth::is_expired` with `expires_at = now()+30, skew=60`                                             | Returns `true` (within skew window); with `expires_at = now()+120, skew=60`, returns `false`.                                                                                       |
| Concurrent test: two `chat_completion_stream` calls on a shared `CodexClient` whose token is expired                  | Exactly one refresh is observed; both calls succeed.                                                                    |
| Run `cargo run -p themion-cli -- "hello"` (print mode) with the Codex profile active and a valid `auth.json`         | Print-mode answer appears on stdout, stats on stderr, no TUI artifacts; `auth.json` remains valid.                                                                                  |
