# PRD-024: Client-Side Git Repo Identity Normalization for Stylos Agent Git Queries

- **Status:** Implemented
- **Version:** v0.13.1
- **Scope:** `themion-core`, `themion-cli`, docs
- **Author:** Tasanakorn (design) + Themion (PRD authoring)
- **Date:** 2026-04-21
- **Implementation status note:** The model-facing `stylos_query_agents_git` tool description now explicitly tells the model to prefer normalized repo identities like `<host>/<owner>/<repo>`, to normalize explicitly named supported forges before calling when safe, and to ask for clarification rather than guessing when the host is omitted and no safe default is documented. Responder-side matching remains based on locally derived `git_repo_keys` plus exact raw-remote fallback for unsupported forms; no free-form conversational parsing was added server-side.

## Goals

- Clarify that Stylos git-targeted agent discovery is based on comparable repository identity, not only on raw remote-string equality.
- Make `stylos_query_agents_git` practical when the requester knows a repo in one form but responding agents report another equivalent form.
- Define when normalization should happen on the requester side versus the responding server side so unsupported or context-dependent remote forms do not get guessed incorrectly.
- Preserve the current fallback behavior for unsupported hosts and remote syntaxes while making the contract more explicit for future Stylos consumers.
- Shift any natural-language repo shorthand guidance toward the tool description and request contract so the model can follow documented behavior instead of depending on hard-coded conversational parsing logic.

## Non-goals

- No redesign of the broader Stylos query namespace, task lifecycle, or per-instance RPC behavior.
- No requirement to introduce a shared cross-repository Stylos library just to centralize git normalization in this patch.
- No attempt to normalize every possible git hosting convention or private forge URL in the first step.
- No change to exported `git_remotes` values; raw remotes remain visible for transparency and debugging.
- No requirement to infer repository identity from local filesystem layout, branch names, or provider-specific API calls.
- No requirement to hard-code free-form conversational repo parsing in the responder implementation.

## Background & Motivation

### Current state

PRD-020 introduced git metadata caching and remote URL simplification, and PRD-022 documented that Stylos git discovery can match against normalized repo identities such as `github.com/owner/repo` in addition to raw remote URLs. The current architecture docs likewise describe supported normalization for common GitHub, GitLab, and Bitbucket SSH/HTTP(S) forms, with fallback to exact raw-remote comparison for unsupported forms.

That is already useful for common public forge layouts, but it leaves an important practical ambiguity in the contract for `stylos_query_agents_git`:

- should the requester normalize the query input before sending it?
- should the responding server normalize the incoming selector?
- should both sides do it?
- what should happen when the selector can only be understood with caller-local context that the responder does not have?

This matters because some repo references are only meaningful in the requester's context, for example:

- shorthand strings produced by caller-local tooling
- host aliases resolved through local SSH config
- organization-specific mirror names
- internal rewrite rules that map one visible URI form to another canonical identity

A responder cannot safely reverse-engineer those forms without the same local context. If the contract implies that the server should always reconstruct or reinterpret arbitrary git URIs, discovery becomes fragile and may silently fail or mis-match.

### Why requester-side normalization must be first-class

The practical rule is that the requester knows the intent of its own selector better than the responder does.

If a caller can convert a context-specific repo reference into a comparable identity before issuing the query, that comparable identity becomes portable across the mesh. The server then only needs to compare the incoming selector against its locally derived comparable identities.

This keeps the matching contract robust:

- requesters normalize what they know
- responders compare against what they can derive locally
- neither side guesses unsupported context-dependent forms

**Alternative considered:** require responders to canonicalize every incoming git selector into a portable identity. Rejected: many forms depend on caller-local context the responder does not have, so forced server-side reconstruction would either fail or guess incorrectly.

### Why natural-language shorthand should be documented in the tool contract, not hard-coded in the server

Some practical repo references may come from user phrasing rather than from a raw git remote string, for example:

- `github tasanakorn/stele`
- `tasanakorn/stele repo`

Those can be useful inputs in higher-level clients and model-facing tools, but they should be treated as requester-local convenience behavior described in the tool contract rather than as mandatory server-side parsing behavior.

