// Port of Tailwind CSS 4.3.4 compile.ts and the core design-system caches.
// See LICENSE.tailwind.
use crate::{
    ast::AstNode,
    candidate::{self, Candidate, Kind, Registry, Variant},
    compare, css_functions,
    custom_utilities::CssUtility,
    id_hasher::IdHasher,
    js_sort,
    syntax::{escape::escape, segment::segment},
    theme::Theme,
    utilities,
    variant_cache::{Identity as VariantIdentity, VariantCache},
    variants::Variants,
};
use indexmap::{IndexMap, IndexSet};
use serde::{Serialize, Serializer};
use std::{
    cmp::Ordering,
    collections::{BTreeSet, HashMap, VecDeque},
    hash::BuildHasherDefault,
    sync::{Arc, LazyLock},
};

static PROPERTY_ORDER: LazyLock<HashMap<String, usize>> = LazyLock::new(|| {
    let properties: Vec<String> = serde_json::from_str(include_str!("../data/property-order.json"))
        .expect("vendored property order");
    let mut order = HashMap::with_capacity(properties.len());
    for (index, property) in properties.into_iter().enumerate() {
        order.entry(property).or_insert(index);
    }
    order
});

static EMPTY_COMPILED_RULES: LazyLock<Arc<Vec<CompiledRule>>> =
    LazyLock::new(|| Arc::new(Vec::new()));
