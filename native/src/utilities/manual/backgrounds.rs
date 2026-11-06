// Port of background, gradient, and mask matchers in Tailwind CSS 4.3.4.
use super::{as_color, property, theme_color, value_type};
use crate::{
    ast::AstNode,
    candidate::{Modifier, UtilityValue},
    data_type::{self, DataType},
    numeric,
    theme::Theme,
};

pub(super) fn background(
    root: &str,
    value: Option<&UtilityValue>,
    modifier: Option<&Modifier>,
    theme: &Theme,
) -> Option<Vec<AstNode>> {
    let mask = root == "mask";
    match value? {
        UtilityValue::Arbitrary { value, data_type } => {
            if mask && modifier.is_some() {
                return None;
            }
            let types = if mask {
                &[
                    DataType::Image,
                    DataType::Percentage,
                    DataType::Position,
                    DataType::BgSize,
                    DataType::Length,
                    DataType::Url,
                ][..]
            } else {
                &[
                    DataType::Image,
                    DataType::Color,
                    DataType::Percentage,
                    DataType::Position,
                    DataType::BgSize,
                    DataType::Length,
                    DataType::Url,
                ][..]
            };
            let suffix = match value_type(value, data_type.as_deref(), types) {
                Some("percentage" | "position") => "position",
                Some("bg-size" | "length" | "size") => "size",
                Some("image" | "url") => "image",
                _ if mask => "image",
                _ => {
                    return as_color(value, modifier, theme)
                        .map(|v| vec![AstNode::decl("background-color", v)]);
                }
            };
            if modifier.is_some() {
                return None;
            }
            Some(vec![AstNode::decl(
                format!("{}-{suffix}", if mask { "mask" } else { "background" }),
                value,
            )])
        }
        UtilityValue::Named { value, .. } => {
            if mask {
                return None;
            }
            if let Some(color) =
                theme_color(value, modifier, theme, &["--background-color", "--color"])
                    .filter(|s| !s.is_empty())
            {
                return Some(vec![AstNode::decl("background-color", color)]);
            }
            if modifier.is_some() {
                return None;
            }
            theme
                .resolve(Some(value), &["--background-image".into()], 0)
                .filter(|s| !s.is_empty())
                .map(|value| vec![AstNode::decl("background-image", value)])
        }
    }
}

fn interpolation(modifier: Option<&Modifier>) -> String {
    match modifier {
        None => "in oklab".into(),
        Some(Modifier::Arbitrary { value }) => value.clone(),
        Some(Modifier::Named { value })
            if ["longer", "shorter", "increasing", "decreasing"].contains(&value.as_str()) =>
        {
            format!("in oklch {value} hue")
        }
        Some(Modifier::Named { value }) => format!("in {value}"),
    }
}

pub(super) fn gradient(
    root: &str,
    value: Option<&UtilityValue>,
    modifier: Option<&Modifier>,
) -> Option<Vec<AstNode>> {
    let negative = root.starts_with('-');
    let kind = root.trim_start_matches('-').strip_prefix("bg-")?;
    if let Some(UtilityValue::Arbitrary { value, data_type }) = value {
        if modifier.is_some() {
            return None;
        }
        let value = if kind == "linear" && negative {
            if value_type(value, data_type.as_deref(), &[DataType::Angle]) != Some("angle") {
                return None;
            }
            format!("calc({value} * -1)")
        } else {
            value.clone()
        };
        return Some(vec![
            AstNode::decl("--tw-gradient-position", &value),
            AstNode::decl(
                "background-image",
                format!("{kind}-gradient(var(--tw-gradient-stops,{value}))"),
            ),
        ]);
    }
    let position = match value {
        None if kind == "conic" || kind == "radial" => interpolation(modifier),
        Some(UtilityValue::Named { value, .. }) if kind == "linear" || kind == "conic" => {
            let direction = if negative {
                None
            } else {
                match value.as_str() {
                    "to-t" => Some("to top"),
                    "to-tr" => Some("to top right"),
                    "to-r" => Some("to right"),
                    "to-br" => Some("to bottom right"),
                    "to-b" => Some("to bottom"),
                    "to-bl" => Some("to bottom left"),
                    "to-l" => Some("to left"),
                    "to-tl" => Some("to top left"),
                    _ => None,
                }
            };
            let value = if let Some(direction) = direction.filter(|_| kind == "linear") {
                direction.into()
            } else if numeric::is_positive_integer(value) {
                if negative {
                    format!("calc({value}deg * -1)")
                } else {
                    format!("{value}deg")
                }
            } else {
                return None;
            };
            if kind == "linear" {
                return Some(vec![
                    AstNode::decl("--tw-gradient-position", &value),
                    AstNode::AtRule {
                        name: "@supports".into(),
                        params: "(background-image: linear-gradient(in lab, red, red))".into(),
                        nodes: vec![AstNode::decl(
                            "--tw-gradient-position",
                            format!("{value} {}", interpolation(modifier)),
                        )],
                    },
                    AstNode::decl(
                        "background-image",
                        "linear-gradient(var(--tw-gradient-stops))",
                    ),
                ]);
            }
            format!("from {value} {}", interpolation(modifier))
        }
        _ => return None,
    };
    Some(vec![
        AstNode::decl("--tw-gradient-position", position),
        AstNode::decl(
            "background-image",
            format!("{kind}-gradient(var(--tw-gradient-stops))"),
        ),
    ])
}

