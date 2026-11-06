// Port of Tailwind CSS 4.3.4 canonicalize-candidates.ts storage and signature indexes.
use crate::{
    apply,
    ast::{self, AstNode, rule},
    candidate::Kind,
    compile::DesignSystem,
    declaration,
    id_hasher::IdHasher,
    numeric, signature,
    syntax::selector::{self, Selector},
};
use compact_str::CompactString;
use indexmap::{IndexMap, IndexSet};
use std::{
    collections::HashMap,
    hash::BuildHasherDefault,
    sync::{Arc, LazyLock},
};

type IdMap<T> = IndexMap<usize, T, BuildHasherDefault<IdHasher>>;
type IdSet = IndexSet<usize, BuildHasherDefault<IdHasher>>;

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct RemKey(Option<u64>);
impl From<Option<f64>> for RemKey {
    fn from(value: Option<f64>) -> Self {
        Self(value.map(|value| {
            if value == 0.0 {
                0
            } else if value.is_nan() {
                f64::NAN.to_bits()
            } else {
                value.to_bits()
            }
        }))
    }
}
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct OptionsKey {
    rem: RemKey,
    features: u8,
}
impl From<signature::Options> for OptionsKey {
    fn from(value: signature::Options) -> Self {
        Self {
            rem: value.rem.into(),
            features: value.features,
        }
    }
}

pub type PropertyValues = IndexMap<CompactString, IndexSet<String>>;
static EMPTY_PROPERTIES: LazyLock<Arc<PropertyValues>> =
    LazyLock::new(|| Arc::new(PropertyValues::new()));
pub type StaticUtilities = IndexMap<String, IndexMap<String, IndexSet<String>>>;
type StaticUtilityIds = IndexMap<CompactString, IndexMap<String, IdSet>>;
pub type SignatureIndex = IndexMap<String, Vec<String>>;
type DynamicUtilities = IndexMap<CompactString, IndexMap<String, IdSet>>;

struct ReplacementSet {
    revision: Arc<()>,
    unions: Vec<(CompactString, Arc<IdSet>)>,
    values: Arc<IdSet>,
}

/// Signature storage belongs to this design system for its complete lifetime.
/// Invalid signatures are cached as None and must never be treated as equivalent.
pub struct Canonicalizer {
    pub design_system: DesignSystem,
    utility_signatures: HashMap<OptionsKey, IndexMap<String, Option<Arc<str>>>>,
    utility_variables: HashMap<OptionsKey, IndexMap<String, Option<Vec<String>>>>,
    utility_properties: HashMap<OptionsKey, IndexMap<String, Arc<PropertyValues>>>,
    replacement_properties: Option<(OptionsKey, Vec<Option<Arc<PropertyValues>>>)>,
    static_utilities: HashMap<OptionsKey, StaticUtilityIds>,
    static_unions: HashMap<OptionsKey, IndexMap<CompactString, Arc<IdSet>>>,
    dynamic_utilities: HashMap<OptionsKey, IndexMap<String, Arc<DynamicUtilities>>>,
    utility_names: IndexSet<String>,
    dynamic_revision: Option<Arc<()>>,
    utility_indexes: HashMap<OptionsKey, Arc<SignatureIndex>>,
    variant_signatures: HashMap<RemKey, IndexMap<String, Option<String>>>,
    variant_indexes: HashMap<RemKey, Arc<SignatureIndex>>,
    spacing: Option<transforms::Spacing>,
    spacing_prepared: bool,
    candidate_cache: HashMap<OptionsKey, IndexMap<String, String>>,
    utility_cache: HashMap<OptionsKey, IndexMap<String, String>>,
    variant_cache: HashMap<OptionsKey, IndexMap<String, Vec<crate::candidate::Variant>>>,
    cache_prefix: HashMap<OptionsKey, String>,
    value_cache: signature::ValueCache,
    replacement_sets: HashMap<OptionsKey, IndexMap<String, ReplacementSet>>,
    line_height_names: Option<(Arc<()>, Arc<Vec<String>>)>,
    prepared_line_heights: HashMap<OptionsKey, IndexSet<(String, bool)>>,
    collapse_results: HashMap<OptionsKey, IndexMap<Vec<String>, collapse::CollapseResult>>,
}

impl Canonicalizer {
    pub fn new(design_system: DesignSystem) -> Self {
        let mut result = Self::with_options(design_system, signature::Options::default());
        result.spacing_prepared = false;
        result
    }

