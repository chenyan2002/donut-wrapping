# Design: `donut!` — donut wrapping for Rust components (WASI p2)

Status: draft / proposal. Revised after comparing with `docs/p2-donut-wrapping-astra.md`
and reviewing [proxy-component](https://github.com/chenyan2002/proxy-component); nothing here is prototyped yet.
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
can't do this: component-level funcs are immutable, funcrefs can't cross component
boundaries, and the resource-type identity problem from 2.1 remains. So **the component
encoder must change**. A post-processing tool around a finished component can't do it.

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
lag behind. Older Wasmtime p2 kept a per-instance `may_enter` flag and would trap with
"cannot enter component instance". **This has to be checked on the target runtime first**
(Phase 0). If it fails, §9 has a fallback.

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
wit_bindgen::donut!({
    path: "wit",
    world: "example:http/socket-server",   // donut's outer world
    guest: "example:http/http-guest",      // the hole's shape
    // Guest imports the donut implements. Every other guest import is
    // passed through from the host unchanged (added to the donut's imports).
    virtualize: ["example:http/types", "example:http/outgoing"],
    // Optional: generate forwarding wrappers (see §6) when a virtualized
    // interface has the same shape as a host interface.
    // forward: { "example:http/types": "example:http/types" },
    // All ordinary `generate!` options (ownership, with, additional_derives…)
    // are accepted and apply to World A.
});
```

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
2. **Build World A** (cloned into a fresh `World` in the same `Resolve`):
   `W.imports ∪ passthrough(G) ∪ {import X | X needed by a virtual interface's use-closure}`,
   `W.exports ∪ {export v | v ∈ virtual}`. A world may both import and export the same
   interface (needed when the donut wraps the real `example:http/types`).
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
     export-side `Request::new<T>` and `get`. Suppress them: initially via `skip`
     (which matches `func.name` globally, e.g. `[constructor]request`,
     `[method]request.path`; `interface.rs:163,819`), and later via an "emission
     purpose" option (definitions-only for dependency interfaces, calls for guest-export
     interfaces).
   * **Resources the guest defines** (declared in a guest-export interface) are not
     remapped. They are generated as ordinary opaque imported resources in `guest::…`,
     and their `[resource-drop]` imports go through the `[guest-export]` module too, so
     the encoder must late-bind them (§5.1).
   * The restriction in §8 still holds: an incoming `RequestBorrow` has no handle in the
     donut's table, so it can't be relayed to a guest export.

   **Symbol routing and metadata:**
   * `import_module_prefix: "[guest-export]"`, applied where `wasm_import_module` is
     computed (`crates/rust/src/lib.rs:1334` and the `$root` equivalent). Core imports
     become e.g. `"[guest-export]example:http/handler" "receive"`.
   * Don't emit a `component-type` section for this world. `type_section_suffix` only
     renames the section and `disable_custom_section_link_helpers` only changes retention,
     so neither does this today.
   * Neither is an option yet. §10 explains how to get both without forking.
4. **Emit the donut manifest** as custom section `wit-bindgen-donut:v0`. The name must
   **not** start with `component-type`: wit-component merges every such section into the
   world (`metadata.rs:237`). Contents (binary WIT package plus a small header):
   * the guest world `G`, encoded so the encoder can produce the guest's component type;
   * the virtual set with the World-A export key ↔ guest import name mapping (including
     `guest-root` flattening);
   * the passthrough set (World-A import key ↔ guest import name);
   * the `[guest-export]` prefix and the World-B function list (for ABI options).

Steps 1–4 form a `DonutPlan` library that a proc macro (`donut!`) and a CLI both drive.
Where it lives, and how it gets the missing generator knobs, is covered in §10. Keep the
mapping between World A, World B, and `G` as explicit port mappings computed **before**
generation. `generate_nominal_type_ids` (`crates/core/src/lib.rs:28`) rewrites arena
IDs during each generator run, so IDs can't be matched across views afterwards.

## 5. The linkage table (encoder side)

The Rust code never touches a table. Calls to `guest::…::receive` are plain wasm imports
from `[guest-export]…`. The encoder satisfies them with shim trampolines, exactly as
`$LinkageTable` does. A Rust-level table that patches `__indirect_function_table` was
rejected: it needs `--export-table`/`--growable-table`, slot numbers the linker controls,
and patching memory at runtime, and it still needs the encoder to lower the guest
export.

### 5.1 New "donut mode" in `wit-component::ComponentEncoder`

Turned on automatically when a `wit-bindgen-donut:*` section is present, or explicitly
via `ComponentEncoder::donut(guest: GuestSource)`, where
`GuestSource = Embed(Vec<u8>) | Import`.

Changes to validation (`validation.rs`):

* New import classification `Import::GuestExportFunc { interface, func }` for core imports
  from `[guest-export]…` modules. ABI info comes from World B in the manifest.
* New import classification `Import::GuestResourceDrop { interface, resource }` for
  `[guest-export]iface` `[resource-drop]R`, where `R` is a resource the guest defines.
  Its type comes from the guest instance, so it is late-bound like a function.
* Virtual World-A exports are classified as **internal**: they are lifted but not exported.
* Semver import merging (`merge_imports_based_on_semver`) must not merge a host import
  with a donut-internal occurrence of the same interface. Exclude manifest-listed keys.

Encoding order, extending `encode_with_fixups` / `encode_core_instantiation`:

| # | Step | Explainer equivalent | New? |
|---|---|---|---|
| 1 | `encode_imports`: World-A imports (host + passthrough) | `(import "net" …)` | existing |
| 2 | `encode_core_modules` | `(core module $Main …)` | existing |
| 3 | shim instance: `IndirectLowering` for host imports, `ResourceDtor` slots, **plus a `GuestExportLowering` slot for every `[guest-export]` function and a `GuestResourceDrop` slot for every guest-defined resource drop, always indirect** | `$LinkageTable` | add two `ShimKind`s |
| 4 | `declare_types_for_imported_intrinsics`: defines virtual resources with dtors from shims | `(type $request (resource (rep i32) (dtor …)))` | existing |
| 5 | `instantiate_main_module` | `(core instance $main …)` | existing |
| 6 | **fixup phase 1**: host lowerings, adapters, dtors, `_initialize` | the `elem` segment in `$Main` | split |
| 7 | lift internal virtual exports and bundle one component instance per interface, without exporting | `(func $send (canon lift …))` | refactor of `encode_exports` |
| 8 | guest: embed `(component …)` or `(import "guest" (component <type of G>))` | `(import "guest" (component $Guest …))` | new |
| 9 | `instantiate` the guest: virtual interface → instance from 7; passthrough → host import instance from 1; `guest-root` items flattened to world-level names | `(instance $guest (instantiate $Guest (with …)))` | new |
| 10 | alias the guest exports and `canon lower` them with main's `memory`, `cabi_realloc`, `string-encoding`; alias guest-defined resource types and emit `canon resource.drop` for them | `(core func $lowered-receive (canon lower …))` | new source for an existing lowering path |
| 11 | **fixup phase 2**: patch `GuestExportLowering` and `GuestResourceDrop` slots | `$PatchLinkageTable` | split |
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

### 5.2 Build pipeline and `wasm-component-ld`

**Prototype without touching wasm-component-ld.** It already has `--skip-wit-component`
(checked on the installed binary), which stops after `wasm-ld` and leaves the linked core
module, including the custom sections. A standalone `donut-componentize` tool then takes:
* the core module (from `-Clink-arg=--skip-wit-component`);
* its `component-type` (World A) and `wit-bindgen-donut` sections;
* the guest component.

It runs the §5.1 encoding and writes the final component. The intermediate core module
can't be run on its own; it is only an input to this tool.

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
  is chosen when `donut-componentize` (or `--donut-guest`) runs, not when the proc macro
  expands. Swapping in a compatible guest needs only a re-link, not a Rust rebuild.
  Before encoding, the tool type-checks the guest's imports and exports against `G`
  with the resource substitutions applied.
* **Hole** (later): the donut has a component-typed `guest` import, so one built donut
  can wrap any guest. Plain WIT worlds can't describe component-valued imports, and
  Wasmtime can't instantiate them. So a small plug step builds the Explainer's
  `$VirtualizedGuest` grandparent: import the donut's outer imports, embed both components,
  instantiate, re-export (wasm-encoder only, about 150 lines). It could later move to
  wasm-tools/wac. Both forms share one `DonutPlan`.

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

## 9. Fallback if the runtime rejects re-entrance

Split the donut into two components with no cycle:

* **A** implements the virtual interfaces (and, if needed, a private factory interface);
* **B** imports `guest.handler` plus A's types and exports `serve`.

Compose host → A → G → B with wac. B → G → A doesn't re-enter any instance on the stack.
The cost: state shared between A and B has to go over an interface (with copies), and A
can't call the guest. The macro could produce this layout as `mode: split` from the same
inputs. Treat it as a fallback, not the main design.

## 10. Where to build it, and work breakdown

### 10.1 Recommendation: a new repo, using proxy-component's out-of-tree pattern

Start in a **separate repository**, not in wit-bindgen. Upstream small, generic pieces
once the end-to-end path works.

[proxy-component](https://github.com/chenyan2002/proxy-component) shows that the
codegen side doesn't need wit-bindgen changes:
* it calls `wit_bindgen_rust::Opts { .. }.build().generate(..)` as a library (pinned
  to 0.63.0);
* it parses the generated source with `syn`;
* it emits additional impls with `quote` (`State`, `src/traits/proxy.rs`), with
  snapshot tests (`BLESS=1 cargo test`).

`donut!` can do the same in-process. Every knob §4 needs is either public API already or
a mechanical patch of the generated source:

| Need (§4) | Out of tree | Later upstream replacement |
|---|---|---|
| World A bindings | `wit_bindgen_rust::Opts` as is | none needed |
| World B resource remapping | type-level `with` keys (public) | none needed |
| Suppress colliding resource methods in World B | `skip` (public), or drop the `impl` items with `syn` | "emission purpose" option |
| `[guest-export]` import modules | rewrite `#[link(wasm_import_module = "…")]` with `syn` | `import_module_prefix` option |
| No `component-type` section for World B | drop the `#[link_section = "component-type:…"]` item with `syn` | `emit_component_type: false` |
| World A ↔ B and host ↔ virtual conversions | generate with `quote`, as `ToProxy` does | none needed |
| Runtime path consistency | set `runtime_path` to the new crate's re-export of `wit_bindgen::rt` | none needed |

Why not start in wit-bindgen:
* **The risky parts aren't in wit-bindgen.** The re-entrance check (Phase 0) and the
  encoder (§5.1) belong to the runtime and wasm-tools. The encoder internals
  (`EncodingState`, shims, fixups) are private, so the spike needs a wasm-tools fork
  wherever the macro lives. proxy-component already depends on a wasm-tools fork for
  `wasm-wave`.
* **The API isn't settled.** A public `wit_bindgen::donut!` and new `Opts` fields are
  hard to change once released. Working end to end first gives upstream PRs a concrete
  use case.
* **The upstream asks shrink to three small options** (the right column), each
  replacing one `syn` patch with a codegen test. That's an easy review, independent of
  donut wrapping.

Costs, and how to contain them:
* `syn` patches are coupled to the shape of wit-bindgen's output. proxy-component
  already depends on details like `RequestBorrow::get` needing ≥ 0.48. Pin
  `wit-bindgen-rust` exactly, keep snapshot tests, and upstream the three options early
  so that coupling goes away.
* Only Rust benefits until the encoder moves into wit-component. The encoder contract
  (the manifest section) stays language-neutral so C/C++/Go generators can target it
  later.

Placement: a new repo is cleaner than adding a mode to proxy-component. proxy-component
is a CLI that generates a temp project from a *binary* and composes siblings with wac;
`donut!` is a library and proc macro for hand-written Rust producing one nested
component. Shared code (the resource-aware conversion generator, WIT-name utilities) is
worth factoring into a small crate once both use it. Until then, copy the techniques,
not the binary-crate modules.

### 10.2 Work breakdown

| Phase | Where | Work | Exit criterion |
|---|---|---|---|
| 0 | scratch | Hand-write the WAT for §3.1 (the Explainer layout plus real canon options, including one string/list function and one guest-defined resource); run on the pinned Wasmtime | re-entrance and destructors work, or the §9 decision is made |
| 1 | wasm-tools fork | `donut-componentize`: §5.1 on top of wit-component internals (`Import::GuestExportFunc`, `GuestResourceDrop`, internal exports, two-phase fixup, embedded-guest instantiation, guest type check) | the Phase-0 example rebuilt from a core module validates and runs |
| 2 | new repo | `DonutPlan` (§4: classification, both-direction dependency checks, port mappings, manifest); World A and B generation with type-level `with`, `skip`, and `syn` patches; `donut!` frontend; snapshot tests | expands for §3.1, core imports/exports and manifest as expected |
| 3 | new repo | end to end with `--skip-wit-component` + `donut-componentize`; runtime tests: `serve → receive → send`, exact-once dtors, strings/lists, borrow-in-record, interface on both sides of the guest, guest-defined resources, null-slot trap before linking | tests pass in CI |
| 4 | new repo | `forward:` (§6) and World A ↔ B conversions, built on the `ToProxy` approach | a passthrough `wasi:http/types` donut forwards a request end to end |
| 5 | upstream | wit-bindgen: `import_module_prefix`, `emit_component_type`, emission purpose. wit-component: donut mode behind an option. wasm-component-ld: `--donut-guest`. Then drop the `syn` patches and the fork | the new repo builds against released crates |
| 6 | later | hole mode and the plug step (§7); deciding whether `donut!` moves into `wit-bindgen` behind an experimental feature | — |

Phases 1 and 2 can run in parallel once the manifest format (§4 step 4) is fixed.

## 11. Open questions

1. Should virtual interfaces also be *exported* publicly, as an option, so the host can
   observe them? The encoder can support this easily; the default is internal only.
2. How should the guest be named in hole mode: always `guest`, or derived from `G`'s name?
3. Should the manifest reuse wit-component's binary WIT encoding for `G`, or a reference
   into the World-A package plus a world name? Reuse is simpler; a reference is smaller.
4. Upstream naming: "donut" is informal; wit-component might prefer
   `ComponentEncoder::nested_instance`.
