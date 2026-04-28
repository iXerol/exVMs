# macOS App Glue Layer Plan

## Summary

Keep the existing Rust CLI/core split intact and add a narrow C ABI glue layer for the SwiftUI macOS app. The Rust core remains responsible for Voice Memos discovery, metadata loading, availability classification, template parsing, export planning, and final export preflight. Swift owns GUI-only state such as selected output directory, file-system monitoring, selection state, and exported markers.

## Rust FFI

- Build the crate as both `rlib` and `cdylib`.
- Keep Cargo features explicit: `default = []` for the library/FFI surface, and `cli` for CLI-only dependencies and the `exvms` binary.
- Keep FFI code isolated in `src/ffi.rs`.
- Keep `include/exvms.h` hand-written while the ABI is small, because it is a stable public contract and can carry Swift-facing nullability and `swift_name` annotations.
- Expose only C-compatible structs and functions.
- Provide explicit `exvms_free_*` functions for all Rust-owned memory returned across the ABI.
- Do not expose Rust `String`, `Vec`, `PathBuf`, references, trait objects, Rust enums, or `Result` across the ABI boundary.

## SwiftUI App

- Generate the macOS app with Tuist from `Project.swift`; do not commit generated `.xcodeproj` or `.xcworkspace` output.
- Target macOS 14+ so the app can use SwiftUI Observation directly.
- Compile Swift in Swift 6 mode with strict concurrency enabled.
- Use `@Observable` for app state instead of Combine's `ObservableObject` / `@Published`.
- The Xcode Rust build phase must not hard-code cargo install paths. It should honor `CARGO`, then Xcode's `PATH`, then the user's login shell.
- Run unsandboxed. Rust should report neutral storage permission errors; the Swift app should check Voice Memos access on launch and translate permission or SQLite open failures into an explicit Full Disk Access panel. The CLI can continue to treat the invoking terminal as the permission boundary.
- Persist the selected output directory in `UserDefaults`. If the stored path is empty or no longer exists, keep the same disabled state as a first launch.
- Store app-facing local paths as `System.FilePath`, including FFI paths converted from Rust-owned C strings with `FilePath(platformString:)`; convert to `URL` or `String` only at framework and display boundaries.
- Keep app resources because the app now ships localized `Localizable.strings` files.
- Support English, Simplified Chinese, Traditional Chinese, and Japanese localizations.
- Use semantic localization keys by purpose and part of speech, such as `action.refresh`, `table.column.export`, and `status.items.count`. Do not use English UI copy as localization keys.
- SwiftUI views should rely on native `LocalizedStringKey` literals. Use `String(localized:)` only where an API requires a concrete `String`, such as AppKit accessibility labels, `NSOpenPanel`, or view-model status text.

## GUI State And Filename Builder

- Disable export until the user selects an output directory and selects at least one currently exportable, not-yet-exported row.
- Swift monitors the selected output directory and marks rows as exported when the expected target file exists.
- The recordings table should use an explicit export checkbox column instead of default row selection. Non-exportable rows must disable that checkbox.
- Local recordings should expose a Quick Look preview action for the resolved source audio file. Missing, cloud-only, and outside-storage rows should disable preview.
- Provide Select All and Select None actions for export checkboxes.
- Show folder names in the table, provide a folder filter, and keep every data column except `Target` sortable.
- Swift stores filename format as a structured recipe of token and literal parts.
- v1 filename tokens are `title`, `id`, `year`, `month`, `day`, `hour`, `minute`, and `second`.
- The filename editor should behave like one mixed text field: literal text can be typed directly in the row, and token chips are inserted at the current cursor position when clicked.
- Filename token insertion should stay internal to the editor view. Do not use system drag-and-drop, `NSItemProvider`, `Transferable`, or `UTType` for title/time chips.
- Token chips inside the editor should be represented as indivisible text attachments, not editable character spans.
- Token chips should keep the blue capsule visual style in both SwiftUI chip controls and generated text attachment images, and the image width should be measured from rendered text rather than fixed character counts.
- Filename editor accessibility must expose meaningful labels for the editor, inserted tokens, literal text, and token insertion controls; no control should be announced as blank.
- The UI should show a rendered sample using the current time and `Recording Title`, with the same recipe that Swift uses to compute exported state.
- Before export, Swift serializes the structured recipe into the Rust template string accepted by the existing export implementation.
- Rust still performs final export validation and no-overwrite checks.

## Verification

Run:

```sh
cargo fmt -- --check
cargo clippy --lib --no-default-features -- -D warnings
cargo clippy --all-targets --features cli -- -D warnings
cargo test --features cli
cargo check --lib --no-default-features
tuist generate --no-open
xcodebuild -workspace exVMs.xcworkspace -scheme exVMsApp -destination 'platform=macOS' -derivedDataPath /tmp/exvms-dd test
```

Swift app tests should use Swift Testing, not XCTest.
