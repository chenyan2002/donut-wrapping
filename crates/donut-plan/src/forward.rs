//! Generated forwarding (`forward:`, design §6).
//!
//! * A forwarded **virtual** interface (a guest import) is implemented by
//!   calling the host's import of the same interface. Each of its resources
//!   is a wrapper around the host's resource: owned, or lent for one call.
//! * A forwarded **export** (exported by both the donut's world and the guest)
//!   is implemented by calling the guest's export. Each of its resources is a
//!   wrapper around the guest's resource.
//!
//! Every forwarded call goes through the user's `Hooks` impl (`before`,
//! `after`, `dropped`), whose defaults do nothing.
//!
//! Only values that contain forwarded resources are converted, by code
//! generated from the WIT structure. Everything else is passed through
//! unchanged. Host-side and virtual copies of resource-free types are the same
//! Rust type, because World A is generated with
//! `merge_structurally_equal_types`. Arguments to imports go through
//! `crate::forward::Arg`, so wit-bindgen's import parameter conventions (`&str`,
//! `&[T]`, `&Record`, ...) are picked by type inference rather than
//! reproduced here.

use crate::{Plan, camel, module_path};
use anyhow::{Result, bail};
use std::collections::{HashMap, HashSet};
use std::fmt::Write;
use wit_bindgen_rust::to_rust_ident;
use wit_parser::{
    Function, FunctionKind, Handle, InterfaceId, Resolve, Type, TypeDefKind, TypeId, TypeOwner,
    WorldItem,
};

pub(crate) fn generate(plan: &Plan, hooks: &str) -> Result<String> {
    let resolve = &plan.resolve;
    let exported = resolve.worlds[plan.world_a]
        .exports
        .values()
        .filter_map(|item| match item {
            WorldItem::Interface { id, .. } => Some(*id),
            _ => None,
        })
        .collect::<HashSet<_>>();
    let mut g = Gen {
        resolve,
        hooks: format!("crate::{hooks}"),
        exported,
        virt: HashMap::new(),
        host: HashMap::new(),
        export_res: HashMap::new(),
        out: String::new(),
    };
    for (v, h) in plan.forward_virtual.iter() {
        for (name, ty) in resolve.interfaces[*v].types.iter() {
            if matches!(resolve.types[*ty].kind, TypeDefKind::Resource) {
                let host_ty = resolve.interfaces[*h].types[name];
                let wrapper = format!("Fwd{}{}", camel(iface_name(resolve, *h)), camel(name));
                g.virt.insert(*ty, (host_ty, wrapper.clone()));
                g.host.insert(host_ty, (*ty, wrapper));
            }
        }
    }
    for e in plan.forward_exports.iter() {
        for (name, ty) in resolve.interfaces[*e].types.iter() {
            if matches!(resolve.types[*ty].kind, TypeDefKind::Resource) {
                let wrapper = format!("Fwd{}{}", camel(iface_name(resolve, *e)), camel(name));
                g.export_res.insert(*ty, wrapper);
            }
        }
    }

    g.prelude();
    for (v, h) in plan.forward_virtual.iter() {
        g.virtual_interface(*v, *h)?;
    }
    for e in plan.forward_exports.iter() {
        g.export_interface(*e)?;
    }
    writeln!(g.out, "}}")?;
    Ok(g.out)
}

fn iface_name(resolve: &Resolve, id: InterfaceId) -> &str {
    resolve.interfaces[id].name.as_deref().unwrap()
}

struct Gen<'a> {
    resolve: &'a Resolve,
    hooks: String,
    /// Interfaces World A exports (as opposed to imports).
    exported: HashSet<InterfaceId>,
    /// Virtual (guest-side) resource → (host resource, wrapper name).
    virt: HashMap<TypeId, (TypeId, String)>,
    /// Host resource → (virtual resource, wrapper name).
    host: HashMap<TypeId, (TypeId, String)>,
    /// Resource of a forwarded export → wrapper name.
    export_res: HashMap<TypeId, String>,
    out: String,
}

