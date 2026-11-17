#!/bin/sh
# Builds this crate's core module (stopping after wasm-ld, as wit-component's
# hole mode will consume it) and checks that its child-related core imports
# and exports are exactly those of `$main` in spike/donut.wat.
set -eu
cd "$(dirname "$0")"
ROOT=$(cd ../.. && pwd)
WASM_TOOLS=${WASM_TOOLS:-wasm-tools}
TARGET=$ROOT/target/core
RUSTFLAGS="-Clink-arg=--skip-wit-component" cargo build --quiet --release \
  --target wasm32-wasip2 -p spike-donut --target-dir "$TARGET"
core=$TARGET/wasm32-wasip2/release/spike_donut.wasm
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

# `(import "m" "n"` / `(export "n"` lines, as `import "m" "n"` / `export "n"`.
names() {
  grep -E '^\s*\((import|export) "' | sed -E 's/^ *\((import|export) ("[^"]*"( "[^"]*")?).*/\1 \2/'
}
"$WASM_TOOLS" print "$core" | names | grep -E 'guest-|virtual-|export "run"|export "reenter"' | sort > "$tmp/rust"
"$WASM_TOOLS" parse "$ROOT/spike/donut.wat" -o "$tmp/donut.wasm"
"$WASM_TOOLS" print "$tmp/donut.wasm" | sed -n '/core module \$main/,/core module \$shim/p' \
  | names | grep -v '"memory"' | sort > "$tmp/spike"
if ! diff -u "$tmp/spike" "$tmp/rust"; then
  echo "FAIL: the core module's names differ from spike/donut.wat's \$main (- spike, + Rust)" >&2
  exit 1
fi
# wit-component can decode and merge every component-type section. The
# children's exports show up as `guest-<child>:` imports, which the encoder
# drops; nothing else of theirs is imported: in particular not `spike:hole/*`,
# which the guests import but the donut virtualizes.
world=$("$WASM_TOOLS" component wit "$core" | sed -n '/^world root/,/^}/p')
echo "$world" | grep -q 'import guest-a:spike-hole/handler;'
echo "$world" | grep -q 'import guest-b:spike-hole/handler;'
if echo "$world" | grep -E '^  import' | grep -v 'import guest-[ab]:' | grep -q 'spike'; then
  echo "FAIL: the merged world imports a child interface outside guest-<child>:" >&2
  echo "$world" | grep -E '^  import .*spike' >&2
  exit 1
fi
echo "ok: $(wc -l < "$tmp/rust" | tr -d ' ') child-related names match spike/donut.wat; metadata decodes;"
echo "    the children's interfaces are imported only as guest-<child>:…"
echo "core module: $core"
