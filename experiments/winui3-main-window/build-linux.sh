#!/usr/bin/env bash

set -euo pipefail

ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
CORE_MANIFEST="$ROOT/../../crates/tiez-core/Cargo.toml"
RUST_MANIFEST="$ROOT/rust-core/Cargo.toml"
WINDOWS_TARGET="x86_64-pc-windows-msvc"
ARTIFACTS="$ROOT/artifacts/x64/Release"
SKIP_TESTS=false
SKIP_WINDOWS_TARGET=false

usage() {
  cat <<'EOF'
Usage: ./build-linux.sh [options]

Runs the Linux-supported part of the WinUI experiment:
  1. native Rust core tests;
  2. Windows MSVC Rust DLL cross-compilation with cargo-xwin;
  3. copies the DLL into artifacts/x64/Release.

Options:
  --skip-tests          Skip native Rust tests.
  --skip-windows-target Skip cargo-xwin and only run native tests.
  -h, --help            Show this help.

The WinUI/XAML executable must still be built on Windows with build.ps1.
EOF
}

while (($# > 0)); do
  case "$1" in
    --skip-tests)
      SKIP_TESTS=true
      ;;
    --skip-windows-target)
      SKIP_WINDOWS_TARGET=true
      ;;
    -h | --help)
      usage
      exit 0
      ;;
    *)
      echo "Unknown option: $1" >&2
      usage >&2
      exit 2
      ;;
  esac
  shift
done

if [[ "$(uname -s)" != "Linux" ]]; then
  echo "This helper is intended for Linux. Use build.ps1 on Windows." >&2
  exit 1
fi

for command in cargo rustc; do
  if ! command -v "$command" >/dev/null 2>&1; then
    echo "$command was not found. Install the repository-pinned Rust toolchain with rustup." >&2
    exit 1
  fi
done

echo "Rust host: $(rustc -vV | awk '/^host:/ { print $2 }')"

export CARGO_TARGET_DIR="$ROOT/rust-core/target"

if [[ "$SKIP_TESTS" == false ]]; then
  cargo test --manifest-path "$CORE_MANIFEST" --locked
  cargo test --manifest-path "$RUST_MANIFEST" --locked
fi

if [[ "$SKIP_WINDOWS_TARGET" == true ]]; then
  echo "Skipped Windows MSVC cross-compilation."
  exit 0
fi

for command in rustup cargo-xwin clang-cl lld-link; do
  if ! command -v "$command" >/dev/null 2>&1; then
    case "$command" in
      cargo-xwin)
        echo "cargo-xwin was not found. Install it with: cargo install --locked cargo-xwin" >&2
        ;;
      *)
        echo "$command was not found; it is required for the Windows MSVC Rust target." >&2
        ;;
    esac
    exit 1
  fi
done

if ! rustup target list --installed | grep -Fxq "$WINDOWS_TARGET"; then
  echo "$WINDOWS_TARGET is not installed. Run: rustup target add $WINDOWS_TARGET" >&2
  exit 1
fi

cargo xwin build \
  --manifest-path "$RUST_MANIFEST" \
  --release \
  --locked \
  --target "$WINDOWS_TARGET"

WINDOWS_DLL="$ROOT/rust-core/target/$WINDOWS_TARGET/release/tiez_winui_core.dll"
if [[ ! -f "$WINDOWS_DLL" ]]; then
  echo "Cross-compilation succeeded but the expected DLL is missing: $WINDOWS_DLL" >&2
  exit 1
fi

mkdir -p "$ARTIFACTS"
cp "$WINDOWS_DLL" "$ARTIFACTS/tiez_winui_core.dll"

echo
echo "Windows Rust DLL: $ARTIFACTS/tiez_winui_core.dll"
echo "Build the WinUI executable on Windows with: .\\build.ps1 -Configuration Release"