    pub fn with_options(design_system: DesignSystem, options: signature::Options) -> Self {
        let spacing = transforms::Spacing::new(&design_system, options.rem);
        Self {
            design_system,
            utility_signatures: HashMap::new(),
            utility_variables: HashMap::new(),
            utility_properties: HashMap::new(),
            replacement_properties: None,
            static_utilities: HashMap::new(),
            static_unions: HashMap::new(),
            dynamic_utilities: HashMap::new(),
            utility_names: IndexSet::new(),
            dynamic_revision: None,
            utility_indexes: HashMap::new(),
            variant_signatures: HashMap::new(),
            variant_indexes: HashMap::new(),
            spacing,
            spacing_prepared: true,
            candidate_cache: HashMap::new(),
            utility_cache: HashMap::new(),
            variant_cache: HashMap::new(),
            cache_prefix: HashMap::new(),
            value_cache: signature::ValueCache::default(),
            replacement_sets: HashMap::new(),
            line_height_names: None,
            prepared_line_heights: HashMap::new(),
            collapse_results: HashMap::new(),
        }
    }

    fn prepare_storage(&mut self, rem: Option<f64>) {
        if !self.spacing_prepared || self.spacing.is_none() {
            self.spacing = transforms::Spacing::new(&self.design_system, rem);
            self.spacing_prepared = true;
        }
    }

    fn prepare_signature_storage(&mut self, rem: Option<f64>) {
        // The low-level API starts with prepareDesignSystemStorage(system),
        // then creates signature options. Keep the first non-null spacing cache.
        if !self.spacing_prepared {
            self.prepare_storage(None);
        }
        self.prepare_storage(rem);
    }

    pub fn utility_signature(
        &mut self,
        utility: &str,
        options: signature::Options,
    ) -> Option<String> {
        self.shared_utility_signature(utility, options)
            .map(|value| value.to_string())
    }

    fn shared_utility_signature(
        &mut self,
        utility: &str,
        options: signature::Options,
    ) -> Option<Arc<str>> {
        self.prepare_signature_storage(options.rem);
        let key = options.into();
        if let Some(value) = self
            .utility_signatures
            .get(&key)
            .and_then(|values| values.get(utility))
        {
            return value.clone();
        }
        let value = signature::compile_utility_with_cache(
            &mut self.design_system,
            utility,
            options,
            &mut self.value_cache,
            false,
        )
        .ok()
        .map(|result| Arc::from(ast::to_css(&result.nodes)));
        self.utility_signatures
            .entry(key)
            .or_default()
            .insert(utility.into(), value.clone());
        value
    }

    pub fn utility_theme_variables(
        &mut self,
        utility: &str,
        options: signature::Options,
    ) -> Option<Vec<String>> {
        self.prepare_signature_storage(options.rem);
        let key = options.into();
        if let Some(value) = self
            .utility_variables
            .get(&key)
            .and_then(|values| values.get(utility))
        {
            return value.clone();
        }
        let value = signature::compile_utility_with_cache(
            &mut self.design_system,
            utility,
            options,
            &mut self.value_cache,
            true,
        )
        .ok()
        .map(|result| result.theme_variables);
        self.utility_variables
            .entry(key)
            .or_default()
            .insert(utility.into(), value.clone());
        value
    }

    pub fn same_utility(&mut self, left: &str, right: &str, options: signature::Options) -> bool {
        let Some(left) = self.shared_utility_signature(left, options) else {
            return false;
        };
        self.shared_utility_signature(right, options)
            .is_some_and(|right| left == right)
    }

    pub fn utility_properties(
        &mut self,
        utility: &str,
        options: signature::Options,
    ) -> PropertyValues {
        (*self.shared_utility_properties(utility, options)).clone()
    }

    fn shared_utility_properties(
        &mut self,
        utility: &str,
        options: signature::Options,
    ) -> Arc<PropertyValues> {
        self.prepare_signature_storage(options.rem);
        let key = options.into();
        if let Some(value) = self
            .utility_properties
            .get(&key)
            .and_then(|values| values.get(utility))
        {
            return value.clone();
        }
        let class = add_prefix(&self.design_system, utility);
        let mut values = PropertyValues::new();
        if let Ok(Some(compiled)) = self.design_system.first_compiled_candidate(&class, true) {
            let mut nodes: Vec<_> = compiled.iter().map(|rule| rule.node.clone()).collect();
            for node in &mut nodes {
                node.fresh_declarations();
            }
            if let Ok(canonical) = signature::canonicalize_ast_with_cache(
                nodes,
                &self.design_system.theme,
                options,
                self.design_system.compatibility_theme,
                &mut self.value_cache,
                false,
            ) {
                let static_utilities = self.static_utilities.entry(key).or_default();
                let mut class_id = self.utility_names.get_index_of(&class);
                collect_properties(
                    &canonical.nodes,
                    &class,
                    &mut values,
                    static_utilities,
                    self.static_unions.entry(key).or_default(),
                    &mut self.utility_names,
                    &mut class_id,
                );
            }
        }
        let values = if values.is_empty() {
            Arc::clone(&EMPTY_PROPERTIES)
        } else {
            Arc::new(values)
        };
        self.utility_properties
            .entry(key)
            .or_default()
            .insert(utility.into(), Arc::clone(&values));
        values
    }

