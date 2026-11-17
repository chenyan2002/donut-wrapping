# Design: `child!` — Rust components that wrap guest components

Status: proposal. It revises the prototype (branch `prototype`, `docs/design.md`
there; "P§n" below refers to its sections). The prototype's constraints still
hold and aren't repeated here: the inverted world can't be written as WIT (P§2.1),
the cycle must be broken inside one component scope by the encoder (P§2.2), and
Wasmtime ≥ 49 is required for parent → child calls (P§2.4).

Scope: component model p2 (sync functions, resources).

## 1. What changes from the prototype

| # | Prototype | Now |
|---|---|---|
| 1 | `donut!`, crate root only | `child!`, still at the crate root, but it emits its bindings into `mod <name>`; **several per component**, one module each |
| 2 | `world:` names the donut's outer world | removed: the donut's own imports and exports come from an ordinary `wit_bindgen::generate!` |
| 3 | `forward`, `mock`, `hooks` | removed, with their generated wrappers and conversions |
| 4 | `virtualize: [..]`, everything else passed through | every guest import is **virtualized by default**. `import_passthrough` and `export_passthrough` opt interfaces out, WASI included |
| 5 | wit-component embeds the guest (`component new --donut-guest`) | wit-component emits a component **with holes**, one component-typed import per child. `wac plug` fills them |

Children don't call each other: a child's imports are never wired to another
child's exports.

## 2. Example

```wit
package example:app;

interface types {
  resource request { constructor(url: string); url: func() -> string; }
}
interface outgoing { use types.{request}; send: func(r: request) -> result<u16, string>; }
interface handler  { handle: func(path: string) -> string; }
interface secrets  { get: func(key: string) -> option<string>; }

world app {                      // what the guest targets: the hole's shape
  include wasi:cli/imports@0.2.6;
  import outgoing;               // pulls in `types`
  export handler;
}

world host {                     // the donut's own world
  import secrets;
  import outgoing;
  export handler;
}
```

```rust
// The donut's own world: an ordinary wit-bindgen world.
mod host {
    wit_bindgen::generate!({ path: "../wit", world: "example:app/host" });
}

// One child, at the crate root; it expands to `pub mod app { … }`.
// Everything `example:app/app` imports is virtualized, except WASI.
donut::child!({
    name: "app",
    path: "../wit",
    guest: "example:app/app",
    import_passthrough: ["wasi:*"],
    // export_passthrough: ["example:app/handler"],
});

use app::virt::example::app::{outgoing, types};

struct Donut;

impl host::exports::example::app::handler::Guest for Donut {
    fn handle(path: String) -> String {
        app::guest::example::app::handler::handle(&path)   // call into the child
    }
}
impl types::Guest for Donut { type Request = MyRequest; }
impl outgoing::Guest for Donut {
    fn send(r: types::Request) -> Result<u16, String> {
        let url = r.into_inner::<MyRequest>().url;
        let token = host::example::app::secrets::get("api").ok_or("no token")?;
        let req = host::example::app::types::Request::new(&url);
        req.set_header("authorization", &token);
        host::example::app::outgoing::send(req)
    }
}

host::export!(Donut with_types_in host);
app::export!(Donut with_types_in app);
```

```sh
cargo build --target wasm32-wasip2 --release      # donut.wasm, with a hole named `app`
wac plug donut.wasm --plug app.wasm -o composed.wasm
```

`donut.wasm` imports what `host` and its own `std` import, plus the
passthrough WASI imports, plus `(import "app" (component <type of example:app/app>))`.
It exports `example:app/handler`. `composed.wasm` has no hole and runs on Wasmtime.

A pure virtualizer needs no `generate!` at all: virtualize `wasi:filesystem/*`,
passthrough the rest, and `export_passthrough: ["wasi:http/incoming-handler"]`.

## 3. `child!` options

| Option | Meaning |
|---|---|
| `path` | WIT path(s), relative to the crate's manifest directory. Default `"wit"`. |
| `guest` | The guest world `G`. It is the hole's exact type (§7). |
| `name` | Required. The child's name in snake_case, like the other options: it's the generated module as written (`my_app` → `mod my_app`). Its kebab-case form (`my-app`) is the hole's import name and part of every generated WIT and symbol name. Must match `[a-z][a-z0-9]*(_[a-z][a-z0-9]*)*`, so the conversion is lossless, and not be a Rust keyword. Unique in the component. |
| `import_passthrough` | Guest imports given to the child straight from the donut's host. Everything else `G` imports is virtualized. |
| `export_passthrough` | Guest exports the donut re-exports unchanged. |
| `with`, `additional_derives`, `runtime_path`, … | Forwarded to wit-bindgen for this child's World A (§5). `with` is how a child shares host bindings with the user's `generate!`. |

