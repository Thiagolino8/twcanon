// Port of Tailwind CSS 4.3.4 ast.ts optimizeAst. See LICENSE.tailwind.
use crate::{
    ast::{AstNode, ContextValue},
    ast_arena::{Arena, GroupId, NodeId},
    nesting::handle_nesting,
    syntax::{
        is_js_whitespace,
        value::{self, Value},
    },
    theme::{self, Theme},
};
use indexmap::{IndexMap, IndexSet};
use std::collections::HashSet;

pub const AT_PROPERTY: u8 = 1;
pub const COLOR_MIX: u8 = 2;
pub const ALL: u8 = AT_PROPERTY | COLOR_MIX;

/// Include fallback words and preserve repeated references, like utils/variables.ts.
pub fn extract_used_variables(raw: &str) -> Vec<String> {
    fn words(nodes: &[Value], output: &mut Vec<String>) {
        for node in nodes {
            match node {
                Value::Word { value } if value.starts_with("--") => output.push(value.clone()),
                Value::Function { nodes, .. } => words(nodes, output),
                _ => {}
            }
        }
    }
    fn walk(nodes: &[Value], output: &mut Vec<String>) {
        for node in nodes {
            if let Value::Function { value, nodes } = node {
                if value == "var" {
                    words(nodes, output);
                } else {
                    walk(nodes, output);
                }
            }
        }
    }
    let mut output = Vec::new();
    walk(&value::parse(raw), &mut output);
    output
}

fn keyframe_names(value: &str) -> Vec<String> {
    let mut output = Vec::new();
    let mut start = 0;
    let mut separator = false;
    for (index, c) in value.char_indices() {
        if c == ',' || is_js_whitespace(c) {
            if !separator {
                output.push(value[start..index].to_string());
            }
            separator = true;
        } else if separator {
            start = index;
            separator = false;
        }
    }
    output.push(if separator {
        String::new()
    } else {
        value[start..].to_string()
    });
    output
}

fn enabled(context: &IndexMap<String, ContextValue>, key: &str) -> bool {
    match context.get(key) {
        Some(ContextValue::Bool(value)) => *value,
        Some(ContextValue::String(value)) => !value.is_empty(),
        None => false,
    }
}

struct Optimizer<'a> {
    arena: Arena,
    theme: &'a mut Theme,
    polyfills: u8,
    root: GroupId,
    at_roots: GroupId,
    properties: HashSet<String>,
    variables: IndexMap<GroupId, IndexSet<NodeId>>,
    mixes: IndexMap<GroupId, IndexSet<NodeId>>,
    keyframes: IndexSet<NodeId>,
    used_keyframes: HashSet<String>,
    root_fallbacks: Vec<NodeId>,
    universal_fallbacks: Vec<NodeId>,
    dependencies: IndexMap<String, IndexSet<String>>,
}

