// Port of Tailwind CSS 4.3.4 createCssUtility and resolveValueFunction.
// See LICENSE.tailwind.
use crate::{
    ast::AstNode,
    candidate::{Candidate, CandidateKind, Kind, Modifier, UtilityValue},
    completions::{self, Group},
    data_type::{self, DataType},
    numeric,
    syntax::{
        escape::unescape,
        is_js_whitespace,
        segment::segment,
        value::{self, Value},
    },
    theme::Theme,
    utilities,
};
use indexmap::IndexSet;
use regex::Regex;
use std::{
    collections::{HashMap, HashSet},
    sync::{LazyLock, OnceLock},
};

#[derive(Clone, Default)]
struct Suggestions {
    spacing_number: bool,
    spacing_integer: bool,
    literals: IndexSet<String>,
    theme_keys: IndexSet<String>,
}

#[derive(Clone)]
pub(crate) struct CssUtility {
    pub root: String,
    pub kind: Kind,
    pub nodes: Vec<AstNode>,
    suggestions: [Suggestions; 2],
    prepared_values: OnceLock<HashMap<String, PreparedValue>>,
}

#[derive(Clone)]
enum PreparedValue {
    Constant(String),
    Dynamic(Vec<Value>),
}

impl PreparedValue {
    fn new(text: &str) -> Self {
        fn has_resolution(nodes: &[Value]) -> bool {
            nodes.iter().any(|node| {
                matches!(node, Value::Function { value, nodes }
                    if function_index(value).is_some() || has_resolution(nodes))
            })
        }
        let nodes = value::parse(text);
        if has_resolution(&nodes) {
            Self::Dynamic(nodes)
        } else {
            Self::Constant(value::to_css(&nodes))
        }
    }
}

fn prepare_values(nodes: &[AstNode], parent: bool, values: &mut HashMap<String, PreparedValue>) {
    for node in nodes {
        if parent
            && let AstNode::Declaration {
                value: Some(text), ..
            } = node
            && !text.is_empty()
        {
            values
                .entry(text.clone())
                .or_insert_with(|| PreparedValue::new(text));
        }
        if let AstNode::Rule {
            nodes: children, ..
        }
        | AstNode::AtRule {
            nodes: children, ..
        }
        | AstNode::Context {
            nodes: children, ..
        }
        | AstNode::AtRoot { nodes: children } = node
        {
            prepare_values(
                children,
                matches!(node, AstNode::Rule { .. } | AstNode::AtRule { .. }),
                values,
            );
        }
    }
}

pub fn valid_functional_name(name: &str) -> bool {
    name.strip_suffix("-*")
        .is_some_and(|root| root_end(root) == Some(root.len()))
}

fn root_end(name: &str) -> Option<usize> {
    let bytes = name.as_bytes();
    let start = usize::from(bytes.first() == Some(&b'-'));
    if !bytes.get(start).is_some_and(u8::is_ascii_lowercase) {
        return None;
    }
    Some(
        bytes
            .iter()
            .enumerate()
            .skip(start + 1)
            .find(|(_, b)| !b.is_ascii_alphanumeric() && !matches!(b, b'_' | b'-'))
            .map_or(bytes.len(), |(i, _)| i),
    )
}

pub fn valid_static_name(name: &str) -> bool {
    let Some(end) = root_end(name) else {
        return false;
    };
    if end == name.len() {
        return !name.ends_with('-');
    }
    let bytes = name.as_bytes();
    let mut slash = false;
    for i in end..bytes.len() {
        match bytes[i] {
            b'%' if i + 1 == bytes.len() && bytes[i - 1].is_ascii_digit() => {}
            b'/' if !slash && i + 1 < bytes.len() => slash = true,
            b'.' if bytes[i - 1].is_ascii_digit()
                && bytes.get(i + 1).is_some_and(u8::is_ascii_digit) => {}
            b'_' | b'-' | b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' => {}
            _ => return false,
        }
    }
    true
}