/// Which way a forwarded call goes.
#[derive(Clone, Copy)]
enum Dir {
    /// Guest → host, through a virtual interface.
    Import,
    /// Host → guest, through a forwarded export.
    Export,
}

impl<'a> Gen<'a> {
    fn prelude(&mut self) {
        self.out.push_str(
            r#"
/// Generated forwarding (`forward:`): wrappers, hooks, and helpers.
#[allow(dead_code, unused_imports, unused_variables, clippy::all)]
pub mod forward {
    use core::fmt::Debug;

    /// Which way a forwarded call goes.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum Direction {
        /// The guest calls the host, through a virtualized interface.
        Import,
        /// The host calls the guest, through a forwarded export.
        Export,
    }

    /// A forwarded call: its direction, interface, and WIT function name.
    #[derive(Clone, Copy, Debug)]
    pub struct Call {
        pub direction: Direction,
        pub interface: &'static str,
        pub function: &'static str,
    }

    /// Stands in for a resource handle in `Hooks` arguments and results.
    pub struct Handle(pub &'static str);

    impl Debug for Handle {
        fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
            write!(f, "<{}>", self.0)
        }
    }

    /// Observes every forwarded call. The defaults do nothing, which makes
    /// forwarding transparent.
    pub trait Hooks {
        fn before(call: &Call, args: &[&dyn Debug]) {}
        fn after(call: &Call, ret: &dyn Debug) {}
        /// A forwarded resource was dropped by whoever owned it.
        fn dropped(direction: Direction, interface: &'static str, resource: &'static str) {}
    }

    /// Adapts an owned value to how an import takes it (`&str`, `&[T]`, `&T`,
    /// `T`, ...). The impl is picked from the import's parameter type.
    pub trait Arg<'a, U> {
        fn arg(&'a self) -> U;
    }
    impl<'a> Arg<'a, &'a str> for String {
        fn arg(&'a self) -> &'a str { self }
    }
    impl<'a, T> Arg<'a, &'a [T]> for Vec<T> {
        fn arg(&'a self) -> &'a [T] { self }
    }
    impl<'a, T> Arg<'a, &'a T> for T {
        fn arg(&'a self) -> &'a T { self }
    }
    impl<'a, T: Clone> Arg<'a, T> for T {
        fn arg(&'a self) -> T { self.clone() }
    }
    impl<'a> Arg<'a, Option<&'a str>> for Option<String> {
        fn arg(&'a self) -> Option<&'a str> { self.as_deref() }
    }
    impl<'a, T> Arg<'a, Option<&'a [T]>> for Option<Vec<T>> {
        fn arg(&'a self) -> Option<&'a [T]> { self.as_deref() }
    }
    impl<'a, T> Arg<'a, Option<&'a T>> for Option<T> {
        fn arg(&'a self) -> Option<&'a T> { self.as_ref() }
    }
"#,
        );
    }

    // ---- paths and types -------------------------------------------------

    fn iface_path(&self, id: InterfaceId) -> String {
        format!("crate::{}", module_path(self.resolve, id, self.exported.contains(&id)))
    }

    fn guest_path(&self, id: InterfaceId) -> String {
        format!("crate::guest::{}", module_path(self.resolve, id, false))
    }

    fn dealias(&self, mut id: TypeId) -> TypeId {
        while let TypeDefKind::Type(Type::Id(next)) = self.resolve.types[id].kind {
            id = next;
        }
        id
    }

    fn owner(&self, id: TypeId) -> Result<InterfaceId> {
        match self.resolve.types[id].owner {
            TypeOwner::Interface(i) => Ok(i),
            _ => bail!("forward: world-level types aren't supported yet"),
        }
    }

    /// The path of a resource's (or named type's) definition.
    fn named_path(&self, id: TypeId) -> Result<String> {
        let def = &self.resolve.types[id];
        Ok(format!(
            "{}::{}",
            self.iface_path(self.owner(id)?),
            camel(def.name.as_deref().unwrap())
        ))
    }

    fn resource_name(&self, id: TypeId) -> &'static str {
        let name = self.resolve.types[self.dealias(id)].name.clone().unwrap();
        Box::leak(name.into_boxed_str())
    }

