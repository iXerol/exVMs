# exVMs

`exVMs` is a Rust CLI for viewing and exporting recordings from Voice Memos.app on macOS.

The tool reads the Voice Memos SQLite database from the active Recordings directory, lists recording metadata, and copies local audio files to a destination directory without overwriting existing files.

## Commands

Build or install the CLI first:

```sh
cargo build --release --bin exvms
```

Use `exvms` when it is installed on `PATH`; if the binary is in the current directory, use `./exvms` instead.

```sh
exvms view
```

Prints:

- active Voice Memos Recordings directory
- selected database path
- recording ID, title, recording time, duration, availability, store, path, folder, and database path

```sh
exvms export --output /path/to/export --id 304
exvms export --output /path/to/export --id 304 --id 305
```

Exports recordings by stable `Z_PK` ID from the `view` output. Explicit ID exports fail before copying anything if any requested ID does not exist, needs download, would overwrite a target, or would collide with another target.

```sh
exvms export --output /path/to/export --all
```

Exports all local-exportable recordings, skips known `needs-download` recordings, and prints a final summary.

```sh
exvms export --output /path/to/export
```

Asks once for confirmation. Only exact `yes` proceeds.

```sh
exvms export --output /path/to/export --all --jobs 4
```

Controls export parallelism. The default is `min(4, available_parallelism)`.

## Data Model

The implementation supports these Voice Memos stores:

- `CloudRecordings.db` with `ZCLOUDRECORDING`
- `Recordings.db` with `ZRECORDING`
- `ZFOLDER` when available for folder names

`ZDATE` is Apple/Core Data reference time. The tool converts it with:

```text
unix_seconds = ZDATE + 978307200
```

`view` reports:

- `store`: `cloud-recordings` or `local-recordings`
- `path`: `relative`, `absolute`, or `missing`
- `availability`: `local`, `needs-download`, or `outside-recordings-dir`

Relative `ZPATH` values are resolved under the active Voice Memos Recordings directory. Absolute `ZPATH` values are only used when they can be safely mapped back into that selected Recordings directory, including mounted backup volumes such as `/Volumes/<name>/Users/<user>/...`.

## Storage Discovery

The tool tries these directories in order:

```text
~/Library/Group Containers/group.com.apple.VoiceMemos.shared/Recordings
~/Library/Containers/Voice Memos/Data/Library/Application Support/Recordings
~/Library/Application Support/com.apple.voicememos/Recordings
```

If macOS denies access to Voice Memos data, grant Full Disk Access to the terminal/app running `exvms`.

## Cloud-Only Recordings

Cloud-only or incomplete recordings are not exported. Missing files, 0-byte files, and rows with a missing database path are classified as `needs-download`.

Voice Memos CloudKit placeholders do not expose a public standard iCloud Drive download path for this tool. `exvms` reports that the recording must be opened/downloaded in Voice Memos.app first. Paths that resolve outside the selected Recordings directory are reported as `outside-recordings-dir`.

## Architecture

`src/main.rs` is the CLI layer. It parses arguments, performs user confirmation, and formats terminal output.

The rest of `src/` is the Rust core library:

- `db.rs`: probe and read Voice Memos SQLite databases and classify store/path metadata
- `storage.rs`: discover the active Recordings directory and database
- `export.rs`: plan, preflight, parallelize, and execute exports
- `macos.rs`: macOS timestamp handling
- `sanitize.rs`: sanitize target file and folder names
- `time.rs`: Apple/Core Data time conversion and display formatting
- `lib.rs`: public modules and error type

## C API / Swift Status

There is no C ABI exposed yet. A Swift/macOS app currently has two practical options:

- shell out to the `exvms` binary and parse its output; or
- add a future FFI target.

A future FFI layer should add a `cdylib` crate type, `extern "C"` entry points, C-compatible result/recording structs, explicit allocation/free functions, and generated headers or Swift bindings. Do not expose Rust-owned structs, `String`, `Vec`, or `Result` directly over the C boundary.

## Verification

```sh
cargo fmt -- --check
cargo test
cargo clippy --all-targets -- -D warnings
cargo check
```

## GitHub Actions

The workflow at `.github/workflows/build.yml` checks formatting, runs clippy and tests on `master` and pull requests targeting `master`. On git tag pushes, it builds the release CLI product with `cargo build --release --bin exvms` and uploads the compiled `exvms` binary as a workflow artifact.

## Plans

Project plans live under `plans/`. The current implementation plan is `plans/implementation.md`.
