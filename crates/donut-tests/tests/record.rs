//! `examples/record`: a recording donut (proxy-component's record mode as one
//! component) has the same world as the app it wraps. Run the app bare and
//! wrapped on the same host `kv`: the results must be identical, and the
//! wrapper must have recorded every call in both directions.

use donut_tests::build;
use std::collections::HashMap;
use std::path::Path;
use wasmtime::component::{Component, Linker, Resource, ResourceTable};
use wasmtime::{Engine, Store};
use wasmtime_wasi::p2::pipe::MemoryOutputPipe;
use wasmtime_wasi::{WasiCtx, WasiCtxView, WasiView};

wasmtime::component::bindgen!({
    path: "../../examples/record/wit",
    world: "example:rec/app-world",
    with: { "example:rec/kv.store": KvStore },
});

/// The host's `store`.
#[derive(Default)]
pub struct KvStore(HashMap<String, String>);

struct State {
    ctx: WasiCtx,
    table: ResourceTable,
    /// Host-side log of `kv` calls, to check the recorder forwarded them.
    calls: Vec<String>,
}

impl WasiView for State {
    fn ctx(&mut self) -> WasiCtxView<'_> {
        WasiCtxView { ctx: &mut self.ctx, table: &mut self.table }
    }
}

impl example::rec::kv::Host for State {
    fn open(&mut self, name: String) -> Resource<KvStore> {
        self.calls.push(format!("open {name}"));
        self.table.push(KvStore::default()).unwrap()
    }
}

impl example::rec::kv::HostStore for State {
    fn set(&mut self, s: Resource<KvStore>, key: String, value: String) {
        self.calls.push(format!("set {key}={value}"));
        self.table.get_mut(&s).unwrap().0.insert(key, value);
    }
    fn get(&mut self, s: Resource<KvStore>, key: String) -> Option<String> {
        self.calls.push(format!("get {key}"));
        self.table.get(&s).unwrap().0.get(&key).cloned()
    }
    fn drop(&mut self, s: Resource<KvStore>) -> wasmtime::Result<()> {
        self.calls.push("drop".into());
        self.table.delete(s)?;
        Ok(())
    }
}

struct Outcome {
    results: Vec<String>,
    host_calls: Vec<String>,
    stdout: String,
}

/// Runs `app.run` and a `session` against a fresh host store.
fn drive(component: &Path) -> wasmtime::Result<Outcome> {
    let engine = Engine::default();
    let component = Component::from_file(&engine, component)?;
    let mut linker = Linker::<State>::new(&engine);
    wasmtime_wasi::p2::add_to_linker_sync(&mut linker)?;
    AppWorld::add_to_linker::<_, wasmtime::component::HasSelf<_>>(&mut linker, |s| s)?;
    let stdout = MemoryOutputPipe::new(1 << 20);
    let ctx = WasiCtx::builder().stdout(stdout.clone()).build();
    let mut store = Store::new(
        &engine,
        State { ctx, table: ResourceTable::new(), calls: Vec::new() },
    );
    let app = AppWorld::instantiate(&mut store, &component, &linker)?;
    let kv = store.data_mut().table.push(KvStore::default())?;
    let borrow = || Resource::<KvStore>::new_borrow(kv.rep());

    let mut results = vec![app.example_rec_app().call_run(&mut store, borrow())?];
    let session = app.example_rec_app().session();
    let s = session.call_constructor(&mut store, "ann")?;
    results.push(session.call_greet(&mut store, s, borrow())?);
    s.resource_drop(&mut store)?;

    Ok(Outcome {
        results,
        host_calls: store.data().calls.clone(),
        stdout: String::from_utf8_lossy(&stdout.contents()).into_owned(),
    })
}