fn gradient_properties() -> AstNode {
    AstNode::AtRoot {
        nodes: vec![
            property("--tw-gradient-position", None, None),
            property("--tw-gradient-from", Some("#0000"), Some("<color>")),
            property("--tw-gradient-via", Some("#0000"), Some("<color>")),
            property("--tw-gradient-to", Some("#0000"), Some("<color>")),
            property("--tw-gradient-stops", None, None),
            property("--tw-gradient-via-stops", None, None),
            property(
                "--tw-gradient-from-position",
                Some("0%"),
                Some("<length-percentage>"),
            ),
            property(
                "--tw-gradient-via-position",
                Some("50%"),
                Some("<length-percentage>"),
            ),
            property(
                "--tw-gradient-to-position",
                Some("100%"),
                Some("<length-percentage>"),
            ),
        ],
    }
}

pub(super) fn gradient_stop(
    root: &str,
    value: Option<&UtilityValue>,
    modifier: Option<&Modifier>,
    theme: &Theme,
) -> Option<Vec<AstNode>> {
    let (value, position) = match value? {
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
                (value.clone(), true)
            } else {
                (as_color(value, modifier, theme)?, false)
            }
        }
        UtilityValue::Named { value, .. } => {
            if let Some(color) =
                theme_color(value, modifier, theme, &["--background-color", "--color"])
                    .filter(|s| !s.is_empty())
            {
                (color, false)
            } else {
                if modifier.is_some() {
                    return None;
                }
                (
                    theme
                        .resolve(Some(value), &["--gradient-color-stop-positions".into()], 0)
                        .filter(|s| !s.is_empty())
                        .or_else(|| {
                            value
                                .strip_suffix('%')
                                .filter(|s| numeric::is_positive_integer(s))
                                .map(|_| value.clone())
                        })?,
                    true,
                )
            }
        }
    };
    if position {
        return Some(vec![
            gradient_properties(),
            AstNode::decl(format!("--tw-gradient-{root}-position"), value),
        ]);
    }
    let mut nodes = vec![
        gradient_properties(),
        AstNode::decl("--tw-sort", format!("--tw-gradient-{root}")),
        AstNode::decl(format!("--tw-gradient-{root}"), value),
    ];
    if root == "via" {
        nodes.push(AstNode::decl("--tw-gradient-via-stops", "var(--tw-gradient-position), var(--tw-gradient-from) var(--tw-gradient-from-position), var(--tw-gradient-via) var(--tw-gradient-via-position), var(--tw-gradient-to) var(--tw-gradient-to-position)"));
        nodes.push(AstNode::decl(
            "--tw-gradient-stops",
            "var(--tw-gradient-via-stops)",
        ));
    } else {
        nodes.push(AstNode::decl("--tw-gradient-stops", "var(--tw-gradient-via-stops, var(--tw-gradient-position), var(--tw-gradient-from) var(--tw-gradient-from-position), var(--tw-gradient-to) var(--tw-gradient-to-position))"));
    }
    Some(nodes)
}

const MASK_IMAGE: &str = "var(--tw-mask-linear), var(--tw-mask-radial), var(--tw-mask-conic)";

fn mask_gradient_properties() -> AstNode {
    AstNode::AtRoot {
        nodes: ["linear", "radial", "conic"]
            .map(|kind| {
                property(
                    &format!("--tw-mask-{kind}"),
                    Some("linear-gradient(#fff, #fff)"),
                    None,
                )
            })
            .into(),
    }
}

fn mask_shape_properties(shape: &str) -> AstNode {
    let fields = match shape {
        "edge" => vec![
            ("left", "linear-gradient(#fff, #fff)"),
            ("right", "linear-gradient(#fff, #fff)"),
            ("bottom", "linear-gradient(#fff, #fff)"),
            ("top", "linear-gradient(#fff, #fff)"),
        ],
        "radial" => vec![
            ("from-position", "0%"),
            ("to-position", "100%"),
            ("from-color", "black"),
            ("to-color", "transparent"),
            ("shape", "ellipse"),
            ("size", "farthest-corner"),
            ("position", "center"),
        ],
        _ => vec![
            ("position", "0deg"),
            ("from-position", "0%"),
            ("to-position", "100%"),
            ("from-color", "black"),
            ("to-color", "transparent"),
        ],
    };
    AstNode::AtRoot {
        nodes: fields
            .into_iter()
            .map(|(key, initial)| {
                property(
                    &format!(
                        "--tw-mask-{}{key}",
                        if shape == "edge" {
                            String::new()
                        } else {
                            format!("{shape}-")
                        }
                    ),
                    Some(initial),
                    None,
                )
            })
            .collect(),
    }
}

