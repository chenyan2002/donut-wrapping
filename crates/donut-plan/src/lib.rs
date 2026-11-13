//! Inverts a guest world into the bindings and manifest of a *donut*: a
//! component that virtualizes some of a guest component's imports, embeds the
//! guest, and calls the guest's exports. See `docs/design.md` §4.
//!
//! Given the donut's own world `W`, the guest's world `G`, and the set `V` of
//! guest imports to virtualize, this produces:
//!
//! * **World A**: `W`, plus every guest import in `V` as an export, plus every
//!   other guest import (a *passthrough*) as an import. Ordinary wit-bindgen
//!   output, with a `component-type` section.
//! * **World B**: the guest's exports, as imports. Generated with
//!   `import_module_prefix: "[guest-export]"` and wrapped in `mod guest`. Its
//!   dependency interfaces don't get functions; their resources are remapped
//!   to World A's types (export side for virtualized interfaces, import side
//!   for passthrough ones).
//! * The `wit-bindgen-donut:v0` manifest, which tells wit-component's donut
//!   mode which guest imports are virtualized.

use anyhow::{Context, Result, bail};
use heck::ToUpperCamelCase;
use std::collections::{BTreeSet, HashSet};
use std::fmt::Write;
use wit_bindgen_core::{Files, WorldGenerator, name_package_module};
use wit_bindgen_rust::{Opts, WithOption, to_rust_ident};
use wit_component::donut::{GUEST_EXPORT_MODULE_PREFIX, MANIFEST_SECTION_NAME, encode_manifest};
use wit_parser::{
    FunctionKind, InterfaceId, LiveTypes, PackageId, Resolve, TypeDefKind, TypeOwner, WorldId,
    WorldItem, WorldKey,
};

/// What to generate.
#[derive(Clone, Debug, Default)]
pub struct Config {
    /// The donut's own (outer) world, e.g. `example:http/socket-server`.
    pub world: String,
    /// The guest's world, e.g. `example:http/http-guest`.
    pub guest: String,
    /// Guest imports the donut implements, e.g. `example:http/types`. Every
    /// other guest import is passed through from the donut's imports.
    pub virtualize: Vec<String>,
    /// wit-bindgen options for World A. `generate_all` is always set, since
    /// passthrough imports need bindings.
    pub opts: Opts,
}

/// The inverted worlds, before generation.
#[derive(Debug)]
pub struct Plan {
    /// The `Resolve` with World A and World B added.
    pub resolve: Resolve,
    /// The guest's world.
    pub guest: WorldId,
    /// World A: the donut's world.
    pub world_a: WorldId,
    /// World B: the guest's exports, as imports.
    pub world_b: WorldId,
    /// Virtualized guest imports (exported by World A).
    pub virtualized: Vec<InterfaceId>,
    /// Passthrough guest imports (imported by World A).
    pub passthrough: Vec<InterfaceId>,
    /// The guest's exported interfaces (imported by World B).
    pub guest_exports: Vec<InterfaceId>,
}

/// Generated Rust source plus the manifest it embeds.
pub struct Output {
    pub source: String,
    pub manifest: Vec<u8>,
}

