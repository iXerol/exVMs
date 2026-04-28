#!/bin/sh
set -eu

cd "$SRCROOT"

artifact_dir="$SRCROOT/target/xcode"
artifact="$artifact_dir/libexvms.dylib"
mkdir -p "$artifact_dir"
rm -f "$artifact"

run_cargo() {
  if [ -n "${CARGO:-}" ]; then
    "$CARGO" "$@"
  elif command -v cargo >/dev/null 2>&1; then
    cargo "$@"
  elif [ -n "${SHELL:-}" ]; then
    "$SHELL" -lc "cd \"$SRCROOT\" && cargo $*"
  else
    echo "error: cargo was not found. Set CARGO or make cargo available through Xcode's PATH/login shell." >&2
    exit 127
  fi
}

run_rustup() {
  if command -v rustup >/dev/null 2>&1; then
    rustup "$@"
  elif [ -n "${SHELL:-}" ]; then
    "$SHELL" -lc "rustup $*"
  else
    return 127
  fi
}

ensure_rust_target() {
  target="$1"
  if run_rustup target list --installed 2>/dev/null | grep -qx "$target"; then
    return 0
  fi
  if ! run_rustup target add "$target"; then
    echo "error: Rust target '$target' is not installed and rustup could not install it." >&2
    echo "hint: run 'rustup target add $target' or set CARGO to a toolchain that has this target." >&2
    exit 1
  fi
}

rust_target_for_arch() {
  case "$1" in
    arm64)
      echo "aarch64-apple-darwin"
      ;;
    x86_64)
      echo "x86_64-apple-darwin"
      ;;
    *)
      echo "error: unsupported macOS architecture '$1'" >&2
      exit 1
      ;;
  esac
}

archs="${ARCHS:-$(uname -m)}"
if [ "${ACTION:-}" = "install" ] || [ "${EXVMS_BUILD_UNIVERSAL:-}" = "1" ]; then
  archs="arm64 x86_64"
fi

targets=""
for arch in $archs; do
  target="$(rust_target_for_arch "$arch")"
  case " $targets " in
    *" $target "*) ;;
    *) targets="$targets $target" ;;
  esac
done

libs=""
for target in $targets; do
  ensure_rust_target "$target"
  run_cargo build --release --lib --no-default-features --target "$target"
  lib="$SRCROOT/target/$target/release/libexvms.dylib"
  if [ ! -f "$lib" ]; then
    echo "error: expected Rust library was not produced at $lib" >&2
    exit 1
  fi
  libs="$libs $lib"
done

set -- $libs
if [ "$#" -eq 1 ]; then
  cp "$1" "$artifact"
else
  xcrun lipo -create "$@" -output "$artifact"
fi

xcrun lipo -info "$artifact"