pub(super) fn mask_stop(
    root: &str,
    value: Option<&UtilityValue>,
    modifier: Option<&Modifier>,
    theme: &Theme,
) -> Option<Vec<AstNode>> {
    let (value, color) = match value? {
        UtilityValue::Arbitrary { value, data_type } => {
            match value_type(
                value,
                data_type.as_deref(),
                &[DataType::Length, DataType::Percentage, DataType::Color],
            ) {
                Some("color") => (as_color(value, modifier, theme)?, true),
                kind => {
                    if modifier.is_some() {
                        return None;
                    }
                    if kind == Some("percentage")
                        && !numeric::is_positive_integer(
                            value.get(..value.char_indices().next_back()?.0)?,
                        )
                    {
                        return None;
                    }
                    (value.clone(), false)
                }
            }
        }
        UtilityValue::Named { value, .. } => {
            if let Some(color) =
                theme_color(value, modifier, theme, &["--background-color", "--color"])
                    .filter(|s| !s.is_empty())
            {
                (color, true)
            } else {
                if modifier.is_some() {
                    return None;
                }
                match data_type::infer(value, &[DataType::Number, DataType::Percentage])? {
                    DataType::Number => {
                        theme
                            .resolve(None, &["--spacing".into()], 0)
                            .filter(|s| !s.is_empty())?;
                        if !numeric::is_spacing_multiplier(value) {
                            return None;
                        }
                        (format!("--spacing({value})"), false)
                    }
                    DataType::Percentage
                        if value
                            .strip_suffix('%')
                            .is_some_and(numeric::is_positive_integer) =>
                    {
                        (value.clone(), false)
                    }
                    _ => return None,
                }
            }
        }
    };
    let (shape, stop) = root.strip_prefix("mask-")?.rsplit_once('-')?;
    let edge = ["x", "y", "t", "r", "b", "l"].contains(&shape);
    let mut nodes = vec![
        mask_gradient_properties(),
        mask_shape_properties(if edge { "edge" } else { shape }),
        AstNode::decl("mask-image", MASK_IMAGE),
        AstNode::decl("mask-composite", "intersect"),
    ];
    let field = if color { "color" } else { "position" };
    if edge {
        nodes.push(AstNode::decl(
            "--tw-mask-linear",
            "var(--tw-mask-left), var(--tw-mask-right), var(--tw-mask-bottom), var(--tw-mask-top)",
        ));
        for (name, active) in [
            ("top", shape == "t" || shape == "y"),
            ("right", shape == "r" || shape == "x"),
            ("bottom", shape == "b" || shape == "y"),
            ("left", shape == "l" || shape == "x"),
        ] {
            if !active {
                continue;
            }
            nodes.push(AstNode::decl(format!("--tw-mask-{name}"), format!("linear-gradient(to {name}, var(--tw-mask-{name}-from-color) var(--tw-mask-{name}-from-position), var(--tw-mask-{name}-to-color) var(--tw-mask-{name}-to-position))")));
            nodes.push(AstNode::AtRoot {
                nodes: [
                    ("from-position", "0%"),
                    ("to-position", "100%"),
                    ("from-color", "black"),
                    ("to-color", "transparent"),
                ]
                .map(|(key, initial)| {
                    property(&format!("--tw-mask-{name}-{key}"), Some(initial), None)
                })
                .into(),
            });
            nodes.push(AstNode::decl(
                format!("--tw-mask-{name}-{stop}-{field}"),
                &value,
            ));
        }
    } else {
        let position = match shape {
            "radial" => "var(--tw-mask-radial-shape) var(--tw-mask-radial-size) at var(--tw-mask-radial-position)".into(),
            "conic" => "from var(--tw-mask-conic-position)".into(),
            _ => format!("var(--tw-mask-{shape}-position)"),
        };
        nodes.push(AstNode::decl(format!("--tw-mask-{shape}-stops"), format!("{position}, var(--tw-mask-{shape}-from-color) var(--tw-mask-{shape}-from-position), var(--tw-mask-{shape}-to-color) var(--tw-mask-{shape}-to-position)")));
        nodes.push(AstNode::decl(
            format!("--tw-mask-{shape}"),
            format!("{shape}-gradient(var(--tw-mask-{shape}-stops))"),
        ));
        nodes.push(AstNode::decl(
            format!("--tw-mask-{shape}-{stop}-{field}"),
            value,
        ));
    }
    Some(nodes)
}
