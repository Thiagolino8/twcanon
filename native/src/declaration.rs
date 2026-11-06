// Port of Tailwind CSS 4.3.4 expand-declaration.ts and constant-fold-declaration.ts.
// See LICENSE.tailwind.
use crate::{
    ast::AstNode,
    data_type::is_length,
    numeric,
    syntax::{
        segment::segment,
        value::{self, Value},
    },
};

pub fn expand(node: &AstNode, logical_to_physical: bool) -> Option<Vec<AstNode>> {
    let AstNode::Declaration {
        property,
        value: Some(value),
        important,
        ..
    } = node
    else {
        return None;
    };
    let pair = if logical_to_physical {
        match property.as_str() {
            "inset-block" => Some(("top", "bottom")),
            "inset-inline" => Some(("left", "right")),
            "margin-block" => Some(("margin-top", "margin-bottom")),
            "margin-inline" => Some(("margin-left", "margin-right")),
            "padding-block" => Some(("padding-top", "padding-bottom")),
            "padding-inline" => Some(("padding-left", "padding-right")),
            "scroll-margin-block" => Some(("scroll-margin-top", "scroll-margin-bottom")),
            "scroll-margin-inline" => Some(("scroll-margin-left", "scroll-margin-right")),
            "scroll-padding-block" => Some(("scroll-padding-top", "scroll-padding-bottom")),
            "scroll-padding-inline" => Some(("scroll-padding-left", "scroll-padding-right")),
            _ => None,
        }
    } else {
        None
    };
    let pair = pair.or(match property.as_str() {
        "gap" => Some(("row-gap", "column-gap")),
        "overflow" => Some(("overflow-x", "overflow-y")),
        "overscroll-behavior" => Some(("overscroll-behavior-x", "overscroll-behavior-y")),
        _ => None,
    });
    let make = |property: String, value: &str| AstNode::Declaration {
        identity: Default::default(),
        property: property.into(),
        value: Some(value.into()),
        important: *important,
    };
    if let Some((left, right)) = pair {
        let args = segment(value, b' ');
        if args.len() > 2 {
            return None;
        }
        return Some(vec![
            make(left.into(), args[0]),
            make(right.into(), args[args.len() - 1]),
        ]);
    }
    if logical_to_physical {
        let properties = match property.as_str() {
            "border-block" => Some(("border-bottom", "border-top")),
            "border-inline" => Some(("border-left", "border-right")),
            "border-block-color" => Some(("border-bottom-color", "border-top-color")),
            "border-inline-color" => Some(("border-left-color", "border-right-color")),
            "border-block-style" => Some(("border-bottom-style", "border-top-style")),
            "border-inline-style" => Some(("border-left-style", "border-right-style")),
            "border-block-width" => Some(("border-bottom-width", "border-top-width")),
            "border-inline-width" => Some(("border-left-width", "border-right-width")),
            _ => None,
        };
        if let Some((left, right)) = properties {
            return Some(vec![make(left.into(), value), make(right.into(), value)]);
        }
    }
    let (prefix, suffix) = match property.as_str() {
        "inset" => ("", ""),
        "margin" => ("margin-", ""),
        "padding" => ("padding-", ""),
        "scroll-margin" => ("scroll-margin-", ""),
        "scroll-padding" => ("scroll-padding-", ""),
        "border-width" => ("border-", "-width"),
        "border-style" => ("border-", "-style"),
        "border-color" => ("border-", "-color"),
        _ => return None,
    };
    let args = segment(value, b' ');
    let indices = match args.len() {
        1 => [0, 0, 0, 0],
        2 => [0, 1, 0, 1],
        3 => [0, 1, 2, 1],
        4 => [0, 1, 2, 3],
        _ => return None,
    };
    Some(
        ["top", "right", "bottom", "left"]
            .iter()
            .zip(indices)
            .map(|(side, index)| make(format!("{prefix}{side}{suffix}"), args[index]))
            .collect(),
    )
}

