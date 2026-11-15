//! Builds each example into a donut component and runs it on Wasmtime.

use donut_tests::{build, call};

fn lines(s: &str) -> Vec<&str> {
    s.lines().collect()
}

/// `examples/http`: re-entrance (`serve → receive → send`), exactly-once
/// destructors of virtual and guest-defined resources, strings and lists, and
/// the guest's clock passthrough.
#[test]
fn http_wasip2() -> anyhow::Result<()> {
    let dir = build("http")?;
    let run = call::<(), (String,)>(&dir.join("out/donut.wasm"), "serve", ())?;
    let expected = std::fs::read_to_string(dir.join("expected.txt"))?;
    assert_eq!(lines(&run.result.0), lines(&expected));
    // The guest's own `std` WASI imports (not in its world) are passed through.
    assert_eq!(run.stderr, "guest: receive in session 1\n");
    Ok(())
}

#[test]
fn http_unknown_unknown() -> anyhow::Result<()> {
    let dir = build("http")?;
    let run = call::<(), (String,)>(&dir.join("out/donut-unknown.wasm"), "serve", ())?;
    let expected = std::fs::read_to_string(dir.join("expected.txt"))?;
    assert_eq!(lines(&run.result.0), lines(&expected));
    Ok(())
}

/// `examples/shapes`: a borrow inside a record in both directions, an owned
/// handle inside a returned record (its destructor runs when the guest drops
/// it), and `handler` imported and exported by the guest (a middleware).
#[test]
fn shapes_records_and_middleware() -> anyhow::Result<()> {
    let dir = build("shapes")?;
    let run = call::<(), (String,)>(&dir.join("out/donut.wasm"), "run", ())?;
    assert_eq!(
        lines(&run.result.0),
        [
            "dtor:token(wrapped7)",
            "guest(name=t1, inspected=t1#from-guest, wrapped=wrapped7:7, \
             downstream=downstream(t1#run+mw))",
            "dtor:token(t1)",
        ]
    );
    Ok(())
}

/// The guest's core start function calls a virtual function while the guest
/// is instantiated: the donut's first fixup phase must be done by then.
#[test]
fn guest_start_calls_virtual_function() -> anyhow::Result<()> {
    let dir = build("shapes")?;
    let run = call::<(), (u32,)>(&dir.join("out/init-tick.wasm"), "run", ())?;
    assert_eq!(run.result.0, 1);
    Ok(())
}

/// ... but a virtual function that calls back into the guest during that
/// start can't work, since the guest's exports aren't linked yet. It must trap
/// cleanly (design §8, "Calls before linking").
#[test]
fn guest_start_calling_back_into_guest_traps() -> anyhow::Result<()> {
    let dir = build("shapes")?;
    let err = match call::<(), (u32,)>(&dir.join("out/init-tick-reentrant.wasm"), "run", ()) {
        Ok(_) => panic!("expected instantiation to trap"),
        Err(e) => format!("{e:?}"),
    };
    assert!(err.contains("uninitialized element"), "{err}");
    assert!(err.contains("guest-export-ticks"), "{err}");
    Ok(())
}

/// `examples/fastly`: a mock Fastly Compute host around proxy-component's
/// real Fastly guest (`tests/rust.wasm`), one request per instance. Skipped if
/// the guest isn't there (set `GUEST`).
#[test]
fn fastly_guest() -> anyhow::Result<()> {
    let guest = std::env::var("GUEST").unwrap_or_else(|_| {
        format!("{}/src/proxy-component/tests/rust.wasm", std::env::var("HOME").unwrap())
    });
    if !std::path::Path::new(&guest).exists() {
        eprintln!("skipping: Fastly guest {guest} not found");
        return Ok(());
    }
    let dir = build("fastly")?;
    let donut = dir.join("out/donut.wasm");

    // The same host calls, in the same order, as proxy-component's recorded
    // run of this guest.
    let calls = [
        "http-req::get-method",
        "http-req::get-uri",
        "http-body::new",
        "http-body::write",
        "http-req::close",
        "http-body::close",
        "http-resp::new",
        "http-resp::set-status",
        "http-resp::set-version",
        "http-resp::append-header",
        "http-resp::set-framing-headers-mode",
        "http-resp::set-http-keepalive-mode",
        "http-resp::send-downstream",
    ];
    for (path, status, body) in [
        ("/", 200, "downstream: body 668 bytes, first line \"<!DOCTYPE html>\""),
        (
            "/missing",
            404,
            "downstream: body 42 bytes, first line \"The page you requested could not be found\"",
        ),
    ] {
        let uri = format!("http://localhost:7676{path}");
        let run = call::<(String,), (String,)>(&donut, "serve", (uri.clone(),))?;
        let log = run.result.0;
        let made: Vec<&str> = log.lines().filter(|l| l.starts_with("http-")).collect();
        assert_eq!(made, calls, "{log}");
        assert!(log.contains(&format!("downstream: status {status}")), "{log}");
        assert!(log.contains(body), "{log}");
        assert!(log.ends_with("handle: Ok(())"), "{log}");
        // The guest's stdout goes through its WASI passthrough.
        assert!(run.stdout.contains("FASTLY_SERVICE_VERSION"), "{}", run.stdout);
    }
    Ok(())
}
