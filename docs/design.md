# Design: `donut!` — donut wrapping for Rust components (WASI p2)

Status: working end to end for the §3.1 example (embed mode). Revised after comparing
with `docs/p2-donut-wrapping-astra.md` and reviewing
[proxy-component](https://github.com/chenyan2002/proxy-component). Implemented:
* the wit-bindgen option (§4 step 3);
* `DonutPlan` and the `donut!` macro (§4, this repo);
* wit-component's donut mode and `wasm-tools component new --donut-guest` (§5).

`examples/http` builds with `donut!` and runs on Wasmtime 49. `spike/` is the
hand-assembled golden output for §5.1. `docs/plan.md` has the status, the branches
and the follow-ups.
Scope: component model p2 only (sync functions, resources; no `async`, `stream`, `future`)
Reference: [Explainer § Virtualization and "donut wrapping"](https://github.com/WebAssembly/component-model/blob/new-explainer/design/mvp/Explainer.md#virtualization-and-donut-wrapping)

## 1. Goal

Let a Rust crate act as `$HttpServer` from the Explainer: a component that

1. **defines** (virtualizes) some or all of the interfaces a *guest* component imports,
   usually by wrapping the real host resources;
2. **instantiates the guest internally**, passing in its virtual interfaces;
3. **calls the guest's exports** from its own code, while the guest calls back into the
   donut's virtual interfaces on the same stack (re-entrantly);
4. exposes its own unrelated world to the host (e.g. `import net; export serve`).

The result should match the Explainer's hand-written WAT, with no new component-model
features. The Rust author should only write a macro invocation plus trait impls, and
build with `cargo build --target wasm32-wasip2`.

## 2. Key constraints found while designing

These constraints shape the rest of the design, so they come first.

### 2.1 The inverted guest world cannot be written as WIT

Inverting `http-guest` naively gives:

```wit
world inverted {
  export types;      // guest imported it, so we define it
  export outgoing;   // guest imported it
  import handler;    // guest exported it  <-- uses types.{request,response}
}
```

In WIT, an *import's* type dependencies always resolve to *imports*. wit-parser therefore
elaborates `import handler` into an extra `import types`, a second, host-side `request`
that has nothing to do with the exported (virtual) `request`. The component model has the
same rule: a component's imports can't mention types it defines. In the Explainer,
`receive` is never an import of `$HttpServer`. It is an export of a **nested instance**
`$guest` that was instantiated with `$HttpServer`'s locally defined `$request`.

**Consequence:** "invert the WIT" happens **in memory on a `Resolve`** and yields **two
worlds plus a mapping**, not one WIT file:

* **World A, the donut world.** The donut's outer world, plus the guest's virtualized
  imports turned into exports (marked *internal*), plus the guest's passthrough imports
  as ordinary imports. Normal wit-bindgen output.
* **World B, the guest-exports world.** `import <each guest export>`. Its bindings are
  generated with type remapping, so the resources point at World A's *export-side* Rust
  types (or at host import types, for passthrough interfaces). Its core imports use a
  reserved module prefix and are not part of World A's `component-type` section.

### 2.2 The cycle can only be broken inside a single component scope

The cycle is donut core instance → guest → donut core instance. Breaking it requires a
core `funcref` table that is patched *after* the guest is instantiated, within the scope
that holds the donut's core instance. Composing two finished components (wac, `wasm-compose`)
can't do this:

* **Before encoding, the donut isn't a component yet.** It is a core module with
  `[guest-export]…` imports plus metadata. Turning it into a component (lifts, lowers,
  shims, fixups) *is* the step that must change, and that step is `ComponentEncoder`'s.
  Composition tools take finished components as input.
* **No finished donut component could be valid anyway.** To call
  `guest.handler.receive(own<request>)`, the donut would have to *import* it, but
  `request` is a resource the donut defines, and imports can't mention a component's
  own types (2.1).
* **The cycle needs core-level patching.** Component-level funcs are immutable and
  funcrefs can't cross component boundaries, so composition can't express a table
  patched after the guest is instantiated.

So **the component encoder must change**. A post-processing tool around a finished
component can't do it. Composition still has a role *after* encoding: plugging a guest
into a hole-mode donut is ordinary composition and belongs in wac (§7). §9.2 looks at
the alternative of having wac generate the linkage itself around an ordinary donut
component, and why it isn't pursued.

### 2.3 wit-component already contains the LinkageTable

| Explainer WAT | Already in `wit-component` (`encoding.rs`) |
|---|---|
| `$LinkageTable` (funcref table + `call_indirect` trampolines) | the *shim module*, `encode_shim_instantiation` |
| `request-dtor` / `response-dtor` slots | `ShimKind::ResourceDtor` |
| lowered imports that need `$Main`'s memory/realloc | `ShimKind::IndirectLowering` |
| `$PatchLinkageTable` (`elem` into the table) | the *fixup module*, `fixup::FixupModule::instantiate` |
| `resource.new/rep/drop` builtins for `$request` | `[export]iface` / `[resource-new]R` … intrinsics |

What's missing:

* **(a)** a lowering whose source is a nested guest instance's export rather than a
  component import, plus late-bound `resource.drop` for resources the *guest* defines
  (those types exist only after the guest is instantiated);
* **(b)** instantiating a guest component with the lifted exports;
* **(c)** splitting the fixup into two phases.

### 2.4 Re-entrance

In the current Canonical ABI reference (`definitions.py`), a **sync** lift does no
backpressure or exclusivity check, so `serve → guest.receive → donut.send` is legal. Runtimes
lag behind. **Checked in the phase-0 spike:**
* Wasmtime 49.0.1 runs the nested, re-entrant layout, including re-entrant dtors.
* Wasmtime 48.0.2 traps with "cannot enter component instance" already on the donut's
  *first* call into its nested guest, before any re-entrance. It rejected any call
  from a parent instance into its child.

So **Wasmtime ≥ 49 is required**, and the §9.1 fallback isn't needed.

**p3 outlook (out of scope here, but it constrains future extensions).** p3 still has no
re-entrance trap. Instead, entry into an `async func` export is gated
(`Task.enter_implicit_thread`). If the export is lifted sync or `async`+`callback`
(`needs_exclusive()`), the call waits while the instance's `exclusive_thread` is held:

| Type of the virtual export the guest calls back into | How it is lifted | While the donut is mid-call into the guest |
|---|---|---|
| plain `func` | any | enters (no gate) |
| `async func` | stackful (`async`, no `callback`) | enters (no exclusive lock needed) |
| `async func` | sync, or `async` + `callback` | waits for `exclusive_thread` |

The last row deadlocks. The donut's task keeps the exclusive lock during a **sync**
lowering of the guest export, because the lock is released only when a callback returns
`WAIT`/`YIELD`. Meanwhile the guest's callback waits for that lock. A p3 extension must
therefore do one of the following:

* async-lower guest exports in World B whenever they are `async func`, so the donut
  yields and releases the lock; or
* lift virtual `async func` exports stackful; or
* keep the virtual interfaces as plain `func`.

This doesn't affect the p2 design, where every function is a plain `func`.

## 3. User-facing API

### 3.1 Example WIT (the Explainer example written as interfaces)

```wit
package example:http;

interface types {
  resource request  { constructor(path: string); path: func() -> string; }
  resource response { constructor(status: u16, body: list<u8>); status: func() -> u16; }
}
interface outgoing { use types.{request, response}; send: func(r: request) -> response; }
interface handler  { use types.{request, response}; receive: func(r: request) -> response; }

world http-guest {            // what the guest targets ($HttpGuest)
  import outgoing;            // pulls in `types`
  import wasi:clocks/monotonic-clock@0.2.0;
  export handler;
}

world socket-server {         // the donut's own outer world ($SocketServer)
  import example:net/sockets;
  export serve: func();
}
```

### 3.2 Macro invocation

```rust
donut::donut!({
    path: "wit",
    world: "example:http/socket-server",   // donut's outer world
    guest: "example:http/http-guest",      // the hole's shape
    // Guest imports the donut implements. Every other guest import is
    // passed through from the host unchanged (added to the donut's imports).
    virtualize: ["example:http/types", "example:http/outgoing"],
    // Planned: forwarding wrappers (see §6) when a virtualized interface has the
    // same shape as a host interface.
    // forward: { "example:http/types": "example:http/types" },
    // Planned: the ordinary `generate!` options (ownership, with,
    // additional_derives…), applied to World A.
});
```

Implemented today: `path` (a string or a list of strings, relative to the crate's
manifest directory; default `"wit"`), `world`, `guest`, `virtualize`. The macro must be
invoked at the crate root, because World B's remap paths are `crate::…` (§4 step 3).
The macro lives in the `donut` crate for now; `wit_bindgen::donut!` is a later
decision (§10.1).

### 3.3 Generated module layout

```
crate::example::net::sockets            // host import (World A, normal)
crate::wasi::clocks::monotonic_clock    // passthrough import (World A); also forwarded to guest
crate::exports::example::http::types    // virtual resources: Guest, GuestRequest, Request, …
crate::exports::example::http::outgoing // virtual `send`
crate::guest::example::http::handler    // guest's exports, called through the linkage table (World B)
crate::Guest                            // donut's own root exports (`serve`)
export!(...)                            // same as generate!
```

### 3.4 User code

```rust
use exports::example::http::types::{self as vtypes, Request, Response};

struct Server;

impl Guest for Server {
    fn serve() {
        let sock = example::net::sockets::Socket::new();
        loop {
            let conn = sock.accept();
            // Create a *virtual* request. The handle lives in the donut's table.
            let req = Request::new(MyRequest::parse(&conn));
            // Call into the guest. It may call `send` and run dtors before returning.
            let resp: Response = guest::example::http::handler::receive(req);
            conn.write(&resp.get::<MyResponse>().encode());
        }
    }
}

impl vtypes::Guest for Server { type Request = MyRequest; type Response = MyResponse; }
impl exports::example::http::outgoing::Guest for Server {
    fn send(r: Request) -> Response { /* uses example:net/sockets */ }
}
export!(Server);
```

Building gives a component with the outer type from the Explainer:
`import example:net/sockets`, `import wasi:clocks/...`, `export serve`. The guest is
either embedded or left as a component-typed import ("the hole"); see §7.

## 4. Inversion algorithm (macro side)

Input: `Resolve`, the outer world `W`, the guest world `G`, and the `virtualize` set `V`.

1. **Classify the guest imports.** For each `(key, item)` in `G.imports`:
   * interface in `V` → **virtual**;
   * interface not in `V` → **passthrough**;
   * world-level func/type in `V` (e.g. the Explainer's root-level `send`, `request`)
     → move into a synthetic interface `donut:internal/guest-root` and mark it virtual.
     Rust's generator can't export world-level resources (`unimplemented!("resource exports
     from worlds")`), and this also avoids name clashes with `W`'s own root exports.
   * **Check dependencies in both directions**, over the closure through `use`, aliases,
     and aggregates:
     * *Forward:* if a virtual interface uses types from interface `X`, then either
       `X ∈ V`, or `X` is passthrough and its types come from the host. (A virtual
       interface may legitimately mention host resources.)
     * *Reverse:* a passthrough interface must not mention a virtualized resource. Host
       `outgoing.send(host-request)` can't satisfy the guest's
       `outgoing.send(virtual-request)` once `types.request` is virtualized. This is a
       configuration error naming the interface to add to `V`. Never widen `V` silently.
       Resource-free dependencies don't trigger it.
2. **Build World A**:
   `W.imports ∪ passthrough(G) ∪ {import X | X needed by a virtual interface's use-closure}`,
   `W.exports ∪ {export v | v ∈ virtual}`. A world may both import and export the same
   interface (needed when the donut wraps the real `example:http/types`).
   As implemented, World A and World B are written as WIT in a synthetic package and
   pushed into the `Resolve`, so wit-parser elaborates their dependencies:
   ```wit
   package donut:generated;
   world donut-a { include example:http/socket-server; import wasi:clocks/monotonic-clock@0.2.0;
                   export example:http/types; export example:http/outgoing; }
   world donut-b { import example:http/handler; }
   ```
   Run `wit_bindgen_rust` normally. This emits the usual
   `component-type:wit-bindgen:…` section for World A.
3. **Build World B:** `import e` for every `e ∈ G.exports`; world-level guest exports go into
   `donut:internal/guest-root-exports`. Run `wit_bindgen_rust` with the settings below.
   Output is wrapped in `pub mod guest { … }`.

   **Remap resource types, never whole interfaces.** Use *type-level* `with` keys
   (`full_wit_type_name`, `crates/rust/src/lib.rs:1321`): each virtual resource
   `example:http/types/request` ↦ `crate::exports::example::http::types::Request`, and
   each passthrough resource ↦ its World-A host binding. For own handles and direct
   borrows, the remapped type works as is: export-side resource structs have the same
   handle API as import-side ones (`from_handle`, `take_handle`, `handle`;
   `crates/rust/src/interface.rs:2925-2940`), which is exactly what `HandleLower`/`HandleLift`
   emit for imports (`crates/rust/src/bindgen.rs:379-420`). Interface-level `with` is
   ruled out for two reasons:
   * **It drops functions.** `import_interface` returns early for a remapped interface
     (`crates/rust/src/lib.rs:1342`). If the guest both imports and exports an interface
     (e.g. a middleware guest with `handler` on both sides), the bindings for calling the
     guest's export of it would never be generated.
   * **It reuses export-side records whose borrow fields can't be lowered.** In World A,
     a `record { r: borrow<request> }` prints its field as `RequestBorrow<'a>`, because
     `is_exported_resource` is true there (`interface.rs:3218`). That type holds a rep
     pointer and has no `.handle()`. When World B regenerates the record instead, the
     resource is not exported *in World B's generator run*, so the field prints as
     `&'a exports::…::Request`: an outbound borrow view that lowers correctly.

   Consequences of resource-only remapping:
   * Aggregates in the dependency interfaces (`types`) are **generated twice**: inbound
     views in World A (`exports::…`) and outbound views in World B (`guest::…`).
     Generated conversions (§6's `ToHost`/`FromHost` machinery, run between A and B)
     move own handles and turn `&exports::Request` borrows into the same type. Shared
     paths are still fine for types that contain no resources and match in ownership mode.
   * World B would also emit import-side functions for the dependency interfaces,
     including inherent `impl Request { fn new(..) }` methods. Those collide with the
     export-side `Request::new<T>` and `get`. Suppress them by removing those functions
     from the dependency-only interfaces in DonutPlan's private copy of the `Resolve`
     before generating World B. No wit-bindgen option is needed. The fallback is
     `skip`, which matches `func.name` globally (e.g. `[constructor]request`;
     `interface.rs:163,819`).
   * **Host (passthrough) dependencies are the exception.** An interface that World B
     only needs as a dependency, and that World A *imports* from the host (e.g. a
     `wasi:io/streams` used by a guest export), is the same import-side binding in
     both worlds. It is remapped with interface-level `with` to World A's import
     module. Neither objection above applies: it has no functions World B needs and
     no export-side types.
   * **Resources the guest defines** (declared in a guest-export interface) are not
     remapped. They are generated as ordinary opaque imported resources in `guest::…`,
     and their `[resource-drop]` imports go through the `[guest-export]` module too, so
     the encoder must late-bind them (§5.1).
   * The restriction in §8 still holds: an incoming `RequestBorrow` has no handle in the
     donut's table, so it can't be relayed to a guest export.

   **Symbol routing and metadata:** one library-only wit-bindgen-rust option, added for
   this design: `Opts::import_module_prefix: Some("[guest-export]")`. It:
   * **Prefixes every import module of the world:** interface and `$root` functions,
     `[resource-drop]` of imported resources (including guest-defined ones), and
     stream/future payload intrinsics. Core imports become e.g.
     `"[guest-export]example:http/handler" "receive"`. `[export]…` modules are unchanged.
   * **Suppresses the world's `component-type` section and its link helpers,** because
     prefixed imports are not component imports. If the section were kept,
     wit-component would merge it and expect the guest's exports as host imports (the
     2.1 problem). `type_section_suffix` only renames the section, and
     `disable_custom_section_link_helpers` only drops the keep-alive helpers, so neither
     could do this.

   The prefix is what lets the encoder tell a call to the guest's export apart from a
   host import of the same interface. An example is a donut that imports the host's
   `handler` and also calls the guest's `handler`. Without the prefix, both would be the
   same `"example:http/handler" "receive"` symbol. This is the same convention as
   `[export]` for export-side intrinsics.
4. **Emit the donut manifest** as custom section `wit-bindgen-donut:v0`. The name must
   **not** start with `component-type`: wit-component merges every such section into the
   world (`metadata.rs:237`). Contents, as implemented by
   `wit_component::donut::encode_manifest`
   (`u8 version | vec<u8> G | vec<string> virtualized`):
   * a version byte (`0`);
   * the guest world `G`, in wit-component's metadata encoding (`metadata::encode`).
     The encoder uses it to produce the guest's component type, rebuild World B, and
     check an embedded guest against it. Hole mode needs it because there is no guest
     binary;
   * the list of virtualized guest imports. It must be explicit: when the donut wraps
     the real interface, World A both imports and exports it.

   Everything else is derived by convention, so it isn't serialized:
   * passthrough = the remaining guest imports, bound to World-A imports by
     `Resolve::canonicalized_id_of` (see §5.1);
   * the `[guest-export]` prefix;
   * the `guest-root` naming;
   * World B (its function list and ABI options), built from `G.exports`.

Steps 1–4 form a `DonutPlan` library that a proc macro (`donut!`) and a CLI both drive.
Where each piece lives is covered in §10. Keep the mapping between World A, World B,
and `G` as explicit port mappings computed **before** generation. `generate_nominal_type_ids` (`crates/core/src/lib.rs:28`) rewrites arena
IDs during each generator run, so IDs can't be matched across views afterwards.

## 5. The linkage table (encoder side)

The Rust code never touches a table. Calls to `guest::…::receive` are plain wasm imports
from `[guest-export]…`. The encoder satisfies them with shim trampolines, exactly as
`$LinkageTable` does. A Rust-level table that patches `__indirect_function_table` was
rejected: it needs `--export-table`/`--growable-table`, slot numbers the linker controls,
and patching memory at runtime, and it still needs the encoder to lower the guest
export.

### 5.1 New "donut mode" in `wit-component::ComponentEncoder`

Turned on by a `wit-bindgen-donut:v0` section in the main module
(`wit_component::donut`). The guest to embed is given with
`ComponentEncoder::donut_guest(&[u8])`, or `wasm-tools component new --donut-guest`.
A manifest without a guest, or a guest without a manifest, is an error. Hole mode
(§7) would add a variant that emits a component-typed import instead.

Changes to validation (`validation.rs`):

* New import classification `Import::GuestExportFunc(key, interface, func)` for core
  imports from `[guest-export]…` modules. ABI info comes from World B, which the
  encoder builds by importizing `G` from the manifest. Only interfaces `G` exports are
  accepted (World B's elaboration also pulls in dependencies like `types`).
* New import classification `Import::GuestResourceDrop(key, resource)` for
  `[guest-export]iface` `[resource-drop]R`, where `R` is a resource the guest defines.
  Its type comes from the guest instance, so it is late-bound like a function.
* The guest's imports are taken from the *guest binary* (decoded), since those are the
  names `instantiate` must supply. Every import that isn't virtualized is a
  passthrough, bound to the donut's import of a semver-compatible interface, whether
  or not it is in `G`. Imports outside `G` are typically added by the guest's
  toolchain: a `wasm32-wasip2` Rust guest imports `std`'s `wasi:cli/*` and
  `wasi:io/*`. The functions and types the guest needs are taken from its own import
  types, and the donut's import must have them. A guest import the donut doesn't
  import is an error, with a hint to also build the donut for `wasm32-wasip2`.
* Virtual World-A exports are classified as **internal**: they are lifted but not exported.
* Passthrough imports are kept even if the donut's core module doesn't use them.
  wit-component normally drops unused imports, but the guest needs them.
* Semver import merging is unchanged. It only touches imports (virtual interfaces are
  World-A exports) and keeps the highest version. A passthrough import often coincides
  with one the donut's own Rust `std` already makes (e.g. the guest's
  `wasi:clocks/…@0.2.0` and std's `@0.2.9`). The guest's import is bound to the merged
  import by `Resolve::canonicalized_id_of` (both are `…@0.2`). The nested `instantiate`
  uses the guest's own import name.

Encoding order, extending `encode_with_fixups` / `encode_core_instantiation`:

| # | Step | Explainer equivalent | New? |
|---|---|---|---|
| 1 | `encode_imports`: World-A imports (host + passthrough) | `(import "net" …)` | existing |
| 2 | `encode_core_modules` | `(core module $Main …)` | existing |
| 3 | shim instance: `IndirectLowering` for host imports, `ResourceDtor` slots, **plus a `GuestExportFunc` slot for every `[guest-export]` function and a `GuestResourceDrop` slot for every guest-defined resource drop, always indirect, placed after all other slots** | `$LinkageTable` | add two `ShimKind`s |
| 4 | `declare_types_for_imported_intrinsics`: defines virtual resources with dtors from shims | `(type $request (resource (rep i32) (dtor …)))` | existing |
| 5 | `instantiate_main_module` | `(core instance $main …)` | existing |
| 6 | **fixup phase 1**: host lowerings, adapters, dtors, `_initialize` | the `elem` segment in `$Main` | split |
| 7 | lift internal virtual exports and build one component instance per interface through `encode_exports`' shim component, without exporting | `(func $send (canon lift …))` | refactor of `encode_exports` |
| 8 | guest: embed `(component …)` or `(import "guest" (component <type of G>))` | `(import "guest" (component $Guest …))` | new |
| 9 | `instantiate` the guest: virtual interface → instance from 7; passthrough → host import instance from 1; `guest-root` items flattened to world-level names | `(instance $guest (instantiate $Guest (with …)))` | new |
| 10 | alias the guest exports and `canon lower` them with main's `memory`, `cabi_realloc`, `string-encoding`; alias guest-defined resource types and emit `canon resource.drop` for them | `(core func $lowered-receive (canon lower …))` | new source for an existing lowering path |
| 11 | **fixup phase 2**: a small `wit-component-donut-fixup` module patches the `GuestExportFunc` and `GuestResourceDrop` slots | `$PatchLinkageTable` | split |
| 12 | `encode_exports` for World-A's public exports | `(func (export "serve") …)` | existing |

Why two fixup phases: the guest's own initialization (its `_initialize`, Rust `std` startup)
runs during step 9 and may call virtual WASI functions in the donut. Those functions may in
turn call host imports through shims, so the shims must already be patched, and the
donut's own `_initialize` must already have run. Only the guest-export slots wait for
step 11. If the guest's init calls something that needs the guest's own exports, it traps
on a null funcref. The WAT version has the same limitation.

Type identity needs no special handling in the encoder. In step 9 the guest is
instantiated with the donut's own `$request`, so the type of `receive` aliased in step 10
already refers to `$request`. World B is used only for ABI flattening and canon options.

Step 7 must keep wit-component's shim-component wrapping. Validation rejects a plain
`(instance (export …))` bundle of the lifted funcs with "resource used in function does
not have a name in this context", because `$request` is defined locally and not
exported. Resource identity survives the shim. `spike/donut.wat.in` follows this table
step by step and is the golden output for the encoder.

Because the guest slots come last, the existing (phase 1) fixup fills a prefix of the
table unchanged. It only imports the table at its full size, and skips guest slots
when building its "actual" instance.

Not supported yet, and rejected with an error: world-level guest imports and exports
(`guest-root`), and `return_call_ref` shims.

### 5.2 Build pipeline and `wasm-component-ld`

**Prototype without touching wasm-component-ld.** It already has `--skip-wit-component`
(checked on the installed binary), which stops after `wasm-ld` and leaves the linked core
module, including the custom sections. `wasm-tools component new --donut-guest <guest.wasm>`
(the CLI frontend for wit-component's donut mode) then takes:
* the core module (from `-Clink-arg=--skip-wit-component`);
* its `component-type` (World A) and `wit-bindgen-donut` sections;
* the guest component.

It runs the §5.1 encoding and writes the final component. The intermediate core module
can't be run on its own; it is only an input to this step. The phase-0 spike confirmed
that a `wasm32-wasip2` core module linked this way needs no preview1 adapter.

**Later integration:**
* wasm-component-ld detects the `wit-bindgen-donut` section and switches wit-component
  into donut mode.
* A `--donut-guest <path.wasm>` flag selects the guest. From cargo:
  `RUSTFLAGS="-Clink-arg=--donut-guest=guest.wasm"`, or a `build.rs` that emits
  `cargo:rustc-link-arg`.
* A donut section in a version the tool doesn't recognize is an error. Ordinary
  componentization is never a silent substitute.

## 6. Wrapping the real resource (`forward:`)

When a virtual interface has the same shape as a host interface (typically the same
interface: `import example:http/types` from the host, `export` the virtual one to the
guest), the macro can generate passthrough-with-hooks code, similar to wasi-virt.

For each resource `R` in the virtual interface:

```rust
pub struct VirtR { pub inner: crate::<host>::R }          // the wrapping rep
impl exports::<v>::GuestR for VirtR {                      // generated, delegates to hooks
    fn new(a: A) -> Self { <Hooks as RHooks>::new(a) }
    fn path(&self) -> String { <Hooks as RHooks>::path(self) }
}
pub trait RHooks {                                         // user may override any method
    fn new(a: A) -> VirtR { VirtR { inner: host::R::new(a.to_host()) } }
    fn path(this: &VirtR) -> String { this.inner.path().from_host() }
}
```

* The user picks the hooks type with `forward: { "<v>": { host: "<h>", hooks: MyHooks } }`.
  Default methods give pure passthrough. Overriding one method intercepts it.
* Generated deep conversions `ToHost` / `FromHost` cover every type in the use-closure
  (records, variants, enums, flags, `list`, `option`, `result`, tuples, handles). They are
  needed even for resource-free records, because the import-side and export-side Rust types
  are distinct. Rules: `own<R>` → `into_inner::<VirtR>().inner`; `borrow<R>` →
  `&get::<VirtR>().inner`; a returned host `R` → `exports::R::new(VirtR { inner })`.
* The dtor is just `Drop` on `VirtR`, which drops `inner` and so the host handle.

`forward:` is optional. Without it the user implements the `Guest*` traits directly, as in
the Explainer, where `$Main` implements `request` on top of sockets.

**Prior art:** [proxy-component](https://github.com/chenyan2002/proxy-component)
(`src/traits/proxy.rs`) already generates this kind of conversion as a `ToProxy` trait,
including resources. Owned import → `Export::new(inner)`, owned export → `into_inner()`,
and an incoming `RequestBorrow<'a>` → `&'a Inner` via `get::<T>()`. The last needs
wit-bindgen ≥ 0.48. It also builds an `ExportBorrow` from `&Import` with `lift(ptr)`.
That works only within one core instance (it depends on the rep layout) and never
crosses the canonical ABI, so it doesn't lift the §8 "Lending" restriction. The same
generator also covers the World-A ↔ World-B conversions from §4 step 3.

## 7. Guest plugging

* **Embed** (first milestone, and the default): one self-contained component; the guest
  is chosen when the encoder runs (`component new --donut-guest`, later the
  wasm-component-ld `--donut-guest` link arg), not when the proc macro expands. Swapping in a compatible guest needs only a re-link, not a Rust rebuild.
  Before encoding, the encoder checks that the guest's imports and exports exist in
  `G`. A type mismatch is currently only caught by the final validation of the output
  component, without a donut-specific message.
* **Hole** (later): the donut has a component-typed `guest` import, so one built donut
  can wrap any guest. Plain WIT worlds can't describe component-valued imports, and
  Wasmtime can't instantiate them. So a plug step builds the Explainer's
  `$VirtualizedGuest` grandparent: import the donut's outer imports, embed both
  components, instantiate the donut with the guest *component* as an argument, and
  re-export. This is ordinary composition, so it should be a **wac feature**
  (`wac plug`-style), not a separate tool. wac's type model already has
  component-typed items (`wac-types` `ItemKind::Component(WorldId)`). The encoder only
  has to emit the component-typed import instead of an embedded guest. Both forms share
  one `DonutPlan`.

## 8. Semantics and pitfalls to document for users

* **Re-entrance:** any `guest::…` call can re-enter the donut's virtual exports *and run
  virtual resource destructors* before it returns. Don't hold `RefCell`/`Mutex` guards
  or `&T` from `Request::get` across a guest call.
* **Lending:** the donut can lend `borrow<R>` to the guest only for handles it *owns*
  (`&Request`). A `RequestBorrow<'_>` received from the guest holds a rep, not a handle,
  so it can't be passed back to a guest export.
* **Calls before linking:** calling `guest::…` from the donut's `_initialize` traps
  (the slot isn't patched yet). Optional nicety: phase-2 fixup calls an exported
  `__donut_linked()` that sets a static flag, and the generated wrappers check it and
  panic with a clear message.
* **Only the guest can't see through the abstraction.** It never gets
  `resource.new`/`rep` for virtual types, as in the Explainer.
* **Out of scope (p2):** async lifts/lowers, `stream`, `future`, `error-context`. The
  macro rejects guest worlds that use them.

## 9. Alternatives considered

### 9.1 Split donut (fallback if the runtime rejected re-entrance)

Split the donut into two components with no cycle:

* **A** implements the virtual interfaces (and, if needed, a private factory interface);
* **B** imports `guest.handler` plus A's types and exports `serve`.

Compose host → A → G → B with wac. B → G → A doesn't re-enter any instance on the stack.
The cost: state shared between A and B has to go over an interface (with copies), and A
can't call the guest. The macro could produce this layout as `mode: split` from the same
inputs. Treat it as a fallback, not the main design.

**Status:** not needed. The phase-0 spike runs the nested, re-entrant layout on
Wasmtime 49.0.1. 48.0.2 traps, so Wasmtime ≥ 49 is required.

### 9.2 An ordinary donut component, linked by wac with mutual references

Idea: let wac name the donut and the guest first, so each can refer to the other
when they're linked, and keep the donut an ordinary component built by stock
tooling. The component binary has no forward references, so wac would have to lower
the cycle itself by generating **glue** in the scope of the composed component, where
the donut and the guest are instantiated as siblings:

* **Function cycle:** the glue is a core module with a table and trampolines. The
  trampolines are lifted and passed to the donut as its guest-export imports. After
  the guest is instantiated, its exports are lowered and patched into the table. The
  donut's memory is sealed inside the donut, so the glue needs **its own memory and
  allocator** for strings and lists. Donut → guest calls are then copied twice.
* **Type cycle:** the donut can't import `receive(own<request>)` if it defines
  `request` (2.1). So the glue has to define the virtual resource types instead and
  pass them in. The donut imports `request` abstractly, plus `resource.new`/`rep`
  lifted as ordinary functions, and exports its destructor (patched into the type's
  dtor slot later).
* **Guest-defined resources** (e.g. `session`) don't exist until the guest is
  instantiated, after the donut. The glue would have to proxy them with its own
  resource types and translate handles in the trampolines, which gets hard for handles
  nested in records and lists.

What it would buy:
* the donut is an ordinary component, with no wit-component changes;
* plugging any guest in later comes for free;
* any language works;
* the §8 "Lending" restriction goes away, because the donut holds real handles.

What it costs:
* double copies;
* a cross-component call for every resource access;
* guest-resource proxying;
* code generation in wac (core modules), which wac doesn't do today.

Not pursued: the encoder route (§5) already works and has none of these costs. wac's
role stays plugging guests into hole-mode donuts (§7).

### 9.3 walrus

walrus transforms core modules only. All the hard parts here are component-level:
lifts and lowers, resource types, nested instantiation, type encoding. The core
modules involved (shim, fixups) are tiny and already generated with `wasm-encoder`.
It would also be an unusual dependency for wit-component, which builds on
`wasmparser`/`wasm-encoder`.

## 10. Where to build it, and work breakdown

### 10.1 Where each piece lives

Each piece goes into the repo where it naturally belongs, and is upstreamed from local
checkouts of those repos. There is no long-lived fork, and generated code is never
patched with `syn`. `syn` only parses the macro's own input, as `wit-bindgen-rust-macro`
does.

| Piece | Where | Status |
|---|---|---|
| `[guest-export]` routing + no `component-type` section for World B (§4 step 3) | wit-bindgen-rust: one option, `Opts::import_module_prefix` | done |
| World B's colliding resource methods (§4 step 3) | DonutPlan strips functions in its private `Resolve` copy | no wit-bindgen change |
| World B resource remapping (§4 step 3) | type-level `with` keys (existing public API) | no change |
| `DonutPlan` + `donut!` (§4) | this repo (`crates/donut-plan`, `crates/donut`), consuming the local wit-bindgen/wasm-tools through `[patch.crates-io]` | done |
| Encoder (§5.1) | wit-component donut mode (`wit_component::donut`, `ComponentEncoder::donut_guest`); CLI `wasm-tools component new --donut-guest` | done |
| `cargo build` integration (§5.2) | wasm-component-ld `--donut-guest` | later |
| Conversions and `forward:` (§6) | this repo, emitted as source from the `Resolve` (proxy-component's `ToProxy` technique) | later |
| Hole-mode plugging (§7) | wac | later |

Why this split:
* **The generator change is small and generic,** so it went upstream first. Patching
  wit-bindgen's output out of tree (the approach in an earlier version of this section)
  would have coupled us to the exact shape of the generated code.
* **The encoder has to live in wit-component.** Its internals (`EncodingState`, shims,
  fixups) are private, and composition tools can't do the job (§2.2).
* **`donut!` stays out of wit-bindgen until the API settles.** A public macro and its
  options are hard to change once released. Moving it into wit-bindgen behind an
  experimental feature is a later decision.
* **Only Rust benefits at first.** The manifest (§4 step 4) stays language-neutral, so
  C/C++/Go generators can target the same encoder later.

Placement: this repo is cleaner than adding a mode to proxy-component. proxy-component
is a CLI that generates a temp project from a *binary* and composes siblings with wac;
`donut!` is a library and proc macro for hand-written Rust producing one nested
component. Shared code (the resource-aware conversion generator, WIT-name utilities) is
worth factoring into a small crate once both use it. Until then, copy the techniques,
not the binary-crate modules.

### 10.2 Work breakdown

`docs/plan.md` has the detailed steps, exit criteria, and status. Summary:

| Phase | Where | Work | Status |
|---|---|---|---|
| 0 | `spike/` | Hand-assembled donut for §3.1: real wit-bindgen guest and core module, hand-written outer component following §5.1 | done: passes on Wasmtime 49.0.1, traps on 48.0.2 |
| 1 | wit-bindgen | `import_module_prefix` | done |
| 2 | this repo | `DonutPlan` (§4) and the `donut!` frontend; snapshot tests | done: `examples/http` passes on Wasmtime 49 |
| 3 | wit-component | donut mode (§5.1) and `component new --donut-guest`; rebuild the spike from its core module | done; fixtures pending |
| 4 | this repo | end to end: `serve → receive → send`, exactly-once dtors, strings/lists, borrow-in-record, an interface on both sides of the guest, guest-defined resources, null-slot trap before linking; `wasm32-wasip2` guests | next |
| 5 | this repo | `forward:` (§6) and World A ↔ B conversions | — |
| 6 | upstream | wit-component PR; wasm-component-ld `--donut-guest`; decide whether `donut!` moves into wit-bindgen | — |
| 7 | later | hole mode + wac plugging (§7), p3 (§2.4), other languages | — |

Phases 2 and 3 were done in that order: the manifest was frozen first, the encoder
was built against the spike's core module, and the macro then generated what the
spike had by hand.

## 11. Open questions

1. Should virtual interfaces also be *exported* publicly, as an option, so the host can
   observe them? The encoder can support this easily; the default is internal only.
2. How should the guest be named in hole mode: always `guest`, or derived from `G`'s name?
3. Should the manifest reuse wit-component's binary WIT encoding for `G`, or a reference
   into the World-A package plus a world name? Reuse is simpler; a reference is smaller.
   The implementation (§4 step 4) reuses the encoding.
4. Upstream naming: "donut" is informal; wit-component might prefer
   `ComponentEncoder::nested_instance`.
5. ~~How to handle guest imports outside `G`, such as a `wasm32-wasip2` guest's `std`
   WASI imports?~~ Resolved: they're passthrough when the donut imports a
   semver-compatible interface, and an error otherwise (§5.1). Adding them as new
   donut imports instead would make the donut's imports depend on the guest binary;
   reconsider for hole mode.
6. Should `donut!` support being invoked outside the crate root (e.g. a
   `bindings_module` option used as the prefix for World B's remap paths)?
