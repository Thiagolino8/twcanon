// Port of manual border and typography matchers in Tailwind CSS 4.3.4.
use super::{as_color, property, theme_color, value_type};
use crate::{
    ast::AstNode,
    candidate::{Modifier, UtilityValue},
    data_type::DataType,
    numeric,
    theme::Theme,
};

pub(super) fn border(
    root: &str,
    value: Option<&UtilityValue>,
    modifier: Option<&Modifier>,
    theme: &Theme,
) -> Option<Vec<AstNode>> {
    let (prefix, namespace) = match root {
        "border" => ("border", "--border"),
        "border-x" => ("border-inline", "--border"),
        "border-y" => ("border-block", "--border"),
        "border-s" => ("border-inline-start", "--border"),
        "border-e" => ("border-inline-end", "--border"),
        "border-bs" => ("border-block-start", "--border"),
        "border-be" => ("border-block-end", "--border"),
        "border-t" => ("border-top", "--border"),
        "border-r" => ("border-right", "--border"),
        "border-b" => ("border-bottom", "--border"),
        "border-l" => ("border-left", "--border"),
        "outline" => ("outline", "--outline"),
        _ => return None,
    };
    let width = |value: String| {
        vec![
            AstNode::AtRoot {
                nodes: vec![property(
                    &format!("--tw-{}-style", &namespace[2..]),
                    Some("solid"),
                    None,
                )],
            },
            AstNode::decl(
                format!("{prefix}-style"),
                format!("var(--tw-{}-style)", &namespace[2..]),
            ),
            AstNode::decl(format!("{prefix}-width"), value),
        ]
    };
    let color = |value: String| vec![AstNode::decl(format!("{prefix}-color"), value)];
    match value {
        None => {
            if modifier.is_some() {
                return None;
            }
            Some(width(
                theme
                    .get(&[format!("--default-{}-width", &namespace[2..])])
                    .unwrap_or("1px")
                    .into(),
            ))
        }
        Some(UtilityValue::Arbitrary { value, data_type }) => {
            let types = if root == "outline" {
                &[
                    DataType::Color,
                    DataType::Length,
                    DataType::Number,
                    DataType::Percentage,
                ][..]
            } else {
                &[DataType::Color, DataType::LineWidth, DataType::Length][..]
            };
            let kind = value_type(value, data_type.as_deref(), types);
            let is_width = if root == "outline" {
                matches!(kind, Some("length" | "number" | "percentage"))
            } else {
                matches!(kind, Some("line-width" | "length"))
            };
            if is_width {
                if modifier.is_some() {
                    return None;
                }
                Some(width(value.clone()))
            } else {
                as_color(value, modifier, theme).map(color)
            }
        }
        Some(UtilityValue::Named { value, .. }) => {
            if let Some(value) = theme_color(
                value,
                modifier,
                theme,
                &[&format!("{namespace}-color"), "--color"],
            )
            .filter(|s| !s.is_empty())
            {
                return Some(color(value));
            }
            if modifier.is_some() {
                return None;
            }
            theme
                .resolve(Some(value), &[format!("{namespace}-width")], 0)
                .filter(|s| !s.is_empty())
                .or_else(|| numeric::is_positive_integer(value).then(|| format!("{value}px")))
                .map(width)
        }
    }
}

pub(super) fn decoration(
    value: Option<&UtilityValue>,
    modifier: Option<&Modifier>,
    theme: &Theme,
) -> Option<Vec<AstNode>> {
    match value? {
        UtilityValue::Arbitrary { value, data_type } => {
            if matches!(
                value_type(
                    value,
                    data_type.as_deref(),
                    &[DataType::Color, DataType::Length, DataType::Percentage]
                ),
                Some("length" | "percentage")
            ) {
                if modifier.is_some() {
                    return None;
                }
                Some(vec![AstNode::decl("text-decoration-thickness", value)])
            } else {
                as_color(value, modifier, theme)
                    .map(|v| vec![AstNode::decl("text-decoration-color", v)])
            }
        }
        UtilityValue::Named { value, .. } => {
            if let Some(width) = theme
                .resolve(Some(value), &["--text-decoration-thickness".into()], 0)
                .filter(|s| !s.is_empty())
                .or_else(|| numeric::is_positive_integer(value).then(|| format!("{value}px")))
            {
                if modifier.is_some() {
                    return None;
                }
                return Some(vec![AstNode::decl("text-decoration-thickness", width)]);
            }
            theme_color(
                value,
                modifier,
                theme,
                &["--text-decoration-color", "--color"],
            )
            .filter(|s| !s.is_empty())
            .map(|v| vec![AstNode::decl("text-decoration-color", v)])
        }
    }
}

