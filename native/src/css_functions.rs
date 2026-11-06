// Port of Tailwind CSS 4.3.4 css-functions.ts. See LICENSE.tailwind.
use crate::{
    ast::AstNode,
    numeric,
    syntax::{
        is_js_whitespace,
        segment::segment,
        value::{self, Value},
    },
    theme::{INLINE, Theme},
    utilities::with_alpha,
};

pub fn resolve_theme_value(theme: &Theme, path: &str, inline: bool) -> Option<String> {
    let (path, modifier) = match path.rsplit_once('/') {
        Some((path, modifier)) => (
            path.trim_matches(is_js_whitespace),
            Some(modifier.trim_matches(is_js_whitespace)),
        ),
        None => (path, None),
    };
    let resolved = theme.resolve(None, &[path.into()], if inline { INLINE } else { 0 })?;
    Some(
        match modifier.filter(|modifier| !modifier.is_empty() && !resolved.is_empty()) {
            Some(modifier) => with_alpha(&resolved, modifier),
            None => resolved,
        },
    )
}

fn inject_fallback(nodes: &mut Vec<Value>, fallback: &str) {
    for node in nodes {
        if let Value::Function { value, nodes } = node {
            if matches!(value.as_str(), "var" | "theme" | "--theme") {
                if nodes.len() == 1 {
                    nodes.push(Value::word(format!(", {fallback}")));
                } else if let Some(Value::Word { value }) = nodes.last_mut()
                    && value == "initial"
                {
                    *value = fallback.into();
                }
            }
            inject_fallback(nodes, fallback);
        }
    }
}

pub(crate) fn eventually_unquote(input: &str) -> String {
    let mut chars = input.chars();
    let Some(quote @ ('\'' | '"')) = chars.next() else {
        return input.into();
    };
    let mut chars = chars.collect::<Vec<_>>();
    chars.pop();
    let mut chars = chars.into_iter().peekable();
    let mut output = String::new();
    while let Some(current) = chars.next() {
        if current == '\\'
            && chars
                .peek()
                .is_some_and(|next| *next == quote || *next == '\\')
        {
            output.push(chars.next().unwrap());
        } else {
            output.push(current);
        }
    }
    output
}

fn evaluate(
    name: &str,
    args: &[&str],
    at_rule: bool,
    theme: &Theme,
    compatibility: bool,
) -> Result<String, String> {
    let path = args[0];
    let fallback = &args[1..];
    match name {
        "--alpha" => {
            let parts = segment(path, b'/');
            let color = parts[0].trim_matches(is_js_whitespace);
            let alpha = parts
                .get(1)
                .copied()
                .unwrap_or("")
                .trim_matches(is_js_whitespace);
            let example = format!(
                "--alpha({} / {})",
                if color.is_empty() {
                    "var(--my-color)"
                } else {
                    color
                },
                if alpha.is_empty() { "50%" } else { alpha }
            );
            if color.is_empty() || alpha.is_empty() {
                return Err(format!(
                    "The --alpha(…) function requires a color and an alpha value, e.g.: `{example}`"
                ));
            }
            if !fallback.is_empty() {
                return Err(format!(
                    "The --alpha(…) function only accepts one argument, e.g.: `{example}`"
                ));
            }
            Ok(with_alpha(color, alpha))
        }
        "--spacing" => {
            if path.is_empty() {
                return Err(
                    "The --spacing(…) function requires an argument, but received none.".into(),
                );
            }
            if !fallback.is_empty() {
                return Err(format!(
                    "The --spacing(…) function only accepts a single argument, but received {}.",
                    args.len()
                ));
            }
            let multiplier = theme.resolve(None, &["--spacing".into()], 0).filter(|v| !v.is_empty()).ok_or_else(|| "The --spacing(…) function requires that the `--spacing` theme variable exists, but it was not found.".to_string())?;
            match numeric::dimension(path) {
                Some((0.0, _)) => Ok("0px".into()),
                Some((1.0, _)) => Ok(multiplier),
                _ => Ok(format!("calc({multiplier} * {path})")),
            }
        }
        "--theme" => {
            if !path.starts_with("--") {
                return Err(
                    "The --theme(…) function can only be used with CSS variables from your theme."
                        .into(),
                );
            }
            let (path, inline) = match path.strip_suffix(" inline") {
                Some(path) => (path, true),
                None => (path, at_rule),
            };
            let Some(resolved) = resolve_theme_value(theme, path, inline).filter(|v| !v.is_empty())
            else {
                return if fallback.is_empty() {
                    Err(format!(
                        "Could not resolve value for theme function: `theme({path})`. Consider checking if the variable name is correct or provide a fallback value to silence this error."
                    ))
                } else {
                    Ok(fallback.join(", "))
                };
            };
            if fallback.is_empty() {
                return Ok(resolved);
            }
            let fallback = fallback.join(", ");
            if fallback == "initial" {
                return Ok(resolved);
            }
            if resolved == "initial" {
                return Ok(fallback);
            }
            if resolved.starts_with("var(")
                || resolved.starts_with("theme(")
                || resolved.starts_with("--theme(")
            {
                let mut nodes = value::parse(&resolved);
                inject_fallback(&mut nodes, &fallback);
                return Ok(value::to_css(&nodes));
            }
            Ok(resolved)
        }
        "theme" => {
            let path = eventually_unquote(path);
            let resolved = if compatibility {
                crate::compat_theme::resolve(theme, &path, true)?
            } else {
                resolve_theme_value(theme, &path, true)
            };
            match resolved.filter(|v| !v.is_empty()) {
                Some(resolved) => Ok(resolved),
                None if !fallback.is_empty() => Ok(fallback.join(", ")),
                None => Err(format!(
                    "Could not resolve value for theme function: `theme({path})`. Consider checking if the path is correct or provide a fallback value to silence this error."
                )),
            }
        }
        // CSS_FUNCTIONS in the upstream is a plain JS object. These inherited
        // methods are also visited when a value containing a theme call is walked.
        "toString" | "toLocaleString" => Ok("[object Object]".into()),
        "hasOwnProperty" | "isPrototypeOf" | "propertyIsEnumerable" => {
            Err("input.replaceAll is not a function".into())
        }
        "constructor" | "valueOf" => Err("input.replaceAll is not a function".into()),
        "__defineGetter__" | "__defineSetter__" => {
            Err(format!("Object.prototype.{name}: Expecting function"))
        }
        "__lookupGetter__" | "__lookupSetter__" => {
            Err("Cannot read properties of undefined (reading 'replaceAll')".into())
        }
        "__proto__" => Err("CSS_FUNCTIONS[node.value] is not a function".into()),
        _ => unreachable!("only CSS functions and inherited Object properties are evaluated"),
    }
}

