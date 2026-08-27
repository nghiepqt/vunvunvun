# Phase 04 - AI layer

Read `../CONTEXT.md` first, especially the architecture invariants. Most of them live in this phase.

## Goal

Build the model routing layer, the cloud proxy, the prompt contracts, and the output validators, then use them for F2 structuring. Everything about how this product talks to a model is decided here, and every later feature reuses it unchanged.

Estimated two and a half weeks.

## Prerequisites

Phase 01 for storage. Phase 03 for the structuring queue that will call this.

## Tier model

Four tiers. Always use the lowest tier that meets the acceptance criteria for a feature. Raising a feature's tier requires editing the phase document, not a judgment call at the keyboard.

Tier 0 is deterministic code and small ONNX models. Free, under 50 ms, used for anything numeric, any ranking, any detection, all retrieval, and all date parsing.

Tier 1 is an on-device language model, free, one and a half to three and a half seconds, used for private, offline, high-frequency work with short output.

Tier 2 is a small cloud model in the Haiku class, roughly three tenths of a cent per call at the output caps below, one to two and a half seconds, used for quality-sensitive structuring and long input.

Tier 3 is a frontier cloud model in the Sonnet class, roughly one cent per call, three to eight seconds. Rare, and only for genuinely multi-step planning.

Cost figures come from the output caps, because output tokens cost roughly five times input tokens. Structuring at four hundred output tokens on a Haiku-class model is already two tenths of a cent in output alone before any input. Do not plan with lower numbers.

### Tier 1 provider ladder

Try in order, first available wins.

1. Foundry Local, if installed. It is generally available, exposes an OpenAI-compatible chat completions endpoint, and falls back across GPU, neural processing unit, and CPU automatically. Its HTTP server is optional configuration rather than automatic, so detect it rather than assuming it.
2. The bundled `llama-server` sidecar with a model downloaded on demand after explicit opt-in.
3. Unavailable, in which case fall through to tier 2, or to deterministic behavior if the user has disallowed cloud.

Windows App SDK 2.3.1 added a structured JSON output API for language-model responses. If that turns out to give schema-constrained output on hardware that is actually widespread, it changes the assessment below, so verify it in the phase 00 spike rather than assuming either way.

Windows AI language model APIs are otherwise deliberately not a rung. The Phi Silica generation is deprecated with removal scheduled before this product's local-inference phase lands, it is a limited-access feature requiring an unlock token, and its discrete-GPU path is experimental. Re-evaluate its successor when it ships; assume nothing until then.

The rungs are not interchangeable for the thing that matters most. Chat completion shape is common, but grammar-constrained decoding is llama.cpp specific and Foundry Local has no documented schema-constrained output. On rung 1 the validate-and-fall-back path is the primary correctness mechanism, not a backstop.

### Bundled model

Default to Phi-4-mini 3.8B, MIT licensed. Qwen 3.5 4B under Apache 2.0 is the alternative and the only option with a small sibling should speculative decoding ever be wanted. Prefer permissive licensing; Gemma ships under a custom agreement whose prohibited-use policy must be contractually passed on to your own users, and Llama carries a monthly-active-user clause and a naming requirement.

## Scope

### 1. Router

`EfAssistant.Ai.Router` is the only place a model or tier is chosen. Feature identifiers and their routing:

- `braindump.structure`: tier 1 by default. Escalate to tier 2 when tier 1 is unavailable, input exceeds eight hundred tokens, or validation failed once. Never tier 3. Output cap four hundred tokens.
- `task.decompose`: tier 1 for granularity 1 to 3. Escalate to tier 2 for granularity 4 or above, when retrieved context is present, or after one validation failure. Tier 3 only when the user explicitly asks for a proper plan and the task spans more than three days. Output cap five hundred tokens below granularity 4, two thousand at or above it.
- `task.resplit`: tier 1, escalate to tier 2 on one failure. Cap two hundred fifty.
- `obstacle.phrase`: tier 1, cap sixty.
- `obstacle.interpret`: tier 1, escalate on answers over two hundred tokens, cap one hundred twenty.
- `transcript.summarize`: tier 2 always, cap eight hundred.
- `review.narrate`: tier 3 through the batch endpoint, cap six hundred.
- `nudge.rephrase`: tier 1, cap forty. Never escalate; skip the call entirely instead.
- Estimation, ranking, stall detection, retrieval, and date parsing are tier 0 and never touch a language model.

The granularity-4-and-above cap is two thousand rather than something smaller because each step's JSON scaffolding costs roughly twenty-five tokens before any content, and twenty-five explanatory steps runs past sixteen hundred. Truncating there voids the schema guarantee and drops the single most important feature to a fallback. Measure the real number and adjust.

Hard router rules:

- If the user has set cloud to never, tiers 2 and 3 are unreachable. Degrade; do not re-prompt.
- If set to ask each time, one consent dialog per session per feature, with a remember option. Background jobs cannot prompt and park work instead.
- If the monthly spend cap is breached, disable tiers 2 and 3, notify once, and continue on the local tier.
- Two consecutive validation failures at any tier means stop and return the deterministic fallback. Never a third attempt.

### 2. Cloud gateway

A separate ASP.NET Core minimal API. Provider API keys live here and never ship to clients.

Responsibilities: authenticate the client, enforce a per-user monthly spend cap, forward to the model provider, apply prompt caching, and record usage. Provider switching happens here without a client release.

