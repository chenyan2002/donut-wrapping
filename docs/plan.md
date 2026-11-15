# Implementation plan for `donut!`

Companion to `docs/design.md`. Section numbers (§) refer to the design doc.

## Status

| Phase | What | Where | Status |
|---|---|---|---|
| 0 | WAT spike: hand-assembled donut | `spike/` | done |
| 1 | `import_module_prefix` option | wit-bindgen, branch `donut` | done, committed |
| 2 | `DonutPlan` + `donut!` | this repo: `crates/`, `examples/http` | done, uncommitted |
| 3 | wit-component donut mode + `--donut-guest` | wasm-tools, branch `donut` | done, committed; fixtures pending |
| 4 | end-to-end tests, `wasm32-wasip2` guests | this repo, wasm-tools | done (uncommitted); CI pending |
| 5 | `forward:` and conversions (§6) | this repo | record-donut spike done (uncommitted); generated wrappers later |
| 6 | upstream PRs, wasm-component-ld integration | upstream | later |
| 7 | hole mode + wac plugging, p3, other languages | — | later |

The whole pipeline works for the §3.1 example:
1. `donut!` generates the donut's core module;
2. `wasm-tools component new --donut-guest` embeds the guest;
3. the component runs on Wasmtime 49 with the expected event log (`examples/http`).

## Branches and build base

| Repo | Branch | Base | Contents |
|---|---|---|---|
| `~/src/bytecodealliance/wit-bindgen` | `donut` | `main` @ 0.63.0 | `import_module_prefix` (`47bafd06`) |
| `~/src/bytecodealliance/wasm-tools` | `donut` | release 1.263.0 (`7a2fa422c`) | wit-component donut mode (`82e38e96d`) |
| `~/src/bytecodealliance/wasmtime` | `yan/ps1-memory-49` | v49.0.1 + 1 unrelated commit | runtime for tests (`target/debug/wasmtime`) |

* This repo's `Cargo.toml` patches `wit-bindgen*` and the wasm-tools crates to these
  checkouts (`[patch.crates-io]`).
