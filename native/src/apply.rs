// Port of Tailwind CSS 4.3.4 apply.ts (source maps are omitted).
// See LICENSE.tailwind.
use crate::{
    ast::{self, AstNode},
    ast_arena::{Arena, GroupId, NodeId},
    candidate::CandidateKind,
    compile::DesignSystem,
    syntax::{is_js_whitespace, segment::segment},
};
use indexmap::{IndexMap, IndexSet};
use std::collections::VecDeque;

fn words(input: &str) -> Vec<&str> {
    let mut result = Vec::new();
    let mut start = 0;
    let mut whitespace = false;
    for (offset, c) in input.char_indices() {
        if is_js_whitespace(c) {
            if !whitespace {
                result.push(&input[start..offset]);
                whitespace = true;
            }
            start = offset + c.len_utf8();
        } else {
            whitespace = false;
        }
    }
    result.push(&input[start..]);
    result
}

struct Apply<'a> {
    arena: Arena,
    system: &'a mut DesignSystem,
    parents: IndexSet<NodeId>,
    dependencies: IndexMap<NodeId, IndexSet<String>>,
    definitions: IndexMap<String, IndexSet<NodeId>>,
    registered_utilities: VecDeque<usize>,
    utility_bindings: IndexMap<NodeId, usize>,
    changed: bool,
}

