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
cargo test
cargo clippy --all-targets -- -D warnings
cargo check
```

Use fixture databases or temporary files for tests. Do not depend on the developer's real Voice Memos database in automated tests.

## FFI

No C ABI exists yet. If adding Swift/macOS app integration, introduce it intentionally as a separate FFI surface with:

- `cdylib` crate type;
- `extern "C"` functions;
- C-compatible structs only;
- explicit free functions for allocated buffers;
- no Rust `String`, `Vec`, references, trait objects, or `Result` across the ABI boundary.