type CompiledCache = HashMap<usize, Arc<Vec<CompiledRule>>, BuildHasherDefault<IdHasher>>;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct PropertySort {
    pub order: Vec<usize>,
    pub count: usize,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct VariantOrder(Vec<u64>);

impl VariantOrder {
    fn insert(&mut self, bit: usize) {
        self.0.resize(self.0.len().max(bit / 64 + 1), 0);
        self.0[bit / 64] |= 1 << (bit % 64);
    }

    pub fn decimal(&self) -> String {
        let mut digits = vec![0u64];
        for &word in self.0.iter().rev() {
            for bit in (0..64).rev() {
                let mut carry = (word >> bit) & 1;
                for digit in &mut digits {
                    let value = *digit * 2 + carry;
                    *digit = value % 1_000_000_000;
                    carry = value / 1_000_000_000;
                }
                if carry != 0 {
                    digits.push(carry);
                }
            }
        }
        let mut result = digits.pop().unwrap().to_string();
        for digit in digits.iter().rev() {
            result.push_str(&format!("{digit:09}"));
        }
        result
    }
}

impl Ord for VariantOrder {
    fn cmp(&self, other: &Self) -> Ordering {
        self.0
            .len()
            .cmp(&other.0.len())
            .then_with(|| self.0.iter().rev().cmp(other.0.iter().rev()))
    }
}
impl PartialOrd for VariantOrder {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl Serialize for VariantOrder {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.decimal())
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct CompiledNode {
    pub node: AstNode,
    pub properties: PropertySort,
    pub variants: VariantOrder,
    pub candidate: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct Compilation {
    pub nodes: Vec<CompiledNode>,
    pub invalid: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct CompiledRule {
    pub node: AstNode,
    pub property_sort: PropertySort,
}

pub struct DesignSystem {
    pub theme: Theme,
    pub variants: Variants,
    pub important: bool,
    pub invalid_candidates: IndexSet<String>,
    registry: Registry,
    utility_bindings: utilities::Bindings,
    utility_roots: IndexMap<String, Vec<Kind>>,
    utility_registry_revision: Arc<()>,
    legacy_utilities: bool,
    pub(crate) compatibility_theme: bool,
    pub(crate) custom_utilities: Vec<CssUtility>,
    parsed_candidates: IndexMap<String, Vec<Candidate>>,
    parsed_variants: VariantCache,
    compiled_ast_nodes: [CompiledCache; 2],
    cached_variant_order: Option<(usize, Arc<IndexMap<String, usize>>)>,
}

fn cached_variant(input: &str, registry: &Registry, cache: &mut VariantCache) -> Option<Variant> {
    cache.parse(input, registry)
}

impl DesignSystem {
    pub fn get_class_list(&self) -> Vec<crate::completions::ClassEntry> {
        crate::completions::class_list_with(
            self.utility_keys(Kind::Static),
            self.utility_keys(Kind::Functional),
            |root| self.utility_groups(root),
            |_| true,
        )
    }

    pub fn new(theme: Theme) -> Self {
        let utility_bindings = utilities::Bindings::new(&theme);
        let variants = Variants::new(&theme);
        let mut registry = utilities::registry(theme.prefix.clone());
        variants.extend_registry(&mut registry);
        Self {
            theme,
            variants,
            registry,
            utility_bindings,
            utility_roots: utilities::root_kinds(),
            utility_registry_revision: Arc::default(),
            legacy_utilities: false,
            compatibility_theme: false,
            custom_utilities: Vec::new(),
            important: false,
            invalid_candidates: IndexSet::new(),
            parsed_candidates: IndexMap::new(),
            parsed_variants: VariantCache::default(),
            compiled_ast_nodes: Default::default(),
            cached_variant_order: None,
        }
    }

    pub fn utility_keys(&self, kind: Kind) -> Vec<String> {
        self.utility_roots
            .iter()
            .filter(|(_, kinds)| kinds.contains(&kind))
            .map(|(root, _)| root.clone())
            .collect()
    }

    pub(crate) fn utility_registry_revision(&self) -> &Arc<()> {
        &self.utility_registry_revision
    }

    pub fn utility_groups(&self, root: &str) -> Vec<crate::completions::Group> {
        let hidden = self.legacy_utilities
            && crate::legacy_utilities::DEFINITIONS
                .get(root)
                .is_some_and(|definition| definition.hidden);
        let mut groups = if crate::completions::has_registered_groups(root) {
            crate::completions::utility_groups(root, &self.theme)
        } else {
            Vec::new()
        };
        groups.extend(
            self.custom_utilities
                .iter()
                .filter(|utility| utility.root == root)
                .filter_map(|utility| utility.suggestions(&self.theme)),
        );
        if groups.is_empty() && !hidden && self.registry.has_utility(root, Kind::Static) {
            groups.push(crate::completions::Group {
                supports_negative: Some(false),
                ..Default::default()
            });
        }
        groups
    }

    /// Register the compatibility utilities before parsing or compiling candidates.
    pub fn register_legacy_utilities(&mut self) {
        Arc::make_mut(&mut self.utility_registry_revision);
        self.legacy_utilities = true;
        for (root, definition) in crate::legacy_utilities::DEFINITIONS.iter() {
            self.registry
                .utilities
                .entry(root.clone())
                .or_default()
                .insert(definition.kind);
            let kinds = self.utility_roots.entry(root.clone()).or_default();
            if !kinds.contains(&definition.kind) {
                kinds.push(definition.kind);
            }
        }
    }

    pub fn custom_utility(&mut self, name: &str, nodes: &[AstNode]) -> Result<(), String> {
        let utility = CssUtility::new(name, nodes.to_vec())
            .ok_or_else(|| format!("Invalid utility name: {name}"))?;
        self.register_prepared_css_utility(utility);
        Ok(())
    }

    pub(crate) fn register_prepared_css_utility(&mut self, utility: CssUtility) -> usize {
        Arc::make_mut(&mut self.utility_registry_revision);
        self.registry
            .utilities
            .entry(utility.root.clone())
            .or_default()
            .insert(utility.kind);
        let kinds = self.utility_roots.entry(utility.root.clone()).or_default();
        if !kinds.contains(&utility.kind) {
            kinds.push(utility.kind);
        }
        self.custom_utilities.push(utility);
        self.custom_utilities.len() - 1
    }

    #[cfg(test)]
    pub(crate) fn register_css_utility(
        &mut self,
        name: &str,
        nodes: &mut Vec<AstNode>,
    ) -> Result<usize, String> {
        self.custom_utility(name, nodes)?;
        let index = self.custom_utilities.len() - 1;
        // createCssUtility preprocesses the same AST later visited by @apply.
        *nodes = self.custom_utilities[index].nodes.clone();
        Ok(index)
    }

    pub fn custom_variant(&mut self, name: &str, nodes: &[AstNode]) {
        self.variants.from_ast(name, nodes);
        self.variants.extend_registry(&mut self.registry);
    }

    pub(crate) fn custom_variant_selectors(&mut self, name: &str, selectors: &[String]) {
        self.variants.register_selectors(name, selectors);
        self.variants.extend_registry(&mut self.registry);
    }

    pub(crate) fn substitute_variants(&mut self, nodes: &mut Vec<AstNode>) -> Result<bool, String> {
        self.variants
            .substitute_at_variant_with(nodes, &mut |name| {
                cached_variant(name, &self.registry, &mut self.parsed_variants)
            })
    }

    pub fn candidates_to_ast(&mut self, classes: &[String]) -> Result<Vec<Vec<AstNode>>, String> {
        let mut result = Vec::with_capacity(classes.len());
        for class_name in classes {
            let compilation = self.compile_candidates(std::slice::from_ref(class_name), true)?;
            let nodes = crate::optimize::optimize_ast(
                compilation
                    .nodes
                    .into_iter()
                    .map(|node| node.node)
                    .collect(),
                &mut self.theme,
                0,
            )?;
            result.push(if compilation.invalid.is_empty() {
                nodes
            } else {
                Vec::new()
            });
        }
        Ok(result)
    }

    pub fn candidates_to_css(&mut self, classes: &[String]) -> Result<Vec<Option<String>>, String> {
        Ok(self
            .candidates_to_ast(classes)?
            .into_iter()
            .map(|nodes| (!nodes.is_empty()).then(|| crate::ast::to_css(&nodes)))
            .collect())
    }

    pub fn get_class_order(
        &mut self,
        classes: &[String],
    ) -> Result<Vec<(String, Option<usize>)>, String> {
        // Ordering needs only sort metadata; retain cached ASTs without cloning them.
        let mut matches = IndexMap::new();
        for raw in classes {
            if !self.invalid_candidates.contains(raw) {
                if let Some(candidates) = self.parsed_candidates.get(raw)
                    && candidates.iter().all(|candidate| {
                        self.compiled_ast_nodes[1].contains_key(&candidate.identity.0)
                    })
                {
                    if !candidates.is_empty() {
                        matches.insert(raw, None);
                    }
                } else {
                    let candidates = self.parse_candidate(raw);
                    if !candidates.is_empty() {
                        matches.insert(raw, Some(candidates));
                    }
                }
            }
        }
        let variant_order = self.shared_variant_order()?;
        let mut rows = Vec::new();
        let mut nodes = Vec::new();
        for (raw, candidates) in matches {
            let variants = self.variant_mask(raw, &variant_order);
            let mut push = |rules: Arc<Vec<CompiledRule>>| {
                nodes.extend((0..rules.len()).map(|rule| (rows.len(), rule)));
                rows.push((raw, variants.clone(), rules));
            };
            if let Some(candidates) = candidates {
                for candidate in candidates {
                    push(self.shared_compile_ast_nodes(&candidate, true)?);
                }
            } else {
                // Parsing a cached spelling only clones its variant snapshots.
                // Complete compiled entries already retain the earlier snapshots.
                for candidate in &self.parsed_candidates[raw] {
                    push(Arc::clone(
                        &self.compiled_ast_nodes[1][&candidate.identity.0],
                    ));
                }
            }
        }
        js_sort::sort_by(&mut nodes, |&(a, ar), &(b, br)| {
            let (aname, avariants, arules) = &rows[a];
            let (bname, bvariants, brules) = &rows[b];
            compare_compilation(
                avariants,
                &arules[ar].property_sort,
                aname,
                bvariants,
                &brules[br].property_sort,
                bname,
            )
        });
        let mut order = IndexMap::new();
        for (row, _) in nodes {
            let index = order.len();
            order.entry(rows[row].0.as_str()).or_insert(index);
        }
        Ok(classes
            .iter()
            .map(|name| (name.clone(), order.get(name.as_str()).copied()))
            .collect())
    }

    fn variant_mask(&self, raw: &str, order: &IndexMap<String, usize>) -> VariantOrder {
        let mut variants = VariantOrder::default();
        let mut parts = segment(raw, b':');
        parts.pop();
        if self
            .registry
            .prefix
            .as_deref()
            .is_some_and(|prefix| !prefix.is_empty())
        {
            parts.remove(0);
        }
        for name in parts {
            variants.insert(order[name]);
        }
        variants
    }

    pub fn parse_variant(&mut self, input: &str) -> Option<Variant> {
        cached_variant(input, &self.registry, &mut self.parsed_variants)
    }

    pub(crate) fn shared_variant(&mut self, input: &str) -> Option<VariantIdentity> {
        self.parsed_variants.parse_identity(input, &self.registry)
    }

    pub(crate) fn shared_variant_child(
        &self,
        identity: VariantIdentity,
    ) -> Option<VariantIdentity> {
        self.parsed_variants.child(identity)
    }

    pub(crate) fn replace_shared_variant(
        &mut self,
        identity: VariantIdentity,
        value: &Variant,
        child: Option<VariantIdentity>,
    ) {
        self.parsed_variants.replace(identity, value, child);
    }

    pub fn parse_candidate(&mut self, input: &str) -> Vec<Candidate> {
        if let Some(candidates) = self.parsed_candidates.get(input) {
            if candidates.is_empty() {
                return Vec::new();
            }
            let mut candidates = candidates.clone();
            let mut names = segment(input, b':');
            names.pop();
            if self
                .registry
                .prefix
                .as_deref()
                .is_some_and(|p| !p.is_empty())
            {
                names.remove(0);
            }
            let variants: Vec<_> = names
                .iter()
                .rev()
                .filter_map(|name| {
                    self.parsed_variants
                        .parsed
                        .get(*name)
                        .copied()
                        .flatten()
                        .map(|identity| self.parsed_variants.snapshot(identity))
                })
                .collect();
            for candidate in &mut candidates {
                candidate.variants = variants.clone();
            }
            return candidates;
        }
        let candidates = candidate::parse_with(input, &self.registry, &mut |variant| {
            cached_variant(variant, &self.registry, &mut self.parsed_variants)
        });
        self.parsed_candidates
            .insert(input.into(), candidates.clone());
        candidates
    }

    // Replacements returned by parseCandidate remain shared with its cache.
    // Mutating them must retain their existing compiledAstNodes cache key.
    pub(crate) fn update_shared_candidate(&mut self, candidate: &Candidate) {
        if let Some(cached) = self.parsed_candidates.get_mut(&candidate.raw)
            && let Some(value) = cached
                .iter_mut()
                .find(|value| value.identity.0 == candidate.identity.0)
        {
            *value = candidate.clone();
        }
    }

    pub fn variant_order(&mut self) -> Result<IndexMap<String, usize>, String> {
        Ok((*self.shared_variant_order()?).clone())
    }

    fn shared_variant_order(&mut self) -> Result<Arc<IndexMap<String, usize>>, String> {
        if let Some((size, order)) = &self.cached_variant_order
            && *size == self.parsed_variants.parsed.len()
        {
            return Ok(order.clone());
        }
        let mut variants = self.parsed_variants.parsed.iter().collect::<Vec<_>>();
        let mut error = None;
        js_sort::sort_by(&mut variants, |(_, left), (_, right)| {
            match self
                .variants
                .compare_cached(&self.parsed_variants, **left, **right)
            {
                Ok(order) if error.is_none() => order,
                Ok(_) => Ordering::Equal,
                Err(cause) => {
                    error.get_or_insert(cause);
                    Ordering::Equal
                }
            }
        });
        if let Some(error) = error {
            return Err(error);
        }
        let mut orders = IndexMap::new();
        let mut previous = None;
        let mut index = 0;
        for (name, value) in variants {
            let Some(value) = *value else {
                continue;
            };
            if let Some(previous) = previous
                && self.variants.compare_cached(
                    &self.parsed_variants,
                    Some(previous),
                    Some(value),
                )? != Ordering::Equal
            {
                index += 1;
            }
            orders.insert(name.clone(), index);
            previous = Some(value);
        }
        let orders = Arc::new(orders);
        self.cached_variant_order = Some((self.parsed_variants.parsed.len(), Arc::clone(&orders)));
        Ok(orders)
    }

    pub fn compile_ast_nodes(
        &mut self,
        candidate: &Candidate,
        respect_important: bool,
    ) -> Result<Vec<CompiledRule>, String> {
        Ok((*self.shared_compile_ast_nodes(candidate, respect_important)?).clone())
    }

    pub(crate) fn shared_compile_ast_nodes(
        &mut self,
        candidate: &Candidate,
        respect_important: bool,
    ) -> Result<Arc<Vec<CompiledRule>>, String> {
        let flag = usize::from(respect_important);
        let key = candidate.identity.0;
        if let Some(rules) = self.compiled_ast_nodes[flag].get(&key) {
            return Ok(rules.clone());
        }
        let rules = self.compile_ast_nodes_uncached(candidate, respect_important)?;
        let rules = if rules.is_empty() {
            Arc::clone(&EMPTY_COMPILED_RULES)
        } else {
            Arc::new(rules)
        };
        self.compiled_ast_nodes[flag].insert(key, Arc::clone(&rules));
        Ok(rules)
    }

    pub(crate) fn first_compiled_candidate(
        &mut self,
        input: &str,
        respect_important: bool,
    ) -> Result<Option<Arc<Vec<CompiledRule>>>, String> {
        if let Some(candidate) = self
            .parsed_candidates
            .get(input)
            .and_then(|items| items.first())
            && let Some(rules) =
                self.compiled_ast_nodes[usize::from(respect_important)].get(&candidate.identity.0)
        {
            return Ok(Some(Arc::clone(rules)));
        }
        // A miss must refresh variant snapshots before compiling, as parse_candidate does.
        self.parse_candidate(input)
            .first()
            .map(|candidate| self.shared_compile_ast_nodes(candidate, respect_important))
            .transpose()
    }

    fn compile_ast_nodes_uncached(
        &mut self,
        candidate: &Candidate,
        respect_important: bool,
    ) -> Result<Vec<CompiledRule>, String> {
        let mut bodies =
            utilities::compile_base_with_bindings(candidate, &self.theme, &self.utility_bindings)
                .map_err(|error| error.to_string())?;
        if self.legacy_utilities {
            bodies.extend(crate::legacy_utilities::compile(
                &candidate.kind,
                &self.theme,
            ));
        }
        bodies.extend(
            self.custom_utilities
                .iter()
                .filter_map(|utility| utility.compile(candidate, &self.theme)),
        );
        let mut rules = Vec::new();
        for mut nodes in bodies {
            let property_sort = property_sort(&nodes);
            if candidate.important || (self.important && respect_important) {
                apply_important(&mut nodes);
            }
            let mut node = AstNode::Rule {
                selector: format!(".{}", escape(&candidate.raw)),
                nodes,
            };
            for variant in &candidate.variants {
                if !self.variants.apply_with(&mut node, variant, &mut |name| {
                    cached_variant(name, &self.registry, &mut self.parsed_variants)
                })? {
                    return Ok(Vec::new());
                }
            }
            rules.push(CompiledRule {
                node,
                property_sort,
            });
        }
        let mut nodes = rules
            .iter_mut()
            .map(|rule| std::mem::replace(&mut rule.node, AstNode::AtRoot { nodes: Vec::new() }))
            .collect::<Vec<_>>();
        if css_functions::substitute_with_options(&mut nodes, &self.theme, self.compatibility_theme)
            .is_err()
            || self
                .variants
                .substitute_at_variant_with(&mut nodes, &mut |name| {
                    cached_variant(name, &self.registry, &mut self.parsed_variants)
                })
                .is_err()
        {
            return Ok(Vec::new());
        }
        for (rule, node) in rules.iter_mut().zip(nodes) {
            rule.node = node;
        }
        Ok(rules)
    }

    pub fn compile_candidates<I, S>(
        &mut self,
        input: I,
        respect_important: bool,
    ) -> Result<Compilation, String>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        self.compile_candidates_with(input, respect_important, |_, _| Ok(()))
    }

    pub fn compile_candidates_with<I, S>(
        &mut self,
        input: I,
        respect_important: bool,
        mut on_invalid: impl FnMut(&mut Self, &str) -> Result<(), String>,
    ) -> Result<Compilation, String>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let mut matches = IndexMap::new();
        let mut invalid = Vec::new();
        for raw in input {
            let raw = raw.as_ref();
            if self.invalid_candidates.contains(raw) {
                on_invalid(self, raw)?;
                invalid.push(raw.into());
                continue;
            }
            let candidates = self.parse_candidate(raw);
            if candidates.is_empty() {
                on_invalid(self, raw)?;
                invalid.push(raw.into());
            } else {
                matches.insert(raw.to_owned(), candidates);
            }
        }
        let order = self.shared_variant_order()?;
        let mut nodes = Vec::new();
        for (raw, candidates) in matches {
            let mut found = false;
            let variants = self.variant_mask(&raw, &order);
            for candidate in candidates {
                let rules = self.compile_ast_nodes(&candidate, respect_important)?;
                found |= !rules.is_empty();
                for rule in rules {
                    nodes.push(CompiledNode {
                        node: rule.node,
                        properties: rule.property_sort,
                        variants: variants.clone(),
                        candidate: raw.clone(),
                    });
                }
            }
            if !found {
                on_invalid(self, &raw)?;
                invalid.push(raw);
            }
        }
        js_sort::sort_by(&mut nodes, |a, b| {
            compare_compilation(
                &a.variants,
                &a.properties,
                &a.candidate,
                &b.variants,
                &b.properties,
                &b.candidate,
            )
        });
        Ok(Compilation { nodes, invalid })
    }
}

fn compare_compilation(
    avariants: &VariantOrder,
    a: &PropertySort,
    aname: &str,
    bvariants: &VariantOrder,
    b: &PropertySort,
    bname: &str,
) -> Ordering {
    avariants
        .cmp(bvariants)
        .then_with(|| {
            for (left, right) in a.order.iter().zip(&b.order) {
                if left != right {
                    return left.cmp(right);
                }
            }
            b.order.len().cmp(&a.order.len())
        })
        .then_with(|| b.count.cmp(&a.count))
        .then_with(|| compare::natural(aname, bname))
}

pub fn apply_important(nodes: &mut [AstNode]) {
    for node in nodes {
        match node {
            AstNode::Declaration { important, .. } => *important = true,
            AstNode::Rule { nodes, .. } | AstNode::AtRule { nodes, .. } => apply_important(nodes),
            _ => {}
        }
    }
}

pub fn property_sort(nodes: &[AstNode]) -> PropertySort {
    let mut order = BTreeSet::new();
    let mut count = 0;
    let mut queue = VecDeque::from_iter(nodes);
    let mut seen_tw_sort = false;
    while let Some(node) = queue.pop_front() {
        match node {
            AstNode::Declaration {
                property,
                value: Some(value),
                ..
            } => {
                count += 1;
                if seen_tw_sort {
                    continue;
                }
                if property == "--tw-sort"
                    && let Some(&index) = PROPERTY_ORDER.get(value)
                {
                    order.insert(index);
                    seen_tw_sort = true;
                    continue;
                }
                if let Some(&index) = PROPERTY_ORDER.get(property.as_str()) {
                    order.insert(index);
                }
            }
            AstNode::Rule { nodes, .. } | AstNode::AtRule { nodes, .. } => queue.extend(nodes),
            _ => {}
        }
    }
    PropertySort {
        order: order.into_iter().collect(),
        count,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn class_order_cache_hits_match_full_compilation_across_state_changes() {
        fn reference(
            system: &mut DesignSystem,
            classes: &[String],
        ) -> Result<Vec<(String, Option<usize>)>, String> {
            let compiled = system.compile_candidates(classes, true)?;
            let mut order = IndexMap::new();
            for node in compiled.nodes {
                let next = order.len();
                order.entry(node.candidate).or_insert(next);
            }
            Ok(classes
                .iter()
                .map(|class| (class.clone(), order.get(class).copied()))
                .collect())
        }
        type CachedEntries = Vec<(String, usize, Vec<(String, PropertySort)>)>;
        fn cache(system: &DesignSystem) -> CachedEntries {
            system
                .parsed_candidates
                .iter()
                .flat_map(|(raw, candidates)| {
                    candidates
                        .iter()
                        .enumerate()
                        .filter_map(|(index, candidate)| {
                            system.compiled_ast_nodes[1]
                                .get(&candidate.identity.0)
                                .map(|rules| {
                                    (
                                        raw.clone(),
                                        index,
                                        rules
                                            .iter()
                                            .map(|rule| {
                                                (
                                                    crate::ast::to_css(std::slice::from_ref(
                                                        &rule.node,
                                                    )),
                                                    rule.property_sort.clone(),
                                                )
                                            })
                                            .collect(),
                                    )
                                })
                        })
                })
                .collect()
        }
        for prefix in [None, Some("tw")] {
            let mut theme = Theme::default();
            theme.prefix = prefix.map(str::to_owned);
            theme.add("--spacing", "0.25rem", 0).unwrap();
            let mut baseline = DesignSystem::new(theme.clone());
            let mut optimized = DesignSystem::new(theme);
            let inputs = [
                "p-4",
                "hover:flex",
                "bg-size-0",
                "p-8!",
                "space-x-2",
                "flex",
                "nope",
                "",
                "hover:flex",
            ]
            .map(|raw| prefix.map_or_else(|| raw.to_owned(), |prefix| format!("{prefix}:{raw}")));
            // Leave one ambiguous spelling partly compiled and fill only the
            // non-respecting cache for another spelling before ordering.
            for system in [&mut baseline, &mut optimized] {
                let candidate = system.parse_candidate(&inputs[2]).remove(0);
                system.shared_compile_ast_nodes(&candidate, true).unwrap();
                let candidate = system.parse_candidate(&inputs[0]).remove(0);
                system.shared_compile_ast_nodes(&candidate, false).unwrap();
                system.invalid_candidates.insert(inputs[6].clone());
            }
            for phase in 0..4 {
                for system in [&mut baseline, &mut optimized] {
                    if phase > 0 {
                        let identity = system.shared_variant("hover").unwrap();
                        system.replace_shared_variant(
                            identity,
                            &Variant::Arbitrary {
                                selector: if phase == 1 { "&:focus" } else { "&:checked" }.into(),
                                relative: false,
                            },
                            None,
                        );
                        system.theme.add("--spacing", "0.5rem", 0).unwrap();
                        system.important = phase == 2;
                    }
                }
                let classes: Vec<_> = if phase % 2 == 0 {
                    inputs.to_vec()
                } else {
                    inputs.iter().rev().cloned().collect()
                };
                assert_eq!(
                    optimized.get_class_order(&classes),
                    reference(&mut baseline, &classes),
                    "{phase}"
                );
                assert_eq!(cache(&optimized), cache(&baseline), "{phase}");
                assert_eq!(
                    optimized.parsed_variants.parsed.len(),
                    baseline.parsed_variants.parsed.len()
                );
            }
        }
    }

    #[test]
    fn first_compiled_candidate_preserves_variant_snapshots_and_flag_caches() {
        for prefix in [None, Some("tw")] {
            let mut theme = Theme::default();
            theme.prefix = prefix.map(str::to_owned);
            theme.add("--spacing", "0.25rem", 0).unwrap();
            let mut baseline = DesignSystem::new(theme.clone());
            let mut optimized = DesignSystem::new(theme);
            let inputs = ["flex", "hover:flex", "bg-size-0", "hover:p-[1rem]", "nope"]
                .map(|input| prefix.map_or_else(|| input.into(), |p| format!("{p}:{input}")));
            for system in [&mut baseline, &mut optimized] {
                for input in &inputs {
                    system.parse_candidate(input);
                }
            }
            for (phase, selector) in ["&:focus", "&:active", "&:checked"].into_iter().enumerate() {
                for system in [&mut baseline, &mut optimized] {
                    let identity = system.shared_variant("hover").unwrap();
                    system.replace_shared_variant(
                        identity,
                        &Variant::Arbitrary {
                            selector: selector.into(),
                            relative: false,
                        },
                        None,
                    );
                    system.important = phase == 2;
                }
                for respect_important in [true, false] {
                    if phase == 0 && !respect_important {
                        continue;
                    }
                    for input in &inputs {
                        let expected = baseline
                            .parse_candidate(input)
                            .first()
                            .map(|candidate| {
                                baseline.shared_compile_ast_nodes(candidate, respect_important)
                            })
                            .transpose()
                            .unwrap();
                        let actual = optimized
                            .first_compiled_candidate(input, respect_important)
                            .unwrap();
                        let observable = |rules: Option<Arc<Vec<CompiledRule>>>| {
                            rules.map(|rules| {
                                rules
                                    .iter()
                                    .map(|rule| {
                                        (
                                            crate::ast::to_css(std::slice::from_ref(&rule.node)),
                                            rule.property_sort.clone(),
                                        )
                                    })
                                    .collect::<Vec<_>>()
                            })
                        };
                        assert_eq!(observable(actual), observable(expected), "{phase} {input}");
                    }
                }
            }
        }
    }
}