impl Optimizer<'_> {
    fn transform_group(
        &mut self,
        source: GroupId,
        target: GroupId,
        context: &IndexMap<String, ContextValue>,
        depth: usize,
    ) {
        for child in self.arena.groups[source].clone() {
            self.transform(child, target, context.clone(), depth);
        }
    }

    fn transform(
        &mut self,
        id: NodeId,
        parent: GroupId,
        mut context: IndexMap<String, ContextValue>,
        depth: usize,
    ) {
        let data = self.arena.nodes[id].data.clone();
        let source = self.arena.nodes[id].children;
        match &data {
            AstNode::Declaration {
                property, value, ..
            } => {
                let Some(value) = value else { return };
                if property == "--tw-sort" {
                    return;
                }
                if enabled(&context, "theme") && property.starts_with("--") {
                    if value == "initial" {
                        if let AstNode::Declaration { value, .. } = &mut self.arena.nodes[id].data {
                            *value = None;
                        }
                        return;
                    }
                    if !enabled(&context, "keyframes") {
                        self.variables.entry(parent).or_default().insert(id);
                    }
                }
                if value.contains("var(") {
                    for variable in extract_used_variables(value) {
                        if enabled(&context, "theme") && property.starts_with("--") {
                            self.dependencies
                                .entry(variable)
                                .or_default()
                                .insert(property.to_string());
                        } else {
                            self.theme.mark_used_variable(&variable);
                        }
                    }
                }
                if property == "animation" {
                    self.used_keyframes.extend(keyframe_names(value));
                }
                if self.polyfills & COLOR_MIX != 0
                    && !enabled(&context, "supportsColorMix")
                    && !enabled(&context, "keyframes")
                    && value.contains("color-mix(")
                {
                    self.mixes.entry(parent).or_default().insert(id);
                }
                self.arena.groups[parent].push(id);
            }
            AstNode::Rule { .. } => {
                let children = self.arena.group(Vec::new());
                self.transform_group(source.unwrap(), children, &context, depth + 1);
                if !self.arena.groups[children].is_empty() {
                    let copy = self.arena.node(data, Some(children));
                    self.arena.groups[parent].push(copy);
                }
            }
            AstNode::AtRule { name, params, .. } if name == "@property" && depth == 0 => {
                if !self.properties.insert(params.clone()) {
                    return;
                }
                if self.polyfills & AT_PROPERTY != 0 {
                    let mut initial = None;
                    let mut inherits = false;
                    for &child in &self.arena.groups[source.unwrap()] {
                        if let AstNode::Declaration {
                            property, value, ..
                        } = &self.arena.nodes[child].data
                        {
                            if property == "initial-value" {
                                initial = value.clone();
                            } else if property == "inherits" {
                                inherits = value.as_deref() == Some("true");
                            }
                        }
                    }
                    let fallback = self.arena.import_node(AstNode::decl(
                        params.clone(),
                        initial.unwrap_or("initial".into()),
                    ));
                    if inherits {
                        self.root_fallbacks.push(fallback);
                    } else {
                        self.universal_fallbacks.push(fallback);
                    }
                }
                let children = self.arena.group(Vec::new());
                self.transform_group(source.unwrap(), children, &context, depth + 1);
                let copy = self.arena.node(data, Some(children));
                self.arena.groups[parent].push(copy);
            }
            AstNode::AtRule { name, params, .. } => {
                if name == "@keyframes" {
                    context.insert("keyframes".into(), ContextValue::Bool(true));
                } else if name == "@supports" && params.contains("color-mix(") {
                    context.insert("supportsColorMix".into(), ContextValue::Bool(true));
                }
                let children = self.arena.group(Vec::new());
                self.transform_group(source.unwrap(), children, &context, depth + 1);
                let keep = !self.arena.groups[children].is_empty()
                    || matches!(
                        name.as_str(),
                        "@layer"
                            | "@charset"
                            | "@custom-media"
                            | "@namespace"
                            | "@import"
                            | "@apply"
                    );
                let keyframe = name == "@keyframes" && enabled(&context, "theme");
                let copy = self.arena.node(data, Some(children));
                if keyframe {
                    self.keyframes.insert(copy);
                }
                if keep {
                    self.arena.groups[parent].push(copy);
                }
            }
            AstNode::AtRoot { .. } => {
                for child in self.arena.groups[source.unwrap()].clone() {
                    let target = self.arena.group(Vec::new());
                    self.transform(child, target, context.clone(), 0);
                    let appended = self.arena.groups[target].clone();
                    self.arena.groups[self.at_roots].extend(appended);
                }
            }
            AstNode::Context { context: extra, .. } => {
                if enabled(extra, "reference") {
                    return;
                }
                context.extend(extra.clone());
                if enabled(extra, "source") {
                    let children = self.arena.group(Vec::new());
                    self.transform_group(source.unwrap(), children, &context, depth);
                    if !self.arena.groups[children].is_empty() {
                        let copy = self.arena.node(data, Some(children));
                        self.arena.groups[parent].push(copy);
                    }
                } else {
                    self.transform_group(source.unwrap(), parent, &context, depth);
                }
            }
            AstNode::Comment { .. } => self.arena.groups[parent].push(id),
        }
    }

    fn variable_used(&self, variable: &str, seen: &mut HashSet<String>) -> bool {
        if !seen.insert(variable.to_string())
            || self.theme.get_options(variable) & (theme::STATIC | theme::USED) != 0
        {
            return true;
        }
        self.dependencies.get(variable).is_some_and(|dependencies| {
            dependencies
                .iter()
                .any(|dependency| self.variable_used(dependency, seen))
        })
    }

    fn find_rule(&self, group: GroupId, target: GroupId, path: &mut Vec<NodeId>) -> bool {
        for &id in &self.arena.groups[group] {
            path.push(id);
            if matches!(self.arena.nodes[id].data, AstNode::Rule { .. })
                && self.arena.nodes[id].children == Some(target)
            {
                return true;
            }
            if let Some(children) = self.arena.nodes[id].children
                && self.find_rule(children, target, path)
            {
                return true;
            }
            path.pop();
        }
        false
    }

    fn remove_unused(&mut self) -> Result<(), String> {
        for (parent, declarations) in self.variables.clone() {
            for declaration in declarations {
                let AstNode::Declaration {
                    property, value, ..
                } = &self.arena.nodes[declaration].data
                else {
                    unreachable!()
                };
                if self.variable_used(property, &mut HashSet::new()) {
                    if property.starts_with(&self.theme.prefix_key("--animate-")?) {
                        self.used_keyframes
                            .extend(keyframe_names(value.as_deref().unwrap()));
                    }
                    continue;
                }
                let group = &mut self.arena.groups[parent];
                if !group.is_empty() {
                    let index = group
                        .iter()
                        .position(|&id| id == declaration)
                        .unwrap_or(group.len() - 1);
                    group.remove(index);
                }
                if group.is_empty() {
                    let mut path = Vec::new();
                    if !self.find_rule(self.root, parent, &mut path) {
                        break;
                    }
                    while let Some(id) = path.pop() {
                        let remove_from = if let Some(&ancestor) = path.last() {
                            if !matches!(
                                self.arena.nodes[ancestor].data,
                                AstNode::AtRoot { .. } | AstNode::AtRule { .. }
                            ) {
                                break;
                            }
                            self.arena.nodes[ancestor].children.unwrap()
                        } else {
                            self.root
                        };
                        let Some(index) =
                            self.arena.groups[remove_from].iter().position(|&n| n == id)
                        else {
                            break;
                        };
                        self.arena.groups[remove_from].remove(index);
                    }
                    break;
                }
            }
        }
        for &keyframe in &self.keyframes {
            let AstNode::AtRule { params, .. } = &self.arena.nodes[keyframe].data else {
                unreachable!()
            };
            if !self.used_keyframes.contains(params) {
                let group = &mut self.arena.groups[self.at_roots];
                if !group.is_empty() {
                    let index = group
                        .iter()
                        .position(|&id| id == keyframe)
                        .unwrap_or(group.len() - 1);
                    group.remove(index);
                }
            }
        }
        Ok(())
    }

    fn color_mix_fallbacks(&mut self) -> Result<(), String> {
        for (parent, declarations) in self.mixes.clone() {
            for declaration in declarations {
                let Some(index) = self.arena.groups[parent]
                    .iter()
                    .position(|&id| id == declaration)
                else {
                    continue;
                };
                let AstNode::Declaration {
                    value: Some(raw), ..
                } = &self.arena.nodes[declaration].data
                else {
                    continue;
                };
                let mut parsed = value::parse(raw);
                let mut required = false;
                mix_values(&mut parsed, self.theme, &mut required)?;
                if !required {
                    continue;
                }
                let mut fallback = self.arena.nodes[declaration].data.clone();
                fallback.fresh_declarations();
                if let AstNode::Declaration { value, .. } = &mut fallback {
                    *value = Some(value::to_css(&parsed));
                }
                let fallback = self.arena.import_node(fallback);
                let children = self.arena.group(vec![declaration]);
                let query = self.arena.node(
                    AstNode::AtRule {
                        name: "@supports".into(),
                        params: "(color: color-mix(in lab, red, red))".into(),
                        nodes: Vec::new(),
                    },
                    Some(children),
                );
                self.arena.groups[parent].splice(index..=index, [fallback, query]);
            }
        }
        Ok(())
    }

    fn property_fallbacks(&mut self) {
        let mut fallbacks = Vec::new();
        for (selector, nodes) in [
            (":root, :host", std::mem::take(&mut self.root_fallbacks)),
            (
                "*, ::before, ::after, ::backdrop",
                std::mem::take(&mut self.universal_fallbacks),
            ),
        ] {
            if nodes.is_empty() {
                continue;
            }
            let children = self.arena.group(nodes);
            fallbacks.push(self.arena.node(
                AstNode::Rule {
                    selector: selector.into(),
                    nodes: Vec::new(),
                },
                Some(children),
            ));
        }
        if fallbacks.is_empty() {
            return;
        }
        let index = self.arena.groups[self.root]
            .iter()
            .position(|&id| match &self.arena.nodes[id].data {
                AstNode::Comment { .. } => false,
                AstNode::AtRule { name, .. } if name == "@charset" || name == "@import" => false,
                _ => true,
            })
            .unwrap_or(self.arena.groups[self.root].len());
        let statement = self.arena.import_node(AstNode::AtRule {
            name: "@layer".into(),
            params: "properties".into(),
            nodes: Vec::new(),
        });
        self.arena.groups[self.root].insert(index, statement);
        let children = self.arena.group(fallbacks);
        let supports = self.arena.node(AstNode::AtRule {
            name: "@supports".into(),
            params: "((-webkit-hyphens: none) and (not (margin-trim: block))) or ((-moz-orient: inline) and (not (color:rgb(from red r g b))))".into(),
            nodes: Vec::new(),
        }, Some(children));
        let children = self.arena.group(vec![supports]);
        let layer = self.arena.node(
            AstNode::AtRule {
                name: "@layer".into(),
                params: "properties".into(),
                nodes: Vec::new(),
            },
            Some(children),
        );
        self.arena.groups[self.root].push(layer);
    }
}

