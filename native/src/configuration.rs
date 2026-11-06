// Port of the CSS configuration phase in Tailwind CSS 4.3.4 index.ts.
// See LICENSE.tailwind. Filesystem and package resolution belong to the host.
use crate::{
    apply,
    ast::{self, AstNode, ContextValue},
    compile::DesignSystem,
    custom_utilities::CssUtility,
    imports::{self, LoadedStylesheet},
    syntax::{
        css,
        escape::{escape, unescape},
        is_js_whitespace,
        segment::segment,
    },
    theme::{DEFAULT, INLINE, REFERENCE, STATIC, Theme},
};
use indexmap::{IndexMap, IndexSet};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Source {
    pub base: String,
    pub pattern: String,
    pub negated: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Root {
    None(String),
    Source { base: String, pattern: String },
}

pub struct Configuration {
    pub design_system: DesignSystem,
    pub ast: Vec<AstNode>,
    pub sources: Vec<Source>,
    pub root: Option<Root>,
    pub inline_candidates: Vec<String>,
    pub has_utilities: bool,
}

enum CustomVariant {
    Selectors(Vec<String>),
    Body(Vec<AstNode>),
}

#[derive(Default)]
struct Parser {
    theme: Theme,
    important: bool,
    variants: IndexMap<String, CustomVariant>,
    utilities: Vec<CssUtility>,
    dependencies: IndexMap<String, IndexSet<String>>,
    first_theme: bool,
    has_utilities: bool,
    sources: Vec<Source>,
    root: Option<Root>,
    inline: Vec<String>,
    ignored: Vec<String>,
}

fn context(key: &str, value: ContextValue, nodes: Vec<AstNode>) -> AstNode {
    AstNode::Context {
        context: IndexMap::from([(key.into(), value)]),
        nodes,
    }
}

fn trim(value: &str) -> &str {
    value.trim_matches(is_js_whitespace)
}

fn quoted(value: &str) -> bool {
    matches!(value.as_bytes().first(), Some(b'\'' | b'"'))
        && value.as_bytes().first() == value.as_bytes().last()
}

fn inner(value: &str, start: usize) -> Result<&str, String> {
    // ASCII delimiters normally surround the content. Malformed input can
    // cut a UTF-16 surrogate pair in JS; Rust strings cannot represent it.
    value
        .get(start..value.len().saturating_sub(1).max(start))
        .ok_or_else(|| "CSS parameter slicing produced an unpaired UTF-16 surrogate".into())
}

fn contains_at_rule(nodes: &[AstNode], target: &str) -> bool {
    nodes.iter().any(|node| {
        matches!(node, AstNode::AtRule { name, .. } if name == target)
            || children(node).is_some_and(|nodes| contains_at_rule(nodes, target))
    })
}

fn children(node: &AstNode) -> Option<&[AstNode]> {
    match node {
        AstNode::Rule { nodes, .. }
        | AstNode::AtRule { nodes, .. }
        | AstNode::Context { nodes, .. }
        | AstNode::AtRoot { nodes } => Some(nodes),
        _ => None,
    }
}

fn dependencies(nodes: &[AstNode], result: &mut IndexSet<String>) {
    for node in nodes {
        if let AstNode::AtRule { name, params, .. } = node
            && name == "@variant"
        {
            result.insert(params.clone());
        }
        if let Some(nodes) = children(node) {
            dependencies(nodes, result);
        }
    }
}

fn valid_variant_name(name: &str) -> bool {
    let name = name.strip_prefix('@').unwrap_or(name);
    name.as_bytes()
        .first()
        .is_some_and(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
        && !name.ends_with(['_', '-'])
}

impl Parser {
    fn visit(
        &mut self,
        nodes: &mut Vec<AstNode>,
        parent: bool,
        inherited: &IndexMap<String, ContextValue>,
    ) -> Result<(), String> {
        let mut index = 0;
        while index < nodes.len() {
            if let AstNode::Context { context, nodes } = &mut nodes[index] {
                let mut scope = inherited.clone();
                scope.extend(context.clone());
                self.visit(nodes, parent, &scope)?;
                index += 1;
                continue;
            }
            if let AstNode::AtRule {
                name, nodes: body, ..
            } = &mut nodes[index]
                && name == "@variant"
                && !parent
                && (body.is_empty() || contains_at_rule(body, "@slot"))
            {
                *name = "@custom-variant".into();
            }

            let mut remove = false;
            let mut replace = None;
            let mut skip = false;
            if let AstNode::AtRule {
                name,
                params,
                nodes: body,
            } = &mut nodes[index]
            {
                match name.as_str() {
                    "@tailwind" if params.starts_with("utilities") => {
                        if self.has_utilities
                            || inherited.get("reference") == Some(&ContextValue::Bool(true))
                        {
                            remove = true;
                        } else {
                            for param in segment(params, b' ') {
                                if param.starts_with("source(") {
                                    let path = inner(param, 7)?;
                                    self.root = if path == "none" {
                                        Some(Root::None(path.into()))
                                    } else {
                                        if !quoted(path) {
                                            return Err("`source(…)` paths must be quoted.".into());
                                        }
                                        Some(Root::Source {
                                            base: scope_string(inherited, "sourceBase")
                                                .unwrap_or_else(|| {
                                                    scope_string(inherited, "base")
                                                        .unwrap_or_default()
                                                })
                                                .into(),
                                            pattern: inner(path, 1)?.into(),
                                        })
                                    };
                                }
                            }
                            self.has_utilities = true;
                        }
                    }
                    "@utility" => {
                        if parent {
                            return Err("`@utility` cannot be nested.".into());
                        }
                        if body.is_empty() {
                            return Err(format!(
                                "`@utility {params}` is empty. Utilities should include at least one property."
                            ));
                        }
                        let Some(utility) = CssUtility::new(params, body.clone()) else {
                            let description = if !params.ends_with("-*") && params.ends_with('*') {
                                "A functional utility must end in `-*`."
                            } else if !params.ends_with("-*") && params.contains('*') {
                                "The dynamic portion marked by `-*` must appear once at the end."
                            } else {
                                "Utilities should be alphanumeric and start with a lowercase letter."
                            };
                            return Err(format!(
                                "`@utility {params}` defines an invalid utility name. {description}"
                            ));
                        };
                        *body = utility.nodes.clone();
                        self.utilities.push(utility);
                    }
                    "@source" => {
                        if !body.is_empty() {
                            return Err("`@source` cannot have a body.".into());
                        }
                        if parent {
                            return Err("`@source` cannot be nested.".into());
                        }
                        let (negated, path) = params
                            .strip_prefix("not ")
                            .map_or((false, params.as_str()), |path| (true, path));
                        let (inline, path) = if path.starts_with("inline(") {
                            (true, trim(inner(path, 7)?))
                        } else {
                            (false, path)
                        };
                        if !quoted(path) {
                            return Err("`@source` paths must be quoted.".into());
                        }
                        let path = inner(path, 1)?;
                        if inline {
                            let destination = if negated {
                                &mut self.ignored
                            } else {
                                &mut self.inline
                            };
                            for pattern in segment(path, b' ') {
                                destination.extend(expand(pattern)?);
                            }
                        } else {
                            self.sources.push(Source {
                                base: scope_string(inherited, "base").unwrap_or_default().into(),
                                pattern: path.into(),
                                negated,
                            });
                        }
                        remove = true;
                    }
                    "@custom-variant" => {
                        if parent {
                            return Err("`@custom-variant` cannot be nested.".into());
                        }
                        let parts = segment(params, b' ');
                        let name = parts[0];
                        let selector = parts.get(1).copied().filter(|s| !s.is_empty());
                        if !valid_variant_name(name) {
                            return Err(format!(
                                "`@custom-variant {name}` defines an invalid variant name. Variants should only contain alphanumeric, dashes, or underscore characters and start with a lowercase letter or number."
                            ));
                        }
                        if !body.is_empty() && selector.is_some() {
                            return Err(format!(
                                "`@custom-variant {name}` cannot have both a selector and a body."
                            ));
                        }
                        let (variant, deps) = if body.is_empty() {
                            let selector = selector.ok_or_else(|| {
                                format!("`@custom-variant {name}` has no selector or body.")
                            })?;
                            let selectors = segment(inner(selector, 1)?, b',');
                            if selectors.iter().any(|selector| trim(selector).is_empty()) {
                                return Err(format!(
                                    "`@custom-variant {name} ({})` selector is invalid.",
                                    selectors.join(",")
                                ));
                            }
                            (
                                CustomVariant::Selectors(
                                    selectors.into_iter().map(|s| trim(s).into()).collect(),
                                ),
                                IndexSet::new(),
                            )
                        } else {
                            let mut deps = IndexSet::new();
                            dependencies(body, &mut deps);
                            (CustomVariant::Body(body.clone()), deps)
                        };
                        self.variants.insert(name.into(), variant);
                        self.dependencies.insert(name.into(), deps);
                        remove = true;
                    }
                    "@media" => {
                        let mut unknown = Vec::new();
                        for param in segment(params, b' ') {
                            if param.starts_with("source(") {
                                media_source(
                                    body,
                                    inner(param, 7)?,
                                    scope_string(inherited, "base").unwrap_or_default(),
                                );
                            } else if param.starts_with("theme(") {
                                let options = inner(param, 6)?;
                                media_theme(body, options, options.contains("reference"))?;
                            } else if param.starts_with("prefix(") {
                                media_theme(body, &format!("prefix({})", inner(param, 7)?), false)?;
                            } else if param == "important" {
                                self.important = true;
                            } else if param == "reference" {
                                *body = vec![context(
                                    "reference",
                                    ContextValue::Bool(true),
                                    std::mem::take(body),
                                )];
                            } else {
                                unknown.push(param);
                            }
                        }
                        if unknown.is_empty() {
                            replace = Some(std::mem::take(body));
                        } else {
                            *params = unknown.join(" ");
                        }
                    }
                    "@theme" => {
                        let mut options =
                            if inherited.get("reference") == Some(&ContextValue::Bool(true)) {
                                REFERENCE
                            } else {
                                0
                            };
                        let mut prefix = None;
                        for token in segment(params, b' ') {
                            match token {
                                "reference" => options |= REFERENCE,
                                "inline" => options |= INLINE,
                                "default" => options |= DEFAULT,
                                "static" => options |= STATIC,
                                _ if token.starts_with("prefix(") && token.ends_with(')') => {
                                    prefix = Some(inner(token, 7)?)
                                }
                                _ => {}
                            }
                        }
                        if let Some(prefix) = prefix.filter(|s| !s.is_empty()) {
                            if !prefix.bytes().all(|b| b.is_ascii_lowercase()) {
                                return Err(format!(
                                    "The prefix \"{prefix}\" is invalid. Prefixes must be lowercase ASCII letters (a-z) only."
                                ));
                            }
                            self.theme.prefix = Some(prefix.into());
                        }
                        for child in body.iter() {
                            match child {
                                AstNode::AtRule { name, .. } if name == "@keyframes" => {
                                    self.theme.add_keyframes(Arc::new(child.clone()))
                                }
                                AstNode::Comment { .. } => {}
                                AstNode::Declaration {
                                    property, value, ..
                                } if property.starts_with("--") => self.theme.add(
                                    &unescape(property),
                                    value.as_deref().unwrap_or(""),
                                    options,
                                )?,
                                _ => {
                                    let snippet = ast::to_css(&[AstNode::AtRule {
                                        name: name.clone(),
                                        params: params.clone(),
                                        nodes: vec![child.clone()],
                                    }]);
                                    let lines: Vec<_> = snippet.split('\n').collect();
                                    let snippet = lines
                                        .iter()
                                        .enumerate()
                                        .map(|(i, line)| {
                                            format!(
                                                "{} {line}",
                                                if i == 0 || i >= lines.len() - 2 {
                                                    " "
                                                } else {
                                                    ">"
                                                }
                                            )
                                        })
                                        .collect::<Vec<_>>()
                                        .join("\n");
                                    return Err(format!(
                                        "`@theme` blocks must only contain custom properties or `@keyframes`.\n\n{snippet}"
                                    ));
                                }
                            }
                        }
                        if self.first_theme {
                            remove = true;
                        } else {
                            self.first_theme = true;
                            replace = Some(vec![context(
                                "__native_theme_placeholder",
                                ContextValue::Bool(true),
                                Vec::new(),
                            )]);
                            skip = true;
                        }
                    }
                    _ => {}
                }
            }
            if remove {
                nodes.remove(index);
                continue;
            }
            if let Some(replacement) = replace {
                let len = replacement.len();
                nodes.splice(index..=index, replacement);
                if skip {
                    index += len;
                }
                continue;
            }
            if let Some(children) = nodes[index].children_mut() {
                self.visit(children, true, inherited)?;
            }
            index += 1;
        }
        Ok(())
    }
}

fn scope_string<'a>(scope: &'a IndexMap<String, ContextValue>, key: &str) -> Option<&'a str> {
    match scope.get(key) {
        Some(ContextValue::String(value)) => Some(value),
        _ => None,
    }
}

