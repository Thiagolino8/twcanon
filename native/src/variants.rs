// Port of Tailwind CSS 4.3.4 variants.ts and compile.ts::applyVariant.
// Static templates are captured from the pinned source during development.
use crate::{
    ast::{AstNode, ContextValue, rule},
    breakpoints::{self, Direction},
    candidate::{Kind, Modifier, Registry, Variant, VariantDefinition, VariantValue},
    numeric,
    syntax::{attribute, is_js_whitespace, segment::segment, value},
    theme::Theme,
};
use indexmap::IndexMap;
use serde::Deserialize;
use std::{cmp::Ordering, collections::BTreeMap, sync::LazyLock};

#[derive(Deserialize)]
struct Builtins {
    operations: Vec<Operation>,
}

#[derive(Deserialize)]
#[serde(tag = "op", rename_all = "kebab-case")]
enum Operation {
    Register {
        root: String,
        kind: Kind,
        compounds: u8,
        compounds_with: u8,
        nodes: Option<Vec<AstNode>>,
    },
    GroupStart {
        compare: String,
    },
    GroupEnd,
    Breakpoints,
}

static BUILTINS: LazyLock<Builtins> = LazyLock::new(|| {
    let bytes = miniz_oxide::inflate::decompress_to_vec(include_bytes!(concat!(
        env!("OUT_DIR"),
        "/variants.json.deflate"
    )))
    .expect("falha ao descomprimir as definições de variantes");
    serde_json::from_slice(&bytes).expect("pinned variant definitions must be valid")
});

#[derive(Clone, Debug)]
enum Handler {
    Template(Vec<AstNode>),
    Builtin(String),
    Breakpoint(String),
    Selectors {
        styles: Vec<String>,
        at_rules: Vec<String>,
        uses_at_scope: bool,
    },
    Custom {
        nodes: Vec<AstNode>,
        uses_at_variant: bool,
        uses_at_scope: bool,
    },
}

#[derive(Clone, Debug)]
struct Definition {
    metadata: VariantDefinition,
    handler: Handler,
}

#[derive(Clone, Debug)]
pub struct Variants {
    theme: Theme,
    definitions: IndexMap<String, Definition>,
    comparators: BTreeMap<usize, (bool, Direction)>,
    last_order: usize,
    group_order: Option<usize>,
}

impl Variants {
    pub fn new(theme: &Theme) -> Self {
        let mut variants = Self {
            theme: theme.clone(),
            definitions: IndexMap::new(),
            comparators: BTreeMap::new(),
            last_order: 0,
            group_order: None,
        };
        for operation in &BUILTINS.operations {
            match operation {
                Operation::Register {
                    root,
                    kind,
                    compounds,
                    compounds_with,
                    nodes,
                } => variants.register(
                    root,
                    *kind,
                    *compounds,
                    *compounds_with,
                    nodes.as_ref().map_or_else(
                        || Handler::Builtin(root.clone()),
                        |nodes| Handler::Template(nodes.clone()),
                    ),
                ),
                Operation::GroupStart { compare } => {
                    let order = variants.next_order();
                    variants.group_order = Some(order);
                    variants.comparators.insert(
                        order,
                        (
                            compare.starts_with("container"),
                            if compare.ends_with("desc") {
                                Direction::Desc
                            } else {
                                Direction::Asc
                            },
                        ),
                    );
                }
                Operation::GroupEnd => variants.group_order = None,
                Operation::Breakpoints => {
                    for (key, value) in theme.namespace("--breakpoint") {
                        if let Some(key) = key {
                            variants.register(&key, Kind::Static, 1, 0, Handler::Breakpoint(value));
                        }
                    }
                }
            }
        }
        variants
    }

    fn next_order(&self) -> usize {
        self.group_order.unwrap_or(self.last_order + 1)
    }

    fn register(
        &mut self,
        root: &str,
        kind: Kind,
        compounds: u8,
        compounds_with: u8,
        handler: Handler,
    ) {
        if let Some(existing) = self.definitions.get_mut(root) {
            existing.metadata.kind = kind;
            existing.metadata.compounds = compounds;
            existing.handler = handler;
        } else {
            self.last_order = self.next_order();
            self.definitions.insert(
                root.into(),
                Definition {
                    metadata: VariantDefinition {
                        kind,
                        order: self.last_order,
                        compounds,
                        compounds_with,
                    },
                    handler,
                },
            );
        }
    }

