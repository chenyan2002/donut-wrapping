#!/bin/sh
# out/app.wasm    the app, an ordinary component
# out/donut.wasm  the app, wrapped for the new host
cd "$(dirname "$0")" && . ../common.sh
guest secrets-app out/app.wasm
donut secrets-donut out/app.wasm out/donut.wasm