fn media_theme(nodes: &mut [AstNode], options: &str, reference: bool) -> Result<(), String> {
    for node in nodes {
        match node {
            AstNode::AtRule { name, params, .. } if name == "@theme" => { params.push(' '); params.push_str(options); continue; },
            AstNode::AtRule { .. } | AstNode::Context { .. } => {},
            _ if reference => return Err("Files imported with `@import \"…\" theme(reference)` must only contain `@theme` blocks.\nUse `@reference \"…\";` instead.".into()),
            _ => {},
        }
        if let Some(children) = node.children_mut() {
            media_theme(children, options, reference)?;
        }
    }
    Ok(())
}

fn media_source(nodes: &mut [AstNode], path: &str, base: &str) {
    for node in nodes {
        if let AstNode::AtRule { name, params, .. } = node
            && name == "@tailwind"
            && params == "utilities"
        {
            params.push_str(&format!(" source({path})"));
            *node = context(
                "sourceBase",
                ContextValue::String(base.into()),
                vec![node.clone()],
            );
            continue;
        }
        if let Some(children) = node.children_mut() {
            media_source(children, path, base);
        }
    }
}

fn sorted_variants(graph: &IndexMap<String, IndexSet<String>>) -> Result<Vec<String>, String> {
    fn visit(
        name: &str,
        graph: &IndexMap<String, IndexSet<String>>,
        path: &mut Vec<String>,
        seen: &mut IndexSet<String>,
        wip: &mut IndexSet<String>,
        output: &mut Vec<String>,
    ) -> Result<(), String> {
        let Some(deps) = graph.get(name) else {
            return Ok(());
        };
        if seen.contains(name) {
            return Ok(());
        }
        if wip.contains(name) {
            let nodes: Vec<_> = path
                .iter()
                .enumerate()
                .map(|(i, item)| AstNode::AtRule {
                    name: "@custom-variant".into(),
                    params: item.clone(),
                    nodes: vec![AstNode::AtRule {
                        name: "@variant".into(),
                        params: path.get(i + 1).map_or(name, String::as_str).into(),
                        nodes: Vec::new(),
                    }],
                })
                .collect();
            let output = ast::to_css(&nodes).replace(';', " { … }").replacen(
                &format!("@custom-variant {name} {{"),
                &format!("@custom-variant {name} {{ /* ← */"),
                1,
            );
            return Err(format!(
                "Circular dependency detected in custom variants:\n\n{output}"
            ));
        }
        wip.insert(name.into());
        for dependency in deps {
            path.push(name.into());
            visit(dependency, graph, path, seen, wip, output)?;
            path.pop();
        }
        seen.insert(name.into());
        wip.shift_remove(name);
        output.push(name.into());
        Ok(())
    }
    let mut output = Vec::new();
    let (mut seen, mut wip) = (IndexSet::new(), IndexSet::new());
    for name in graph.keys() {
        visit(
            name,
            graph,
            &mut Vec::new(),
            &mut seen,
            &mut wip,
            &mut output,
        )?;
    }
    Ok(output)
}