    pub fn registry(&self) -> BTreeMap<String, VariantDefinition> {
        self.definitions
            .iter()
            .map(|(root, definition)| (root.clone(), definition.metadata.clone()))
            .collect()
    }

    pub fn extend_registry(&self, registry: &mut Registry) {
        registry.variants = self.registry();
    }

    pub fn entries(&self) -> impl Iterator<Item = (&str, &VariantDefinition)> {
        self.definitions
            .iter()
            .map(|(root, definition)| (root.as_str(), &definition.metadata))
    }

    pub fn compounds_with(&self, parent: &str, child: &str) -> bool {
        let (Some(parent), Some(child)) =
            (self.definitions.get(parent), self.definitions.get(child))
        else {
            return false;
        };
        parent.metadata.kind == Kind::Compound
            && parent.metadata.compounds_with & child.metadata.compounds != 0
    }

    pub fn completions(&self, root: &str) -> Vec<String> {
        match root {
            "not" | "group" | "peer" | "in" | "has" => self
                .definitions
                .keys()
                .filter(|child| self.compounds_with(root, child))
                .cloned()
                .collect(),
            "aria" => [
                "busy", "checked", "disabled", "expanded", "hidden", "pressed", "readonly",
                "required", "selected",
            ]
            .into_iter()
            .map(str::to_owned)
            .collect(),
            "min" | "max" => self
                .theme
                .namespace("--breakpoint")
                .into_iter()
                .filter_map(|(key, _)| key)
                .collect(),
            "@" | "@min" | "@max" => self
                .theme
                .namespace("--container")
                .into_iter()
                .filter_map(|(key, _)| key)
                .collect(),
            _ => Vec::new(),
        }
    }

    pub fn from_ast(&mut self, name: &str, nodes: &[AstNode]) {
        fn inspect(
            nodes: &[AstNode],
            selectors: &mut Vec<String>,
            uses_at_variant: &mut bool,
            uses_at_scope: &mut bool,
        ) {
            for node in nodes {
                match node {
                    AstNode::Rule { selector, .. } => selectors.push(selector.clone()),
                    AstNode::AtRule { name, .. } if name == "@variant" => *uses_at_variant = true,
                    AstNode::AtRule { name, params, .. } if name != "@slot" => {
                        *uses_at_scope |= name == "@scope";
                        selectors.push(format!("{name} {params}"));
                    }
                    _ => {}
                }
                match node {
                    AstNode::Rule { nodes, .. }
                    | AstNode::AtRule { nodes, .. }
                    | AstNode::AtRoot { nodes }
                    | AstNode::Context { nodes, .. } => {
                        inspect(nodes, selectors, uses_at_variant, uses_at_scope)
                    }
                    _ => {}
                }
            }
        }
        let mut selectors = vec![];
        let (mut uses_at_variant, mut uses_at_scope) = (false, false);
        inspect(
            nodes,
            &mut selectors,
            &mut uses_at_variant,
            &mut uses_at_scope,
        );
        self.register(
            name,
            Kind::Static,
            compounds_for_selectors(selectors.iter().map(String::as_str)),
            0,
            Handler::Custom {
                nodes: nodes.to_vec(),
                uses_at_variant,
                uses_at_scope,
            },
        );
    }

    pub(crate) fn reserve(&mut self, name: &str) {
        self.register(
            name,
            Kind::Static,
            2,
            0,
            Handler::Selectors {
                styles: Vec::new(),
                at_rules: Vec::new(),
                uses_at_scope: false,
            },
        );
    }

    pub(crate) fn register_selectors(&mut self, name: &str, selectors: &[String]) {
        let (at_rules, styles): (Vec<_>, Vec<_>) = selectors
            .iter()
            .cloned()
            .partition(|selector| selector.starts_with('@'));
        let uses_at_scope = at_rules
            .iter()
            .any(|selector| selector.starts_with("@scope"));
        self.register(
            name,
            Kind::Static,
            compounds_for_selectors(styles.iter().chain(at_rules.iter()).map(String::as_str)),
            0,
            Handler::Selectors {
                styles,
                at_rules,
                uses_at_scope,
            },
        );
    }

    pub fn substitute_at_variant(&self, ast: &mut Vec<AstNode>) -> Result<bool, String> {
        let registry = Registry {
            variants: self.registry(),
            ..Registry::default()
        };
        self.substitute_at_variant_with(ast, &mut |name| {
            crate::candidate::parse_variant(name, &registry)
        })
    }

