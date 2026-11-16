# donut wrapping for Rust components

`donut!` lets a Rust component **wrap another component**:
* it implements (*virtualizes*) some of the inner component's imports,
* embeds and instantiates the inner component (the **guest**), and
* calls the guest's exports from its own code, while the guest calls back into
  it on the same stack.

The result is a single component with whatever world you choose. This is the
"donut wrapping" pattern from the component model Explainer's
[Virtualization](https://github.com/WebAssembly/component-model/blob/main/design/mvp/Explainer.md)
section. Use it to:
* sandbox or mock a guest's host APIs;
* run a guest against a host it wasn't built for;
* record or replay a guest's calls;
* build a server that embeds plugin components.

## Requirements

* Rust with the `wasm32-wasip2` target: `rustup target add wasm32-wasip2`
* `wasm-tools` from the donut branch, which adds `component new --donut-guest`:
  ```sh
  cargo install --git https://github.com/chenyan2002/wasm-tools --branch donut wasm-tools --locked
  ```
  (or install it elsewhere with `--root`, and set `WASM_TOOLS=<path>/bin/wasm-tools`).
* **Wasmtime ≥ 49** to run the result. Wasmtime 48 traps with "cannot enter component
  instance" when a component calls into a component nested inside it.

## Example: an app on a host it wasn't built for

An app was written for a platform with a filesystem: it reads its settings from
`/etc/app.conf` and calls an internal API. You want to run it on a new host,
which has no filesystem, and which keeps API tokens in a secret vault. Requests
to the internal API must carry a token, the app must never see one, and it
shouldn't be able to reach hosts you don't have tokens for.

You can't change the app. Instead, you wrap it in a donut that runs on the new
host. The code is in [`examples/secrets`](examples/secrets), and `cargo test`
runs it on a Wasmtime host
([`crates/donut-tests/tests/secrets.rs`](crates/donut-tests/tests/secrets.rs)).

### The WIT

```wit
package example:app;

interface types {
  resource request {
    constructor(url: string);
    url: func() -> string;
    set-header: func(name: string, value: string);
  }
}
interface outgoing { 
  use types.{request}; 
  send: func(r: request) -> result<u16, string>; 
}
interface handler { 
  handle: func(path: string) -> string; 
}

interface fs {  // only on the guest side
  resource file { 
    read: func() -> string; 
    write: func(data: string); 
  }
  open: func(path: string) -> result<file, string>;
}
interface secrets {  // only on the host side
  resource vault {
    open: static func(name: string) -> vault;
    get: func(key: string) -> option<string>;
  }
}

world app  { 
  import outgoing; 
  import fs;
  export handler; 
}
world host { 
  import outgoing; 
  import secrets;
  export handler; 
}
```

The app ([`app/src/lib.rs`](examples/secrets/app/src/lib.rs)) is an ordinary
component, and knows nothing about donuts.

```rust
wit_bindgen::generate!({ path: "../wit", world: "app" });
use example::app::{fs, outgoing, types::Request};

struct App;
impl exports::example::app::handler::Guest for App {
    fn handle(path: String) -> String {
        let conf = fs::open("/etc/app.conf").map(|f| f.read()).unwrap_or_default();
        let upstream = conf.trim().strip_prefix("upstream=").unwrap_or("https://example.com");
        println!("app: {path} -> {upstream}");
        // It also phones home.
        if let Err(e) = outgoing::send(Request::new("https://telemetry.example/ping")) {
            println!("app: telemetry failed: {e}");
        }
        match outgoing::send(Request::new(&format!("{upstream}{path}"))) {
            Ok(status) => format!("status {status}"),
            Err(e) => format!("error: {e}"),
        }
    }
}
export!(App);
```

### The donut

Your component has the host's world, and embeds the app. For each of the app's
imports, you choose who implements it
([`donut/src/lib.rs`](examples/secrets/donut/src/lib.rs)):

```rust
donut::donut!({
    path: "../wit",
    world: "example:app/host",          // your host's world
    guest: "example:app/app",           // the guest's world
    // Allowed: passed on to the host (requests) or the app (`handle`).
    forward: ["example:app/types", "example:app/handler"],
    // Customize: control the only way out to the network.
    virtualize: ["example:app/outgoing"],
    // Missing on this host: only what the app uses. A mocked `file` is an
    // `OpenFile`.
    mock: [{ "example:app/fs": { file: OpenFile } }],
    hooks: Policy,
});

use example::app::{outgoing as host_outgoing, secrets::Vault};      // the host's
use exports::virtual_example::app::{fs::File, types::Request};      // the app's
use forward::{FwdExampleAppTypesRequest, MockExampleAppFsFile};

const CONF: &str = "upstream=https://api.internal";

/// What a mocked `fs` file holds.
pub struct OpenFile {
    contents: String,
}

pub struct Policy;

impl exports::virtual_example::app::outgoing::Guest for Policy {
    fn send(r: Request) -> Result<u16, String> {
        // The app's request wraps a host request: take it back.
        let req = r.into_inner::<FwdExampleAppTypesRequest>().into_host();
        let url = req.url();
        let host = url.split("://").nth(1).and_then(|s| s.split('/').next()).unwrap_or_default();
        let token = Vault::open("tokens").get(host).ok_or_else(|| format!("{host}: not allowed"))?;
        req.set_header("authorization", &format!("Bearer {token}"));
        host_outgoing::send(req)
    }
}

impl forward::Hooks for Policy {
    // Every forwarded and mocked call.
    fn before(call: &forward::Call, args: &[&dyn core::fmt::Debug]) {
        eprintln!("donut: {} {}{args:?}", call.interface, call.function);
    }

    // The app reads one settings file. Every other `fs` function traps.
    fn example_app_fs_open(path: String) -> Result<File, String> {
        match path.as_str() {
            "/etc/app.conf" => Ok(File::new(MockExampleAppFsFile::new(OpenFile { contents: CONF.into() }))),
            _ => Err(format!("{path}: no such file")),
        }
    }
    fn example_app_fs_file_read(this: &MockExampleAppFsFile) -> String {
        this.contents.clone()
    }
}

export!(Policy);
```

What goes where, and why:

| Interface | | |
|---|---|---|
| `handler` | `forward` | Both worlds export it. The host calls your component, which calls the app. |
| `types` | `forward` | There's no policy on building a request, so the generated code passes every call on to the host. The app's requests wrap host requests (`FwdExampleAppTypesRequest`), so nothing is copied. |
| `outgoing` | `virtualize` | The gate. You write `send`: find the token, attach it, send, or refuse. |
| `fs` | `mock` | The host has no filesystem. Every function is generated, defaulting to `unimplemented!()`, and you override the two the app calls. A mocked `file` holds an `OpenFile`, declared in `mock`; the wrapper `MockExampleAppFsFile` derefs to it. |
| `secrets` | — | Not a guest import: an ordinary import of your world, which only your code can reach. |
| WASI | not listed | *Passthrough*: the app's `println!` goes straight to the host. |

The host sees a component with exactly its own world: `fs` is gone, and nothing
of the vault leaks to the app. A call to `handle("/hello")` returns `status 200`.
The host receives one request, to `https://api.internal/hello`, with the token
attached, and the app prints:

```
app: /hello -> https://api.internal
app: telemetry failed: telemetry.example: not allowed
```

#### Why `outgoing` is virtualized, not forwarded

Forwarding `outgoing` and overriding `Hooks::example_app_outgoing_send` with the
same code would behave the same today. The difference is in the code you don't
write. Each list has a default for every function you don't implement:

| List | A function you don't write | Fails |
|---|---|---|
| `forward` | calls the host | open |
| `mock` | panics (`unimplemented!()`) | closed, at run time |
| `virtualize` | doesn't compile | closed, at compile time |

If `outgoing` gains a `send-many` in a later version, forwarding would send it
to the host without a token, and still compile. Virtualizing stops the build
until you write it. Forwarding also generates `forward::example_app_outgoing::send`,
an unchecked path to the network. With `virtualize`, the only path is your code.

The same reasoning puts `types` in `forward` (nothing to check) and `fs` in
`mock` (a missing function should fail, not silently succeed). In short:
**forward** what you allow, **mock** what's missing, and **virtualize** what you
decide.

### Setup and build

```toml
[lib]
crate-type = ["cdylib"]

[dependencies]
donut = { git = "https://github.com/chenyan2002/donut-wrapping" }
wit-bindgen = "0.63"

# Required: `donut!` generates code with unreleased wit-bindgen and
# wit-component changes. Patches only apply in your own workspace, so copy
# this block (from this repo's Cargo.toml) into it.
[patch.crates-io]
wit-bindgen = { git = "https://github.com/chenyan2002/wit-bindgen", branch = "donut" }
wit-bindgen-rust-macro = { git = "https://github.com/chenyan2002/wit-bindgen", branch = "donut" }
wit-bindgen-rust = { git = "https://github.com/chenyan2002/wit-bindgen", branch = "donut" }
wit-bindgen-core = { git = "https://github.com/chenyan2002/wit-bindgen", branch = "donut" }
wit-component = { git = "https://github.com/chenyan2002/wasm-tools", branch = "donut" }
wit-parser = { git = "https://github.com/chenyan2002/wasm-tools", branch = "donut" }
wasm-encoder = { git = "https://github.com/chenyan2002/wasm-tools", branch = "donut" }
wasm-metadata = { git = "https://github.com/chenyan2002/wasm-tools", branch = "donut" }
wasmparser = { git = "https://github.com/chenyan2002/wasm-tools", branch = "donut" }
```

Build your component, stopping after linking, then embed the app:

```sh
RUSTFLAGS="-Clink-arg=--skip-wit-component" \
  cargo build --release --target wasm32-wasip2
wasm-tools component new target/wasm32-wasip2/release/donut.wasm \
  --donut-guest app.wasm -o app-on-host.wasm
```

The app is chosen at this step, not at compile time: any component targeting
`app` can be embedded without rebuilding your code.

## Reference

### `donut!` options

| Option | |
|---|---|
| `path` | WIT directory or directories, relative to the crate (default `"wit"`) |
| `world` | your component's world |
| `guest` | the guest's world |
| `virtualize` | guest imports you implement |
| `forward` | guest imports to forward to the host, and interfaces both worlds export to forward to the guest |
| `mock` | guest imports to mock: generated impls defaulting to `unimplemented!()`. An entry is an interface name, or `{ "<interface>": { <resource>: <Type>, … } }` to give resources a state type |
| `hooks` | the type (at the crate root) implementing `forward::Hooks` (required with `forward` or `mock`) |

Lists take interface names, with a version if the WIT has one
(`"fastly:compute/http-req@0.1.0"`).

A mocked resource holds the state type you declare for it, which must be
`pub`. Resources are named in snake case (`incoming_request` for
`incoming-request`) or as strings. A resource you don't declare can't be
created, which fits interfaces the guest imports but never reaches: every
function stays `unimplemented!()`, and an override that tries to create one
doesn't compile.

### What happens to each guest import

| Guest import is… | Your component | Who implements it |
|---|---|---|
| in `virtualize` | doesn't import it | you: a wit-bindgen export trait |
| in `mock` | doesn't import it | generated; you override `Hooks` methods |
| in `forward` | imports it from the host | generated, calling the host; you override `Hooks` methods |
| not listed (*passthrough*) | imports it from the host | the host, called directly by the guest |

Passthrough includes imports that are in the guest's binary but not in its WIT,
such as the WASI imports `std` adds: the encoder reads them from the guest. If
the guest and your component import different versions of a package (`0.2.0` and
`0.2.9`), they're merged into the highest.

Each interface goes in at most one list. You can mix the lists, with two rules,
both checked at compile time:
* **A passthrough import can't use a resource you virtualize or mock.** For
  example, you can't virtualize `types` but pass `outgoing` through: the host's
  `send` can't take your `request`. Virtualize `outgoing` too.
* **A forwarded import can't use a resource you virtualize or mock.** For example,
  you can't virtualize `types` but forward `outgoing`: there's no host `request`
  to pass on. Forward `types` too, or virtualize `outgoing`.

The other way around is fine, as in the example: `outgoing` is virtualized and
uses the forwarded `types`, so `send` receives wrappers over host requests. A
virtualized `outgoing` could equally use a passthrough `types`, and then
receive the host's requests directly.

### Generated names

`donut!` generates the usual wit-bindgen bindings for your world, plus:

| Item | Name |
|---|---|
| The guest's side of an import you list | `exports::virtual_<ns>::<pkg>::<iface>`, as a wit-bindgen export |
| The host's side (forwarded, passthrough, or your world's own imports) | `<ns>::<pkg>::<iface>`, as a wit-bindgen import |
| A guest export you call | `guest::<ns>::<pkg>::<iface>::<function>` |
| Forwarded resource wrapper | `forward::Fwd<Ns><Pkg><Iface><Resource>` |
| Mocked resource wrapper | `forward::Mock<Ns><Pkg><Iface><Resource>`: `new(state)`, `into_inner()`, and `Deref` to the state |
| `Hooks` method | `<ns>_<pkg>_<iface>_<function>`, or `<ns>_<pkg>_<iface>_<resource>_<method>` (constructors are `new`) |
| Its default | `forward::<ns>_<pkg>_<iface>::<function>`, or `forward::<ns>_<pkg>_<iface>_<resource>::<method>` |
| Hooks support | `forward::Hooks`, `forward::Call`, `forward::Direction` |