impl Apply<'_> {
    fn node(&self, id: NodeId) -> AstNode {
        let mut node = self.arena.nodes[id].data.clone();
        if let Some(children) = self.arena.nodes[id].children {
            *node.children_mut().unwrap() = self.arena.export(children);
        }
        node
    }

    fn apply_nodes(&self, group: GroupId, output: &mut Vec<NodeId>) {
        for &id in &self.arena.groups[group] {
            if matches!(&self.arena.nodes[id].data, AstNode::AtRule { name, .. } if name == "@apply")
            {
                output.push(id);
            }
            if let Some(children) = self.arena.nodes[id].children {
                self.apply_nodes(children, output);
            }
        }
    }

    fn resolve_dependencies(&mut self, id: NodeId) -> IndexSet<String> {
        let AstNode::AtRule { params, .. } = self.arena.nodes[id].data.clone() else {
            unreachable!()
        };
        let mut dependencies = IndexSet::new();
        for raw in words(&params) {
            for candidate in self.system.parse_candidate(raw) {
                match candidate.kind {
                    CandidateKind::Arbitrary { .. } => {}
                    CandidateKind::Static { root } | CandidateKind::Functional { root, .. } => {
                        dependencies.insert(root);
                    }
                }
            }
        }
        dependencies
    }

    fn collect(&mut self, group: GroupId, path: &mut Vec<NodeId>) -> Result<(), String> {
        for id in self.arena.groups[group].clone() {
            if let AstNode::Context { context, .. } = &self.arena.nodes[id].data
                && let Some(crate::ast::ContextValue::String(binding)) =
                    context.get("__native_utility_binding")
                && let Ok(binding) = binding.parse::<usize>()
                && let Some(children) = self.arena.nodes[id].children
            {
                for &child in &self.arena.groups[children] {
                    if matches!(&self.arena.nodes[child].data, AstNode::AtRule { name, .. } if name == "@utility")
                    {
                        self.utility_bindings.insert(child, binding);
                    }
                }
            }
            if let AstNode::AtRule { name, params, .. } = self.arena.nodes[id].data.clone() {
                let children = self.arena.nodes[id].children.unwrap();
                if name == "@keyframes" {
                    let mut applies = Vec::new();
                    self.apply_nodes(children, &mut applies);
                    if !applies.is_empty() {
                        return Err("You cannot use `@apply` inside `@keyframes`.".into());
                    }
                    continue;
                }
                if name == "@utility" {
                    if let Some(binding) = self.registered_utilities.pop_front() {
                        self.utility_bindings.insert(id, binding);
                    }
                    self.definitions
                        .entry(params.strip_suffix("-*").unwrap_or(&params).into())
                        .or_default()
                        .insert(id);
                    let mut applies = Vec::new();
                    self.apply_nodes(children, &mut applies);
                    for apply in applies {
                        self.parents.insert(id);
                        let dependencies = self.resolve_dependencies(apply);
                        self.dependencies
                            .entry(id)
                            .or_default()
                            .extend(dependencies);
                    }
                }
                if name == "@apply"
                    && let Some(&parent) = path.last()
                {
                    self.changed = true;
                    self.parents.insert(parent);
                    let dependencies = self.resolve_dependencies(id);
                    for &ancestor in path.iter() {
                        if self.parents.contains(&ancestor) {
                            self.dependencies
                                .entry(ancestor)
                                .or_default()
                                .extend(dependencies.clone());
                        }
                    }
                }
            }
            if let Some(children) = self.arena.nodes[id].children {
                path.push(id);
                self.collect(children, path)?;
                path.pop();
            }
        }
        Ok(())
    }

    fn visit(
        &mut self,
        id: NodeId,
        path: &mut Vec<NodeId>,
        seen: &mut IndexSet<NodeId>,
        wip: &mut IndexSet<NodeId>,
        sorted: &mut Vec<NodeId>,
    ) -> Result<(), String> {
        if seen.contains(&id) {
            return Ok(());
        }
        if wip.contains(&id) {
            let next = path[(path.iter().position(|&node| node == id).unwrap() + 1) % path.len()];
            if let (
                AstNode::AtRule { name, .. },
                AstNode::AtRule {
                    name: next_name,
                    params,
                    ..
                },
            ) = (&self.arena.nodes[id].data, &self.arena.nodes[next].data)
                && name == "@utility"
                && next_name == "@utility"
            {
                let next_root = params.strip_suffix("-*").unwrap_or(params).to_owned();
                let mut applies = Vec::new();
                self.apply_nodes(self.arena.nodes[id].children.unwrap(), &mut applies);
                for apply in applies {
                    let AstNode::AtRule { params, .. } = self.arena.nodes[apply].data.clone()
                    else {
                        unreachable!()
                    };
                    for raw in words(&params) {
                        for candidate in self.system.parse_candidate(raw) {
                            if matches!(&candidate.kind, CandidateKind::Static { root } | CandidateKind::Functional { root, .. } if root == &next_root)
                            {
                                return Err(format!(
                                    "You cannot `@apply` the `{raw}` utility here because it creates a circular dependency."
                                ));
                            }
                        }
                    }
                }
            }
            return Err(format!(
                "Circular dependency detected:\n\n{}\nRelies on:\n\n{}",
                ast::to_css(&[self.node(id)]),
                ast::to_css(&[self.node(next)])
            ));
        }
        wip.insert(id);
        for dependency in self.dependencies.get(&id).cloned().unwrap_or_default() {
            for dependency in self
                .definitions
                .get(&dependency)
                .cloned()
                .unwrap_or_default()
            {
                path.push(id);
                self.visit(dependency, path, seen, wip, sorted)?;
                path.pop();
            }
        }
        seen.insert(id);
        wip.shift_remove(&id);
        sorted.push(id);
        Ok(())
    }

    fn invalid(system: &mut DesignSystem, candidate: &str) -> Result<String, String> {
        if let Some(prefix) = system
            .theme
            .prefix
            .as_deref()
            .filter(|prefix| !prefix.is_empty())
            && !candidate.starts_with(prefix)
        {
            return Ok(format!(
                "Cannot apply unprefixed utility class `{candidate}`. Did you mean `{prefix}:{candidate}`?"
            ));
        }
        if system.invalid_candidates.contains(candidate) {
            return Ok(format!(
                "Cannot apply utility class `{candidate}` because it has been explicitly disabled: https://tailwindcss.com/docs/detecting-classes-in-source-files#explicitly-excluding-classes"
            ));
        }
        let mut parts = segment(candidate, b':');
        if parts.len() > 1 {
            let utility = parts.pop().unwrap();
            if system.candidates_to_css(&[utility.into()])?[0].is_some() {
                let probes = parts
                    .iter()
                    .map(|variant| format!("{variant}:[--tw-variant-check:1]"))
                    .collect::<Vec<_>>();
                let compiled = system.candidates_to_css(&probes)?;
                let unknown = parts
                    .into_iter()
                    .zip(compiled)
                    .filter_map(|(variant, css)| css.is_none().then(|| format!("`{variant}`")))
                    .collect::<Vec<_>>();
                if unknown.len() == 1 {
                    return Ok(format!(
                        "Cannot apply utility class `{candidate}` because the {} variant does not exist.",
                        unknown[0]
                    ));
                }
                if unknown.len() > 1 {
                    let list = if unknown.len() == 2 {
                        unknown.join(" and ")
                    } else {
                        format!(
                            "{}, and {}",
                            unknown[..unknown.len() - 1].join(", "),
                            unknown.last().unwrap()
                        )
                    };
                    return Ok(format!(
                        "Cannot apply utility class `{candidate}` because the {list} variants do not exist."
                    ));
                }
            }
        }
        Ok(if system.theme.is_empty() {
            format!(
                "Cannot apply unknown utility class `{candidate}`. Are you using CSS modules or similar and missing `@reference`? https://tailwindcss.com/docs/functions-and-directives#reference-directive"
            )
        } else {
            format!("Cannot apply unknown utility class `{candidate}`")
        })
    }

    fn substitute(&mut self, group: GroupId) -> Result<(), String> {
        let mut index = 0;
        while index < self.arena.groups[group].len() {
            let id = self.arena.groups[group][index];
            if let AstNode::AtRule { name, params, .. } = self.arena.nodes[id].data.clone()
                && name == "@apply"
            {
                let Some(replacement) = compile_apply(
                    &params,
                    !self.arena.groups[self.arena.nodes[id].children.unwrap()].is_empty(),
                    self.system,
                )?
                else {
                    index += 1;
                    continue;
                };
                let replacement = replacement
                    .into_iter()
                    .map(|node| self.arena.import_node(node))
                    .collect::<Vec<_>>();
                self.arena.groups[group].splice(index..=index, replacement);
                continue;
            }
            if let Some(children) = self.arena.nodes[id].children {
                self.substitute(children)?;
            }
            index += 1;
        }
        Ok(())
    }
}