impl Plan {
    /// Classifies the guest's imports, checks dependencies, and builds World A
    /// and World B in a copy of `resolve`.
    pub fn new(resolve: &Resolve, packages: &[PackageId], config: &Config) -> Result<Plan> {
        let mut resolve = resolve.clone();
        let outer = resolve
            .select_world(packages, Some(&config.world))
            .with_context(|| format!("selecting the donut world `{}`", config.world))?;
        let guest = resolve
            .select_world(packages, Some(&config.guest))
            .with_context(|| format!("selecting the guest world `{}`", config.guest))?;
        let g = &resolve.worlds[guest];

        reject_unsupported(&resolve, guest)?;

        // 1. Classify the guest's imports.
        let wanted = config.virtualize.iter().collect::<HashSet<_>>();
        let mut virtualized = Vec::new();
        let mut passthrough = Vec::new();
        for (key, item) in g.imports.iter() {
            let name = resolve.name_world_key(key);
            match item {
                WorldItem::Interface { id, .. } => {
                    if let WorldKey::Name(_) = key {
                        bail!("guest import `{name}` is an inline interface, which isn't supported yet");
                    }
                    if wanted.contains(&name) {
                        virtualized.push(*id);
                    } else {
                        passthrough.push(*id);
                    }
                }
                WorldItem::Function(_) => {
                    bail!("guest import `{name}` is a world-level function, which isn't supported yet")
                }
                WorldItem::Type { .. } => {}
            }
        }
        for name in config.virtualize.iter() {
            if !virtualized.iter().any(|id| resolve.id_of(*id).as_ref() == Some(name)) {
                bail!("`{name}` in `virtualize` is not an interface imported by `{}`", config.guest);
            }
        }

        // 2. A passthrough interface can't mention a virtualized resource:
        // the host's `f(request)` can't take the donut's virtual `request`.
        let virtual_set = virtualized.iter().copied().collect::<HashSet<_>>();
        for p in passthrough.iter() {
            let mut live = LiveTypes::default();
            live.add_interface(&resolve, *p);
            for ty in live.iter() {
                let def = &resolve.types[ty];
                let TypeOwner::Interface(owner) = def.owner else { continue };
                if matches!(def.kind, TypeDefKind::Resource) && virtual_set.contains(&owner) {
                    bail!(
                        "guest import `{p}` uses resource `{r}` of virtualized interface `{o}`, \
                         so it must be virtualized too: add `{p}` to `virtualize`",
                        p = resolve.id_of(*p).unwrap(),
                        r = def.name.as_deref().unwrap_or("?"),
                        o = resolve.id_of(owner).unwrap(),
                    );
                }
            }
        }

        let mut guest_exports = Vec::new();
        for (key, item) in g.exports.iter() {
            let name = resolve.name_world_key(key);
            match (key, item) {
                (WorldKey::Interface(_), WorldItem::Interface { id, .. }) => guest_exports.push(*id),
                (WorldKey::Name(_), WorldItem::Interface { .. }) => {
                    bail!("guest export `{name}` is an inline interface, which isn't supported yet")
                }
                _ => bail!("guest export `{name}` is a world-level item, which isn't supported yet"),
            }
        }

        // 3 + 4. World A and World B, written as WIT so that wit-parser
        // elaborates their dependencies.
        let w = &resolve.worlds[outer];
        let mut wit = String::from("package donut:generated;\n\n");
        writeln!(wit, "world donut-a {{")?;
        writeln!(wit, "  include {};", resolve.id_of_name(w.package.unwrap(), &w.name))?;
        for id in passthrough.iter() {
            if !w.imports.contains_key(&WorldKey::Interface(*id)) {
                writeln!(wit, "  import {};", resolve.id_of(*id).unwrap())?;
            }
        }
        for id in virtualized.iter() {
            if !w.exports.contains_key(&WorldKey::Interface(*id)) {
                writeln!(wit, "  export {};", resolve.id_of(*id).unwrap())?;
            }
        }
        writeln!(wit, "}}\n\nworld donut-b {{")?;
        for id in guest_exports.iter() {
            writeln!(wit, "  import {};", resolve.id_of(*id).unwrap())?;
        }
        writeln!(wit, "}}")?;
        let pkg = resolve
            .push_source("<donut-generated>.wit", &wit)
            .with_context(|| format!("building the donut worlds:\n{wit}"))?;
        let world_a = resolve.select_world(&[pkg], Some("donut-a"))?;
        let world_b = resolve.select_world(&[pkg], Some("donut-b"))?;

        Ok(Plan {
            resolve,
            guest,
            world_a,
            world_b,
            virtualized,
            passthrough,
            guest_exports,
        })
    }

    /// Generates the Rust source for World A, `mod guest` (World B), and the
    /// manifest static.
    pub fn generate(&self, opts: &Opts) -> Result<Output> {
        let world_a = self.generate_world_a(opts)?;
        let world_b = self.generate_world_b(opts)?;
        let names = self
            .virtualized
            .iter()
            .map(|id| self.resolve.id_of(*id).unwrap())
            .collect::<Vec<_>>();
        let manifest = encode_manifest(&self.resolve, self.guest, &names)?;

        let mut source = world_a;
        writeln!(
            source,
            "\n/// Calls into the embedded guest component (its exports).\n\
             #[allow(dead_code, unused_imports, clippy::all)]\n\
             pub mod guest {{\n{world_b}\n}}\n"
        )?;
        writeln!(
            source,
            "#[cfg(target_arch = \"wasm32\")]\n\
             #[unsafe(link_section = \"{MANIFEST_SECTION_NAME}\")]\n\
             #[used]\n\
             #[doc(hidden)]\n\
             static __DONUT_MANIFEST: [u8; {len}] = {bytes:?};",
            len = manifest.len(),
            bytes = manifest,
        )?;
        Ok(Output { source, manifest })
    }

    fn generate_world_a(&self, opts: &Opts) -> Result<String> {
        let mut opts = opts.clone();
        opts.generate_all = true;
        run(opts, &self.resolve, self.world_a).context("generating bindings for the donut's world")
    }