    /// The Rust type of `ty` in an export-side (owned) signature of World A.
    fn ty(&self, ty: &Type) -> Result<String> {
        Ok(match ty {
            Type::Bool => "bool".into(),
            Type::U8 => "u8".into(),
            Type::U16 => "u16".into(),
            Type::U32 => "u32".into(),
            Type::U64 => "u64".into(),
            Type::S8 => "i8".into(),
            Type::S16 => "i16".into(),
            Type::S32 => "i32".into(),
            Type::S64 => "i64".into(),
            Type::F32 => "f32".into(),
            Type::F64 => "f64".into(),
            Type::Char => "char".into(),
            Type::String => "String".into(),
            Type::ErrorContext => bail!("forward: `error-context` isn't supported"),
            Type::Id(id) => self.ty_id(*id)?,
        })
    }

    fn ty_id(&self, id: TypeId) -> Result<String> {
        let def = &self.resolve.types[id];
        if def.name.is_some() {
            let lt = if self.has(&Type::Id(id), &|h| matches!(h, Handle::Borrow(_))) {
                "<'_>"
            } else {
                ""
            };
            return Ok(format!("{}{lt}", self.named_path(id)?));
        }
        Ok(match &def.kind {
            TypeDefKind::Handle(Handle::Own(r)) => self.named_path(self.dealias(*r))?,
            TypeDefKind::Handle(Handle::Borrow(r)) => {
                let r = self.dealias(*r);
                if self.exported.contains(&self.owner(r)?) {
                    format!("{}Borrow<'_>", self.named_path(r)?)
                } else {
                    format!("&{}", self.named_path(r)?)
                }
            }
            TypeDefKind::Option(t) => format!("Option<{}>", self.ty(t)?),
            TypeDefKind::List(t) => format!("Vec<{}>", self.ty(t)?),
            TypeDefKind::Result(r) => format!(
                "Result<{}, {}>",
                r.ok.as_ref().map(|t| self.ty(t)).transpose()?.unwrap_or("()".into()),
                r.err.as_ref().map(|t| self.ty(t)).transpose()?.unwrap_or("()".into()),
            ),
            TypeDefKind::Tuple(t) => {
                let mut s = String::from("(");
                for t in t.types.iter() {
                    write!(s, "{}, ", self.ty(t)?)?;
                }
                s.push(')');
                s
            }
            TypeDefKind::Type(t) => self.ty(t)?,
            _ => bail!("forward: this kind of type isn't supported yet"),
        })
    }

    /// Whether `ty` contains a handle matching `f`.
    fn has(&self, ty: &Type, f: &dyn Fn(&Handle) -> bool) -> bool {
        let Type::Id(id) = ty else { return false };
        match &self.resolve.types[*id].kind {
            TypeDefKind::Handle(h) => f(h),
            TypeDefKind::Record(r) => r.fields.iter().any(|x| self.has(&x.ty, f)),
            TypeDefKind::Variant(v) => v.cases.iter().any(|c| c.ty.as_ref().is_some_and(|t| self.has(t, f))),
            TypeDefKind::Tuple(t) => t.types.iter().any(|t| self.has(t, f)),
            TypeDefKind::Option(t) | TypeDefKind::List(t) | TypeDefKind::Type(t) => self.has(t, f),
            TypeDefKind::Result(r) => {
                r.ok.as_ref().is_some_and(|t| self.has(t, f))
                    || r.err.as_ref().is_some_and(|t| self.has(t, f))
            }
            _ => false,
        }
    }