fn resolve_mix_variables(
    nodes: &mut [Value],
    theme: &Theme,
    required: &mut bool,
    unresolved: &mut bool,
    current: &mut bool,
) {
    for node in nodes {
        if matches!(node, Value::Word { value } if value.to_lowercase() == "currentcolor") {
            *current = true;
            *required = true;
            continue;
        }
        let mut variable = node.clone();
        let mut seen = HashSet::new();
        let replacement = loop {
            let Value::Function { value, nodes } = &variable else {
                break None;
            };
            if value != "var" {
                break None;
            }
            let Some(Value::Word { value: name }) = nodes.first() else {
                break None;
            };
            if !seen.insert(name.clone()) {
                *unresolved = true;
                break None;
            }
            *required = true;
            let Some(inlined) = theme
                .resolve_value(None, std::slice::from_ref(name))
                .filter(|value| !value.is_empty())
            else {
                *unresolved = true;
                break None;
            };
            if inlined.to_lowercase() == "currentcolor" {
                *current = true;
                break None;
            }
            if inlined.starts_with("var(") {
                if let Some(next) = value::parse(inlined).into_iter().next() {
                    variable = next;
                } else {
                    break Some(inlined.to_string());
                }
            } else {
                break Some(inlined.to_string());
            }
        };
        if let Some(inlined) = replacement {
            *node = Value::word(inlined);
        }
        if let Value::Function { nodes, .. } = node {
            resolve_mix_variables(nodes, theme, required, unresolved, current);
        }
    }
}

