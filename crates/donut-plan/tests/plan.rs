//! Snapshot of the `examples/virtualize` expansion, plus configuration errors.
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
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/snapshots/virtualize.rs");
    if std::env::var_os("BLESS").is_some() {
        std::fs::write(&path, &out.source).unwrap();
    }
    let expected = std::fs::read_to_string(&path).expect("run with BLESS=1 to create the snapshot");
    assert!(expected == out.source, "snapshot mismatch; run with BLESS=1 to update {}", path.display());

    // World B: only the guest's exports, routed to `[guest-export]`.
    assert!(out.source.contains(r#"wasm_import_module = "[guest-export]example:virt/handler""#));
    // World B's dependency interface has no functions of its own.
    assert!(!out.source.contains(r#"wasm_import_module = "[guest-export]example:virt/types""#));
    // One `component-type` section: World A's.
    assert_eq!(out.source.matches("component-type:").count(), 1);
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
