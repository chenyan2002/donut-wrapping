# A mock Fastly Compute host, as a donut

The guest is proxy-component's `tests/rust.wasm`, a real Fastly Compute
application built with the Fastly Rust SDK. It exports
`fastly:compute/http-incoming@0.1.0` and imports ten `fastly:compute/*`
interfaces plus `std`'s WASI.

This donut virtualizes all of `fastly:compute/*`, passes WASI through, and runs
one request through the guest per instance (`serve(uri)`). A Fastly instance
handles exactly one request; the guest's Viceroy adapter asserts this. The
donut reports the host calls the guest made and the response it sent
downstream.

* `wit/deps/` is the guest's own world, decoded from the binary with
  `wasm-tools component wit rust.wasm --out-dir`, and flattened to one file per
  package. The guest world is `root:component/root`. `wit/host.wit` is the
  donut's world.
* `host/src/lib.rs` mocks all of `fastly:compute/*` with `mock:`. The 60 virtual
  functions are generated, each defaulting to `unimplemented!()`. The host
  overrides the 13 that this guest calls for a request (from proxy-component's
  recorded `trace.out`) in its `forward::Hooks` impl, and logs every call in
  `before`. The three resources the guest creates hold state types declared in
  `mock` (`{ "fastly:compute/http-resp@0.1.0": { response: ResponseState } }`),
  and each wrapper derefs to its state. The other resources have no state
  type, so they can't be created.
* `build.sh` builds `out/donut.wasm`. `GUEST` overrides the guest path
  (default `~/src/proxy-component/tests/rust.wasm`).
* Tested by `crates/donut-tests` (`fastly_guest`). `/` gives the same 200 response
  and 668-byte page as the recorded run, and `/missing` gives a 404. Both make the
  same 13 host calls, in the same order as the recording.
