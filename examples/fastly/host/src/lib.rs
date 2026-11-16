//! A mock Fastly Compute host as a donut: it mocks `fastly:compute/*`, passes
//! WASI through, and runs one request through the embedded guest
//! (proxy-component's `tests/rust.wasm`), reporting what the guest did.
//!
//! `mock:` generates the 60 virtual functions, each defaulting to
//! `unimplemented!()`. We override the 13 this guest calls for a request (from
//! proxy-component's recorded trace), and log every call with `before`.

use core::fmt::Debug;
use std::cell::{Cell, RefCell};

donut::donut!({
    path: "../wit",
    world: "test:fastly-host/host",
    guest: "root:component/root",
    mock: [
        "fastly:compute/types@0.1.0",
        "fastly:compute/http-body@0.1.0",
        "fastly:compute/http-types@0.1.0",
        // The resources the guest creates, and the state each holds.
        {
            "fastly:compute/async-io@0.1.0": { pollable: BodyState },
            "fastly:compute/http-req@0.1.0": { request: RequestState },
            "fastly:compute/http-resp@0.1.0": { response: ResponseState },
        },
        "fastly:compute/cache@0.1.0",
        "fastly:compute/http-cache@0.1.0",
        "fastly:compute/http-downstream@0.1.0",
    ],
    hooks: Host,
});

use exports::virtual_fastly::compute::async_io::{Pollable as Body, PollableBorrow as BodyBorrow};
use exports::virtual_fastly::compute::http_resp::{
    FramingHeadersMode, HttpStatus, HttpVersion, KeepaliveMode, Response,
};
use exports::virtual_fastly::compute::{http_body, http_req, http_resp};
use forward::{MockFastlyComputeHttpReqRequest, MockFastlyComputeHttpRespResponse};

/// A body (`http-body` uses `async-io.pollable` as `body`).
#[derive(Default)]
pub struct BodyState {
    data: RefCell<Vec<u8>>,
}

pub struct RequestState {
    method: String,
    uri: String,
}

#[derive(Default)]
pub struct ResponseState {
    status: Cell<u16>,
    version: RefCell<String>,
    headers: RefCell<Vec<(String, Vec<u8>)>>,
}

pub struct Host;

thread_local! {
    static LOG: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
}

fn log(s: impl Into<String>) {
    LOG.with(|l| l.borrow_mut().push(s.into()));
}

impl forward::Hooks for Host {
    fn before(call: &forward::Call, _args: &[&dyn Debug]) {
        // e.g. `[method]request.get-uri` in `fastly:compute/http-req@0.1.0`
        // logs as `http-req::get-uri`.
        let iface = call.interface.split('/').nth(1).unwrap().split('@').next().unwrap();
        let func = call.function.rsplit(['.', ']']).next().unwrap();
        log(format!("{iface}::{func}"));
    }

    fn fastly_compute_http_body_new() -> Result<Body, http_body::Error> {
        Ok(Body::new(forward::MockFastlyComputeAsyncIoPollable::new(BodyState::default())))
    }
    fn fastly_compute_http_body_write(body: BodyBorrow<'_>, buf: Vec<u8>) -> Result<u32, http_body::Error> {
        let state = body.get::<forward::MockFastlyComputeAsyncIoPollable>();
        state.data.borrow_mut().extend_from_slice(&buf);
        Ok(buf.len() as u32)
    }
    fn fastly_compute_http_body_close(_body: Body) -> Result<(), http_body::Error> {
        Ok(())
    }

    fn fastly_compute_http_req_close(_request: http_req::Request) -> Result<(), http_req::Error> {
        Ok(())
    }
    fn fastly_compute_http_req_request_get_method(this: &MockFastlyComputeHttpReqRequest, _max_len: u64) -> Result<String, http_req::Error> {
        Ok(this.method.clone())
    }
    fn fastly_compute_http_req_request_get_uri(this: &MockFastlyComputeHttpReqRequest, _max_len: u64) -> Result<String, http_req::Error> {
        Ok(this.uri.clone())
    }

    fn fastly_compute_http_resp_response_new() -> Result<Response, http_resp::Error> {
        Ok(Response::new(MockFastlyComputeHttpRespResponse::new(ResponseState::default())))
    }
    fn fastly_compute_http_resp_response_set_status(this: &MockFastlyComputeHttpRespResponse, status: HttpStatus) -> Result<(), http_resp::Error> {
        this.status.set(status);
        Ok(())
    }
    fn fastly_compute_http_resp_response_set_version(this: &MockFastlyComputeHttpRespResponse, version: HttpVersion) -> Result<(), http_resp::Error> {
        this.version.replace(format!("{version:?}"));
        Ok(())
    }
    fn fastly_compute_http_resp_response_append_header(this: &MockFastlyComputeHttpRespResponse, name: String, value: Vec<u8>) -> Result<(), http_resp::Error> {
        this.headers.borrow_mut().push((name, value));
        Ok(())
    }
    fn fastly_compute_http_resp_response_set_framing_headers_mode(_this: &MockFastlyComputeHttpRespResponse, mode: FramingHeadersMode) -> Result<(), http_resp::Error> {
        log(format!("framing-headers-mode {mode:?}"));
        Ok(())
    }
    fn fastly_compute_http_resp_response_set_http_keepalive_mode(_this: &MockFastlyComputeHttpRespResponse, mode: KeepaliveMode) -> Result<(), http_resp::Error> {
        log(format!("keepalive-mode {mode:?}"));
        Ok(())
    }
    fn fastly_compute_http_resp_send_downstream(response: Response, body: Body) -> Result<(), http_resp::Error> {
        let r = response.get::<MockFastlyComputeHttpRespResponse>();
        log(format!("downstream: status {}", r.status.get()));
        log(format!("downstream: version {}", r.version.borrow()));
        for (name, value) in r.headers.borrow().iter() {
            log(format!("downstream: header {name}: {}", String::from_utf8_lossy(value)));
        }
        let data = body.get::<forward::MockFastlyComputeAsyncIoPollable>().data.borrow().clone();
        let text = String::from_utf8_lossy(&data);
        log(format!(
            "downstream: body {} bytes, first line {:?}",
            data.len(),
            text.lines().next().unwrap_or("")
        ));
        Ok(())
    }
}

impl Guest for Host {
    fn serve(uri: String) -> String {
        log(format!("request: GET {uri}"));
        let request = http_req::Request::new(MockFastlyComputeHttpReqRequest::new(RequestState {
            method: "GET".into(),
            uri,
        }));
        let body = Body::new(forward::MockFastlyComputeAsyncIoPollable::new(BodyState::default()));
        let result = guest::fastly::compute::http_incoming::handle(request, body);
        log(format!("handle: {result:?}"));
        LOG.with(|l| l.borrow_mut().drain(..).collect::<Vec<_>>().join("\n"))
    }
}

export!(Host);
