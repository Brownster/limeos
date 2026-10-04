# LimeOS assistant: subscription, APIs, and model routing

Status: proposed design. Prepared: 2026-10-04; revised the same day to match the cutover scope.
Companion documents: [target architecture](2026-10-04-rust-rewrite-architecture.md), [delivery roadmap](2026-10-04-rust-rewrite-roadmap.md), and [defect register](2026-10-04-rust-rewrite-defect-register.md).

## 1. Product behavior

The assistant is a native LimeOS feature with access to explicitly permitted application operations. It can answer questions using versioned documentation and current observations, investigate an incident, request media, and execute approved maintenance. It shares the application's operation registry, policy, jobs, and verification. A model proposes actions; core decides whether an action is authorized and executes it through the appropriate adapter.

Support four provider paths:

1. Claude subscription through an isolated, supported Claude Code runtime.
2. Native APIs, initially Anthropic, then OpenAI and Gemini.
3. Explicitly configured compatible endpoints, including gateways, qualified individually.
4. Local or off-box model services, initially an Ollama adapter where useful.

The cutover release (P05) ships paths 1 and 2 for Anthropic only. OpenAI, Gemini, compatible endpoints, and local models follow cutover in P08; media requests follow in P07.

Subscription support ships with the first assistant milestone. The user can prefer subscription for normal work and configure API models for selected tasks or opt-in fallback. A subscription limit never silently starts paid API calls.

The default conversation interface is inside LimeOS. Mattermost and future chat transports deliver the same scoped conversation events; they do not define the assistant's capabilities or force deployment of a chat server. P00 decides from actual use whether the Mattermost transport returns after cutover (P09).

## 2. Separate four concerns

| Concern | Meaning | Example |
|---|---|---|
| Connection | Authentication, endpoint, privacy rules, rate limits, availability. | One subscription account; a direct API account; a LAN model server. |
| Model deployment | Model identity through a connection, protocol, supported features, context/output limits, price revision. | The same model through a direct provider or a gateway is two deployments. |
| Task profile | Required capabilities, quality qualification, privacy, latency, and budget. | Documentation answer, incident diagnosis, media request, or code proposal. |
| Authority | Principal, resource scopes, allowed operations, approval rules, expiry. | May add movies to one library; may not change disks or deploy stacks. |

A more capable or more expensive model does not receive additional authority. An administrator changing the routing profile does not change role grants. Transport identity and model identity are also independent.

## 3. Canonical task interface and native adapters

Use a Rust provider trait with bounded request/response types and streaming events. Keep business logic out of adapters. The contract includes:

- Request/task ID, model deployment, task profile, deadline, cancellation, and input/output bounds.
- Canonical messages containing user text, assistant text, typed tool calls/results, and source-tagged evidence. Logs, document excerpts, and external messages are untrusted evidence, not instructions.
- The permitted tool definitions for this task: versioned operation name, parameter schema, and explanation. Do not send unrelated administrator tools to a media requester.
- Adapter-owned native continuation state; capability declarations; supported schema subset and generation controls.
- Events for public text deltas, fully assembled tool proposals, usage, completion, and typed errors. No public chain-of-thought stream or chain-of-thought audit record.

Illustrative internal types, not a frozen API:

```rust
struct ToolProposal {
    call_id: String,
    operation: OperationVersion,
    parameters: serde_json::Value, // untrusted until core validates the operation
}

enum ModelEvent {
    TextDelta(String),
    ToolProposals(Vec<ToolProposal>),
    Usage(UsageReport),
    Complete(CompletionReason),
    Error(ProviderError),
}
```

Adapters map provider-compatible tool names to canonical names through an explicit table. For example, `container_restart_v1` maps to `container.restart@1`; do not replace punctuation in a way that creates collisions. Arguments are untrusted until core validates them, despite any structured-output guarantee.

Support multiple tool proposals in a response. Independent safe reads may run concurrently within limits. Each mutation goes through durable planning and authorization. Execute nothing from a partial streaming tool call; assemble and validate the complete response before dispatching proposals. Return bounded structured results with native call IDs, evidence timestamps, and explicit errors.

### Protocol families

