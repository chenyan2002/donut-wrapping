//! The app being recorded: an ordinary component.
wit_bindgen::generate!({ path: "../wit", world: "app-world" });

use example::rec::kv::{self, Store};
use exports::example::rec::app::{Guest, GuestSession};

struct App;

pub struct Session(String);

impl GuestSession for Session {
    fn new(user: String) -> Self {
        Session(user)
    }
    fn greet(&self, s: &Store) -> String {
        format!("hello {} (visits={:?})", self.0, s.get("visits"))
    }
}

impl Guest for App {
    type Session = Session;
    fn run(s: &Store) -> String {
        s.set("visits", "1");
        let cache = kv::open("cache");
        cache.set("k", "v");
        format!("visits={:?} cache={:?}", s.get("visits"), cache.get("k"))
    }
}

export!(App);