/// Load CSS directives using a host-provided synchronous stylesheet resolver.
/// JavaScript `@plugin` and `@config` modules require an explicit separate adapter.
pub fn load(
    css: &str,
    base: &str,
    loader: &mut impl FnMut(&str, &str) -> Result<LoadedStylesheet, String>,
) -> Result<Configuration, String> {
    let mut nodes = vec![context(
        "base",
        ContextValue::String(base.into()),
        css::parse(css).map_err(|e| e.message)?,
    )];
    imports::substitute_at_imports(&mut nodes, base, loader)?;
    let mut parser = Parser::default();
    parser.visit(&mut nodes, false, &IndexMap::new())?;
    let mut system = DesignSystem::new(parser.theme);
    system.important = parser.important;
    system.invalid_candidates.extend(parser.ignored);
    check_modules(&nodes, false)?;
    system.register_legacy_utilities();
    system.compatibility_theme = true;
    for name in parser.variants.keys() {
        system.variants.reserve(name);
    }
    for name in sorted_variants(&parser.dependencies)? {
        match &parser.variants[&name] {
            CustomVariant::Body(body) => system.custom_variant(&name, body),
            CustomVariant::Selectors(selectors) => {
                system.custom_variant_selectors(&name, selectors)
            }
        }
    }
    let bindings = parser
        .utilities
        .into_iter()
        .map(|utility| system.register_prepared_css_utility(utility))
        .collect::<Vec<_>>();
    bind_utilities(&mut nodes, &mut bindings.iter().copied());
    if parser.first_theme {
        let declarations = system
            .theme
            .entries()?
            .into_iter()
            .filter(|(_, value)| value.options & REFERENCE == 0)
            .map(|(key, value)| AstNode::decl(escape(&key), value.value))
            .collect();
        fill_theme(
            &mut nodes,
            &mut Some(vec![context(
                "theme",
                ContextValue::Bool(true),
                declarations,
            )]),
        );
        nodes.extend(system.theme.keyframes().iter().map(|keyframes| {
            context(
                "theme",
                ContextValue::Bool(true),
                vec![AstNode::AtRoot {
                    nodes: vec![keyframes.as_ref().clone()],
                }],
            )
        }));
    }
    system.substitute_variants(&mut nodes)?;
    crate::css_functions::substitute_with_options(&mut nodes, &system.theme, true)?;
    refresh_utilities(&nodes, &mut system);
    apply::substitute(&mut nodes, &mut system)?;
    erase_bindings(&mut nodes);
    remove_utilities(&mut nodes, true);
    Ok(Configuration {
        design_system: system,
        ast: nodes,
        sources: parser.sources,
        root: parser.root,
        inline_candidates: parser.inline,
        has_utilities: parser.has_utilities,
    })
}

