wit_bindgen::generate!({ path: "../wit", world: "example:greeter/app", generate_all });

use example::greeter::config::Settings;

struct App;

impl exports::example::greeter::greeting::Guest for App {
    fn greet(settings: &Settings, name: String) -> String {
        format!("{}, {name}!", settings.get())
    }
}

export!(App);
