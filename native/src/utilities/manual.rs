// Manual built-in matchers from Tailwind CSS 4.3.4 utilities.ts. See LICENSE.tailwind.
use super::as_color;
use crate::{
    ast::AstNode,
    breakpoints::{self, Direction},
    candidate::{Candidate, CandidateKind, Modifier, UtilityValue},
    data_type::{self, DataType},
    numeric,
    syntax::segment::segment,
    theme::Theme,
};

mod backgrounds;
pub(crate) mod shadows;
mod typography_border;

fn property(name: &str, initial: Option<&str>, syntax: Option<&str>) -> AstNode {
    let mut nodes = vec![
        AstNode::decl(
            "syntax",
            format!("\"{}\"", syntax.filter(|s| !s.is_empty()).unwrap_or("*")),
        ),
        AstNode::decl("inherits", "false"),
    ];
    if let Some(initial) = initial.filter(|s| !s.is_empty()) {
        nodes.push(AstNode::decl("initial-value", initial));
    }
    AstNode::AtRule {
        name: "@property".into(),
        params: name.into(),
        nodes,
    }
}

fn value_type<'a>(value: &str, hint: Option<&'a str>, types: &[DataType]) -> Option<&'a str> {
    hint.or_else(|| data_type::infer(value, types).map(DataType::as_str))
}

fn theme_color(
    value: &str,
    modifier: Option<&Modifier>,
    theme: &Theme,
    namespaces: &[&str],
) -> Option<String> {
    let value = match value {
        "inherit" => "inherit".into(),
        "transparent" => "transparent".into(),
        "current" => "currentcolor".into(),
        _ => theme.resolve(
            Some(value),
            &namespaces.iter().map(|s| (*s).into()).collect::<Vec<_>>(),
            0,
        )?,
    };
    if value.is_empty() {
        return None;
    }
    as_color(&value, modifier, theme)
}

fn flex(value: Option<&UtilityValue>, modifier: Option<&Modifier>) -> Option<Vec<AstNode>> {
    let value = match value? {
        UtilityValue::Arbitrary { value, .. } => {
            if modifier.is_some() {
                return None;
            }
            value.clone()
        }
        UtilityValue::Named { value, fraction } => {
            if let Some(fraction) = fraction {
                let parts = segment(fraction, b'/');
                if parts.len() < 2
                    || !numeric::is_positive_integer(parts[0])
                    || !numeric::is_positive_integer(parts[1])
                {
                    return None;
                }
                format!("calc({fraction} * 100%)")
            } else {
                if modifier.is_some() || !numeric::is_positive_integer(value) {
                    return None;
                }
                value.clone()
            }
        }
    };
    Some(vec![AstNode::decl("flex", value)])
}

fn scale(
    value: Option<&UtilityValue>,
    modifier: Option<&Modifier>,
    negative: bool,
    theme: &Theme,
) -> Option<Vec<AstNode>> {
    if modifier.is_some() {
        return None;
    }
    let value = value?;
    let resolved = match value {
        UtilityValue::Arbitrary { value, .. } => value.clone(),
        UtilityValue::Named { value, .. } => theme
            .resolve(Some(value), &["--scale".into()], 0)
            .filter(|v| !v.is_empty())
            .or_else(|| numeric::is_positive_integer(value).then(|| format!("{value}%")))?,
    };
    let resolved = if negative {
        format!("calc({resolved} * -1)")
    } else {
        resolved
    };
    if matches!(value, UtilityValue::Arbitrary { .. }) {
        return Some(vec![AstNode::decl("scale", resolved)]);
    }
    Some(vec![
        AstNode::AtRoot {
            nodes: ["x", "y", "z"]
                .map(|axis| property(&format!("--tw-scale-{axis}"), Some("1"), None))
                .into(),
        },
        AstNode::decl("--tw-scale-x", &resolved),
        AstNode::decl("--tw-scale-y", &resolved),
        AstNode::decl("--tw-scale-z", resolved),
        AstNode::decl("scale", "var(--tw-scale-x) var(--tw-scale-y)"),
    ])
}

