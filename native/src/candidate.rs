// Port of Tailwind CSS 4.3.4 candidate.ts. See LICENSE.tailwind.
use crate::syntax::{arbitrary, is_js_whitespace, segment::segment};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::atomic::{AtomicUsize, Ordering},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Static,
    Functional,
    Compound,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VariantDefinition {
    pub kind: Kind,
    pub order: usize,
    pub compounds: u8,
    pub compounds_with: u8,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Registry {
    pub prefix: Option<String>,
    pub utilities: BTreeMap<String, BTreeSet<Kind>>,
    pub variants: BTreeMap<String, VariantDefinition>,
}

impl Registry {
    pub fn has_utility(&self, name: &str, kind: Kind) -> bool {
        self.utilities
            .get(name)
            .is_some_and(|kinds| kinds.contains(&kind))
    }

    fn compounds_with(&self, parent: &str, child: &Variant) -> bool {
        let Some(parent) = self.variants.get(parent) else {
            return false;
        };
        if parent.kind != Kind::Compound || parent.compounds_with == 0 {
            return false;
        }
        let compounds = match child {
            Variant::Arbitrary { selector, .. } => {
                if selector.starts_with('@') {
                    if selector.starts_with("@media")
                        || selector.starts_with("@supports")
                        || selector.starts_with("@container")
                    {
                        1
                    } else {
                        0
                    }
                } else if selector.contains("::") {
                    0
                } else {
                    2
                }
            }
            Variant::Static { root }
            | Variant::Functional { root, .. }
            | Variant::Compound { root, .. } => {
                let Some(child) = self.variants.get(root) else {
                    return false;
                };
                child.compounds
            }
        };
        compounds != 0 && (parent.compounds_with & compounds) != 0
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum VariantValue {
    Arbitrary { value: String },
    Named { value: String },
}

pub type Modifier = VariantValue;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum UtilityValue {
    Arbitrary {
        #[serde(rename = "dataType")]
        data_type: Option<String>,
        value: String,
    },
    Named {
        value: String,
        fraction: Option<String>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Variant {
    Arbitrary {
        selector: String,
        relative: bool,
    },
    Static {
        root: String,
    },
    Functional {
        root: String,
        value: Option<VariantValue>,
        modifier: Option<Modifier>,
    },
    Compound {
        root: String,
        modifier: Option<Modifier>,
        variant: Box<Variant>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum CandidateKind {
    Arbitrary {
        property: String,
        value: String,
        modifier: Option<Modifier>,
    },
    Static {
        root: String,
    },
    Functional {
        root: String,
        value: Option<UtilityValue>,
        modifier: Option<Modifier>,
    },
}

#[derive(Clone, Copy, Debug)]
pub struct CandidateIdentity(pub(crate) usize);

impl Default for CandidateIdentity {
    fn default() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(1);
        Self(NEXT.fetch_add(1, Ordering::Relaxed))
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Candidate {
    #[serde(skip)]
    pub identity: CandidateIdentity,
    #[serde(flatten)]
    pub kind: CandidateKind,
    pub variants: Vec<Variant>,
    pub important: bool,
    pub raw: String,
}

impl PartialEq for Candidate {
    fn eq(&self, other: &Self) -> bool {
        self.kind == other.kind
            && self.variants == other.variants
            && self.important == other.important
            && self.raw == other.raw
    }
}
impl Eq for Candidate {}

impl Candidate {
    /// Equivalent to the upstream cloneCandidate: a distinct cache key.
    pub fn clone_fresh(&self) -> Self {
        Self {
            identity: CandidateIdentity::default(),
            ..self.clone()
        }
    }
}

fn named_value(value: &str) -> bool {
    !value.is_empty()
        && value
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'_' | b'.' | b'%' | b'-'))
}

fn empty(value: &str) -> bool {
    value.trim_matches(is_js_whitespace).is_empty()
}

fn find_roots(input: &str, exists: impl Fn(&str) -> bool) -> Vec<(String, Option<String>)> {
    let mut roots = Vec::new();
    if exists(input) {
        roots.push((input.into(), None));
    }
    let mut remainder = input;
    while let Some(index) = remainder.rfind('-').filter(|i| *i > 0) {
        let root = &input[..index];
        if exists(root) {
            let value = &input[index + 1..];
            if value.is_empty() || root == "@" {
                break;
            }
            roots.push((root.into(), Some(value.into())));
        }
        remainder = &input[..index];
    }
    if input.starts_with('@') && exists("@") {
        roots.push(("@".into(), Some(input[1..].into())));
    }
    roots
}

pub fn parse_modifier(input: &str) -> Option<Modifier> {
    if let Some(raw) = input.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
        let value = arbitrary::decode_valid(raw)?;
        return (!empty(&value)).then_some(Modifier::Arbitrary { value });
    }
    if let Some(raw) = input.strip_prefix('(').and_then(|s| s.strip_suffix(')')) {
        if !raw.starts_with("--") || !arbitrary::is_valid(raw) {
            return None;
        }
        return Some(Modifier::Arbitrary {
            value: arbitrary::decode(&format!("var({raw})")),
        });
    }
    named_value(input).then(|| Modifier::Named {
        value: input.into(),
    })
}

pub fn parse_variant(input: &str, registry: &Registry) -> Option<Variant> {
    parse_variant_with(input, registry, &mut |child| parse_variant(child, registry))
}

pub fn parse_variant_with(
    input: &str,
    registry: &Registry,
    parse_child: &mut impl FnMut(&str) -> Option<Variant>,
) -> Option<Variant> {
    if let Some(raw) = input.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
        if raw.starts_with('@') && raw.contains('&') {
            return None;
        }
        let mut selector = arbitrary::decode_valid(raw)?;
        if empty(&selector) {
            return None;
        }
        let relative = selector.starts_with(['>', '+', '~']);
        if !relative && !selector.starts_with('@') && !selector.contains('&') {
            selector = format!("&:is({selector})");
        }
        return Some(Variant::Arbitrary { selector, relative });
    }
    let parts = segment(input, b'/');
    if parts.len() > 2 {
        return None;
    }
    let raw_modifier = parts.get(1).copied();
    for (root, value) in find_roots(parts[0], |r| registry.variants.contains_key(r)) {
        let definition = &registry.variants[&root];
        match definition.kind {
            Kind::Static => {
                if value.is_some() || raw_modifier.is_some() {
                    return None;
                }
                return Some(Variant::Static { root });
            }
            Kind::Functional => {
                let modifier = match raw_modifier {
                    Some(value) => Some(parse_modifier(value)?),
                    None => None,
                };
                let Some(value) = value else {
                    return Some(Variant::Functional {
                        root,
                        value: None,
                        modifier,
                    });
                };
                let parsed = if value.ends_with(']') {
                    let Some(raw) = value.strip_prefix('[').and_then(|s| s.strip_suffix(']'))
                    else {
                        continue;
                    };
                    let value = arbitrary::decode_valid(raw)?;
                    if empty(&value) {
                        return None;
                    }
                    VariantValue::Arbitrary { value }
                } else if value.ends_with(')') {
                    let Some(raw) = value.strip_prefix('(').and_then(|s| s.strip_suffix(')'))
                    else {
                        continue;
                    };
                    let value = arbitrary::decode_valid(raw)?;
                    if empty(&value) || !value.starts_with("--") {
                        return None;
                    }
                    VariantValue::Arbitrary {
                        value: format!("var({value})"),
                    }
                } else {
                    if !named_value(&value) {
                        continue;
                    }
                    VariantValue::Named { value }
                };
                return Some(Variant::Functional {
                    root,
                    value: Some(parsed),
                    modifier,
                });
            }
            Kind::Compound => {
                let mut value = value?;
                let mut modifier = raw_modifier;
                if modifier.is_some_and(|s| !s.is_empty())
                    && ["not", "has", "in"].contains(&root.as_str())
                {
                    value.push('/');
                    value.push_str(modifier?);
                    modifier = None;
                }
                let variant = parse_child(&value)?;
                if !registry.compounds_with(&root, &variant) {
                    return None;
                }
                let modifier = match modifier {
                    Some(value) => Some(parse_modifier(value)?),
                    None => None,
                };
                return Some(Variant::Compound {
                    root,
                    modifier,
                    variant: Box::new(variant),
                });
            }
        }
    }
    None
}

pub fn parse(input: &str, registry: &Registry) -> Vec<Candidate> {
    parse_with(input, registry, &mut |variant| {
        parse_variant(variant, registry)
    })
}

pub fn parse_with(
    input: &str,
    registry: &Registry,
    parse_variant: &mut impl FnMut(&str) -> Option<Variant>,
) -> Vec<Candidate> {
    let mut result = Vec::new();
    let mut parts = segment(input, b':');
    if let Some(prefix) = registry.prefix.as_deref().filter(|p| !p.is_empty()) {
        if parts.len() == 1 || parts[0] != prefix {
            return result;
        }
        parts.remove(0);
    }
    let mut base = parts.pop().unwrap_or("");
    let Some(variants) = parts
        .into_iter()
        .rev()
        .map(parse_variant)
        .collect::<Option<Vec<_>>>()
    else {
        return result;
    };
    let important = if let Some(value) = base.strip_suffix('!') {
        base = value;
        true
    } else if let Some(value) = base.strip_prefix('!') {
        base = value;
        true
    } else {
        false
    };
    let make = |kind| Candidate {
        identity: CandidateIdentity::default(),
        kind,
        variants: variants.clone(),
        important,
        raw: input.into(),
    };
    if registry.has_utility(base, Kind::Static) && !base.contains('[') {
        result.push(make(CandidateKind::Static { root: base.into() }));
    }
    let parts = segment(base, b'/');
    if parts.len() > 2 {
        return result;
    }
    let base = parts[0];
    let raw_modifier = parts.get(1).copied();
    let modifier = match raw_modifier {
        Some(value) => match parse_modifier(value) {
            Some(value) => Some(value),
            None => return result,
        },
        None => None,
    };
    if base.starts_with('[') {
        let Some(raw) = base.strip_prefix('[').and_then(|s| s.strip_suffix(']')) else {
            return result;
        };
        if !raw
            .as_bytes()
            .first()
            .is_some_and(|c| *c == b'-' || c.is_ascii_lowercase())
        {
            return result;
        }
        let Some((property, raw)) = raw.split_once(':') else {
            return result;
        };
        if property.is_empty() || raw.is_empty() {
            return result;
        }
        if let Some(value) = arbitrary::decode_valid(raw) {
            result.push(make(CandidateKind::Arbitrary {
                property: property.into(),
                value,
                modifier,
            }));
        }
        return result;
    }
    let roots = if base.ends_with(']') {
        let Some((root, rest)) = base.split_once("-[") else {
            return result;
        };
        if !registry.has_utility(root, Kind::Functional) {
            return result;
        }
        vec![(root.into(), Some(format!("[{rest}")))]
    } else if base.ends_with(')') {
        let Some((root, raw)) = base.split_once("-(") else {
            return result;
        };
        if !registry.has_utility(root, Kind::Functional) {
            return result;
        }
        let raw = &raw[..raw.len() - 1];
        let parts = segment(raw, b':');
        let (data_type, value) = if parts.len() == 2 {
            (Some(parts[0]), parts[1])
        } else {
            (None, raw)
        };
        if !value.starts_with("--") || !arbitrary::is_valid(value) {
            return result;
        }
        let value = match data_type {
            Some(t) => format!("[{t}:var({value})]"),
            None => format!("[var({value})]"),
        };
        vec![(root.into(), Some(value))]
    } else {
        find_roots(base, |r| registry.has_utility(r, Kind::Functional))
    };
    for (root, raw) in roots {
        let value = if let Some(raw) = raw {
            if let Some(index) = raw.find('[') {
                if !raw.ends_with(']') {
                    return result;
                }
                let Some(mut value) = arbitrary::decode_valid(&raw[index + 1..raw.len() - 1])
                else {
                    continue;
                };
                let mut data_type = None;
                for (index, c) in value.char_indices() {
                    if c == ':' {
                        data_type = Some(value[..index].to_owned());
                        value = value[index + 1..].to_owned();
                        break;
                    }
                    if c != '-' && !c.is_ascii_lowercase() {
                        break;
                    }
                }
                if empty(&value) || data_type.as_deref() == Some("") {
                    continue;
                }
                Some(UtilityValue::Arbitrary { data_type, value })
            } else {
                if !named_value(&raw) {
                    continue;
                }
                let fraction = if matches!(modifier, Some(Modifier::Arbitrary { .. })) {
                    None
                } else {
                    raw_modifier.map(|m| format!("{raw}/{m}"))
                };
                Some(UtilityValue::Named {
                    value: raw,
                    fraction,
                })
            }
        } else {
            None
        };
        result.push(make(CandidateKind::Functional {
            root,
            value,
            modifier: modifier.clone(),
        }));
    }
    result
}
