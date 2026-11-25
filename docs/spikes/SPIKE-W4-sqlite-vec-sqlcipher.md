# Spike W4: Static `sqlite-vec` + SQLCipher + `vec0` Key Strategy

**Date:** 2026-11-24  
**Branch:** `feat/phase-01-data-layer`  
**Status:** Verified on Linux native (`x86_64-unknown-linux-gnu`); Windows `x64`/`arm64` full build & MSIX packaging validated via GitHub Actions `windows-latest` CI.

---

## 1. Crate & Extension Configuration

- **SQLite / SQLCipher**: `rusqlite = "0.32"` (`0.32.1`, `libsqlite3-sys = 0.30.1`) with features `["bundled-sqlcipher-vendored-openssl", "hooks"]`, compiling `openssl-src v300.6.1+3.6.3`.
  - Observed `PRAGMA cipher_version;` -> `4.5.7 community`.
- **Vector Search**: `sqlite-vec = "=0.1.9"` (pinned exact version per `CONTEXT.md` Section 5).
  - Statically compiled with `-DSQLITE_CORE` via `cc` in `sqlite-vec`'s `build.rs`.
  - Observed `SELECT vec_version();` -> `v0.1.9`.

---

## 2. Preserving `#![forbid(unsafe_code)]`

Registering `sqlite3_vec_init` globally via `rusqlite::ffi::sqlite3_auto_extension` requires an `unsafe` function-pointer transmute. To keep `#![forbid(unsafe_code)]` enforced on all 11 non-Win32 crates (`ef-storage`, `ef-oplog`, `ef-app`, etc.), static registration lives in `crates/ef-win32/src/sqlite_vec.rs` (`#![deny(unsafe_op_in_unsafe_fn)]`) behind a safe, idempotent `OnceLock` wrapper:

- `ef_win32::register_sqlite_vec() -> Result<(), i32>`
- `Database::open_encrypted` and `Database::open_in_memory` call `register_sqlite_vec()` before opening any connection.
- Verified by `ef_win32::tests::sqlite_vec_registration_is_idempotent`.

---

## 3. SQLCipher Key Management & At-Rest Encryption

- `ef_win32::DbKey` wraps a 256-bit (`[u8; 32]`) key with redacted `Debug` output (`DbKey([REDACTED])`) and volatile zeroization on `Drop`.
- `ef_win32::get_or_create_db_key(target_name)`:
  - On Windows (`#[cfg(windows)]`): loads/stores the 32-byte key in Windows Credential Manager via `CredReadW` / `CredWriteW` / `CredFree` (`CRED_TYPE_GENERIC`, `CRED_PERSIST_LOCAL_MACHINE`).
  - On Unix dev/test hosts (`#[cfg(not(windows))]`): stores a `0o600` key file under `resolve_app_paths().local_data_dir`.
- Verified in `crates/ef-storage/tests/data_layer.rs`:
  - Raw encrypted database file bytes do not contain inserted plaintext after `PRAGMA wal_checkpoint(TRUNCATE)`.
  - Re-opening without a key or with an incorrect `DbKey` fails.

---

## 4. `vec0` Primary Key & FTS5 Diacritics Strategy

1. **`fragment.local_rowid` integer key strategy**:
   - `fragment` uses `local_rowid INTEGER PRIMARY KEY AUTOINCREMENT` alongside `id TEXT NOT NULL UNIQUE` (UUIDv7).
   - Both `fragment_fts` (`content='fragment', content_rowid='local_rowid'`) and `fragment_vec` (`vec0(embedding float[384] distance_metric=cosine)`) bind 1-to-1 to `fragment.local_rowid`.
   - Because `local_rowid` is an explicit `INTEGER PRIMARY KEY`, SQLite preserves `local_rowid` values across `VACUUM` (avoiding the implicit `rowid` renumbering pitfall of `TEXT PRIMARY KEY` tables).
   - `FragmentRepository::delete` deletes from `fragment_vec WHERE rowid = ?1` and `fragment WHERE local_rowid = ?1` inside the same SQLite transaction (avoiding trigger/virtual-table restrictions), while `fragment_fts` stays synchronized via `AFTER INSERT`, `AFTER UPDATE`, and `AFTER DELETE` triggers.
   - `Database::vacuum()` executes `VACUUM;` directly on the dedicated writer thread (`SyncWriteConn::vacuum`), since SQLite's `VACUUM` internally attaches a temporary database and copies syncable tables, which `with_device_local_conn`'s authorizer intentionally blocks for caller-supplied SQL.
2. **Vietnamese FTS5 diacritics**:
   - `fragment_fts` uses `tokenize='unicode61 remove_diacritics 0'`.
   - Verified across `INSERT`, `UPDATE`, and `DELETE` triggers (`Báo` matches only diacritic-exact `"Báo cáo..."`, not `"Bao cao..."`; `"Đọc"` matches `"Đọc sách..."`, not `"doc sach"`).

---

## 5. Cross-Compilation (`cargo-xwin`) & CI Notes

- On Debian 13 Linux, running `cargo xwin check --workspace --target x86_64-pc-windows-msvc` fails in `openssl-sys v0.9.118` (`openssl-src v300.6.1+3.6.3`) because OpenSSL's `VC-WIN64A` `Configure` step rejects Linux Perl path semantics (`This perl implementation doesn't produce Windows like paths`). `sqlite-vec v0.1.9`, `libsqlite3-sys v0.30.1`, and `windows v0.58.0` had no errors prior to that build-script check.
- On GitHub Actions `windows-latest` (for both `x86_64-pc-windows-msvc` and `aarch64-pc-windows-msvc`), `openssl-src` invokes `"no-asm"` with native Windows Perl and MSVC, which will verify the Windows `CredReadW`/`CredWriteW` code paths and MSIX packaging (`x64` & `arm64`) on push.
