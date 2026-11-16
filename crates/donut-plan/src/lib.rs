//! Inverts a guest world into the bindings and manifest of a *donut*: a
//! component that virtualizes some of a guest component's imports, embeds the
//! guest, and calls the guest's exports. See `docs/design.md` §4.
//!
//! Given the donut's own world `W`, the guest's world `G`, and the set `V` of
//! guest imports to virtualize, this produces:
//!
//! * **World A**: `W`, plus every guest import in `V` as an export (a copy, in
//!   a package renamed `virtual-<namespace>:<name>`), plus every other guest
//!   import (a *passthrough*) as an import. Ordinary wit-bindgen
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
    /// Guest imports to mock (design §6): virtualized, with generated trait
    /// impls whose `forward::Hooks` methods default to `unimplemented!()`.
    /// Override the ones the guest uses. Requires `hooks`.
    pub mock: Vec<String>,
    /// For mocked interfaces, the Rust type of each resource's state:
    /// interface → resource (its WIT name) → type, written as at the crate
    /// root. A mocked resource without a type can't be created.
    pub mock_types: BTreeMap<String, BTreeMap<String, String>>,
    /// The Rust type (at the crate root) that implements `forward::Hooks` and
    /// receives the generated forwarding and mock impls.
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
    /// Mocked virtual interfaces.
    pub mock_virtual: Vec<Mocked>,
}

/// A mocked virtual interface.
#[derive(Debug)]
pub struct Mocked {
    /// World A's export (the virtual copy).
    pub export: InterfaceId,
    /// The guest's name for it, e.g. `example:app/fs`.
    pub name: String,
    /// Resource (WIT name) → the Rust type of its state.
    pub state: BTreeMap<String, String>,
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
        for name in config.virtualize.iter() {
            if forwarded.contains(name) || config.mock.contains(name) {
                bail!(
                    "`{name}` is in `virtualize` and in `{}`: list it once (`virtualize` \
                     means you implement it, `forward` and `mock` generate the implementation)",
                    if forwarded.contains(name) { "forward" } else { "mock" }
                );
            }
        }
        for name in config.mock.iter() {
            if !guest_imports.contains(name) {
                bail!("`{name}` in `mock` is not an interface imported by `{}`", config.guest);
            }
            if forwarded.contains(name) {
                bail!("`{name}` can't be both forwarded and mocked");
            }
        }
        let wanted = config
            .virtualize
            .iter()
            .chain(config.forward.iter().filter(|f| guest_imports.contains(*f)))
            .chain(config.mock.iter())
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

        // 2a. Likewise, a forwarded import can't mention a resource that's
        // virtualized (or mocked) but not forwarded: there's no host resource
        // behind it to pass on.
        for f in virtualized.iter() {
            let fname = resolve.id_of(*f).unwrap();
            if !forwarded.contains(&fname) {
                continue;
            }
            let mut live = LiveTypes::default();
            live.add_interface(&resolve, *f);
            for ty in live.iter() {
                let def = &resolve.types[ty];
                let TypeOwner::Interface(owner) = def.owner else { continue };
                let oname = resolve.id_of(owner).unwrap();
                if matches!(def.kind, TypeDefKind::Resource)
                    && virtual_set.contains(&owner)
                    && !forwarded.contains(&oname)
                {
                    bail!(
                        "forwarded `{fname}` uses resource `{r}` of `{oname}`, which isn't \
                         forwarded, so there's no host `{r}` to pass on: forward `{oname}` too, \
                         or virtualize `{fname}` instead",
                        r = def.name.as_deref().unwrap_or("?"),
                    );
                }
            }
        }