    pub fn substitute_at_variant_with(
        &self,
        ast: &mut Vec<AstNode>,
        parse_variant: &mut impl FnMut(&str) -> Option<Variant>,
    ) -> Result<bool, String> {
        let mut changed = false;
        let mut index = 0;
        while index < ast.len() {
            if let AstNode::AtRule {
                name,
                params,
                nodes,
            } = &ast[index]
                && name == "@variant"
            {
                let mut replacement = vec![];
                let compounds = segment(params, b',');
                for (index, compound) in compounds.iter().enumerate() {
                    let mut body = nodes.clone();
                    if index + 1 < compounds.len() {
                        for node in &mut body {
                            node.fresh_declarations();
                        }
                    }
                    let mut node = rule("&", body);
                    for name in segment(compound, b':').iter().rev() {
                        let name = name.trim_matches(is_js_whitespace);
                        if name.is_empty() {
                            return Err("Cannot use `@variant` with empty variant".into());
                        }
                        let variant = parse_variant(name).ok_or_else(|| {
                            format!("Cannot use `@variant` with unknown variant: {name}")
                        })?;
                        if !self.apply_with(&mut node, &variant, parse_variant)? {
                            return Err(format!("Cannot use `@variant` with variant: {name}"));
                        }
                    }
                    match node {
                        AstNode::Rule { selector, nodes } if selector == "&" => {
                            replacement.extend(nodes)
                        }
                        other => replacement.push(other),
                    }
                }
                ast.splice(index..=index, replacement);
                changed = true;
                continue;
            }
            if let Some(nodes) = ast[index].children_mut() {
                changed |= self.substitute_at_variant_with(nodes, parse_variant)?;
            }
            index += 1;
        }
        Ok(changed)
    }

    /// Compare canonical parsed variants. Identical ASTs are interned by the
    /// caller, as in the upstream design-system parse cache.
    pub fn compare(&self, left: Option<&Variant>, right: Option<&Variant>) -> Ordering {
        self.compare_internal(left, right, None)
    }

    /// Preserve upstream parse-cache identity when distinct raw spellings decode
    /// to equal ASTs (for example arbitrary values containing spaces).
    pub fn compare_parsed(
        &self,
        left: Option<(&str, &Variant)>,
        right: Option<(&str, &Variant)>,
    ) -> Ordering {
        self.compare_internal(
            left.map(|(_, ast)| ast),
            right.map(|(_, ast)| ast),
            left.zip(right).map(|((left, _), (right, _))| (left, right)),
        )
    }

    pub(crate) fn compare_cached(
        &self,
        cache: &crate::variant_cache::VariantCache,
        left: Option<crate::variant_cache::Identity>,
        right: Option<crate::variant_cache::Identity>,
    ) -> Result<Ordering, String> {
        if left == right {
            return Ok(Ordering::Equal);
        }
        let (left_id, right_id) = match (left, right) {
            (None, _) => return Ok(Ordering::Less),
            (_, None) => return Ok(Ordering::Greater),
            (Some(left), Some(right)) => (left, right),
        };
        let left = cache.value(left_id);
        let right = cache.value(right_id);
        if let (Some(a_root), Some(z_root)) = (root(left), root(right)) {
            let a_order = self.definitions[a_root].metadata.order;
            let z_order = self.definitions[z_root].metadata.order;
            if a_order == z_order {
                if let (
                    Variant::Compound {
                        modifier: a_modifier,
                        ..
                    },
                    Variant::Compound {
                        modifier: z_modifier,
                        ..
                    },
                ) = (left, right)
                {
                    return Ok(self
                        .compare_cached(cache, cache.child(left_id), cache.child(right_id))?
                        .then_with(|| match (a_modifier, z_modifier) {
                            (Some(a), Some(z)) => less_or_greater(text(a), text(z)),
                            (Some(_), None) => Ordering::Greater,
                            (None, Some(_)) => Ordering::Less,
                            (None, None) => Ordering::Equal,
                        }));
                }
                if a_root == z_root && !self.comparators.contains_key(&a_order) {
                    if matches!(left, Variant::Functional { value: None, .. }) {
                        return Ok(Ordering::Less);
                    }
                    if matches!(right, Variant::Functional { value: None, .. }) {
                        return Ok(Ordering::Greater);
                    }
                    // Static/compound objects have an absent `value`, rather than null.
                    if !matches!(left, Variant::Functional { .. })
                        || !matches!(right, Variant::Functional { .. })
                    {
                        return Err("Cannot read properties of undefined (reading 'kind')".into());
                    }
                }
            }
        }
        Ok(self.compare_internal(Some(left), Some(right), Some(("left", "right"))))
    }