**Patterns.** A passthrough entry is an interface name, with or without a version
(`wasi:cli/stdout`, `wasi:cli/stdout@0.2.6`), all of a package (`wasi:cli/*`), or
all of a namespace (`wasi:*`). An entry that matches nothing in `G` is an error,
which catches typos.

## 4. Classification and checks

For each interface `G` imports: in `import_passthrough` → **passthrough**,
otherwise **virtual**. For each interface `G` exports: always callable from Rust
(`guest::…`); in `export_passthrough` → also **re-exported**.

Checks, over the `use` closure, counting only resources (resource-free types are
structural and never trigger them):

* **Passthrough imports can't mention virtual resources.** The host's
  `send(request)` can't take the child's virtual `request`. The error names the
  interface to add to `import_passthrough` (with virtualize-by-default, the fix is
  to pass the dependency through, not to virtualize the user).
* **Re-exported interfaces can only mention host resources (passthrough imports)
  or resources of other re-exported interfaces.** A virtual resource is internal
  to the donut and has no name outside it.
* A virtual interface may mention passthrough resources: the Rust code hands the
  child real host handles (e.g. a virtual `wasi:cli/stdout` returning a host
  `output-stream`).
* Not supported, as in the prototype: world-level (non-interface) guest imports and
  exports; async, `stream`, `future`.

Virtualizing by default means a forgotten interface is a compile error (an
unimplemented `Guest` trait), not a silent hole to the host.

## 5. Generated code

`child!` must be invoked at the crate root, and expands to `pub mod <name> { … }`.
A proc macro can't learn the module it was invoked in, so this is what lets it
use absolute paths: World B's type-level `with` remaps (each virtual resource ↦
World A's export-side type; P§4 step 3) point at
`crate::<name>::exports::virtual_…::Request`. wit-bindgen pastes type-level remap
paths verbatim at each use site, so they have to be absolute; no wit-bindgen
change is needed.

`virt` is the only path users write. It re-exports wit-bindgen's output for the
renamed copies (below), one `pub use` per namespace
(`pub use super::exports::virtual_app_wasi as wasi;`), so the WIT rename never
shows up in user code.

| Item | Path (in `crate::<name>`) |
|---|---|
| Virtual interfaces, to implement | `virt::<ns>::<pkg>::<iface>` |
| World A: host bindings the virtual interfaces need (passthrough dependencies) | `<ns>::<pkg>::<iface>`, unless remapped with `with` |
| World A's export macro | `export!` |
| World B: the child's exports, to call | `guest::<ns>::<pkg>::<iface>::<func>` |

**Virtual interfaces are always copies.** The guest's view of a virtualized
interface is a copy in a renamed package. The prototype needed copies for
forwarding, but there are still three cases without it:

1. **Two children virtualize the same interface.** This is the normal case for
   WASI. Without per-child copies, both export `wasi:cli/stdout#get-stdout`: a
   duplicate symbol, and one ambiguous export in the merged world.
