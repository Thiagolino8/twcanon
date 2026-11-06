// Port of Tailwind CSS 4.3.4 ast.ts handleNesting. See LICENSE.tailwind.
use crate::{
    ast::{AstNode, ContextValue},
    ast_arena::{Arena, GroupId, NodeId},
    syntax::{is_js_whitespace, segment, selector, value},
};
use indexmap::IndexSet;
use selector::Selector;
use std::collections::HashSet;

fn truthy(value: &ContextValue) -> bool {
    match value {
        ContextValue::String(value) => !value.is_empty(),
        ContextValue::Bool(value) => *value,
    }
}

fn universal(node: &Selector) -> bool {
    matches!(node, Selector::Selector { value } if value.starts_with('*'))
}

fn type_selector(node: &Selector) -> bool {
    matches!(node, Selector::Selector { value }
        if !value.starts_with(['*', '&', '.', '#', ':', '[']))
}

fn standard_pseudo(node: &Selector) -> bool {
    if !node.is_pseudo_element() {
        return false;
    }
    let (Selector::Selector { value } | Selector::Function { value, .. }) = node else {
        return false;
    };
    let value = value.to_lowercase();
    !value.starts_with("::")
        || value.starts_with("::-")
        || matches!(
            value.as_str(),
            "::after"
                | "::backdrop"
                | "::before"
                | "::checkmark"
                | "::column"
                | "::cue"
                | "::cue-region"
                | "::details-content"
                | "::file-selector-button"
                | "::first-letter"
                | "::first-line"
                | "::grammar-error"
                | "::highlight"
                | "::marker"
                | "::part"
                | "::picker"
                | "::picker-icon"
                | "::placeholder"
                | "::scroll-button"
                | "::scroll-marker"
                | "::scroll-marker-group"
                | "::search-text"
                | "::selection"
                | "::slotted"
                | "::spelling-error"
                | "::target-text"
                | "::view-transition"
                | "::view-transition-group"
                | "::view-transition-group-children"
                | "::view-transition-image-pair"
                | "::view-transition-new"
                | "::view-transition-old"
        )
}

fn dead_selector(node: &Selector) -> bool {
    match node {
        Selector::List { nodes } => nodes.iter().all(dead_selector),
        Selector::Complex { nodes } => nodes.iter().enumerate().any(|(index, child)| {
            if matches!(child, Selector::Combinator { .. }) {
                nodes.get(index.wrapping_sub(1)).is_some_and(|previous| {
                    standard_pseudo(previous)
                        || matches!(previous, Selector::Compound { nodes } if nodes.iter().any(standard_pseudo))
                })
            } else {
                dead_selector(child)
            }
        }),
        Selector::Compound { nodes } => nodes.iter().any(dead_selector),
        Selector::Function { value, nodes } if value == ":is" || value == ":where" => {
            never_matches(nodes)
        }
        _ => false,
    }
}

