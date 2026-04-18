# PRD-003: OpenAI Codex Subscription Provider

- **Status:** Proposed
- **Version:** v0.3.0
- **Scope:** `themion-core` (new `client_codex` module, `CodexAuth` data type, agent client abstraction); `themion-cli` (auth store IO, login flow in TUI, config provider arm); workspace `Cargo.toml`; docs
- **Author:** Tasanakorn (design) + Claude Code (PRD authoring)
- **Date:** 2026-04-18

## 1. Goals

- Let users authenticate against their existing ChatGPT / OpenAI Codex subscription via a `/login codex` command in the TUI; no API key, no separate billing.
- Drive completions through the OpenAI Responses API at `https://chatgpt.com/backend-api/codex/responses` using the OAuth access token issued during login.
- Persist `{ access, refresh, expires, account_id }` to `~/.config/themion/auth.json` so subsequent process launches reuse the session without re-authenticating.
- Refresh the access token transparently before any request whose token has expired (or is about to), and rewrite `auth.json` with the fresh values.
- Default the provider's model alias `codex-mini-latest` to the upstream model id `gpt-5.1-codex-mini`, while leaving the alias overridable in config.

## 2. Non-goals

- No support for the standard OpenAI platform API key flow (`api.openai.com/v1/chat/completions`). The Responses-API endpoint and the subscription auth path are the only OpenAI integration this PRD covers.
- No WebSocket transport. The Responses API supports a websocket variant; this PRD is SSE-only, matching the existing `chat_completion_stream` model.
- No headless / device-code OAuth flow. The login flow opens a browser and listens on a fixed loopback port; users on remote shells fall back to copying the URL by hand.
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
- `CODEX_DEFAULT_MODEL    = "codex-mini-latest"`

The `api_key`-required guard at `config.rs:157–167` is widened to skip when `provider == "openai-codex"`. No `api_key` env var is honored for this provider — the credential lives entirely in `auth.json`.

The auto-generated `CONFIG_TEMPLATE` gains a commented example:

```toml
# [profile.codex]
# provider = "openai-codex"
# # model defaults to "codex-mini-latest" → gpt-5.1-codex-mini
# # log in once with: /login codex
```

Model alias mapping (`codex-mini-latest` → `gpt-5.1-codex-mini`) lives inside `CodexClient::resolve_model_alias`, not in config — the alias is a property of the upstream API and would otherwise force every user to know the underlying SKU.

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
2. Resolve the model alias: `model_id = resolve_model_alias(model)` (single-arm match: `"codex-mini-latest" => "gpt-5.1-codex-mini"`, otherwise pass-through).
3. Translate `&[Message]` → Responses-API `input` array (see §4.5).
4. POST to `{base_url}/responses` with body `{"model": model_id, "instructions": system_prompt, "input": [...], "tools": [...], "stream": true}`. Required headers: `Authorization: Bearer {access}`, `chatgpt-account-id: {account_id}`, `originator: pi`, `OpenAI-Beta: responses=experimental`, `Content-Type: application/json`, `Accept: text/event-stream`.
5. Parse SSE frames in the named-event format (see §4.6), assemble a `ResponseMessage` and `Option<Usage>` from the `response.completed` payload, return.

System-prompt handling: `Agent` currently prepends a `role="system"` `Message` to the slice it sends. `CodexClient` extracts that first system message into the Responses-API `instructions` field and drops it from `input`; subsequent system-role messages (the windowing hint from PRD-002) are translated as `{"type": "message", "role": "developer", "content": [...]}` items inside `input`, since `instructions` accepts only one string.

**Implementer note:** the hardcoded OAuth `client_id` (`app_EMoamEEZ73f0CkXaXp7hrann` per §4.7), the `originator: pi` header value, and the `OpenAI-Beta: responses=experimental` header are all transcribed from the public Codex CLI behavior and were not independently re-verified for this PRD. Cross-check against the upstream Codex CLI source before merging.

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

A new module `crates/themion-cli/src/login_codex.rs` exposes:

```rust
pub async fn run_login_flow() -> Result<CodexAuth>;
```

Implementation steps when invoked:

