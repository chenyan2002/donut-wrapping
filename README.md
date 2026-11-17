# Donut wrapping for Rust components

`donut::child!` lets a Rust component wrap other components (its *children*):
it implements the interfaces a child imports, calls the child's exports, and
leaves a typed hole that `wac plug` fills with the child component later.
See [`docs/design.md`](docs/design.md).

Status: being rebuilt from the [prototype branch](https://github.com/chenyan2002/donut-wrapping/tree/prototype)

```rust
wit_bindgen::generate!({ path: "wit", world: "example:app/host" });  // the donut's own world

donut::child!({
    name: "app",
    path: "wit",
    guest: "example:app/app",          // the hole's type
    import_passthrough: ["wasi:*"],    // everything else the guest imports is virtualized
    export_passthrough: ["example:app/info"],
});

// Implement `app::virt::…::Guest`, call `app::guest::…`, then:
app::export!(MyDonut);
```

Until step 3 lands, a crate using `child!` builds to a core module only:
`RUSTFLAGS=-Clink-arg=--skip-wit-component cargo build --target wasm32-wasip2`
(see `examples/spike/check.sh`).

```sh
cargo test            # donut-plan's snapshot and error tests
spike/run.sh          # the hand-written hole-mode spike, on Wasmtime ≥ 49
examples/spike/check.sh
```

Everything builds on stock crates: wit-bindgen 0.63 and wasm-tools 0.262.
