# Phase 06 - Estimation and alpha release

Read `../CONTEXT.md` first.

## Goal

Add pure-Rust Tier-0 duration estimation (F8), finish packaging and compliance artifacts, and ship the alpha to 10–20 target users.

Estimated two weeks.

## Prerequisites

Phase 05.

## Part one: Pure-Rust duration estimation (`ef-ml`)

### Why Tier-0 and pure Rust

Language models must never invent durations or deadlines. M1 is a small LightGBM 3-head quantile regression model (p50, p80, p90) exported as a static JSON/binary tree artifact and evaluated in pure Rust inside `ef-ml` (<1 ms, zero network, zero external C++ ONNX DLL dependency for tabular trees).

Features: category one-hot, step word count, `is_physical`, position in plan, total steps, hour of day, day of week, optional self-reported energy level, EWMA of `actual_sec / est_sec` for this category, and prior completion count.

### Personalization without on-device tree retraining

1. Personal historical features fed into the frozen population trees.
2. Empirical Bayes blend: `w = n / (n + 15.0)` blending personal category statistic with population prediction.
3. Additive per-user quantile SGD residual correction in pure Rust, persisted in device-local `ml_state`.

### Display & calibration

- Always display as a range selected by `UserProfile.time_buffer_policy` (never a point estimate).
- Countdowns and elapsed-time displays remain off by default.
- Concrete anchors ("about as long as making coffee") allowed only when `LanguageMode::Natural`, never in `Literal`.
- Monitor empirical p80 interval coverage (target 75–85%) and surface calibration status in settings/diagnostics.

## Part two: Alpha release

### Packaging

Produce both channels from CI:
- **MSIX (`Vun_x64.msix`, `Vun_arm64.msix`)** built via `build/Pack-Msix.ps1`: primary Microsoft Store channel ($0 Microsoft re-signing, zero SmartScreen warnings).
- **Standalone `vun.exe`** (`x86_64-pc-windows-msvc`): unpackaged direct-download fallback (signed with OV certificate when distributing outside Store).

Verify clean installation on Windows 11 and Windows 10 22H2 (degraded cleanly where newer OS features are absent).

### Store submission & compliance

- Confirm `runFullTrust` approval on Partner Center (Hidden/Private Audience or Alpha flight).
- Run `Test-BannedVocabulary.ps1` and `check_policy.py` over `i18n/*.json`, `packaging/msix/AppxManifest.xml`, Store listings, screenshots, and website copy.
- Publish accurate privacy policy (GDPR Art. 9 local-first handling, explicit cloud consent, zero-retention provider DPA, full data export/delete including derived fragments and caches).
- Include in-app non-medical disclaimer and static human-written crisis support resource card.

### First-run, telemetry & alpha gate

- Useful immediately with zero account, tour, or permission wall—F1 capture works on launch.
- Ask cloud preference at first cloud-eligible action (`AskEachTime` default).
- Telemetry off by default (opt-in); never collect or display streaks, scores, peer comparisons, or productivity indices.
- Alpha gate with 10–20 neurodivergent users at week 2:
  - >= 60% still capturing in week 2.
  - Median hotkey-to-captured < 5s.
  - Decomposition accepted clean >= 50%.
  - Cloud cost <= $2/active user/month.
  - Zero crash loops, data loss, or accessibility blockers.

## Out of scope

Post-MVP features: full hybrid `sqlite-vec` retrieval (F5), energy routing (F6), stall detection (F7), obstacle dialogue (F4), meeting notes (F9), weekly review (F10), local LLM shipping (Phase 3 post-MVP), multi-device sync (Phase 5 post-MVP).

## Definition of done

- M1 runs in-process in pure Rust under 1 ms with zero network and zero LLM calls.
- All durations shown as ranges; p80 empirical coverage tracked.
- MSIX and standalone `vun.exe` install and run cleanly on Windows 11 and Windows 10 22H2.
- Store submission passes with `runFullTrust`; banned-vocabulary and contrast checks pass in CI.
- Alpha telemetry confirms retention and latency gates with 10–20 target users.