    fn compare_internal(
        &self,
        left: Option<&Variant>,
        right: Option<&Variant>,
        names: Option<(&str, &str)>,
    ) -> Ordering {
        if names.map_or(left == right, |(left, right)| left == right) {
            return Ordering::Equal;
        }
        let (left, right) = match (left, right) {
            (None, _) => return Ordering::Less,
            (_, None) => return Ordering::Greater,
            (Some(left), Some(right)) => (left, right),
        };
        match (left, right) {
            (Variant::Arbitrary { selector: a, .. }, Variant::Arbitrary { selector: z, .. }) => {
                return less_or_greater(a, z);
            }
            (Variant::Arbitrary { .. }, _) => return Ordering::Greater,
            (_, Variant::Arbitrary { .. }) => return Ordering::Less,
            _ => {}
        }
        let a_root = root(left).unwrap();
        let z_root = root(right).unwrap();
        let a_order = self.definitions[a_root].metadata.order;
        let z_order = self.definitions[z_root].metadata.order;
        let order = a_order.cmp(&z_order);
        if order != Ordering::Equal {
            return order;
        }
        if let (
            Variant::Compound {
                variant: a,
                modifier: a_modifier,
                ..
            },
            Variant::Compound {
                variant: z,
                modifier: z_modifier,
                ..
            },
        ) = (left, right)
        {
            let child_names = names.map(|(left, right)| {
                let left = if matches!(a_root, "not" | "has" | "in") {
                    left
                } else {
                    segment(left, b'/')[0]
                };
                let right = if matches!(z_root, "not" | "has" | "in") {
                    right
                } else {
                    segment(right, b'/')[0]
                };
                (&left[a_root.len() + 1..], &right[z_root.len() + 1..])
            });
            return self
                .compare_internal(Some(a), Some(z), child_names)
                .then_with(|| match (a_modifier, z_modifier) {
                    (Some(a), Some(z)) => less_or_greater(text(a), text(z)),
                    (Some(_), None) => Ordering::Greater,
                    (None, Some(_)) => Ordering::Less,
                    (None, None) => Ordering::Equal,
                });
        }
        if let Some(&(container, direction)) = self.comparators.get(&a_order) {
            let a = self.resolved_size(left, container);
            let z = self.resolved_size(right, container);
            return match (a, z) {
                (None, _) => {
                    if direction == Direction::Asc {
                        Ordering::Less
                    } else {
                        Ordering::Greater
                    }
                }
                (_, None) => {
                    if direction == Direction::Asc {
                        Ordering::Greater
                    } else {
                        Ordering::Less
                    }
                }
                (Some(a), Some(z)) => breakpoints::compare(a, z, direction),
            };
        }
        if a_root != z_root {
            return utf16(a_root, z_root);
        }
        let (Variant::Functional { value: a, .. }, Variant::Functional { value: z, .. }) =
            (left, right)
        else {
            return Ordering::Equal;
        };
        let (a, z) = match (a, z) {
            (None, _) => return Ordering::Less,
            (_, None) => return Ordering::Greater,
            (Some(a), Some(z)) => (a, z),
        };
        matches!(a, VariantValue::Arbitrary { .. })
            .cmp(&matches!(z, VariantValue::Arbitrary { .. }))
            .then_with(|| less_or_greater(text(a), text(z)))
    }

    fn resolved_size<'a>(&'a self, variant: &'a Variant, container: bool) -> Option<&'a str> {
        let namespace = if container {
            "--container"
        } else {
            "--breakpoint"
        };
        let value = match variant {
            Variant::Static { root } if !container => {
                return self.theme.resolve_value(Some(root), &[namespace.into()]);
            }
            Variant::Functional {
                value: Some(value),
                modifier,
                ..
            } => {
                if !container && modifier.is_some() {
                    return None;
                }
                match value {
                    VariantValue::Named { value } => {
                        self.theme.resolve_value(Some(value), &[namespace.into()])?
                    }
                    VariantValue::Arbitrary { value } => value,
                }
            }
            _ => return None,
        };
        (!value.is_empty() && !value.contains("var(")).then_some(value)
    }