2. **The donut's own world uses the interface.** If the user's world exports
   `handler` (which uses `types.request`) and a child virtualizes `types`, the
   merged world exports `types`. Since an export's dependencies bind to an
   exported version of the interface when one exists (wit-bindgen's
   `generate_nominal_type_ids`, wit-component's `exports_used`), the user's
   `handler` would bind to the virtual `request` instead of the host's.
3. **A middleware guest** imports and exports `handler`: the child's virtual
   `handler` and the donut's re-export (or its own export) would have the same name.

`child!` doesn't see the user's world anymore, so it can't detect case 2 and copy
only on a collision. It always copies.

The copy is internal: users only see `virt` (Rust) and the guest's original names
(the hole's type). Dropping the rename from WIT altogether would need per-child
prefixes in wit-bindgen for export symbols (`export_prefix` exists) *and* for
`[export]<iface>` resource intrinsics (two children's `[resource-new]input-stream`
would otherwise link to one import). It would also need each child's World A kept
out of the merged world (no `component-type` section), and a wit-component
refactor, since its encoder state is keyed by interface. That's the same rename,
moved into the encoder.

**Naming is per child**, so that WIT names and core symbols don't collide when
wasm-ld links the children and wit-component merges their worlds. `<child>` is
the kebab-case form of `name` (`my_app` → `my-app`), since WIT and component
names can't contain `_`:

* virtual copies live in package `virtual-<CHILD>-<ns>:<pkg>`, with the child
  part in uppercase, so the export symbols are e.g.
  `virtual-APP-wasi:cli/stdout#get-stdout`. Joining with `-` alone would lose the
  boundary between child and namespace: child `a` with `b-c:pkg` and child `a-b`
  with `c:pkg` would both be `virtual-a-b-c:pkg`. WIT allows all-uppercase words,
  so the uppercase words are the child and the lowercase ones the namespace
  (`virtual-A-b-c` vs `virtual-A-B-c`). To keep this unambiguous, `child!` rejects
  a guest interface whose namespace has an uppercase word;
* World A/B are synthesized in a per-child package (`donut-child:<child>`), so
  each wit-bindgen run's `component-type:…` section has a distinct name (wasm-ld
  concatenates same-named custom sections);
* World B's core imports use the prefix `[child=<child>]` (the prototype's
  `[guest-export]`), e.g. `"[child=app]example:app/handler" "handle"`;
* the manifest section is `wit-bindgen-child:<child>` (§6).

A duplicate `name` within one crate is a duplicate-module error from rustc.
Across crates, the encoder reports it.

### 5.1 Living next to the user's `generate!`

* Each `generate!`/`child!` emits its own `component-type` section. wit-component
  already merges them into one world, so the donut's world is the user's world ∪
  every child's World A. Interfaces imported by several (e.g. `wasi:io/streams`)
  merge into one import.
