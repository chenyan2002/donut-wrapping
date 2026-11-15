//! The guest: an ordinary wasip2 component. It doesn't know that `outgoing`
//! and its downstream `relay` are virtualized by the component embedding it.

use std::cell::Cell;

wit_bindgen::generate!({ path: "../wit", world: "guest", generate_all });

use example::virt::types::{Request, Response, Tagged};
use example::virt::{outgoing, relay as downstream};
use exports::example::virt::handler::{Guest as Handler, GuestSession, SessionBorrow};
use exports::example::virt::relay::Guest as Relay;
use wasi::clocks::monotonic_clock;

thread_local! {
    static NEXT_ID: Cell<u32> = const { Cell::new(1) };
    static DROPPED: Cell<u32> = const { Cell::new(0) };
}

struct Component;

pub struct Session {
    id: u32,
}

impl GuestSession for Session {
    fn new() -> Self {
        let id = NEXT_ID.get();
        NEXT_ID.set(id + 1);
        Session { id }
    }
    fn describe(&self, prefix: String) -> String {
        format!("{prefix}{}", self.id)
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        DROPPED.set(DROPPED.get() + 1);
    }
}

impl Handler for Component {
    type Session = Session;

    fn receive(s: SessionBorrow<'_>, r: Request) -> Response {
        let id = s.get::<Session>().id;
        // Goes through the guest's own `std` WASI imports (wasi:cli/stderr,
        // wasi:io/streams), which aren't in its world: the donut passes them
        // through from the host.
        eprintln!("guest: receive in session {id}");
        // A virtual method returning a string.
        let path = r.path();
        // A virtual constructor, and a re-entrant call back into the donut.
        let upstream = outgoing::send(Request::new(&format!("/upstream{path}")));
        let status = upstream.status();
        let upstream_body = String::from_utf8(upstream.body()).unwrap();
        drop(upstream); // runs the donut's response dtor, re-entrantly
        // Lend the donut's own request back to it, inside a record.
        let inspected = outgoing::inspect(&Tagged { req: &r, tag: "from-guest".into() });
        // An owned handle comes back inside a record; dropping it runs the
        // donut's destructor.
        let w = outgoing::wrap(7);
        let wrapped = format!("{} {}", w.resp.status(), w.note);
        drop(w);
        // A passthrough host import.
        let clock = if monotonic_clock::now() > 0 { "ok" } else { "zero" };
        let body = format!(
            "session={id} path={path} upstream=({status} {upstream_body}) \
             inspected={inspected} wrapped=({wrapped}) clock={clock}"
        );
        drop(r); // runs the donut's request dtor, re-entrantly
        Response::new(200, body.as_bytes())
    }

    fn dropped() -> u32 {
        DROPPED.get()
    }
}

// A middleware: forwards to the donut's downstream `relay`, lending on the
// borrow it was given.
impl Relay for Component {
    fn relay(t: Tagged<'_>) -> String {
        let down = downstream::relay(&Tagged { req: t.req, tag: format!("{}+mw", t.tag) });
        format!("relay(path={}, downstream={down})", t.req.path())
    }
}

export!(Component);
