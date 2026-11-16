//! The app: an ordinary component, written for a platform with a filesystem.
//! It knows nothing about donuts, vaults, or tokens.
wit_bindgen::generate!({ path: "../wit", world: "app" });

use example::app::{fs, outgoing, types::Request};

struct App;

impl exports::example::app::handler::Guest for App {
    fn handle(path: String) -> String {
        let conf = fs::open("/etc/app.conf").map(|f| f.read()).unwrap_or_default();
        let upstream = conf.trim().strip_prefix("upstream=").unwrap_or("https://example.com");
        println!("app: {path} -> {upstream}");
        // It also phones home.
        if let Err(e) = outgoing::send(Request::new("https://telemetry.example/ping")) {
            println!("app: telemetry failed: {e}");
        }
        match outgoing::send(Request::new(&format!("{upstream}{path}"))) {
            Ok(status) => format!("status {status}"),
            Err(e) => format!("error: {e}"),
        }
    }
}

export!(App);
