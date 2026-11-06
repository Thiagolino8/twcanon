// Port of filter, shadow, and ring matchers in Tailwind CSS 4.3.4 utilities.ts.
use super::{as_color, property, theme_color};
use crate::{
    ast::AstNode,
    candidate::{Modifier, UtilityValue},
    data_type::{self, DataType},
    numeric,
    syntax::{
        is_js_whitespace,
        segment::segment,
        value::{self, Value},
    },
    theme::Theme,
    utilities::{replace_alpha, with_alpha},
};

const FILTER: &str = "var(--tw-blur, ) var(--tw-brightness, ) var(--tw-contrast, ) var(--tw-grayscale, ) var(--tw-hue-rotate, ) var(--tw-invert, ) var(--tw-saturate, ) var(--tw-sepia, ) var(--tw-drop-shadow, )";
const BACKDROP: &str = "var(--tw-backdrop-blur, ) var(--tw-backdrop-brightness, ) var(--tw-backdrop-contrast, ) var(--tw-backdrop-grayscale, ) var(--tw-backdrop-hue-rotate, ) var(--tw-backdrop-invert, ) var(--tw-backdrop-opacity, ) var(--tw-backdrop-saturate, ) var(--tw-backdrop-sepia, )";
const BOX_SHADOW: &str = "var(--tw-inset-shadow), var(--tw-inset-ring-shadow), var(--tw-ring-offset-shadow), var(--tw-ring-shadow), var(--tw-shadow)";

fn filter_properties(backdrop: bool) -> AstNode {
    let mut nodes: Vec<_> = [
        "blur",
        "brightness",
        "contrast",
        "grayscale",
        "hue-rotate",
        "invert",
        "opacity",
        "saturate",
        "sepia",
    ]
    .map(|name| {
        property(
            &format!("--tw-{}{name}", if backdrop { "backdrop-" } else { "" }),
            None,
            None,
        )
    })
    .into();
    if !backdrop {
        nodes.extend([
            property("--tw-drop-shadow", None, None),
            property("--tw-drop-shadow-color", None, None),
            property("--tw-drop-shadow-alpha", Some("100%"), Some("<percentage>")),
            property("--tw-drop-shadow-size", None, None),
        ]);
    }
    AstNode::AtRoot { nodes }
}

fn box_properties() -> AstNode {
    AstNode::AtRoot {
        nodes: vec![
            property("--tw-shadow", Some("0 0 #0000"), None),
            property("--tw-shadow-color", None, None),
            property("--tw-shadow-alpha", Some("100%"), Some("<percentage>")),
            property("--tw-inset-shadow", Some("0 0 #0000"), None),
            property("--tw-inset-shadow-color", None, None),
            property(
                "--tw-inset-shadow-alpha",
                Some("100%"),
                Some("<percentage>"),
            ),
            property("--tw-ring-color", None, None),
            property("--tw-ring-shadow", Some("0 0 #0000"), None),
            property("--tw-inset-ring-color", None, None),
            property("--tw-inset-ring-shadow", Some("0 0 #0000"), None),
            property("--tw-ring-inset", None, None),
            property("--tw-ring-offset-width", Some("0px"), Some("<length>")),
            property("--tw-ring-offset-color", Some("#fff"), None),
            property("--tw-ring-offset-shadow", Some("0 0 #0000"), None),
        ],
    }
}

fn shadow_properties(root: &str) -> AstNode {
    match root {
        "drop-shadow" => filter_properties(false),
        "text-shadow" => AstNode::AtRoot {
            nodes: vec![
                property("--tw-text-shadow-color", None, None),
                property("--tw-text-shadow-alpha", Some("100%"), Some("<percentage>")),
            ],
        },
        _ => box_properties(),
    }
}