// Shared by the full @apply walk and the signature's isolated .x wrapper.
fn compile_apply(
    params: &str,
    has_body: bool,
    system: &mut DesignSystem,
) -> Result<Option<Vec<AstNode>>, String> {
    let mut normal = Vec::new();
    let mut dashed = Vec::new();
    let mut candidates = IndexSet::new();
    for part in words(params) {
        if part.starts_with("--") {
            dashed.push(part);
        } else {
            normal.push(part);
        }
        candidates.insert(part);
    }
    if !dashed.is_empty() {
        if normal.is_empty() {
            return Ok(None);
        }
        return Err(format!(
            "You cannot use `@apply` with both mixins and utilities. Please move `@apply {}` into a separate rule.",
            dashed.join(" ")
        ));
    }
    if has_body && !normal.is_empty() {
        return Err(format!(
            "The rule `@apply {}` must not have a body.",
            normal.join(" ")
        ));
    }
    let mut candidates = candidates.into_iter().collect::<Vec<_>>();
    let array_index = |name: &str| {
        name.parse::<u32>()
            .ok()
            .filter(|&value| value != u32::MAX && value.to_string() == name)
    };
    candidates.sort_by_key(|name| match array_index(name) {
        Some(index) => (false, index),
        None => (true, 0),
    });
    let compiled = system.compile_candidates_with(candidates, false, |system, candidate| {
        Err(Apply::invalid(system, candidate)?)
    })?;
    let mut replacement = Vec::new();
    for compiled in compiled.nodes {
        let mut node = compiled.node;
        node.fresh_declarations();
        match node {
            AstNode::Rule { nodes, .. } => replacement.extend(nodes),
            node => replacement.push(node),
        }
    }
    Ok(Some(replacement))
}