fn recognized(name: &str) -> bool {
    matches!(
        name,
        "--alpha"
            | "--spacing"
            | "--theme"
            | "theme"
            | "toString"
            | "toLocaleString"
            | "hasOwnProperty"
            | "isPrototypeOf"
            | "propertyIsEnumerable"
            | "constructor"
            | "valueOf"
            | "__defineGetter__"
            | "__defineSetter__"
            | "__lookupGetter__"
            | "__lookupSetter__"
            | "__proto__"
    )
}

pub fn substitute_in_value(input: &str, at_rule: bool, theme: &Theme) -> Result<String, String> {
    substitute_value(input, at_rule, theme, false)
}

fn substitute_value(
    input: &str,
    at_rule: bool,
    theme: &Theme,
    compatibility: bool,
) -> Result<String, String> {
    fn walk(
        nodes: &mut Vec<Value>,
        at_rule: bool,
        theme: &Theme,
        compatibility: bool,
    ) -> Result<(), String> {
        let mut index = 0;
        while index < nodes.len() {
            if let Value::Function {
                value: name,
                nodes: children,
            } = &mut nodes[index]
            {
                if recognized(name) {
                    let raw = value::to_css(children);
                    let args = segment(raw.trim_matches(is_js_whitespace), b',')
                        .into_iter()
                        .map(|arg| arg.trim_matches(is_js_whitespace))
                        .collect::<Vec<_>>();
                    let output = evaluate(name, &args, at_rule, theme, compatibility)?;
                    nodes.splice(index..=index, value::parse(&output));
                    continue;
                }
                walk(children, at_rule, theme, compatibility)?;
            }
            index += 1;
        }
        Ok(())
    }
    let mut nodes = value::parse(input);
    walk(&mut nodes, at_rule, theme, compatibility)?;
    Ok(value::to_css(&nodes))
}

fn has_invocation(value: &str) -> bool {
    value.match_indices('(').any(|(index, _)| {
        let before = &value[..index];
        before.ends_with("--alpha") || before.ends_with("--spacing") || before.ends_with("theme")
    })
}

pub fn substitute(ast: &mut [AstNode], theme: &Theme) -> Result<bool, String> {
    substitute_with_options(ast, theme, false)
}

pub(crate) fn substitute_with_options(
    ast: &mut [AstNode],
    theme: &Theme,
    compatibility: bool,
) -> Result<bool, String> {
    let mut changed = false;
    for node in ast {
        match node {
            AstNode::Declaration {
                value: Some(value), ..
            } if has_invocation(value) => {
                *value = substitute_value(value, false, theme, compatibility)?;
                changed = true;
            }
            AstNode::AtRule { name, params, .. }
                if matches!(
                    name.as_str(),
                    "@media" | "@custom-media" | "@container" | "@supports"
                ) && has_invocation(params) =>
            {
                *params = substitute_value(params, true, theme, compatibility)?;
                changed = true;
            }
            _ => {}
        }
        if let Some(children) = node.children_mut() {
            changed |= substitute_with_options(children, theme, compatibility)?;
        }
    }
    Ok(changed)
}