impl CssUtility {
    pub fn new(name: &str, mut nodes: Vec<AstNode>) -> Option<Self> {
        let name = unescape(name);
        let kind = if valid_functional_name(&name) {
            Kind::Functional
        } else if valid_static_name(&name) {
            Kind::Static
        } else {
            return None;
        };
        let root = if kind == Kind::Functional {
            name[..name.len() - 2].into()
        } else {
            name
        };
        let mut suggestions = Default::default();
        if kind == Kind::Functional {
            preprocess(&mut nodes, &mut suggestions);
        }
        Some(Self {
            root,
            kind,
            nodes,
            suggestions,
            prepared_values: OnceLock::new(),
        })
    }

    pub fn compile(&self, candidate: &Candidate, theme: &Theme) -> Option<Vec<AstNode>> {
        let matches = match &candidate.kind {
            CandidateKind::Static { root } => self.kind == Kind::Static && root == &self.root,
            CandidateKind::Functional { root, .. } => {
                self.kind == Kind::Functional && root == &self.root
            }
            _ => false,
        };
        if !matches {
            return None;
        }
        let mut nodes = self.nodes.clone();
        for node in &mut nodes {
            node.fresh_declarations();
        }
        match &candidate.kind {
            CandidateKind::Static { root } if self.kind == Kind::Static && root == &self.root => {
                Some(nodes)
            }
            CandidateKind::Functional {
                root,
                value,
                modifier,
            } if self.kind == Kind::Functional && root == &self.root => {
                let mut state = Resolution::default();
                let input = value.as_ref().map(Input::utility);
                let modifier_input = modifier.as_ref().map(Input::modifier);
                let prepared = self.prepared_values.get_or_init(|| {
                    let mut values = HashMap::new();
                    prepare_values(&self.nodes, true, &mut values);
                    values
                });
                resolve_declarations(
                    &mut nodes,
                    true,
                    &mut state,
                    input,
                    modifier_input,
                    theme,
                    prepared,
                );
                if !state.used_value
                    || !state.resolved_value
                    || (state.used_modifier && !state.resolved_modifier && modifier.is_some())
                    || (state.ratio && state.resolved_modifier)
                    || (modifier.is_some() && !state.ratio && !state.resolved_modifier)
                {
                    return None;
                }
                if state.ratio {
                    remove_non_ratios(&mut nodes, &state.non_ratios);
                }
                Some(nodes)
            }
            _ => None,
        }
    }

    pub fn suggestions(&self, theme: &Theme) -> Option<Group> {
        if self.kind != Kind::Functional {
            return None;
        }
        let mut lists: [Vec<String>; 2] = Default::default();
        for (storage, values) in self.suggestions.iter().zip(&mut lists) {
            values.extend(storage.literals.iter().cloned());
            if storage.spacing_number || storage.spacing_integer {
                values.extend(
                    completions::SPACING_VALUES
                        .iter()
                        .filter(|value| {
                            storage.spacing_number || numeric::is_positive_integer(value)
                        })
                        .map(|v| (*v).into()),
                );
            }
            values.extend(completions::resolve(
                theme,
                &storage.theme_keys.iter().cloned().collect::<Vec<_>>(),
            ));
        }
        let [values, modifiers] = lists;
        Some(Group {
            supports_negative: None,
            values: values.into_iter().map(Some).collect(),
            modifiers,
        })
    }
}

fn preprocess(nodes: &mut [AstNode], storage: &mut [Suggestions; 2]) {
    for node in nodes {
        if let AstNode::Declaration {
            value: Some(text), ..
        } = node
            && (text.contains("--value(") || text.contains("--modifier("))
        {
            let mut ast = value::parse(text);
            normalize_functions(&mut ast, storage);
            *text = value::to_css(&ast);
        }
        if let Some(children) = node.children_mut() {
            preprocess(children, storage);
        }
    }
}

fn function_index(name: &str) -> Option<usize> {
    match name {
        "--value" => Some(0),
        "--modifier" => Some(1),
        _ => None,
    }
}

