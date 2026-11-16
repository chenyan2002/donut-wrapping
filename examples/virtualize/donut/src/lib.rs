//! The donut ($HttpServer): virtualizes the guest's `types`, `outgoing`, and
//! downstream `relay`, embeds the guest, and calls its `handler` and `relay`
//! exports from `serve`.

use std::cell::RefCell;

donut::donut!({
    path: "../wit",
    world: "example:virt/donut",
    guest: "example:virt/guest",
    virtualize: ["example:virt/types", "example:virt/outgoing", "example:virt/relay"],
});

use exports::virtual_example::virt::types::{
    Guest as TypesGuest, GuestRequest, GuestResponse, Request, Response, Tagged, Wrapped,
};
use guest::example::virt::handler::{self, Session};

thread_local! {
    static LOG: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
}

fn log(s: impl Into<String>) {
    // Never hold the borrow across a guest call (re-entrance!).
    LOG.with(|l| l.borrow_mut().push(s.into()));
}

pub struct MyRequest {
    path: String,
}

impl GuestRequest for MyRequest {
    fn new(path: String) -> Self {
        log(format!("guest:new-request({path})"));
        MyRequest { path }
    }
    fn path(&self) -> String {
        log(format!("guest:request.path({})", self.path));
        self.path.clone()
    }
}

impl Drop for MyRequest {
    fn drop(&mut self) {
        log(format!("dtor:request({})", self.path));
    }
}

pub struct MyResponse {
    status: u16,
    body: Vec<u8>,
}

impl GuestResponse for MyResponse {
    fn new(status: u16, body: Vec<u8>) -> Self {
        log(format!("guest:new-response({status})"));
        MyResponse { status, body }
    }
    fn status(&self) -> u16 {
        self.status
    }
    fn body(&self) -> Vec<u8> {
        self.body.clone()
    }
}

impl Drop for MyResponse {
    fn drop(&mut self) {
        log(format!("dtor:response({})", self.status));
    }
}

struct Donut;

impl TypesGuest for Donut {
    type Request = MyRequest;
    type Response = MyResponse;
}

impl exports::virtual_example::virt::outgoing::Guest for Donut {
    fn send(r: Request) -> Response {
        let path = r.get::<MyRequest>().path.clone();
        log(format!("guest:send({path})"));
        drop(r);
        Response::new(MyResponse { status: 201, body: format!("hi from {path}").into_bytes() })
    }
    fn inspect(t: Tagged<'_>) -> String {
        // A borrow received inside a record.
        format!("{}#{}", t.req.get::<MyRequest>().path, t.tag)
    }
    fn wrap(status: u16) -> Wrapped {
        Wrapped {
            resp: Response::new(MyResponse { status, body: Vec::new() }),
            note: format!("wrapped{status}"),
        }
    }
}

// The downstream the guest's middleware `relay` forwards to.
impl exports::virtual_example::virt::relay::Guest for Donut {
    fn relay(t: Tagged<'_>) -> String {
        format!("downstream({}#{})", t.req.get::<MyRequest>().path, t.tag)
    }
}

impl Guest for Donut {
    fn serve() -> String {
        log("serve");
        let session = Session::new();
        log(session.describe("session#"));
        let req = Request::new(MyRequest { path: "/hello".into() });
        // Lend one of our own handles to the guest, inside a record; the
        // guest lends it on to our downstream `relay`.
        log(guest::example::virt::relay::relay(&guest::example::virt::types::Tagged {
            req: &req,
            tag: "run".into(),
        }));
        let resp = handler::receive(&session, req);
        {
            let r = resp.get::<MyResponse>();
            log(format!("serve:got({} {})", r.status, String::from_utf8_lossy(&r.body)));
        }
        drop(resp);
        drop(session);
        log(format!("serve:guest-dropped-sessions={}", handler::dropped()));
        LOG.with(|l| l.borrow_mut().drain(..).collect::<Vec<_>>().join("\n"))
    }
}

export!(Donut);