fn never_matches(nodes: &[Selector]) -> bool {
    !nodes.is_empty() && nodes.iter().all(dead_selector)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Parent {
    Root,
    Complex,
    Compound,
    Function,
    List,
}

fn substitute_selector(input: &str, parent: &str) -> Result<String, String> {
    let parent_ast = selector::parse(parent);
    let is_list = matches!(parent_ast.as_slice(), [Selector::List { .. }]);
    let first = parent_ast.first();
    let complex = matches!(first, Some(Selector::Complex { .. }));
    let mut ast = selector::parse(input);
    let mut changed = false;
    struct Replacement<'a> {
        parent: &'a str,
        first: Option<&'a Selector>,
        list: bool,
        complex: bool,
    }
    fn walk(
        nodes: &mut Vec<Selector>,
        kind: Parent,
        compound_after_first: bool,
        replacement: &Replacement<'_>,
        changed: &mut bool,
    ) -> Result<(), String> {
        let mut index = 0;
        while index < nodes.len() {
            let nesting = matches!(&nodes[index], Selector::Selector { value } if value == "&");
            if nesting {
                *changed = true;
                let mut simple = false;
                if kind != Parent::Root && !replacement.list {
                    match kind {
                        Parent::Complex if index == 0 => simple = true,
                        Parent::Complex => {
                            if replacement.first.is_none() {
                                return Err(
                                    "Cannot read properties of undefined (reading 'kind')".into()
                                );
                            }
                            simple = !replacement.complex;
                        }
                        Parent::Compound => {
                            let first = replacement.first.ok_or_else(|| {
                                "Cannot read properties of undefined (reading 'kind')".to_string()
                            })?;
                            if !(replacement.complex && compound_after_first
                                || nodes[index + 1..]
                                    .iter()
                                    .any(|n| universal(n) || type_selector(n)))
                            {
                                simple = index == 0
                                    || !(replacement.complex
                                        || universal(first)
                                        || type_selector(first));
                            }
                        }
                        Parent::Function => simple = true,
                        _ => {}
                    }
                }
                nodes[index] = Selector::Selector {
                    value: if simple {
                        replacement.parent.to_string()
                    } else {
                        format!(":is({})", replacement.parent)
                    },
                };
            }
            let child_kind = match &nodes[index] {
                Selector::List { .. } => Parent::List,
                Selector::Complex { .. } => Parent::Complex,
                Selector::Compound { .. } => Parent::Compound,
                Selector::Function { .. } => Parent::Function,
                _ => Parent::Root,
            };
            if let Some(children) = nodes[index].children_mut() {
                walk(
                    children,
                    child_kind,
                    kind == Parent::Complex && index != 0,
                    replacement,
                    changed,
                )?;
            }
            if index == 0
                && nodes.len() > 1
                && kind == Parent::Compound
                && universal(&nodes[index])
                && !matches!(&nodes[1], Selector::Selector { value } if value.starts_with('|'))
            {
                nodes.remove(index);
            } else {
                index += 1;
            }
        }
        Ok(())
    }
    if input.contains('&') {
        walk(
            &mut ast,
            Parent::Root,
            false,
            &Replacement {
                parent,
                first,
                list: is_list,
                complex,
            },
            &mut changed,
        )?;
        if changed {
            return Ok(selector::to_css(&ast, false));
        }
    }
    Ok(format!(
        "{} {input}",
        if is_list {
            format!(":is({parent})")
        } else {
            parent.to_string()
        }
    ))
}

fn replace_root_nesting(nodes: &mut [Selector], replacement: &str) -> bool {
    let mut changed = false;
    for node in nodes {
        if let Selector::Selector { value } = node
            && value == "&"
        {
            *value = replacement.to_string();
            changed = true;
        }
        if let Some(children) = node.children_mut() {
            changed |= replace_root_nesting(children, replacement);
        }
    }
    changed
}

fn contains_scope(node: &AstNode) -> bool {
    match node {
        AstNode::AtRule { name, nodes, .. } => name == "@scope" || nodes.iter().any(contains_scope),
        AstNode::Rule { nodes, .. }
        | AstNode::AtRoot { nodes }
        | AstNode::Context { nodes, .. } => nodes.iter().any(contains_scope),
        _ => false,
    }
}

fn wrap_scope_declarations(nodes: Vec<AstNode>) -> Vec<AstNode> {
    let mut output = Vec::new();
    let mut group = None;
    for mut node in nodes {
        if let AstNode::Context { nodes, .. } = &mut node {
            *nodes = wrap_scope_declarations(std::mem::take(nodes));
        }
        if matches!(node, AstNode::Declaration { .. } | AstNode::Comment { .. }) {
            let index = *group.get_or_insert_with(|| {
                let index = output.len();
                output.push(AstNode::Rule {
                    selector: "&".into(),
                    nodes: Vec::new(),
                });
                index
            });
            output[index].children_mut().unwrap().push(node);
        } else {
            group = None;
            output.push(node);
        }
    }
    output
}

#[derive(Default)]
struct Nesting {
    arena: Arena,
    result: GroupId,
    current: Option<GroupId>,
    selectors: Vec<(String, bool)>,
    at_rules: Vec<(String, String)>,
    sources: Vec<ContextValue>,
    properties: HashSet<String>,
    dedupe: IndexSet<GroupId>,
}

impl Nesting {
    fn dead(&self) -> bool {
        self.selectors.last().is_some_and(|entry| entry.1)
    }

