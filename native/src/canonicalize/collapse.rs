// Port of collapseCandidates/collapseGroup from Tailwind CSS 4.3.4.
use super::{Canonicalizer, DynamicUtilities, IdMap, IdSet, PropertyValues};
use crate::{
    candidate::{CandidateKind, Kind},
    compare::Collation,
    js_sort, numeric, print, signature,
    syntax::segment::segment,
};
use indexmap::{IndexMap, IndexSet};
use std::{cmp::Ordering, sync::Arc};

pub(super) struct CollapseResult {
    replacements: Vec<Arc<IdSet>>,
    class_order: IndexMap<String, Option<usize>>,
    registry_revision: Arc<()>,
    theme_revision: Arc<()>,
    ignored_options: u8,
    prefix: Option<String>,
    important: bool,
    compatibility_theme: bool,
    result: Vec<String>,
    plan: Arc<CollapsePlan>,
}

#[derive(Default)]
struct CollapsePlan {
    other_utilities: Vec<IdSet>,
    unique: IndexSet<Vec<usize>>,
    linked: Vec<usize>,
}

fn intersection(left: &IdSet, right: &IdSet) -> IdSet {
    let (small, large) = if left.len() <= right.len() {
        (left, right)
    } else {
        (right, left)
    };
    small
        .iter()
        .filter(|item| large.contains(*item))
        .cloned()
        .collect()
}
fn compare_order(left: Option<usize>, right: Option<usize>) -> Ordering {
    match (left, right) {
        (None, None) => Ordering::Equal,
        (None, _) => Ordering::Greater,
        (_, None) => Ordering::Less,
        (Some(left), Some(right)) => left.cmp(&right),
    }
}
fn same_variables(left: Option<Vec<String>>, right: Option<Vec<String>>) -> bool {
    match (left, right) {
        (Some(left), Some(right)) if left.len() == right.len() => left
            .iter()
            .zip(right)
            .all(|(left, right)| left.is_empty() || *left == right),
        _ => false,
    }
}

/// Descending cardinality, then ascending bit-mask order, without a word-size limit.
struct Combinations {
    size: usize,
    positions: Option<Vec<usize>>,
}
impl Combinations {
    fn new(size: usize) -> Self {
        Self {
            size,
            positions: (size >= 2).then(|| (0..size).collect()),
        }
    }
}
impl Iterator for Combinations {
    type Item = Vec<usize>;
    fn next(&mut self) -> Option<Self::Item> {
        let current = self.positions.as_mut()?;
        let result = current.clone();
        if let Some(index) = (0..current.len()).find(|&index| {
            current[index] + 1 < current.get(index + 1).copied().unwrap_or(self.size)
        }) {
            current[index] += 1;
            for (position, value) in current.iter_mut().enumerate().take(index) {
                *value = position;
            }
        } else {
            self.positions = (current.len() > 2).then(|| (0..current.len() - 1).collect());
        }
        Some(result)
    }
}

impl Canonicalizer {
    pub(super) fn collapse_candidates(
        &mut self,
        candidates: Vec<String>,
        options: signature::Options,
    ) -> Result<Vec<String>, String> {
        let prefix = self
            .design_system
            .theme
            .prefix
            .as_ref()
            .filter(|p| !p.is_empty())
            .map(|p| format!("{p}:"))
            .unwrap_or_default();
        let mut groups = IndexMap::<String, IndexMap<String, IndexSet<String>>>::new();
        for candidate in candidates {
            let mut variants = segment(&candidate, b':');
            let utility = variants.pop().unwrap();
            let (utility, after) = utility
                .strip_suffix('!')
                .map(|s| (s, "!"))
                .unwrap_or((utility, ""));
            let before = if variants.is_empty() {
                String::new()
            } else {
                format!("{}:", variants.join(":"))
            };
            groups
                .entry(before)
                .or_default()
                .entry(after.into())
                .or_default()
                .insert(format!("{prefix}{utility}"));
        }
        let mut result = IndexSet::new();
        for (before, groups) in groups {
            for (after, candidates) in groups {
                for candidate in self.collapse_group(candidates.into_iter().collect(), options)? {
                    let candidate = if prefix.is_empty() {
                        &candidate
                    } else {
                        candidate.strip_prefix(&prefix).unwrap_or(&candidate)
                    };
                    result.insert(format!("{before}{candidate}{after}"));
                }
            }
        }
        Ok(result.into_iter().collect())
    }

