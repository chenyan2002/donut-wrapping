// The donut's own world.
wit_bindgen::generate!({ path: "../wit", world: "example:greeter/host" });

// Two children of the same world. Everything they import is virtualized,
// except WASI, which goes straight to the host.
donut::child!({
    name: "english",
    path: "../wit",
    guest: "example:greeter/app",
    import_passthrough: ["wasi:*"],
});
donut::child!({
    name: "french",
    path: "../wit",
    guest: "example:greeter/app",
    import_passthrough: ["wasi:*"],
});

struct Donut;

// The donut's own export: route `<lang>/<name>` to a child, with settings
// in its own `config`.
impl exports::example::greeter::handler::Guest for Donut {
    fn handle(path: String) -> String {
        match path.split_once('/') {
            Some(("en", name)) => {
                let settings = english::virt::example::greeter::config::Settings::new(Settings);
                english::guest::example::greeter::greeting::greet(&settings, name)
            }
            Some(("fr", name)) => {
                let settings = french::virt::example::greeter::config::Settings::new(Settings);
                french::guest::example::greeter::greeting::greet(&settings, name)
            }
            _ => format!("unknown route `{path}`"),
        }
    }
}

/// A child's `settings`: one Rust type behind both children's resources.
struct Settings;

// Each child's `config`, with its own greeting.
impl english::virt::example::greeter::config::Guest for Donut {
    type Settings = Settings;
}
impl english::virt::example::greeter::config::GuestSettings for Settings {
    fn get(&self) -> String {
        "Hello".to_string()
    }
}
impl french::virt::example::greeter::config::Guest for Donut {
    type Settings = Settings;
}
impl french::virt::example::greeter::config::GuestSettings for Settings {
    fn get(&self) -> String {
        "Bonjour".to_string()
    }
}

export!(Donut);
english::export!(Donut);
french::export!(Donut);
