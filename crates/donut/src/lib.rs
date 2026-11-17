//! `child!`: bindings for a guest component (a *child*) plugged into this
//! component.
//!
//! ```ignore
//! // The component's own world: an ordinary wit-bindgen world.
//! wit_bindgen::generate!({ path: "wit", world: "example:app/host" });
//!
//! // One child. Every interface the guest world imports is virtualized,
//! // except those passed through.
//! donut::child!({
//!     name: "app",
//!     path: "wit",
//!     guest: "example:app/app",
//!     import_passthrough: ["wasi:*"],
//!     export_passthrough: ["example:app/info"],
//! });
//! ```
//!
//! Invoke it at the crate root. It expands to `pub mod <name>` with:
//!
//! * `virt::<ns>::<pkg>::<iface>`: the virtualized interfaces; implement
//!   their `Guest` traits and register them with `<name>::export!(T)`;
//! * `guest::<ns>::<pkg>::<iface>`: functions that call the child's exports;
//! * the bindings for host interfaces the above need;
//! * the `wit-bindgen-child:<name>` manifest section.
//!
//! The crate also needs a `wit-bindgen` dependency, as for `generate!`.
//! Several `child!`s can be used in one crate, each with its own `name`.
//! Children are instantiated one after another, in an unspecified order, so
//! a child's start function must not call code that calls another child:
//! that child may not be linked yet, and the call traps.
//!
//! Options:
//!
//! | Option | |
//! |---|---|
//! | `name` | required: the module, in snake case; its kebab-case form is the hole's import name |
//! | `guest` | required: the guest world, the hole's type |
//! | `path` | WIT path(s), relative to the crate's manifest directory; default `"wit"` |
//! | `import_passthrough` | guest imports passed straight through from the host |
//! | `export_passthrough` | guest exports re-exported unchanged |
//! | `with` | as in `generate!`, for the child's host bindings: `{ "wasi:io/streams": crate::host::wasi::io::streams }` or `generate` |
//! | `additional_derives` | as in `generate!` |
//! | `runtime_path` | as in `generate!` |
//!
//! Passthrough entries are `ns:pkg/iface`, `ns:pkg/iface@version`,
//! `ns:pkg/*` (optionally with `@version`), or `ns:*`.

use proc_macro2::{Span, TokenStream};
use quote::{ToTokens, quote};
use std::path::PathBuf;
use syn::parse::{Parse, ParseStream};
use syn::punctuated::Punctuated;
use syn::{Ident, LitStr, Token, braced, bracketed};

#[proc_macro]
pub fn child(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    let input = syn::parse_macro_input!(input as Input);
    match expand(input) {
        Ok(tokens) => tokens.into(),
        Err(e) => e.to_compile_error().into(),
    }
}

const OPTIONS: &str = "`name`, `guest`, `path`, `import_passthrough`, `export_passthrough`, `with`, \
                       `additional_derives`, `runtime_path`";

struct Input {
    name: LitStr,
    guest: LitStr,
    paths: Vec<LitStr>,
    import_passthrough: Vec<LitStr>,
    export_passthrough: Vec<LitStr>,
    with: Vec<(String, donut_plan::wit_bindgen_rust::WithOption)>,
    additional_derives: Vec<String>,
    runtime_path: Option<String>,
}

