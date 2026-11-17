//! `import_passthrough` / `export_passthrough` patterns.

use anyhow::Result;
use wit_parser::{InterfaceId, Resolve};

/// One entry of `import_passthrough` or `export_passthrough`:
///
/// * `ns:pkg/iface` or `ns:pkg/iface@1.2.3`: one interface;
/// * `ns:pkg/*` or `ns:pkg/*@1.2.3`: every interface of a package;
/// * `ns:*`: every interface of a namespace.
///
/// Without a version, any version matches.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pattern {
    text: String,
    namespace: String,
    package: Option<String>,
    interface: Option<String>,
    version: Option<String>,
}

impl Pattern {
    pub fn parse(text: &str) -> Result<Pattern> {
        let bad = |why: &str| {
            anyhow::anyhow!(
                "invalid passthrough pattern `{text}`: {why}; expected `ns:pkg/iface`, \
                 `ns:pkg/iface@version`, `ns:pkg/*`, or `ns:*`"
            )
        };
        let (path, version) = match text.split_once('@') {
            Some((path, version)) if !version.is_empty() => (path, Some(version.to_string())),
            Some(_) => return Err(bad("empty version")),
            None => (text, None),
        };
        let Some((namespace, rest)) = path.split_once(':') else {
            return Err(bad("no `:`"));
        };
        if !is_name(namespace) {
            return Err(bad("bad namespace"));
        }
        let (package, interface) = if rest == "*" {
            if version.is_some() {
                return Err(bad("a namespace pattern can't have a version"));
            }
            (None, None)
        } else {
            let Some((package, interface)) = rest.split_once('/') else {
                return Err(bad("no `/`"));
            };
            if !is_name(package) {
                return Err(bad("bad package name"));
            }
            match interface {
                "*" => (Some(package.to_string()), None),
                i if is_name(i) => (Some(package.to_string()), Some(i.to_string())),
                _ => return Err(bad("bad interface name")),
            }
        };
        Ok(Pattern { text: text.to_string(), namespace: namespace.to_string(), package, interface, version })
    }

    pub fn matches(&self, resolve: &Resolve, id: InterfaceId) -> bool {
        let iface = &resolve.interfaces[id];
        let (Some(pkg), Some(name)) = (iface.package, iface.name.as_deref()) else {
            return false;
        };
        let pkg = &resolve.packages[pkg].name;
        pkg.namespace == self.namespace
            && self.package.as_ref().is_none_or(|p| *p == pkg.name)
            && self.interface.as_ref().is_none_or(|i| i == name)
            && self
                .version
                .as_ref()
                .is_none_or(|v| pkg.version.as_ref().is_some_and(|pv| pv.to_string() == *v))
    }

    pub fn as_str(&self) -> &str {
        &self.text
    }
}

/// A WIT identifier, loosely: kebab-case words of ASCII alphanumerics.
fn is_name(s: &str) -> bool {
    !s.is_empty()
        && s.split('-').all(|w| {
            w.chars().next().is_some_and(|c| c.is_ascii_alphabetic())
                && w.chars().all(|c| c.is_ascii_alphanumeric())
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn resolve() -> (Resolve, Vec<InterfaceId>) {
        let mut resolve = Resolve::default();
        resolve.push_str("a.wit", "package wasi:cli@0.2.6; interface stdout {} interface stdin {}").unwrap();
        resolve.push_str("b.wit", "package wasi:io@0.2.6; interface streams {}").unwrap();
        resolve.push_str("c.wit", "package example:app; interface types {}").unwrap();
        let ids = resolve.interfaces.iter().map(|(id, _)| id).collect();
        (resolve, ids)
    }

    fn matching(pattern: &str) -> Vec<String> {
        let (resolve, ids) = resolve();
        let p = Pattern::parse(pattern).unwrap();
        ids.into_iter().filter(|id| p.matches(&resolve, *id)).map(|id| resolve.id_of(id).unwrap()).collect()
    }

    #[test]
    fn matches() {
        assert_eq!(matching("wasi:cli/stdout"), ["wasi:cli/stdout@0.2.6"]);
        assert_eq!(matching("wasi:cli/stdout@0.2.6"), ["wasi:cli/stdout@0.2.6"]);
        assert!(matching("wasi:cli/stdout@0.2.0").is_empty());
        assert_eq!(matching("wasi:cli/*"), ["wasi:cli/stdout@0.2.6", "wasi:cli/stdin@0.2.6"]);
        assert_eq!(matching("wasi:cli/*@0.2.6").len(), 2);
        assert_eq!(matching("wasi:*").len(), 3);
        assert_eq!(matching("example:app/types"), ["example:app/types"]);
        assert!(matching("example:app/types@1.0.0").is_empty());
        assert!(matching("wasi:cl/*").is_empty());
    }

    #[test]
    fn rejects() {
        for bad in ["wasi", "wasi:", "wasi:cli", "wasi:cli/", "wasi:*@0.2.6", "wasi:cli/stdout@", "wasi:cli/std_out", "wa si:*", "wasi:cli/**"] {
            assert!(Pattern::parse(bad).is_err(), "{bad} should be rejected");
        }
    }
}