The guest's side is always a copy of the interface, in a package renamed
`virtual-<ns>:<pkg>`, so it never collides with the host's side, which your
component may also import (`outgoing` in the example) or export. The rename is
internal: the guest still imports `example:app/outgoing`. Only the interfaces
you list are copied; anything else they use, such as the rest of the package,
is the original.

Names are converted to Rust case (`fastly:compute/http-req` becomes
`fastly_compute_http_req_…` and `FastlyComputeHttpReq…`), and versions are left
out. None of them depend on what else is listed, and they're fully qualified:
`wasi:http/types` and `example:http/types` can be forwarded together.

## Things to know

* **Re-entrance.** Any `guest::…` call can call back into your virtual interfaces
  and run their resources' destructors before it returns. Don't hold `RefCell`
  borrows or `get()` references across a guest call.
* **Calling the guest during instantiation traps.** A guest's start function can
  call your virtual functions. But if those call back into the guest, they trap
  (`uninitialized element`), because the guest's exports aren't linked yet.
* **Borrows you received can't be passed on to the guest.** You can lend the guest
  handles you own, but not a borrow the guest gave you.
* `donut!` must be invoked at the **crate root**. Only the options above are
  supported (not yet the other `generate!` options).
* Not supported yet:
  * world-level (non-interface) guest imports and exports;
  * `forward` of a host borrow nested deeper than one list or option, e.g.
    `list<record { borrow<r> }>`;
  * Rust *exports* taking `list<borrow<R>>` of an imported resource, which hit a
    wit-bindgen 0.63 bug, in your donut or in a guest.

