use crate::{
    ast::AstNode,
    candidate::{CandidateKind, Kind, UtilityValue},
    numeric,
    syntax::segment::segment,
    theme::Theme,
};
use indexmap::IndexMap;
use std::sync::LazyLock;

enum Body {
    Static(Vec<(&'static str, String)>),
    Breakpoint,
    Flex(&'static str),
    Inset(&'static str, bool),
}

pub(crate) struct Definition {
    pub kind: Kind,
    pub hidden: bool,
    body: Body,
}

pub(crate) static DEFINITIONS: LazyLock<IndexMap<String, Definition>> = LazyLock::new(|| {
    let mut definitions = IndexMap::new();
    let mut add = |name: String, hidden, body: Body| {
        definitions.insert(
            name,
            Definition {
                kind: if matches!(body, Body::Static(_)) {
                    Kind::Static
                } else {
                    Kind::Functional
                },
                hidden,
                body,
            },
        );
    };
    for (name, direction) in [
        ("t", "top"),
        ("tr", "top right"),
        ("r", "right"),
        ("br", "bottom right"),
        ("b", "bottom"),
        ("bl", "bottom left"),
        ("l", "left"),
        ("tl", "top left"),
    ] {
        add(
            format!("bg-gradient-to-{name}"),
            true,
            Body::Static(vec![
                ("--tw-gradient-position", format!("to {direction} in oklab")),
                (
                    "background-image",
                    "linear-gradient(var(--tw-gradient-stops))".into(),
                ),
            ]),
        );
    }
    for (root, property) in [("bg", "background-position"), ("object", "object-position")] {
        for (name, value) in [
            ("left-top", "left top"),
            ("right-top", "right top"),
            ("left-bottom", "left bottom"),
            ("right-bottom", "right bottom"),
        ] {
            add(
                format!("{root}-{name}"),
                true,
                Body::Static(vec![(property, value.into())]),
            );
        }
    }
    add("max-w-screen".into(), true, Body::Breakpoint);
    add(
        "overflow-ellipsis".into(),
        true,
        Body::Static(vec![("text-overflow", "ellipsis".into())]),
    );
    for value in ["slice", "clone"] {
        add(
            format!("decoration-{value}"),
            true,
            Body::Static(vec![
                ("-webkit-box-decoration-break", value.into()),
                ("box-decoration-break", value.into()),
            ]),
        );
    }
    for property in ["flex-shrink", "flex-grow"] {
        add(property.into(), true, Body::Flex(property));
    }
    add(
        "order-none".into(),
        true,
        Body::Static(vec![("order", "0".into())]),
    );
    add(
        "break-words".into(),
        true,
        Body::Static(vec![("overflow-wrap", "break-word".into())]),
    );
    for (name, property) in [("start", "inset-inline-start"), ("end", "inset-inline-end")] {
        for (suffix, value, negative) in [
            ("auto", "auto", false),
            ("full", "100%", false),
            ("full", "-100%", true),
            ("px", "1px", false),
            ("px", "-1px", true),
        ] {
            add(
                format!("{}{name}-{suffix}", if negative { "-" } else { "" }),
                false,
                Body::Static(vec![(property, value.into())]),
            );
        }
        add(format!("-{name}"), false, Body::Inset(property, true));
        add(name.into(), false, Body::Inset(property, false));
    }
    definitions
});

pub(crate) fn compile(candidate: &CandidateKind, theme: &Theme) -> Option<Vec<AstNode>> {
    let (root, value, modifier, kind) = match candidate {
        CandidateKind::Static { root } => (root, None, None, Kind::Static),
        CandidateKind::Functional {
            root,
            value,
            modifier,
        } => (root, value.as_ref(), modifier.as_ref(), Kind::Functional),
        CandidateKind::Arbitrary { .. } => return None,
    };
    let definition = DEFINITIONS.get(root)?;
    if definition.kind != kind {
        return None;
    }
    let (property, value) = match &definition.body {
        Body::Static(declarations) => {
            return Some(
                declarations
                    .iter()
                    .map(|(property, value)| AstNode::decl(*property, value))
                    .collect(),
            );
        }
        Body::Breakpoint => {
            let UtilityValue::Named { value, .. } = value? else {
                return None;
            };
            let value = theme.resolve(Some(value), &["--breakpoint".into()], 0)?;
            if value.is_empty() {
                return None;
            }
            ("max-width", value)
        }
        Body::Flex(property) => {
            if modifier.is_some() {
                return None;
            }
            let value = match value {
                None => "1".into(),
                Some(UtilityValue::Arbitrary { value, .. }) => value.clone(),
                Some(UtilityValue::Named { value, .. }) if numeric::is_positive_integer(value) => {
                    value.clone()
                }
                _ => return None,
            };
            (*property, value)
        }
        Body::Inset(property, negative) => {
            let value = match value? {
                UtilityValue::Arbitrary { value, .. } => {
                    if modifier.is_some() {
                        return None;
                    }
                    value.clone()
                }
                UtilityValue::Named { value, fraction } => {
                    let mut resolved = theme.resolve(
                        Some(fraction.as_deref().unwrap_or(value)),
                        &["--inset".into(), "--spacing".into()],
                        0,
                    );
                    if resolved.is_none()
                        && let Some(fraction) = fraction
                    {
                        let parts = segment(fraction, b'/');
                        let [lhs, rhs, ..] = parts.as_slice() else {
                            return None;
                        };
                        if !numeric::is_positive_integer(lhs) || !numeric::is_positive_integer(rhs)
                        {
                            return None;
                        }
                        resolved = Some(format!("calc({fraction} * 100%)"));
                    }
                    if resolved.is_none() {
                        let multiplier = theme.resolve(None, &["--spacing".into()], 0)?;
                        if multiplier.is_empty() || !numeric::is_spacing_multiplier(value) {
                            return None;
                        }
                        return Some(vec![AstNode::decl(
                            *property,
                            format!(
                                "calc({multiplier} * {}{value})",
                                if *negative { "-" } else { "" }
                            ),
                        )]);
                    }
                    resolved?
                }
            };
            (
                *property,
                if *negative {
                    format!("calc({value} * -1)")
                } else {
                    value
                },
            )
        }
    };
    Some(vec![AstNode::decl(property, value)])
}
