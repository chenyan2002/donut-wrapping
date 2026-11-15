// The virtualized `fastly:compute/*` interfaces. Generated from the `donut!`
// expansion as `unimplemented!()` stubs (see ../README.md), then
// the functions this guest calls for one request (proxy-component's recorded
// trace) were filled in.

mod async_io {
    #![allow(unused_variables)]
    use crate::exports::fastly::compute::async_io::*;
    impl Guest for crate::Host {
        type Pollable = crate::rep::Pollable;
    }
    impl GuestPollable for crate::rep::Pollable {
    }
}

mod http_body {
    #![allow(unused_variables)]
    use crate::_rt;
    use crate::exports::fastly::compute::http_body::*;
    impl Guest for crate::Host {
        fn new() -> ::core::result::Result<Body, Error> { crate::log("http-body::new"); Ok(Body::new(crate::rep::Pollable::default())) }
        fn append( dest: BodyBorrow<'_>, src: Body, ) -> ::core::result::Result<(), Error> { unimplemented!("http_body.append") }
        fn write( body: BodyBorrow<'_>, buf: _rt::Vec<u8>, ) -> ::core::result::Result<u32, Error> { crate::log("http-body::write"); let n = buf.len() as u32; body.get::<crate::rep::Pollable>().data.borrow_mut().extend_from_slice(&buf); Ok(n) }
        fn write_front( body: BodyBorrow<'_>, buf: _rt::Vec<u8>, ) -> ::core::result::Result<(), Error> { unimplemented!("http_body.write_front") }
        fn close(body: Body) -> ::core::result::Result<(), Error> { crate::log("http-body::close"); drop(body); Ok(()) }
        fn get_known_length( body: BodyBorrow<'_>, ) -> ::core::option::Option<u64> { unimplemented!("http_body.get_known_length") }
    }
}

mod http_resp {
    #![allow(unused_variables)]
    use crate::_rt;
    use crate::exports::fastly::compute::http_resp::*;
    impl Guest for crate::Host {
        type Response = crate::rep::Response;
        fn send_downstream( response: Response, body: Body, ) -> ::core::result::Result<(), Error> { crate::log("http-resp::send-downstream"); crate::record_downstream(response, body); Ok(()) }
        fn send_downstream_streaming( response: Response, body: BodyBorrow<'_>, ) -> ::core::result::Result<(), Error> { unimplemented!("http_resp.send_downstream_streaming") }
        fn close(response: Response) -> ::core::result::Result<(), Error> { unimplemented!("http_resp.close") }
    }
    impl GuestResponse for crate::rep::Response {
        fn new() -> ::core::result::Result<Response, Error> { crate::log("http-resp::new"); Ok(Response::new(crate::rep::Response::default())) }
        fn get_header_names( &self, max_len: u64, cursor: u32, ) -> ::core::result::Result< (_rt::String, ::core::option::Option<u32>), Error, > { unimplemented!("http_resp.get_header_names") }
        fn get_header_values( &self, name: _rt::String, max_len: u64, cursor: u32, ) -> ::core::result::Result< (_rt::Vec<u8>, ::core::option::Option<u32>), Error, > { unimplemented!("http_resp.get_header_values") }
        fn insert_header( &self, name: _rt::String, value: _rt::Vec<u8>, ) -> ::core::result::Result<(), Error> { unimplemented!("http_resp.insert_header") }
        fn append_header( &self, name: _rt::String, value: _rt::Vec<u8>, ) -> ::core::result::Result<(), Error> { crate::log("http-resp::append-header"); self.headers.borrow_mut().push((name, value)); Ok(()) }
        fn get_version(&self) -> ::core::result::Result<HttpVersion, Error> { unimplemented!("http_resp.get_version") }
        fn set_version( &self, version: HttpVersion, ) -> ::core::result::Result<(), Error> { crate::log("http-resp::set-version"); self.version.replace(format!("{version:?}")); Ok(()) }
        fn get_status(&self) -> ::core::result::Result<HttpStatus, Error> { unimplemented!("http_resp.get_status") }
        fn set_status( &self, status: HttpStatus, ) -> ::core::result::Result<(), Error> { crate::log("http-resp::set-status"); self.status.set(status); Ok(()) }
        fn set_framing_headers_mode( &self, mode: FramingHeadersMode, ) -> ::core::result::Result<(), Error> { crate::log("http-resp::set-framing-headers-mode"); crate::log(format!("framing-headers-mode {mode:?}")); Ok(()) }
        fn set_http_keepalive_mode( &self, mode: KeepaliveMode, ) -> ::core::result::Result<(), Error> { crate::log("http-resp::set-http-keepalive-mode"); crate::log(format!("keepalive-mode {mode:?}")); Ok(()) }
        fn get_remote_ip_addr(&self) -> ::core::option::Option<IpAddress> { unimplemented!("http_resp.get_remote_ip_addr") }
        fn get_remote_port(&self) -> ::core::option::Option<u16> { unimplemented!("http_resp.get_remote_port") }
    }
}

mod http_req {
    #![allow(unused_variables)]
    use crate::_rt;
    use crate::exports::fastly::compute::http_req::*;
    impl Guest for crate::Host {
        type Request = crate::rep::Request;
        type ExtraSendErrorDetail = crate::rep::ExtraSendErrorDetail;
        fn await_response( pending: PendingResponse, ) -> ::core::result::Result<ResponseWithBody, ErrorWithDetail> { unimplemented!("http_req.await_response") }
        fn close(request: Request) -> ::core::result::Result<(), Error> { crate::log("http-req::close"); drop(request); Ok(()) }
    }
    impl GuestRequest for crate::rep::Request {
        fn get_method( &self, max_len: u64, ) -> ::core::result::Result<_rt::String, Error> { crate::log("http-req::get-method"); Ok(self.method.clone()) }
        fn set_method( &self, method: _rt::String, ) -> ::core::result::Result<(), Error> { unimplemented!("http_req.set_method") }
        fn get_uri( &self, max_len: u64, ) -> ::core::result::Result<_rt::String, Error> { crate::log("http-req::get-uri"); Ok(self.uri.clone()) }
        fn set_uri( &self, uri: _rt::String, ) -> ::core::result::Result<(), Error> { unimplemented!("http_req.set_uri") }
        fn get_version(&self) -> ::core::result::Result<HttpVersion, Error> { unimplemented!("http_req.get_version") }
        fn set_version( &self, version: HttpVersion, ) -> ::core::result::Result<(), Error> { unimplemented!("http_req.set_version") }
    }
    impl GuestExtraSendErrorDetail for crate::rep::ExtraSendErrorDetail {
    }
}

mod http_cache {
    #![allow(unused_variables)]
    use crate::_rt;
    use crate::exports::fastly::compute::http_cache::*;
    impl Guest for crate::Host {
        type Entry = crate::rep::Entry;
        type ExtraWriteOptions = crate::rep::ExtraWriteOptions;
        type SuggestedWriteOptions = crate::rep::SuggestedWriteOptions;
        fn close_entry(handle: Entry) -> ::core::result::Result<(), Error> { unimplemented!("http_cache.close_entry") }
    }
    impl GuestEntry for crate::rep::Entry {
        fn transaction_insert( &self, resp_handle: Response, options: WriteOptions<'_>, ) -> ::core::result::Result<Body, Error> { unimplemented!("http_cache.transaction_insert") }
        fn transaction_update( &self, resp_handle: Response, options: WriteOptions<'_>, ) -> ::core::result::Result<(), Error> { unimplemented!("http_cache.transaction_update") }
        fn transaction_record_not_cacheable( &self, options: WriteOptions<'_>, ) -> ::core::result::Result<(), Error> { unimplemented!("http_cache.transaction_record_not_cacheable") }
        fn get_suggested_write_options( &self, response: ResponseBorrow<'_>, ) -> ::core::result::Result<SuggestedWriteOptions, Error> { unimplemented!("http_cache.get_suggested_write_options") }
        fn prepare_response_for_storage( &self, response: ResponseBorrow<'_>, ) -> ::core::result::Result<(StorageAction, Response), Error> { unimplemented!("http_cache.prepare_response_for_storage") }
        fn transaction_abandon(&self) -> ::core::result::Result<(), Error> { unimplemented!("http_cache.transaction_abandon") }
    }
    impl GuestExtraWriteOptions for crate::rep::ExtraWriteOptions {
    }
    impl GuestSuggestedWriteOptions for crate::rep::SuggestedWriteOptions {
        fn get_max_age_ns(&self) -> DurationNs { unimplemented!("http_cache.get_max_age_ns") }
        fn get_vary_rule( &self, max_len: u64, ) -> ::core::result::Result<_rt::String, Error> { unimplemented!("http_cache.get_vary_rule") }
        fn get_initial_age_ns(&self) -> DurationNs { unimplemented!("http_cache.get_initial_age_ns") }
        fn get_stale_while_revalidate_ns(&self) -> DurationNs { unimplemented!("http_cache.get_stale_while_revalidate_ns") }
        fn get_surrogate_keys( &self, max_len: u64, ) -> ::core::result::Result<_rt::String, Error> { unimplemented!("http_cache.get_surrogate_keys") }
        fn get_length(&self) -> ::core::option::Option<ObjectLength> { unimplemented!("http_cache.get_length") }
        fn get_sensitive_data(&self) -> bool { unimplemented!("http_cache.get_sensitive_data") }
    }
}

mod http_downstream {
    #![allow(unused_variables)]
    use crate::_rt;
    use crate::exports::fastly::compute::http_downstream::*;
    impl Guest for crate::Host {
        fn downstream_client_ip_addr( ds_request: RequestBorrow<'_>, ) -> ::core::option::Option<IpAddress> { unimplemented!("http_downstream.downstream_client_ip_addr") }
        fn downstream_server_ip_addr( ds_request: RequestBorrow<'_>, ) -> ::core::option::Option<IpAddress> { unimplemented!("http_downstream.downstream_server_ip_addr") }
        fn downstream_client_h2_fingerprint( ds_request: RequestBorrow<'_>, max_len: u64, ) -> ::core::result::Result<_rt::String, Error> { unimplemented!("http_downstream.downstream_client_h2_fingerprint") }
        fn downstream_client_request_id( ds_request: RequestBorrow<'_>, max_len: u64, ) -> ::core::result::Result<_rt::String, Error> { unimplemented!("http_downstream.downstream_client_request_id") }
        fn downstream_client_oh_fingerprint( ds_request: RequestBorrow<'_>, max_len: u64, ) -> ::core::result::Result<_rt::String, Error> { unimplemented!("http_downstream.downstream_client_oh_fingerprint") }
        fn downstream_client_ddos_detected( ds_request: RequestBorrow<'_>, ) -> ::core::result::Result<bool, Error> { unimplemented!("http_downstream.downstream_client_ddos_detected") }
        fn downstream_tls_cipher_openssl_name( ds_request: RequestBorrow<'_>, max_len: u64, ) -> ::core::result::Result< ::core::option::Option<_rt::Vec<u8>>, Error, > { unimplemented!("http_downstream.downstream_tls_cipher_openssl_name") }
        fn downstream_tls_protocol( ds_request: RequestBorrow<'_>, max_len: u64, ) -> ::core::result::Result< ::core::option::Option<_rt::Vec<u8>>, Error, > { unimplemented!("http_downstream.downstream_tls_protocol") }
        fn downstream_tls_client_hello( ds_request: RequestBorrow<'_>, max_len: u64, ) -> ::core::result::Result< ::core::option::Option<_rt::Vec<u8>>, Error, > { unimplemented!("http_downstream.downstream_tls_client_hello") }
        fn downstream_tls_raw_client_certificate( ds_request: RequestBorrow<'_>, max_len: u64, ) -> ::core::result::Result< ::core::option::Option<_rt::Vec<u8>>, Error, > { unimplemented!("http_downstream.downstream_tls_raw_client_certificate") }
        fn downstream_tls_client_cert_verify_result( ds_request: RequestBorrow<'_>, ) -> ::core::result::Result< ::core::option::Option<ClientCertVerifyResult>, Error, > { unimplemented!("http_downstream.downstream_tls_client_cert_verify_result") }
        fn downstream_tls_client_servername( ds_request: RequestBorrow<'_>, max_len: u64, ) -> ::core::result::Result< ::core::option::Option<_rt::String>, Error, > { unimplemented!("http_downstream.downstream_tls_client_servername") }
        fn downstream_tls_ja3_md5( ds_request: RequestBorrow<'_>, ) -> ::core::result::Result< ::core::option::Option<_rt::Vec<u8>>, Error, > { unimplemented!("http_downstream.downstream_tls_ja3_md5") }
        fn downstream_tls_ja4( ds_request: RequestBorrow<'_>, max_len: u64, ) -> ::core::result::Result< ::core::option::Option<_rt::String>, Error, > { unimplemented!("http_downstream.downstream_tls_ja4") }
        fn downstream_compliance_region( ds_request: RequestBorrow<'_>, max_len: u64, ) -> ::core::result::Result< ::core::option::Option<_rt::String>, Error, > { unimplemented!("http_downstream.downstream_compliance_region") }
        fn fastly_key_is_valid( ds_request: RequestBorrow<'_>, ) -> ::core::result::Result<bool, Error> { unimplemented!("http_downstream.fastly_key_is_valid") }
    }
}