* Each child's `export!` lives in its own module (`app::export!(… with_types_in
  app)`), so it doesn't clash with the user's. The user calls all of them.
* Host bindings generated twice (by the user's world and a child's World A) are
  distinct Rust types for the same host resource. To pass one to the other, map it:
  `with: { "wasi:io/streams": crate::host::wasi::io::streams }` in `child!`.
* A re-exported guest interface and a donut export with the same name is an
  encoder error.

### 5.3 What's removed

`forward`, `mock`, `hooks`, and everything behind them: `forward.rs`
(`Fwd…`/`Mock…` wrappers, `Hooks`, `ToHost`/`FromHost` conversions, `lend`), and
`merge_structurally_equal_types` for World A. Passthrough is now purely
component-level: the child gets the host's instance, or the host gets the child's,
with no Rust code in between. To intercept, virtualize and write the delegating
impl by hand. `Plan::world_b_shared_types` stays: it keeps resource-free types
shared between `exports::…` and `guest::…`.

## 6. Manifest v1

Custom section `wit-bindgen-child:<child>` (must not start with `component-type`):

```
u8 version = 1
string child-name
vec<u8> G                         // wit-component's metadata encoding
vec<(string, string)> virtualized // guest import name → World A export implementing it
vec<string> export_passthrough    // guest export names
```

Passthrough imports are derived: `G`'s imports minus `virtualized`, each bound
to the donut import with the same `Resolve::canonicalized_id_of` (semver merging
with the donut's own `std` imports is unchanged from P§5.1).

## 7. Encoder: hole mode (wit-component)

Turned on by one or more `wit-bindgen-child:*` sections. The guest binary is never
an input: `ComponentEncoder::donut_guest` and `component new --donut-guest` are
removed. Changes from P§5.1:

* **The hole.** Per child, `(import "<child>" (component $G))`, where `$G` is `G`
  encoded as a component type (the same world → component-type encoding
  wit-component uses for WIT packages). The hole's type is exactly `G`, so a
  plugged guest must import a subset of `G`'s imports (§8).
* **Import classification.** `[child=<name>]<iface>` modules are classified as
  `GuestExportFunc`/`GuestResourceDrop` of that child. Virtual exports listed in
  any manifest are internal.
* **Passthrough imports** are kept even when the core module doesn't use them, as
  before. They come from `G` now, not from a decoded guest binary.
* **Instantiation, per child, in manifest order** (the order the sections appear
  in the linked module), after phase-1 fixup (host lowerings, dtors, `_initialize`):
  1. instantiate the hole with the child's virtual instances and passthrough imports;
  2. alias its exports, `canon lower` them, and `canon resource.drop` for its
     resources;
  3. run a fixup module that patches **this child's** slots only;
  4. alias and export its `export_passthrough` interfaces.

  Patching after each child, rather than once at the end, means a later child's
  start function can call donut code that calls an earlier child. Calling a child
  that isn't linked yet traps (`uninitialized element`), as in P§8.
* **Errors:** duplicate child names; a hole name colliding with another import;
  a re-exported interface colliding with a donut export.

Because the guest is no longer an input, the encoder's output depends only on the
core module. So `cargo build` alone can produce the holey component once
wasm-component-ld uses this wit-component, with no `--donut-guest` flag. Until then:
`-Clink-arg=--skip-wit-component`, then `wasm-tools component new`.

## 8. Plugging (wac)

`wac plug donut.wasm --plug app.wasm` builds the Explainer's `$VirtualizedGuest`:
import the donut's imports, embed both components, `instantiate $donut (with "app"
(component $app))`, and re-export the donut's exports. This is ordinary
composition. What wac has and lacks today (0.12):

* has: `ItemKind::Component`, world subtyping in `SubtypeChecker` (`checker.rs`),
  and encoding of component-typed imports;
* lacks: a graph node for "a registered package, as a component value", so it can
  be an instantiation argument. `plug` only aliases plug *exports* into socket
  imports.

**Changes:**
* `CompositionGraph`: a node that refers to a package's component definition, and
  `set_instantiation_argument` accepting it.
* `plug`: for each plug, first try the socket's component-typed imports. If
  exactly one accepts the plug's type, pass the plug component. If none do, fall
  back to today's export matching. If several do, it's an error that asks for
  `--plug <hole>=<path>` (new syntax, also usable to be explicit).

Wasmtime's inliner already accepts components as `instantiate` arguments
(`ComponentItemDef::Component` in `translate/inline.rs`). Only exporting a
component from the root is unsupported, which this never does.

**Version skew.** Component subtyping matches import names exactly (wasmparser
`open_instance_type` looks args up by name). A guest built with a newer Rust
`std` imports e.g. `wasi:cli/environment@0.2.9`, but a hole typed with `@0.2.6`
doesn't provide that name, so the plug fails. First cut: `G` must use the WASI
version the guest was built with, and wac's error says which name mismatched.
Later: wac inserts a small renaming wrapper component for semver-compatible names
(it already has `are_semver_compatible`). That's safe in practice because
wit-component prunes a guest's import instance types to the functions it uses.

## 9. Work plan

Code moves from `prototype` to `main` in this order, trimmed as it moves.

| Step | Where | Work |
|---|---|---|
| 1 | `spike/` | Rewrite `donut.wat.in` in hole form, with two children, plug it by hand, and run it on Wasmtime 49. This checks component-valued arguments and per-child fixups before any code. |
| 2 | this repo | `donut-plan` (without `forward.rs`) and `child!`: options, patterns, checks (§4), `mod <name>` and per-child naming (§5), manifest v1. Snapshot tests, including two children. |
| 3 | wit-component | Hole mode (§7). Remove embed mode and `--donut-guest`. |
| 4 | wac | Component-valued plug (§8). |
| 5 | this repo | End-to-end tests (`cargo build` → `component new` → `wac plug` → Wasmtime). Port `examples/virtualize`, port `examples/secrets` with hand-written impls, and add a two-child example. Drop `examples/forward` and `examples/fastly` (they're about generated forwarding and mocks). |
| 6 | upstream | PRs for steps 3 and 4. wasm-component-ld picks up hole mode with no new flag. |

## 10. Open questions

1. **Nested placement.** `child!` only works at the crate root. If that turns out
   too restrictive, a `module: "crate::a::b"` option could prefix the paths.
2. **Mediated calls between children.** Rust code may call child B while handling
   a callback from child A. That isn't direct child-to-child wiring, so it's
   allowed. Calling back into a child that is already on the stack is not; we
   still need to confirm that Wasmtime traps there.
3. **Hole type vs. guest's actual imports.** Should `child!` accept several worlds
   (`guest: ["example:app/app", "wasi:cli/imports@0.2.6"]`) so users don't have to
   edit the guest's WIT to add `include wasi:…`?
4. **Crate and section naming.** The macro is `donut::child!`. Should the crates and
   wit-component's API (`wit_component::donut`) be renamed too?