    fn collapse_group(
        &mut self,
        candidates: Vec<String>,
        options: signature::Options,
    ) -> Result<Vec<String>, String> {
        let candidate_properties: Vec<_> = candidates
            .iter()
            .map(|candidate| self.shared_utility_properties(candidate, options))
            .collect();
        self.precompute_line_heights(&candidate_properties, options);
        let mut other_utilities = Vec::new();
        for (index, properties) in candidate_properties.iter().enumerate() {
            other_utilities.push(self.replacements(&candidates[index], properties, options));
        }
        // Preparing the replacements above can populate indexes and register
        // utilities. Preserve those effects even when no pair can collapse.
        if candidates.len() < 2 {
            return Ok(candidates);
        }
        let key = options.into();
        let (theme_revision, ignored_options) = self.design_system.theme.value_revision();
        let cached_plan = if let Some(cached) = self
            .collapse_results
            .get(&key)
            .and_then(|results| results.get(&candidates))
            && Arc::ptr_eq(
                &cached.registry_revision,
                self.design_system.utility_registry_revision(),
            )
            && Arc::ptr_eq(&cached.theme_revision, theme_revision)
            && cached.ignored_options == ignored_options
            && cached.prefix == self.design_system.theme.prefix
            && cached.important == self.design_system.important
            && cached.compatibility_theme == self.design_system.compatibility_theme
            && cached.replacements.len() == other_utilities.len()
            && cached
                .replacements
                .iter()
                .zip(&other_utilities)
                .all(|(left, right)| Arc::ptr_eq(left, right))
        {
            Some(Arc::clone(&cached.plan))
        } else {
            None
        };
        let replacement_sets = other_utilities;
        let plan = cached_plan.unwrap_or_else(|| {
            Arc::new(self.collapse_plan(&candidate_properties, &replacement_sets, options))
        });
        let mut orderable = Vec::new();
        for index in &plan.linked {
            if self
                .design_system
                .get_class_order(&[candidates[*index].clone()])
                .is_ok()
            {
                orderable.push(candidates[*index].clone());
            }
        }
        let class_order: IndexMap<_, _> = if plan.linked.is_empty() {
            IndexMap::new()
        } else {
            self.design_system
                .get_class_order(&orderable)?
                .into_iter()
                .collect()
        };
        // All index preparation and class-order calls above must still run:
        // they populate caches observed by later operations on the design system.
        if let Some(cached) = self
            .collapse_results
            .get(&key)
            .and_then(|results| results.get(&candidates))
            && Arc::ptr_eq(&cached.plan, &plan)
            && cached.class_order == class_order
        {
            return Ok(cached.result.clone());
        }
        let result: Vec<_> = if plan.linked.is_empty() {
            candidates.clone()
        } else {
            let mut result: IndexSet<_> = candidates.iter().cloned().collect();
            let mut drop = IndexSet::new();
            let collation = Collation::english();
            for indexes in &plan.unique {
                for positions in Combinations::new(indexes.len()) {
                    let combo: Vec<_> = positions
                        .into_iter()
                        .map(|position| indexes[position])
                        .collect();
                    if combo.iter().any(|index| drop.contains(&candidates[*index])) {
                        continue;
                    }
                    // JS flatMap does not flatten Set, so reduce intersects one Set per index.
                    let mut potential = plan.other_utilities[combo[0]].clone();
                    for index in &combo[1..] {
                        potential = intersection(&potential, &plan.other_utilities[*index]);
                    }
                    if potential.is_empty() {
                        continue;
                    }
                    let mut collapsed: Vec<_> = combo
                        .iter()
                        .map(|index| candidates[*index].clone())
                        .collect();
                    js_sort::sort_by(&mut collapsed, |a, b| collation.compare(a, b));
                    let collapsed = collapsed.join(" ");
                    let collapsed_signature = self.shared_utility_signature(&collapsed, options);
                    let mut ordered: Vec<_> = combo
                        .iter()
                        .map(|index| candidates[*index].clone())
                        .collect();
                    js_sort::sort_by(&mut ordered, |a, b| {
                        compare_order(
                            class_order.get(b).copied().flatten(),
                            class_order.get(a).copied().flatten(),
                        )
                    });
                    let mut replacements: IndexSet<_> = ordered.into_iter().collect();
                    replacements.extend(
                        potential
                            .into_iter()
                            .map(|id| self.utility_names[id].clone()),
                    );
                    for replacement in replacements {
                        let signature = self.shared_utility_signature(&replacement, options);
                        if !(replacement == collapsed
                            || signature.is_some() && signature == collapsed_signature)
                        {
                            continue;
                        }
                        if !same_variables(
                            self.utility_theme_variables(&collapsed, options),
                            self.utility_theme_variables(&replacement, options),
                        ) {
                            continue;
                        }
                        result.insert(replacement.clone());
                        for index in &combo {
                            if candidates[*index] != replacement {
                                drop.insert(candidates[*index].clone());
                            }
                        }
                        break;
                    }
                }
            }
            result.retain(|item| !drop.contains(item));
            result.into_iter().collect()
        };
        let (theme_revision, ignored_options) = self.design_system.theme.value_revision();
        self.collapse_results.entry(key).or_default().insert(
            candidates,
            CollapseResult {
                replacements: replacement_sets,
                class_order,
                registry_revision: Arc::clone(self.design_system.utility_registry_revision()),
                theme_revision: Arc::clone(theme_revision),
                ignored_options,
                prefix: self.design_system.theme.prefix.clone(),
                important: self.design_system.important,
                compatibility_theme: self.design_system.compatibility_theme,
                result: result.clone(),
                plan,
            },
        );
        Ok(result)
    }

