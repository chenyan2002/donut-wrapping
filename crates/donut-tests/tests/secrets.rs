//! `examples/secrets`, the README's example: the app, wrapped for a host with
//! a vault and no filesystem, run on a Wasmtime host for `example:app/host`.

use donut_tests::build;
use std::collections::BTreeMap;
use wasmtime::component::{Component, Linker, Resource, ResourceTable};
use wasmtime::{Engine, Store};
use wasmtime_wasi::p2::pipe::MemoryOutputPipe;
use wasmtime_wasi::{WasiCtx, WasiCtxView, WasiView};

// The world `host` becomes `Host_`, since `Host` is taken by the traits.
wasmtime::component::bindgen!({
    path: "../../examples/secrets/wit",
    world: "example:app/host",
    with: {
        "example:app/types.request": HostRequest,
        "example:app/secrets.vault": HostVault,
    },
});

pub struct HostRequest {
    url: String,
    headers: BTreeMap<String, String>,
}
pub struct HostVault;

#[derive(Default)]
struct Log {
    /// (url, authorization header) of every request the host sent.
    sent: Vec<(String, Option<String>)>,
    /// Every key looked up in the vault.
    lookups: Vec<String>,
    /// Live host requests and vaults.
    live: i32,
}

struct State {
    ctx: WasiCtx,
    table: ResourceTable,
    log: Log,
}

impl WasiView for State {
    fn ctx(&mut self) -> WasiCtxView<'_> {
        WasiCtxView { ctx: &mut self.ctx, table: &mut self.table }
    }
}

impl example::app::types::Host for State {}

impl example::app::types::HostRequest for State {
    fn new(&mut self, url: String) -> Resource<HostRequest> {
        self.log.live += 1;
        self.table.push(HostRequest { url, headers: BTreeMap::new() }).unwrap()
    }
    fn url(&mut self, r: Resource<HostRequest>) -> String {
        self.table.get(&r).unwrap().url.clone()
    }
    fn set_header(&mut self, r: Resource<HostRequest>, name: String, value: String) {
        self.table.get_mut(&r).unwrap().headers.insert(name, value);
    }
    fn drop(&mut self, r: Resource<HostRequest>) -> wasmtime::Result<()> {
        self.log.live -= 1;
        self.table.delete(r)?;
        Ok(())
    }
}

impl example::app::outgoing::Host for State {
    fn send(&mut self, r: Resource<HostRequest>) -> Result<u16, String> {
        self.log.live -= 1;
        let r = self.table.delete(r).unwrap();
        self.log.sent.push((r.url, r.headers.get("authorization").cloned()));
        Ok(200)
    }
}

impl example::app::secrets::Host for State {}

impl example::app::secrets::HostVault for State {
    fn open(&mut self, _name: String) -> Resource<HostVault> {
        self.log.live += 1;
        self.table.push(HostVault).unwrap()
    }
    fn get(&mut self, _v: Resource<HostVault>, key: String) -> Option<String> {
        self.log.lookups.push(key.clone());
        (key == "api.internal").then(|| "t0ken".to_string())
    }
    fn drop(&mut self, v: Resource<HostVault>) -> wasmtime::Result<()> {
        self.log.live -= 1;
        self.table.delete(v)?;
        Ok(())
    }
}

#[test]
fn app_gets_tokens_it_never_sees() -> anyhow::Result<()> {
    let out = build("secrets")?.join("out/donut.wasm");
    let w = |e: wasmtime::Error| anyhow::anyhow!("{e:?}");

    let engine = Engine::default();
    let component = Component::from_file(&engine, &out).map_err(w)?;
    let mut linker = Linker::<State>::new(&engine);
    wasmtime_wasi::p2::add_to_linker_sync(&mut linker).map_err(w)?;
    Host_::add_to_linker::<_, wasmtime::component::HasSelf<_>>(&mut linker, |s| s).map_err(w)?;
    let stdout = MemoryOutputPipe::new(1 << 20);
    let stderr = MemoryOutputPipe::new(1 << 20);
    let ctx = WasiCtx::builder().stdout(stdout.clone()).stderr(stderr.clone()).build();
    let mut store = Store::new(&engine, State { ctx, table: ResourceTable::new(), log: Log::default() });
    let host = Host_::instantiate(&mut store, &component, &linker).map_err(w)?;
    let result = host.example_app_handler().call_handle(&mut store, "/hello").map_err(w)?;

    // The settings file came from the mocked `fs`.
    assert_eq!(result, "status 200");
    let stdout = String::from_utf8_lossy(&stdout.contents()).into_owned();
    assert_eq!(
        stdout,
        "app: /hello -> https://api.internal\n\
         app: telemetry failed: telemetry.example: not allowed\n"
    );
    // Only the allowed request reached the host, with the vault's token.
    let log = &store.data().log;
    assert_eq!(log.sent, [("https://api.internal/hello".to_string(), Some("Bearer t0ken".to_string()))]);
    assert_eq!(log.lookups, ["telemetry.example", "api.internal"]);
    assert_eq!(log.live, 0, "the donut leaked host requests or vaults");
    // `before` saw the forwarded and mocked calls.
    let stderr = String::from_utf8_lossy(&stderr.contents()).into_owned();
    assert!(stderr.contains("donut: example:app/fs open[\"/etc/app.conf\"]"), "{stderr}");
    assert!(stderr.contains("donut: example:app/handler handle[\"/hello\"]"), "{stderr}");
    Ok(())
}
