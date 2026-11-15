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
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write;
use wit_bindgen_core::{Files, WorldGenerator, name_package_module};
use wit_bindgen_rust::{Opts, WithOption, to_rust_ident};
use wit_component::donut::{GUEST_EXPORT_MODULE_PREFIX, MANIFEST_SECTION_NAME, encode_manifest};
mod forward;

use wit_parser::{
    FunctionKind, Handle, Type, TypeId, InterfaceId, LiveTypes, PackageId, Resolve, TypeDefKind, TypeOwner, WorldId,
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
    /// Interfaces to forward (design §6). A guest import is virtualized and
    /// forwarded to the host's import of it. An export of both the donut's
    /// world and the guest is forwarded to the guest's. Requires `hooks`.
    pub forward: Vec<String>,
    /// The Rust type (at the crate root) that implements `forward::Hooks` and
    /// receives the generated forwarding impls.
    pub hooks: Option<String>,
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
    /// Virtualized guest imports.
    pub virtualized: Vec<InterfaceId>,
    /// For each of `virtualized`, the interface World A exports to implement
    /// it: the same interface, or a renamed copy (`virtual-<namespace>:…`) when
    /// the donut's own world also imports or exports it.
    pub virtual_exports: Vec<InterfaceId>,
    /// Passthrough guest imports (imported by World A).
    pub passthrough: Vec<InterfaceId>,
    /// The guest's exported interfaces (imported by World B).
    pub guest_exports: Vec<InterfaceId>,
    /// Forwarded virtual interfaces: (World A's virtual copy, the host's).
    pub forward_virtual: Vec<(InterfaceId, InterfaceId)>,
    /// Forwarded exports (of both World A and the guest).
    pub forward_exports: Vec<InterfaceId>,
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
        let guest_imports = g
            .imports
            .keys()
            .map(|k| resolve.name_world_key(k))
            .collect::<BTreeSet<_>>();
        let forwarded = config.forward.iter().collect::<BTreeSet<_>>();
        let wanted = config
            .virtualize
            .iter()
            .chain(config.forward.iter().filter(|f| guest_imports.contains(*f)))
            .collect::<BTreeSet<_>>();
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
        let virtual_set = virtualized.iter().copied().collect::<BTreeSet<_>>();
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

        // 2b. If the donut's own world also imports or exports a virtualized
        // interface (e.g. a recording donut whose world is the guest's), World
        // A can't export that interface for the guest as well: the donut's own
        // exports would then bind to the virtual copy rather than the host's,
        // since wit-bindgen and wit-component bind an export's dependencies to
        // exports. So export a renamed copy of its package instead. That's the
        // same reason proxy-component uses `wrapped-` names.
        let w = &resolve.worlds[outer];
        let mut virtual_exports = virtualized.clone();
        // Forwarded virtual interfaces always collide: World A imports the
        // host's to forward to it.
        let conflicting = virtualized
            .iter()
            .filter(|id| {
                let key = WorldKey::Interface(**id);
                w.imports.contains_key(&key)
                    || w.exports.contains_key(&key)
                    || forwarded.contains(&resolve.id_of(**id).unwrap())
            })
            .map(|id| resolve.interfaces[*id].package.unwrap())
            .collect::<BTreeSet<_>>();
        for pkg in conflicting {
            let mut copy = resolve.clone();
            let old = copy.packages[pkg].name.clone();
            let mut name = old.clone();
            name.namespace = format!("virtual-{}", name.namespace);
            copy.package_names.shift_remove(&old);
            copy.packages[pkg].name = name.clone();
            copy.package_names.insert(name.clone(), pkg);
            resolve
                .merge(copy)
                .with_context(|| format!("copying `{old}` as `{name}`"))?;
            let new_pkg = resolve.package_names[&name];
            for (i, id) in virtualized.iter().enumerate() {
                if resolve.interfaces[*id].package != Some(pkg) {
                    continue;
                }
                // The copy's same-package dependencies are copies too, so they
                // must be virtualized as well.
                for dep in resolve.interface_direct_deps(*id) {
                    if resolve.interfaces[dep].package == Some(pkg) && !virtual_set.contains(&dep) {
                        bail!(
                            "virtualized `{}` uses `{}` from the same package, which must \
                             then be virtualized too (the donut's own world uses `{}`, so \
                             the guest's copy is a renamed package)",
                            resolve.id_of(*id).unwrap(),
                            resolve.id_of(dep).unwrap(),
                            resolve.id_of(*id).unwrap(),
                        );
                    }
                }
                let iface = resolve.interfaces[*id].name.clone().unwrap();
                virtual_exports[i] = resolve.packages[new_pkg].interfaces[&iface];
            }
        }

        let g = &resolve.worlds[guest];
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
        // Forwarded virtual interfaces forward to the host's.
        let mut forward_virtual = Vec::new();
        for (id, copy) in virtualized.iter().zip(virtual_exports.iter()) {
            if forwarded.contains(&resolve.id_of(*id).unwrap()) {
                if !w.imports.contains_key(&WorldKey::Interface(*id)) {
                    writeln!(wit, "  import {};", resolve.id_of(*id).unwrap())?;
                }
                forward_virtual.push((*copy, *id));
            }
        }
        // Forwarded exports: exported by both the donut's world and the guest.
        let mut forward_exports = Vec::new();
        for name in config.forward.iter() {
            if guest_imports.contains(name) {
                continue;
            }
            let id = guest_exports
                .iter()
                .find(|id| resolve.id_of(**id).as_ref() == Some(name))
                .filter(|id| w.exports.contains_key(&WorldKey::Interface(**id)));
            match id {
                Some(id) => forward_exports.push(*id),
                None => bail!(
                    "`{name}` in `forward` is neither a guest import nor an interface \
                     exported by both `{}` and `{}`",
                    config.world,
                    config.guest
                ),
            }
        }
        if !config.forward.is_empty() && config.hooks.is_none() {
            bail!("`forward` needs `hooks`: the type that implements `forward::Hooks`");
        }
        for id in virtual_exports.iter() {
            writeln!(wit, "  export {};", resolve.id_of(*id).unwrap())?;
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
            virtual_exports,
            passthrough,
            guest_exports,
            forward_virtual,
            forward_exports,
        })
    }

    /// Generates the Rust source for World A, `mod guest` (World B), and the
    /// manifest static.
    pub fn generate(&self, opts: &Opts) -> Result<Output> {
        self.generate_with(opts, None)
    }

    /// Like `generate`, plus forwarding impls for `hooks` if anything is
    /// forwarded.
    pub fn generate_with(&self, opts: &Opts, hooks: Option<&str>) -> Result<Output> {
        let forwarding = !self.forward_virtual.is_empty() || !self.forward_exports.is_empty();
        let mut opts = opts.clone();
        if forwarding {
            // Host-side and virtual copies of resource-free types become the
            // same Rust type, so forwarding passes them through unchanged.
            opts.merge_structurally_equal_types = Some(Some(true));
        }
        let opts = &opts;
        let world_a = self.generate_world_a(opts)?;
        let world_b = self.generate_world_b(opts)?;
        let names = self
            .virtualized
            .iter()
            .zip(self.virtual_exports.iter())
            .map(|(id, export)| {
                (self.resolve.id_of(*id).unwrap(), self.resolve.id_of(*export).unwrap())
            })
            .collect::<Vec<_>>();
        let manifest = encode_manifest(&self.resolve, self.guest, &names)?;

        let mut source = world_a;
        if forwarding {
            let hooks = hooks.ok_or_else(|| anyhow::anyhow!("`forward` needs `hooks`"))?;
            source.push_str(&forward::generate(self, hooks).context("generating forwarding")?);
        }
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
        let guest_exports = self.guest_exports.iter().copied().collect::<BTreeSet<_>>();
        let virtual_export = self
            .virtualized
            .iter()
            .copied()
            .zip(self.virtual_exports.iter().copied())
            .collect::<BTreeMap<_, _>>();
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
        // Types World B can share with World A (see `world_b_shared_types`).
        for (id, path) in self.world_b_shared_types() {
            let def = &resolve.types[id];
            let TypeOwner::Interface(owner) = def.owner else { continue };
            with.push((
                format!("{}/{}", resolve.id_of(owner).unwrap(), def.name.as_ref().unwrap()),
                WithOption::Path(path),
            ));
        }
        for (key, id) in deps {
            if let Some(export) = virtual_export.get(&id) {
                // A virtualized dependency: keep its non-resource types (they're
                // regenerated as outbound views), drop its functions (they'd
                // collide with World A's export-side resource methods), and map
                // its resources to World A's export-side types.
                resolve.interfaces[id].functions.clear();
                let module = module_path(&resolve, *export, true);
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

impl Plan {
    /// The types World B reuses from World A instead of generating its own,
    /// with their World-A paths. A type is shared when its Rust rendering is
    /// the same on World A's export side and World B's import side:
    ///
    /// * in a virtualized dependency, unless it contains a borrow of a
    ///   virtualized resource (World A renders that as an inbound `RBorrow`,
    ///   World B as an outbound `&R`); owned virtual resources are remapped to
    ///   World A's types anyway;
    /// * in a forwarded export (exported by both World A and the guest),
    ///   unless it contains any handle, since World A's handles there are the
    ///   host's and World B's are the guest's or virtual.
    ///
    /// Only types defined in the interface itself (not `use`d aliases) and
    /// not resources.
    pub fn world_b_shared_types(&self) -> BTreeMap<TypeId, String> {
        let resolve = &self.resolve;
        let virtualized = self.virtualized.iter().copied().collect::<BTreeSet<_>>();
        let virtual_resource = |r: TypeId| {
            let r = dealias(resolve, r);
            matches!(resolve.types[r].owner, TypeOwner::Interface(i) if virtualized.contains(&i))
        };
        let mut out = BTreeMap::new();
        let mut share = |iface: InterfaceId, target: InterfaceId, ok: &dyn Fn(&Type) -> bool| {
            for (name, ty) in resolve.interfaces[iface].types.iter() {
                let def = &resolve.types[*ty];
                if matches!(def.kind, TypeDefKind::Resource | TypeDefKind::Type(Type::Id(_))) {
                    continue;
                }
                if ok(&Type::Id(*ty)) {
                    out.insert(
                        *ty,
                        format!("crate::{}::{}", module_path(resolve, target, true), camel(name)),
                    );
                }
            }
        };
        // Only interfaces World B imports: wit-bindgen rejects unused remaps.
        let in_world_b = resolve.worlds[self.world_b]
            .imports
            .values()
            .filter_map(|item| match item {
                WorldItem::Interface { id, .. } => Some(*id),
                _ => None,
            })
            .collect::<BTreeSet<_>>();
        for (id, export) in self.virtualized.iter().zip(self.virtual_exports.iter()) {
            if !in_world_b.contains(id) {
                continue;
            }
            share(*id, *export, &|t| {
                !type_has(resolve, t, &|h| matches!(h, Handle::Borrow(r) if virtual_resource(*r)))
            });
        }
        for e in self.forward_exports.iter() {
            share(*e, *e, &|t| !type_has(resolve, t, &|_| true));
        }
        out
    }
}

fn dealias(resolve: &Resolve, mut id: TypeId) -> TypeId {
    while let TypeDefKind::Type(Type::Id(next)) = resolve.types[id].kind {
        id = next;
    }
    id
}

/// Whether `ty` contains a handle matching `f`.
pub(crate) fn type_has(resolve: &Resolve, ty: &Type, f: &dyn Fn(&Handle) -> bool) -> bool {
    let Type::Id(id) = ty else { return false };
    match &resolve.types[*id].kind {
        TypeDefKind::Handle(h) => f(h),
        TypeDefKind::Record(r) => r.fields.iter().any(|x| type_has(resolve, &x.ty, f)),
        TypeDefKind::Variant(v) => {
            v.cases.iter().any(|c| c.ty.as_ref().is_some_and(|t| type_has(resolve, t, f)))
        }
        TypeDefKind::Tuple(t) => t.types.iter().any(|t| type_has(resolve, t, f)),
        TypeDefKind::Option(t) | TypeDefKind::List(t) | TypeDefKind::Type(t) => type_has(resolve, t, f),
        TypeDefKind::Result(r) => {
            r.ok.as_ref().is_some_and(|t| type_has(resolve, t, f))
                || r.err.as_ref().is_some_and(|t| type_has(resolve, t, f))
        }
        _ => false,
    }
}

/// Plans and generates in one step.
pub fn generate(resolve: &Resolve, packages: &[PackageId], config: &Config) -> Result<Output> {
    Plan::new(resolve, packages, config)?.generate_with(&config.opts, config.hooks.as_deref())
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