fn rotate(
    value: Option<&UtilityValue>,
    modifier: Option<&Modifier>,
    negative: bool,
    theme: &Theme,
) -> Option<Vec<AstNode>> {
    if modifier.is_some() {
        return None;
    }
    let value = match value? {
        UtilityValue::Arbitrary { value, data_type } => {
            let vector = match data_type.as_deref() {
                Some(hint) => hint == "vector",
                None => {
                    data_type::infer(value, &[DataType::Angle, DataType::Vector])
                        == Some(DataType::Vector)
                }
            };
            if vector {
                return Some(vec![AstNode::decl(
                    "rotate",
                    format!("{value} var(--tw-rotate)"),
                )]);
            }
            value.clone()
        }
        UtilityValue::Named { value, .. } => theme
            .resolve(Some(value), &["--rotate".into()], 0)
            .filter(|v| !v.is_empty())
            .or_else(|| numeric::is_positive_integer(value).then(|| format!("{value}deg")))?,
    };
    Some(vec![AstNode::decl(
        "rotate",
        if negative {
            format!("calc({value} * -1)")
        } else {
            value
        },
    )])
}

fn transform(value: Option<&UtilityValue>, modifier: Option<&Modifier>) -> Option<Vec<AstNode>> {
    if modifier.is_some() {
        return None;
    }
    let value = match value {
        None => "var(--tw-rotate-x, ) var(--tw-rotate-y, ) var(--tw-rotate-z, ) var(--tw-skew-x, ) var(--tw-skew-y, )".into(),
        Some(UtilityValue::Arbitrary { value, .. }) => value.clone(),
        _ => return None,
    };
    Some(vec![
        AstNode::AtRoot {
            nodes: [
                "--tw-rotate-x",
                "--tw-rotate-y",
                "--tw-rotate-z",
                "--tw-skew-x",
                "--tw-skew-y",
            ]
            .map(|name| property(name, None, None))
            .into(),
        },
        AstNode::decl("transform", value),
    ])
}

fn stroke(
    value: Option<&UtilityValue>,
    modifier: Option<&Modifier>,
    theme: &Theme,
) -> Option<Vec<AstNode>> {
    match value? {
        UtilityValue::Arbitrary { value, data_type } => {
            let width = match data_type.as_deref() {
                Some(hint) => matches!(hint, "number" | "length" | "percentage"),
                None => matches!(
                    data_type::infer(
                        value,
                        &[
                            DataType::Color,
                            DataType::Number,
                            DataType::Length,
                            DataType::Percentage
                        ]
                    ),
                    Some(DataType::Number | DataType::Length | DataType::Percentage)
                ),
            };
            if width {
                if modifier.is_some() {
                    return None;
                }
                Some(vec![AstNode::decl("stroke-width", value)])
            } else {
                as_color(value, modifier, theme).map(|value| vec![AstNode::decl("stroke", value)])
            }
        }
        UtilityValue::Named { value, .. } => {
            if let Some(color) = theme_color(value, modifier, theme, &["--stroke", "--color"])
                .filter(|s| !s.is_empty())
            {
                return Some(vec![AstNode::decl("stroke", color)]);
            }
            if modifier.is_some() {
                return None;
            }
            theme
                .resolve(Some(value), &["--stroke-width".into()], 0)
                .filter(|s| !s.is_empty())
                .or_else(|| numeric::is_positive_integer(value).then(|| value.clone()))
                .map(|value| vec![AstNode::decl("stroke-width", value)])
        }
    }
}

fn duration(
    value: Option<&UtilityValue>,
    modifier: Option<&Modifier>,
    theme: &Theme,
) -> Option<Vec<AstNode>> {
    if modifier.is_some() {
        return None;
    }
    let value = match value? {
        UtilityValue::Arbitrary { value, .. } => value.clone(),
        UtilityValue::Named { value, fraction } => theme
            .resolve(
                Some(fraction.as_deref().unwrap_or(value)),
                &["--transition-duration".into()],
                0,
            )
            .or_else(|| numeric::is_positive_integer(value).then(|| format!("{value}ms")))?,
    };
    Some(vec![
        AstNode::AtRoot {
            nodes: vec![property("--tw-duration", None, None)],
        },
        AstNode::decl("--tw-duration", &value),
        AstNode::decl("transition-duration", value),
    ])
}

