# ADR-004: Rust Core and Split-Surface Hybrid UI (Option A)

## Status
Accepted (Supersedes [ADR-001](ADR-001-winui3-and-dotnet.md))

## Context
The EF Assistant ("Vun") is a resident Windows desktop cognitive-scaffolding assistant for neurodivergent adults. Its non-negotiable constraints create a tight three-way engineering trade-off:
1. **Legal Accessibility (EN 301 549 / WCAG 2.1 AA)**: Full NVDA and Narrator screen reader support, 100% keyboard operability, polite live-region announcements, 200% zoom, and reliable Vietnamese IME input (Unikey/EVKey/Windows Telex).
2. **Capture Latency**: F1 capture overlay must become visible and ready for typing in under 150 ms cold from the system tray.
3. **Resident Memory Budget**: Idle memory footprint in the tray (where the app spends ~90–95% of its day) must remain under 150 MB with no local model loaded.

Pure-Rust GPU-rendered UI frameworks (`gpui`, `egui`, `iced`) either lack production-ready Windows UI Automation (`ITextProvider`, live regions) or impose high risk around IME composition and accessibility compliance. Conversely, keeping a Chromium/WebView2 renderer resident 24/7 violates the 150 MB idle memory budget (~300–400 MB) and creating a fresh WebView2 window on hotkey exceeds the 150 ms F1 wake budget.

## Decision
We adopt **Rust (Edition 2024, stable toolchain)** for the entire workspace (`ef-*` crates + `ef-gateway`), paired with a **Split-Surface Hybrid UI (Option A)**:

1. **Resident Daemon, Tray, F1 Capture Overlay, and F7 Nudge Overlay — Native Win32 in Rust (`ef-win32`)**:
   - Built directly with Microsoft's `windows` crate using native Win32 windows, `MSFTEDIT_CLASS` (RichEdit) for text capture, and DWM Mica/Acrylic backdrops.
   - Wakes from the tray in ~5–15 ms, idles at ~15–25 MB RAM, provides exact `HWND` control (`WS_EX_TOOLWINDOW`, `WS_EX_NOACTIVATE`, `SetForegroundWindow` before hiding), and inherits native Windows UIA (`ITextProvider`), high contrast, and Vietnamese IME support.
2. **F11 Workspace, F2 Review, F3 Decomposition, and Settings — Ephemeral WebView2 (`wry` + `tao`)**:
   - Instantiated only when the user opens the main workspace window, and **explicitly destroyed (`.close()` / dropped)** when closed back to the tray so `msedgewebview2.exe` processes release memory and idle RAM returns to ~15–25 MB.
   - Provides Chromium's mature UIA accessibility tree, ARIA live regions, roving tabindex, CSS custom-property theming (`ui/tokens/palette.json`), `prefers-reduced-motion`, and automated `axe-core` verification in CI.
3. **Compile-Time Invariant Enforcement via Cargo Workspace & Clippy**:
   - `#![forbid(unsafe_code)]` on every crate except `ef-win32` (which enforces `#![deny(unsafe_op_in_unsafe_fn)]`).
   - `clippy.toml` `disallowed-types` forbids `reqwest::Client` outside `ef-ai-transport` and `ef-gateway`.
   - `ef-oplog` encapsulates `SyncWriteConn` privately so no other crate can issue raw `INSERT`/`UPDATE` statements against synced tables.
   - `ef-core` provides a sealed `UserAcceptedProof` token (non-deserializable) so AI responses can never auto-set `accepted_at`.
4. **Tier-0 ML & Persistence**:
   - `rusqlite` with `bundled-sqlcipher-vendored-openssl` (FTS5 + GDPR Art. 9 encryption at rest) and statically linked `sqlite-vec` (`sqlite3_auto_extension`).
   - Pure-Rust GBDT quantile evaluator for M1/M2 (<1 ms, zero C++ runtime) and deterministic LinUCB for M3; `tokenizers` + `candle` (or optional `ort` DirectML) for 384-dim `multilingual-e5-small` embeddings.
   - `llama-server` sidecar supervised under a Win32 Job Object (`JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` + memory cap).
5. **Packaging**:
   - Still packaged as **MSIX** (`MakeAppx.exe`, `runFullTrust`) for primary Microsoft Store distribution ($0 Microsoft re-signing, zero SmartScreen warnings), plus a standalone `vun.exe` direct-download channel.

## Consequences
- **Positive**: Meets <150 ms F1 wake (~5–15 ms) and <150 MB idle tray budget (~15–25 MB) simultaneously without sacrificing EN 301 549 accessibility or Vietnamese IME reliability. Compile-time isolation prevents entire classes of AI-agent regressions.
- **Negative**: Two UI technologies must be maintained (native Win32 overlays vs WebView2 workspace), and `ef-app` must cleanly bridge typed IPC state between the Rust core and ephemeral WebView2 frontend while ensuring WebView2 teardown is leak-free.