    fn collapse_plan(
        &mut self,
        candidate_properties: &[Arc<PropertyValues>],
        replacement_sets: &[Arc<IdSet>],
        options: signature::Options,
    ) -> CollapsePlan {
        let entries: usize = replacement_sets.iter().map(|values| values.len()).sum();
        let other_utilities = if entries < self.utility_names.len().div_ceil(1024) {
            self.sparse_collapse_replacements(candidate_properties, replacement_sets, options)
        } else {
            self.dense_collapse_replacements(candidate_properties, replacement_sets, options)
        };
        let mut linked = IdMap::<IdSet>::default();
        for i in 0..other_utilities.len() {
            for j in i + 1..other_utilities.len() {
                if other_utilities[i]
                    .iter()
                    .any(|replacement| other_utilities[j].contains(replacement))
                {
                    linked
                        .entry(i)
                        .or_insert_with(|| [i].into_iter().collect())
                        .insert(j);
                    linked
                        .entry(j)
                        .or_insert_with(|| [j].into_iter().collect())
                        .insert(i);
                }
            }
        }
        if linked.is_empty() {
            return CollapsePlan::default();
        }
        let mut unique = IndexSet::new();
        for group in linked.values() {
            let mut sorted: Vec<_> = group.iter().copied().collect();
            sorted.sort_unstable();
            unique.insert(sorted);
        }
        CollapsePlan {
            other_utilities,
            unique,
            linked: linked.into_keys().collect(),
        }
    }