    pub fn apply(&self, node: &mut AstNode, variant: &Variant) -> Result<bool, String> {
        let registry = Registry {
            variants: self.registry(),
            ..Registry::default()
        };
        self.apply_with(node, variant, &mut |name| {
            crate::candidate::parse_variant(name, &registry)
        })
    }

    pub fn apply_with(
        &self,
        node: &mut AstNode,
        variant: &Variant,
        parse_variant: &mut impl FnMut(&str) -> Option<Variant>,
    ) -> Result<bool, String> {
        self.apply_at_depth(node, variant, 0, parse_variant)
    }

    fn apply_at_depth(
        &self,
        node: &mut AstNode,
        variant: &Variant,
        depth: usize,
        parse_variant: &mut impl FnMut(&str) -> Option<Variant>,
    ) -> Result<bool, String> {
        if let Variant::Arbitrary { selector, relative } = variant {
            if *relative && depth == 0 {
                return Ok(false);
            }
            let Some(nodes) = node.children_mut() else {
                return Ok(false);
            };
            let body = std::mem::take(nodes);
            let mut child = rule(selector, body.clone());
            if let AstNode::AtRule { name, nodes, .. } = &mut child
                && name == "@scope"
            {
                *nodes = vec![context("user", body)];
                child = context("variant", vec![child]);
            }
            *nodes = vec![child];
            return Ok(true);
        }
        let Some(definition) = root(variant).and_then(|root| self.definitions.get(root)) else {
            return Ok(false);
        };
        if let Variant::Compound {
            root,
            variant: child_variant,
            ..
        } = variant
        {
            let mut isolated = AstNode::AtRule {
                name: "@slot".into(),
                params: String::new(),
                nodes: vec![],
            };
            if !self.apply_at_depth(&mut isolated, child_variant, depth + 1, parse_variant)? {
                return Ok(false);
            }
            let AstNode::AtRule { mut nodes, .. } = isolated else {
                unreachable!()
            };
            if root == "not" && nodes.len() > 1 {
                return Ok(false);
            }
            for child in &mut nodes {
                if !matches!(child, AstNode::Rule { .. } | AstNode::AtRule { .. })
                    || !self.apply_handler(child, variant, &definition.handler, parse_variant)?
                {
                    return Ok(false);
                }
            }
            let Some(body) = node.children_mut() else {
                return Ok(false);
            };
            fill_empty_rules(&mut nodes, body);
            *body = nodes;
            return Ok(true);
        }
        self.apply_handler(node, variant, &definition.handler, parse_variant)
    }

    fn apply_handler(
        &self,
        node: &mut AstNode,
        variant: &Variant,
        handler: &Handler,
        parse_variant: &mut impl FnMut(&str) -> Option<Variant>,
    ) -> Result<bool, String> {
        Ok(match handler {
            Handler::Selectors {
                styles,
                at_rules,
                uses_at_scope,
            } => {
                let Some(body) = node.children_mut() else {
                    return Ok(false);
                };
                let slot = if *uses_at_scope {
                    vec![context("user", std::mem::take(body))]
                } else {
                    std::mem::take(body)
                };
                let mut nodes = Vec::new();
                if !styles.is_empty() {
                    nodes.push(AstNode::Rule {
                        selector: styles.join(", "),
                        nodes: slot.clone(),
                    });
                }
                nodes.extend(at_rules.iter().map(|selector| rule(selector, slot.clone())));
                *body = if *uses_at_scope {
                    vec![context("variant", nodes)]
                } else {
                    nodes
                };
                true
            }
            Handler::Template(template) => {
                let Some(body) = node.children_mut() else {
                    return Ok(false);
                };
                let mut template = template.clone();
                for node in &mut template {
                    node.fresh_declarations();
                }
                fill_slots(&mut template, body, false);
                *body = template;
                true
            }
            Handler::Custom {
                nodes,
                uses_at_variant,
                uses_at_scope,
            } => {
                let mut template = nodes.clone();
                for node in &mut template {
                    node.fresh_declarations();
                }
                if *uses_at_variant {
                    self.substitute_at_variant_with(&mut template, parse_variant)?;
                }
                let Some(body) = node.children_mut() else {
                    return Ok(false);
                };
                if *uses_at_scope {
                    substitute_at_slot(&mut template, &[context("user", body.clone())]);
                    *body = vec![context("variant", template)];
                } else {
                    substitute_at_slot(&mut template, body);
                    *body = template;
                }
                true
            }
            Handler::Breakpoint(value) => wrap(
                node,
                AstNode::AtRule {
                    name: "@media".into(),
                    params: format!("(width >= {value})"),
                    nodes: vec![],
                },
            ),
            Handler::Builtin(root) => match variant {
                Variant::Functional {
                    value, modifier, ..
                } => self.functional(node, variant, root, value.as_ref(), modifier.as_ref()),
                Variant::Compound {
                    modifier, variant, ..
                } => compound(
                    node,
                    root,
                    modifier.as_ref(),
                    variant,
                    self.theme.prefix.as_deref(),
                ),
                _ => false,
            },
        })
    }