pub(crate) fn substitute_utility(
    params: &str,
    system: &mut DesignSystem,
) -> Result<Vec<AstNode>, String> {
    // collect() sees only this one @apply, with no @utility definitions. Keep
    // its dependency parsing before validation and compilation, in source order.
    for raw in words(params) {
        system.parse_candidate(raw);
    }
    let Some(replacement) = compile_apply(params, false, system)? else {
        return Ok(vec![ast::rule(
            ".x",
            vec![AstNode::AtRule {
                name: "@apply".into(),
                params: params.into(),
                nodes: Vec::new(),
            }],
        )]);
    };
    fn contains_apply(nodes: &[AstNode]) -> bool {
        nodes.iter().any(|node| match node {
            AstNode::AtRule { name, nodes, .. } => name == "@apply" || contains_apply(nodes),
            AstNode::Rule { nodes, .. }
            | AstNode::Context { nodes, .. }
            | AstNode::AtRoot { nodes } => contains_apply(nodes),
            _ => false,
        })
    }
    if !contains_apply(&replacement) {
        return Ok(vec![ast::rule(".x", replacement)]);
    }
    // Resume the original substitution walk after the outer splice. Do not
    // collect the generated nodes: the original collect() never visited them.
    let mut apply = Apply {
        arena: Arena::default(),
        system,
        parents: IndexSet::new(),
        dependencies: IndexMap::new(),
        definitions: IndexMap::new(),
        registered_utilities: VecDeque::new(),
        utility_bindings: IndexMap::new(),
        changed: false,
    };
    let root = apply.arena.import(replacement);
    apply.substitute(root)?;
    Ok(vec![ast::rule(".x", apply.arena.into_export(root))])
}

pub fn substitute(nodes: &mut Vec<AstNode>, system: &mut DesignSystem) -> Result<bool, String> {
    substitute_registered(nodes, system, Vec::new())
}

