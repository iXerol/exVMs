# Agent Notes

## Project Shape

- This is a Rust package named `exvms` with a binary command named `exvms`.
- `src/main.rs` is the CLI layer only.
- Core behavior belongs in library modules under `src/` and should remain callable from Rust without going through terminal formatting.
- Keep public errors in `src/lib.rs`.

## Important Behavior

- Voice Memos data is private user data. Avoid printing real titles or paths in tests unless the user explicitly asks for live output.
- Open `CloudRecordings.db` read-only.
- Use `Z_PK` as the stable user-facing recording ID.
- Treat `ZDATE` as Apple/Core Data reference time and convert with `+ 978307200`.
- Treat `ZPATH` as relative to the Recordings directory and reject absolute paths, `..`, roots, or prefixes.
- Never overwrite export targets.
- Explicit `--id` exports must fail before copying if any requested ID is missing or cloud-only/incomplete.
- Full export may skip known cloud-only/incomplete recordings, but must report them.
- Keep `view` non-mutating; do not trigger downloads from `view`.
- Only `export` may perform best-effort Foundation ubiquitous-item download attempts.

## Testing

Run these before handing off changes:

```sh
cargo fmt -- --check
cargo clippy --lib --no-default-features -- -D warnings
cargo clippy --all-targets --features cli -- -D warnings
cargo test --features cli
cargo check --lib --no-default-features
tuist generate --no-open
xcodebuild -workspace exVMs.xcworkspace -scheme exVMsApp -destination 'platform=macOS' -derivedDataPath /tmp/exvms-dd test
```

Use fixture databases or temporary files for tests. Do not depend on the developer's real Voice Memos database in automated tests.
Use Swift Testing for app tests, not XCTest.

## FFI

The C ABI lives in `src/ffi.rs`, and its hand-written Swift-facing header lives in `include/exvms.h`.

- Keep FFI as a separate glue surface over the Rust core modules.
- Keep the header stable and annotate it for Swift with nullability and `swift_name`.
- Use C-compatible structs only.
- Provide explicit free functions for allocated buffers.
- Do not expose Rust `String`, `Vec`, references, trait objects, Rust enums, or `Result` across the ABI boundary.
- GUI-only state, including output-directory monitoring and `exported` status, belongs in Swift.

## macOS App

- The app is generated with Tuist from `Project.swift`; do not commit generated `.xcodeproj` or `.xcworkspace` files.
- The deployment target is macOS 14+ so the app can use SwiftUI's Observation framework directly.
- Swift code must compile in Swift 6 mode with strict concurrency enabled.
- Use `@Observable` for app state instead of Combine's `ObservableObject` / `@Published`.
- Do not hard-code cargo install paths in Xcode scripts. Resolve cargo through `CARGO`, current `PATH`, or the user's login shell.
- Keep Cargo features explicit: default library builds should use `--no-default-features`; CLI builds/tests should opt into `--features cli`.
- Keep Full Disk Access wording in the app layer. Rust core errors should describe storage permission failures without naming app-specific remediation; the app should check access on launch and show an explicit Full Disk Access panel when Voice Memos data cannot be opened.
- Persist the selected output directory in `UserDefaults`; if the saved path is empty or no longer exists, behave like first launch.
- Filename token insertion should be internal to the filename editor view. Do not use system drag-and-drop, `NSItemProvider`, `Transferable`, or `UTType` for title/time chips.
- Token chips in the filename editor must remain indivisible text attachments with blue capsule styling and meaningful accessibility labels.
- Use `System.FilePath` for local file paths in Swift app state and FFI path conversion. Convert to `URL` or `String` only at framework boundaries.
- App resources are currently used for localization. Supported localizations are English, Simplified Chinese, Traditional Chinese, and Japanese.
- SwiftUI visible text should use native `LocalizedStringKey` literals. Use `String(localized:)` only at framework boundaries that require `String`.
- Localization keys should describe purpose and part of speech, for example `action.refresh`, `table.column.export`, and `status.items.count`. Do not use English source copy as keys.