That distinction matters because model guidance should come from the tool description and schema semantics whenever possible. If the intended behavior is documented clearly in the tool description, the model can normalize or reformulate the selector before issuing `stylos_query_agents_git`, instead of requiring the responder to contain special hard-coded logic for conversational phrases.

Requester-side guidance:

- a tool description may instruct the model to prefer normalized selectors such as `github.com/tasanakorn/stele`
- a client or tool adapter may expand `github tasanakorn/stele` to `github.com/tasanakorn/stele`
- a client or tool adapter may expand `tasanakorn/stele repo` to `github.com/tasanakorn/stele` only if that client explicitly defines a local heuristic such as "host omitted means GitHub"
- if the client or tool adapter does not have a well-defined heuristic, it should leave the selector unresolved or ask for clarification instead of guessing

Responder-side guidance:

- responders should not attempt to interpret free-form phrases such as `github tasanakorn/stele` or `tasanakorn/stele repo`
- responders should only compare the concrete selector they receive, using documented normalization rules when safe

This keeps natural-language reconstruction in the place that has the user's immediate intent and local UX policy, while also making the model-facing contract explicit.

**Alternative considered:** let responders parse conversational repo phrases directly. Rejected: the meaning of those phrases depends on client UX choices and local assumptions, especially when the host is omitted.

### Why this is a patch-level clarification

This proposal sharpens an existing behavior area rather than introducing a brand-new user-visible Stylos capability. The existing implementation already uses normalized comparable repo identities for common hosts and falls back to exact raw-remote comparison when normalization is not possible.

The change needed is to make the contract explicit and practical: client-side/requester-side normalization should be recommended and supported as the primary way to turn caller-specific git references into portable selectors.

**Alternative considered:** treat this as a minor feature because it affects discovery ergonomics. Rejected: the core capability already exists; this PRD clarifies responsibility boundaries and small additive request behavior rather than adding a new query family.

## Design

### Define two selector classes for `stylos_query_agents_git`

`stylos_query_agents_git` should explicitly accept either of these selector classes:

1. a raw remote string
2. a portable comparable repo identity in normalized form

Recommended normalized form remains:

```text
<host>/<owner>/<repo>
```

Examples:

- `github.com/example/themion`
- `gitlab.com/group/project`
- `bitbucket.org/team/repo`

Normative behavior:

- when the requester already knows a portable comparable identity, it should send that identity directly
- when the requester has only a raw remote URL in a supported form, it may send the raw remote and let existing normalization rules apply
- when the requester has a context-dependent shorthand or alias, it should normalize or expand that value locally before sending the request when possible
- unsupported selector forms remain allowed, but only exact raw-string comparison should be expected in that case

This separates portable identities from transport-visible raw remote strings without removing support for either.

**Alternative considered:** add a separate dedicated request field for normalized identities and deprecate `remote`. Rejected: unnecessary API churn for a patch-level improvement when one field can already carry either a raw remote or a normalized comparable identity.

### Make requester-side normalization the preferred contract

The docs should explicitly state that normalization may happen on both sides, but requester-side normalization is preferred whenever the caller has context the responder lacks.

Normative matching model:

- the requester should normalize its selector to `<host>/<owner>/<repo>` when it can do so confidently
- the responder should continue deriving `git_repo_keys` from its local `git_remotes` using the documented supported-host normalization rules
- matching should first compare the incoming selector against responder-derived comparable identities when the selector is already in normalized comparable form or can be normalized safely under the existing known-host rules
- if the selector cannot be normalized safely, the responder should compare it only against raw exported `git_remotes` for exact equality
- the responder must not invent host aliases, SSH-config rewrites, or organization-local mappings it cannot prove from local data

This gives a practical division of responsibility instead of implying that one side can solve every case alone.

**Alternative considered:** require all requesters to send only normalized repo identities. Rejected: existing callers may still only have a raw remote string, and common supported remote forms should remain convenient.

### Put model guidance in the tool description

The model-facing description for `stylos_query_agents_git` should explicitly instruct the model to prefer a normalized comparable repo identity when it can infer one safely, instead of relying on responder-side parsing of conversational shorthand.