    fn any_handle(&self, ty: &Type) -> bool {
        self.has(ty, &|_| true)
    }

    // ---- conversions ------------------------------------------------------

    /// A structure-directed conversion of `e` (of WIT type `ty`), applying
    /// `leaf` at each handle. Returns `None` if nothing needs converting.
    fn convert(
        &self,
        ty: &Type,
        e: &str,
        leaf: &dyn Fn(&Handle, &str) -> Result<Option<String>>,
    ) -> Result<Option<String>> {
        let Type::Id(id) = ty else { return Ok(None) };
        let some = |s: String| Ok(Some(s));
        match &self.resolve.types[*id].kind {
            TypeDefKind::Handle(h) => leaf(h, e),
            TypeDefKind::Type(t) => self.convert(t, e, leaf),
            TypeDefKind::Option(t) => match self.convert(t, "v", leaf)? {
                Some(c) => some(format!("{e}.map(|v| {c})")),
                None => Ok(None),
            },
            TypeDefKind::List(t) => match self.convert(t, "v", leaf)? {
                Some(c) => some(format!("{e}.into_iter().map(|v| {c}).collect::<Vec<_>>()")),
                None => Ok(None),
            },
            TypeDefKind::Result(r) => {
                let ok = match &r.ok {
                    Some(t) => self.convert(t, "v", leaf)?,
                    None => None,
                };
                let err = match &r.err {
                    Some(t) => self.convert(t, "v", leaf)?,
                    None => None,
                };
                if ok.is_none() && err.is_none() {
                    return Ok(None);
                }
                some(format!(
                    "{e}.map(|v| {}).map_err(|v| {})",
                    ok.unwrap_or("v".into()),
                    err.unwrap_or("v".into())
                ))
            }
            TypeDefKind::Tuple(t) => {
                let mut parts = Vec::new();
                let mut any = false;
                for (i, t) in t.types.iter().enumerate() {
                    match self.convert(t, &format!("t.{i}"), leaf)? {
                        Some(c) => {
                            any = true;
                            parts.push(c);
                        }
                        None => parts.push(format!("t.{i}")),
                    }
                }
                if !any {
                    return Ok(None);
                }
                some(format!("{{ let t = {e}; ({},) }}", parts.join(", ")))
            }
            _ if self.any_handle(ty) => bail!(
                "forward: records and variants that contain resources aren't supported yet"
            ),
            _ => Ok(None),
        }
    }

    /// Virtual (guest-side) value → host value, for arguments to the host.
    fn virt_to_host(&self, ty: &Type, e: &str) -> Result<String> {
        let leaf = |h: &Handle, e: &str| -> Result<Option<String>> {
            Ok(match h {
                Handle::Own(r) => self
                    .virt
                    .get(&self.dealias(*r))
                    .map(|(_, w)| format!("{e}.into_inner::<{w}>().into_host()")),
                Handle::Borrow(r) => self
                    .virt
                    .get(&self.dealias(*r))
                    .map(|(_, w)| format!("{e}.get::<{w}>().host()")),
            })
        };
        Ok(self.convert(ty, e, &leaf)?.unwrap_or(e.into()))
    }

    /// Host value → virtual value, for results from the host. `ty` is the
    /// virtual-side type.
    fn host_to_virt(&self, ty: &Type, e: &str) -> Result<String> {
        let leaf = |h: &Handle, e: &str| -> Result<Option<String>> {
            match h {
                Handle::Own(r) => {
                    let r = self.dealias(*r);
                    Ok(match self.virt.get(&r) {
                        Some((_, w)) => Some(format!(
                            "{}::new(crate::forward::{w}::owned({e}))",
                            self.named_path(r)?
                        )),
                        None => None,
                    })
                }
                Handle::Borrow(_) => Ok(None),
            }
        };
        Ok(self.convert(ty, e, &leaf)?.unwrap_or(e.into()))
    }