    /// Includes declarations indexed as a side effect of utility_properties.
    fn static_union(&mut self, property: &str, options: signature::Options) -> Arc<IdSet> {
        let key = options.into();
        if let Some(union) = self
            .static_unions
            .get(&key)
            .and_then(|cache| cache.get(property))
        {
            return Arc::clone(union);
        }
        let union = Arc::new(
            self.static_utilities
                .get(&key)
                .and_then(|index| index.get(property))
                .into_iter()
                .flat_map(|values| values.values())
                .flatten()
                .copied()
                .collect(),
        );
        self.static_unions
            .entry(key)
            .or_default()
            .insert(property.into(), Arc::clone(&union));
        union
    }

    pub fn static_utilities(&self, options: signature::Options) -> StaticUtilities {
        self.static_utilities
            .get(&options.into())
            .into_iter()
            .flat_map(|index| index.iter())
            .map(|(property, values)| {
                (
                    property.to_string(),
                    values
                        .iter()
                        .map(|(value, ids)| {
                            (
                                value.clone(),
                                ids.iter()
                                    .map(|id| self.utility_names[*id].clone())
                                    .collect(),
                            )
                        })
                        .collect(),
                )
            })
            .collect()
    }

    pub fn utility_index(&mut self, options: signature::Options) -> SignatureIndex {
        (*self.shared_utility_index(options)).clone()
    }

    fn shared_utility_index(&mut self, options: signature::Options) -> Arc<SignatureIndex> {
        self.prepare_signature_storage(options.rem);
        let key = options.into();
        if let Some(index) = self.utility_indexes.get(&key) {
            return index.clone();
        }
        let mut index = SignatureIndex::new();
        let classes = crate::completions::class_list_with(
            self.design_system.utility_keys(Kind::Static),
            self.design_system.utility_keys(Kind::Functional),
            |root| self.design_system.utility_groups(root),
            |modifier| !numeric::is_spacing_multiplier(modifier),
        );
        for (class, metadata) in classes {
            let Some(signature) = self.utility_signature(&class, options) else {
                continue;
            };
            if class.starts_with('-')
                && class.ends_with("-0")
                && self
                    .utility_signature(&class[1..], options)
                    .is_some_and(|positive| positive == signature)
            {
                continue;
            }
            index.entry(signature).or_default().push(class.clone());
            self.shared_utility_properties(&class, options);
            for modifier in metadata.modifiers {
                let class = format!("{class}/{modifier}");
                let Some(signature) = self.utility_signature(&class, options) else {
                    continue;
                };
                index.entry(signature).or_default().push(class.clone());
                self.shared_utility_properties(&class, options);
            }
        }
        let index = Arc::new(index);
        self.utility_indexes.insert(key, Arc::clone(&index));
        index
    }

    pub fn variant_signature(&mut self, variant: &str, rem: Option<f64>) -> Option<String> {
        self.prepare_storage(None);
        let key = rem.into();
        if let Some(value) = self
            .variant_signatures
            .get(&key)
            .and_then(|values| values.get(variant))
        {
            return value.clone();
        }
        let name = add_prefix(&self.design_system, variant);
        let mut nodes = vec![rule(
            ".x",
            vec![AstNode::AtRule {
                name: "@apply".into(),
                params: format!("{name}:flex"),
                nodes: Vec::new(),
            }],
        )];
        let value = apply::substitute(&mut nodes, &mut self.design_system)
            .ok()
            .map(|_| {
                canonicalize_variant_ast(&mut nodes, rem);
                ast::to_css(&nodes)
            });
        self.variant_signatures
            .entry(key)
            .or_default()
            .insert(variant.into(), value.clone());
        value
    }

    pub fn variant_index(&mut self, rem: Option<f64>) -> SignatureIndex {
        (*self.shared_variant_index(rem)).clone()
    }

