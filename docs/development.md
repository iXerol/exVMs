# Development

## Architecture

`src/main.rs` is the CLI layer. It parses arguments, performs user confirmation, and formats terminal output.

The rest of `src/` is the Rust core library:

- `db.rs`: probes and reads Voice Memos SQLite databases and classifies store/path metadata
- `storage.rs`: discovers the active Recordings directory and database
- `export.rs`: plans, preflights, parallelizes, and executes exports
- `macos.rs`: handles macOS timestamp conversion
- `sanitize.rs`: sanitizes target file and folder names
- `template.rs`: renders filename templates
- `time.rs`: converts Apple/Core Data time and display formatting
- `ffi.rs`: exposes the C ABI used by the SwiftUI app
- `lib.rs`: public modules and error type

## Voice Memos Stores

The implementation supports these stores:

- `CloudRecordings.db` with `ZCLOUDRECORDING`
- `Recordings.db` with `ZRECORDING`
- `ZFOLDER` when available for folder names

`ZDATE` is Apple/Core Data reference time:

```text
unix_seconds = ZDATE + 978307200
```

Relative `ZPATH` values are resolved under the active Voice Memos Recordings directory. Absolute `ZPATH` values are only used when they can be safely mapped back into that selected Recordings directory, including mounted backup volumes such as `/Volumes/<name>/Users/<user>/...`.

The default discovery order is:

```text
~/Library/Group Containers/group.com.apple.VoiceMemos.shared/Recordings
~/Library/Containers/Voice Memos/Data/Library/Application Support/Recordings
~/Library/Application Support/com.apple.voicememos/Recordings
```

Cloud-only or incomplete recordings are classified as `needs-download`. Paths that resolve outside the selected Recordings directory are classified as `outside-recordings-dir`.

## Rust Library And FFI

The Rust crate builds both an `rlib` and a `cdylib`. The default feature set is intentionally empty, so a library build does not include CLI-only dependencies such as `clap` and `unicode-width`. The CLI binary is behind the `cli` feature.

The C ABI is declared in `include/exvms.h`. The header is hand-written so the project can keep a small stable ABI and add Swift-facing nullability and naming annotations without requiring cbindgen in every app build.

The FFI layer exposes:

- `exvms_discover_store`
- `exvms_list_recordings`
- `exvms_export_recordings`
- matching `exvms_free_*` functions for all Rust-owned returned memory

The C boundary intentionally does not expose Rust-owned `String`, `Vec`, `PathBuf`, `Result`, or Rust enums directly. GUI-specific state such as whether a planned output file is already exported remains in the Swift layer.

## macOS App Build

The macOS app is generated with Tuist and implemented in SwiftUI. It links the Rust `cdylib` through `include/exvms.h`, targets macOS 14+, and compiles in Swift 6 mode with strict concurrency enabled.

```sh
tuist generate --no-open
xcodebuild -workspace exVMs.xcworkspace -scheme exVMsApp -destination 'platform=macOS' -derivedDataPath /tmp/exvms-dd build
```

The Xcode target runs this in a pre-build script:

```sh
cargo build --release --lib --no-default-features
```

It embeds `target/release/libexvms.dylib` into the app bundle. The script resolves Cargo through `CARGO`, Xcode's `PATH`, or the user's login shell; it does not assume a rustup or Homebrew install path.

The app is intended to run unsandboxed. Rust reports neutral storage permission errors; the Swift app checks Voice Memos access on launch and shows a Full Disk Access panel when macOS blocks the database.

The app stores local file paths as `System.FilePath`, including paths converted from Rust-owned C strings with `FilePath(platformString:)`; it converts to `URL` or `String` only at framework/display boundaries.

The selected output directory is stored in `UserDefaults`. If the saved path is empty or no longer exists, launch behaves the same as no directory being selected.

Localized app strings live under `App/Resources/*.lproj/Localizable.strings`. The supported app localizations are English, Simplified Chinese, Traditional Chinese, and Japanese. Use semantic keys that describe purpose and part of speech, such as `action.refresh`, `table.column.export`, and `status.items.count`; do not use English UI copy as keys. SwiftUI views should use native `LocalizedStringKey` literals, while AppKit and view-model boundaries that require a concrete `String` should use `String(localized:)`.

App tests use Swift Testing, not XCTest.

## Verification

Rust checks:

```sh
cargo fmt -- --check
cargo clippy --lib --no-default-features -- -D warnings
cargo clippy --all-targets --features cli -- -D warnings
cargo test --features cli
cargo check --lib --no-default-features
```

App checks:

```sh
tuist generate --no-open
xcodebuild -workspace exVMs.xcworkspace -scheme exVMsApp -destination 'platform=macOS' -derivedDataPath /tmp/exvms-dd test
```

When reading Xcode or Swift build output locally, pipe through `xcsift`:

```sh
xcodebuild -workspace exVMs.xcworkspace -scheme exVMsApp -destination 'platform=macOS' -derivedDataPath /tmp/exvms-dd test 2>&1 | xcsift -f toon -w
```

## CI

`.github/workflows/build.yml` checks formatting, runs clippy for the library without default features, runs CLI clippy/tests with `--features cli`, generates the Tuist workspace, and runs the macOS app test action on `master` and pull requests targeting `master`.

The app build in CI does not require a signing certificate for verification. Xcode can build/test the app with local/ad-hoc signing on the GitHub macOS runner because the target is not being distributed, notarized, or uploaded to users.

For distribution outside CI verification, configure Developer ID signing separately:

- Store a Developer ID Application `.p12` certificate in a GitHub secret, usually base64-encoded.
- Store the `.p12` password in a separate secret.
- Import the certificate into a temporary CI keychain before the archive/export step.
- Set `DEVELOPMENT_TEAM`, `CODE_SIGN_IDENTITY="Developer ID Application"`, and any export options needed by the release job.
- For notarization, provide `APPLE_ID`, `APPLE_TEAM_ID`, and an app-specific password or App Store Connect API key, then submit the exported app/zip/dmg with `xcrun notarytool`.

Keep these signing/notarization steps in a tag or release workflow. Pull request CI should continue to run unsigned/ad-hoc build and test only.

On git tag pushes, CI builds the release CLI product with:

```sh
cargo build --release --bin exvms --features cli
```

The compiled `exvms` binary is uploaded as a workflow artifact.
