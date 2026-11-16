//! Generated forwarding with two functions overridden: the app can't see the
//! `visits` key, and `session.greet` gets decorated. Everything else forwards.

use core::fmt::Debug;

donut::donut!({
    path: "../wit",
    world: "example:rec/app-world",
    guest: "example:rec/app-world",
    forward: ["example:rec/kv", "example:rec/app"],
    hooks: Interceptor,
});

pub struct Interceptor;

impl forward::Hooks for Interceptor {
    fn after(call: &forward::Call, ret: &dyn Debug) {
        println!("{:?} {} -> {ret:?}", call.direction, call.function);
    }

    // The app's `store.get` (guest → host): hide one key.
    fn example_rec_kv_store_get(this: &forward::FwdExampleRecKvStore, key: String) -> Option<String> {
        if key == "visits" {
            None
        } else {
            forward::example_rec_kv_store::get(this, key)
        }
    }

    // The host's `session.greet` (host → guest): decorate the guest's answer.
    fn example_rec_app_session_greet(
        this: &forward::FwdExampleRecAppSession,
        s: &example::rec::kv::Store,
    ) -> String {
        format!("{} [intercepted]", forward::example_rec_app_session::greet(this, s))
    }
}

export!(Interceptor);
