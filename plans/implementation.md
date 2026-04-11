# exVMs Implementation Plan

## Summary

Build a Rust CLI product named `exVMs` with the command `exvms` for exporting recordings from Voice Memos.app. Keep `src/main.rs` as the CLI layer and keep database, storage discovery, export planning, macOS file handling, path sanitizing, and time conversion in the Rust library.

The implementation targets the verified Voice Memos database schema:

- `ZCLOUDRECORDING`: `Z_PK`, `ZENCRYPTEDTITLE`, `ZDATE`, `ZDURATION`, `ZLOCALDURATION`, `ZEVICTIONDATE`, `ZPATH`, `ZFOLDER`
- `ZFOLDER`: `Z_PK`, `ZENCRYPTEDNAME`
- `ZDATE` uses Apple/Core Data reference time, so Unix time is `ZDATE + 978307200`.
- `ZPATH` is treated as a path relative to the Voice Memos Recordings directory.

## CLI Behavior

- `exvms view` prints the active storage path, database path, and a metadata table with stable ID, title, recording time, duration, local/cloud status, folder, and relative path.
- `exvms export --output <DIR> --id <ID> [--id <ID>...]` exports explicit recordings by stable `Z_PK` ID.
- `exvms export --output <DIR> --all` exports all local-exportable recordings and reports known cloud-only recordings as skipped.
- `exvms export --output <DIR>` asks once for confirmation; only exact `yes` proceeds with all-recordings behavior.
- `exvms export ... --jobs <N>` controls bounded parallel export. The default is `min(4, available_parallelism)`.

## Export Rules

- Explicit `--id` export is all-or-nothing during preflight:
  - every requested ID must exist;
  - no requested ID may be cloud-only or incomplete;
  - no target may already exist;
  - no two selected recordings may resolve to the same target;
  - no source path may escape the Recordings directory.
- Full export skips known cloud-only/incomplete recordings and reports them at the end.
- Unexpected per-file failures after preflight do not stop the whole batch; the final summary reports successes, skipped cloud-only IDs, and failures.
- Exported filenames come from sanitized `ZENCRYPTEDTITLE` plus the source extension when needed.
- If a recording has `ZFOLDER`, export it under a sanitized `ZFOLDER.ZENCRYPTEDNAME` subdirectory.
- Do not overwrite existing target files.
- After copying, set modification time and Finder creation time to `ZDATE`.

## iCloud / Cloud-Only Handling

The tool treats a missing or 0-byte local audio file as not exportable unless macOS Foundation identifies it as a downloadable ubiquitous item. `export` may make a best-effort call to `NSFileManager.startDownloadingUbiquitousItemAtURL:error:` for standard ubiquitous items.

Voice Memos CloudKit placeholders in the verified local database did not expose standard ubiquitous item metadata, so those files are reported as unavailable with a message telling the user to open/download them in Voice Memos.app before retrying.

## Test Plan

- Unit tests for path discovery order, filename sanitizing, Apple date conversion, duration formatting, target collision detection, no-overwrite preflight, and explicit-ID validation.
- SQLite fixture tests matching the verified schema with foldered recordings, duplicate sanitized names, missing/0-byte source files, and `--all` cloud-only skipping.
- CLI smoke tests:
  - `exvms view` prints the storage path and metadata table.
  - explicit cloud-only ID export exits before copying anything.
  - explicit local ID export copies the file and preserves creation/modification time.

## Current FFI Status

No C ABI is currently exposed. The Rust library is available to Rust callers through the `exvms` crate modules. A Swift/macOS app should currently either shell out to the `exvms` binary or add a future FFI layer with a `cdylib`, `extern "C"` functions, C-compatible structs, and generated Swift bindings.
