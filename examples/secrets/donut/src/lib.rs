//! Runs the app on a host with no filesystem but with a secret vault. Each of
//! the app's requests gets the vault's token for its host, and requests to
//! hosts without one are refused. The app never sees a token.
donut::donut!({
    path: "../wit",
    world: "example:app/host",
    guest: "example:app/app",
    // Allowed: passed on to the host (requests) or the app (`handle`).
    forward: ["example:app/types", "example:app/handler"],
    // Decided: the only way out to the network.
    virtualize: ["example:app/outgoing"],
    // Missing on this host: only what the app uses. A mocked `file` is an
    // `OpenFile`.
    mock: [{ "example:app/fs": { file: OpenFile } }],
    hooks: Policy,
});

use example::app::{outgoing as host_outgoing, secrets::Vault};
use exports::virtual_example::app::{fs::File, types::Request};
use forward::{FwdExampleAppTypesRequest, MockExampleAppFsFile};

const CONF: &str = "upstream=https://api.internal";

/// What a mocked `fs` file holds.
pub struct OpenFile {
    contents: String,
}

pub struct Policy;

impl exports::virtual_example::app::outgoing::Guest for Policy {
    fn send(r: Request) -> Result<u16, String> {
        // Read host request
        let req: &example::app::types::Request = r.get::<FwdExampleAppTypesRequest>().host();
        let url = req.url();
        let host = url.split("://").nth(1).and_then(|s| s.split('/').next()).unwrap_or_default();
        let token = Vault::open("tokens").get(host).ok_or_else(|| format!("{host}: not allowed"))?;

        // Transfer the request ownership to host
        let req: example::app::types::Request = r.into_inner::<FwdExampleAppTypesRequest>().into_host();
        req.set_header("authorization", &format!("Bearer {token}"));
        host_outgoing::send(req)
    }
}

impl forward::Hooks for Policy {
    // Every forwarded and mocked call.
    fn before(call: &forward::Call, args: &[&dyn core::fmt::Debug]) {
        eprintln!("donut: {} {}{args:?}", call.interface, call.function);
    }

    // The app reads one settings file. Every other `fs` function traps.
    fn example_app_fs_open(path: String) -> Result<File, String> {
        match path.as_str() {
            "/etc/app.conf" => Ok(File::new(MockExampleAppFsFile::new(OpenFile { contents: CONF.into() }))),
            _ => Err(format!("{path}: no such file")),
        }
    }
    fn example_app_fs_file_read(this: &MockExampleAppFsFile) -> String {
        this.contents.clone()
    }
}

export!(Policy);
