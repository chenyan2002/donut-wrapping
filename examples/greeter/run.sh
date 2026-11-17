#!/bin/sh
# Builds the greeter donut and app, plugs the app into both holes, and checks
# what the result prints, on Wasmtime ≥ 49.
#
# Usage: ./run.sh. WASM_TOOLS must be a wasm-tools with hole mode, and WAC a
# wac that plugs holes (defaults: the donut-hole builds in
# ~/src/bytecodealliance); WASMTIME picks the runtime.
set -e
cd "$(dirname "$0")"
ROOT=$(cd ../.. && pwd)
HOLE_WASM_TOOLS=$HOME/src/bytecodealliance/wasm-tools/target/debug/wasm-tools
if [ -z "${WASM_TOOLS:-}" ] && [ -x "$HOLE_WASM_TOOLS" ]; then WASM_TOOLS=$HOLE_WASM_TOOLS; fi
WASM_TOOLS=${WASM_TOOLS:-wasm-tools}
HOLE_WAC=$HOME/src/bytecodealliance/wac/target/debug/wac
if [ -z "${WAC:-}" ] && [ -x "$HOLE_WAC" ]; then WAC=$HOLE_WAC; fi
WAC=${WAC:-wac}
WASMTIME=${WASMTIME:-$HOME/src/bytecodealliance/wasmtime/target/debug/wasmtime}
RELEASE=wasm32-wasip2/release
mkdir -p out

# 1. The donut, to a core module. (A separate target directory, so that the
#    linker flag doesn't invalidate the app's build.)
RUSTFLAGS="-Clink-arg=--skip-wit-component" cargo build --quiet --release \
  --target wasm32-wasip2 -p greeter-donut --target-dir "$ROOT/target/core"

# 2. The core module, to a component with two holes, `english` and `french`.
$WASM_TOOLS component new "$ROOT/target/core/$RELEASE/greeter_donut.wasm" -o out/donut.wasm

# 3. The app: an ordinary component.
cargo build --quiet --release --target wasm32-wasip2 -p greeter-app

# 4. Plug the app into both holes.
$WAC plug out/donut.wasm \
  --plug english="$ROOT/target/$RELEASE/greeter_app.wasm" \
  --plug french="$ROOT/target/$RELEASE/greeter_app.wasm" \
  -o out/composed.wasm

# 5. Run.
pass=0 fail=0
check() { # check <path> <expected>
  actual=$($WASMTIME run --invoke "handle(\"$1\")" out/composed.wasm 2>&1 || true)
  if [ "$actual" = "$2" ]; then echo "ok   handle(\"$1\") = $actual"; pass=$((pass + 1))
  else printf 'FAIL handle("%s")\n  expected: %s\n  actual:   %s\n' "$1" "$2" "$actual"; fail=$((fail + 1)); fi
}
check en/Ada '"Hello, Ada!"'
check fr/Ada '"Bonjour, Ada!"'
check de/Ada '"unknown route `de/Ada`"'
echo "$pass passed, $fail failed"
[ "$fail" = 0 ]