    /// Host-side value → guest-side value, for arguments to the guest.
    /// Top-level borrows of forwarded host resources are lent through
    /// temporaries, declared in `lets`.
    fn host_to_guest_arg(&self, ty: &Type, e: &str, lets: &mut String, i: usize) -> Result<String> {
        if let Type::Id(id) = ty {
            if let TypeDefKind::Handle(Handle::Borrow(r)) = &self.resolve.types[*id].kind {
                let r = self.dealias(*r);
                if let Some((_, w)) = self.host.get(&r) {
                    writeln!(lets, "let lent{i} = crate::forward::{w}::lend({e});")?;
                    return Ok(format!("&lent{i}"));
                }
            }
        }
        let leaf = |h: &Handle, e: &str| -> Result<Option<String>> {
            match h {
                Handle::Own(r) => {
                    let r = self.dealias(*r);
                    if let Some((v, w)) = self.host.get(&r) {
                        return Ok(Some(format!(
                            "{}::new(crate::forward::{w}::owned({e}))",
                            self.named_path(*v)?
                        )));
                    }
                    Ok(self
                        .export_res
                        .get(&r)
                        .map(|w| format!("{e}.into_inner::<crate::forward::{w}>().into_guest()")))
                }
                Handle::Borrow(r) => {
                    let r = self.dealias(*r);
                    if self.host.contains_key(&r) {
                        bail!("forward: borrows of forwarded resources are only supported as top-level parameters");
                    }
                    Ok(self
                        .export_res
                        .get(&r)
                        .map(|w| format!("{e}.get::<crate::forward::{w}>().guest()")))
                }
            }
        };
        Ok(self.convert(ty, e, &leaf)?.unwrap_or(e.into()))
    }

    /// Guest-side value → host-side value, for results from the guest. `ty` is
    /// the host-side (World A) type.
    fn guest_to_host(&self, ty: &Type, e: &str) -> Result<String> {
        let leaf = |h: &Handle, e: &str| -> Result<Option<String>> {
            match h {
                Handle::Own(r) => {
                    let r = self.dealias(*r);
                    if let Some((_, w)) = self.host.get(&r) {
                        return Ok(Some(format!("{e}.into_inner::<crate::forward::{w}>().into_host()")));
                    }
                    Ok(match self.export_res.get(&r) {
                        Some(w) => Some(format!(
                            "{}::new(crate::forward::{w}::wrap({e}))",
                            self.named_path(r)?
                        )),
                        None => None,
                    })
                }
                Handle::Borrow(_) => Ok(None),
            }
        };
        Ok(self.convert(ty, e, &leaf)?.unwrap_or(e.into()))
    }

    /// How a converted argument is passed to an import: values containing
    /// handles are passed as converted, everything else through `Arg`.
    fn pass(&self, ty: &Type, converted: &str, name: &str) -> String {
        if self.any_handle(ty) {
            converted.to_string()
        } else {
            format!("crate::forward::Arg::arg(&{name})")
        }
    }

    fn debug_arg(&self, ty: &Type, name: &str) -> String {
        if !self.any_handle(ty) {
            return format!("&{name}");
        }
        // A handle shows as its resource's name; anything containing one as
        // `<..>`.
        let label = match ty {
            Type::Id(id) => match &self.resolve.types[*id].kind {
                TypeDefKind::Handle(Handle::Own(r) | Handle::Borrow(r)) => self.resource_name(*r),
                _ => "..",
            },
            _ => "..",
        };
        format!("&crate::forward::Handle(\"{label}\")")
    }

    // ---- wrappers ---------------------------------------------------------

