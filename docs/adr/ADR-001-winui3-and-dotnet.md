# ADR-001: WinUI 3 and .NET 10 Platform Selection

## Status
Accepted

## Context
The EF Assistant application is a native Windows desktop assistant designed for neurodivergent adults. The core requirements dictate:
1. Native UI Automation and comprehensive accessibility (Narrator, NVDA, screen readers, keyboard navigation).
2. Spatial stability, low-latency overlay rendering (< 150 ms cold from tray), and non-blocking background operations.
3. Native Win32 interop for global hotkeys, activity monitoring, and window focus management.
4. Long-term maintainability, MSIX packaging for Microsoft Store distribution, and modern C# source-generation tooling.

## Decision
We select **WinUI 3 on Windows App SDK (2.x)** running on **.NET 10 (LTS)** as the primary UI and runtime framework.

Key architectural specifics:
- Use C# latest with nullable reference types enabled and warnings treated as errors.
- Adopt `CommunityToolkit.Mvvm` with source generators for clean MVVM architecture.
- Confine Win32 interop and hooks to `EfAssistant.Activity` using `Microsoft.Windows.CsWin32` source-generated P/Invoke bindings.
- WinUI 3 2.x breaking changes from 1.x are adhered to: ignore deprecated `Window.Current`, `DependencyObject.Dispatcher`, and `FocusManager.GetFocusedElement`.

## Consequences
- **Positive**: First-class UI Automation and accessibility tree integration out of the box. High performance, native XAML rendering, source generator support across JSON serialization, MVVM, and Win32 P/Invoke.
- **Negative**: Windows-only client. Requires MSIX packaging tooling and careful handling of WinUI window handles (`HWND`) for overlay focus semantics.