| Adapter | Native behavior to preserve | Required qualification |
|---|---|---|
| Claude subscription CLI | Process lifecycle, structured output, explicit model choice, authentication state, rate-limit/error reporting. | Exact CLI version/flags, isolation, login lifecycle, schema and cancellation fixtures. |
| Anthropic API | Native content blocks, tool requests/results, streaming, usage, supported continuation state. | Multi-tool conversations and model-specific schema/capability tests. |
| OpenAI API | Responses API items, function call IDs and outputs, supported reasoning continuation items, usage. | Stateful tool loops, stream completion, explicit storage/privacy configuration. |
| Gemini API | Its selected native API's function calls and opaque continuation/signature requirements. | Pin one supported API surface per adapter version; verify sequential/parallel tool flow. |
| Compatible endpoint | Only the specific documented subset that endpoint implements. | It cannot inherit a provider's qualifications by claiming compatibility. |
| Ollama | Selected service/model's native tool and streaming behavior. | Host capacity, context bounds, actual tool accuracy, and overload handling. |

Native tool protocols differ. Anthropic uses tool content blocks; OpenAI uses function-call items and associated outputs; Gemini may require opaque thought signatures across turns. Preserve each provider's required state without translating it into public text or another provider's internal state. [Anthropic tool documentation](https://platform.claude.com/docs/en/agents-and-tools/tool-use/overview), [OpenAI function-calling documentation](https://developers.openai.com/api/docs/guides/function-calling), [Gemini thinking documentation](https://ai.google.dev/gemini-api/docs/thinking)

Canonical conversation, task checkpoints, and verified tool results belong to LimeOS. Provider continuation blobs are encrypted, scoped, short-lived adapter data. Cross-provider changes reconstruct a task from those canonical facts at a safe checkpoint. They do not transplant opaque reasoning state or replay completed mutations.

Provider-hosted shell, computer, browsing, code execution, or remote MCP tools are disabled by default. Any future use requires the same resource policy, disclosure controls, and auditable operation path; enabling native APIs must not create a second ungoverned execution route. [OpenAI tools documentation](https://developers.openai.com/api/docs/guides/tools)

## 4. Claude subscription runtime

Use a dedicated provider identity and login home, with a fixed executable, explicit model selection, read-only working directory, and no inherited checkout customization. Disable built-in execution tools and unapproved MCP servers. Treat its structured answer as an untrusted proposal returned to core.

Pin and test the actual CLI command contract. `--safe-mode` disables customization but does not remove built-in tools; `--tools ""` does not itself disable MCP servers. Use the documented restriction mechanisms and an explicit empty/strict MCP configuration as qualified for that release. Output schemas and flags are version-sensitive. Never depend on a flag name as the operating-system security boundary. [Claude CLI reference](https://code.claude.com/docs/en/cli-reference)

The runner has no executor sockets, application state, media filesystem, or administrative credentials. Restrict subprocesses, filesystem visibility, outbound destinations, memory, process count, output size, and wall time. Default to one subscription request at a time. Test token refresh, account disconnect/reconnect, CLI upgrades, timeout, descendant termination, and subscription exhaustion.

Authentication happens through the provider's supported login flow. LimeOS displays only safe status and validated login destinations; it does not expose authentication tokens in chat or logs. Do not reinterpret subscription OAuth credentials as a generic API key.

As checked on 2026-10-04, Anthropic's support guidance says previously announced subscription changes were paused and subscription access for the relevant third-party agent integrations remains available under subscription limits. Recheck the supported integration route and terms before shipping; encapsulation allows a runtime replacement without changing application tools. [Anthropic subscription guidance](https://support.claude.com/en/articles/15036540-use-the-claude-agent-sdk-with-your-claude-plan)

Report availability and limits only to the extent the runtime actually exposes them. Unknown remaining quota is unknown, not unlimited. A subscription has a separate call/time/concurrency allowance; do not advertise a zero-cost API rate or pretend its included tokens are a known monetary balance.

## 5. Deterministic routing and escalation

Start with configured rules and measured qualifications, not an LLM deciding which LLM should run. Task classification uses the known workflow and user intent; an ambiguous request can ask for clarification before granting any operation scope.

For each turn:

1. Resolve principal, task profile, disclosure policy, authority, and user model preference.
2. Filter deployments by enabled connection, health, privacy/region/endpoint policy, required capabilities, context/output bounds, and qualification for that profile.
3. Check subscription allowance or reserve the worst-case API spend atomically in core.
4. Select the user's pinned deployment or the first eligible entry in the configured profile order. Cost optimization may rank qualified entries using measured cost per successful task and latency, not token price alone.
5. Keep that deployment for the turn. Escalate or fail over only at a persisted safe checkpoint with no unresolved dispatch or partial tool response.
6. Verify application actions independently; model self-reported confidence is neither verification nor authority.

Default profiles:

| Profile | Intended work | Routing policy |
|---|---|---|
| Subscription preferred | Normal conversation and qualified application workflows. | Claude subscription first; paid fallback disabled until explicitly configured. |
| Economy | Simple sourced answers, summaries, straightforward media requests. | Cheapest qualified deployment within latency and privacy requirements. |
| Standard | Incident investigation and multi-step application workflows. | Qualified stronger reasoning/tool use, bounded step count. |
| Deep analysis | Difficult diagnosis or a reviewable code proposal. | Explicit higher budget; no additional host authority. |
| Local only | Sensitive local questions or offline use. | Approved local endpoints only; fail clearly when insufficient. |

Do not hard-code a provider's current model ranking into the architecture. Model deployments are configuration with recorded capabilities, rates, and evaluation results. A new model can be registered without adding route-handler conditionals; a new protocol still needs a reviewed adapter.

Suggested initial bounds: eight model rounds per turn, sixteen tool calls total, three minutes wall time for interactive diagnosis, and bounded evidence/tool-result bytes. Long media downloads run as application jobs, not a model loop held open for hours. A task-specific profile may adjust these limits deliberately.

Failure handling is explicit: authentication failure, rate/quota limit, context overflow, unavailable endpoint, timeout, invalid tool schema, unsupported capability, cancellation, and unknown request outcome. Honor retry-after where exposed. Permit at most one automatic transient request retry initially; do not retry rejected credentials, invalid schemas, or unknown mutation outcomes. Cap the full attempt sequence with the task's original deadline and budget.

Failover cannot circumvent a refusal to disclose data, an application permission denial, or an approval requirement. It also cannot resend a completed tool effect because a more expensive model wants to start again.

## 6. Spending and usage accounting

Core owns daily/monthly installation and per-principal API budgets, per-task caps, and a separate background-automation allocation. A conversation projection can display usage but cannot authorize spend.

Before each paid request, reserve an upper-bound estimate using the selected price revision, input estimate with safety margin, maximum billable output/reasoning units, and any applicable tool or request fees. Use integer minor/microcurrency units and explicit rate units. Do not assume prompt-cache discounts. If the provider cannot offer enforceable output bounds or a usable upper estimate, exclude it from a strict-budget profile.

Reconcile the reservation with reported usage and retain price revision, deployment, cache status, estimate method, and uncertainty. Avoid counting reasoning tokens twice when a provider already includes them in another billed field. For a timed-out request with unknown charge, retain a conservative reservation until reconciled or explicitly resolved; do not immediately release it and issue unlimited retries.

Budgets limit requests issued by LimeOS. They cannot guarantee the exact external invoice because pricing, delayed reporting, fees, and other clients on the same account can differ. Expose estimated versus reported totals; recommend provider-side spending limits as a second control where available. Subscription consumption is reported separately from API currency spending.

All fallback destinations and spending ceilings must be configured by an administrator. A user can opt into a task's larger permitted budget; a model cannot increase it. Price refreshes are versioned configuration changes with alerts for stale or missing rates.

## 7. Permissions, evidence, and conversation privacy

Core issues an expiring task token, an opaque value recorded in core's database, containing principal, permitted operations/resources, grant revision, conversation scope, and task limits. The assistant and any chat adapter cannot expand it. Approval is a core record bound to a precise plan, confirmed by an authenticated authorized actor; model text or an unverified chat reaction is not approval.

Documentation retrieval starts with release-versioned files and full-text search. Add embeddings only if an evaluation demonstrates a useful recall improvement. Answers distinguish current observations, documentation, and assumptions, and cite the relevant evidence. Do not send an entire logfile, secret-bearing environment, or home directory because a model asks for more context.

Redact credentials and identifiers as appropriate before model disclosure. Register endpoints with TLS validation, redirect restrictions, and private-address/SSRF policy; local service endpoints require explicit registration rather than unrestricted URL access. Evaluate whether an external provider is approved for each evidence class. Never fall back from local-only to cloud because a local model is slow.

Conversations are scoped to the principal and explicitly shared participants. External transports map immutable, verified account IDs to local principals. A channel membership or display name does not imply operator permissions. Disconnecting an integration revokes its mapping/tokens and stops further delivery. Unknown external actors receive no application access.

Separate user preferences from system facts. Remember library/profile preferences only with user consent; do not promote a conversation summary into policy. Retention, deletion, export, and safe audit retention are explicit. Provider-side storage and logging settings are recorded per connection; local deletion cannot be represented as erasure of every provider copy.

## 8. Three primary workflows

Explaining and maintaining ship in the cutover release; adding media follows in P07.

### Explain and investigate

The user asks why Jellyfin is unavailable. The assistant reads scoped observations, recent bounded logs, mount state, and release documentation. It reports evidence and uncertainty, proposes a registered repair if appropriate, obtains authorization under the configured grant, and verifies recovery. If the cause is unknown, it says so rather than repeatedly restarting services.

### Add a movie or television series

Search the registered Radarr/Sonarr connector; identify the title by stable TMDB/TVDB identity and clarify an ambiguous match. Core selects an approved root folder, quality profile, and season-monitoring policy from validated user preferences. Show a compact preview when clarification is needed, then create a durable idempotent request under the user's media grant.

Handle already-present titles, selected seasons, disconnected services, profile changes, lost downloads mounts, search failure, and timeout after a successful add. Query actual service state before retrying. Report distinct states: requested, searching, downloading, imported, and available in the library. Do not claim availability merely because an add endpoint returned success. The user can subscribe to progress without holding an LLM turn open.

Connector schemas are verified against installed supported versions during implementation. These services remain the source of truth for their own library/download state. [Radarr API documentation](https://radarr.video/docs/api/), [Sonarr API documentation](https://sonarr.tv/docs/api/)

### Maintain the application

For a broken container, diagnose, plan a bounded restart, execute only within the grant, and verify. For a LimeOS update, inspect the signed release and run the ordinary update job. For a code defect, create a sandboxed reproduction and reviewable patch through a separate development workflow. Installing unreviewed code into the live privileged application is never the assistant's repair mechanism.

Scheduled maintenance, after cutover in P09, uses explicit runbooks, resource scopes, maintenance windows, cooldowns, attempt caps, and automatic stop conditions. Deterministic incident detection can request a budgeted diagnosis; monitoring does not require continuous model calls.

## 9. Model qualification and rollout

Qualify each deployment per task profile with a versioned dataset and reproducible settings. Include ambiguous titles, duplicates, tool errors, parallel reads, provider-native continuation, long contexts, prompt injection in logs, missing permissions, stale approval, revoked grants, wrong mount identity, quota failure, cancellation, and fallback after a completed side effect.

Measure task success, verified factual accuracy, schema errors, unsafe proposals, latency, total billable usage, and cost per successful task. Human review uses an evidence-based rubric for diagnosis; a model's own grading is insufficient. Record dataset size and uncertainty. A passing handful of prompts is not proof of safe behavior.

Admission requires all deterministic authorization, isolation, no-duplicate-effect, and budget tests to pass. Profile-specific quality/latency thresholds are set in P05/P08 from representative scenarios and recorded before model comparison. An unsafe proposal must be rejected by core even from a qualified model; model quality is not the security boundary.

Use recorded protocol fixtures in routine CI. Live-provider tests are opt-in, isolated, and capped, using dedicated accounts; never make an uncapped paid test suite. After a provider/model/runtime change, rerun conformance and affected profile evaluations before enabling it. Roll back the routing configuration independently of host/application releases.

The first shipped assistant must pass subscription and Anthropic API conformance, native UI setup, permission and budget checks, evidence-based diagnosis, and restart recovery. OpenAI, Gemini, compatible endpoints, and local routing expand this foundation after cutover; they must not delay delivery of a coherent subscription workflow.
