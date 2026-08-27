# ADR-003: Prompt Caching and Prefix Alignment Strategy

## Status
Proposed (Stubbed for Phase 04)

## Context
Model inference occurs across two tiers:
1. Cloud tier via proxy gateway.
2. Local sidecar (`llama-server`) hosting small quantized models (e.g. Phi-4-mini / Qwen 3.5).

To satisfy latency budgets (< 800 ms to first token) and minimize cost (< $2/user/month in cloud, < $0.25 local), prompt prefixes (system instructions, tool definitions, static context) must leverage prompt caching.

## Decision
1. Prompts are treated as versioned static asset files rather than inline runtime strings.
2. Structured prompt construction places static invariants (system prompts, JSON schemas, base instructions) strictly at the prefix, followed by clustered user preferences, and finally the dynamic user capture text.
3. User profile preferences are hashed (`profile_hash`) and clustered (`cluster_id`) so that minor preference edits that do not alter the system prompt keep cache hits intact.
4. For local `llama-server`, prompt cache slots are managed via server lifecycle flags (`--slots`, `--sleep-idle-seconds -1`).

## Consequences
- **Positive**: Maximizes prefix cache hit rate on both cloud providers and local sidecar KV caches.
- **Negative**: Dynamic context cannot be arbitrarily prepended; prompt templates must follow strict segment ordering.
