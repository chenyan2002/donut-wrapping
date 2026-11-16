//! Embeds a guest whose start function calls into us during instantiation.
use std::cell::Cell;

donut::donut!({
    path: "../../wit",
    world: "example:virt/init-donut",
    guest: "example:virt/init-guest",
    virtualize: ["example:virt/init-host"],
});

thread_local! {
    static TICKS: Cell<u32> = const { Cell::new(0) };
}

struct Donut;

impl exports::virtual_example::virt::init_host::Guest for Donut {
    fn tick() -> u32 {
        TICKS.set(TICKS.get() + 1);
        TICKS.get()
    }
    fn tick_reentrant() -> u32 {
        // Calls the guest before its exports are linked: must trap.
        guest::example::virt::probe::ticks()
    }
}

impl Guest for Donut {
    fn run() -> u32 {
        // How many times the guest's start function reached us, as the guest
        // saw it.
        guest::example::virt::probe::ticks()
    }
}

export!(Donut);
