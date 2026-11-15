//! A mock Fastly Compute host as a donut: it virtualizes `fastly:compute/*`,
//! passes WASI through, and runs one request through the embedded guest
//! (proxy-component's `tests/rust.wasm`), reporting what the guest did.

use std::cell::{Cell, RefCell};

donut::donut!({
    path: "../wit",
    world: "test:fastly-host/host",
    guest: "root:component/root",
    virtualize: [
        "fastly:compute/types@0.1.0",
        "fastly:compute/async-io@0.1.0",
        "fastly:compute/http-body@0.1.0",
        "fastly:compute/http-types@0.1.0",
        "fastly:compute/http-resp@0.1.0",
        "fastly:compute/http-req@0.1.0",
        "fastly:compute/cache@0.1.0",
        "fastly:compute/http-cache@0.1.0",
        "fastly:compute/http-downstream@0.1.0",
    ],
});

mod fastly;

use exports::fastly::compute::async_io::Pollable as Body;
use exports::fastly::compute::http_req::Request;
use exports::fastly::compute::http_resp::Response;

/// Resource representations of the virtual `fastly:compute` resources.
pub mod rep {
    use super::*;

    /// `async-io.pollable`, which `http-body` uses as `body`.
    #[derive(Default)]
    pub struct Pollable {
        pub data: RefCell<Vec<u8>>,
    }

    pub struct Request {
        pub method: String,
        pub uri: String,
    }

    #[derive(Default)]
    pub struct Response {
        pub status: Cell<u16>,
        pub version: RefCell<String>,
        pub headers: RefCell<Vec<(String, Vec<u8>)>>,
    }

    pub struct ExtraSendErrorDetail;
    pub struct Entry;
    pub struct ExtraWriteOptions;
    pub struct SuggestedWriteOptions;
}

pub struct Host;

thread_local! {
    static LOG: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
}

pub fn log(s: impl Into<String>) {
    LOG.with(|l| l.borrow_mut().push(s.into()));
}

/// `http-resp.send-downstream`: record the response the guest produced.
pub fn record_downstream(response: Response, body: Body) {
    let r = response.get::<rep::Response>();
    log(format!("downstream: status {}", r.status.get()));
    log(format!("downstream: version {}", r.version.borrow()));
    for (name, value) in r.headers.borrow().iter() {
        log(format!("downstream: header {name}: {}", String::from_utf8_lossy(value)));
    }
    let data = body.get::<rep::Pollable>().data.borrow().clone();
    let text = String::from_utf8_lossy(&data);
    log(format!(
        "downstream: body {} bytes, first line {:?}",
        data.len(),
        text.lines().next().unwrap_or("")
    ));
}

impl Guest for Host {
    fn serve(uri: String) -> String {
        log(format!("request: GET {uri}"));
        let request = Request::new(rep::Request {
            method: "GET".into(),
            uri,
        });
        let body = Body::new(rep::Pollable::default());
        let result = guest::fastly::compute::http_incoming::handle(request, body);
        log(format!("handle: {result:?}"));
        LOG.with(|l| l.borrow_mut().drain(..).collect::<Vec<_>>().join("\n"))
    }
}

export!(Host);