    fn host_wrapper(&mut self, v: TypeId, h: TypeId, w: &str, iface: &str) -> Result<()> {
        let host = self.named_path(h)?;
        let virt = self.named_path(v)?;
        let res = self.resource_name(v);
        let hooks = &self.hooks;
        write!(
            self.out,
            r#"
    /// The guest's `{res}`: a host `{res}`, owned or lent for one call.
    pub struct {w} {{ inner: Option<{host}>, lent: bool }}
    impl {w} {{
        pub fn owned(h: {host}) -> Self {{ Self {{ inner: Some(h), lent: false }} }}
        /// Lends a host `{res}` to the guest for the duration of one call.
        pub fn lend(h: &{host}) -> {virt} {{
            // Never dropped (`lent`), and dropped by the caller before the
            // borrow ends.
            {virt}::new(Self {{ inner: Some(unsafe {{ {host}::from_handle(h.handle()) }}), lent: true }})
        }}
        pub fn host(&self) -> &{host} {{ self.inner.as_ref().unwrap() }}
        pub fn into_host(mut self) -> {host} {{
            assert!(!self.lent, "a lent `{res}` can't be moved");
            self.inner.take().unwrap()
        }}
    }}
    impl Drop for {w} {{
        fn drop(&mut self) {{
            if let Some(h) = self.inner.take() {{
                if self.lent {{
                    core::mem::forget(h);
                }} else {{
                    <{hooks} as Hooks>::dropped(Direction::Import, "{iface}", "{res}");
                }}
            }}
        }}
    }}
"#
        )?;
        Ok(())
    }

    fn guest_wrapper(&mut self, r: TypeId, e: InterfaceId, w: &str, iface: &str) -> Result<()> {
        let res = self.resource_name(r);
        let guest = format!("{}::{}", self.guest_path(e), camel(res));
        let hooks = &self.hooks;
        write!(
            self.out,
            r#"
    /// Our `{res}`: wraps the guest's.
    pub struct {w} {{ inner: Option<{guest}> }}
    impl {w} {{
        pub fn wrap(g: {guest}) -> Self {{ Self {{ inner: Some(g) }} }}
        pub fn guest(&self) -> &{guest} {{ self.inner.as_ref().unwrap() }}
        pub fn into_guest(mut self) -> {guest} {{ self.inner.take().unwrap() }}
    }}
    impl Drop for {w} {{
        fn drop(&mut self) {{
            if self.inner.take().is_some() {{
                <{hooks} as Hooks>::dropped(Direction::Export, "{iface}", "{res}");
            }}
        }}
    }}
"#
        )?;
        Ok(())
    }

    // ---- functions --------------------------------------------------------

    /// The signature of `func` in its export-side trait, skipping `self`.
    fn signature(&self, func: &Function) -> Result<(String, Vec<(String, Type)>)> {
        let mut params = Vec::new();
        let mut sig = format!("fn {}(", self.fn_name(func));
        let skip = usize::from(matches!(func.kind, FunctionKind::Method(_)));
        if skip == 1 {
            sig.push_str("&self, ");
        }
        for (i, p) in func.params.iter().enumerate().skip(skip) {
            let name = format!("p{i}");
            write!(sig, "{name}: {}, ", self.ty(&p.ty)?)?;
            params.push((name, p.ty));
        }
        sig.push(')');
        match (&func.kind, &func.result) {
            (FunctionKind::Constructor(_), _) => sig.push_str(" -> Self"),
            (_, Some(t)) => write!(sig, " -> {}", self.ty(t)?)?,
            (_, None) => {}
        }
        Ok((sig, params))
    }

    fn fn_name(&self, func: &Function) -> String {
        match func.kind {
            FunctionKind::Constructor(_) => "new".into(),
            _ => to_rust_ident(func.item_name()),
        }
    }

