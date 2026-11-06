// Tailwind 4.3.4 legacy theme() migration and calc(var(--spacing) * n).
use crate::{
    candidate::Modifier,
    compat,
    css_functions::eventually_unquote,
    numeric,
    syntax::{
        is_js_whitespace,
        segment::segment,
        value::{self, Value},
    },
    theme::Theme,
    utilities,
};
use regex::Regex;
use std::sync::LazyLock;

#[derive(Clone, Copy)]
pub enum Convert {
    All,
    MigrateModifier,
    MigrateThemeOnly,
}

fn text(node: &Value) -> &str {
    match node {
        Value::Word { value } | Value::Separator { value } | Value::Function { value, .. } => value,
    }
}

fn variable(theme: &Theme, path: &str, prefix: bool) -> Result<Option<String>, String> {
    let name =
        compat::key_path_to_css_property(&compat::to_key_path(path))?.unwrap_or("null".into());
    let key = format!("--{name}");
    if theme
        .get(std::slice::from_ref(&key))
        .is_none_or(|v| v.is_empty())
    {
        return Ok(None);
    }
    Ok(Some(
        match theme.prefix.as_deref().filter(|p| prefix && !p.is_empty()) {
            Some(prefix) => format!("--{prefix}-{name}"),
            None => key,
        },
    ))
}

fn to_var(theme: &Theme, path: &str, fallback: Option<&str>) -> Result<Option<String>, String> {
    if let Some(variable) = variable(theme, path, true)? {
        return Ok(Some(match fallback.filter(|f| !f.is_empty()) {
            Some(fallback) => format!("var({variable}, {fallback})"),
            None => format!("var({variable})"),
        }));
    }
    let parts = compat::to_key_path(path);
    if parts.first().is_some_and(|p| p == "spacing")
        && theme
            .get(&["--spacing".into()])
            .is_some_and(|v| !v.is_empty())
        && let Some(multiplier) = parts.get(1).filter(|v| numeric::is_spacing_multiplier(v))
    {
        return Ok(Some(format!("--spacing({multiplier})")));
    }
    Ok(None)
}

fn to_theme(theme: &Theme, path: &str, fallback: Option<&str>) -> Result<Option<String>, String> {
    let parts: Vec<_> = segment(path, b'/')
        .into_iter()
        .map(|s| s.trim_matches(is_js_whitespace))
        .collect();
    let Some(variable) = variable(theme, parts[0], false)? else {
        return Ok(None);
    };
    let modifier = if parts.len() > 1 {
        format!("/{}", parts[1..].join("/"))
    } else {
        String::new()
    };
    Ok(Some(match fallback.filter(|f| !f.is_empty()) {
        Some(fallback) => format!("--theme({variable}{modifier}, {fallback})"),
        None => format!("--theme({variable}{modifier})"),
    }))
}

fn substitute(
    nodes: &mut Vec<Value>,
    handle: &mut impl FnMut(&str, Option<&str>) -> Result<Option<String>, String>,
) -> Result<(), String> {
    let mut index = 0;
    while index < nodes.len() {
        let replacement = if let Value::Function { value, nodes: args } = &mut nodes[index]
            && value == "theme"
            && !args.is_empty()
        {
            if matches!(&args[0], Value::Separator { value } if value.trim_matches(is_js_whitespace).is_empty())
            {
                args.remove(0);
                if args.is_empty() {
                    return Err("Cannot read properties of undefined (reading 'kind')".into());
                }
            }
            if let Some(Value::Word { value }) = args.first() {
                let mut path = value.clone();
                let mut skip = 1;
                for (i, arg) in args.iter().enumerate().skip(1) {
                    if text(arg).contains(',') {
                        break;
                    }
                    path.push_str(&value::to_css(std::slice::from_ref(arg)));
                    skip = i + 1;
                }
                let fallback = if skip + 1 < args.len() {
                    Some(value::to_css(&args[skip + 1..]))
                } else {
                    None
                };
                handle(&eventually_unquote(&path), fallback.as_deref())?
            } else {
                None
            }
        } else {
            None
        };
        if let Some(mut replacement) = replacement {
            if let Some(previous) = nodes[..index].iter().rev().find(|node| !matches!(node, Value::Separator { value } if value.trim_matches(is_js_whitespace).is_empty()))
                && matches!(text(previous).trim_matches(is_js_whitespace), "-" | "+" | "*" | "/")
            { replacement = format!("({replacement})") }
            nodes.splice(index..=index, value::parse(&replacement));
            continue;
        }
        if let Value::Function { nodes, .. } = &mut nodes[index] {
            substitute(nodes, handle)?
        }
        index += 1;
    }
    Ok(())
}