Recommended tool-description guidance:

- when asking for a specific repository, prefer `remote` values in normalized form such as `<host>/<owner>/<repo>`
- if the user mentions a supported forge explicitly, the model may convert that into normalized form before calling the tool
- if the user omits the host and the client/tool contract does not define a safe default, the model should ask for clarification rather than guessing
- do not rely on the responder to parse conversational phrases like `owner/repo repo`

Example description-oriented behavior:

- user says `ask who has github tasanakorn/stele` → model should call the tool with `remote: "github.com/tasanakorn/stele"`
- user says `ask who has tasanakorn/stele repo` → model should either use a documented client-side GitHub-default rule or ask a clarification question

This keeps behavior driven by documented tool semantics that the model can follow, rather than by hidden hard-coded parsing logic in the server.

**Alternative considered:** keep the tool description minimal and rely on implementation heuristics for conversational forms. Rejected: that makes behavior less transparent and harder for the model to follow consistently.

### Examples of requester-local expansion

Examples that are reasonable only as requester-local behavior:

- `github tasanakorn/stele` → requester or tool adapter may normalize to `github.com/tasanakorn/stele`
- `tasanakorn/stele repo` → requester or tool adapter may normalize to `github.com/tasanakorn/stele` only when its local UX explicitly defines GitHub as the omitted-host default
- `tasanakorn/stele repo` with no such default → requester or tool adapter should ask for clarification or leave the selector unresolved

Normative rule:

- conversational shorthand may be reconstructed into normalized repo identity by the requester or model-facing tool adapter
- conversational shorthand must not be treated as a guaranteed portable wire syntax that responders are expected to parse

This keeps the mesh contract concrete while still allowing ergonomic clients.

**Alternative considered:** document conversational shorthand as equivalent wire-level input to raw remotes and normalized identities. Rejected: too ambiguous across clients, hosts, and future forge defaults.

### Treat server-side normalization as local derivation, not URI reconstruction

Responding Themion instances should continue normalizing only from concrete locally known git metadata:

- locally exported `git_remotes`
- supported public forge URI forms already documented
- direct comparable repo identity strings when supplied as such

They should not try to reconstruct meaning from unknown caller-specific forms beyond what the current rules can prove.

Normative responder rule:

- normalize what is locally known and well-specified
- do not guess what is caller-specific and ambiguous

This keeps false positives lower and makes failures easier to reason about.

**Alternative considered:** add heuristics for unknown hosts by splitting on punctuation and guessing owner/repo structure. Rejected: likely to create unstable matching semantics and accidental collisions across private forge setups.

### Keep `git_repo_keys` as the portable comparison surface

`git_repo_keys` should remain the documented portable comparison surface in replies and status snapshots.

Normative behavior:

- responding instances continue to export raw `git_remotes` for transparency
- responding instances continue to export derived `git_repo_keys` when local normalization succeeds
- callers that plan to cache or reuse discovery answers should prefer `git_repo_keys` over raw remotes when present
- future Stylos clients should treat `git_repo_keys` as the de-facto comparable identity contract for repo-targeted discovery

This helps stabilize cross-instance matching without hiding the original remote strings.

**Alternative considered:** stop returning raw remotes once `git_repo_keys` exist. Rejected: raw remotes remain useful for diagnostics and for exact-match fallback on unsupported forms.

## Changes by Component

| File | Change |
| ---- | ------ |
| `docs/prd/prd-024-client-side-git-repo-identity-normalization-for-stylos-agent-git-queries.md` | Add the proposed contract clarifying requester-side normalization responsibility, model/tool-description guidance, requester-local shorthand expansion examples, and responder-side comparison limits for `stylos_query_agents_git`. |
| `docs/architecture.md` | Clarify that git query selectors may be raw remotes or normalized comparable identities, that requester-side normalization is preferred when caller-local context is required, and that responder-side normalization is limited to locally known/supported forms. |
| `crates/themion-core/src/tools.rs` | Update the `stylos_query_agents_git` tool description so the model is instructed to prefer normalized comparable identities and not depend on responder-side parsing of conversational shorthand. |
| `crates/themion-cli/src/stylos.rs` | Keep the existing matching behavior but, if implementation updates are needed, ensure request handling treats normalized comparable selectors as first-class input and does not attempt unsupported URI reconstruction beyond documented rules. |
| `docs/README.md` | Add this PRD to the PRD index with proposed status. |