    /// Writes one forwarding function: hooks around a call to `callee`.
    #[allow(clippy::too_many_arguments)]
    fn function(
        &mut self,
        func: &Function,
        dir: Dir,
        iface: &str,
        callee: String,
        args: Vec<String>,
        lets: String,
        convert_ret: &dyn Fn(&Self, &Type, &str) -> Result<String>,
        constructor_wrap: Option<String>,
    ) -> Result<()> {
        let (sig, params) = self.signature(func)?;
        let direction = match dir {
            Dir::Import => "Import",
            Dir::Export => "Export",
        };
        let debug = params
            .iter()
            .map(|(n, t)| self.debug_arg(t, n))
            .collect::<Vec<_>>()
            .join(", ");
        let hooks = &self.hooks;
        let mut body = format!(
            "let call = crate::forward::Call {{ direction: crate::forward::Direction::{direction}, \
             interface: \"{iface}\", function: \"{}\" }};\n\
             <{hooks} as crate::forward::Hooks>::before(&call, &[{debug}]);\n{lets}",
            func.name
        );
        let call = format!("{callee}({})", args.join(", "));
        match (&func.kind, &func.result, constructor_wrap) {
            (FunctionKind::Constructor(_), _, Some(wrap)) => {
                writeln!(body, "let ret = {call};")?;
                let res = self.resource_name(func.kind.resource().unwrap());
                writeln!(body, "<{hooks} as crate::forward::Hooks>::after(&call, &crate::forward::Handle(\"{res}\"));")?;
                writeln!(body, "{wrap}(ret)")?;
            }
            (_, Some(t), _) => {
                writeln!(body, "let ret = {call};")?;
                writeln!(body, "let ret = {};", convert_ret(self, t, "ret")?)?;
                let dbg = self.debug_arg(t, "ret");
                writeln!(body, "<{hooks} as crate::forward::Hooks>::after(&call, {dbg});")?;
                writeln!(body, "ret")?;
            }
            (_, None, _) => {
                writeln!(body, "{call};")?;
                writeln!(body, "<{hooks} as crate::forward::Hooks>::after(&call, &());")?;
            }
        }
        writeln!(self.out, "        {sig} {{\n{body}        }}")?;
        Ok(())
    }

    /// A forwarded virtual interface `v` (the guest's copy), implemented by
    /// calling the host's `h`.
    fn virtual_interface(&mut self, v: InterfaceId, h: InterfaceId) -> Result<()> {
        let iface = self.resolve.id_of(h).unwrap();
        let vpath = self.iface_path(v);
        let hpath = self.iface_path(h);
        let resolve = self.resolve;
        let resources = resolve.interfaces[v]
            .types
            .iter()
            .filter(|(_, t)| matches!(resolve.types[**t].kind, TypeDefKind::Resource))
            .map(|(n, t)| (n.clone(), *t))
            .collect::<Vec<_>>();
        for (_, r) in resources.iter() {
            let (host_r, w) = self.virt[r].clone();
            self.host_wrapper(*r, host_r, &w, &iface)?;
        }

        // The interface's free functions, and its resource types.
        writeln!(self.out, "    impl {vpath}::Guest for {} {{", self.hooks)?;
        for (name, r) in resources.iter() {
            writeln!(self.out, "        type {} = {};", camel(name), self.virt[r].1)?;
        }
        for func in resolve.interfaces[v].functions.values() {
            if func.kind.resource().is_some() {
                continue;
            }
            let (args, lets) = self.host_args(func, 0)?;
            self.function(
                func,
                Dir::Import,
                &iface,
                format!("{hpath}::{}", self.fn_name(func)),
                args,
                lets,
                &|g, t, e| g.host_to_virt(t, e),
                None,
            )?;
        }
        writeln!(self.out, "    }}")?;

        for (name, r) in resources.iter() {
            let w = self.virt[r].1.clone();
            writeln!(self.out, "    impl {vpath}::Guest{} for {w} {{", camel(name))?;
            for func in resolve.interfaces[v].functions.values() {
                if func.kind.resource() != Some(*r) {
                    continue;
                }
                let host_r = format!("{hpath}::{}", camel(name));
                let (callee, skip, wrap) = match func.kind {
                    FunctionKind::Method(_) => (format!("self.host().{}", self.fn_name(func)), 1, None),
                    FunctionKind::Constructor(_) => {
                        (format!("{host_r}::new"), 0, Some("Self::owned".to_string()))
                    }
                    _ => (format!("{host_r}::{}", self.fn_name(func)), 0, None),
                };
                let (args, lets) = self.host_args(func, skip)?;
                self.function(func, Dir::Import, &iface, callee, args, lets, &|g, t, e| g.host_to_virt(t, e), wrap)?;
            }
            writeln!(self.out, "    }}")?;
        }
        Ok(())
    }