    fn shared_variant_index(&mut self, rem: Option<f64>) -> Arc<SignatureIndex> {
        let key = rem.into();
        if let Some(index) = self.variant_indexes.get(&key) {
            return index.clone();
        }
        let entries: Vec<_> = self
            .design_system
            .variants
            .entries()
            .map(|(root, definition)| (root.to_owned(), definition.kind))
            .collect();
        let mut index = SignatureIndex::new();
        for (root, kind) in entries {
            let names = match kind {
                Kind::Static => vec![root],
                Kind::Functional => self
                    .design_system
                    .variants
                    .completions(&root)
                    .into_iter()
                    .map(|value| {
                        if root == "@" {
                            format!("@{value}")
                        } else {
                            format!("{root}-{value}")
                        }
                    })
                    .collect(),
                Kind::Compound => continue,
            };
            for name in names {
                if let Some(signature) = self.variant_signature(&name, rem) {
                    index.entry(signature).or_default().push(name);
                }
            }
        }
        let index = Arc::new(index);
        self.variant_indexes.insert(key, Arc::clone(&index));
        index
    }
}

fn add_prefix(system: &DesignSystem, input: &str) -> String {
    match system
        .theme
        .prefix
        .as_deref()
        .filter(|prefix| !prefix.is_empty())
    {
        Some(prefix) if !input.starts_with(prefix) => format!("{prefix}:{input}"),
        _ => input.into(),
    }
}

fn collect_properties(
    nodes: &[AstNode],
    class: &str,
    values: &mut PropertyValues,
    index: &mut StaticUtilityIds,
    unions: &mut IndexMap<CompactString, Arc<IdSet>>,
    names: &mut IndexSet<String>,
    class_id: &mut Option<usize>,
) {
    for node in nodes {
        match node {
            AstNode::Declaration {
                property,
                value: Some(value),
                ..
            } => {
                values
                    .entry(property.clone())
                    .or_default()
                    .insert(value.clone());
                if index
                    .entry(property.clone())
                    .or_default()
                    .entry(value.clone())
                    .or_default()
                    .insert(*class_id.get_or_insert_with(|| names.insert_full(class.into()).0))
                {
                    unions.shift_remove(property.as_str());
                }
            }
            AstNode::Rule { nodes, .. }
            | AstNode::AtRule { nodes, .. }
            | AstNode::AtRoot { nodes }
            | AstNode::Context { nodes, .. } => {
                collect_properties(nodes, class, values, index, unions, names, class_id)
            }
            _ => {}
        }
    }
}

fn canonicalize_variant_ast(nodes: &mut [AstNode], rem: Option<f64>) {
    for node in nodes {
        if let AstNode::AtRule { params, .. } = node {
            *params = declaration::constant_fold(params, rem, true).replace(' ', "");
        } else if let AstNode::Rule { selector, .. } = node {
            let mut nodes = selector::parse(selector);
            let mut changed = false;
            canonicalize_selectors(&mut nodes, &mut changed);
            if changed {
                *selector = selector::to_css(&nodes, true);
            }
        }
        if let Some(children) = node.children_mut() {
            canonicalize_variant_ast(children, rem);
        }
    }
}

fn canonicalize_selectors(nodes: &mut Vec<Selector>, changed: &mut bool) {
    let mut index = 0;
    while index < nodes.len() {
        let mut replacement = None;
        match &mut nodes[index] {
            Selector::List { .. } | Selector::Combinator { .. } => *changed = true,
            Selector::Function { value, nodes } if value == ":is" => {
                if nodes.len() == 1 {
                    replacement = Some(std::mem::take(nodes));
                } else if nodes.len() == 2
                    && matches!(&nodes[0], Selector::Selector { value } if value == "*")
                    && matches!(&nodes[1], Selector::Selector { value } if value.starts_with(':'))
                {
                    replacement = Some(vec![nodes.remove(1)]);
                }
            }
            Selector::Function { value, nodes }
                if value.starts_with(':')
                    && matches!(nodes.first(), Some(Selector::Selector { value }) if value.starts_with(':')) =>
            {
                *changed = true;
                nodes.insert(0, Selector::Selector { value: "*".into() });
            }
            _ => {}
        }
        if let Some(replacement) = replacement {
            *changed = true;
            nodes.splice(index..=index, replacement);
            continue;
        }
        if let Some(children) = nodes[index].children_mut() {
            canonicalize_selectors(children, changed);
        }
        index += 1;
    }
}

mod collapse;
pub mod converter;
mod pipeline;
pub mod transforms;
pub mod variant_transforms;
pub use pipeline::CanonicalizeOptions;