fn check_modules(nodes: &[AstNode], parent: bool) -> Result<(), String> {
    fn visit(nodes: &[AstNode], parent: bool) -> Result<bool, String> {
        let mut found = false;
        for node in nodes {
            if let AstNode::AtRule {
                name,
                params,
                nodes,
            } = node
            {
                if name == "@config" {
                    if !nodes.is_empty() {
                        return Err("`@config` cannot have a body.".into());
                    }
                    if parent {
                        return Err("`@config` cannot be nested.".into());
                    }
                    found = true;
                    continue;
                }
                if name == "@plugin" {
                    if parent {
                        return Err("`@plugin` cannot be nested.".into());
                    }
                    if inner(params, 1)?.is_empty() {
                        return Err("`@plugin` must have a path.".into());
                    }
                    for child in nodes {
                        let AstNode::Declaration { value, .. } = child else {
                            return Err(format!(
                                "Unexpected `@plugin` option:\n\n{}\n\n`@plugin` options must be a flat list of declarations.",
                                ast::to_css(std::slice::from_ref(child))
                            ));
                        };
                        if let Some(value) = value {
                            for part in segment(value, b',') {
                                let part = trim(part);
                                if part.starts_with('{') && part.ends_with('}') {
                                    return Err(format!(
                                        "Unexpected `@plugin` option: Value of declaration `{}` is not supported.\n\nUsing an object as a plugin option is currently only supported in JavaScript configuration files.",
                                        trim(&ast::to_css(std::slice::from_ref(child)))
                                    ));
                                }
                            }
                        }
                    }
                    found = true;
                    continue;
                }
            }
            if let Some(body) = children(node) {
                found |= visit(body, parent || !matches!(node, AstNode::Context { .. }))?;
            }
        }
        Ok(found)
    }
    if visit(nodes, parent)? {
        Err("No `loadModule` function provided to `compile`".into())
    } else {
        Ok(())
    }
}

