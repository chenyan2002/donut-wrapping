//! The spike's donut (`spike/donut.wat`), written with `child!`: two
//! children `a` and `b` of world `spike:hole/app`, and the donut's own world
//! `spike:hole/donut`. It logs the same events as the hand-written `$main`.

use std::cell::{Cell, RefCell};
use std::fmt::Write;

// The donut's own world.
wit_bindgen::generate!({ path: "../../spike/wit", world: "spike:hole/donut" });

donut::child!({
    name: "a",
    path: "../../spike/wit",
    guest: "spike:hole/app",
    import_passthrough: ["wasi:clocks/*"],
    export_passthrough: ["spike:hole/info"],
});

donut::child!({
    name: "b",
    path: "../../spike/wit",
    guest: "spike:hole/app",
    import_passthrough: ["wasi:clocks/*"],
});

thread_local! {
    static LOG: RefCell<String> = const { RefCell::new(String::new()) };
    static REENTER: Cell<bool> = const { Cell::new(false) };
}

fn emit(tag: &str, n: u32) {
    LOG.with(|log| write!(log.borrow_mut(), "{tag}{n} ").unwrap());
}

/// A virtual `counter`, for either child.
pub struct Counter {
    child: &'static str,
    value: Cell<u32>,
}

impl Counter {
    fn new(child: &'static str, start: u32) -> Counter {
        Counter { child, value: Cell::new(start) }
    }
    fn bump(&self) -> u32 {
        self.value.set(self.value.get() + 1);
        emit(&format!("{}.bump=", self.child), self.value.get());
        self.value.get()
    }
}

impl Drop for Counter {
    fn drop(&mut self) {
        emit(&format!("{}.drop=", self.child), self.value.get());
    }
}

struct Donut;

// ---- the donut's own exports

impl Guest for Donut {
    fn run() -> String {
        call_a();
        LOG.with(|log| log.take())
    }
    fn reenter() -> String {
        REENTER.set(true);
        call_a();
        LOG.with(|log| log.take())
    }
}

fn call_a() {
    use a::guest::spike::hole::handler;
    let c = a::virt::spike::hole::types::Counter::new(Counter::new("a", 0));
    emit("a.run=", handler::run(&c));
    let session = handler::start();
    emit("a.session.id=", session.id());
    drop(session); // the child's dtor calls back: `a.log=305`
    drop(c); // our dtor: `a.drop=1`
}

fn call_b() {
    let c = b::virt::spike::hole::types::Counter::new(Counter::new("b", 20));
    emit("b.run=", b::guest::spike::hole::handler::run(&c));
}

// ---- child a's virtual interfaces

impl a::virt::spike::hole::types::Guest for Donut {
    type Counter = Counter;
}

impl a::virt::spike::hole::types::GuestCounter for Counter {
    fn new(start: u32) -> Counter {
        Counter::new("a", start)
    }
    fn bump(&self) -> u32 {
        Counter::bump(self)
    }
}

impl a::virt::spike::hole::host_api::Guest for Donut {
    fn log(code: u32) {
        emit("a.log=", code);
        if code == 7 {
            if REENTER.get() {
                // call `a` again while it's on the stack
                emit("a.version=", a::guest::spike::hole::info::version());
            } else {
                // call `b` while `a` is on the stack
                call_b();
            }
        }
    }
    fn make(start: u32) -> a::virt::spike::hole::types::Counter {
        a::virt::spike::hole::types::Counter::new(Counter::new("a", start))
    }
}

// ---- child b's virtual interfaces

impl b::virt::spike::hole::types::Guest for Donut {
    type Counter = Counter;
}

impl b::virt::spike::hole::types::GuestCounter for Counter {
    fn new(start: u32) -> Counter {
        Counter::new("b", start)
    }
    fn bump(&self) -> u32 {
        Counter::bump(self)
    }
}

impl b::virt::spike::hole::host_api::Guest for Donut {
    fn log(code: u32) {
        emit("b.log=", code);
        match code {
            // b's start function: call `a`, which is linked already
            500 => emit("a.version=", a::guest::spike::hole::info::version()),
            // b-early's start function: call `b`, which isn't linked yet
            6 => call_b(),
            _ => {}
        }
    }
    fn make(start: u32) -> b::virt::spike::hole::types::Counter {
        b::virt::spike::hole::types::Counter::new(Counter::new("b", start))
    }
}

export!(Donut);
a::export!(Donut);
b::export!(Donut);