Negotiate a zero-retention arrangement with the model provider before beta. The content flowing through this proxy describes users' work, difficulties, and neurotype.

### 3. Prompt contracts

Prompts are versioned files under `EfAssistant.Ai/Prompts/`, never inline strings. Every version has golden tests, and a change that alters golden output requires a version bump.

Structure every prompt cache-prefix-first:

The stable prefix contains system rules identical for all users, the output contract describing the schema, style rules derived from the user's preferences, and two few-shot examples matched to the user's preference cluster. The dynamic suffix contains retrieved fragments each tagged with its fragment identifier, then the task or raw text.

Only the style rules and examples vary per user, so bucket users into preference clusters stored on the profile row. Do the arithmetic deliberately: demand style has three values, language mode two, and step verbosity three, which is eighteen combinations, not eight. Eighteen clusters is faithful but warms each prefix with a small fraction of the user base. Collapsing to six by folding neutral demand style into direct and enforcing literal language in the validator instead gives much better cache economics with slightly weaker steering. Start with six while the user base is small.

The caching decision itself is ADR-003 and must be resolved in this phase. The natural prefix runs nine hundred to fourteen hundred tokens. That clears a Sonnet-class minimum of about one thousand tokens but falls far short of a Haiku-class minimum of about four thousand, and a prefix below the minimum is simply not cached with no partial credit. The two options are padding to four thousand with examples that genuinely improve output, which only pays off above roughly eight to ten calls per prefix per hour across a cluster because cache writes cost double, or using a Sonnet-class model for the two high-volume features and not padding at all. At low user counts the second is usually cheaper and always simpler. Measure before committing.

Note also that a four-thousand-token cloud prefix cannot be shared with the local tier. Maintain two variants: a compact one of roughly five hundred tokens for local, and whichever cloud variant the decision selects.

### 4. Output schemas

Structuring returns candidate items, each with a type, text, a confidence score, cited fragment identifiers, and the user's temporal expression copied verbatim.

Decomposition returns ordered steps, each with text, a flag for whether it is a physical action, cited fragment identifiers, and a blocker risk rating; plus a top-level list of clarifications needed and a confidence score.

No schema anywhere contains a date, time, duration, deadline, or numeric-estimate field. This is enforced by a test that inspects the schema files. A confidence score is explicitly allowed, because it is a model self-assessment rather than a claim about the world.

### 5. Validators

Run in this order. Each is cheap and they compound.

1. `stop_reason`. If generation hit the token cap or the model refused, route straight to the deterministic fallback. This does not count as a validation failure and does not consume the retry. Record it.
2. Schema. Guaranteed by constrained decoding for complete generations, checked anyway.
3. Grounding. Every cited fragment identifier must exist and must actually have been in this prompt. Drop context-dependent claims that cite nothing.
4. Step shape. Verb-initial, no compound "and then", word count within the user's verbosity preference.
5. Unknown tools. No step may reference an application or file that appeared neither in the input nor in retrieved context.
6. Style. Phrasing matches the user's demand style; no imperatives when the preference is invitational.
7. Banned vocabulary, as listed in the context file.

A failure gets one retry with the error appended. A second failure returns the deterministic fallback.

### 6. Deterministic date parsing

The model copies the user's phrase verbatim. A parser resolves it against the user's time zone and writes `due_at` and `due_precision`, or `starts_at` for events. This code owns every date in the system.

### 7. Response cache

Key on feature, prompt version, profile hash, normalized input, and granularity. Granularity is required; without it a coarse and a micro decomposition of the same title collide, and users repeat task titles constantly. Thirty-day time to live, invalidated on prompt version bump. Expect a twenty to thirty-five percent hit rate.

### 8. Usage recording

Record per call: feature, tier, model, prompt version, input tokens, cached tokens, output tokens, latency, cost, stop reason, whether truncated, whether valid, and whether retried. Dashboard cost per active user weekly.

### 9. F2 structuring

Wire the pipeline: load the raw text, retrieve up to three related fragments by keyword search, build the prompt, call through the router with a constrained schema, run the validators, resolve dates deterministically, and present the results as candidates with `accepted_at` still null.

Users whose executive-function profile includes working memory difficulty see the original raw text pinned beside the candidates, so nothing feels lost.

A dump producing no confident candidates returns the clarification list rather than inventing items.

## Out of scope

No decomposition; that is phase 05. No vector retrieval; keyword search only for now. No local model tier; the MVP is cloud-first and local inference is post-MVP. Ship the deterministic heuristic splitter described below so that cloud-refusing users are not left with an empty screen.

## The deterministic fallback

Every model-dependent path needs one, and users who disallow cloud entirely will live on it during the MVP. For structuring it is clause segmentation into candidate items with light normalization. It is worse than the model. It is not nothing, and it keeps the promise that the application degrades rather than blocks.

## Definition of done

- Every model call goes through the router; an architecture test enforces it.
- No output schema contains a temporal or numeric-estimate field; a test enforces it.
- `stop_reason` is checked on every call and truncation routes to the fallback without consuming a retry.
- Golden tests exist for every prompt version.
- The two-strike rule is enforced and tested.
- The banned-vocabulary CI check runs against the localization catalog with an allowlist and does not run against identifiers.
- Cost per call is recorded and the spend cap actually stops tier 2 and 3 traffic.
- Structuring works end to end from a captured item to reviewable candidates.
- With cloud disallowed, structuring falls back to the heuristic splitter and says so in plain language.
- ADR-003 is written and the caching approach is chosen with a measured reason.