## Edge Cases

- a caller sends `github.com/owner/repo` directly → responders should compare it against derived `git_repo_keys` and match regardless of local SSH or HTTPS remote syntax.
- a caller sends `git@github.com:owner/repo.git` → responders should normalize it under existing supported-host rules and compare successfully against `git_repo_keys`.
- a user asks for `github tasanakorn/stele` and the model follows the tool description → the tool call should use `remote: "github.com/tasanakorn/stele"`, after which responders should match normally via `git_repo_keys`.
- a user asks for `tasanakorn/stele repo` and the model has no documented omitted-host default in the tool/client contract → the model should ask for clarification instead of guessing.
- a caller sends `tasanakorn/stele repo` through a client that defaults omitted hosts to GitHub → the client may send `github.com/tasanakorn/stele`, but that default should be documented as client-local behavior rather than responder behavior.
- a caller sends a private host alias such as `corp-gh:team/repo` that only has meaning in local SSH config → responders should not guess; matching should succeed only if the caller normalized it first or if an exact raw remote string match exists.
- a caller sends an internal mirror URL that maps to a public upstream repo by local policy → responders should not infer the upstream identity unless that mapping is explicitly encoded in the selector sent by the caller.
- a responder has unsupported-host remotes and therefore no derived `git_repo_keys` → it should still participate in exact raw-remote matching using `git_remotes`.
- a caller caches discovery answers and reuses them later → it should prefer cached `git_repo_keys` when available because they are more portable than raw remotes.

## Migration

This is a backward-compatible clarification and small behavior-tightening for Stylos-enabled git discovery.

Migration expectations:

- existing callers that send supported GitHub, GitLab, or Bitbucket remote forms continue to work
- model-facing tool descriptions should now steer callers toward normalized comparable identities when practical
- callers with caller-local shorthand, alias, mirror forms, or conversational repo phrases should normalize those selectors before issuing `stylos_query_agents_git` when they want portable matching
- responders continue exporting `git_remotes` and `git_repo_keys`, with no wire-shape break required
- future Stylos clients should prefer normalized comparable identities for repo-targeted discovery whenever practical

No database or non-Stylos migration is required.

## Testing

- update the `stylos_query_agents_git` tool description to prefer normalized comparable repo identities → verify: the documented tool contract tells the model to send `remote` as `<host>/<owner>/<repo>` when it can infer that safely.
- invoke `stylos_query_agents_git` with a normalized selector such as `github.com/example/themion` → verify: responders match agents whose exported remotes use equivalent SSH or HTTPS forms.
- invoke `stylos_query_agents_git` with a supported raw remote such as `git@github.com:example/themion.git` → verify: responders normalize and match against the same repo identity.
- issue a user request equivalent to `ask who has github tasanakorn/stele` through the model-facing tool path → verify: the model/tool path emits `remote: "github.com/tasanakorn/stele"` rather than depending on responder-side phrase parsing.
- issue a user request equivalent to `ask who has tasanakorn/stele repo` with no documented omitted-host default → verify: the model asks for clarification or leaves the selector unresolved rather than guessing silently.
- invoke `stylos_query_agents_git` with an unsupported caller-local alias such as `corp-gh:team/repo` without prior normalization → verify: responders do not guess a comparable identity and only exact raw-remote matches succeed.
- invoke `stylos_query_agents_git` with the same caller-local alias after client-side expansion to a portable identity → verify: matching succeeds through `git_repo_keys` comparison.
- inspect a git discovery reply from a responder attached to a supported forge repo → verify: both raw `git_remotes` and normalized `git_repo_keys` remain visible.
- run `cargo check -p themion-core -p themion-cli --features stylos` after any implementation touch-up → verify: Stylos-enabled builds still compile cleanly with the clarified tool-description and git-query contract.
