//! `examples/forward`: donuts with generated forwarding (`forward:`) around an
//! app, with the same world as the app (`import kv; export app`), run on a
//! wasmtime host `kv` next to the bare app. The hand-written equivalent of the
//! recorder is `spike/recorder`.

use donut_tests::build;
use std::collections::HashMap;
use std::path::Path;
use wasmtime::component::{Component, Linker, Resource, ResourceTable};
use wasmtime::{Engine, Store};
use wasmtime_wasi::p2::pipe::MemoryOutputPipe;
use wasmtime_wasi::{WasiCtx, WasiCtxView, WasiView};

wasmtime::component::bindgen!({
    path: "../../examples/forward/wit",
    world: "example:rec/app-world",
    with: { "example:rec/kv.store": KvStore },
});

/// The host's `store`.
#[derive(Default)]
pub struct KvStore(HashMap<String, String>);

use example::rec::kv::{Entry, Lookup, Pair, View};

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
    fn open_all(&mut self, names: Vec<String>) -> Vec<Resource<KvStore>> {
        self.calls.push(format!("open-all {names:?}"));
        names.iter().map(|_| self.table.push(KvStore::default()).unwrap()).collect()
    }
    fn open_pair(&mut self, a: String, b: String) -> Pair {
        self.calls.push(format!("open-pair {a} {b}"));
        Pair {
            first: self.table.push(KvStore::default()).unwrap(),
            second: self.table.push(KvStore::default()).unwrap(),
        }
    }
    fn find(&mut self, name: String) -> Lookup {
        self.calls.push(format!("find {name}"));
        if name.starts_with('x') {
            Lookup::Found(self.table.push(KvStore::default()).unwrap())
        } else {
            Lookup::Missing
        }
    }
    fn count(&mut self, stores: Vec<Resource<KvStore>>, extra: Option<Resource<KvStore>>) -> u32 {
        self.calls.push(format!("count {} {}", stores.len(), extra.is_some()));
        stores
            .iter()
            .chain(extra.iter())
            .map(|s| self.table.get(s).unwrap().0.len() as u32)
            .sum()
    }
    fn dump(&mut self, v: View) -> Vec<Entry> {
        self.calls.push(format!("dump {}", v.prefix));
        let mut entries = self
            .table
            .get(&v.s)
            .unwrap()
            .0
            .iter()
            .filter(|(k, _)| k.starts_with(&v.prefix))
            .map(|(k, v)| Entry { key: k.clone(), value: v.clone() })
            .collect::<Vec<_>>();
        entries.sort_by(|a, b| a.key.cmp(&b.key));
        entries
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
    // A borrow inside a record in; a record with owned host stores out.
    let view = View { s: borrow(), prefix: "vis".into() };
    let report = app.example_rec_app().call_summarize(&mut store, &view)?;
    let kept = |r: &Resource<KvStore>, store: &Store<State>| {
        let mut kv = store.data().table.get(r).unwrap().0.iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>();
        kv.sort();
        kv.join(",")
    };
    // A borrow in an option, lent by the donut; the app passes it on, with
    // another, as a list and an option of borrows.
    let tally = app.example_rec_app().call_tally(&mut store, borrow(), Some(borrow()))?;
    results.push(format!("tally={tally}"));
    results.push(format!(
        "report lines={:?} first={} second={}",
        report.lines,
        kept(&report.kept.first, &store),
        kept(&report.kept.second, &store),
    ));

    Ok(Outcome {
        results,
        host_calls: store.data().calls.clone(),
        stdout: String::from_utf8_lossy(&stdout.contents()).into_owned(),
    })
}

/// The recorder: transparent (same results and host calls as the bare app),
/// and its `Hooks` see every call and drop, in both directions.
#[test]
fn recorder_is_transparent_and_sees_every_call() -> anyhow::Result<()> {
    let dir = build("forward")?;
    let w = |e: wasmtime::Error| anyhow::anyhow!("{e:?}");
    let bare = drive(&dir.join("out/app.wasm")).map_err(w)?;
    let wrapped = drive(&dir.join("out/recorder.wasm")).map_err(w)?;
    assert_eq!(wrapped.results, bare.results);
    assert_eq!(wrapped.host_calls, bare.host_calls);
    // Every call and drop, in order (`BLESS=1` to update).
    let expected = dir.join("recorder.expected");
    if std::env::var_os("BLESS").is_some() {
        std::fs::write(&expected, &wrapped.stdout)?;
    }
    assert_eq!(
        wrapped.stdout.lines().collect::<Vec<_>>(),
        std::fs::read_to_string(&expected)?.lines().collect::<Vec<_>>(),
    );
    Ok(())
}

/// Overriding generated hooks: `kv_store_get` hides `visits` from the app
/// (without calling the host), and `app_session_greet` decorates the guest's
/// answer. Everything else still forwards, and observation sees the
/// overridden calls.
#[test]
fn overridden_hooks_change_behaviour() -> anyhow::Result<()> {
    let dir = build("forward")?;
    let w = |e: wasmtime::Error| anyhow::anyhow!("{e:?}");
    let bare = drive(&dir.join("out/app.wasm")).map_err(w)?;
    let wrapped = drive(&dir.join("out/interceptor.wasm")).map_err(w)?;
    // The same as the bare app, except where the two overrides apply.
    assert_eq!(
        wrapped.results,
        [
            bare.results[0].replacen("visits=Some(\"1\")", "visits=None", 1),
            "hello ann (visits=None) [intercepted]".to_string(),
            bare.results[2].clone(),
            bare.results[3].clone(),
        ]
    );
    // The host never saw the hidden `get`s; it saw everything else.
    let expected_host: Vec<_> = bare.host_calls.iter().filter(|c| *c != "get visits").cloned().collect();
    assert_eq!(wrapped.host_calls, expected_host);
    assert!(wrapped.stdout.contains("Import [method]store.get -> None"), "{}", wrapped.stdout);
    assert!(
        wrapped.stdout.contains("Export [method]session.greet -> \"hello ann (visits=None) [intercepted]\""),
        "{}",
        wrapped.stdout
    );
    Ok(())
}