fn spacing_suggestions(nodes: &[Value], storage: &mut [Suggestions; 2]) {
    for node in nodes {
        if let Value::Function { value, nodes } = node {
            if let Some(index) = function_index(value) {
                for arg in nodes {
                    if let Value::Word { value } = arg {
                        storage[index].spacing_number |= value == "number";
                        storage[index].spacing_integer |= value == "integer";
                    }
                }
            }
            spacing_suggestions(nodes, storage);
        }
    }
}

fn normalize_functions(nodes: &mut [Value], storage: &mut [Suggestions; 2]) {
    static NESTED: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"--([^\n\r\u{2028}\u{2029}]*?)[\t\n\x0b\x0c\r \u{00a0}\u{1680}\u{2000}-\u{200a}\u{2028}\u{2029}\u{202f}\u{205f}\u{3000}\u{feff}]--(.*?)").unwrap()
    });
    static STARS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(-\*){2,}").unwrap());
    for node in nodes {
        let Value::Function { value: name, nodes } = node else {
            continue;
        };
        if name == "--spacing" && !(storage[0].spacing_number && storage[1].spacing_number) {
            spacing_suggestions(nodes, storage);
        }
        if let Some(index) = function_index(name) {
            let source = value::to_css(nodes);
            let args: Vec<_> = segment(&source, b',')
                .into_iter()
                .map(|arg| {
                    let arg = arg.trim_matches(is_js_whitespace);
                    if arg.starts_with("--default(") {
                        return arg.to_owned();
                    }
                    let arg = arg.replace(r"\*", "*");
                    let arg = NESTED.replace_all(&arg, "--${1}-*--${2}");
                    let arg: String = arg.chars().filter(|c| !is_js_whitespace(*c)).collect();
                    let mut arg = STARS.replace_all(&arg, "-*").into_owned();
                    if arg.starts_with("--") && !arg.contains('(') && !arg.contains("-*") {
                        arg.push_str("-*");
                    }
                    arg
                })
                .collect();
            *nodes = value::parse(&args.join(","));
            for arg in nodes.iter() {
                if let Value::Word { value } = arg {
                    if value.len() >= 2
                        && matches!(value.as_bytes()[0], b'\'' | b'"')
                        && value.as_bytes().first() == value.as_bytes().last()
                    {
                        storage[index]
                            .literals
                            .insert(value[1..value.len() - 1].into());
                    } else if value.starts_with("--") {
                        storage[index].theme_keys.insert(
                            value
                                .split_once("-*")
                                .map_or(value.as_str(), |(prefix, _)| prefix)
                                .into(),
                        );
                    }
                }
            }
        }
        normalize_functions(nodes, storage);
    }
}

#[derive(Clone, Copy)]
struct Input<'a> {
    value: &'a str,
    arbitrary: bool,
    data_type: Option<&'a str>,
    fraction: Option<Option<&'a str>>,
}

impl<'a> Input<'a> {
    fn utility(value: &'a UtilityValue) -> Self {
        match value {
            UtilityValue::Named { value, fraction } => Self {
                value,
                arbitrary: false,
                data_type: None,
                fraction: Some(fraction.as_deref()),
            },
            UtilityValue::Arbitrary { value, data_type } => Self {
                value,
                arbitrary: true,
                data_type: data_type.as_deref(),
                fraction: None,
            },
        }
    }
    fn modifier(value: &'a Modifier) -> Self {
        match value {
            Modifier::Named { value } => Self {
                value,
                arbitrary: false,
                data_type: None,
                fraction: None,
            },
            Modifier::Arbitrary { value } => Self {
                value,
                arbitrary: true,
                data_type: None,
                fraction: None,
            },
        }
    }
}

fn data_type(name: &str) -> Option<DataType> {
    serde_json::from_value(serde_json::Value::String(name.into())).ok()
}

