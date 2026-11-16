# Implementation plan for `donut!`

Companion to `docs/design.md`. Section numbers (§) refer to the design doc.

## Status

| Phase | What | Where | Status |
|---|---|---|---|
| 0 | WAT spike: hand-assembled donut | `spike/` | done |
| 1 | `import_module_prefix` option | wit-bindgen, branch `donut` | done, committed |
| 2 | `DonutPlan` + `donut!` | this repo: `crates/`, `examples/` | done, uncommitted |
| 3 | wit-component donut mode + `--donut-guest` | wasm-tools, branch `donut` | done, committed; fixtures pending |
| 4 | end-to-end tests, `wasm32-wasip2` guests | this repo, wasm-tools | done (uncommitted); CI pending |
| 5 | `forward:` and conversions (§6) | this repo | first cut of generated forwarding done (uncommitted) |
| 6 | upstream PRs, wasm-component-ld integration | upstream | later |
| 7 | hole mode + wac plugging, p3, other languages | — | later |

The whole pipeline works for the §3.1 example:
1. `donut!` generates the donut's core module;
2. `wasm-tools component new --donut-guest` embeds the guest;
3. the component runs on Wasmtime 49 with the expected event log (`examples/virtualize`).

## Examples

| Example | What it shows | Tests (`crates/donut-tests`) |
|---|---|---|
| `examples/secrets` | the README example: an app wrapped for a host with a vault and no filesystem. `forward` (`types`, `handler`), `virtualize` (`outgoing`, the token gate), `mock` (`fs`), and passthrough (WASI) in one donut, with a world different from the guest's | `app_gets_tokens_it_never_sees` |
| `examples/virtualize` | plain virtualization: re-entrance, exactly-once destructors, strings and lists, a guest-defined resource, a borrow and an owned handle inside records, `relay` imported and exported by the guest (a middleware), and passthrough of the guest's clock and `std` WASI. `init/` has two WAT guests whose start functions call the donut. | `virtualize`, `guest_start_calls_virtual_function`, `guest_start_calling_back_into_guest_traps` |
| `examples/forward` | generated forwarding around an app with the same world as the donut: `recorder` (hooks observe every call) and `interceptor` (two functions overridden) | `recorder_is_transparent_and_sees_every_call`, `overridden_hooks_change_behaviour` |
| `examples/fastly` | a mock Fastly Compute host around a real Fastly guest (proxy-component's `tests/rust.wasm`): 60 virtual functions | `fastly_guest` |

* Every example's `build.sh` sources `examples/common.sh`: `guest`, `wat_guest`, and
  `donut` (core module plus `component new --donut-guest`).
* Guests and donuts are built for `wasm32-wasip2`.
* `spike/` keeps the hand-written references: the phase-0 component in WAT, and
  `spike/recorder`, the hand-written recorder that `forward:` now generates.

`examples/virtualize` merges the former `examples/http` and `examples/shapes`, and
`examples/forward` was `examples/record`. The phase notes below mention the old
names.

## Branches and build base

| Repo | Branch | Base | Contents |
|---|---|---|---|
| `~/src/bytecodealliance/wit-bindgen` | `donut` | `main` @ 0.63.0 | `import_module_prefix` (`47bafd06`) |
| `~/src/bytecodealliance/wasm-tools` | `donut` | release 1.263.0 (`7a2fa422c`) | wit-component donut mode (`82e38e96d`) |
| `~/src/bytecodealliance/wasmtime` | `yan/ps1-memory-49` | v49.0.1 + 1 unrelated commit | runtime for tests (`target/debug/wasmtime`) |

* This repo's `Cargo.toml` patches `wit-bindgen*` and the wasm-tools crates to the
  pushed `donut` branches as **git** dependencies (`github.com/chenyan2002/{wit-bindgen,wasm-tools}`,
  `[patch.crates-io]`), so others can build it. Users of `donut` have to copy that
  block, since patches only apply in the root workspace (see `README.md`).
* The `wasm-tools` CLI comes from the same branch (`cargo install --git … --branch donut
  wasm-tools --locked`). `examples/common.sh` uses `wasm-tools` on PATH (or
  `WASM_TOOLS`), and fails with install instructions if it lacks `--donut-guest`.
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
* **Tests:** snapshots of the `examples/virtualize` and `examples/forward`
  expansions (`tests/snapshots/{virtualize,forward}.rs`)
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
   Tool added: `cargo run -p donut-plan --example expand` prints a formatted
   expansion (`FORWARD`, `MOCK`, `HOOKS` for generated impls).
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
  It now always does, copying only the listed interfaces.
* The manifest's virtual list is now (guest import → donut export) pairs, in
  donut-plan and in the wasm-tools `donut` working tree (uncommitted).

The wrappers (`VStore`, `Session`) and the `lend` helper are hand-written in the
spike. They're exactly what generated `forward:` code should produce.

**Generated forwarding: first cut done** (`crates/donut-plan/src/forward.rs`,
`examples/record/forwarder`). The recorder becomes a `donut!` call with
`forward: ["example:rec/kv", "example:rec/app"], hooks: Recorder`, plus three
logging hooks (`before`, `after`, `dropped`). The test checks it's as transparent
as the hand-written recorder (same results, same host calls) and that the hooks
see every call and drop in both directions. Generated pieces:
* **Forwarded virtual interface** (a guest import, here `kv`):
  * it's virtualized implicitly, and World A imports the host's copy (so the
    virtual side is the renamed copy);
  * each resource gets a wrapper over the host's resource, owned or lent
    (`ManuallyDrop`), with `lend`;
  * the trait impls forward to the host.
* **Forwarded export** (exported by both the donut's world and the guest, here
  `app`): each resource gets a wrapper over the guest's, and the trait impls
  forward to the guest.
* **Hooks:** a `forward::Hooks` trait (`before(call, args)`, `after(call, ret)`,
  `dropped`), whose defaults make forwarding transparent.
* **Conversions** are generated from the WIT structure, and only for values that
  contain forwarded resources: own, borrow, and their `option`/`list`/`result`/
  tuple containers.
  * Resource-free values pass through. World A is generated with
    `merge_structurally_equal_types` when forwarding.
  * Import calls go through `forward::Arg`, whose impl is picked by type inference
    (`String → &str`, `Vec<T> → &[T]`, `T → &T`, `T: Clone → T`, `Option` forms).
    So wit-bindgen's import-parameter rules aren't reproduced.
  * Trait impl signatures use wit-bindgen's export-side (all-owned) rule.

**Mocking: done.** `mock: [...]` virtualizes guest imports with generated impls
whose `Hooks` methods default to `unimplemented!()`. The panic message names the
hook to override. Resources get a `Mock<Ns><Pkg><Iface><Res>` wrapper over the
state type declared in `mock` (`{ "<iface>": { <res>: <Type> } }`); undeclared
resources get an uninhabited wrapper and can't be created.
`examples/fastly` now uses it: the 60-function hand-written `fastly.rs` became 13
overrides.

**Per-method interception: done.** `forward::Hooks` has one overridable method
per forwarded function (`example_rec_kv_store_get`, `example_rec_app_session_greet`, ...). Each defaults to
a generated forwarding function (`forward::example_rec_kv_store::get`) that an override can
call. The generated impls run `before` and `after` around it.
`examples/record/interceptor` and the `overridden_hooks_change_behaviour` test
cover it: a hidden key never reaches the host, and a decorated guest result is
returned to the host.

**Forwarding gaps: closed** (`examples/forward`; the recorder's full log is
`examples/forward/recorder.expected`):
* **World B reuses World A's types** (`Plan::world_b_shared_types`, used both by
  World B generation and by `forward.rs`). A virtualized dependency's types are
  shared with World A's copy unless they contain a borrow of a virtual resource.
  A forwarded export's types are shared unless they contain any handle. So
  resource-free values need no conversion in any direction.