pub(crate) fn substitute_registered(
    nodes: &mut Vec<AstNode>,
    system: &mut DesignSystem,
    registered_utilities: Vec<usize>,
) -> Result<bool, String> {
    let mut apply = Apply {
        arena: Arena::default(),
        system,
        parents: IndexSet::new(),
        dependencies: IndexMap::new(),
        definitions: IndexMap::new(),
        registered_utilities: registered_utilities.into(),
        utility_bindings: IndexMap::new(),
        changed: false,
    };
    let root = apply.arena.import_node(AstNode::Rule {
        selector: "&".into(),
        nodes: std::mem::take(nodes),
    });
    let group = apply.arena.group(vec![root]);
    let result = (|| {
        apply.collect(group, &mut Vec::new())?;
        let mut sorted = Vec::new();
        let mut seen = IndexSet::new();
        let mut wip = IndexSet::new();
        for parent in apply.parents.clone() {
            apply.visit(parent, &mut Vec::new(), &mut seen, &mut wip, &mut sorted)?;
        }
        for parent in sorted {
            if let Some(children) = apply.arena.nodes[parent].children {
                let result = apply.substitute(children);
                if let Some(&index) = apply.utility_bindings.get(&parent) {
                    apply.system.custom_utilities[index].nodes = apply.arena.export(children);
                }
                result?;
            }
        }
        Ok(apply.changed)
    })();
    let output = apply.arena.nodes[root].children.unwrap();
    *nodes = apply.arena.into_export(output);
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{syntax::css, theme::Theme};
    use serde::Deserialize;

    #[test]
    fn isolated_apply_keeps_nested_generated_directives_and_error_cache_effects() {
        fn system() -> DesignSystem {
            let mut theme = Theme::default();
            theme
                .add("--color-brand", "#ABCDEF", crate::theme::INLINE)
                .unwrap();
            let mut system = DesignSystem::new(theme);
            for (name, body) in [
                ("nested", "@apply grid;"),
                (
                    "generated-keyframes",
                    "@keyframes a { from { @apply flex; } }",
                ),
                ("generated-utility", "@utility later { @apply grid; }"),
                ("generated-mixin", "@apply --token;"),
                ("bad", "@apply does-not-exist;"),
            ] {
                system
                    .custom_utility(name, &css::parse(body).unwrap())
                    .unwrap();
            }
            system.custom_variant(
                "inject",
                &css::parse("@slot; @keyframes b { from { @apply grid; } }").unwrap(),
            );
            system
        }
        let mut generic = system();
        let mut isolated = system();
        for params in [
            "nested",
            "generated-keyframes",
            "generated-utility",
            "inject:flex",
            "generated-mixin",
            "--token",
            "--token flex",
            "grid grid flex",
            "flex\u{a0}grid",
            "2 1 flex",
            "bg-brand does-not-exist",
            "does-not-exist bg-brand",
            "bad",
            "unknown:bg-brand",
        ] {
            let mut expected = vec![ast::rule(
                ".x",
                vec![AstNode::AtRule {
                    name: "@apply".into(),
                    params: params.into(),
                    nodes: Vec::new(),
                }],
            )];
            let expected_result = substitute(&mut expected, &mut generic);
            let actual = substitute_utility(params, &mut isolated);
            assert_eq!(
                actual.as_ref().err(),
                expected_result.as_ref().err(),
                "{params}"
            );
            if let Ok(actual) = actual {
                assert_eq!(actual, expected, "{params}");
            }
            // Error diagnostics may compile candidates reentrantly. Subsequent
            // CSS must observe exactly the same cache state on both paths.
            let probes = ["bg-brand".into(), "hover:bg-brand".into(), "grid".into()];
            assert_eq!(
                isolated.candidates_to_css(&probes),
                generic.candidates_to_css(&probes),
                "after {params}"
            );
        }
    }

    #[derive(Deserialize)]
    struct Fixture {
        scenarios: Vec<Scenario>,
    }
    #[derive(Deserialize)]
    struct Scenario {
        input: String,
        probes: Vec<String>,
        expected: Expected,
    }
    #[derive(Deserialize)]
    struct Expected {
        nodes: Vec<AstNode>,
        error: Option<String>,
        css: Vec<Option<String>>,
    }

    fn register(nodes: &mut [AstNode], system: &mut DesignSystem, bindings: &mut Vec<usize>) {
        for node in nodes {
            if let AstNode::AtRule {
                name,
                params,
                nodes,
            } = node
                && name == "@utility"
            {
                bindings.push(system.register_css_utility(params, nodes).unwrap());
            }
            if let Some(children) = node.children_mut() {
                register(children, system, bindings);
            }
        }
    }

    #[test]
    fn registered_css_utilities_observe_apply_expansion_in_dependency_order_and_on_error() {
        let fixture: Fixture =
            serde_json::from_str(include_str!("../fixtures/custom-apply.json")).unwrap();
        for (index, scenario) in fixture.scenarios.into_iter().enumerate() {
            let mut system = DesignSystem::new(Theme::default());
            let mut nodes = css::parse(&scenario.input).unwrap();
            let mut bindings = Vec::new();
            register(&mut nodes, &mut system, &mut bindings);
            let result = substitute_registered(&mut nodes, &mut system, bindings);
            assert_eq!(result.err(), scenario.expected.error, "case {index}");
            assert_eq!(nodes, scenario.expected.nodes, "case {index}");
            assert_eq!(
                system.candidates_to_css(&scenario.probes).unwrap(),
                scenario.expected.css,
                "case {index}"
            );
        }
    }
}