    fn host_args(&self, func: &Function, skip: usize) -> Result<(Vec<String>, String)> {
        let mut args = Vec::new();
        for (i, p) in func.params.iter().enumerate().skip(skip) {
            let name = format!("p{i}");
            let converted = self.virt_to_host(&p.ty, &name)?;
            args.push(self.pass(&p.ty, &converted, &name));
        }
        Ok((args, String::new()))
    }

    fn guest_args(&self, func: &Function, skip: usize) -> Result<(Vec<String>, String)> {
        let mut args = Vec::new();
        let mut lets = String::new();
        for (i, p) in func.params.iter().enumerate().skip(skip) {
            let name = format!("p{i}");
            let converted = self.host_to_guest_arg(&p.ty, &name, &mut lets, i)?;
            args.push(self.pass(&p.ty, &converted, &name));
        }
        Ok((args, lets))
    }

    /// A forwarded export `e`, implemented by calling the guest's `e`.
    fn export_interface(&mut self, e: InterfaceId) -> Result<()> {
        let iface = self.resolve.id_of(e).unwrap();
        let epath = self.iface_path(e);
        let gpath = self.guest_path(e);
        let resolve = self.resolve;
        let resources = resolve.interfaces[e]
            .types
            .iter()
            .filter(|(_, t)| matches!(resolve.types[**t].kind, TypeDefKind::Resource))
            .map(|(n, t)| (n.clone(), *t))
            .collect::<Vec<_>>();
        for (_, r) in resources.iter() {
            let w = self.export_res[r].clone();
            self.guest_wrapper(*r, e, &w, &iface)?;
        }

        writeln!(self.out, "    impl {epath}::Guest for {} {{", self.hooks)?;
        for (name, r) in resources.iter() {
            writeln!(self.out, "        type {} = {};", camel(name), self.export_res[r])?;
        }
        for func in resolve.interfaces[e].functions.values() {
            if func.kind.resource().is_some() {
                continue;
            }
            let (args, lets) = self.guest_args(func, 0)?;
            self.function(
                func,
                Dir::Export,
                &iface,
                format!("{gpath}::{}", self.fn_name(func)),
                args,
                lets,
                &|g, t, e| g.guest_to_host(t, e),
                None,
            )?;
        }
        writeln!(self.out, "    }}")?;

        for (name, r) in resources.iter() {
            let w = self.export_res[r].clone();
            writeln!(self.out, "    impl {epath}::Guest{} for {w} {{", camel(name))?;
            for func in resolve.interfaces[e].functions.values() {
                if func.kind.resource() != Some(*r) {
                    continue;
                }
                let guest_r = format!("{gpath}::{}", camel(name));
                let (callee, skip, wrap) = match func.kind {
                    FunctionKind::Method(_) => (format!("self.guest().{}", self.fn_name(func)), 1, None),
                    FunctionKind::Constructor(_) => {
                        (format!("{guest_r}::new"), 0, Some("Self::wrap".to_string()))
                    }
                    _ => (format!("{guest_r}::{}", self.fn_name(func)), 0, None),
                };
                let (args, lets) = self.guest_args(func, skip)?;
                self.function(func, Dir::Export, &iface, callee, args, lets, &|g, t, e| g.guest_to_host(t, e), wrap)?;
            }
            writeln!(self.out, "    }}")?;
        }
        Ok(())
    }
}
