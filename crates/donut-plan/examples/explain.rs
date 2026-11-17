//! Dumps the intermediate steps of one `child!`, for explaining it.
//!
//! ```sh
//! cargo run -p donut-plan --example explain -- \
//!     examples/greeter/wit example:greeter/app english 'wasi:*' > /dev/null
//! ```
//!
//! Args: WIT path, guest world, child name, import_passthrough patterns…
//! Writes `target/explain/<name>/`:
//!
//! * `1-child-world.wit`: the child's world, with the copies it uses;
//! * `2-wit-bindgen.txt`: the equivalent wit-bindgen call;
//! * `3-bindings.rs`: the generated code, formatted;
//! * `4-manifest.txt`: the manifest, with its guest world decoded.

use donut_plan::wit_parser::{self, PackageId, Resolve, WorldItem};
use donut_plan::{Config, Plan};
use std::collections::BTreeSet;
use std::fmt::Write;
use std::path::PathBuf;
use wit_component::WitPrinter;

fn main() -> anyhow::Result<()> {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    let [path, guest, name, passthrough @ ..] = &args[..] else {
        anyhow::bail!("usage: explain <wit-path> <guest-world> <child-name> [import-passthrough…]");
    };
    let mut resolve = Resolve::default();
    let (pkg, _) = resolve.push_path(path)?;
    let mut config =
        Config { name: name.clone(), guest: guest.clone(), import_passthrough: passthrough.to_vec(), ..Config::default() };
    config.opts.format = true;
    let plan = Plan::new(&resolve, &[pkg], &config)?;
    let out = PathBuf::from("target/explain").join(name);
    std::fs::create_dir_all(&out)?;

    // 1. The child's world, and the packages of the copies it names.
    let r = &plan.resolve;
    let world_pkg = r.worlds[plan.world].package.unwrap();
    let mut copies = BTreeSet::new();
    let w = &r.worlds[plan.world];
    for item in w.imports.values().chain(w.exports.values()) {
        if let WorldItem::Interface { id, .. } = item {
            copies.insert(r.interfaces[*id].package.unwrap());
        }
    }
    let mut printer = WitPrinter::default();
    printer.print(r, world_pkg, &copies.into_iter().collect::<Vec<PackageId>>())?;
    let mut wit = String::new();
    writeln!(wit, "// Classification of `{guest}`'s items for child `{name}`:")?;
    let id_of = |ids: &[wit_parser::InterfaceId]| ids.iter().map(|i| r.id_of(*i).unwrap()).collect::<Vec<_>>();
    writeln!(wit, "//   virtualized imports:  {:?}", id_of(&plan.virtualized))?;
    writeln!(wit, "//   passthrough imports:  {:?}", id_of(&plan.passthrough))?;
    writeln!(wit, "//   exports (called):     {:?}", id_of(&plan.guest_exports))?;
    writeln!(wit, "//   copies, virtual side: {:?}", id_of(&plan.virtual_exports))?;
    writeln!(wit, "//   copies, guest side:   {:?}", id_of(&plan.guest_export_copies))?;
    writeln!(wit, "//   types-only copies:    {:?}\n", id_of(&plan.guest_dep_copies))?;
    wit.push_str(&printer.output.to_string());
    std::fs::write(out.join("1-child-world.wit"), wit)?;

    // 2. The options `generate_bindings` adds, as the equivalent
    // `generate!` (inside `pub mod <name>`) and CLI calls.
    let mut opts = String::new();
    let world = r.id_of_name(world_pkg, &r.worlds[plan.world].name);
    let snake = &plan.name.snake;
    let remaps = plan.guest_side_remaps();
    writeln!(opts, "pub mod {snake} {{\n    wit_bindgen::generate!({{")?;
    writeln!(opts, "        path: \"1-child-world.wit\",\n        world: \"{world}\",")?;
    writeln!(opts, "        generate_all,\n        default_bindings_module: \"crate::{snake}\",")?;
    writeln!(opts, "        with: {{")?;
    for (k, v) in remaps.iter() {
        writeln!(opts, "            {k:?}: {v},")?;
    }
    writeln!(opts, "        }},\n    }});\n}}\n\n// or:\nwit-bindgen rust 1-child-world.wit \\")?;
    writeln!(opts, "  --world {world} \\\n  --generate-all \\\n  --default-bindings-module crate::{snake} \\")?;
    for (k, v) in remaps.iter() {
        writeln!(opts, "  --with '{k}={v}' \\")?;
    }
    writeln!(opts, "  --format")?;
    std::fs::write(out.join("2-wit-bindgen.txt"), opts)?;

    // 3. The generated code.
    let output = plan.generate()?;
    std::fs::write(out.join("3-bindings.rs"), &output.source)?;

    // 4. The manifest.
    let m = &output.manifest;
    let mut s = String::new();
    writeln!(s, "section: {}  ({} bytes)", m.section_name(), m.encode().len())?;
    writeln!(s, "name:    {}", m.name)?;
    writeln!(s, "virtualized (guest import -> donut export implementing it):")?;
    for (a, b) in m.virtualized.iter() {
        writeln!(s, "  {a} -> {b}")?;
    }
    writeln!(s, "exports (guest export -> donut import calling it):")?;
    for (a, b) in m.exports.iter() {
        writeln!(s, "  {a} -> {b}")?;
    }
    writeln!(s, "guest_types (types-only copies, dropped from the donut's imports): {:?}", m.guest_types)?;
    writeln!(s, "export_passthrough: {:?}", m.export_passthrough)?;
    let (gr, gw) = wit_parser::decoding::decode_world(&m.guest)?;
    let gpkg = gr.worlds[gw].package.unwrap();
    let mut printer = WitPrinter::default();
    printer.print(&gr, gpkg, &[])?;
    writeln!(s, "\nguest ({} bytes, component-type encoding), decoded:\n", m.guest.len())?;
    s.push_str(&printer.output.to_string());
    std::fs::write(out.join("4-manifest.txt"), s)?;

    eprintln!("wrote {}", out.display());
    Ok(())
}