    fn functional(
        &self,
        node: &mut AstNode,
        variant: &Variant,
        root: &str,
        value: Option<&VariantValue>,
        modifier: Option<&Modifier>,
    ) -> bool {
        let Some(value) = value else {
            return false;
        };
        let raw = text(value);
        if !root.starts_with('@') && modifier.is_some() {
            return false;
        }
        let wrapper = match root {
            "aria" | "data" => {
                let attribute = if root == "aria" && matches!(value, VariantValue::Named { .. }) {
                    format!("{raw}=\"true\"")
                } else {
                    quote_attribute(raw)
                };
                let selector = format!("[{root}-{attribute}]");
                if attribute::parse(&selector).is_none() {
                    return false;
                }
                rule(&format!("&{selector}"), vec![])
            }
            "nth" | "nth-last" | "nth-of-type" | "nth-last-of-type" => {
                if matches!(value, VariantValue::Named { .. }) && !numeric::is_positive_integer(raw)
                {
                    return false;
                }
                let selector = match root {
                    "nth" => "nth-child",
                    "nth-last" => "nth-last-child",
                    other => other,
                };
                rule(&format!("&:{selector}({raw})"), vec![])
            }
            "supports" => {
                let params = supports(raw);
                AstNode::AtRule {
                    name: "@supports".into(),
                    params,
                    nodes: vec![],
                }
            }
            "min" | "max" | "@" | "@min" | "@max" => {
                let container = root.starts_with('@');
                let Some(value) = self.resolved_size(variant, container) else {
                    return false;
                };
                let operator = if root.ends_with("max") { "<" } else { ">=" };
                let params = format!(
                    "{}(width {operator} {value})",
                    modifier.map_or_else(String::new, |modifier| format!("{} ", text(modifier)))
                );
                AstNode::AtRule {
                    name: if container { "@container" } else { "@media" }.into(),
                    params,
                    nodes: vec![],
                }
            }
            _ => return false,
        };
        wrap(node, wrapper)
    }
}

fn root(variant: &Variant) -> Option<&str> {
    match variant {
        Variant::Arbitrary { .. } => None,
        Variant::Static { root }
        | Variant::Functional { root, .. }
        | Variant::Compound { root, .. } => Some(root),
    }
}

fn text(value: &VariantValue) -> &str {
    match value {
        VariantValue::Named { value } | VariantValue::Arbitrary { value } => value,
    }
}

fn utf16(left: &str, right: &str) -> Ordering {
    left.encode_utf16().cmp(right.encode_utf16())
}
fn less_or_greater(left: &str, right: &str) -> Ordering {
    if utf16(left, right) == Ordering::Less {
        Ordering::Less
    } else {
        Ordering::Greater
    }
}

fn context(source: &str, nodes: Vec<AstNode>) -> AstNode {
    AstNode::Context {
        context: IndexMap::from([("source".into(), ContextValue::String(source.into()))]),
        nodes,
    }
}

fn wrap(node: &mut AstNode, mut wrapper: AstNode) -> bool {
    let Some(nodes) = node.children_mut() else {
        return false;
    };
    *wrapper.children_mut().unwrap() = std::mem::take(nodes);
    *nodes = vec![wrapper];
    true
}

fn fill_empty_rules(nodes: &mut [AstNode], body: &[AstNode]) {
    for node in nodes {
        if matches!(node, AstNode::Rule { nodes, .. } | AstNode::AtRule { nodes, .. } if nodes.is_empty())
        {
            *node.children_mut().unwrap() = body.to_vec();
        } else if let Some(nodes) = node.children_mut() {
            fill_empty_rules(nodes, body);
        }
    }
}

