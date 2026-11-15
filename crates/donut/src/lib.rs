//! `donut!`: generate bindings for a component that virtualizes some of a
//! guest component's imports, embeds the guest, and calls its exports.
//!
//! ```ignore
//! donut::donut!({
//!     path: "wit",
//!     world: "example:http/socket-server",   // the donut's own world
//!     guest: "example:http/http-guest",      // the guest's world
//!     virtualize: ["example:http/types", "example:http/outgoing"],
//! });
//! ```
//!
//! This expands, at the crate root, to:
//!
//! * the usual wit-bindgen bindings for the donut's world, where every
//!   virtualized guest import is an export (`exports::…`: implement its
//!   `Guest*` traits) and every other guest import is an import;
//! * `mod guest`, with functions that call the guest's exports
//!   (`guest::…`);
//! * the `wit-bindgen-donut:v0` manifest section.
//!
//! The resulting core module is turned into a component by wit-component's
//! donut mode, e.g. `wasm-tools component new <core.wasm> --donut-guest
//! <guest.wasm>`.
//!
//! Supported options: `path` (a string or a list of strings, relative to the
//! crate's manifest directory; default `"wit"`), `world`, `guest`,
//! `virtualize`, and for generated forwarding, `forward` (interfaces to
//! forward: guest imports go to the host, shared exports go to the guest) with
//! `hooks` (the type that implements `forward::Hooks`).

use proc_macro2::{Span, TokenStream};
use quote::quote;
use std::path::PathBuf;
use syn::parse::{Parse, ParseStream};
use syn::punctuated::Punctuated;
use syn::{Ident, LitStr, Token, braced, bracketed};

#[proc_macro]
pub fn donut(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    let input = syn::parse_macro_input!(input as Input);
    match expand(input) {
        Ok(tokens) => tokens.into(),
        Err(e) => e.to_compile_error().into(),
    }
}

struct Input {
    paths: Vec<LitStr>,
    world: LitStr,
    guest: LitStr,
    virtualize: Vec<LitStr>,
    forward: Vec<LitStr>,
    hooks: Option<syn::Path>,
}

impl Parse for Input {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        let content;
        braced!(content in input);
        let mut paths = None;
        let mut world = None;
        let mut guest = None;
        let mut virtualize = None;
        let mut forward = Vec::new();
        let mut hooks = None;
        while !content.is_empty() {
            let key: Ident = content.parse()?;
            content.parse::<Token![:]>()?;
            match key.to_string().as_str() {
                "path" => paths = Some(parse_strings(&content)?),
                "world" => world = Some(content.parse::<LitStr>()?),
                "guest" => guest = Some(content.parse::<LitStr>()?),
                "virtualize" => virtualize = Some(parse_strings(&content)?),
                "forward" => forward = parse_strings(&content)?,
                "hooks" => hooks = Some(content.parse::<syn::Path>()?),
                other => {
                    return Err(syn::Error::new(
                        key.span(),
                        format!(
                            "unknown `donut!` option `{other}`; expected one of \
                             `path`, `world`, `guest`, `virtualize`, `forward`, `hooks`"
                        ),
                    ));
                }
            }
            if content.is_empty() {
                break;
            }
            content.parse::<Token![,]>()?;
        }
        let missing = |name: &str| syn::Error::new(Span::call_site(), format!("missing `{name}`"));
        Ok(Input {
            paths: paths.unwrap_or_else(|| vec![LitStr::new("wit", Span::call_site())]),
            world: world.ok_or_else(|| missing("world"))?,
            guest: guest.ok_or_else(|| missing("guest"))?,
            virtualize: virtualize.unwrap_or_default(),
            forward,
            hooks,
        })
    }
}

/// A string literal or a bracketed list of them.
fn parse_strings(input: ParseStream<'_>) -> syn::Result<Vec<LitStr>> {
    if input.peek(syn::token::Bracket) {
        let content;
        bracketed!(content in input);
        let list = Punctuated::<LitStr, Token![,]>::parse_terminated(&content)?;
        Ok(list.into_iter().collect())
    } else {
        Ok(vec![input.parse()?])
    }
}

fn expand(input: Input) -> syn::Result<TokenStream> {
    let err = |e: anyhow::Error| syn::Error::new(Span::call_site(), format!("{e:?}"));
    let root = PathBuf::from(
        std::env::var("CARGO_MANIFEST_DIR").map_err(|_| err(anyhow::anyhow!("CARGO_MANIFEST_DIR is not set")))?,
    );

    let mut resolve = donut_plan::wit_parser::Resolve::default();
    let mut packages = Vec::new();
    let mut files = Vec::new();
    for path in input.paths.iter() {
        let (pkg, sources) = resolve
            .push_path(root.join(path.value()))
            .map_err(|e| syn::Error::new(path.span(), format!("{e:?}")))?;
        packages.push(pkg);
        files.extend(sources.paths().map(|p| p.display().to_string()));
    }

    let config = donut_plan::Config {
        world: input.world.value(),
        guest: input.guest.value(),
        virtualize: input.virtualize.iter().map(|s| s.value()).collect(),
        forward: input.forward.iter().map(|s| s.value()).collect(),
        hooks: input
            .hooks
            .as_ref()
            .map(|p| quote!(#p).to_string().replace(' ', "")),
        opts: Default::default(),
    };
    let output = donut_plan::generate(&resolve, &packages, &config).map_err(err)?;
    let source: TokenStream = output
        .source
        .parse()
        .map_err(|e| err(anyhow::anyhow!("generated code doesn't parse: {e}")))?;

    // Rebuild when any WIT file changes.
    Ok(quote! {
        #(const _: &[u8] = include_bytes!(#files);)*
        #source
    })
}
