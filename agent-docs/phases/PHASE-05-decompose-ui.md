# Phase 05 - Decomposition and workspace

Read `../CONTEXT.md` first.

## Goal

Build the core product value in the ephemeral WebView2 UI (`ui/workspace`) backed by `ef-app`, `ef-ai`, and `ef-oplog`: turning any task into steps small enough to start at user-controlled granularity (1–5), plus the spatially stable Workspace (F11) and full Settings surface.

Estimated three weeks.

## Prerequisites

Phase 04 (router, prompts, validators) and Phase 01 (`task` self-referencing table + base-62 fractional ordering).

## Scope

### 1. Decomposition (F3)

- Granularity slider 1–5 initialized from `UserProfile.default_granularity[category]`:
  - Level 1: 3–5 steps (~1h each).
  - Level 3: 6–10 steps (10–30m each).
  - Level 5: 12–25 steps (10s–2m each, zero decision-making).
- Respects `require_physical_first_step`, `plan_visibility` (rendering filter only—full plan is always generated/stored), `demand_style` (enforced by validator), `step_verbosity`, and optional `anchoring_enabled`.
- Single-step re-split (`task.resplit`) scoped strictly to that step.
- Streaming output via typed IPC events from Rust `ef-ai` to WebView2 (`<800 ms` to first step, skeleton state before that).

### 2. Task & step persistence

- Tasks and steps share the `task` table (`parent_id`).
- Accepting proposals requires `UserAcceptedProof` from explicit user action in `ui/workspace` → `OpLog::apply`, setting `accepted_at`, `origin`, `origin_model`, and `origin_prompt_ver`.
- Reordering computes a single base-62 fractional index string with `device_id` tie-breaker.

### 3. Spatially stable Workspace (F11)

- Fixed-position blocks: **Now** (one task), **Today**, **Inbox**, **Parked**. User-arrangeable, never self-reordering or reflowing between sessions.
- Ephemeral WebView2 lifecycle: opening Workspace mounts `ui/workspace` and loads state over IPC; closing destroys WebView2 so resident tray RAM stays ~15–25 MB.
- Visual tokens from `ui/tokens/palette.json` (`standard` and `muted` intensities, light/dark) mapped to CSS variables (`--density`, `--motion`), with OS `prefers-reduced-motion` always winning when stricter.
- Deterministic color/label mapping per category (never AI-chosen).
- Virtualized scrolling at sustained 60 fps for 500+ items.

### 4. Settings surface

- Exposes every `UserProfile` preference with immediate effect (no Apply button, no restart).
- Plain-language explanation for `show_countdowns` (off by default—demand signal) and `show_elapsed_time` (separate toggle, off by default).
- Updating prompt-relevant preferences recomputes `profile_hash` and `cluster_id` locally via `ef-core`.

### 5. Inbox review (F2 UI)

- Accept, edit-then-accept, or discard candidates in one action; accepting a task can transition directly into decomposition.

## Accessibility requirements (EN 301 549 / WCAG 2.1 AA)

- Full keyboard operability, including arrow-key granularity slider announcing numeric level and meaning.
- Accessible names on every control and step ("Step 3 of 11").
- `aria-live="polite"` announces completion of streamed steps, never spamming token-by-token.
- Contrast >= 4.5:1 text and >= 3:1 components verified by `build/Test-Contrast.ps1` + `axe-core` in CI, plus manual NVDA and Narrator testing.

## Out of scope

No duration estimation display (Phase 06), no energy routing, no stall detection, no obstacle dialogue, no weekly review.

## Definition of done

- Step counts land within ±20% of level target; all steps are verb-initial without compound "and then".
- First streamed step visible within 800 ms; p95 < 2.5s on cloud tier.
- Workspace scrolls 500 items at 60 fps and restores identical layout after WebView2 close/reopen.
- Closing Workspace destroys `msedgewebview2.exe` cleanly.
- Entire capture-to-accepted-steps flow passes with keyboard + NVDA + Narrator alone.
