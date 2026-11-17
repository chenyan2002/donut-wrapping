//! A child's private copies of `G`'s interfaces: `virtual-…`
//! copies of the imports it virtualizes (which the child's world exports),
//! and `guest-…` copies of its exports and what they use (which it imports).

use std::collections::BTreeMap;
use wit_parser::{Handle, InterfaceId, PackageName, Resolve, Type, TypeDefKind, TypeId, TypeOwner};

/// Prefix of the copies of the interfaces a child imports and the donut
/// implements.
pub const VIRTUAL: &str = "virtual";
/// Prefix of the copies of a child's exports, which the donut calls, and of
/// the virtualized interfaces they use (types only).
pub const GUEST: &str = "guest";

/// The package of child `child`'s (kebab-case) copy of package `ns:pkg`:
/// `<prefix>-<child>:<ns>-<pkg>`, same version. The prefix and child are the
/// whole namespace, so two children's copies never collide. (Component names
/// don't allow uppercase in package names, which rules out marking the child
/// part with it.) Within one child, `a:b-c` and `a-b:c` would collide; the
/// plan rejects that.
pub fn copy_package_name(prefix: &str, child: &str, original: &PackageName) -> PackageName {
    PackageName {
        namespace: format!("{prefix}-{child}"),
        name: format!("{}-{}", original.namespace, original.name),
        version: original.version.clone(),
    }
}

/// Copies `ifaces` into packages renamed with [`copy_package_name`] (created
/// as needed), returning original → copy. References between the copied
/// interfaces point to the copies. Everything else they use (types of other
/// interfaces, including unlisted ones of the same package) keeps pointing to
/// the original.
pub fn copy_interfaces(
    resolve: &mut Resolve,
    prefix: &str,
    child: &str,
    ifaces: &[InterfaceId],
) -> BTreeMap<InterfaceId, InterfaceId> {
    let mut map = BTreeMap::new();
    // Empty copies first, so that types can name their new owners.
    for id in ifaces.iter() {
        let pkg = resolve.interfaces[*id].package.unwrap();
        let name = copy_package_name(prefix, child, &resolve.packages[pkg].name);
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