// The upstream walker stops after the first color replacement, and never enters functions.
pub(crate) fn replace_colors(input: &str, mut replacement: impl FnMut(&str) -> String) -> String {
    segment(input, b',')
        .into_iter()
        .map(|shadow| {
            let shadow = shadow.trim_matches(is_js_whitespace);
            let mut ast = value::parse(shadow);
            let mut unknown = None;
            let mut unknowns = 0;
            let mut lengths = 0;
            let mut found = None;
            for (index, node) in ast.iter().enumerate() {
                let (text, function) = match node {
                    Value::Separator { .. } => continue,
                    Value::Word { value } => (value, false),
                    Value::Function { value, .. } => (value, true),
                };
                let lower = text.to_lowercase();
                let color = if function {
                    if ["calc", "clamp", "max", "min", "--spacing"].contains(&lower.as_str()) {
                        lengths += 1;
                        continue;
                    }
                    [
                        "color",
                        "color-mix",
                        "contrast-color",
                        "device-cmyk",
                        "hsl",
                        "hsla",
                        "hwb",
                        "lab",
                        "lch",
                        "light-dark",
                        "oklab",
                        "oklch",
                        "rgb",
                        "rgba",
                        "--alpha",
                    ]
                    .contains(&lower.as_str())
                } else {
                    if ["inset", "inherit", "initial", "revert", "unset"].contains(&lower.as_str())
                    {
                        continue;
                    }
                    let digits = lower.strip_prefix('-').unwrap_or(&lower);
                    if digits.as_bytes().first().is_some_and(u8::is_ascii_digit)
                        || digits
                            .strip_prefix('.')
                            .and_then(|s| s.as_bytes().first())
                            .is_some_and(u8::is_ascii_digit)
                    {
                        lengths += 1;
                        continue;
                    }
                    text.starts_with('#') || data_type::is_named_color(text)
                };
                if color {
                    found = Some(index);
                    break;
                }
                unknown = Some(index);
                unknowns += 1;
            }
            let index = match found {
                Some(index) => index,
                None if lengths < 2 => return shadow.into(),
                None if unknowns == 0 => {
                    return format!("{shadow} {}", replacement("currentcolor"));
                }
                None if unknowns == 1 => unknown.unwrap(),
                None => return shadow.into(),
            };
            let color = value::to_css(&ast[index..=index]);
            ast.splice(index..=index, value::parse(&replacement(&color)));
            value::to_css(&ast)
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn alpha_replaced(
    property_name: &str,
    input: &str,
    alpha: Option<&str>,
    root: &str,
    inset: bool,
) -> Vec<AstNode> {
    let inject = |color: &str| format!("var(--tw-{root}-color, {color})");
    let mut requires_fallback = false;
    let mut replace = |input: &str| {
        replace_colors(input, |color| {
            let Some(alpha) = alpha else {
                return inject(color);
            };
            if color.starts_with("current") {
                return inject(&with_alpha(color, alpha));
            }
            if color.starts_with("var(") || alpha.starts_with("var(") {
                requires_fallback = true;
            }
            inject(&replace_alpha(color, alpha))
        })
    };
    let replaced = if root == "drop-shadow" {
        segment(input, b',')
            .into_iter()
            .map(|s| format!("drop-shadow({})", replace(s)))
            .collect::<Vec<_>>()
            .join(" ")
    } else {
        replace(input)
    };
    let prefix = |value: String| {
        if inset {
            segment(&value, b',')
                .into_iter()
                .map(|s| format!("inset {}", s.trim_matches(is_js_whitespace)))
                .collect::<Vec<_>>()
                .join(", ")
        } else {
            value
        }
    };
    if requires_fallback {
        let fallback = if root == "drop-shadow" {
            segment(input, b',')
                .into_iter()
                .map(|s| format!("drop-shadow({})", replace_colors(s, inject)))
                .collect::<Vec<_>>()
                .join(" ")
        } else {
            replace_colors(input, inject)
        };
        vec![
            AstNode::decl(property_name, prefix(fallback)),
            AstNode::AtRule {
                name: "@supports".into(),
                params: "(color: lab(from red l a b))".into(),
                nodes: vec![AstNode::decl(property_name, prefix(replaced))],
            },
        ]
    } else {
        vec![AstNode::decl(property_name, prefix(replaced))]
    }
}

pub(super) fn filter(
    root: &str,
    value: Option<&UtilityValue>,
    modifier: Option<&Modifier>,
) -> Option<Vec<AstNode>> {
    if modifier.is_some() {
        return None;
    }
    let backdrop = root == "backdrop-filter";
    let mut nodes = Vec::new();
    let value = match value {
        None => {
            nodes.push(filter_properties(backdrop));
            if backdrop { BACKDROP } else { FILTER }
        }
        Some(UtilityValue::Arbitrary { value, .. }) => value,
        Some(UtilityValue::Named { value, .. }) if value == "none" => "none",
        _ => return None,
    };
    if backdrop {
        nodes.push(AstNode::decl("-webkit-backdrop-filter", value));
    }
    nodes.push(AstNode::decl(
        if backdrop {
            "backdrop-filter"
        } else {
            "filter"
        },
        value,
    ));
    Some(nodes)
}

pub(super) fn shadow(
    root: &str,
    value: Option<&UtilityValue>,
    modifier: Option<&Modifier>,
    theme: &Theme,
) -> Option<Vec<AstNode>> {
    let alpha = modifier.and_then(|modifier| match modifier {
        Modifier::Arbitrary { value } => Some(value.clone()),
        Modifier::Named { value } => {
            numeric::is_spacing_multiplier(value).then(|| format!("{value}%"))
        }
    });
    let alpha_truthy = alpha.as_deref().is_some_and(|s| !s.is_empty());
    let color_name = format!("--tw-{root}-color");
    let size_name = if root == "text-shadow" {
        "text-shadow".into()
    } else if root == "drop-shadow" {
        "--tw-drop-shadow-size".into()
    } else {
        format!("--tw-{root}")
    };
    let color_nodes = |color: String| {
        let mut nodes = vec![shadow_properties(root), AstNode::decl(&color_name, color)];
        if root == "drop-shadow" {
            nodes.push(AstNode::decl(
                "--tw-drop-shadow",
                "var(--tw-drop-shadow-size)",
            ));
        }
        nodes
    };
    let finish = |input: &str, inset: bool, resolved: Option<String>| -> Option<Vec<AstNode>> {
        if modifier.is_some() && !alpha_truthy {
            return None;
        }
        let mut nodes = vec![
            shadow_properties(root),
            AstNode::Declaration {
                identity: Default::default(),
                property: format!("--tw-{root}-alpha").into(),
                value: alpha.clone(),
                important: false,
            },
        ];
        nodes.extend(alpha_replaced(
            &size_name,
            input,
            alpha.as_deref(),
            root,
            inset,
        ));
        match root {
            "drop-shadow" => {
                let size = resolved
                    .map(|value| {
                        segment(&value, b',')
                            .into_iter()
                            .map(|s| format!("drop-shadow({s})"))
                            .collect::<Vec<_>>()
                            .join(" ")
                    })
                    .unwrap_or_else(|| "var(--tw-drop-shadow-size)".into());
                nodes.push(AstNode::decl("--tw-drop-shadow", size));
                nodes.push(AstNode::decl("filter", FILTER));
            }
            "text-shadow" => {}
            _ => nodes.push(AstNode::decl("box-shadow", BOX_SHADOW)),
        }
        Some(nodes)
    };
    match value {
        None => {
            let input = theme.get(&[format!("--{root}")])?;
            let resolved = if root == "drop-shadow" {
                Some(theme.resolve(None, &["--drop-shadow".into()], 0)?)
            } else {
                None
            };
            finish(input, false, resolved)
        }
        Some(UtilityValue::Arbitrary { value, data_type }) => {
            let is_color = data_type.as_deref().map_or_else(
                || data_type::infer(value, &[DataType::Color]) == Some(DataType::Color),
                |hint| hint == "color",
            );
            if is_color {
                Some(color_nodes(with_alpha(
                    &as_color(value, modifier, theme)?,
                    &format!("var(--tw-{root}-alpha)"),
                )))
            } else {
                finish(value, root == "inset-shadow", None)
            }
        }
        Some(UtilityValue::Named { value, .. }) => {
            if root != "drop-shadow" {
                match value.as_str() {
                    "none" => {
                        if modifier.is_some() {
                            return None;
                        }
                        let mut nodes = vec![
                            shadow_properties(root),
                            AstNode::decl(
                                &size_name,
                                if root == "text-shadow" {
                                    "none"
                                } else if root == "inset-shadow" {
                                    "inset 0 0 #0000"
                                } else {
                                    "0 0 #0000"
                                },
                            ),
                        ];
                        if root != "text-shadow" {
                            nodes.push(AstNode::decl("box-shadow", BOX_SHADOW));
                        }
                        return Some(nodes);
                    }
                    "inherit" => {
                        if modifier.is_some() {
                            return None;
                        }
                        return Some(color_nodes("inherit".into()));
                    }
                    _ => {}
                }
            }
            if let Some(input) = theme
                .get(&[format!("--{root}-{value}")])
                .filter(|s| !s.is_empty())
            {
                if root == "drop-shadow" {
                    if let Some(resolved) = theme
                        .resolve(Some(value), &["--drop-shadow".into()], 0)
                        .filter(|s| !s.is_empty())
                    {
                        return finish(input, false, (!alpha_truthy).then_some(resolved));
                    }
                } else {
                    return finish(input, false, None);
                }
            }
            let namespace = if matches!(root, "shadow" | "inset-shadow") {
                "--box-shadow-color".into()
            } else {
                format!("--{root}-color")
            };
            let color = theme_color(value, modifier, theme, &[&namespace, "--color"])
                .filter(|s| !s.is_empty())?;
            Some(color_nodes(
                if root == "drop-shadow" && color == "inherit" {
                    color
                } else {
                    with_alpha(&color, &format!("var(--tw-{root}-alpha)"))
                },
            ))
        }
    }
}

pub(super) fn ring(
    root: &str,
    value: Option<&UtilityValue>,
    modifier: Option<&Modifier>,
    theme: &Theme,
) -> Option<Vec<AstNode>> {
    let color = |value: String| vec![AstNode::decl(format!("--tw-{root}-color"), value)];
    let width = |value: String| {
        if root == "ring-offset" {
            return vec![
                AstNode::decl("--tw-ring-offset-width", value),
                AstNode::decl(
                    "--tw-ring-offset-shadow",
                    "var(--tw-ring-inset, ) 0 0 0 var(--tw-ring-offset-width) var(--tw-ring-offset-color)",
                ),
            ];
        }
        let shadow = if root == "inset-ring" {
            format!("inset 0 0 0 {value} var(--tw-inset-ring-color, currentcolor)")
        } else {
            format!(
                "var(--tw-ring-inset, ) 0 0 0 calc({value} + var(--tw-ring-offset-width)) var(--tw-ring-color, {})",
                theme
                    .get(&["--default-ring-color".into()])
                    .unwrap_or("currentcolor")
            )
        };
        vec![
            box_properties(),
            AstNode::decl(format!("--tw-{root}-shadow"), shadow),
            AstNode::decl("box-shadow", BOX_SHADOW),
        ]
    };
    match value {
        None => {
            if root == "ring-offset" || modifier.is_some() {
                return None;
            }
            Some(width(if root == "inset-ring" {
                "1px".into()
            } else {
                theme
                    .get(&["--default-ring-width".into()])
                    .unwrap_or("1px")
                    .into()
            }))
        }
        Some(UtilityValue::Arbitrary { value, data_type }) => {
            let is_length = data_type.as_deref().map_or_else(
                || {
                    data_type::infer(value, &[DataType::Color, DataType::Length])
                        == Some(DataType::Length)
                },
                |hint| hint == "length",
            );
            if is_length {
                if modifier.is_some() {
                    return None;
                }
                Some(width(value.clone()))
            } else {
                as_color(value, modifier, theme).map(color)
            }
        }
        Some(UtilityValue::Named { value, .. }) => {
            if root == "ring-offset"
                && let Some(value) = theme
                    .resolve(Some(value), &["--ring-offset-width".into()], 0)
                    .filter(|s| !s.is_empty())
                    .or_else(|| numeric::is_positive_integer(value).then(|| format!("{value}px")))
            {
                if modifier.is_some() {
                    return None;
                }
                return Some(width(value));
            }
            if let Some(color_value) = theme_color(
                value,
                modifier,
                theme,
                &[
                    if root == "ring-offset" {
                        "--ring-offset-color"
                    } else {
                        "--ring-color"
                    },
                    "--color",
                ],
            )
            .filter(|s| !s.is_empty())
            {
                return Some(color(color_value));
            }
            if root == "ring-offset" || modifier.is_some() {
                return None;
            }
            theme
                .resolve(Some(value), &["--ring-width".into()], 0)
                .or_else(|| numeric::is_positive_integer(value).then(|| format!("{value}px")))
                .filter(|s| !s.is_empty())
                .map(width)
        }
    }
}