1. Generate a 32-byte random `code_verifier` (URL-safe base64), derive `code_challenge = base64url(sha256(code_verifier))`.
2. Bind a `tokio::net::TcpListener` on `127.0.0.1:1455`. If the bind fails (port in use), surface a clear error mentioning the port; do not retry on a different port — the OAuth client registration pins this exact `redirect_uri`.
3. Construct the auth URL `https://auth.openai.com/authorize?response_type=code&client_id={CLIENT_ID}&redirect_uri=http://localhost:1455/auth/callback&scope=openid%20profile%20email%20offline_access&code_challenge={challenge}&code_challenge_method=S256&state={random}`.
4. Open it in the user's browser via the `open` crate. If the open call fails, print the URL to the TUI as a fallback (the listener still works).
5. Accept exactly one HTTP request. Parse `code` and `state` from the query string; reject if `state` mismatches. Respond with a static HTML page reading "Login complete, you may close this tab."
6. POST to `https://auth.openai.com/oauth/token` with `grant_type=authorization_code&code={code}&code_verifier={verifier}&redirect_uri=http://localhost:1455/auth/callback&client_id={CLIENT_ID}`. Parse `{access_token, refresh_token, expires_in, id_token}`.
7. Decode the `id_token` JWT (split on `.`, base64url-decode the middle segment) and read the `https://api.openai.com/auth.chatgpt_account_id` claim into `account_id`.
8. Build `CodexAuth { access_token, refresh_token, expires_at: now() + expires_in, account_id }` and return it.

`CLIENT_ID` is hardcoded as `app_EMoamEEZ73f0CkXaXp7hrann` (the public Codex CLI client id; not a secret). It lives in a `const` in `login_codex.rs`. See the implementer note in §4.4 about cross-checking this value against upstream Codex CLI source.

The HTTP "server" is a hand-rolled read of one request from one accepted socket — `tokio::io::AsyncReadExt::read_buf` until the request line and headers are complete, parse the request line for the path/query, write a fixed 200 response. No HTTP framework dependency.

**Alternative considered:** spawn a real HTTP server (`axum`, `hyper`). Rejected: `axum` would add ~80 transitive crates for a single-request lifetime; the hand-rolled reader is ~60 lines.

**Alternative considered:** request a system-assigned ephemeral port and use it in `redirect_uri`. Rejected: OpenAI's OAuth client registration for the Codex CLI fixes the redirect URI to `http://localhost:1455/auth/callback`; an ephemeral port would be rejected by the authorize endpoint.

### 4.8 TUI `/login codex` command

`handle_command` gains an arm for `/login codex`. Because the function is sync, it cannot run the OAuth flow inline; it instead:

1. Sets `self.agent_busy = true` and pushes an `Entry::Assistant("logging in to OpenAI Codex…")` line.
2. Spawns a `tokio::task` that calls `login_codex::run_login_flow().await`, then `auth_store::save(&auth)`, and dispatches `AppEvent::LoginComplete(Result<CodexAuth>)` back into the main loop.
3. Returns `vec![]` from `handle_command` so the user sees only the placeholder line; the result line is appended later when the event arrives.

Two new `AppEvent` variants:

```rust
AppEvent::LoginStarted,                          // emitted immediately for symmetry
AppEvent::LoginComplete(Result<CodexAuth>),
```

Handler in the main loop:

- `LoginStarted` → no-op today; reserved for animating a spinner if needed.
- `LoginComplete(Ok(auth))` → push `Entry::Assistant("logged in as account {auth.account_id}; codex profile is now usable")`; clear `agent_busy`; if the active profile is already `provider == "openai-codex"`, rebuild the agent so the new token is picked up immediately (the existing pattern from `/config profile use`).
- `LoginComplete(Err(e))` → push `Entry::Assistant(format!("login failed: {e}"))`; clear `agent_busy`.

The `agent_busy` guard prevents a user from triggering a second login or sending a chat message while the OAuth flow is in flight.

### 4.9 Wiring the Codex client into `build_agent`

`build_agent` in `tui.rs` currently constructs a `ChatClient` for every provider. With the trait abstraction, the factory becomes:

```rust
fn build_agent(...) -> Result<Agent> {
    let client: Box<dyn ChatBackend + Send + Sync> = match session.provider.as_str() {
        "openai-codex" => {
            let auth = auth_store::load()?
                .ok_or_else(|| anyhow!("no codex auth; run /login codex first"))?;
            Box::new(CodexClient::new(
                session.base_url.clone(),
                auth,
                Box::new(|a: &CodexAuth| auth_store::save(a)),
            ))
        }
        _ => {
            let mut c = ChatClient::new(session.base_url.clone(), session.api_key.clone());
            if session.provider == "openrouter" {
                c = c.with_headers([…existing headers…]);
            }
            Box::new(c)
        }
    };
    Ok(Agent::new_with_db(client, …))
}
```