fn resolve(input: Option<Input<'_>>, args: &[Value], theme: &Theme) -> Option<(Vec<Value>, bool)> {
    let Some(input) = input else {
        return args.iter().find_map(|arg| match arg {
            Value::Function { value, nodes } if value == "--default" => {
                Some((nodes.clone(), false))
            }
            _ => None,
        });
    };
    for arg in args {
        let Value::Word { value: arg } = arg else {
            continue;
        };
        if !input.arbitrary {
            if arg.len() >= 2
                && matches!(arg.as_bytes()[0], b'\'' | b'"')
                && arg.as_bytes().first() == arg.as_bytes().last()
                && arg[1..arg.len() - 1] == *input.value
            {
                return Some((value::parse(input.value), false));
            } else if arg.starts_with("--") {
                let resolved = if let Some(namespace) = arg.strip_suffix("-*") {
                    theme.resolve(Some(input.value), &[namespace.into()], 0)
                } else {
                    let mut keys = arg.split("-*");
                    let namespace = keys.next().unwrap();
                    let nested: Vec<String> = keys.map(String::from).collect();
                    nested.last().and_then(|last| {
                        theme
                            .resolve_with(input.value, &[namespace.into()], &nested)
                            .and_then(|(_, mut extra)| extra.shift_remove(last))
                    })
                };
                if let Some(resolved) = resolved.filter(|value| !value.is_empty()) {
                    return Some((value::parse(&resolved), false));
                }
            } else if matches!(arg.as_str(), "number" | "integer" | "ratio" | "percentage") {
                let resolved = if arg == "ratio" {
                    input.fraction.unwrap_or(Some(input.value))
                } else {
                    Some(input.value)
                };
                let Some(resolved) = resolved.filter(|v| !v.is_empty()) else {
                    continue;
                };
                let kind = data_type(arg)?;
                if data_type::infer(resolved, &[kind]).is_none() {
                    continue;
                }
                if kind == DataType::Ratio {
                    let parts = segment(resolved, b'/');
                    if !parts.iter().take(2).all(|part| {
                        let n = utilities::number(part);
                        n.is_finite() && n >= 0.0 && n.fract() == 0.0
                    }) {
                        continue;
                    }
                    return Some((
                        value::parse(&format!(
                            "{} / {}",
                            parts[0].trim_matches(is_js_whitespace),
                            parts[1].trim_matches(is_js_whitespace)
                        )),
                        true,
                    ));
                }
                if kind == DataType::Number && !numeric::is_spacing_multiplier(resolved) {
                    continue;
                }
                if kind == DataType::Percentage
                    && !numeric::is_positive_integer(&resolved[..resolved.len() - 1])
                {
                    continue;
                }
                return Some((value::parse(resolved), false));
            }
        } else if let Some(kind) = arg.strip_prefix('[').and_then(|v| v.strip_suffix(']')) {
            if kind == "*" {
                return Some((value::parse(input.value), false));
            }
            if let Some(hint) = input.data_type.filter(|hint| !hint.is_empty()) {
                if hint == kind {
                    return Some((value::parse(input.value), false));
                }
            } else if data_type(kind)
                .is_some_and(|kind| data_type::infer(input.value, &[kind]).is_some())
            {
                return Some((value::parse(input.value), false));
            }
        }
    }
    None
}

#[derive(Default)]
struct Resolution {
    used_value: bool,
    resolved_value: bool,
    used_modifier: bool,
    resolved_modifier: bool,
    ratio: bool,
    non_ratios: HashSet<usize>,
}

fn resolve_functions(
    nodes: &mut Vec<Value>,
    state: &mut Resolution,
    input: Option<Input<'_>>,
    modifier: Option<Input<'_>>,
    theme: &Theme,
    identity: usize,
) -> bool {
    let mut index = 0;
    while index < nodes.len() {
        if let Value::Function {
            value: name,
            nodes: args,
        } = &mut nodes[index]
        {
            if let Some(which) = function_index(name) {
                if which == 0 {
                    state.used_value = true;
                } else {
                    state.used_modifier = true;
                }
                let Some((replacement, ratio)) =
                    resolve(if which == 0 { input } else { modifier }, args, theme)
                else {
                    return false;
                };
                if which == 0 {
                    state.resolved_value = true;
                    if ratio {
                        state.ratio = true;
                    } else {
                        state.non_ratios.insert(identity);
                    }
                } else {
                    state.resolved_modifier = true;
                }
                let len = replacement.len();
                nodes.splice(index..=index, replacement);
                index += len;
                continue;
            }
            if !resolve_functions(args, state, input, modifier, theme, identity) {
                return false;
            }
        }
        index += 1;
    }
    true
}

