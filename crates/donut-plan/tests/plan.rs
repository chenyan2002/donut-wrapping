//! `child!` plans: a snapshot of the spike's child `a`, the manifest, and
//! configuration errors. Update the snapshot with `BLESS=1 cargo test -p donut-plan`.

use donut_plan::{Config, Manifest, Plan, generate};
use std::path::Path;
use wit_parser::{PackageId, Resolve};

fn spike() -> (Resolve, Vec<PackageId>) {
    let mut resolve = Resolve::default();
    let wit = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../spike/wit");
    let (pkg, _) = resolve.push_path(wit).unwrap();
    (resolve, vec![pkg])
}

fn config(name: &str, imports: &[&str], exports: &[&str]) -> Config {
    let mut config = Config {
        name: name.into(),
        guest: "spike:hole/app".into(),
        import_passthrough: imports.iter().map(|s| s.to_string()).collect(),
        export_passthrough: exports.iter().map(|s| s.to_string()).collect(),
        ..Config::default()
    };
    config.opts.format = true;
    config
}

fn error(resolve: &Resolve, pkgs: &[PackageId], config: &Config) -> String {
    match generate(resolve, pkgs, config) {
        Ok(_) => panic!("expected an error"),
        Err(e) => format!("{e:#}"),
    }
}

/// A one-off WIT package with a guest world `t:t/g`.
fn wit(src: &str) -> (Resolve, Vec<PackageId>) {
    let mut resolve = Resolve::default();
    let pkg = resolve.push_str("t.wit", src).unwrap();
    (resolve, vec![pkg])
}

