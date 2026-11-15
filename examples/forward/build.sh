#!/bin/sh
# out/app.wasm          the app, an ordinary component
# out/recorder.wasm     generated forwarding, logging every call (`Hooks`)
# out/interceptor.wasm  generated forwarding with two functions overridden
cd "$(dirname "$0")" && . ../common.sh
guest forward-app out/app.wasm
donut forward-recorder out/app.wasm out/recorder.wasm
donut forward-interceptor out/app.wasm out/interceptor.wasm
