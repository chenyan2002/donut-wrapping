#!/bin/sh
# out/donut.wasm             the donut around the Rust guest
# out/init-tick.wasm         a WAT guest whose start function calls the donut
# out/init-tick-reentrant.wasm  ... and calls back into itself (must trap)
cd "$(dirname "$0")" && . ../common.sh
guest virtualize-guest out/guest.wasm
donut virtualize-donut out/guest.wasm out/donut.wasm
for v in tick tick-reentrant; do
  wat_guest init/$v.wat out/init-$v-guest.wasm
  donut virtualize-init-donut out/init-$v-guest.wasm out/init-$v.wasm
done