fn bind_utilities(nodes: &mut [AstNode], bindings: &mut impl Iterator<Item = usize>) {
    for node in nodes {
        if let Some(body) = node.children_mut() {
            bind_utilities(body, bindings);
        }
        if matches!(node, AstNode::AtRule {name,..} if name == "@utility")
            && let Some(binding) = bindings.next()
        {
            *node = context(
                "__native_utility_binding",
                ContextValue::String(binding.to_string()),
                vec![node.clone()],
            );
        }
    }
}

fn refresh_utilities(nodes: &[AstNode], system: &mut DesignSystem) {
    for node in nodes {
        if let AstNode::Context { context, nodes } = node
            && let Some(ContextValue::String(binding)) = context.get("__native_utility_binding")
            && let Ok(binding) = binding.parse::<usize>()
            && let Some(AstNode::AtRule { nodes, .. }) = nodes.first()
        {
            system.custom_utilities[binding].nodes = nodes.clone();
        }
        if let Some(body) = children(node) {
            refresh_utilities(body, system);
        }
    }
}

fn erase_bindings(nodes: &mut Vec<AstNode>) {
    let mut index = 0;
    while index < nodes.len() {
        if let AstNode::Context {
            context,
            nodes: body,
        } = &mut nodes[index]
            && context.contains_key("__native_utility_binding")
        {
            let body = std::mem::take(body);
            nodes.splice(index..=index, body);
            continue;
        }
        if let Some(body) = nodes[index].children_mut() {
            erase_bindings(body);
        }
        index += 1;
    }
}

