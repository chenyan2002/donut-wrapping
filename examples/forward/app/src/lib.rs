//! The app being forwarded around: an ordinary component.
wit_bindgen::generate!({ path: "../wit", world: "app-world" });

use example::rec::kv::{self, Lookup, Store, View};
use exports::example::rec::app::{Guest, GuestSession, Report};

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

fn found(name: &str) -> String {
    match kv::find(name) {
        Lookup::Found(st) => {
            st.set("seen", "1");
            format!("found:{:?}", st.get("seen"))
        }
        Lookup::Missing => "missing".into(),
    }
}

impl Guest for App {
    type Session = Session;
    fn run(s: &Store) -> String {
        s.set("visits", "1");
        let cache = kv::open("cache");
        cache.set("k", "v");
        let all = kv::open_all(&["a".to_string(), "b".to_string()]);
        for (i, st) in all.iter().enumerate() {
            st.set("i", &i.to_string());
        }
        let pair = kv::open_pair("p1", "p2");
        pair.first.set("side", "first");
        let dump = kv::dump(&View { s, prefix: "vi".into() });
        format!(
            "visits={:?} cache={:?} all={:?} pair={:?} {} {} dump={dump:?}",
            s.get("visits"),
            cache.get("k"),
            all.iter().map(|st| st.get("i")).collect::<Vec<_>>(),
            pair.first.get("side"),
            found("xyz"),
            found("nope"),
        )
    }
    fn summarize(v: View<'_>) -> Report {
        let lines = kv::dump(&View { s: v.s, prefix: v.prefix.clone() })
            .into_iter()
            .map(|e| format!("{}={}", e.key, e.value))
            .collect();
        let kept = kv::open_pair("k1", "k2");
        kept.first.set("from", "summarize");
        Report { lines, kept }
    }
    fn tally(first: &Store, extra: Option<&Store>) -> u32 {
        kv::count(&[first], extra)
    }
}

export!(App);