    fn sparse_collapse_replacements(
        &mut self,
        candidate_properties: &[Arc<PropertyValues>],
        replacement_sets: &[Arc<IdSet>],
        options: signature::Options,
    ) -> Vec<IdSet> {
        let mut seen = IdSet::default();
        let mut shared = IdSet::default();
        for replacements in replacement_sets {
            for replacement in replacements.iter() {
                if !seen.insert(*replacement) {
                    shared.insert(*replacement);
                }
            }
        }
        if shared.is_empty() {
            return Vec::new();
        }
        let mut covered = IdMap::<IndexSet<&str>>::default();
        for (index, replacements) in replacement_sets.iter().enumerate() {
            for replacement in replacements.iter() {
                if shared.contains(replacement) {
                    covered.entry(*replacement).or_default().extend(
                        candidate_properties[index]
                            .keys()
                            .map(|property| property.as_str()),
                    );
                }
            }
        }
        let mut possible = IdMap::default();
        replacement_sets
            .iter()
            .map(|replacements| {
                replacements
                    .iter()
                    .copied()
                    .filter(|replacement| {
                        shared.contains(replacement)
                            && *possible.entry(*replacement).or_insert_with(|| {
                                let properties = self.replacement_properties(*replacement, options);
                                properties.keys().all(|property| {
                                    covered
                                        .get(replacement)
                                        .is_some_and(|values| values.contains(property.as_str()))
                                })
                            })
                    })
                    .collect()
            })
            .collect()
    }

    fn dense_collapse_replacements(
        &mut self,
        candidate_properties: &[Arc<PropertyValues>],
        replacement_sets: &[Arc<IdSet>],
        options: signature::Options,
    ) -> Vec<IdSet> {
        let mut counts = vec![0_u8; self.utility_names.len()];
        for replacements in replacement_sets {
            for replacement in replacements.iter() {
                counts[*replacement] = (counts[*replacement] + 1).min(2);
            }
        }
        if !counts.contains(&2) {
            return Vec::new();
        }
        // Only membership is observed here: transpose the candidate/property
        // coverage into bitsets instead of cloning property names per utility.
        let mut covered = IndexMap::<&str, Vec<u64>>::new();
        for (index, replacements) in replacement_sets.iter().enumerate() {
            for property in candidate_properties[index].keys() {
                let ids = covered
                    .entry(property)
                    .or_insert_with(|| vec![0; counts.len().div_ceil(64)]);
                for replacement in replacements.iter() {
                    if counts[*replacement] == 2 {
                        ids[*replacement / 64] |= 1 << (*replacement % 64);
                    }
                }
            }
        }
        let mut possible = vec![None; counts.len()];
        replacement_sets
            .iter()
            .map(|replacements| {
                replacements
                    .iter()
                    .copied()
                    .filter(|replacement| {
                        if counts[*replacement] != 2 {
                            return false;
                        }
                        if let Some(possible) = possible[*replacement] {
                            return possible;
                        }
                        let properties = self.replacement_properties(*replacement, options);
                        let value = properties.keys().all(|property| {
                            covered.get(property.as_str()).is_some_and(|ids| {
                                ids[*replacement / 64] & (1 << (*replacement % 64)) != 0
                            })
                        });
                        possible[*replacement] = Some(value);
                        value
                    })
                    .collect()
            })
            .collect()
    }

    fn replacement_properties(
        &mut self,
        id: usize,
        options: signature::Options,
    ) -> Arc<PropertyValues> {
        self.prepare_signature_storage(options.rem);
        let key = options.into();
        if let Some((cached_key, values)) = &self.replacement_properties
            && *cached_key == key
            && let Some(Some(value)) = values.get(id)
        {
            return Arc::clone(value);
        }
        let name = self.utility_names[id].clone();
        let value = self.shared_utility_properties(&name, options);
        let (cached_key, values) = self
            .replacement_properties
            .get_or_insert_with(|| (key, Vec::new()));
        if *cached_key != key {
            *cached_key = key;
            values.clear();
        }
        values.resize_with(values.len().max(id + 1), || None);
        values[id] = Some(Arc::clone(&value));
        value
    }