`build_agent` becomes fallible (returns `Result<Agent>`). The two call sites in `tui.rs` (`App::new` and the `/config profile use` arm) handle the error by showing a message and falling back: in `App::new`, by panicking with a clear stderr line (it's startup); in the profile-switch arm, by emitting an entry and reverting the profile.

`Agent::new_with_db`'s signature changes from taking `OpenRouterClient` to `Box<dyn ChatBackend + Send + Sync>`. Existing call sites in `main.rs` (print mode) wrap their `ChatClient` in a `Box::new` — one line each.

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
| `crates/themion-cli/src/login_codex.rs` (new)         | `run_login_flow() -> Result<CodexAuth>`: PKCE keygen, browser launch, loopback listener on 1455, code exchange, JWT account-id extraction.                                                                                                                                                                                                                              |
| `crates/themion-cli/src/config.rs`                    | Add `"openai-codex"` arm to `resolve_profile`; widen the `api_key`-required guard to skip when `provider == "openai-codex"`; add codex constants and the commented example to `CONFIG_TEMPLATE`.                                                                                                                                                                        |
| `crates/themion-cli/src/tui.rs`                       | Add `/login codex` arm in `handle_command`; add `AppEvent::LoginStarted` and `AppEvent::LoginComplete` variants; wire `build_agent` to choose between `ChatClient` and `CodexClient`; rebuild the interactive agent on `LoginComplete(Ok)` when the active profile is codex.                                                                                            |
| `crates/themion-cli/src/main.rs`                      | Update print-mode `Agent::new_with_db` call to pass `Box::new(ChatClient::…)`; if `cfg.provider == "openai-codex"`, load `CodexAuth` (or error out with the `/login codex` hint); same `Box::new` wrapping for `CodexClient`.                                                                                                                                           |
| `docs/architecture.md`                                | Add a "Providers" subsection covering `ChatBackend`, the OpenRouter / llamacpp / Codex matrix, and the `auth.json` location. Note the SSE-format divergence between Chat Completions and Responses API.                                                                                                                                                                 |
| `docs/README.md`                                      | Add the PRD-003 row to the PRD table.                                                                                                                                                                                                                                                                                                                                   |

## 6. Edge Cases

- **Port 1455 already in use** (another themion login in flight, or another tool squatting): `run_login_flow` returns an error; `LoginComplete(Err)` shows a message naming the port. The user kills the offender and retries.
- **Browser fails to open** (no DISPLAY, headless server): `open` returns an error; the URL is printed to the TUI so the user can paste it into a browser on another machine. The loopback listener will not see the callback in that case — the user needs to forward port 1455 themselves; this case is documented but not specially supported.
- **OAuth callback returns `error=…`** (user denies consent, expired authorize URL): the listener parses the `error` query parameter and propagates it as the `LoginComplete(Err)` payload.
- **`state` mismatch**: the listener returns 400 to the browser and bails; `LoginComplete(Err)` shows "state mismatch". Prevents a CSRF attacker from feeding a foreign `code` into our listener.
- **`auth.json` missing when a Codex profile is selected at startup**: `build_agent` fails fast; `App::new` prints a clear message including the `/login codex` hint and exits. (TUI startup already exits cleanly on `?` errors today.)
- **`auth.json` corrupt or malformed JSON**: `auth_store::load` returns `Err`; same exit path. The user deletes the file and re-runs login.
- **Refresh token rejected by `/oauth/token`** (revoked, expired, account suspended): `chat_completion_stream` returns an error containing the OAuth error body; the TUI surfaces it as a normal turn failure. The user re-runs `/login codex`. `auth.json` is **not** deleted automatically — the user retains the option to inspect it.
- **Clock skew** between client and server pushes a still-valid token into "expired": the 60-second skew window in `is_expired` covers small drift; on larger drift, refresh succeeds anyway because the refresh path is server-validated.
- **Concurrent agents share one `CodexClient`** (future multi-agent case from PRD-002): the `Arc<RwLock<CodexAuth>>` serializes refresh, so two agents that both notice expiry race only on the write lock; whichever wins refreshes, the loser sees a fresh token on its second read. `auth_writer` is called once per refresh.
- **Responses API returns a non-streaming error frame mid-stream** (`response.failed`): the parser breaks the loop and returns `anyhow::bail!` with the error string from the event payload. The agent loop treats this exactly like any other client error.
- **Tool-call streaming with no `output_item.added` before deltas arrive**: the parser allocates a slot lazily on the first `function_call_arguments.delta` if needed, keyed by `item_id`, with empty `name`/`call_id`; if the missing fields never arrive, the resulting `ToolCall` has empty name/id and the agent loop will fail naturally on the next tool dispatch — no panic.
- **Model alias resolved to an upstream id the account can't access**: the Responses API returns HTTP 403 with an explanatory body; `chat_completion_stream` propagates it. The user overrides `model = …` in their profile.
- **Switching from a Codex profile back to an OpenRouter profile mid-session via `/config profile use`**: `build_agent` is re-invoked and constructs a fresh `ChatClient`; the `CodexClient` is dropped along with the old `Agent`. `auth.json` is untouched.

## 7. Migration

Existing OpenRouter and llamacpp users see no behavior change: their profiles' `provider` strings unchanged; `Agent::new_with_db` still accepts a client (now boxed); `ChatClient` retains its inherent `chat_completion_stream` method as well as the new trait impl, so any external embedder that called the inherent method continues to work.

The agent's `client` field type changes from concrete `OpenRouterClient` to `Box<dyn ChatBackend + Send + Sync>`. This is a breaking change for any external consumer of `themion-core` — there are none beyond `themion-cli` today. All four `Agent::new*` constructors widen their first parameter symmetrically.

First-time Codex users follow this path:

1. Run themion (any version after upgrade) — the OpenRouter or llamacpp default still works.
2. Edit `~/.config/themion/config.toml`, uncomment the `[profile.codex]` block (now present in the regenerated template; existing files are not auto-edited).
3. In TUI, run `/login codex`, complete the browser flow.
4. Run `/config profile use codex`. Subsequent turns hit the Codex Responses API.

`auth.json` survives across themion versions; the schema is intentionally minimal so future additions can be `Option<…>` with `serde(default)`.

## 8. Testing

| Step                                                                                                                | Verify                                                                                                                                                                              |
| ------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Run `cargo build` after the trait refactor with no Codex profile configured                                         | OpenRouter and llamacpp profiles still build and run; one-turn REPL against OpenRouter returns an answer; no compile warnings about unused trait imports.                            |
| In TUI with no `auth.json`, run `/login codex`                                                                      | Browser opens to `auth.openai.com/authorize?…`; loopback listener accepts the callback; entry shows "logged in as account …"; `~/.config/themion/auth.json` exists with mode `0600`. |
| Re-run `/login codex` while a previous login is still in flight                                                     | Second invocation is rejected by the `agent_busy` guard with "wait for current operation"; only one browser tab opens.                                                               |
| With a valid `auth.json`, set `[profile.codex] provider = "openai-codex"` and run `/config profile use codex`        | Status bar shows `codex` profile and `codex-mini-latest` model; one-turn chat completes and stats line shows non-zero `in:` / `out:` token counts.                                  |
| Manually edit `auth.json` to set `expires_at` to one second in the past, then send a chat                            | Token refresh fires before the request; `auth.json` `access_token` and `expires_at` are rewritten on disk; chat completes normally.                                                  |
| Manually corrupt `auth.json` (write `not json`), restart themion targeting the codex profile                        | Startup prints a clear error naming `auth.json` and the `/login codex` hint; process exits non-zero; no panic.                                                                      |
| Delete `auth.json` entirely and run `cargo run -p themion-cli -- "hello"` (print mode) against the codex profile     | Print mode exits non-zero with a stderr line "no codex auth; run /login codex first"; nothing is written to stdout.                                                                  |
| Bind port 1455 from another process, then run `/login codex`                                                         | Login fails with "port 1455 in use"; `auth.json` is untouched.                                                                                                                       |
| Send a chat that triggers a tool call (e.g. "list files in this dir") through the Codex client                       | TUI shows `↳ list_directory: …  ✓`; the assistant continues with a coherent reply citing the tool output.                                                                            |
| Unit test: SSE parser fed a synthetic byte stream containing `response.output_text.delta` × 3, `response.completed`  | `chat_completion_stream` returns content equal to the concatenated deltas, `Usage` populated from the completed event.                                                              |
| Unit test: SSE parser fed a `response.failed` event mid-stream                                                       | Function returns `Err`; error string contains the event's `error.message`.                                                                                                          |
| Unit test: `translate_messages` over a `[user, assistant+tool_calls, tool, user]` history                            | Output sequence matches the §4.5 table row-for-row; first system message is hoisted into `instructions`.                                                                            |
| Unit test: `CodexAuth::is_expired` with `expires_at = now()+30, skew=60`                                             | Returns `true` (within skew window); with `expires_at = now()+120, skew=60`, returns `false`.                                                                                       |
| Concurrent test: two `chat_completion_stream` calls on a shared `CodexClient` whose token is expired                  | Exactly one POST to `/oauth/token` is observed (instrument with a counter in `auth_writer`); both calls succeed.                                                                    |
| Run `cargo run -p themion-cli -- "hello"` (print mode) with the Codex profile active and a valid `auth.json`         | Print-mode answer appears on stdout, stats on stderr, no TUI artifacts; `auth.json` remains valid.                                                                                  |