fn fill_slots(ast: &mut Vec<AstNode>, body: &[AstNode], hoist: bool) {
    let mut index = 0;
    while index < ast.len() {
        match &mut ast[index] {
            AstNode::AtRule { name, .. } if name == "@slot" => {
                ast.splice(index..=index, body.iter().cloned());
                index += body.len();
                continue;
            }
            AstNode::AtRule { name, .. }
                if hoist && (name == "@keyframes" || name == "@property") =>
            {
                ast[index] = AstNode::AtRoot {
                    nodes: vec![ast[index].clone()],
                };
            }
            other => {
                if let Some(nodes) = other.children_mut() {
                    fill_slots(nodes, body, hoist);
                }
            }
        }
        index += 1;
    }
}

pub fn substitute_at_slot(ast: &mut Vec<AstNode>, body: &[AstNode]) {
    fill_slots(ast, body, true);
}

pub fn compounds_for_selectors<'a>(selectors: impl IntoIterator<Item = &'a str>) -> u8 {
    let mut compounds = 0;
    for selector in selectors {
        if selector.starts_with('@') {
            if !["@media", "@supports", "@container"]
                .iter()
                .any(|name| selector.starts_with(name))
            {
                return 0;
            }
            compounds |= 1;
        } else {
            if selector.contains("::") {
                return 0;
            }
            compounds |= 2;
        }
    }
    compounds
}

fn quote_attribute(input: &str) -> String {
    if !input.contains('=') {
        return input.into();
    }
    let parts = segment(input, b'=');
    let value = parts[1..].join("=");
    let value = value.trim_matches(is_js_whitespace);
    if value.starts_with(['\'', '"']) {
        return input.into();
    }
    if let Some(flag) = value
        .as_bytes()
        .last()
        .copied()
        .filter(|flag| b"iIsS".contains(flag))
        && value.as_bytes().get(value.len().saturating_sub(2)) == Some(&b' ')
    {
        return format!(
            "{}=\"{}\" {}",
            parts[0],
            &value[..value.len() - 2],
            flag as char
        );
    }
    format!("{}=\"{value}\"", parts[0])
}

