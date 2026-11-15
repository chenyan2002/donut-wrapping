# Shared helpers for examples/*/build.sh (sourced; run from the example's dir).
#
#   CARGO_TARGET_DIR  where to build (default: <repo>/target)
#   WASM_TOOLS        a wasm-tools with wit-component's donut mode
#                     (default: the local ~/src/bytecodealliance/wasm-tools build)
set -eu
ROOT=$(cd ../.. && pwd)
TARGET=${CARGO_TARGET_DIR:-$ROOT/target}
WASM_TOOLS=${WASM_TOOLS:-$HOME/src/bytecodealliance/wasm-tools/target/debug/wasm-tools}
mkdir -p out

# guest <crate> <out.wasm>: an ordinary component, built for wasm32-wasip2.
guest() {
  cargo build --quiet --release --target wasm32-wasip2 -p "$1"
  cp "$TARGET/wasm32-wasip2/release/$(echo "$1" | tr - _).wasm" "$2"
}

# wat_guest <file.wat> <out.wasm>: a guest component written in WAT.
wat_guest() {
  "$WASM_TOOLS" parse "$1" -o "$2"
}

# donut <crate> <guest.wasm> <out.wasm>: a donut's core module (wasm32-wasip2,
# stopping after wasm-ld), embedding <guest.wasm> with wit-component's donut
# mode.
donut() {
  RUSTFLAGS="-Clink-arg=--skip-wit-component" cargo build --quiet --release \
    --target wasm32-wasip2 -p "$1" --target-dir "$TARGET/p2-donut"
  "$WASM_TOOLS" component new \
    "$TARGET/p2-donut/wasm32-wasip2/release/$(echo "$1" | tr - _).wasm" \
    --donut-guest "$2" -o "$3"
  "$WASM_TOOLS" validate "$3"
  echo "built $3"
}