    fn emit(&mut self, node: AstNode) {
        let id = self.arena.import_node(node);
        if let Some(group) = self.current {
            if let AstNode::Declaration { property, .. } = &self.arena.nodes[id].data
                && !self.properties.insert(property.to_string())
            {
                self.dedupe.insert(group);
            }
            self.arena.groups[group].push(id);
            return;
        }
        if self.selectors.is_empty() && self.at_rules.is_empty() {
            if let Some(&last) = self.arena.groups[self.result].last()
                && let (
                    AstNode::AtRule {
                        name: a, params: b, ..
                    },
                    AstNode::AtRule {
                        name: c, params: d, ..
                    },
                ) = (&self.arena.nodes[last].data, &self.arena.nodes[id].data)
                && a == c
                && b == d
                && self.arena.groups[self.arena.nodes[last].children.unwrap()].is_empty()
                && self.arena.groups[self.arena.nodes[id].children.unwrap()].is_empty()
            {
                return;
            }
            self.arena.groups[self.result].push(id);
            return;
        }
        let group = self.arena.group(vec![id]);
        self.current = Some(group);
        self.properties.clear();
        if let AstNode::Declaration { property, .. } = &self.arena.nodes[id].data {
            self.properties.insert(property.to_string());
        }
        let mut target = self.result;
        let mut offset = 0;
        for (name, params) in &self.at_rules {
            let Some(&last) = self.arena.groups[target].last() else {
                break;
            };
            if !matches!(&self.arena.nodes[last].data, AstNode::AtRule { name: a, params: b, .. } if a == name && b == params)
            {
                break;
            }
            offset += 1;
            target = self.arena.nodes[last].children.unwrap();
        }
        let mut root: Option<NodeId> = None;
        if let Some((selector, _)) = self.selectors.last() {
            if self.at_rules.len() == offset
                && let Some(&last) = self.arena.groups[target].last()
                && matches!(&self.arena.nodes[last].data, AstNode::Rule { selector: s, .. } if s == selector)
            {
                let children = self.arena.nodes[last].children.unwrap();
                let appended = self.arena.groups[group].clone();
                self.arena.groups[children].extend(appended);
                self.current = Some(children);
                self.dedupe.insert(children);
                return;
            }
            root = Some(
                self.arena
                    .node(crate::ast::rule(selector, Vec::new()), Some(group)),
            );
        }
        for (name, params) in self.at_rules[offset..].iter().rev() {
            let children = root.map(|id| self.arena.group(vec![id])).unwrap_or(group);
            root = Some(self.arena.node(
                AstNode::AtRule {
                    name: name.clone(),
                    params: params.clone(),
                    nodes: Vec::new(),
                },
                Some(children),
            ));
        }
        if let Some(root) = root {
            self.arena.groups[target].push(root);
        } else {
            let appended = self.arena.groups[group].clone();
            self.arena.groups[target].extend(appended);
        }
    }

    fn walk(&mut self, nodes: Vec<AstNode>) -> Result<(), String> {
        for node in nodes {
            self.visit(node)?;
        }
        Ok(())
    }