#[test]
fn recorder_is_transparent_and_records_both_directions() -> anyhow::Result<()> {
    let dir = build("record")?;
    let w = |e: wasmtime::Error| anyhow::anyhow!("{e:?}");
    let bare = drive(&dir.join("out/app.wasm")).map_err(w)?;
    let wrapped = drive(&dir.join("out/recorder.wasm")).map_err(w)?;

    // Transparent: same results, same host calls.
    assert_eq!(
        bare.results,
        [
            "visits=Some(\"1\") cache=Some(\"v\")",
            "hello ann (visits=Some(\"1\"))",
        ]
    );
    assert_eq!(wrapped.results, bare.results);
    assert_eq!(wrapped.host_calls, bare.host_calls);
    assert!(bare.stdout.is_empty());

    // Recorded, in order: host → app calls (exports), and the app's calls to
    // the host (imports), interleaved on the same stack.
    assert_eq!(
        wrapped.stdout.lines().collect::<Vec<_>>(),
        [
            "record: export app.run",
            "record: import store.set(\"visits\", \"1\")",
            "record: import kv.open(\"cache\")",
            "record: import store.set(\"k\", \"v\")",
            "record: import store.get(\"visits\") -> Some(\"1\")",
            "record: import store.get(\"k\") -> Some(\"v\")",
            "record: export app.run -> \"visits=Some(\\\"1\\\") cache=Some(\\\"v\\\")\"",
            "record: export session.new(\"ann\")",
            "record: import store.get(\"visits\") -> Some(\"1\")",
            "record: export session.greet -> \"hello ann (visits=Some(\\\"1\\\"))\"",
            "record: export session.drop",
        ]
    );
    Ok(())
}

/// The same recording donut with generated forwarding (`forward:`): just as
/// transparent, and its hooks see every call and drop.
#[test]
fn generated_forwarding_is_transparent_and_hooks_see_every_call() -> anyhow::Result<()> {
    let dir = build("record")?;
    let w = |e: wasmtime::Error| anyhow::anyhow!("{e:?}");
    let bare = drive(&dir.join("out/app.wasm")).map_err(w)?;
    let wrapped = drive(&dir.join("out/forwarder.wasm")).map_err(w)?;
    assert_eq!(wrapped.results, bare.results);
    assert_eq!(wrapped.host_calls, bare.host_calls);
    assert_eq!(
        wrapped.stdout.lines().collect::<Vec<_>>(),
        [
            "record: Export example:rec/app.run[<store>]",
            "record: Import example:rec/kv.[method]store.set[\"visits\", \"1\"]",
            "record: Import example:rec/kv.[method]store.set -> ()",
            "record: Import example:rec/kv.open[\"cache\"]",
            "record: Import example:rec/kv.open -> <store>",
            "record: Import example:rec/kv.[method]store.set[\"k\", \"v\"]",
            "record: Import example:rec/kv.[method]store.set -> ()",
            "record: Import example:rec/kv.[method]store.get[\"visits\"]",
            "record: Import example:rec/kv.[method]store.get -> Some(\"1\")",
            "record: Import example:rec/kv.[method]store.get[\"k\"]",
            "record: Import example:rec/kv.[method]store.get -> Some(\"v\")",
            // The app dropped its `cache` store: the virtual wrapper and, with
            // it, the host's store.
            "record: Import example:rec/kv.store dropped",
            "record: Export example:rec/app.run -> \"visits=Some(\\\"1\\\") cache=Some(\\\"v\\\")\"",
            "record: Export example:rec/app.[constructor]session[\"ann\"]",
            "record: Export example:rec/app.[constructor]session -> <session>",
            "record: Export example:rec/app.[method]session.greet[<store>]",
            "record: Import example:rec/kv.[method]store.get[\"visits\"]",
            "record: Import example:rec/kv.[method]store.get -> Some(\"1\")",
            "record: Export example:rec/app.[method]session.greet -> \"hello ann (visits=Some(\\\"1\\\"))\"",
            "record: Export example:rec/app.session dropped",
        ]
    );
    Ok(())
}
