//! Helpers for the runtime tests in `tests/`: build an example (once per test
//! process) and run its donut component on Wasmtime.
//!
//! The examples are built by their `build.sh` scripts, into
//! `target/donut-examples` so they don't contend with `cargo test`'s own build.
//! `WASM_TOOLS` must point at a wasm-tools with wit-component's donut mode
//! (defaults to the local `~/src/bytecodealliance/wasm-tools` debug build).

use anyhow::{Result, anyhow, bail};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;
use wasmtime::component::{Component, ComponentNamedList, Lift, Linker, Lower, ResourceTable};
use wasmtime::{Engine, Store};
use wasmtime_wasi::p2::pipe::MemoryOutputPipe;
use wasmtime_wasi::{WasiCtx, WasiCtxView, WasiView};

pub fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap()
}

/// Runs `examples/<name>/build.sh` once per process and returns the example's
/// directory.
pub fn build(name: &str) -> Result<PathBuf> {
    static BUILT: Mutex<Option<HashMap<String, Result<(), String>>>> = Mutex::new(None);
    let dir = repo().join("examples").join(name);
    let mut built = BUILT.lock().unwrap();
    let built = built.get_or_insert_with(HashMap::new);
    let result = built.entry(name.to_string()).or_insert_with(|| {
        let out = Command::new("sh")
            .arg(dir.join("build.sh"))
            .env("CARGO_TARGET_DIR", repo().join("target/donut-examples"))
            .env_remove("CARGO_ENCODED_RUSTFLAGS")
            .env_remove("RUSTFLAGS")
            .output()
            .map_err(|e| e.to_string())?;
        if out.status.success() {
            Ok(())
        } else {
            Err(format!(
                "examples/{name}/build.sh failed:\n{}{}",
                String::from_utf8_lossy(&out.stdout),
                String::from_utf8_lossy(&out.stderr)
            ))
        }
    });
    match result {
        Ok(()) => Ok(dir),
        Err(e) => bail!("{e}"),
    }
}

/// What a run produced besides its return value.
pub struct Run<R> {
    pub result: R,
    pub stdout: String,
    pub stderr: String,
}

struct State {
    ctx: WasiCtx,
    table: ResourceTable,
}

impl WasiView for State {
    fn ctx(&mut self) -> WasiCtxView<'_> {
        WasiCtxView {
            ctx: &mut self.ctx,
            table: &mut self.table,
        }
    }
}

/// Instantiates `component` with WASI (p2) in a fresh store and calls its
/// export `func`.
pub fn call<P, R>(component: &Path, func: &str, params: P) -> Result<Run<R>>
where
    P: ComponentNamedList + Lower,
    R: ComponentNamedList + Lift,
{
    let engine = Engine::default();
    // Keep wasmtime's full error text (it carries the wasm backtrace).
    let w = |e: wasmtime::Error| anyhow!("{e:?}");
    let component = Component::from_file(&engine, component)
        .map_err(|e| anyhow!("loading {}: {e:?}", component.display()))?;
    let mut linker = Linker::<State>::new(&engine);
    wasmtime_wasi::p2::add_to_linker_sync(&mut linker).map_err(w)?;
    let stdout = MemoryOutputPipe::new(1 << 20);
    let stderr = MemoryOutputPipe::new(1 << 20);
    let ctx = WasiCtx::builder()
        .stdout(stdout.clone())
        .stderr(stderr.clone())
        .build();
    let mut store = Store::new(&engine, State { ctx, table: ResourceTable::new() });
    let instance = linker.instantiate(&mut store, &component).map_err(w)?;
    let f = instance.get_typed_func::<P, R>(&mut store, func).map_err(w)?;
    let result = f.call(&mut store, params).map_err(w)?;
    Ok(Run {
        result,
        stdout: String::from_utf8_lossy(&stdout.contents()).into_owned(),
        stderr: String::from_utf8_lossy(&stderr.contents()).into_owned(),
    })
}