* **Why wasm-tools is on 1.263.0:** wit-bindgen 0.63 builds against wasm-tools 0.263.
  Later wasm-tools `main` changes `Resolve::wasm_signature`, `Resolve::push_flat` and
  `SizeAlign::fill` to take a `WorldId` (the cm64 work, #2807/#2810), and
  `dummy_module` to take an `IndexType`. Adopting that in wit-bindgen means:
  * threading a world through `wit-bindgen-core`'s ABI API, a breaking change to
    that crate;
  * updating 31 call sites across 7 generators;
  * expecting small changes in generated code: a borrow of a resource the world
    exports becomes `Pointer`-typed in core signatures.

  We wait for wit-bindgen's own bump to the next wasm-tools release, then rebase.
* **The upstream wit-component PR** targets `main`, so the donut commit needs to be
  rebased onto `main` again for it. The difference is a few call sites: `wasm_signature`
  and `RequiredOptions::for_import` take a world, `validate_func` an index type, and
  shim types use `IndexType::encoder_val_type`. A version on `main` existed as
  `6ca1c2ff7` (in the reflog).
* **Wasmtime ≥ 49 is required.** 48.0.2 traps on the donut's first call into its
  nested guest (design §2.4).

## Phase 0: spike (`spike/`). Done

Details: `spike/README.md`.
* A real wit-bindgen guest, plus the donut's core module: real wit-bindgen bindings
  for World A, with World B written by hand.
* The outer component is hand-written in `donut.wat.in`, following design §5.1.
* `spike/build.sh` builds it two ways: hand-assembled (the golden output) and with
  the encoder. Both pass `spike/run.sh` on Wasmtime 49.

Findings that shaped later phases:
* Wasmtime ≥ 49 is required.
* Virtual instances need `encode_exports`' shim-component wrapping.
* A `wasm32-wasip2` core module linked with `--skip-wit-component` needs no adapter.
* Passthrough imports share the donut's own, semver-merged, host imports.

## Phase 1: wit-bindgen option. Done

`Opts::import_module_prefix: Option<String>`, library-only: `arg(skip)` for clap, and
not wired into `generate!`. Setting it:
* prefixes every import module of the world: interfaces, `$root` functions and types,
  `[resource-drop]` of imported resources, and stream/future payload intrinsics.
  `[export]…` modules are unchanged;
* suppresses the `component-type` section (both copies), the
  `__link_custom_section_describing_imports` helper, and the `__FORCE_SECTION_REF`
  statics.

With the option unset, generated code is byte-identical to before.

## Phase 2: `DonutPlan` + `donut!` (this repo). Done

```
crates/donut-plan   # lib: classify, check, build World A/B, generate, manifest
crates/donut        # proc macro `donut!`: parses its input with syn, calls donut-plan
examples/http       # §3.1 WIT, guest crate, donut crate (`server`), build.sh, run.sh
```

* **Classification:** guest imports in `virtualize` are virtual, other interfaces are
  passthrough. Rejected with an error: world-level and inline items, async functions,
  streams and futures.
* **Reverse dependency check:** a passthrough interface that uses a virtualized
  resource is an error, and the message names the interface to add to `virtualize`.
* **World A and World B** are written as WIT (`package donut:generated`, worlds
  `donut-a` and `donut-b`) and pushed into the `Resolve`, so wit-parser elaborates
  them.
* **World B generation:**
  * `import_module_prefix: "[guest-export]"`, output wrapped in `pub mod guest`;
  * dependency interfaces that are virtualized lose their functions, and their
    resources get type-level `with` to `crate::exports::…`;
  * dependency interfaces that are host imports get interface-level `with` to World
    A's import modules.
* **Manifest:** `wit_component::donut::encode_manifest`, emitted as a
  `#[link_section = "wit-bindgen-donut:v0"]` static.
* **Tests:** a snapshot of the `examples/http` expansion
  (`BLESS=1 cargo test -p donut-plan`), plus two configuration errors.
* **Checked end to end:** the donut built for both `wasm32-unknown-unknown` and
  `wasm32-wasip2`. In the wasip2 case, std's WASI imports at `@0.2.9` are merged
  with the guest's clock passthrough at `@0.2.0`.

Limitations:
* Macro options are only `path`, `world`, `guest`, `virtualize`.
* `donut!` must be invoked at the crate root (World B's `with` paths are `crate::…`).

## Phase 3: wit-component donut mode. Done (fixtures pending)

Implemented as described in design §5.1:
* `wit_component::donut`: the manifest (`encode_manifest`, extraction, decoding), and
  merging `G` and World B into the encoder's `Resolve`.
* `validation.rs`: `Import::GuestExportFunc` and `Import::GuestResourceDrop`.
* `encoding/world.rs` (`DonutWorld`):
  * binds the guest binary's imports to virtual exports, or to passthrough imports
    via `canonicalized_id_of`;
  * keeps functions the guest needs in passthrough import types;
  * records which exports are internal.
* `encoding.rs`, `fixup.rs`:
  * guest shim slots placed last, so phase-1 fixup fills a prefix of the table;
  * internal interface exports;
  * embedding, instantiating, aliasing and lowering the guest;
  * the phase-2 `wit-component-donut-fixup` module.
* CLI: `wasm-tools component new --donut-guest <GUEST.wasm>`.

Existing wit-component tests pass.

Remaining:
* `tests/components/donut-*` fixtures (`module.wat`, `module.wit`, guest, expected
  `component.wat`). Cover the basics, a string/list, a guest-defined resource,
  passthrough, and the error cases. Needed for the upstream PR.
* Readable errors for guest type mismatches. Today only missing imports and exports
  get a donut-specific message; type mismatches surface from the final validator.

## Phase 4: end to end. Next

1. **`wasm32-wasip2` guests. Done** (wasm-tools `donut` working tree, uncommitted).
   * Every guest import that isn't virtualized is now a passthrough, whether or not
     it's in `G`, bound by `canonicalized_id_of`. Its needed functions *and* types
     come from the decoded guest's import types (`PassthroughNeeds`), not from `G`.
   * A guest import the donut doesn't import is an error, with a hint.
   * `examples/http` now vendors the real WASI 0.2.12 `clocks`/`io` WIT (from
     wasmtime), and `build.sh` builds two variants. `out/donut.wasm` has guest and
     donut both on `wasm32-wasip2`, and `run.sh` also checks that the guest's
     `eprintln!` reaches the host. `out/donut-unknown.wasm` is the
     `wasm32-unknown-unknown` pair. Both pass on Wasmtime 49.
2. **Runtime tests. Done.** `cargo test -p donut-tests` (or `cargo test --workspace`)
   builds each example with its `build.sh` (into `target/donut-examples`) and runs it
   on Wasmtime 49.0.2 as a library:

   | Test | Example | What it checks |
   |---|---|---|
   | `http_wasip2` | `examples/http` (wasip2 pair) | re-entrance, exactly-once dtors (virtual and guest-defined), strings/lists, the guest's `std` WASI passthrough (stderr) |
   | `http_unknown_unknown` | `examples/http` (`wasm32-unknown-unknown` pair) | the same without WASI |
   | `shapes_records_and_middleware` | `examples/shapes` | a borrow inside a record (both directions), an owned handle inside a returned record and its dtor, `handler` imported *and* exported by the guest, re-lending a borrow the guest received |
   | `guest_start_calls_virtual_function` | `examples/shapes` + `init-guests/tick.wat` | a guest's core start function calling a virtual function during instantiation (phase-1 fixup done first) |
   | `guest_start_calling_back_into_guest_traps` | `examples/shapes` + `init-guests/tick-reentrant.wat` | ... that calls back into the guest: traps with `uninitialized element` at `guest-export-ticks` |
   | `fastly_guest` | `examples/fastly` + proxy-component's `tests/rust.wasm` | a real Fastly Compute guest (60 virtual functions, a versioned package, resource aliases, resources in `result`s); the host calls match proxy-component's recorded trace; 200 and 404 responses; the guest's stdout via WASI passthrough. Skipped if the guest isn't found (`GUEST`) |

   The WAT guests exist because Rust components can't run code at instantiation.
   Tools added: `cargo run -p donut-plan --example expand` prints a formatted
   expansion, and `tools/stubs.py` turns it into `unimplemented!()` trait impls.
3. **CI.** Blocked on the local `[patch]` paths. Use git dependencies on the fork
   branches, or wait for upstream releases.

## Phase 5: `forward:` and conversions (§6)

**Record-donut spike: done** (`examples/record`, test
`crates/donut-tests/tests/record.rs`). proxy-component's record mode works as a
single donut with the same world as the app it wraps (`import kv; export app`):
* the app's `kv` is virtualized, and the virtual `store` wraps a host `store` (owned,
  or lent for one call through `ManuallyDrop`);
* `app` is exported publicly and forwards to the app's `app`, and the donut's
  `session` wraps the app's `session`;
* every call in both directions is printed.

On a wasmtime host `kv`, the bare app and the recorder give identical results and
make identical host calls. The recorder's log has every export and import in
order, interleaved on the same stack. Compared with proxy-component, it needs no
imports/exports proxy pair, no `proxy:conversion` interface, no separate recorder
component, and one fewer hop per call.

It needed two changes:
* DonutPlan exports a **renamed copy** (`virtual-<ns>:…`) of a virtualized
  interface when the donut's own world imports or exports it (design §4 step 2).
* The manifest's virtual list is now (guest import → donut export) pairs, in
  donut-plan and in the wasm-tools `donut` working tree (uncommitted).

The wrappers (`VStore`, `Session`) and the `lend` helper are hand-written in the
spike. They're exactly what generated `forward:` code should produce.

Remaining:

* Generate `VirtR` wrappers, `RHooks` default methods, and deep `ToHost`/`FromHost`
  conversions, both between World A and World B and between host and virtual types.
  Emit them as source from the `Resolve`, like proxy-component's `ToProxy`
  (`~/src/proxy-component/src/traits/proxy.rs`).
* Exit: a passthrough `wasi:http/types` donut forwards a request end to end.
* Later, consider factoring the conversion generator into a crate shared with
  proxy-component.

## Phase 6: upstream and integration

1. wit-bindgen PR: `import_module_prefix` (branch `donut`, ready).
2. wit-component PR: donut mode and `--donut-guest`. Needs a rebase onto `main` (see
   "Branches and build base") and the phase-3 fixtures.
3. wasm-component-ld: detect the manifest and add a `--donut-guest` link arg, so
   `cargo build` alone produces the final component. Not checked out locally yet.
4. After wit-bindgen moves to the next wasm-tools release: rebase the wasm-tools
   `donut` branch onto `main` and drop the 1.263.0 base.
5. Decide whether `donut!` moves into wit-bindgen behind an experimental feature, and
   expose the remaining `generate!` options.

## Phase 7: later

* **Hole mode** (§7): the encoder emits a component-typed `guest` import. Plugging a
  guest in is ordinary composition, so it's a wac feature (`wac-types` already has
  `ItemKind::Component`).
* **The p3 extension** (§2.4).
* **Other source languages**, using the same manifest.

## Risks to watch

* **Guest `_initialize` calling virtual functions** relies on fixup phase 1 having run
  first (§5.1). It is covered by a WAT guest's start function
  (`guest_start_calls_virtual_function`), since Rust components have no
  `_initialize`.
* **Dead-code warnings and doc noise** from passthrough bindings in World A.
* **Two `_rt` modules** (crate root and `guest::_rt`). Expected and harmless; pinned by
  the snapshot.
* **`donut!` couples to wit-bindgen's module paths.** `module_path`/`camel` in
  `donut-plan` mirror wit-bindgen's `compute_module_path` and `to_upper_camel_case`.
  The snapshot test catches drift.