#[test]
fn spike_snapshot() {
    let (resolve, pkgs) = spike();
    let out = generate(&resolve, &pkgs, &config("a", &["wasi:clocks/*"], &["spike:hole/info"])).unwrap();
    check_snapshot("spike-a", &out.source);

    let s = &out.source;
    assert!(s.starts_with("/// Bindings for child `a`"));
    assert!(s.contains("pub mod a {"));
    // The copies, and `virt` re-exporting them under the original paths.
    assert!(s.contains(r#""virtual-a:spike-hole/types#[method]counter.bump""#));
    assert!(s.contains("pub use super::super::exports::virtual_a::spike_hole as hole;"));
    // The child's exports, as imports of their `guest-a:` copies, re-exported
    // as `guest`. The types-only copy of `types` has no functions, and the
    // original `types` isn't imported at all.
    assert!(s.contains(r#"wasm_import_module = "guest-a:spike-hole/handler""#));
    assert!(s.contains(r#"wasm_import_module = "guest-a:spike-hole/info""#));
    assert!(!s.contains(r#"wasm_import_module = "guest-a:spike-hole/types""#));
    assert!(!s.contains(r#""spike:hole/types""#));
    assert!(s.contains("pub use super::super::guest_a::spike_hole as hole;"));
    // The guest side's `counter` is the virtual copy's, by absolute path.
    assert!(s.contains(r#"with "guest-a:spike-hole/types/counter" = "crate::a::exports::virtual_a::spike_hole::types::Counter""#));
    // The passed-through clock isn't needed by the generated code.
    assert!(!s.contains("monotonic_clock"));
    // One `component-type` section, and the manifest.
    assert_eq!(s.matches("component-type:").count(), 1);
    assert!(s.contains(r#"link_section = "wit-bindgen-child:a""#));
}

#[test]
fn manifest() {
    let (resolve, pkgs) = spike();
    let out = generate(&resolve, &pkgs, &config("my_app", &["wasi:clocks/*"], &["spike:hole/info"])).unwrap();
    let m = &out.manifest;
    assert_eq!(m.name, "my-app");
    assert_eq!(m.section_name(), "wit-bindgen-child:my-app");
    assert_eq!(
        m.virtualized,
        [
            ("spike:hole/types".to_string(), "virtual-my-app:spike-hole/types".to_string()),
            ("spike:hole/host-api".to_string(), "virtual-my-app:spike-hole/host-api".to_string()),
        ]
    );
    assert_eq!(
        m.exports,
        [
            ("spike:hole/handler".to_string(), "guest-my-app:spike-hole/handler".to_string()),
            ("spike:hole/info".to_string(), "guest-my-app:spike-hole/info".to_string()),
        ]
    );
    assert_eq!(m.export_passthrough, ["spike:hole/info"]);
    assert_eq!(&Manifest::decode(&m.encode()).unwrap(), m);

    // `G` decodes back to the guest world.
    let guest = wit_component::metadata::decode(&{
        let mut module = wasm_encoder_module();
        custom_section(&mut module, "component-type:g", &m.guest);
        module
    })
    .unwrap()
    .1;
    let r = &guest.resolve;
    let w = &r.worlds[guest.world];
    let keys = |m: &wit_parser::IndexMap<wit_parser::WorldKey, wit_parser::WorldItem>| {
        m.keys().map(|k| r.name_world_key(k)).collect::<Vec<_>>()
    };
    assert_eq!(keys(&w.imports), ["wasi:clocks/monotonic-clock@0.2.0", "spike:hole/types", "spike:hole/host-api"]);
    assert_eq!(keys(&w.exports), ["spike:hole/handler", "spike:hole/info"]);
}

/// An empty core module.
fn wasm_encoder_module() -> Vec<u8> {
    b"\0asm\x01\0\0\0".to_vec()
}

fn custom_section(module: &mut Vec<u8>, name: &str, data: &[u8]) {
    let mut payload = Vec::new();
    leb(&mut payload, name.len());
    payload.extend_from_slice(name.as_bytes());
    payload.extend_from_slice(data);
    module.push(0);
    leb(module, payload.len());
    module.extend_from_slice(&payload);
}

fn leb(out: &mut Vec<u8>, mut n: usize) {
    loop {
        let b = (n & 0x7f) as u8;
        n >>= 7;
        out.push(if n == 0 { b } else { b | 0x80 });
        if n == 0 {
            return;
        }
    }
}

/// Two children of the same world don't share any generated name.
#[test]
fn two_children() {
    let (resolve, pkgs) = spike();
    let a = generate(&resolve, &pkgs, &config("a", &["wasi:clocks/*"], &[])).unwrap();
    let b = generate(&resolve, &pkgs, &config("b", &["wasi:clocks/*"], &[])).unwrap();
    // Export symbols, and `[export]…` and `guest-…` import modules: every
    // string literal naming a copy.
    let names = |s: &str| {
        s.split('"')
            .skip(1)
            .step_by(2)
            .filter(|l| l.contains("virtual-") || l.contains("guest-"))
            .map(String::from)
            .collect::<Vec<_>>()
    };
    let (a_names, b_names) = (names(&a.source), names(&b.source));
    assert!(!a_names.is_empty());
    assert!(a_names.iter().all(|n| !b_names.contains(n)), "{a_names:?}\n{b_names:?}");
    assert_ne!(a.manifest.section_name(), b.manifest.section_name());
}

#[test]
fn everything_is_virtualized_by_default() {
    let (resolve, pkgs) = spike();
    let plan = Plan::new(&resolve, &pkgs, &config("a", &[], &[])).unwrap();
    let names = plan.virtualized.iter().map(|id| plan.resolve.id_of(*id).unwrap()).collect::<Vec<_>>();
    assert_eq!(names, ["wasi:clocks/monotonic-clock@0.2.0", "spike:hole/types", "spike:hole/host-api"]);
    assert!(plan.passthrough.is_empty());
}

#[test]
fn passthrough_using_a_virtual_resource_is_rejected() {
    let (resolve, pkgs) = spike();
    // `host-api` uses `types.counter`, which stays virtualized.
    let msg = error(&resolve, &pkgs, &config("a", &["spike:hole/host-api"], &[]));
    assert!(msg.contains("`spike:hole/host-api` is passed through, but it uses resource `counter`"), "{msg}");
    assert!(msg.contains("Add `spike:hole/types` to `import_passthrough` too"), "{msg}");
}

#[test]
fn reexporting_a_user_of_a_virtual_resource_is_rejected() {
    let (resolve, pkgs) = spike();
    // `handler.run` takes a virtual `counter`.
    let msg = error(&resolve, &pkgs, &config("a", &[], &["spike:hole/handler"]));
    assert!(msg.contains("`spike:hole/handler` is re-exported, but it uses resource `counter`"), "{msg}");
}

#[test]
fn reexporting_needs_the_guest_resources_it_uses_reexported() {
    let (resolve, pkgs) = wit(
        "package t:t;
         interface model { resource doc; }
         interface api { use model.{doc}; open: func() -> doc; }
         world g { export model; export api; }",
    );
    let mut config = Config { name: "c".into(), guest: "t:t/g".into(), ..Config::default() };
    config.export_passthrough = vec!["t:t/api".into()];
    let msg = error(&resolve, &pkgs, &config);
    assert!(msg.contains("add `t:t/model` to `export_passthrough` too"), "{msg}");
    config.export_passthrough.push("t:t/model".into());
    generate(&resolve, &pkgs, &config).unwrap();
}

#[test]
fn unmatched_patterns_are_rejected() {
    let (resolve, pkgs) = spike();
    let msg = error(&resolve, &pkgs, &config("a", &["wasi:cli/*"], &[]));
    assert!(msg.contains("`wasi:cli/*` in `import_passthrough` matches no interface imported by `spike:hole/app`"), "{msg}");
    let msg = error(&resolve, &pkgs, &config("a", &["wasi:clocks/*@0.2.1"], &[]));
    assert!(msg.contains("matches no interface imported"), "{msg}");
    // Export patterns match exports only.
    let msg = error(&resolve, &pkgs, &config("a", &[], &["spike:hole/types"]));
    assert!(msg.contains("in `export_passthrough` matches no interface exported"), "{msg}");
    let msg = error(&resolve, &pkgs, &config("a", &["wasi:clocks"], &[]));
    assert!(msg.contains("invalid passthrough pattern"), "{msg}");
}

#[test]
fn bad_names_are_rejected() {
    let (resolve, pkgs) = spike();
    for name in ["", "App", "my-app", "my__app", "_app", "my_2app", "2app"] {
        let msg = error(&resolve, &pkgs, &config(name, &[], &[]));
        assert!(msg.contains("invalid child name"), "{name}: {msg}");
    }
    let msg = error(&resolve, &pkgs, &config("type", &[], &[]));
    assert!(msg.contains("it's a Rust keyword"), "{msg}");
    generate(&resolve, &pkgs, &config("app2", &[], &[])).unwrap();
    generate(&resolve, &pkgs, &config("my_app2", &[], &[])).unwrap();
}

#[test]
fn colliding_copies_are_rejected() {
    let (resolve, pkgs) = wit(
        "package t:t;
         world g { import a:b-c/x; import a-b:c/x; }
         package a:b-c { interface x { f: func(); } }
         package a-b:c { interface x { f: func(); } }",
    );
    let config = Config { name: "c".into(), guest: "t:t/g".into(), ..Config::default() };
    let msg = error(&resolve, &pkgs, &config);
    assert!(msg.contains("would both be copied to `virtual-c:a-b-c`"), "{msg}");
}

#[test]
fn unsupported_guests_are_rejected() {
    let (resolve, pkgs) = wit("package t:t; world g { import f: func(); }");
    let config = Config { name: "c".into(), guest: "t:t/g".into(), ..Config::default() };
    let msg = error(&resolve, &pkgs, &config);
    assert!(msg.contains("world-level function"), "{msg}");

    let (resolve, pkgs) = wit("package t:t; interface i { f: async func(); } world g { import i; }");
    let config = Config { name: "c".into(), guest: "t:t/g".into(), ..Config::default() };
    let msg = error(&resolve, &pkgs, &config);
    assert!(msg.contains("async functions"), "{msg}");
}

/// Compares `source` with `tests/snapshots/<name>.rs` (`BLESS=1` to update).
fn check_snapshot(name: &str, source: &str) {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("tests/snapshots/{name}.rs"));
    if std::env::var_os("BLESS").is_some() {
        std::fs::write(&path, source).unwrap();
    }
    let expected = std::fs::read_to_string(&path).expect("run with BLESS=1 to create the snapshot");
    assert!(expected == source, "snapshot mismatch; run with BLESS=1 to update {}", path.display());
}

/// A host interface the child's bindings need (`streams`, used by both the
/// virtual `out` and the exported `api`) can come from the user's own
/// bindings via `with`, for both sides of the child's world.
#[test]
fn with_shares_host_bindings() {
    let (resolve, pkgs) = wit(
        "package t:t;
         interface streams { resource output; }
         interface out { use streams.{output}; get: func() -> output; }
         interface api { use streams.{output}; take: func(s: output); }
         world g { import out; export api; }",
    );
    let mut config = Config {
        name: "c".into(),
        guest: "t:t/g".into(),
        import_passthrough: vec!["t:t/streams".into()],
        ..Config::default()
    };
    // One set of bindings for `streams`, used by both sides.
    let generated = generate(&resolve, &pkgs, &config).unwrap().source;
    assert_eq!(generated.matches("pub mod streams").count(), 1);

    config.opts.with = vec![(
        "t:t/streams".into(),
        donut_plan::wit_bindgen_rust::WithOption::Path("crate::host::t::t::streams".into()),
    )];
    let shared = generate(&resolve, &pkgs, &config).unwrap().source;
    assert!(!shared.contains("pub mod streams"));
    assert!(shared.contains("crate::host::t::t::streams"));
}

/// A record shared by both sides is one Rust type, unless it holds a borrow of
/// a virtual resource, which renders differently on each side.
#[test]
fn guest_side_remaps() {
    let (resolve, pkgs) = wit(
        "package t:t;
         interface types {
           resource r;
           record plain { n: u32 }
           record owning { x: r }
           record borrowing { x: borrow<r> }
         }
         interface api {
           use types.{r, plain, owning, borrowing};
           f: func(a: plain, b: owning, c: borrowing);
         }
         world g { import types; export api; }",
    );
    let config = Config { name: "c".into(), guest: "t:t/g".into(), ..Config::default() };
    let plan = Plan::new(&resolve, &pkgs, &config).unwrap();
    let remaps = plan.guest_side_remaps().into_iter().map(|(k, _)| k).collect::<Vec<_>>();
    assert_eq!(remaps, ["guest-c:t-t/types/r", "guest-c:t-t/types/plain", "guest-c:t-t/types/owning"]);
    generate(&resolve, &pkgs, &config).unwrap();
}