fn percentage(value: f64) -> f64 {
    numeric::round_fixed(value * 100.0, 10)
}

fn opacity_modifier(theme: &Theme, number: f64) -> Modifier {
    let key = numeric::format(number);
    if number.is_finite() && number.fract() == 0.0 {
        let opacity = theme.resolve_value(Some(&key), &["--opacity".into()]);
        if opacity.as_ref().is_none_or(|opacity| {
            let value = opacity
                .strip_suffix('%')
                .map_or_else(|| percentage(utilities::number(opacity)), utilities::number);
            value == number
        }) {
            return Modifier::Named { value: key };
        }
    }
    Modifier::Arbitrary {
        value: format!("{key}%"),
    }
}

pub fn convert(
    theme: &Theme,
    input: &str,
    mode: Convert,
) -> Result<(String, Option<Modifier>), String> {
    let mut ast = value::parse(input);
    if matches!(mode, Convert::MigrateThemeOnly) {
        substitute(&mut ast, &mut |path, fallback| {
            to_theme(theme, path, fallback)
        })?;
        return Ok((value::to_css(&ast), None));
    }
    fn counts(nodes: &[Value], usage: &mut usize, modifiers: &mut usize) {
        for node in nodes {
            if let Value::Function { value, nodes } = node {
                if value == "theme" {
                    *usage += 1;
                    for child in nodes {
                        if matches!(child, Value::Separator { value } if value.contains(',')) {
                            break;
                        }
                        if matches!(child, Value::Word { value } if value == "/") {
                            *modifiers += 1;
                            break;
                        }
                    }
                }
                counts(nodes, usage, modifiers);
            }
        }
    }
    let (mut usages, mut modifiers) = (0, 0);
    counts(&ast, &mut usages, &mut modifiers);
    if usages == 0 {
        return Ok((input.into(), None));
    }
    if modifiers == 0 {
        substitute(&mut ast, &mut |path, fallback| {
            to_var(theme, path, fallback)
        })?;
        return Ok((value::to_css(&ast), None));
    }
    if modifiers > 1 {
        substitute(&mut ast, &mut |path, fallback| {
            to_theme(theme, path, fallback)
        })?;
        return Ok((value::to_css(&ast), None));
    }
    let single = ast.len() == 1;
    let mut modifier = None;
    static PERCENT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[0-9]+%$").unwrap());
    static DECIMAL: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^0?\.[0-9]+$").unwrap());
    substitute(&mut ast, &mut |path, fallback| {
        let parts: Vec<_> = segment(path, b'/')
            .into_iter()
            .map(|s| s.trim_matches(is_js_whitespace))
            .collect();
        if parts.len() > 2 {
            return Ok(None);
        }
        let mut path = path;
        if single && parts.len() == 2 && matches!(mode, Convert::MigrateModifier) {
            let value = parts[1];
            modifier = Some(if PERCENT.is_match(value) {
                opacity_modifier(theme, utilities::number(&value[..value.len() - 1]))
            } else if DECIMAL.is_match(value) {
                opacity_modifier(theme, percentage(utilities::number(value)))
            } else {
                Modifier::Arbitrary {
                    value: value.into(),
                }
            });
            path = parts[0];
        }
        match to_var(theme, path, fallback)? {
            Some(value) if !value.is_empty() => Ok(Some(value)),
            _ => to_theme(theme, path, fallback),
        }
    })?;
    Ok((value::to_css(&ast), modifier))
}

pub fn spacing_calc(input: &str, theme: &Theme) -> String {
    let spacing = match theme.prefix.as_deref().filter(|p| !p.is_empty()) {
        Some(prefix) => format!("--{prefix}-spacing"),
        None => "--spacing".into(),
    };
    fn visit(nodes: &mut Vec<Value>, spacing: &str) {
        let mut index = 0;
        while index < nodes.len() {
            if let Value::Function { value, nodes: args } = &nodes[index]
                && value == "calc"
                && args.len() == 5
                && matches!(&args[2], Value::Word { value } if value == "*")
                && matches!(&args[0], Value::Function { value, nodes } if value == "var" && nodes.len() == 1 && matches!(&nodes[0], Value::Word { value } if value == spacing))
            {
                let replacement =
                    value::parse(&format!("--spacing({})", value::to_css(&args[4..5])));
                nodes.splice(index..=index, replacement);
                continue;
            }
            if let Value::Function { nodes, .. } = &mut nodes[index] {
                visit(nodes, spacing)
            }
            index += 1;
        }
    }
    let mut ast = value::parse(input);
    visit(&mut ast, &spacing);
    value::to_css(&ast)
}
