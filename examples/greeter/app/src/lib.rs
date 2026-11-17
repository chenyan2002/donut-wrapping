wit_bindgen::generate!({ path: "../wit", world: "example:greeter/app", generate_all });

use example::greeter::config;

struct App;

impl exports::example::greeter::handler::Guest for App {
    fn handle(name: String) -> String {
        let greeting = config::get("greeting").unwrap_or_else(|| "hello".to_string());
        format!("{greeting}, {name}!")
    }
}

export!(App);
