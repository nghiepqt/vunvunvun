# ADR-002: Sync Conflict Resolution Shape and Hybrid Logical Clocks

## Status
Accepted

## Context
Multi-device sync will ship in a future phase, but the underlying data model, schema conventions, and conflict resolution mechanics must be established from Migration 0001 to prevent breaking data migrations after user adoption.

Key constraints:
1. No central server or account is required for the local MVP, but all syncable writes must generate an immutable audit trail.
2. Clocks across physical devices experience drift; wall-clock timestamps cannot guarantee causality or monotonic ordering.
3. Certain columns (e.g. `raw_text` in `inbox_item`) are immutable by design to preserve user intention.

## Decision
1. **Identifiers**: Client-generated UUIDv7 text identifiers across all syncable entities to guarantee time-locality and zero ID collisions.
2. **Op-Log Architecture**: All writes to syncable tables (`task`, `note`, `inbox_item`, `user_profile`, `transcript`, `project`) are recorded in an append-only `op_log` table. Device-local tables remain separate and un-synced.
3. **Ordering**: Hybrid Logical Clocks (HLC) with a bounded drift check (e.g. max 60 seconds) provide deterministic causal ordering.
4. **Conflict Resolution**: Last-Write-Wins (LWW) per field based on HLC timestamps with client ID tie-breaking. Variable-length base-62 fractional indexing with device ID tie-breaker for item reordering.

## Consequences
- **Positive**: Clean separation between syncable user data and machine-local runtime telemetry/caches. Eliminates merge collisions on ordered lists.
- **Negative**: Adds op-log write amplification for local updates; requires strict encapsulation in `ef_oplog::OpLog`.
