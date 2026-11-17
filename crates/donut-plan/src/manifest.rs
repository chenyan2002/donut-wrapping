//! The per-child manifest section.
//!
//! It tells wit-component's hole mode how to build a child's hole. This
//! encoder (and the decoder, used by tests) live here until the encoder side
//! exists in wit-component; then both move there, next to each other.

use anyhow::{Result, bail};
use wit_component::StringEncoding;
use wit_parser::{Resolve, WorldId};

/// Prefix of the manifest section's name; the child's (kebab-case) name
/// follows: `wit-bindgen-child:my-app`. One section per child, so wasm-ld
/// never concatenates two of them.
pub const SECTION_PREFIX: &str = "wit-bindgen-child:";

/// Version byte at the start of the payload.
pub const VERSION: u8 = 1;

/// What a child's manifest says.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Manifest {
    /// The child's name, in kebab case: the hole's import name.
    pub name: String,
    /// The guest world `G`, in wit-component's metadata encoding
    /// (`wit_component::metadata::encode`). It's the hole's exact type.
    pub guest: Vec<u8>,
    /// Virtualized guest imports: the guest's import name, and the name of
    /// the export implementing it (its `virtual-<child>:…` copy).
    /// Every other import of `G` is passed through.
    pub virtualized: Vec<(String, String)>,
    /// Every export of `G`: its name, and the import (its `guest-<name>:…`
    /// copy) through which the donut calls it. Calls to
    /// `"guest-a:ns-pkg/iface" "f"` go to child `a`'s `ns:pkg/iface.f`. Every
    /// import in namespace `guest-<name>` (these, and types-only copies of
    /// what they use) is the child's, not the host's: the encoder drops them
    /// from the merged world.
    pub exports: Vec<(String, String)>,
    /// Guest exports the donut re-exports unchanged.
    pub export_passthrough: Vec<String>,
}

impl Manifest {
    pub fn new(
        resolve: &Resolve,
        guest: WorldId,
        name: &str,
        virtualized: Vec<(String, String)>,
        exports: Vec<(String, String)>,
        export_passthrough: Vec<String>,
    ) -> Result<Manifest> {
        let guest = wit_component::metadata::encode(resolve, guest, StringEncoding::UTF8, None, false)?;
        Ok(Manifest { name: name.to_string(), guest, virtualized, exports, export_passthrough })
    }

    /// The custom section's name.
    pub fn section_name(&self) -> String {
        format!("{SECTION_PREFIX}{}", self.name)
    }

    /// Layout: `version: u8`, `name: string`, `guest: vec<u8>`,
    /// `virtualized: vec<(string, string)>`,
    /// `exports: vec<(string, string)>`, `export_passthrough: vec<string>`,
    /// with LEB128 lengths, as in the component binary format.
    pub fn encode(&self) -> Vec<u8> {
        let mut out = vec![VERSION];
        bytes(&mut out, self.name.as_bytes());
        bytes(&mut out, &self.guest);
        for pairs in [&self.virtualized, &self.exports] {
            leb(&mut out, pairs.len());
            for (a, b) in pairs.iter() {
                bytes(&mut out, a.as_bytes());
                bytes(&mut out, b.as_bytes());
            }
        }
        leb(&mut out, self.export_passthrough.len());
        for name in self.export_passthrough.iter() {
            bytes(&mut out, name.as_bytes());
        }
        out
    }

    pub fn decode(data: &[u8]) -> Result<Manifest> {
        let mut r = Reader(data);
        let version = r.byte()?;
        if version != VERSION {
            bail!("unsupported child manifest version {version}, expected {VERSION}");
        }
        let name = r.string()?;
        let guest = r.bytes()?.to_vec();
        let virtualized = (0..r.leb()?).map(|_| Ok((r.string()?, r.string()?))).collect::<Result<_>>()?;
        let exports = (0..r.leb()?).map(|_| Ok((r.string()?, r.string()?))).collect::<Result<_>>()?;
        let export_passthrough = (0..r.leb()?).map(|_| r.string()).collect::<Result<_>>()?;
        if !r.0.is_empty() {
            bail!("trailing bytes in child manifest");
        }
        Ok(Manifest { name, guest, virtualized, exports, export_passthrough })
    }
}

fn leb(out: &mut Vec<u8>, mut n: usize) {
    loop {
        let byte = (n & 0x7f) as u8;
        n >>= 7;
        if n == 0 {
            out.push(byte);
            return;
        }
        out.push(byte | 0x80);
    }
}

fn bytes(out: &mut Vec<u8>, b: &[u8]) {
    leb(out, b.len());
    out.extend_from_slice(b);
}

struct Reader<'a>(&'a [u8]);

impl<'a> Reader<'a> {
    fn byte(&mut self) -> Result<u8> {
        let Some((b, rest)) = self.0.split_first() else { bail!("unexpected end of child manifest") };
        self.0 = rest;
        Ok(*b)
    }
    fn leb(&mut self) -> Result<usize> {
        let mut n = 0usize;
        for shift in (0..64).step_by(7) {
            let b = self.byte()?;
            n |= ((b & 0x7f) as usize) << shift;
            if b & 0x80 == 0 {
                return Ok(n);
            }
        }
        bail!("LEB128 too long in child manifest")
    }
    fn bytes(&mut self) -> Result<&'a [u8]> {
        let len = self.leb()?;
        if len > self.0.len() {
            bail!("unexpected end of child manifest");
        }
        let (b, rest) = self.0.split_at(len);
        self.0 = rest;
        Ok(b)
    }
    fn string(&mut self) -> Result<String> {
        Ok(std::str::from_utf8(self.bytes()?)?.to_string())
    }
}