fn supports(input: &str) -> String {
    let offset = input
        .bytes()
        .take_while(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
        .count();
    if input[offset..]
        .trim_start_matches(is_js_whitespace)
        .starts_with('(')
    {
        fn space_operators(nodes: &mut [value::Value]) -> bool {
            let mut changed = false;
            for node in nodes {
                if let value::Value::Function { value, nodes } = node {
                    if value == "selector" {
                        continue;
                    }
                    if matches!(value.as_str(), "and" | "or" | "not") {
                        *value = format!(" {value} ");
                        changed = true;
                    }
                    changed |= space_operators(nodes);
                }
            }
            changed
        }
        let mut ast = value::parse(input);
        return if space_operators(&mut ast) {
            value::to_css(&ast)
        } else {
            input.into()
        };
    }
    let mut query = if input.contains(':') {
        input.into()
    } else {
        format!("{input}: var(--tw)")
    };
    if !query.starts_with('(') || !query.ends_with(')') {
        query = format!("({query})");
    }
    query
}

fn compound(
    node: &mut AstNode,
    root: &str,
    modifier: Option<&Modifier>,
    variant: &Variant,
    prefix: Option<&str>,
) -> bool {
    if root == "not" {
        if modifier.is_some() || matches!(variant, Variant::Arbitrary { relative: true, .. }) {
            return false;
        }
        return negate(node);
    }
    if !matches!(root, "group" | "peer" | "has" | "in") {
        return false;
    }
    if matches!(root, "group" | "peer") {
        if matches!(variant, Variant::Arbitrary { relative: true, .. }) {
            return false;
        }
    } else if modifier.is_some() {
        return false;
    }
    let marker = format!(
        ":where(.{}{root}{})",
        prefix
            .filter(|prefix| !prefix.is_empty())
            .map_or_else(String::new, |prefix| format!("{prefix}\\:")),
        modifier.map_or_else(String::new, |modifier| format!("\\/{}", text(modifier)))
    );
    fn rewrite(
        node: &mut AstNode,
        nested: bool,
        root: &str,
        marker: &str,
        did_apply: &mut bool,
    ) -> bool {
        let mut child_nested = nested;
        if let AstNode::Rule { selector, .. } = node {
            if nested {
                *did_apply = false;
                return false;
            }
            *selector = match root {
                "group" | "peer" => {
                    let mut target = selector.replace('&', marker);
                    if segment(&target, b',').len() > 1 {
                        target = format!(":is({target})");
                    }
                    if root == "group" {
                        format!(":is({target} &)")
                    } else {
                        format!(":is({target} ~ &)")
                    }
                }
                "in" => format!(":where({}) &", selector.replace('&', "*")),
                "has" => format!("&:has({})", selector.replace('&', "*")),
                _ => unreachable!(),
            };
            *did_apply = true;
            child_nested = true;
        }
        if let Some(nodes) = node.children_mut() {
            for child in nodes {
                if !rewrite(child, child_nested, root, marker, did_apply) {
                    return false;
                }
            }
        }
        true
    }
    let mut did_apply = false;
    rewrite(node, false, root, &marker, &mut did_apply) && did_apply
}

fn negate(node: &mut AstNode) -> bool {
    // The upstream walk retains its original child frames after replacing the
    // outer rule. Keep those paths, and read the current outer rule on each visit.
    fn paths(node: &AstNode, path: &mut Vec<AstNode>, output: &mut Vec<Vec<AstNode>>) {
        path.push(node.clone());
        match node {
            AstNode::Rule { nodes, .. } | AstNode::AtRule { nodes, .. } if nodes.is_empty() => {
                output.push(path.clone())
            }
            AstNode::Rule { nodes, .. }
            | AstNode::AtRule { nodes, .. }
            | AstNode::Context { nodes, .. }
            | AstNode::AtRoot { nodes } => {
                for child in nodes {
                    paths(child, path, output);
                }
            }
            _ => {}
        }
        path.pop();
    }
    let mut leaves = vec![];
    paths(node, &mut vec![], &mut leaves);
    let mut did_apply = false;
    for mut path in leaves {
        path[0] = node.clone();
        let style_rules: Vec<_> = path
            .iter()
            .filter_map(|node| match node {
                AstNode::Rule { selector, .. } => Some(selector),
                _ => None,
            })
            .collect();
        let at_rules: Vec<_> = path
            .iter()
            .filter_map(|node| match node {
                AstNode::AtRule { name, params, .. } => Some((name, params)),
                _ => None,
            })
            .collect();
        if style_rules.len() > 1 || at_rules.len() > 1 {
            break;
        }
        let mut rules = vec![];
        for selector in style_rules {
            if selector.contains("::") {
                return false;
            }
            rules.push(rule(
                &format!(
                    "&:not({})",
                    segment(selector, b',')
                        .iter()
                        .map(|selector| selector.replace('&', "*"))
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
                vec![],
            ));
        }
        for (name, params) in at_rules {
            if !matches!(name.as_str(), "@media" | "@supports" | "@container")
                || segment(params, b',').len() > 1
            {
                return false;
            }
            rules.push(AstNode::AtRule {
                name: name.clone(),
                params: negate_condition(name, params),
                nodes: vec![],
            });
        }
        *node = rule("&", rules);
        did_apply = true;
    }
    if let AstNode::Rule { selector, nodes } = node
        && selector == "&"
        && nodes.len() == 1
    {
        *node = nodes.remove(0);
    }
    did_apply
}

fn negate_condition(name: &str, condition: &str) -> String {
    let trimmed = condition.trim_matches(is_js_whitespace);
    if name != "@container" {
        let parts = segment(trimmed, b' ');
        return if parts[0] == "not" {
            parts[1..].join(" ")
        } else {
            format!("not {trimmed}")
        };
    }
    use value::Value::{Function, Separator, Word};
    let mut ast = value::parse(trimmed);
    if matches!(ast.first(), Some(Function { .. })) {
        return format!("not {condition}");
    }
    if matches!(ast.first(), Some(Word { value }) if value == "not")
        && matches!(ast.get(2), Some(Function { .. }))
    {
        ast.drain(..2);
    } else if matches!(ast.first(), Some(Word { .. }))
        && matches!(ast.get(2), Some(Word { value }) if value == "not")
        && matches!(ast.get(4), Some(Function { .. }))
    {
        ast.drain(2..4);
    } else if matches!(ast.first(), Some(Word { value }) if value != "not")
        && matches!(ast.get(2), Some(Function { .. }))
    {
        ast.splice(
            1..1,
            [
                Separator { value: " ".into() },
                Word {
                    value: "not".into(),
                },
            ],
        );
    } else {
        return format!("not {condition}");
    }
    value::to_css(&ast)
}