    fn precompute_line_heights(
        &mut self,
        properties: &[std::sync::Arc<PropertyValues>],
        options: signature::Options,
    ) {
        if !properties.iter().any(|p| p.contains_key("line-height")) {
            return;
        }
        let revision = self.design_system.theme.value_revision().0;
        let names = if let Some((cached_revision, names)) = &self.line_height_names
            && Arc::ptr_eq(cached_revision, revision)
        {
            Arc::clone(names)
        } else {
            let names = Arc::new(
                self.design_system
                    .theme
                    .keys_in_namespaces(&["--text".into()]),
            );
            self.line_height_names = Some((Arc::clone(revision), Arc::clone(&names)));
            self.prepared_line_heights.clear();
            names
        };
        if names.is_empty() {
            return;
        }
        let mut interesting = IndexSet::new();
        let mut seen = IndexSet::new();
        for pairs in properties {
            for value in pairs.get("line-height").into_iter().flatten() {
                if !seen.insert(value.clone()) {
                    continue;
                }
                let Some(bare) = self.spacing.as_mut().and_then(|s| s.get(value)) else {
                    continue;
                };
                let bare = numeric::format(bare);
                let named = numeric::is_spacing_multiplier(&bare);
                let line_height = if named { bare } else { value.clone() };
                interesting.insert((line_height.clone(), named));
                if self
                    .prepared_line_heights
                    .entry(options.into())
                    .or_default()
                    .insert((line_height.clone(), named))
                {
                    for name in names.iter() {
                        self.shared_utility_properties(
                            &format!(
                                "text-{name}/{}",
                                if named {
                                    line_height.clone()
                                } else {
                                    format!("[{line_height}]")
                                }
                            ),
                            options,
                        );
                    }
                }
            }
        }
        let mut seen = IndexSet::new();
        for pairs in properties {
            for value in pairs.get("font-size").into_iter().flatten() {
                if !seen.insert(value.clone()) {
                    continue;
                }
                for (height, _) in &interesting {
                    self.shared_utility_properties(
                        &format!(
                            "text-[{value}]/{}",
                            if numeric::is_spacing_multiplier(height) {
                                height.clone()
                            } else {
                                format!("[{height}]")
                            }
                        ),
                        options,
                    );
                }
            }
        }
    }

    fn replacements(
        &mut self,
        candidate: &str,
        properties: &PropertyValues,
        options: signature::Options,
    ) -> std::sync::Arc<IdSet> {
        let key = options.into();
        let revision = std::sync::Arc::clone(self.design_system.utility_registry_revision());
        if let Some(cached) = self
            .replacement_sets
            .get(&key)
            .and_then(|cache| cache.get(candidate))
            && std::sync::Arc::ptr_eq(&cached.revision, &revision)
            && cached.unions.iter().all(|(property, union)| {
                self.static_unions
                    .get(&key)
                    .and_then(|unions| unions.get(property))
                    .is_some_and(|current| std::sync::Arc::ptr_eq(current, union))
            })
        {
            return std::sync::Arc::clone(&cached.values);
        }
        let mut unions = Vec::new();
        let mut dynamic = None;
        let mut result = None;
        for (property, values) in properties {
            let union = self.static_union(property, options);
            let mut others = (*union).clone();
            unions.push((property.clone(), union));
            for value in values {
                let dynamic =
                    dynamic.get_or_insert_with(|| self.dynamic_utilities(candidate, options));
                if let Some(group) = dynamic.get(property).and_then(|lookup| lookup.get(value)) {
                    others.extend(group.iter().copied());
                }
            }
            result = Some(match result {
                None => others,
                Some(ref previous) => intersection(previous, &others),
            });
            if result.as_ref().is_some_and(IdSet::is_empty) {
                break;
            }
        }
        let result = result.unwrap_or_default();
        // A union can change without changing this intersection. Retain the
        // old identity only when both membership and insertion order match.
        let result = self
            .replacement_sets
            .get(&key)
            .and_then(|cache| cache.get(candidate))
            .filter(|cached| cached.values.iter().eq(result.iter()))
            .map(|cached| std::sync::Arc::clone(&cached.values))
            .unwrap_or_else(|| std::sync::Arc::new(result));
        self.replacement_sets.entry(key).or_default().insert(
            candidate.into(),
            super::ReplacementSet {
                revision,
                unions,
                values: std::sync::Arc::clone(&result),
            },
        );
        result
    }

