# Shared helpers for examples/*/build.sh (sourced; run from the example's dir).
#
#   CARGO_TARGET_DIR  where to build (default: <repo>/target)
#   WASM_TOOLS        a wasm-tools with `component new --donut-guest`
#                     (default: `wasm-tools` on PATH), from the donut branch:
#                     cargo install --git https://github.com/chenyan2002/wasm-tools \
#                       --branch donut wasm-tools --locked
set -eu
ROOT=$(cd ../.. && pwd)
TARGET=${CARGO_TARGET_DIR:-$ROOT/target}
WASM_TOOLS=${WASM_TOOLS:-wasm-tools}
if ! "$WASM_TOOLS" component new --help 2>/dev/null | grep -q -- --donut-guest; then
  echo "error: \`$WASM_TOOLS\` has no \`component new --donut-guest\`. Install the donut branch:" >&2
  echo "  cargo install --git https://github.com/chenyan2002/wasm-tools --branch donut wasm-tools --locked" >&2
  echo "or point WASM_TOOLS at such a build." >&2
  exit 1
fi
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