pub(super) fn font(
    value: Option<&UtilityValue>,
    modifier: Option<&Modifier>,
    theme: &Theme,
) -> Option<Vec<AstNode>> {
    if modifier.is_some() {
        return None;
    }
    let weight = match value? {
        UtilityValue::Arbitrary { value, data_type } => {
            if matches!(
                value_type(
                    value,
                    data_type.as_deref(),
                    &[
                        DataType::Number,
                        DataType::GenericName,
                        DataType::FamilyName
                    ]
                ),
                Some("generic-name" | "family-name")
            ) {
                return Some(vec![AstNode::decl("font-family", value)]);
            }
            value.clone()
        }
        UtilityValue::Named { value, .. } => {
            if let Some((families, options)) = theme.resolve_with(
                value,
                &["--font".into()],
                &[
                    "--font-feature-settings".into(),
                    "--font-variation-settings".into(),
                ],
            ) {
                return Some(vec![
                    AstNode::decl("font-family", families),
                    AstNode::Declaration {
                        identity: Default::default(),
                        property: "font-feature-settings".into(),
                        value: options.get("--font-feature-settings").cloned(),
                        important: false,
                    },
                    AstNode::Declaration {
                        identity: Default::default(),
                        property: "font-variation-settings".into(),
                        value: options.get("--font-variation-settings").cloned(),
                        important: false,
                    },
                ]);
            }
            theme
                .resolve(Some(value), &["--font-weight".into()], 0)
                .filter(|s| !s.is_empty())?
        }
    };
    Some(vec![
        AstNode::AtRoot {
            nodes: vec![property("--tw-font-weight", None, None)],
        },
        AstNode::decl("--tw-font-weight", &weight),
        AstNode::decl("font-weight", weight),
    ])
}

fn leading(modifier: &Modifier, theme: &Theme) -> Option<String> {
    let (value, mut resolved) = match modifier {
        Modifier::Arbitrary { value } => (value, Some(value.clone())),
        Modifier::Named { value } => (value, theme.resolve(Some(value), &["--leading".into()], 0)),
    };
    if resolved.as_ref().is_none_or(|s| s.is_empty()) && numeric::is_spacing_multiplier(value) {
        theme
            .resolve(None, &["--spacing".into()], 0)
            .filter(|s| !s.is_empty())?;
        resolved = Some(format!("--spacing({value})"));
    }
    if resolved.as_ref().is_none_or(|s| s.is_empty()) && value == "none" {
        resolved = Some("1".into());
    }
    resolved.filter(|s| !s.is_empty())
}

pub(super) fn text(
    value: Option<&UtilityValue>,
    modifier: Option<&Modifier>,
    theme: &Theme,
) -> Option<Vec<AstNode>> {
    match value? {
        UtilityValue::Arbitrary { value, data_type } => {
            if matches!(
                value_type(
                    value,
                    data_type.as_deref(),
                    &[
                        DataType::Color,
                        DataType::Length,
                        DataType::Percentage,
                        DataType::AbsoluteSize,
                        DataType::RelativeSize
                    ]
                ),
                Some("size" | "length" | "percentage" | "absolute-size" | "relative-size")
            ) {
                let mut nodes = vec![AstNode::decl("font-size", value)];
                if let Some(modifier) = modifier {
                    nodes.push(AstNode::decl("line-height", leading(modifier, theme)?));
                }
                Some(nodes)
            } else {
                as_color(value, modifier, theme).map(|v| vec![AstNode::decl("color", v)])
            }
        }
        UtilityValue::Named { value, .. } => {
            if let Some(value) = theme_color(value, modifier, theme, &["--text-color", "--color"])
                .filter(|s| !s.is_empty())
            {
                return Some(vec![AstNode::decl("color", value)]);
            }
            let (size, options) = theme.resolve_with(
                value,
                &["--text".into()],
                &[
                    "--line-height".into(),
                    "--letter-spacing".into(),
                    "--font-weight".into(),
                ],
            )?;
            let mut nodes = vec![AstNode::decl("font-size", size)];
            if let Some(modifier) = modifier {
                nodes.push(AstNode::decl("line-height", leading(modifier, theme)?));
            } else {
                for (property, key, variable) in [
                    ("line-height", "--line-height", "--tw-leading"),
                    ("letter-spacing", "--letter-spacing", "--tw-tracking"),
                    ("font-weight", "--font-weight", "--tw-font-weight"),
                ] {
                    nodes.push(AstNode::Declaration {
                        identity: Default::default(),
                        property: property.into(),
                        value: options
                            .get(key)
                            .filter(|s| !s.is_empty())
                            .map(|value| format!("var({variable}, {value})")),
                        important: false,
                    });
                }
            }
            Some(nodes)
        }
    }
}