    fn dynamic_utilities(
        &mut self,
        candidate: &str,
        options: signature::Options,
    ) -> std::sync::Arc<DynamicUtilities> {
        let revision = self.design_system.utility_registry_revision();
        if !self
            .dynamic_revision
            .as_ref()
            .is_some_and(|cached| std::sync::Arc::ptr_eq(cached, revision))
        {
            self.dynamic_utilities.clear();
            self.dynamic_revision = Some(std::sync::Arc::clone(revision));
        }
        let key = options.into();
        if let Some(cached) = self
            .dynamic_utilities
            .get(&key)
            .and_then(|cache| cache.get(candidate))
        {
            return std::sync::Arc::clone(cached);
        }
        let result = std::sync::Arc::new(self.compute_dynamic_utilities(candidate, options));
        self.dynamic_utilities
            .entry(key)
            .or_default()
            .insert(candidate.into(), std::sync::Arc::clone(&result));
        result
    }

    fn compute_dynamic_utilities(
        &mut self,
        candidate: &str,
        options: signature::Options,
    ) -> DynamicUtilities {
        let mut result = DynamicUtilities::new();
        let relevant: IndexSet<_> = self
            .shared_utility_properties(candidate, options)
            .keys()
            .cloned()
            .collect();
        if relevant.is_empty() {
            return result;
        }
        let input = if let Some(prefix) = self
            .design_system
            .theme
            .prefix
            .as_deref()
            .filter(|p| !p.is_empty())
            && !candidate.starts_with(&format!("{prefix}:"))
        {
            format!("{prefix}:{candidate}")
        } else {
            candidate.into()
        };
        for parsed in self.design_system.parse_candidate(&input) {
            let CandidateKind::Functional {
                root: original,
                value: Some(_),
                ..
            } = &parsed.kind
            else {
                continue;
            };
            let original = original.clone();
            let (template, Some(root_range)) = print::candidate_with_root(&parsed, None) else {
                unreachable!();
            };
            let before = &template[..root_range.start];
            let after = &template[root_range.end..];
            for root in self.design_system.utility_keys(Kind::Functional) {
                if root == original {
                    continue;
                }
                let mut replacement =
                    String::with_capacity(before.len() + root.len() + after.len());
                replacement.push_str(before);
                replacement.push_str(&root);
                replacement.push_str(after);
                let properties = self.shared_utility_properties(&replacement, options);
                if !properties
                    .keys()
                    .any(|property| relevant.contains(property))
                {
                    continue;
                }
                let replacement_id = self.utility_names.insert_full(replacement).0;
                for (property, values) in properties.iter() {
                    if !relevant.contains(property) {
                        continue;
                    }
                    for value in values {
                        result
                            .entry(property.clone())
                            .or_default()
                            .entry(value.clone())
                            .or_default()
                            .insert(replacement_id);
                    }
                }
            }
            return result;
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::Combinations;
    #[test]
    fn combinations_follow_upstream_mask_order() {
        assert_eq!(
            Combinations::new(4).collect::<Vec<_>>(),
            vec![
                vec![0, 1, 2, 3],
                vec![0, 1, 2],
                vec![0, 1, 3],
                vec![0, 2, 3],
                vec![1, 2, 3],
                vec![0, 1],
                vec![0, 2],
                vec![1, 2],
                vec![0, 3],
                vec![1, 3],
                vec![2, 3]
            ]
        );
        let mut large = Combinations::new(70);
        assert_eq!(large.next().unwrap().len(), 70);
        assert_eq!(large.next().unwrap(), (0..69).collect::<Vec<_>>());
    }
}