    fn generate_world_b(&self, opts: &Opts) -> Result<String> {
        let mut resolve = self.resolve.clone();
        let guest_exports = self.guest_exports.iter().copied().collect::<HashSet<_>>();
        let virtualized = self.virtualized.iter().copied().collect::<HashSet<_>>();
        let mut with = Vec::new();
        let deps = resolve.worlds[self.world_b]
            .imports
            .iter()
            .filter_map(|(key, item)| match item {
                WorldItem::Interface { id, .. } if !guest_exports.contains(id) => {
                    Some((key.clone(), *id))
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        for (key, id) in deps {
            if virtualized.contains(&id) {
                // A virtualized dependency: keep its non-resource types (they're
                // regenerated as outbound views), drop its functions (they'd
                // collide with World A's export-side resource methods), and map
                // its resources to World A's export-side types.
                resolve.interfaces[id].functions.clear();
                let module = module_path(&resolve, id, true);
                for (name, ty) in resolve.interfaces[id].types.iter() {
                    if matches!(resolve.types[*ty].kind, TypeDefKind::Resource) {
                        with.push((
                            format!("{}/{name}", resolve.id_of(id).unwrap()),
                            WithOption::Path(format!("crate::{module}::{}", camel(name))),
                        ));
                    }
                }
            } else {
                // A host interface: reuse World A's import-side bindings.
                with.push((
                    resolve.name_world_key(&key),
                    WithOption::Path(format!("crate::{}", module_path(&resolve, id, false))),
                ));
            }
        }

        let opts = Opts {
            generate_all: true,
            import_module_prefix: Some(GUEST_EXPORT_MODULE_PREFIX.to_string()),
            with,
            runtime_path: opts.runtime_path.clone(),
            bitflags_path: opts.bitflags_path.clone(),
            std_feature: opts.std_feature,
            ownership: opts.ownership.clone(),
            format: opts.format,
            ..Opts::default()
        };
        run(opts, &resolve, self.world_b).context("generating bindings for the guest's exports")
    }
}

/// Plans and generates in one step.
pub fn generate(resolve: &Resolve, packages: &[PackageId], config: &Config) -> Result<Output> {
    Plan::new(resolve, packages, config)?.generate(&config.opts)
}

fn run(opts: Opts, resolve: &Resolve, world: WorldId) -> Result<String> {
    let mut resolve = resolve.clone();
    let mut files = Files::default();
    opts.build().generate(&mut resolve, world, &mut files)?;
    let mut out = String::new();
    for (_, bytes) in files.iter() {
        out.push_str(std::str::from_utf8(bytes)?);
    }
    Ok(out)
}

/// The module path wit-bindgen gives an interface (see its
/// `compute_module_path`), relative to where the bindings are generated.
fn module_path(resolve: &Resolve, id: InterfaceId, is_export: bool) -> String {
    let iface = &resolve.interfaces[id];
    let pkg = iface.package.unwrap();
    let mut path = Vec::new();
    if is_export {
        path.push("exports".to_string());
    }
    path.push(to_rust_ident(&resolve.packages[pkg].name.namespace));
    path.push(name_package_module(resolve, pkg));
    path.push(to_rust_ident(iface.name.as_ref().unwrap()));
    path.join("::")
}

/// wit-bindgen's name for a type (`guest` is reserved for traits).
fn camel(name: &str) -> String {
    match name {
        "guest" => "Guest_".to_string(),
        s => s.to_upper_camel_case(),
    }
}

/// p2 only: no async functions, streams, futures, or error contexts.
fn reject_unsupported(resolve: &Resolve, world: WorldId) -> Result<()> {
    let w = &resolve.worlds[world];
    let mut seen = BTreeSet::new();
    for (key, item) in w.imports.iter().chain(w.exports.iter()) {
        let funcs = match item {
            WorldItem::Interface { id, .. } => {
                resolve.interfaces[*id].functions.values().collect::<Vec<_>>()
            }
            WorldItem::Function(f) => vec![f],
            WorldItem::Type { .. } => continue,
        };
        for f in funcs {
            let is_async = matches!(
                f.kind,
                FunctionKind::AsyncFreestanding
                    | FunctionKind::AsyncMethod(_)
                    | FunctionKind::AsyncStatic(_)
            );
            if is_async || !f.find_futures_and_streams(resolve).is_empty() {
                seen.insert(format!("{}: {}", resolve.name_world_key(key), f.name));
            }
        }
    }
    if !seen.is_empty() {
        bail!(
            "the guest world uses async functions, streams, or futures, which donuts don't \
             support yet (component model p2 only):\n  {}",
            seen.into_iter().collect::<Vec<_>>().join("\n  ")
        );
    }
    Ok(())
}

pub use wit_bindgen_rust;
pub use wit_parser;