pub(super) fn compile(candidate: &Candidate, theme: &Theme) -> Option<Vec<Vec<AstNode>>> {
    if let CandidateKind::Static { root } = &candidate.kind {
        if root != "container" {
            return None;
        }
        let mut values: Vec<_> = theme
            .namespace("--breakpoint")
            .into_iter()
            .map(|(_, value)| value)
            .collect();
        crate::js_sort::sort_by(&mut values, |left, right| {
            breakpoints::compare(left, right, Direction::Asc)
        });
        let mut nodes = vec![
            AstNode::decl("--tw-sort", "--tw-container-component"),
            AstNode::decl("width", "100%"),
        ];
        nodes.extend(values.into_iter().map(|value| AstNode::AtRule {
            name: "@media".into(),
            params: format!("(width >= {value})"),
            nodes: vec![AstNode::decl("max-width", value)],
        }));
        return Some(vec![nodes]);
    }
    let CandidateKind::Functional {
        root,
        value,
        modifier,
    } = &candidate.kind
    else {
        return None;
    };
    let value = value.as_ref();
    let modifier = modifier.as_ref();
    let result = match root.as_str() {
        "flex" => flex(value, modifier),
        "scale" | "-scale" => scale(value, modifier, root.starts_with('-'), theme),
        "rotate" | "-rotate" => rotate(value, modifier, root.starts_with('-'), theme),
        "transform" => transform(value, modifier),
        "duration" => duration(value, modifier, theme),
        "stroke" => stroke(value, modifier, theme),
        "border" | "border-x" | "border-y" | "border-s" | "border-e" | "border-bs"
        | "border-be" | "border-t" | "border-r" | "border-b" | "border-l" | "outline" => {
            typography_border::border(root, value, modifier, theme)
        }
        "font" => typography_border::font(value, modifier, theme),
        "text" => typography_border::text(value, modifier, theme),
        "decoration" => typography_border::decoration(value, modifier, theme),
        "filter" | "backdrop-filter" => shadows::filter(root, value, modifier),
        "shadow" | "inset-shadow" | "text-shadow" | "drop-shadow" => {
            shadows::shadow(root, value, modifier, theme)
        }
        "ring" | "inset-ring" | "ring-offset" => shadows::ring(root, value, modifier, theme),
        "bg" | "mask" => backgrounds::background(root, value, modifier, theme),
        "bg-linear" | "-bg-linear" | "bg-conic" | "-bg-conic" | "bg-radial" => {
            backgrounds::gradient(root, value, modifier)
        }
        "from" | "via" | "to" => backgrounds::gradient_stop(root, value, modifier, theme),
        "mask-x-from" | "mask-x-to" | "mask-y-from" | "mask-y-to" | "mask-t-from" | "mask-t-to"
        | "mask-r-from" | "mask-r-to" | "mask-b-from" | "mask-b-to" | "mask-l-from"
        | "mask-l-to" | "mask-linear-from" | "mask-linear-to" | "mask-radial-from"
        | "mask-radial-to" | "mask-conic-from" | "mask-conic-to" => {
            backgrounds::mask_stop(root, value, modifier, theme)
        }
        "fill" => match value {
            None => None,
            Some(UtilityValue::Arbitrary { value, .. }) => {
                as_color(value, modifier, theme).map(|value| vec![AstNode::decl("fill", value)])
            }
            Some(UtilityValue::Named { value, .. }) => {
                theme_color(value, modifier, theme, &["--fill", "--color"])
                    .filter(|s| !s.is_empty())
                    .map(|value| vec![AstNode::decl("fill", value)])
            }
        },
        "@container" => {
            let resolved = match value {
                None => Some("inline-size".into()),
                Some(UtilityValue::Arbitrary { value, .. }) => Some(value.clone()),
                Some(UtilityValue::Named { value, .. })
                    if matches!(value.as_str(), "normal" | "size") =>
                {
                    Some(value.clone())
                }
                _ => None,
            };
            resolved.map(|value| {
                let mut nodes = vec![AstNode::decl("container-type", value)];
                if let Some(modifier) = modifier {
                    let (Modifier::Arbitrary { value } | Modifier::Named { value }) = modifier;
                    nodes.push(AstNode::decl("container-name", value));
                }
                nodes
            })
        }
        _ => return None,
    };
    Some(result.into_iter().collect())
}