* **Records and variants that contain forwarded resources** are converted by
  generated code, with source and destination type paths per direction (virtual ↔
  host for forwarded imports, host ↔ guest for forwarded exports). Records and
  tuples are rebuilt from *places* (`p0.field`), so borrows in them stay borrows
  of the caller's value.
* **Borrows nested in records**, including lending a host borrow inside a record
  to the guest (`summarize(view)`: `let lent0 = …lend(p0.s)` before the call).

The example covers:
* a list of owned stores (`open-all`);
* owned stores in a record (`open-pair`);
* an owned store in a variant (`find`);
* a borrow in a record (`dump`);
* a resource-free record (`entry`);
* `summarize`, which takes a lent borrow inside a record and returns an app
  record holding a `kv` record of owned host stores.

The recorder stays transparent, and the interceptor still only changes what it
overrides.

**Borrows inside containers.** wit-bindgen's `RBorrow<'a>::get(&self)` returns
`&'a T`, so converting a borrow works wherever the value is rebound:
* **guest → host** (virtual borrow → host borrow) works anywhere: lists, options,
  results, variants (`kv.count(list<borrow<store>>, option<borrow<store>>)`);
* **host → guest, borrows of guest-wrapping resources**, likewise;
* **host → guest lending** (a host borrow presented to the guest) needs a
  temporary virtual resource that outlives the call. It's supported at the top
  level, in records, and directly in `option<borrow<..>>` and
  `list<borrow<..>>` (temporaries plus a vector of references). The option
  case is tested by `app.tally`.

Still rejected, with an error (covered by a `donut-plan` test):
* lending deeper inside containers (e.g. `list<record { borrow<..> }>`);
* world-level items;
* a forwarded interface using a resource from a virtualized but unforwarded one.

proxy-component supports guest → host borrows in lists the same way
(`ToProxy for Vec<T>` plus `get`), but not host → guest lending at all (its
exports proxy has `&Host → &Wrapped` as `unreachable!()`).

**Upstream wit-bindgen bug** (released 0.63.0 too): a Rust *export* whose
parameter is `list<borrow<R>>`, with `R` an *imported* resource, doesn't compile.
The lifting code declares one `let handleN;` slot outside the list loop and
reassigns it per element while pushing `&handleN` (E0506). It's also wrong in
principle, since every element would point at one slot. That blocks lending
inside `list<borrow<..>>` from being exercised. A donut exporting such a
function would also hit the bug in its own World A bindings. Minimal repro: an
interface `app { use kv.{store}; tally: func(stores: list<borrow<store>>) -> u32; }`
in `world w { import kv; export app; }`. Worth filing upstream (or fixing in the
wit-bindgen checkout).

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