fn canonical_dimension(input: &str, rem: Option<f64>, normalize_unit: bool) -> Option<String> {
    let (value, unit) = numeric::dimension(input)?;
    let Some(unit) = unit else {
        return Some(numeric::format(value));
    };
    if value == 0.0 && is_length(input) {
        return Some(if normalize_unit {
            "0".into()
        } else {
            format!("0{unit}")
        });
    }
    if !normalize_unit {
        return Some(input.into());
    }
    let (value, unit) = match unit.to_ascii_lowercase().as_str() {
        "in" => (value * 96.0, "px"),
        "cm" => (value * 96.0 / 2.54, "px"),
        "mm" => (value * 96.0 / 2.54 / 10.0, "px"),
        "q" => (value * 96.0 / 2.54 / 10.0 / 4.0, "px"),
        "pc" => (value * 96.0 / 6.0, "px"),
        "pt" => (value * 96.0 / 72.0, "px"),
        "rem" => (value * rem?, "px"),
        "grad" => (value * 0.9, "deg"),
        "rad" => (value * 180.0 / std::f64::consts::PI, "deg"),
        "turn" => (value * 360.0, "deg"),
        "ms" => (value / 1000.0, "s"),
        "khz" => (value * 1000.0, "hz"),
        _ => (value, unit),
    };
    Some(format!("{}{unit}", numeric::format(value)))
}

fn dimension(node: &Value) -> Option<(f64, Option<&str>)> {
    match node {
        Value::Word { value } => numeric::dimension(value),
        _ => None,
    }
}

fn fold_node(
    node: &Value,
    parent_function: bool,
    rem: Option<f64>,
    normalize_unit: bool,
) -> Option<Value> {
    match node {
        Value::Word { value } if value != "0" => {
            let canonical = canonical_dimension(value, rem, normalize_unit)?;
            if canonical == *value {
                return None;
            }
            if canonical == "0" && parent_function {
                return Some(Value::word(canonical_dimension(value, rem, false)?));
            }
            Some(Value::word(canonical))
        }
        Value::Function { value, nodes }
            if (value == "calc" || value.is_empty()) && nodes.len() == 5 =>
        {
            let Value::Word { value: operator } = &nodes[2] else {
                return None;
            };
            let lhs_node = &nodes[0];
            let rhs_node = &nodes[4];
            let lhs = dimension(lhs_node);
            let rhs = dimension(rhs_node);
            if operator == "*" {
                if lhs == Some((0.0, None)) || rhs == Some((0.0, None)) {
                    return Some(Value::word("0"));
                }
                if let Some((0.0, Some(unit))) = lhs
                    && rhs.is_some_and(|(_, u)| u.is_none())
                {
                    return Some(Value::word(if parent_function {
                        format!("0{unit}")
                    } else {
                        "0".into()
                    }));
                }
                if let Some((0.0, Some(unit))) = rhs
                    && lhs.is_some_and(|(_, u)| u.is_none())
                {
                    return Some(Value::word(if parent_function {
                        format!("0{unit}")
                    } else {
                        "0".into()
                    }));
                }
                if lhs == Some((1.0, None)) {
                    return Some(rhs_node.clone());
                }
                if rhs == Some((1.0, None)) {
                    return Some(lhs_node.clone());
                }
            }
            if matches!(operator.as_str(), "*" | "+") {
                let constant = lhs.or(rhs);
                let nested = if lhs.is_none() {
                    Some(lhs_node)
                } else if rhs.is_none() {
                    Some(rhs_node)
                } else {
                    None
                };
                if let (
                    Some(constant),
                    Some(Value::Function {
                        value: name,
                        nodes: nested,
                    }),
                ) = (constant, nested)
                    && (name == "calc" || name.is_empty())
                    && nested.len() == 5
                    && matches!(&nested[2], Value::Word { value } if value == operator)
                {
                    let left = dimension(&nested[0]);
                    let right = dimension(&nested[4]);
                    let known = left.or(right);
                    let unknown = if left.is_none() {
                        Some(&nested[0])
                    } else if right.is_none() {
                        Some(&nested[4])
                    } else {
                        None
                    };
                    if let (Some(known), Some(unknown)) = (known, unknown) {
                        if operator == "*" && constant.1.is_some() && known.1.is_some()
                            || operator == "+" && constant.1 != known.1
                        {
                            return None;
                        }
                        let computed = if operator == "*" {
                            constant.0 * known.0
                        } else {
                            constant.0 + known.0
                        };
                        let combined = format!(
                            "{}{}",
                            numeric::format(computed),
                            constant.1.or(known.1).unwrap_or("")
                        );
                        if operator == "*" && combined == "1" {
                            return Some(unknown.clone());
                        }
                        return Some(Value::Function {
                            value: value.clone(),
                            nodes: vec![
                                Value::word(combined),
                                nodes[1].clone(),
                                nodes[2].clone(),
                                nodes[3].clone(),
                                unknown.clone(),
                            ],
                        });
                    }
                }
            }
            let (lhs, rhs) = (lhs?, rhs?);
            let (computed, unit) = match operator.as_str() {
                "*" if lhs.1 == rhs.1 || lhs.1.is_none() || rhs.1.is_none() => {
                    (lhs.0 * rhs.0, lhs.1.or(rhs.1))
                }
                "+" if lhs.1 == rhs.1 => (lhs.0 + rhs.0, lhs.1),
                "-" if lhs.1 == rhs.1 => (lhs.0 - rhs.0, lhs.1),
                "/" if rhs.0 != 0.0 && rhs.1.is_none() => {
                    let computed = lhs.0 / rhs.0;
                    if (computed * 100.0).round() / 100.0 != computed {
                        return None;
                    }
                    (computed, lhs.1)
                }
                _ => return None,
            };
            Some(Value::word(format!(
                "{}{}",
                numeric::format(computed),
                unit.unwrap_or("")
            )))
        }
        _ => None,
    }
}

