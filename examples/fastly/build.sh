#!/bin/sh
# out/donut.wasm  the mock Fastly host around proxy-component's Fastly guest
#                 (GUEST, default ~/src/proxy-component/tests/rust.wasm)
cd "$(dirname "$0")" && . ../common.sh
donut fastly-host "${GUEST:-$HOME/src/proxy-component/tests/rust.wasm}" out/donut.wasm
