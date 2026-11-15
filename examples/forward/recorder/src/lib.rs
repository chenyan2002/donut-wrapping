//! A recording donut (proxy-component's record mode as one component): same
//! world as the app it embeds, with all forwarding generated (`forward:`). The
//! only hand-written code is the logging in `Hooks`. The hand-written
//! equivalent is `spike/recorder`.

use core::fmt::Debug;

donut::donut!({
    path: "../wit",
    world: "example:rec/app-world",
    guest: "example:rec/app-world",
    forward: ["example:rec/kv", "example:rec/app"],
    hooks: Recorder,
});

pub struct Recorder;

impl forward::Hooks for Recorder {
    fn before(call: &forward::Call, args: &[&dyn Debug]) {
        println!("record: {:?} {}.{}{args:?}", call.direction, call.interface, call.function);
    }
    fn after(call: &forward::Call, ret: &dyn Debug) {
        println!("record: {:?} {}.{} -> {ret:?}", call.direction, call.interface, call.function);
    }
    fn dropped(direction: forward::Direction, interface: &str, resource: &str) {
        println!("record: {direction:?} {interface}.{resource} dropped");
    }
}

export!(Recorder);