pub fn constant_fold_ast(nodes: &mut [Value], rem: Option<f64>, normalize_unit: bool) -> bool {
    fn walk(
        nodes: &mut [Value],
        parent_function: bool,
        rem: Option<f64>,
        normalize_unit: bool,
    ) -> bool {
        let mut folded = false;
        for node in nodes {
            if let Value::Function { nodes, .. } = node {
                folded |= walk(nodes, true, rem, normalize_unit);
            }
            if let Some(replacement) = fold_node(node, parent_function, rem, normalize_unit) {
                *node = replacement;
                folded = true;
            }
        }
        folded
    }
    walk(nodes, false, rem, normalize_unit)
}

pub fn constant_fold(input: &str, rem: Option<f64>, normalize_unit: bool) -> String {
    let mut nodes = value::parse(input);
    if constant_fold_ast(&mut nodes, rem, normalize_unit) {
        value::to_css(&nodes)
    } else {
        input.into()
    }
}

pub fn canonicalize_calc_ast(nodes: &mut [Value], collation: &crate::compare::Collation) -> bool {
    use std::cmp::Ordering;
    fn should_swap(left: &Value, right: &Value, collation: &crate::compare::Collation) -> bool {
        match (dimension(left), dimension(right)) {
            (Some(_), None) => return true,
            (None, Some(_)) => return false,
            (Some((left, lu)), Some((right, ru))) => {
                if lu.is_none() && ru.is_some() {
                    return true;
                }
                if lu.is_some() && ru.is_none() {
                    return false;
                }
                if left != right {
                    return left - right > 0.0;
                }
                if lu != ru {
                    return collation.compare(lu.unwrap_or(""), ru.unwrap_or(""))
                        == Ordering::Greater;
                }
            }
            _ => {}
        }
        collation.compare(
            &value::to_css(std::slice::from_ref(left)),
            &value::to_css(std::slice::from_ref(right)),
        ) == Ordering::Greater
    }
    let mut changed = false;
    for node in nodes {
        if let Value::Function { value, nodes } = node {
            changed |= canonicalize_calc_ast(nodes, collation);
            if (value == "calc" || value.is_empty())
                && nodes.len() == 5
                && matches!(&nodes[2], Value::Word { value } if value == "*" || value == "+")
                && should_swap(&nodes[0], &nodes[4], collation)
            {
                nodes.swap(0, 4);
                changed = true;
            }
        }
    }
    changed
}

pub fn canonicalize_calc(input: &str, collation: &crate::compare::Collation) -> String {
    let mut nodes = value::parse(input);
    if canonicalize_calc_ast(&mut nodes, collation) {
        value::to_css(&nodes)
    } else {
        input.into()
    }
}