    fn visit(&mut self, node: AstNode) -> Result<(), String> {
        let has_scope = contains_scope(&node);
        match node {
            AstNode::Rule { selector, nodes } => {
                self.current = None;
                if self.selectors.is_empty() {
                    let mut parsed = selector::parse(&selector);
                    let replacement = if self.at_rules.iter().any(|(name, _)| name == "@scope") {
                        ":where(:scope)"
                    } else {
                        ":scope"
                    };
                    let changed =
                        selector.contains('&') && replace_root_nesting(&mut parsed, replacement);
                    self.selectors.push((
                        if changed {
                            selector::to_css(&parsed, false)
                        } else {
                            selector
                        },
                        false,
                    ));
                } else {
                    if selector == "&" {
                        return self.walk(nodes);
                    }
                    let parent = &self.selectors.last().unwrap().0;
                    let selectors = segment::segment(&selector, b',')
                        .into_iter()
                        .map(|part| substitute_selector(part, parent))
                        .collect::<Result<Vec<_>, _>>()?;
                    let live = selectors
                        .iter()
                        .filter(|s| !never_matches(&selector::parse(s)))
                        .cloned()
                        .collect::<Vec<_>>();
                    if live.is_empty() {
                        self.selectors.push((selectors.join(", "), true));
                        self.walk(nodes)?;
                        self.current = None;
                        self.selectors.pop();
                        return Ok(());
                    }
                    self.selectors.push((live.join(", "), false));
                }
                if !has_scope
                    && nodes
                        .iter()
                        .any(|node| matches!(node, AstNode::Declaration { .. }))
                {
                    for child in nodes {
                        self.emit(child);
                    }
                } else {
                    self.walk(nodes)?;
                }
                self.current = None;
                self.selectors.pop();
            }
            AstNode::AtRule {
                name,
                mut params,
                mut nodes,
            } => {
                self.current = None;
                let hoist = matches!(
                    name.as_str(),
                    "@media"
                        | "@supports"
                        | "@container"
                        | "@layer"
                        | "@scope"
                        | "@starting-style"
                        | "@page"
                        | "@view-transition"
                );
                let droppable = matches!(
                    name.as_str(),
                    "@media"
                        | "@supports"
                        | "@container"
                        | "@scope"
                        | "@starting-style"
                        | "@page"
                        | "@view-transition"
                );
                if self.dead() {
                    if name == "@layer" && nodes.is_empty() {
                        let saved = std::mem::take(&mut self.selectors);
                        self.emit(AstNode::AtRule {
                            name,
                            params,
                            nodes,
                        });
                        self.selectors = saved;
                        return Ok(());
                    }
                    if nodes.is_empty() || !hoist {
                        return Ok(());
                    }
                }
                if nodes.is_empty() && !droppable || !hoist {
                    self.emit(AstNode::AtRule {
                        name,
                        params,
                        nodes,
                    });
                    return Ok(());
                }
                let mut masked = None;
                if name == "@scope" {
                    nodes = wrap_scope_declarations(nodes);
                    let is_variant = matches!(self.sources.last(), Some(ContextValue::String(value)) if value == "variant");
                    let parent = self.selectors.last().map(|entry| entry.0.clone());
                    if params.contains('&') || !is_variant && parent.is_some() {
                        let mut parsed = value::parse(&params);
                        let mut end = false;
                        let mut consumed = false;
                        let mut start = None;
                        for param in &mut parsed {
                            if matches!(param, value::Value::Word { value } if value == "to") {
                                end = true;
                                continue;
                            }
                            let value::Value::Function { value, nodes } = param else {
                                continue;
                            };
                            if !value.is_empty() {
                                continue;
                            }
                            let selector = value::to_css(nodes);
                            if !end && !is_variant && parent.is_some() {
                                let substituted = segment::segment(&selector, b',')
                                    .into_iter()
                                    .map(|part| {
                                        substitute_selector(
                                            part.trim_matches(is_js_whitespace),
                                            parent.as_ref().unwrap(),
                                        )
                                    })
                                    .collect::<Result<Vec<_>, _>>()?
                                    .join(", ");
                                *nodes = vec![value::Value::word(substituted)];
                                consumed = true;
                            } else if selector.contains('&') {
                                *nodes = vec![value::Value::word(substitute_selector(
                                    &selector,
                                    if end {
                                        ":where(:scope)"
                                    } else {
                                        parent.as_deref().unwrap_or(":scope")
                                    },
                                )?)];
                                if !end && parent.is_some() {
                                    consumed = true;
                                }
                            }
                            if !end {
                                start = Some(value::to_css(nodes));
                            }
                        }
                        params = value::to_css(&parsed);
                        if consumed {
                            if self.dead()
                                && start
                                    .is_some_and(|start| never_matches(&selector::parse(&start)))
                            {
                                return Ok(());
                            }
                            masked = Some(std::mem::take(&mut self.selectors));
                        }
                    }
                }
                self.at_rules.push((name, params));
                self.walk(nodes)?;
                self.current = None;
                self.at_rules.pop();
                if let Some(saved) = masked {
                    self.selectors = saved;
                }
            }
            AstNode::Declaration { .. } | AstNode::Comment { .. } => {
                if !self.dead() {
                    self.emit(node);
                }
            }
            AstNode::Context { context, nodes } => {
                let source = context.get("source").filter(|value| truthy(value)).cloned();
                if let Some(source) = source.as_ref() {
                    self.sources.push(source.clone());
                }
                self.walk(nodes)?;
                if source.is_some() {
                    self.sources.pop();
                }
            }
            AstNode::AtRoot { nodes } => self.walk(nodes)?,
        }
        Ok(())
    }
}

pub fn handle_nesting(ast: Vec<AstNode>) -> Result<Vec<AstNode>, String> {
    let mut state = Nesting::default();
    state.result = state.arena.group(Vec::new());
    state.walk(ast)?;
    for &group in &state.dedupe {
        let mut seen = HashSet::new();
        let arena = &mut state.arena;
        arena.groups[group].reverse();
        arena.groups[group].retain(|&id| match &arena.nodes[id].data {
            AstNode::Declaration {
                property,
                value,
                important,
                ..
            } => seen.insert(format!(
                "{property}\0{}\0{important}",
                value.as_deref().unwrap_or("undefined")
            )),
            _ => true,
        });
        arena.groups[group].reverse();
    }
    Ok(state.arena.into_export(state.result))
}