fn fill_theme(nodes: &mut [AstNode], value: &mut Option<Vec<AstNode>>) {
    for node in nodes {
        if let AstNode::Context { context, .. } = node
            && context.contains_key("__native_theme_placeholder")
            && let Some(value) = value.take()
        {
            *node = AstNode::Rule {
                selector: ":root, :host".into(),
                nodes: value,
            };
            return;
        }
        if let Some(body) = node.children_mut() {
            fill_theme(body, value);
        }
    }
}

fn remove_utilities(nodes: &mut Vec<AstNode>, top_level: bool) {
    nodes.retain_mut(|node| {
        match node {
            AstNode::AtRule { name, .. } if name == "@utility" && top_level => return false,
            AstNode::AtRule {
                name,
                params,
                nodes,
            } if name == "@tailwind" && params.starts_with("utilities") => {
                // The upstream mutates the registered node at any depth. Its
                // residual name/params are not represented by Context in Rust.
                *node = AstNode::Context {
                    context: IndexMap::new(),
                    nodes: std::mem::take(nodes),
                };
            }
            AstNode::Context { nodes, .. } => remove_utilities(nodes, top_level),
            _ => {
                if let Some(body) = node.children_mut() {
                    remove_utilities(body, false);
                }
            }
        }
        true
    });
}

fn expand(pattern: &str) -> Result<Vec<String>, String> {
    let Some(start) = pattern.find('{') else {
        return Ok(vec![pattern.into()]);
    };
    let rest = &pattern[start..];
    let mut depth = 0;
    let mut end = rest.rfind('}');
    for (index, c) in rest.char_indices() {
        if c == '{' {
            depth += 1;
        } else if c == '}' {
            depth -= 1;
            if depth == 0 {
                end = Some(index);
                break;
            }
        }
    }
    let end = end.ok_or_else(|| format!("The pattern `{pattern}` is not balanced."))?;
    let inside = &rest[1..end];
    let sequence: Vec<_> = inside.split("..").collect();
    let parts = if (2..=3).contains(&sequence.len())
        && sequence.iter().all(|s| {
            !s.strip_prefix('-').unwrap_or(s).is_empty()
                && s.strip_prefix('-')
                    .unwrap_or(s)
                    .bytes()
                    .all(|b| b.is_ascii_digit())
        }) {
        let start: f64 = sequence[0].parse().unwrap_or(f64::NAN);
        let end: f64 = sequence[1].parse().unwrap_or(f64::NAN);
        let increasing = start < end;
        let step = sequence
            .get(2)
            .map_or(if start <= end { 1.0 } else { -1.0 }, |s| {
                s.parse().unwrap_or(f64::NAN)
            });
        if step == 0.0 {
            return Err("Step cannot be zero in sequence expansion.".into());
        }
        let step = if increasing { step.abs() } else { -step.abs() };
        let mut values = Vec::new();
        let mut current = start;
        while if increasing {
            current <= end
        } else {
            current >= end
        } {
            values.push(crate::numeric::format(current));
            let next = current + step;
            if next == current {
                return Err("Sequence expansion cannot advance at this numeric precision".into());
            }
            current = next;
        }
        values
    } else {
        segment(inside, b',')
            .into_iter()
            .map(str::to_owned)
            .collect()
    };
    let parts = parts
        .iter()
        .map(|part| expand(part))
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();
    let tails = expand(&rest[end + 1..])?;
    Ok(tails
        .iter()
        .flat_map(|tail| {
            parts
                .iter()
                .map(move |part| format!("{}{part}{tail}", &pattern[..start]))
        })
        .collect())
}
