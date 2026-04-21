# exVMs Implementation Plan

## Summary

Build a Rust CLI product named `exVMs` with the command `exvms` for exporting recordings from Voice Memos.app. Keep `src/main.rs` as the CLI layer and keep database, storage discovery, export planning, macOS file handling, path sanitizing, and time conversion in the Rust library.

The implementation targets these Voice Memos database stores:

- `CloudRecordings.db` / `ZCLOUDRECORDING`, including title fallback from `ZENCRYPTEDTITLE` / `ZCUSTOMLABEL` and optional `ZLOCALDURATION`
- `Recordings.db` / `ZRECORDING`
- `ZFOLDER` / `ZENCRYPTEDNAME` when available
- `ZDATE` uses Apple/Core Data reference time, so Unix time is `ZDATE + 978307200`.
- `view` exposes three user-facing classifications:
  - `store`: `cloud-recordings` or `local-recordings`
  - `path`: `relative`, `absolute`, or `missing`
  - `availability`: `local`, `needs-download`, or `outside-recordings-dir`
- Relative `ZPATH` values resolve under the Voice Memos Recordings directory; absolute `ZPATH` values are safely mapped back into that selected directory when possible.

## CLI Behavior

- `exvms view` prints the active storage path, database path, and a metadata table with stable ID, title, recording time, duration, availability, store, path, folder, and database path.
- `exvms export --output <DIR> --id <ID> [--id <ID>...]` exports explicit recordings by stable `Z_PK` ID.
- `exvms export --output <DIR> --all` exports all local-exportable recordings and reports known `needs-download` recordings as skipped.
- `exvms export --output <DIR>` asks once for confirmation; only exact `yes` proceeds with all-recordings behavior.
- `exvms export ... --jobs <N>` controls bounded parallel export. The default is `min(4, available_parallelism)`.

## Export Rules

- Explicit `--id` export is all-or-nothing during preflight:
  - every requested ID must exist;
  - no requested ID may be `needs-download`;
  - no target may already exist;
  - no two selected recordings may resolve to the same target;
  - no source path may resolve to `outside-recordings-dir`.
- Full export skips known `needs-download` recordings and reports them at the end.
- Unexpected per-file failures after preflight do not stop the whole batch; the final summary reports successes, skipped `needs-download` IDs, and failures.
- Exported filenames come from the sanitized title fallback chain plus the source extension when needed.
- If a recording has `ZFOLDER`, export it under a sanitized `ZFOLDER.ZENCRYPTEDNAME` subdirectory.
- Do not overwrite existing target files.
- After copying, set modification time and Finder creation time to `ZDATE`.

## iCloud / Cloud-Only Handling

The tool treats a missing database path, missing local audio file, or 0-byte local audio file as `needs-download`. Voice Memos CloudKit placeholders do not expose a public standard iCloud Drive download path for this tool, so unavailable recordings are reported with a message telling the user to open/download them in Voice Memos.app before retrying. Paths that resolve outside the selected Recordings directory are reported as `outside-recordings-dir` and are not exported.

## Test Plan

- Unit tests for path discovery order, filename sanitizing, Apple date conversion, duration formatting, target collision detection, no-overwrite preflight, and explicit-ID validation.
- SQLite fixture tests matching `CloudRecordings.db` and `Recordings.db`, with foldered recordings, duplicate sanitized names, missing/0-byte source files, missing paths, absolute paths, mounted-backup path remapping, and `--all` `needs-download` skipping.
- CLI smoke tests:
  - `exvms view` prints the storage path and metadata table.
  - explicit `needs-download` ID export exits before copying anything.
  - explicit local ID export copies the file and preserves creation/modification time.

## Current FFI Status

No C ABI is currently exposed. The Rust library is available to Rust callers through the `exvms` crate modules. A Swift/macOS app should currently either shell out to the `exvms` binary or add a future FFI layer with a `cdylib`, `extern "C"` functions, C-compatible structs, and generated Swift bindings.
