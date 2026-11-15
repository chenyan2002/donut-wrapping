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
* `host/src/fastly.rs` implements the 60 virtual functions. It was generated as
  `unimplemented!()` stubs with:
  ```
  cargo run -p donut-plan --example expand -- examples/fastly/wit \
    test:fastly-host/host root:component/root fastly:compute/types@0.1.0 ... > /tmp/x.rs
  python3 tools/stubs.py /tmp/x.rs
  ```
  Then the 13 functions on the guest's path for a request (from proxy-component's
  recorded `trace.out`) were filled in by hand.
* `build.sh` builds `out/donut.wasm`. `GUEST` overrides the guest path
  (default `~/src/proxy-component/tests/rust.wasm`).
* Tested by `crates/donut-tests` (`fastly_guest`). `/` gives the same 200 response
  and 668-byte page as the recorded run, and `/missing` gives a 404. Both make the
  same 13 host calls, in the same order as the recording.