fn mix_values(nodes: &mut [Value], theme: &Theme, required: &mut bool) -> Result<(), String> {
    for node in nodes {
        loop {
            let mut replacement = None;
            if let Value::Function { value, nodes } = node
                && value == "color-mix"
            {
                let mut unresolved = false;
                let mut current = false;
                resolve_mix_variables(nodes, theme, required, &mut unresolved, &mut current);
                if unresolved || current {
                    if let Some(index) = nodes.iter().position(|node| matches!(node, Value::Separator { value } if value.trim_matches(is_js_whitespace).contains(','))) {
                            replacement = nodes.get(index + 1).cloned();
                        }
                } else if *required {
                    let colorspace = nodes.get_mut(2).ok_or_else(|| {
                        "Cannot read properties of undefined (reading 'kind')".to_string()
                    })?;
                    if let Value::Word { value } = colorspace
                        && matches!(value.as_str(), "oklab" | "oklch" | "lab" | "lch")
                    {
                        *value = "srgb".into();
                    }
                }
            }
            if let Some(replacement) = replacement {
                *node = replacement;
            } else {
                break;
            }
        }
        if let Value::Function { nodes, .. } = node {
            mix_values(nodes, theme, required)?;
        }
    }
    Ok(())
}

pub fn optimize_ast(
    ast: Vec<AstNode>,
    theme: &mut Theme,
    polyfills: u8,
) -> Result<Vec<AstNode>, String> {
    let mut arena = Arena::default();
    let source = arena.import(ast);
    let root = arena.group(Vec::new());
    let at_roots = arena.group(Vec::new());
    let mut state = Optimizer {
        arena,
        theme,
        polyfills,
        root,
        at_roots,
        properties: HashSet::new(),
        variables: IndexMap::new(),
        mixes: IndexMap::new(),
        keyframes: IndexSet::new(),
        used_keyframes: HashSet::new(),
        root_fallbacks: Vec::new(),
        universal_fallbacks: Vec::new(),
        dependencies: IndexMap::new(),
    };
    state.transform_group(source, root, &IndexMap::new(), 0);
    state.remove_unused()?;
    let mut combined = state.arena.groups[root].clone();
    combined.extend(&state.arena.groups[at_roots]);
    state.root = state.arena.group(combined);
    if polyfills & COLOR_MIX != 0 {
        state.color_mix_fallbacks()?;
    }
    if polyfills & AT_PROPERTY != 0 {
        state.property_fallbacks();
    }
    handle_nesting(state.arena.into_export(state.root))
}
