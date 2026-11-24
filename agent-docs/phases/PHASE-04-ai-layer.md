# Phase 04 - AI layer

Read `../CONTEXT.md` first, especially Section 6 invariants.

## Goal

Implement `ef-ai`, `ef-ai-transport`, `ef-prompts`, `ef-gateway`, deterministic date parsing, and F2 structuring. Every downstream AI feature reuses this pipeline unchanged.

Estimated two and a half weeks.

## Prerequisites

Phase 01 (`llm_call`, `response_cache`, `OpLog::apply`) and Phase 03 (structuring queue).

## Tier model

Four tiers. Always pick the lowest tier meeting acceptance criteria:
- **T0**: Deterministic Rust + pure-Rust ML (<50 ms, $0): estimation, ranking, stall detection, retrieval, date parsing.
- **T1**: Local LLM ($0, 1.5–3.5s p95): Foundry Local (if HTTP endpoint detected, validate-and-fallback primary because `json_schema` is undocumented) → bundled `llama-server` sidecar (with `schemars` GBNF/`json_schema` constrained decoding). Phi-4-mini 3.8B (MIT) default, Qwen 3.5 4B (Apache 2.0) alternative.
- **T2**: Small cloud model, Haiku class (~$0.003–0.0045/call, 1–2.5s): quality-sensitive structuring and long input.
- **T3**: Frontier cloud model, Sonnet class (~$0.010/call, ~$0.005 batch, 3–8s): rare multi-step planning and weekly narrative.

## Scope

### 1. Router (`ef_ai::Router`)

`ef_ai::Router` is the sole selector of tier and model. `clippy.toml` (`disallowed-types`) prevents `reqwest::Client` outside `ef-ai-transport` and `ef-gateway`, and `ef-ai-transport` requires `RouterDispatchToken` (constructed only by `ef_ai::Router`).

Feature routing & output caps:
- `braindump.structure`: T1 default → T2 if T1 unavailable, input > 800 tokens, or 1 validation failure. Never T3. Cap: 400.
- `task.decompose`: T1 (gran ≤ 3) → T2 if gran ≥ 4, retrieved context present, or 1 failure → T3 only if user explicitly asks for a full plan spanning > 3 days. Cap: 500 (gran ≤ 3) / 2000 (gran ≥ 4).
- `task.resplit`: T1 → T2 on 1 failure. Cap: 250.
- `obstacle.phrase`: T1 (T2 if unavailable). Cap: 60.
- `obstacle.interpret`: T1 → T2 if answer > 200 tokens. Cap: 120.
- `transcript.summarize`: T2 always. Cap: 800.
- `review.narrate`: T3 Batch API. Cap: 600.
- `nudge.rephrase`: T1 only; never escalate (skip call if unavailable). Cap: 40.
- Estimation, ranking, stall detection, retrieval, date parsing: T0 always.

Hard router rules:
- `CloudAiMode::Never`: T2/T3 unreachable; degrade cleanly.
- `CloudAiMode::AskEachTime`: one consent prompt per session per feature (with "remember for this feature"); background jobs park as `pending_consent`.
- Monthly spend cap breached: disable T2/T3, notify once, continue locally.
- Two consecutive validation failures at any tier → deterministic fallback (never a 3rd attempt).

### 2. Cloud gateway (`gateway/ef-gateway`)

Separate `axum` service sharing wire types via `ef-protocol`. Holds provider API keys, authenticates client, enforces per-user monthly spend cap, applies prompt caching, logs token/cost usage, and connects to a zero-retention provider agreement before beta.

### 3. Prompt contracts (`ef-prompts`)

Versioned prompt assets included from `crates/ef-prompts/prompts/` (never inline strings in application code), each covered by golden tests.

Cache-prefix-first layout:
1. Static system rules + `schemars` output contract.
2. Style rules + 2 few-shot examples matched to the user's `cluster_id` (start with 6 clusters by folding `Neutral` into `Direct` and enforcing literal style in the validator).
3. Dynamic suffix: retrieved fragments tagged with `fragment_id`, then task/raw text.

Resolve ADR-003 (Haiku 4096-token padding vs Sonnet 1024-token prefix economics) by measurement before beta, keeping a compact ~500-token prompt variant for T1 local inference.

### 4. Output schemas & 7-stage validator

`ef-prompts` output structs derive `Serialize`, `Deserialize`, `JsonSchema` (`schemars`) and strictly forbid date, time, duration, deadline, or numeric priority fields (enforced by a schema-inspection unit test). Only `temporal_phrase: Option<String>` and `confidence: f32` are permitted.

Validator pipeline (in order):
1. `stop_reason` (`MaxTokens` or `Refusal` routes straight to deterministic fallback without consuming retry).
2. Schema validity.
3. Grounding (cited `fragment_id`s must exist in prompt context).
4. Step shape (verb-initial, no compound "and then", within `StepVerbosity` word limit).
5. Unknown tools (no app/file absent from input or context).
6. Style (`DemandStyle::Invitational` forbids imperatives).
7. Banned vocabulary check.

### 5. Deterministic date parsing, response cache & F2 structuring

- Deterministic temporal parser (`chrono` + `chrono-tz`) converts `temporal_phrase` to `due_at`/`due_precision` or `starts_at`.
- `response_cache` keyed on `(feature, prompt_ver, profile_hash, normalized_input, granularity)` with 30-day TTL.
- Record all calls in `llm_call` (`feature`, `tier`, `model`, `prompt_ver`, token counts, cached tokens, latency, cost, `stop_reason`, `truncated`, `valid`, `retried`).
- F2 candidate pipeline: `accepted_at` stays `None` until explicit user accept (`UserAcceptedProof`), raw text stays pinned beside candidates, and `CloudAiMode::Never` degrades to a deterministic clause-splitter heuristic.

## Out of scope

No task decomposition UI (Phase 05), no vector embeddings (Phase 02 post-MVP / Phase 05+ retrieval), no local model inference shipping requirement (MVP is cloud-first with T1 path tested).

## Definition of done

- `clippy.toml` + `RouterDispatchToken` statically block model HTTP calls outside `ef-ai-transport`.
- Schema unit test confirms zero temporal/numeric fields in any `schemars` schema.
- `stop_reason` truncation/refusal bypasses retry and hits deterministic fallback.
- Golden tests pass for every prompt version; 2-strike rule and all 7 validators have adversarial unit tests.
- F2 structuring works end-to-end and degrades gracefully to deterministic clause splitting offline.
