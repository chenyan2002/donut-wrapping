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
    assert!(out.source.contains("pub struct FwdKvStore"));
    assert!(out.source.contains("pub struct FwdAppSession"));
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
