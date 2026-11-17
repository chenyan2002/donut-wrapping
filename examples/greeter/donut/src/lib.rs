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

// The donut's own export: route `<lang>/<name>` to a child.
impl exports::example::greeter::handler::Guest for Donut {
    fn handle(path: String) -> String {
        match path.split_once('/') {
            Some(("en", name)) => english::guest::example::greeter::handler::handle(name),
            Some(("fr", name)) => french::guest::example::greeter::handler::handle(name),
            _ => format!("unknown route `{path}`"),
        }
    }
}

// Each child's `config`: a different greeting.
impl english::virt::example::greeter::config::Guest for Donut {
    fn get(key: String) -> Option<String> {
        (key == "greeting").then(|| "Hello".to_string())
    }
}
impl french::virt::example::greeter::config::Guest for Donut {
    fn get(key: String) -> Option<String> {
        (key == "greeting").then(|| "Bonjour".to_string())
    }
}

export!(Donut);
english::export!(Donut);
french::export!(Donut);