## Examples

```sh
cargo test        # builds every example and runs it on Wasmtime 49
```

`cargo test` needs `WASM_TOOLS` (or `wasm-tools` on `PATH`) from the donut branch.

| Example | What it shows |
|---|---|
| [`examples/secrets`](examples/secrets) | The example above: `forward`, `virtualize`, `mock`, and passthrough in one donut, whose world differs from the guest's. |
| [`examples/virtualize`](examples/virtualize) | The corner cases of virtualizing: re-entrant calls, resources defined by the donut and by the guest, records holding borrows and owned handles, a guest that imports and exports the same interface (middleware), passthrough, and guests whose start function calls the donut. |
| [`examples/forward`](examples/forward) | **A transparent wrapper.** The donut has the same world as the app it wraps, and forwards every call to the host or the app. `recorder` logs every call; `interceptor` changes two of them. |
| [`examples/fastly`](examples/fastly) | **Mocking a large API.** A mock Fastly Compute host around a real Fastly app ([proxy-component](https://github.com/chenyan2002/proxy-component)'s `tests/rust.wasm`; set `GUEST=<path>`). It mocks 60 functions and overrides the 13 the app uses. |

## How it works

* [`docs/design.md`](docs/design.md) is the design: how the guest's world is
  inverted into bindings, and how wit-component's donut mode builds the nested
  component, with a linkage table patched after the guest is instantiated.
* [`docs/plan.md`](docs/plan.md) has the implementation status.
* [`spike/`](spike) has a hand-written version of what the encoder produces.
