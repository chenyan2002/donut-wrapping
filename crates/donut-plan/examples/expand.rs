//! Prints what `donut!` expands to, formatted.
//!
//! cargo run -p donut-plan --example expand -- <wit-dir> <world> <guest-world> <virtualized>...
//!
//! For forwarding, set `FORWARD` (comma-separated interfaces) and `HOOKS`.
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
        hooks: std::env::var("HOOKS").ok(),
        ..Default::default()
    };
    config.opts.format = true;
    print!("{}", donut_plan::generate(&resolve, &[pkg], &config)?.source);
    Ok(())
}
