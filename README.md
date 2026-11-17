# Donut wrapping for Rust components

`donut::child!` lets a Rust component wrap other components, its *children*.
For each child, your component:

* implements the interfaces the child imports (it *virtualizes* them), or
  passes them straight through from the host;
* calls the child's exports from its own code;
* leaves a typed *hole* where the child goes. `wac plug` fills it with a real
  component later, so one built donut can wrap any component of the right type.

This is the [donut wrapping](https://github.com/WebAssembly/component-model/blob/main/design/mvp/Explainer.md#virtualization-and-donut-wrapping)
pattern from the component model Explainer. There is a prototype implementation with more features on the
[prototype branch](https://github.com/chenyan2002/donut-wrapping/tree/prototype).

## Example: one app, two configurations

An app greets people, using a greeting it reads from a `settings` resource
that comes with each request. We want to run it twice in one component, once
in English and once in French, and route requests to the right copy. The donut
has two children, `english` and `french`, both of the app's world. Each child
gets its own `config`, whose `settings` the donut makes, so the same app binary
behaves differently in each hole.

The code is in [`examples/greeter`](examples/greeter): `wit/`, `app/` (crate
`greeter-app`), and `donut/` (crate `greeter-donut`). Its `run.sh` runs the
steps below and checks the output.

### WIT

```wit
package example:greeter;

interface config {
  resource settings {
    get: func(key: string) -> option<string>;
  }
}

interface greeting {
  use config.{settings};
  greet: func(settings: borrow<settings>, name: string) -> string;
}

interface handler {
  handle: func(path: string) -> string;
}

world app {
  include wasi:cli/imports@0.2.9;
  import config;
  export greeting;
}

world host {
  export handler;
}
```

### The app

An ordinary component that knows nothing about donuts:

```rust
wit_bindgen::generate!({ path: "../wit", world: "example:greeter/app", generate_all });

use example::greeter::config::Settings;

struct App;

impl exports::example::greeter::greeting::Guest for App {
    fn greet(settings: &Settings, name: String) -> String {
        let greeting = settings.get("greeting").unwrap_or_else(|| "hello".to_string());
        format!("{greeting}, {name}!")
    }
}

export!(App);
```

### The donut

```rust
// The donut's own world.
wit_bindgen::generate!({ path: "../wit", world: "example:greeter/host" });

// Two children of the same world. Everything they import is virtualized,
// except WASI, which goes straight to the host.
donut::child!({
    name: "english",
    path: "../wit",
    guest: "example:greeter/app",
    import_passthrough: ["wasi:*"],
});
donut::child!({
    name: "french",
    path: "../wit",
    guest: "example:greeter/app",
    import_passthrough: ["wasi:*"],
});

struct Donut;

// The donut's own export: route `<lang>/<name>` to a child, with settings
// in its own `config`.
impl exports::example::greeter::handler::Guest for Donut {
    fn handle(path: String) -> String {
        match path.split_once('/') {
            Some(("en", name)) => {
                let settings = english::virt::example::greeter::config::Settings::new(Settings("Hello"));
                english::guest::example::greeter::greeting::greet(&settings, name)
            }
            Some(("fr", name)) => {
                let settings = french::virt::example::greeter::config::Settings::new(Settings("Bonjour"));
                french::guest::example::greeter::greeting::greet(&settings, name)
            }
            _ => format!("unknown route `{path}`"),
        }
    }
}

/// A child's `settings`: one Rust type behind both children's resources.
struct Settings(&'static str);

impl Settings {
    fn get(&self, key: &str) -> Option<String> {
        (key == "greeting").then(|| self.0.to_string())
    }
}

// Each child's `config`.
impl english::virt::example::greeter::config::Guest for Donut {
    type Settings = Settings;
}
impl english::virt::example::greeter::config::GuestSettings for Settings {
    fn get(&self, key: String) -> Option<String> {
        Settings::get(self, &key)
    }
}
impl french::virt::example::greeter::config::Guest for Donut {
    type Settings = Settings;
}
impl french::virt::example::greeter::config::GuestSettings for Settings {
    fn get(&self, key: String) -> Option<String> {
        Settings::get(self, &key)
    }
}

export!(Donut);
english::export!(Donut);
french::export!(Donut);
```

Each `child!` expands to a module named after the child:

| Path | What |
|---|---|
| `english::virt::<ns>::<pkg>::<iface>` | the interfaces the child imports and the donut implements: implement their `Guest` traits, and a `Guest<Resource>` trait per resource. A resource type here (`english::virt::…::config::Settings`) is the donut's own: make one with `Settings::new(value)` |
| `english::guest::<ns>::<pkg>::<iface>` | functions that call the child's exports. They take and return the `virt` resource types, so `greet` takes a `&english::virt::…::config::Settings` |
| `english::export!` | registers your implementation, like `generate!`'s `export!` |

The donut crate depends on `donut` and `wit-bindgen`, and is a `cdylib`.

### Building and running

```sh
# 1. The donut, to a core module.
RUSTFLAGS="-Clink-arg=--skip-wit-component" \
  cargo build --release --target wasm32-wasip2 -p greeter-donut

# 2. The core module, to a component with two holes.
wasm-tools component new target/wasm32-wasip2/release/greeter_donut.wasm -o donut.wasm

# 3. The app: an ordinary component.
cargo build --release --target wasm32-wasip2 -p greeter-app

# 4. Plug the app into both holes.
wac plug donut.wasm \
  --plug english=target/wasm32-wasip2/release/greeter_app.wasm \
  --plug french=target/wasm32-wasip2/release/greeter_app.wasm \
  -o composed.wasm

# 5. Run.
wasmtime run --invoke 'handle("en/Ada")' composed.wasm   # "Hello, Ada!"
wasmtime run --invoke 'handle("fr/Ada")' composed.wasm   # "Bonjour, Ada!"
```

What each step does:

1. **`cargo build`** compiles the donut. `child!` generated, per child, the
   bindings above, and a `wit-bindgen-child:<name>` custom section (the child's
   *manifest*): the child's world, and which interfaces are virtualized,
   passed through, and called. `--skip-wit-component` makes the linker stop at
   the core module: the stock componentizer doesn't know about holes yet.
2. **`wasm-tools component new`** turns the core module into a component, in
   *hole mode*, because the module has manifests. For each child, the
   component:
   * imports a component of the child's world: `(import "english" (component …))`;
   * instantiates it, with the donut's `config` implementation and the host's
     WASI interfaces;
   * links the child's exports to the calls in `english::guest::…`, before the
     next child is instantiated.

   The result's own imports are the host's WASI interfaces (the donut's and the
   ones passed through), plus the two holes. Its export is the donut's
   `handler`.
3. **The app** is built the usual way. It doesn't know it'll be wrapped.
4. **`wac plug`** builds a component that embeds the donut and the app, and
   instantiates the donut with the app as both holes. A plug fills the one hole
   its type fits; here both holes have the same type, so each plug names its
   hole (`english=…`). The holes are gone from the result, which only imports
   WASI.
5. **Wasmtime** runs it like any component.

The same `donut.wasm` can be plugged with any components of the app's world,
without rebuilding the donut.

## Options

| `child!` option | |
|---|---|
| `name` | required: the module, in snake case; its kebab-case form is the hole's name |
| `guest` | required: the child's world, which is the hole's exact type |
| `path` | WIT path(s), relative to the crate's manifest directory; default `"wit"` |
| `import_passthrough` | imports of the child passed straight through from the host; every other import is virtualized |
| `export_passthrough` | exports of the child the donut re-exports unchanged (no Rust code involved) |
| `with`, `additional_derives`, `runtime_path` | as in `wit_bindgen::generate!`, for the child's bindings |

Passthrough entries are `ns:pkg/iface`, `ns:pkg/iface@version`, `ns:pkg/*`
(optionally with `@version`), or `ns:*`.

## Things to know

* **A hole's type is exactly the child's world.** A component plugged into it
  must import a subset of that world's imports, by exact name. So the world has
  to include the WASI interfaces the guests import, at the guests' version:
  `wasi:cli/imports@0.2.9` for Rust `std` today. A guest built against another
  WASI version doesn't fit the hole. The composed component imports everything
  in the world (here all of `wasi:cli/imports`, sockets and filesystem
  included), even what the plugged app doesn't use.
* **Re-entrance.** A call into a child can call back into the donut's
  virtualized interfaces before it returns.
* **Instantiation order.** Children are instantiated one after another, in an
  unspecified order decided by `rustc`. A child's start function must not call code that calls
  another child: that child may not be linked yet, and the call traps.
* **Children don't call each other directly.** A child's imports are
  implemented by the donut or the host, never by another child.
* Component model p2 only: no `async`, `stream`, or `future` in a child's world.

## Requirements

* Rust with the `wasm32-wasip2` target: `rustup target add wasm32-wasip2`.
* `wasm-tools` and `wac` built from their `donut-hole` branches, which add hole
  mode and component-typed plugs. Neither is upstream yet.
* Wasmtime ≥ 49 to run the result.

The crates themselves build on stock releases: wit-bindgen 0.63 and wasm-tools
0.262.
