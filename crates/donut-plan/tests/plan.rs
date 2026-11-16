//! Snapshots of the `examples/virtualize` and `examples/forward` expansions,
//! plus configuration errors.
//! Update the snapshot with `BLESS=1 cargo test -p donut-plan`.

use donut_plan::{Config, generate};
use std::path::Path;
use wit_parser::Resolve;

fn config(virtualize: &[&str]) -> Config {
    let mut config = Config {
        world: "example:virt/donut".into(),
        guest: "example:virt/guest".into(),
        virtualize: virtualize.iter().map(|s| s.to_string()).collect(),
        ..Config::default()
    };
    config.opts.format = true;
    config
}

fn example() -> (Resolve, Vec<wit_parser::PackageId>) {
    let mut resolve = Resolve::default();
    let wit = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/virtualize/wit");
    let (pkg, _) = resolve.push_path(wit).unwrap();
    (resolve, vec![pkg])
}

#[test]
fn virtualize_snapshot() {
    let (resolve, pkgs) = example();
    let out = generate(&resolve, &pkgs, &config(&["example:virt/types", "example:virt/outgoing", "example:virt/relay"]))
        .unwrap();
    check_snapshot("virtualize", &out.source);

    // World B: only the guest's exports, routed to `[guest-export]`.
    assert!(out.source.contains(r#"wasm_import_module = "[guest-export]example:virt/handler""#));
    // World B's dependency interface has no functions of its own.
    assert!(!out.source.contains(r#"wasm_import_module = "[guest-export]example:virt/types""#));
    // One `component-type` section: World A's.
    assert_eq!(out.source.matches("component-type:").count(), 1);
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

/// `examples/forward`: generated forwarding (`forward:`) around an app whose
/// world is also the donut's, so `kv` is virtualized as a renamed copy.
#[test]
fn forward_snapshot() {
    let mut resolve = Resolve::default();
    let wit = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/forward/wit");
    let (pkg, _) = resolve.push_path(wit).unwrap();
    let mut config = Config {
        world: "example:rec/app-world".into(),
        guest: "example:rec/app-world".into(),
        forward: vec!["example:rec/kv".into(), "example:rec/app".into()],
        hooks: Some("Recorder".into()),
        ..Config::default()
    };
    config.opts.format = true;
    let out = generate(&resolve, &[pkg], &config).unwrap();
    check_snapshot("forward", &out.source);

    // The virtual side is a renamed copy, and the forwarding module is there.
    assert!(out.source.contains("pub mod virtual_example"));
    assert!(out.source.contains("pub mod forward"));
    assert!(out.source.contains("pub struct FwdExampleRecKvStore"));
    assert!(out.source.contains("pub struct FwdExampleRecAppSession"));
}

#[test]
fn passthrough_using_virtual_resource_is_rejected() {
    let (resolve, pkgs) = example();
    // `outgoing` uses `types.request`; virtualizing only `types` leaves the
    // host's `outgoing` with no way to take the virtual request.
    let err = generate(&resolve, &pkgs, &config(&["example:virt/types"])).err().unwrap();
    let msg = format!("{err:#}");
    assert!(msg.contains("to `virtualize`"), "{msg}");
}

#[test]
fn unknown_virtualized_interface_is_rejected() {
    let (resolve, pkgs) = example();
    let err = generate(&resolve, &pkgs, &config(&["example:virt/handler"])).err().unwrap();
    let msg = format!("{err:#}");
    assert!(msg.contains("is not an interface imported by"), "{msg}");
}

#[test]
fn forward_rejects_lending_deep_inside_lists() {
    // A forwarded export taking `list<record { borrow<store> }>`: each host
    // borrow would need a lent temporary inside a rebuilt list of records.
    let mut resolve = Resolve::default();
    let pkg = resolve
        .push_str(
            "t.wit",
            "package t:t;
             interface kv { resource store; }
             interface app { use kv.{store}; record v { s: borrow<store> } peek: func(vs: list<v>) -> u32; }
             world donut { import kv; export app; }
             world guest { import kv; export app; }",
        )
        .unwrap();
    let config = Config {
        world: "t:t/donut".into(),
        guest: "t:t/guest".into(),
        forward: vec!["t:t/kv".into(), "t:t/app".into()],
        hooks: Some("H".into()),
        ..Config::default()
    };
    let err = generate(&resolve, &[pkg], &config).err().expect("should be rejected");
    let msg = format!("{err:#}");
    assert!(msg.contains("lending a host borrow"), "{msg}");
}

#[test]
fn forwarding_a_user_of_a_virtualized_resource_is_rejected() {
    let (resolve, pkgs) = example();
    // `outgoing` uses `types.request`, which is virtualized but not
    // forwarded, so there's no host request to forward to the host's `send`.
    let mut config = config(&["example:virt/types", "example:virt/relay"]);
    config.forward = vec!["example:virt/outgoing".into()];
    config.hooks = Some("Hooks".into());
    let err = generate(&resolve, &pkgs, &config).err().unwrap();
    let msg = format!("{err:#}");
    assert!(msg.contains("forwarded `example:virt/outgoing` uses resource `request`"), "{msg}");
    assert!(msg.contains("forward `example:virt/types` too"), "{msg}");
}

#[test]
fn an_interface_listed_twice_is_rejected() {
    let (resolve, pkgs) = example();
    let mut config = config(&["example:virt/types", "example:virt/outgoing", "example:virt/relay"]);
    config.forward = vec!["example:virt/types".into()];
    config.hooks = Some("Hooks".into());
    let err = generate(&resolve, &pkgs, &config).err().unwrap();
    let msg = format!("{err:#}");
    assert!(msg.contains("is in `virtualize` and in `forward`"), "{msg}");
}

/// The guest's side is always a `virtual-` copy, of only the listed
/// interfaces: a virtualized `outgoing` keeps using the passthrough `types`,
/// even though the donut's own world also imports `outgoing`.
#[test]
fn virtual_copies_only_the_listed_interfaces() {
    let mut resolve = Resolve::default();
    let pkg = resolve
        .push_str(
            "app.wit",
            r#"
            package example:app;
            interface types { resource request { constructor(url: string); } }
            interface outgoing { use types.{request}; send: func(r: request) -> u16; }
            interface handler { handle: func() -> u16; }
            world app { import outgoing; export handler; }
            world host { import outgoing; export handler; }
            "#,
        )
        .unwrap();
    let mut config = Config {
        world: "example:app/host".into(),
        guest: "example:app/app".into(),
        virtualize: vec!["example:app/outgoing".into()],
        ..Config::default()
    };
    config.opts.format = true;
    let out = generate(&resolve, &[pkg], &config).unwrap();
    // The guest's `outgoing` is the renamed copy; its `request` is the host's.
    assert!(out.source.contains("pub mod virtual_example"), "no virtual copy");
    assert!(!out.source.contains("virtual_example::app::types"), "`types` was copied");
}

fn fs_example() -> (Resolve, wit_parser::PackageId) {
    let mut resolve = Resolve::default();
    let pkg = resolve
        .push_str(
            "app.wit",
            r#"
            package example:app;
            interface fs {
              resource file { read: func() -> string; }
              resource dir { entries: func() -> list<string>; }
              open: func(path: string) -> result<file, string>;
            }
            interface handler { handle: func() -> string; }
            world app { import fs; export handler; }
            world host { export handler; }
            "#,
        )
        .unwrap();
    (resolve, pkg)
}

fn mock_config(types: &[(&str, &str, &str)]) -> Config {
    let mut config = Config {
        world: "example:app/host".into(),
        guest: "example:app/app".into(),
        mock: vec!["example:app/fs".into()],
        hooks: Some("Hooks".into()),
        ..Config::default()
    };
    for (iface, res, ty) in types {
        config.mock_types.entry(iface.to_string()).or_default().insert(res.to_string(), ty.to_string());
    }
    config
}

/// A mocked resource with a state type is a newtype over it; one without
/// can't be created at all.
#[test]
fn mock_state_types() {
    let (resolve, pkg) = fs_example();
    let out = generate(&resolve, &[pkg], &mock_config(&[("example:app/fs", "file", "crate::OpenFile")])).unwrap();
    assert!(out.source.contains("type __DonutMockExampleAppFsFileState = crate::OpenFile;"));
    assert!(out.source.contains("pub struct MockExampleAppFsFile { state: Option<crate::__DonutMockExampleAppFsFileState> }"));
    assert!(out.source.contains("pub struct MockExampleAppFsDir { never: core::convert::Infallible }"));
}

#[test]
fn mock_state_type_errors() {
    let (resolve, pkg) = fs_example();
    let msg = |c: &Config| format!("{:#}", generate(&resolve, &[pkg], c).err().unwrap());
    let m = msg(&mock_config(&[("example:app/fs", "open", "T")]));
    assert!(m.contains("has no resource `open`"), "{m}");
    let m = msg(&mock_config(&[("example:app/handler", "x", "T")]));
    assert!(m.contains("isn't in `mock`"), "{m}");
}