fn resolve_declarations(
    nodes: &mut Vec<AstNode>,
    valid_parent: bool,
    state: &mut Resolution,
    input: Option<Input<'_>>,
    modifier: Option<Input<'_>>,
    theme: &Theme,
    prepared: &HashMap<String, PreparedValue>,
) {
    nodes.retain_mut(|node| {
        if valid_parent
            && let AstNode::Declaration {
                identity,
                value: Some(text),
                ..
            } = node
            && !text.is_empty()
        {
            let mut ast = match prepared.get(text.as_str()) {
                Some(PreparedValue::Constant(normalized)) => {
                    *text = normalized.clone();
                    return true;
                }
                Some(PreparedValue::Dynamic(nodes)) => nodes.clone(),
                None => value::parse(text),
            };
            if !resolve_functions(&mut ast, state, input, modifier, theme, identity.0) {
                return false;
            }
            *text = value::to_css(&ast);
        }
        let parent = matches!(node, AstNode::Rule { .. } | AstNode::AtRule { .. });
        if let Some(children) = node.children_mut() {
            resolve_declarations(children, parent, state, input, modifier, theme, prepared);
        }
        true
    });
}

fn remove_non_ratios(nodes: &mut Vec<AstNode>, ids: &HashSet<usize>) {
    nodes.retain_mut(|node| {
        if let AstNode::Declaration { identity, .. } = node
            && ids.contains(&identity.0)
        {
            return false;
        }
        if let Some(children) = node.children_mut() {
            remove_non_ratios(children, ids);
        }
        true
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{candidate, syntax::css};

    #[test]
    fn prepared_values_survive_theme_changes_and_replaced_utility_bodies() {
        let initial =
            "width: calc(--value(number) * 1px); color: --value(--color-*); content: 'constant';";
        let mut utility = CssUtility::new("probe-*", css::parse(initial).unwrap()).unwrap();
        let mut registry = candidate::Registry::default();
        registry
            .utilities
            .insert("probe".into(), [Kind::Functional].into());
        let mut theme = Theme::default();
        for phase in 0..4 {
            theme
                .add(
                    "--color-brand",
                    if phase < 2 { "red" } else { "blue" },
                    if phase % 2 == 0 {
                        0
                    } else {
                        crate::theme::INLINE
                    },
                )
                .unwrap();
            if phase == 1 {
                utility.nodes = css::parse("height: --value(number); aspect-ratio: --value(ratio); opacity: --modifier(number); &:hover { color: --value(--color-*); }").unwrap();
                utility.nodes.push(AstNode::Declaration {
                    identity: Default::default(),
                    property: "content".into(),
                    value: Some("unclosed(\\".into()),
                    important: false,
                });
            } else if phase == 2 {
                utility.nodes = css::parse(initial).unwrap();
            }
            // An empty prepared map exercises the original parse/resolve/print
            // path on every declaration, including texts introduced by @apply.
            let reference = CssUtility {
                prepared_values: OnceLock::from(HashMap::new()),
                ..utility.clone()
            };
            for raw in [
                "probe-2",
                "probe-3",
                "probe-brand",
                "probe-2/3",
                "probe-[5px]",
                "probe-2/[50%]",
                "probe-2/25",
                "probe-unknown",
            ] {
                let candidates = candidate::parse(raw, &registry);
                assert!(!candidates.is_empty(), "{raw}");
                for candidate in candidates {
                    assert_eq!(
                        utility.compile(&candidate, &theme),
                        reference.compile(&candidate, &theme),
                        "phase {phase}: {raw}"
                    );
                }
            }
        }
    }
}