impl Parse for Input {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        use donut_plan::wit_bindgen_rust::WithOption;
        let content;
        braced!(content in input);
        let mut name = None;
        let mut guest = None;
        let mut paths = None;
        let mut import_passthrough = Vec::new();
        let mut export_passthrough = Vec::new();
        let mut with = Vec::new();
        let mut additional_derives = Vec::new();
        let mut runtime_path = None;
        let mut seen = Vec::new();
        while !content.is_empty() {
            let key: Ident = content.parse()?;
            content.parse::<Token![:]>()?;
            let key_name = key.to_string();
            if seen.contains(&key_name) {
                return Err(syn::Error::new(key.span(), format!("`{key_name}` is given twice")));
            }
            seen.push(key_name.clone());
            match key_name.as_str() {
                "name" => name = Some(content.parse::<LitStr>()?),
                "guest" => guest = Some(content.parse::<LitStr>()?),
                "path" => paths = Some(parse_strings(&content)?),
                "import_passthrough" => import_passthrough = parse_list(&content)?,
                "export_passthrough" => export_passthrough = parse_list(&content)?,
                "with" => {
                    let map;
                    braced!(map in content);
                    while !map.is_empty() {
                        let iface: LitStr = map.parse()?;
                        map.parse::<Token![:]>()?;
                        let value = if map.peek(Ident) && map.fork().parse::<Ident>()? == "generate" {
                            map.parse::<Ident>()?;
                            WithOption::Generate
                        } else {
                            WithOption::Path(path_string(&map.parse::<syn::Path>()?))
                        };
                        with.push((iface.value(), value));
                        if map.is_empty() {
                            break;
                        }
                        map.parse::<Token![,]>()?;
                    }
                }
                "additional_derives" => {
                    let list;
                    bracketed!(list in content);
                    let paths = Punctuated::<syn::Path, Token![,]>::parse_terminated(&list)?;
                    additional_derives = paths.iter().map(path_string).collect();
                }
                "runtime_path" => runtime_path = Some(content.parse::<LitStr>()?.value()),
                other => {
                    return Err(syn::Error::new(
                        key.span(),
                        format!("unknown `child!` option `{other}`; expected one of {OPTIONS}"),
                    ));
                }
            }
            if content.is_empty() {
                break;
            }
            content.parse::<Token![,]>()?;
        }
        let missing = |name: &str| syn::Error::new(Span::call_site(), format!("`child!` needs `{name}`"));
        Ok(Input {
            name: name.ok_or_else(|| missing("name"))?,
            guest: guest.ok_or_else(|| missing("guest"))?,
            paths: paths.unwrap_or_else(|| vec![LitStr::new("wit", Span::call_site())]),
            import_passthrough,
            export_passthrough,
            with,
            additional_derives,
            runtime_path,
        })
    }
}

fn path_string(path: &syn::Path) -> String {
    path.to_token_stream().to_string().replace(' ', "")
}

/// A string literal or a bracketed list of them.
fn parse_strings(input: ParseStream<'_>) -> syn::Result<Vec<LitStr>> {
    if input.peek(syn::token::Bracket) { parse_list(input) } else { Ok(vec![input.parse()?]) }
}

/// A bracketed list of string literals.
fn parse_list(input: ParseStream<'_>) -> syn::Result<Vec<LitStr>> {
    let content;
    bracketed!(content in input);
    Ok(Punctuated::<LitStr, Token![,]>::parse_terminated(&content)?.into_iter().collect())
}

fn expand(input: Input) -> syn::Result<TokenStream> {
    let err = |e: anyhow::Error| syn::Error::new(Span::call_site(), format!("{e:#}"));
    let root = PathBuf::from(
        std::env::var("CARGO_MANIFEST_DIR").map_err(|_| err(anyhow::anyhow!("CARGO_MANIFEST_DIR is not set")))?,
    );

    // Errors that belong to one literal point at it.
    donut_plan::Name::parse(&input.name.value()).map_err(|e| syn::Error::new(input.name.span(), format!("{e:#}")))?;
    for p in input.import_passthrough.iter().chain(input.export_passthrough.iter()) {
        donut_plan::Pattern::parse(&p.value()).map_err(|e| syn::Error::new(p.span(), format!("{e:#}")))?;
    }

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

    let mut config = donut_plan::Config {
        name: input.name.value(),
        guest: input.guest.value(),
        import_passthrough: input.import_passthrough.iter().map(|s| s.value()).collect(),
        export_passthrough: input.export_passthrough.iter().map(|s| s.value()).collect(),
        opts: Default::default(),
    };
    config.opts.with = input.with;
    config.opts.additional_derive_attributes = input.additional_derives;
    config.opts.runtime_path = input.runtime_path;
    let output = donut_plan::generate(&resolve, &packages, &config).map_err(|e| {
        // An error about one passthrough entry points at it.
        let msg = format!("{e:#}");
        let span = input
            .import_passthrough
            .iter()
            .chain(input.export_passthrough.iter())
            .find(|p| msg.starts_with(&format!("`{}` in `", p.value())))
            .map_or(Span::call_site(), |p| p.span());
        syn::Error::new(span, msg)
    })?;
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
