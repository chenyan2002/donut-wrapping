//! Prints what `donut!` expands to, formatted.
//!
//! cargo run -p donut-plan --example expand -- <wit-dir> <world> <guest-world> <virtualized>...
//!
//! For generated impls, set `FORWARD` and/or `MOCK` (comma-separated
//! interfaces) and `HOOKS`, and for mock state types `MOCK_TYPES`
//! (`<interface>#<resource>=<type>;...`).
use anyhow::{Result, bail};

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [wit, world, guest, virtualize @ ..] = &args[..] else {
        bail!("usage: expand <wit-dir> <world> <guest-world> <virtualized>...");
    };
    let mut resolve = wit_parser::Resolve::default();
    let (pkg, _) = resolve.push_path(wit)?;
    let mut config = donut_plan::Config {
        world: world.clone(),
        guest: guest.clone(),
        virtualize: virtualize.to_vec(),
        forward: std::env::var("FORWARD")
            .map(|f| f.split(',').map(String::from).collect())
            .unwrap_or_default(),
        mock: std::env::var("MOCK")
            .map(|f| f.split(',').map(String::from).collect())
            .unwrap_or_default(),
        mock_types: std::env::var("MOCK_TYPES")
            .map(|t| {
                let mut types = std::collections::BTreeMap::<String, std::collections::BTreeMap<String, String>>::new();
                for entry in t.split(';').filter(|e| !e.is_empty()) {
                    let (key, ty) = entry.split_once('=').expect("`<interface>#<resource>=<type>`");
                    let (iface, res) = key.split_once('#').expect("`<interface>#<resource>=<type>`");
                    types.entry(iface.into()).or_default().insert(res.into(), ty.into());
                }
                types
            })
            .unwrap_or_default(),
        hooks: std::env::var("HOOKS").ok(),
        ..Default::default()
    };
    config.opts.format = true;
    print!("{}", donut_plan::generate(&resolve, &[pkg], &config)?.source);
    Ok(())
}
