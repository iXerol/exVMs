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
- `CloudRecordings.db` path
- recording ID, title, recording time, duration, local/cloud status, folder, and relative audio path

```sh
exvms export --output /path/to/export --id 304
exvms export --output /path/to/export --id 304 --id 305
```

Exports recordings by stable `Z_PK` ID from the `view` output. Explicit ID exports fail before copying anything if any requested ID does not exist, is cloud-only/incomplete, would overwrite a target, or would collide with another target.

```sh
exvms export --output /path/to/export --all
```

Exports all local-exportable recordings, skips known cloud-only/incomplete recordings, and prints a final summary.

```sh
exvms export --output /path/to/export
```

Asks once for confirmation. Only exact `yes` proceeds.

```sh
exvms export --output /path/to/export --all --jobs 4
```

Controls export parallelism. The default is `min(4, available_parallelism)`.

## Data Model

The current implementation uses the verified Voice Memos schema:

- `ZCLOUDRECORDING`: `Z_PK`, `ZENCRYPTEDTITLE`, `ZDATE`, `ZDURATION`, `ZLOCALDURATION`, `ZEVICTIONDATE`, `ZPATH`, `ZFOLDER`
- `ZFOLDER`: `Z_PK`, `ZENCRYPTEDNAME`

`ZDATE` is Apple/Core Data reference time. The tool converts it with:

```text
unix_seconds = ZDATE + 978307200
```

`ZPATH` is treated as relative to the active Voice Memos Recordings directory.

## Storage Discovery

The tool tries these directories in order:

```text
~/Library/Group Containers/group.com.apple.VoiceMemos.shared/Recordings
~/Library/Containers/Voice Memos/Data/Library/Application Support/Recordings
~/Library/Application Support/com.apple.voicememos/Recordings
```

If macOS denies access to Voice Memos data, grant Full Disk Access to the terminal/app running `exvms`.

## Cloud-Only Recordings

Cloud-only or incomplete recordings are not exported. A missing or 0-byte local audio file is classified as unavailable unless macOS Foundation identifies it as a downloadable ubiquitous item.

Voice Memos CloudKit placeholders may not expose standard iCloud Drive ubiquitous metadata. In that case, `exvms` reports that the recording must be opened/downloaded in Voice Memos.app first.

## Architecture

`src/main.rs` is the CLI layer. It parses arguments, performs user confirmation, and formats terminal output.

The rest of `src/` is the Rust core library:

- `db.rs`: read and validate the Voice Memos SQLite database
- `storage.rs`: discover the active Recordings directory
- `export.rs`: plan, preflight, parallelize, and execute exports
- `macos.rs`: macOS timestamp and best-effort ubiquitous item handling
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