        // 2b. World A exports, for the guest, a copy of each virtualized
        // interface in a package renamed `virtual-<namespace>:<name>`, so
        // that its name never collides with the interface itself, which the
        // donut may also import (to forward to the host) or export. Always
        // renaming keeps the guest side's Rust paths independent of what else
        // is listed. Only the listed interfaces are copied: their uses of
        // anything unlisted (including the rest of the package) still refer
        // to the originals, which World A imports.
        let copies = copy_interfaces(&mut resolve, &virtualized);
        let virtual_exports = virtualized.iter().map(|id| copies[id]).collect::<Vec<_>>();

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
        if (!config.forward.is_empty() || !config.mock.is_empty()) && config.hooks.is_none() {
            bail!("`forward` and `mock` need `hooks`: the type that implements `forward::Hooks`");
        }
        for name in config.mock_types.keys() {
            if !config.mock.contains(name) {
                bail!("`{name}` has mock state types but isn't in `mock`");
            }
        }
        let mut mock_virtual = Vec::new();
        for (id, export) in virtualized.iter().zip(virtual_exports.iter()) {
            let name = resolve.id_of(*id).unwrap();
            if !config.mock.contains(&name) {
                continue;
            }
            let state = config.mock_types.get(&name).cloned().unwrap_or_default();
            for res in state.keys() {
                let is_resource = resolve.interfaces[*id]
                    .types
                    .get(res)
                    .is_some_and(|t| matches!(resolve.types[*t].kind, TypeDefKind::Resource));
                if !is_resource {
                    bail!("`{name}` has no resource `{res}` to give a mock state type");
                }
            }
            mock_virtual.push(Mocked { export: *export, name, state });
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
            mock_virtual,
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
        if forwarding || !self.mock_virtual.is_empty() {
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
/// Copies `ifaces` into packages renamed `virtual-<namespace>:<name>`
/// (created as needed), returning original → copy. References between the
/// copied interfaces point to the copies. Everything else they use (types of
/// other interfaces, including unlisted ones of the same package) keeps
/// pointing to the original.
fn copy_interfaces(resolve: &mut Resolve, ifaces: &[InterfaceId]) -> BTreeMap<InterfaceId, InterfaceId> {
    let mut map = BTreeMap::new();
    // Empty copies first, so that types can name their new owners.
    for id in ifaces.iter() {
        let pkg = resolve.interfaces[*id].package.unwrap();
        let mut name = resolve.packages[pkg].name.clone();
        name.namespace = format!("virtual-{}", name.namespace);
        let new_pkg = match resolve.package_names.get(&name) {
            Some(p) => *p,
            None => {
                let mut package = resolve.packages[pkg].clone();
                package.name = name.clone();
                package.interfaces.clear();
                package.worlds.clear();
                let p = resolve.packages.alloc(package);
                resolve.package_names.insert(name, p);
                p
            }
        };
        let mut copy = resolve.interfaces[*id].clone();
        copy.types.clear();
        copy.functions.clear();
        copy.package = Some(new_pkg);
        let iface_name = copy.name.clone().unwrap();
        let new_id = resolve.interfaces.alloc(copy);
        resolve.packages[new_pkg].interfaces.insert(iface_name, new_id);
        map.insert(*id, new_id);
    }
    let mut copier = Copier { resolve, ifaces: &map, types: BTreeMap::new() };
    for (old, new) in map.iter() {
        let types = copier.resolve.interfaces[*old].types.clone();
        for (name, ty) in types {
            let ty = copier.type_id(ty);
            copier.resolve.interfaces[*new].types.insert(name, ty);
        }
        let funcs = copier.resolve.interfaces[*old].functions.clone();
        for (name, mut func) in funcs {
            if let Some(r) = func.kind.resource_mut() {
                *r = copier.type_id(*r);
            }
            for p in func.params.iter_mut() {
                copier.ty(&mut p.ty);
            }
            if let Some(t) = &mut func.result {
                copier.ty(t);
            }
            copier.resolve.interfaces[*new].functions.insert(name, func);
        }
    }
    map
}

struct Copier<'a> {
    resolve: &'a mut Resolve,
    ifaces: &'a BTreeMap<InterfaceId, InterfaceId>,
    types: BTreeMap<TypeId, TypeId>,
}

impl Copier<'_> {
    /// The copy of `id`: types owned by a copied interface, and anonymous
    /// types (which may contain those), are copied; others are kept.
    fn type_id(&mut self, id: TypeId) -> TypeId {
        if let Some(new) = self.types.get(&id) {
            return *new;
        }
        let mut def = self.resolve.types[id].clone();
        match def.owner {
            TypeOwner::Interface(i) => match self.ifaces.get(&i) {
                Some(new) => def.owner = TypeOwner::Interface(*new),
                None => return id,
            },
            TypeOwner::None => {}
            TypeOwner::World(_) => return id,
        }
        match &mut def.kind {
            TypeDefKind::Type(t) | TypeDefKind::Option(t) | TypeDefKind::List(t) => self.ty(t),
            TypeDefKind::FixedLengthList(t, _) => self.ty(t),
            TypeDefKind::Map(k, v) => {
                self.ty(k);
                self.ty(v);
            }
            TypeDefKind::Handle(Handle::Own(r) | Handle::Borrow(r)) => *r = self.type_id(*r),
            TypeDefKind::Tuple(t) => t.types.iter_mut().for_each(|t| self.ty(t)),
            TypeDefKind::Record(r) => r.fields.iter_mut().for_each(|f| self.ty(&mut f.ty)),
            TypeDefKind::Variant(v) => v.cases.iter_mut().for_each(|c| {
                if let Some(t) = &mut c.ty {
                    self.ty(t)
                }
            }),
            TypeDefKind::Result(r) => {
                if let Some(t) = &mut r.ok {
                    self.ty(t);
                }
                if let Some(t) = &mut r.err {
                    self.ty(t);
                }
            }
            TypeDefKind::Future(t) | TypeDefKind::Stream(t) => {
                if let Some(t) = t {
                    self.ty(t);
                }
            }
            TypeDefKind::Resource | TypeDefKind::Flags(_) | TypeDefKind::Enum(_) | TypeDefKind::Unknown => {}
        }
        let new = self.resolve.types.alloc(def);
        self.types.insert(id, new);
        new
    }

    fn ty(&mut self, t: &mut Type) {
        if let Type::Id(id) = t {
            *id = self.type_id(*id);
        }
    }
}

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
