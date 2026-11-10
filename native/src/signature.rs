// Port of Tailwind CSS 4.3.4 canonicalize-candidates.ts signature AST passes.
// See LICENSE.tailwind.
use crate::{
    apply,
    ast::AstNode,
    ast_arena::{Arena, GroupId, NodeId},
    compare::Collation,
    compile::DesignSystem,
    css_functions, declaration, js_sort, print,
    syntax::value::{self, Value},
    theme::Theme,
};
use indexmap::IndexSet;
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::{
    cmp::Ordering,
    collections::HashMap,
    sync::{Arc, LazyLock},
};

pub const EXPAND_PROPERTIES: u8 = 1;
pub const LOGICAL_TO_PHYSICAL: u8 = 2;

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize)]
pub struct Options {
    pub rem: Option<f64>,
    pub features: u8,
}

#[derive(Debug, Serialize)]
pub struct CanonicalAst {
    pub nodes: Vec<AstNode>,
    pub theme_variables: Vec<String>,
}

static HEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)#(?:[a-f0-9]{8}|[a-f0-9]{6}|[a-f0-9]{4}|[a-f0-9]{3})")
        .expect("constant hex color regex")
});
static COLLATION: LazyLock<Collation> = LazyLock::new(Collation::english);

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct ValueOptions {
    rem: Option<u64>,
    custom_property: bool,
    compatibility_theme: bool,
}

struct NormalizedValue {
    value: String,
    inlined: IndexSet<String>,
}

#[derive(Default)]
pub(crate) struct ValueCache {
    revision: Option<(Arc<()>, u8)>,
    prefix: Option<String>,
    values: HashMap<ValueOptions, HashMap<String, Arc<NormalizedValue>>>,
}

impl ValueCache {
    fn prepare(&mut self, theme: &Theme) {
        let (revision, ignored) = theme.value_revision();
        if self
            .revision
            .as_ref()
            .is_none_or(|(cached, mask)| !Arc::ptr_eq(cached, revision) || *mask != ignored)
            || self.prefix != theme.prefix
        {
            self.values.clear();
            self.revision = Some((Arc::clone(revision), ignored));
            self.prefix.clone_from(&theme.prefix);
        }
    }
}

fn text(node: &Value) -> &str {
    match node {
        Value::Word { value } | Value::Separator { value } | Value::Function { value, .. } => value,
    }
}

/// Resolve only theme lookups whose fallback matches the theme value exactly.
/// The seen set belongs to the complete value, including repeated occurrences.
pub fn resolve_variables(input: &str, theme: &Theme, inlined: &mut IndexSet<String>) -> String {
    resolve_variables_with_options(input, theme, inlined, false)
        .expect("the core theme resolver does not return errors")
}

fn resolve_variables_with_options(
    input: &str,
    theme: &Theme,
    inlined: &mut IndexSet<String>,
    compatibility_theme: bool,
) -> Result<String, String> {
    fn walk(
        nodes: &mut Vec<Value>,
        theme: &Theme,
        seen: &mut IndexSet<String>,
        inlined: &mut IndexSet<String>,
        changed: &mut bool,
        compatibility_theme: bool,
    ) -> Result<(), String> {
        let mut i = 0;
        while i < nodes.len() {
            let mut replacement = None;
            if let Value::Function { value, nodes } = &mut nodes[i]
                && value == "var"
                && (nodes.len() == 1 || nodes.len() >= 3)
            {
                let mut variable = text(&nodes[0]).to_owned();
                if let Some(prefix) = theme.prefix.as_deref().filter(|prefix| !prefix.is_empty())
                    && let Some(unprefixed) = variable.strip_prefix(&format!("--{prefix}-"))
                {
                    variable = unprefixed.into();
                }
                // The upstream resolves before checking/adding the seen entry.
                let resolved = if compatibility_theme {
                    crate::compat_theme::resolve(theme, &variable, true)?
                } else {
                    css_functions::resolve_theme_value(theme, &variable, true)
                };
                if seen.insert(variable.clone())
                    && let Some(resolved) = resolved
                    && !matches!(
                        resolved.to_lowercase().as_str(),
                        "initial" | "inherit" | "revert" | "revert-layer" | "revert-rule" | "unset"
                    )
                {
                    if nodes.len() == 1 {
                        *changed = true;
                        nodes.extend(value::parse(&format!(",{resolved}")));
                    }
                    if nodes.len() >= 3
                        && value::to_css(nodes) == format!("{},{resolved}", text(&nodes[0]))
                    {
                        *changed = true;
                        inlined.insert(variable);
                        replacement = Some(value::parse(&resolved));
                    }
                }
            }
            if let Some(replacement) = replacement {
                nodes.splice(i..=i, replacement);
                continue;
            }
            if let Value::Function { nodes, .. } = &mut nodes[i] {
                walk(nodes, theme, seen, inlined, changed, compatibility_theme)?;
            }
            i += 1;
        }
        Ok(())
    }
    let mut nodes = value::parse(input);
    let mut changed = false;
    walk(
        &mut nodes,
        theme,
        &mut IndexSet::new(),
        inlined,
        &mut changed,
        compatibility_theme,
    )?;
    if changed {
        Ok(value::to_css(&nodes))
    } else {
        Ok(input.into())
    }
}

struct Canonicalizer<'a> {
    arena: Arena,
    theme: &'a Theme,
    options: Options,
    compatibility_theme: bool,
    track_theme_variables: bool,
    collation: &'static Collation,
    inlined: HashMap<NodeId, IndexSet<String>>,
    values: &'a mut ValueCache,
}

impl Canonicalizer<'_> {
    fn normalize_value(
        &mut self,
        input: &str,
        custom_property: bool,
    ) -> Result<Arc<NormalizedValue>, String> {
        let key = ValueOptions {
            rem: self.options.rem.map(f64::to_bits),
            custom_property,
            compatibility_theme: self.compatibility_theme,
        };
        let cache = self.values.values.entry(key).or_default();
        if let Some(value) = cache.get(input) {
            return Ok(Arc::clone(value));
        }
        let mut inlined = IndexSet::new();
        let mut value = if input.contains("var(") {
            resolve_variables_with_options(
                input,
                self.theme,
                &mut inlined,
                self.compatibility_theme,
            )?
        } else {
            input.into()
        };
        let mut parsed = value::parse(&value);
        let folded = declaration::constant_fold_ast(&mut parsed, self.options.rem, true);
        let normalized = declaration::canonicalize_calc_ast(&mut parsed, self.collation);
        value = if folded || normalized {
            // Reparse serialized replacements: adjacent AST tokens can coalesce.
            print::arbitrary(&value::to_css(&parsed))
        } else {
            print::arbitrary_ast(&mut parsed)
        };
        if !custom_property && value.contains('#') {
            value = HEX
                .replace_all(&value, |capture: &regex::Captures<'_>| {
                    capture[0].to_ascii_lowercase()
                })
                .into_owned();
        }
        let value = Arc::new(NormalizedValue { value, inlined });
        cache.insert(input.into(), Arc::clone(&value));
        Ok(value)
    }

    // The common utility body has independent declarations and needs no graph.
    // Keep the sequential walk: redundancy observes earlier normalized siblings
    // and later siblings that have not been normalized or expanded yet.
    fn flat_rule(&mut self, node: AstNode) -> Result<CanonicalAst, String> {
        let AstNode::Rule {
            selector,
            mut nodes,
        } = node
        else {
            unreachable!()
        };
        let mut inlined = HashMap::new();
        let mut index = 0;
        while index < nodes.len() {
            let AstNode::Declaration {
                property,
                value,
                important,
                ..
            } = &nodes[index]
            else {
                unreachable!()
            };
            let Some(value) = value else {
                nodes.remove(index);
                continue;
            };
            let redundant = property.strip_prefix("--tw-").is_some_and(|tail| {
                let target = match property.as_str() {
                    "--tw-duration" => "transition-duration",
                    "--tw-ease" => "transition-timing-function",
                    "--tw-leading" => "line-height",
                    "--tw-tracking" => "letter-spacing",
                    _ => tail,
                };
                nodes.iter().any(|sibling| {
                    matches!(
                        sibling,
                        AstNode::Declaration { property, value: Some(sibling_value), important: sibling_important, .. }
                            if property == target && sibling_value == value && sibling_important == important
                    )
                })
            });
            if property == "--tw-sort" || redundant {
                nodes.remove(index);
                continue;
            }
            if self.options.features & EXPAND_PROPERTIES != 0
                && let Some(expansion) = declaration::expand(
                    &nodes[index],
                    self.options.features & LOGICAL_TO_PHYSICAL != 0,
                )
            {
                nodes.splice(index..=index, expansion);
                continue;
            }
            let custom_property = property.starts_with("--");
            let AstNode::Declaration {
                identity, value, ..
            } = &mut nodes[index]
            else {
                unreachable!()
            };
            let normalized = self.normalize_value(&value.take().unwrap(), custom_property)?;
            *value = Some(normalized.value.clone());
            if self.track_theme_variables && !normalized.inlined.is_empty() {
                inlined.insert(identity.0, normalized);
            }
            index += 1;
        }
        let mut seen = IndexSet::new();
        let mut keep = vec![true; nodes.len()];
        for (index, node) in nodes.iter().enumerate().rev() {
            let AstNode::Declaration { property, .. } = node else {
                unreachable!()
            };
            keep[index] = seen.insert(property.clone());
        }
        let mut position = 0;
        nodes.retain(|_| {
            let retained = keep[position];
            position += 1;
            retained
        });
        js_sort::sort_by(&mut nodes, |a, b| match (a, b) {
            (
                AstNode::Declaration { property: a, .. },
                AstNode::Declaration { property: b, .. },
            ) => self.collation.compare(a, b),
            _ => unreachable!(),
        });
        let theme_variables = if self.track_theme_variables {
            nodes
                .iter()
                .map(|node| {
                    let AstNode::Declaration { identity, .. } = node else {
                        unreachable!()
                    };
                    let mut names: Vec<_> = inlined
                        .get(&identity.0)
                        .into_iter()
                        .flat_map(|value| value.inlined.iter().map(String::as_str))
                        .collect();
                    names.sort_by(|a, b| a.encode_utf16().cmp(b.encode_utf16()));
                    names.join(" ")
                })
                .collect()
        } else {
            Vec::new()
        };
        Ok(CanonicalAst {
            nodes: vec![AstNode::Rule { selector, nodes }],
            theme_variables,
        })
    }

    fn group(&mut self, group: GroupId) -> Result<(), String> {
        let mut index = 0;
        while index < self.arena.groups[group].len() {
            let id = self.arena.groups[group][index];
            match &self.arena.nodes[id].data {
                AstNode::Declaration {
                    property,
                    value,
                    important,
                    ..
                } => {
                    let Some(value) = value else {
                        self.arena.groups[group].remove(index);
                        continue;
                    };
                    let redundant = property.strip_prefix("--tw-").is_some_and(|tail| {
                        let target = match property.as_str() {
                            "--tw-duration" => "transition-duration",
                            "--tw-ease" => "transition-timing-function",
                            "--tw-leading" => "line-height",
                            "--tw-tracking" => "letter-spacing",
                            _ => tail,
                        };
                        self.arena.groups[group].iter().any(|&sibling| {
                            matches!(
                                &self.arena.nodes[sibling].data,
                                AstNode::Declaration { property, value: Some(sibling_value), important: sibling_important, .. }
                                    if property == target && sibling_value == value && sibling_important == important
                            )
                        })
                    });
                    if property == "--tw-sort" || redundant {
                        self.arena.groups[group].remove(index);
                        continue;
                    }
                    if self.options.features & EXPAND_PROPERTIES != 0
                        && let Some(expansion) = declaration::expand(
                            &self.arena.nodes[id].data,
                            self.options.features & LOGICAL_TO_PHYSICAL != 0,
                        )
                    {
                        let replacement: Vec<_> = expansion
                            .into_iter()
                            .map(|node| self.arena.import_node(node))
                            .collect();
                        self.arena.groups[group].splice(index..=index, replacement);
                        continue;
                    }
                    let custom_property = property.starts_with("--");
                    let AstNode::Declaration { value, .. } = &mut self.arena.nodes[id].data else {
                        unreachable!()
                    };
                    let value = value.take().unwrap();
                    let value = self.normalize_value(&value, custom_property)?;
                    if self.track_theme_variables && !value.inlined.is_empty() {
                        self.inlined
                            .entry(id)
                            .or_default()
                            .extend(value.inlined.iter().cloned());
                    }
                    if let AstNode::Declaration { value: target, .. } =
                        &mut self.arena.nodes[id].data
                    {
                        *target = Some(value.value.clone());
                    }
                }
                AstNode::Context { .. } | AstNode::AtRoot { .. } => {
                    let children = self.arena.nodes[id].children.unwrap();
                    let replacement = self.arena.groups[children].clone();
                    self.arena.groups[group].splice(index..=index, replacement);
                    continue;
                }
                AstNode::Comment { .. } => {
                    self.arena.groups[group].remove(index);
                    continue;
                }
                AstNode::AtRule { name, .. } if name == "@property" => {
                    self.arena.groups[group].remove(index);
                    continue;
                }
                AstNode::Rule { .. } | AstNode::AtRule { .. } => {
                    let children = self.arena.nodes[id].children.unwrap();
                    self.group(children)?;
                    let mut seen = IndexSet::new();
                    let mut keep = vec![true; self.arena.groups[children].len()];
                    for (index, &child) in self.arena.groups[children].iter().enumerate().rev() {
                        if let AstNode::Declaration {
                            property,
                            value: Some(_),
                            ..
                        } = &self.arena.nodes[child].data
                        {
                            keep[index] = seen.insert(property.clone());
                        }
                    }
                    let mut position = 0;
                    self.arena.groups[children].retain(|_| {
                        let retained = keep[position];
                        position += 1;
                        retained
                    });
                    js_sort::sort_by(&mut self.arena.groups[children], |&a, &b| {
                        match (&self.arena.nodes[a].data, &self.arena.nodes[b].data) {
                            (
                                AstNode::Declaration { property: a, .. },
                                AstNode::Declaration { property: b, .. },
                            ) => self.collation.compare(a, b),
                            _ => Ordering::Equal,
                        }
                    });
                }
            }
            index += 1;
        }
        Ok(())
    }

    fn variables(&self, group: GroupId, output: &mut Vec<String>) {
        for &id in &self.arena.groups[group] {
            if matches!(self.arena.nodes[id].data, AstNode::Declaration { .. }) {
                let mut names: Vec<_> = self.inlined.get(&id).into_iter().flatten().collect();
                names.sort_by(|a, b| a.encode_utf16().cmp(b.encode_utf16()));
                output.push(
                    names
                        .into_iter()
                        .map(String::as_str)
                        .collect::<Vec<_>>()
                        .join(" "),
                );
            }
            if let Some(children) = self.arena.nodes[id].children {
                self.variables(children, output);
            }
        }
    }
}

pub fn canonicalize_ast(nodes: Vec<AstNode>, theme: &Theme, options: Options) -> CanonicalAst {
    canonicalize_ast_with_options(nodes, theme, options, false)
        .expect("the core theme resolver does not return errors")
}

pub(crate) fn canonicalize_ast_with_options(
    nodes: Vec<AstNode>,
    theme: &Theme,
    options: Options,
    compatibility_theme: bool,
) -> Result<CanonicalAst, String> {
    canonicalize_ast_with_cache(
        nodes,
        theme,
        options,
        compatibility_theme,
        &mut ValueCache::default(),
        true,
    )
}

pub(crate) fn canonicalize_ast_with_cache(
    mut nodes: Vec<AstNode>,
    theme: &Theme,
    options: Options,
    compatibility_theme: bool,
    values: &mut ValueCache,
    track_theme_variables: bool,
) -> Result<CanonicalAst, String> {
    values.prepare(theme);
    let mut canonicalizer = Canonicalizer {
        arena: Arena::default(),
        theme,
        options,
        compatibility_theme,
        track_theme_variables,
        collation: &COLLATION,
        inlined: HashMap::new(),
        values,
    };
    // Bound the alias check and leave all larger or graph-shaped ASTs on the
    // general path. Repeated identities must retain shared-object semantics.
    if let [AstNode::Rule { nodes: children, .. }] = nodes.as_slice()
        && children.len() <= 16
        && children.iter().enumerate().all(|(index, node)| {
            let AstNode::Declaration { identity, .. } = node else {
                return false;
            };
            !children[..index].iter().any(|previous| {
                matches!(previous, AstNode::Declaration { identity: previous, .. } if previous.0 == identity.0)
            })
        })
    {
        return canonicalizer.flat_rule(nodes.pop().unwrap());
    }
    let root = canonicalizer.arena.import(nodes);
    canonicalizer.group(root)?;
    let mut theme_variables = Vec::new();
    if track_theme_variables {
        canonicalizer.variables(root, &mut theme_variables);
    }
    Ok(CanonicalAst {
        nodes: canonicalizer.arena.into_export(root),
        theme_variables,
    })
}

pub fn compile_utility(
    system: &mut DesignSystem,
    utility: &str,
    options: Options,
) -> Result<CanonicalAst, String> {
    compile_utility_with_cache(system, utility, options, &mut ValueCache::default(), true)
}

pub(crate) fn compile_utility_with_cache(
    system: &mut DesignSystem,
    utility: &str,
    options: Options,
    values: &mut ValueCache,
    track_theme_variables: bool,
) -> Result<CanonicalAst, String> {
    let utility = match system
        .theme
        .prefix
        .as_deref()
        .filter(|prefix| !prefix.is_empty())
    {
        Some(prefix) if !utility.starts_with(prefix) => format!("{prefix}:{utility}"),
        _ => utility.into(),
    };
    let inline = system.theme.disable_inline();
    let result = (|| {
        for candidate in system.parse_candidate(&utility) {
            system.shared_compile_ast_nodes(&candidate, true)?;
        }
        apply::substitute_utility(&utility, system)
    })();
    system.theme.restore_inline(inline);
    let nodes = result?;
    canonicalize_ast_with_cache(
        nodes,
        &system.theme,
        options,
        system.compatibility_theme,
        values,
        track_theme_variables,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::rule;

    #[test]
    fn flat_declarations_match_the_arena_walk_and_value_cache_effects() {
        let mut theme = Theme::default();
        theme.add("--color-brand", "#ABCDEF", 0).unwrap();
        theme.add("--spacing", "0.25rem", 0).unwrap();
        let properties = [
            "--tw-sort",
            "color",
            "padding",
            "--tw-color",
            "--tw-leading",
            "line-height",
            "--tw-duration",
            "transition-duration",
            "margin-inline",
            "gap",
            "--custom",
            "width",
            "border-inline",
            "--tw-width",
        ];
        let values = [
            "#ABCDEF",
            "1rem",
            "1rem 2rem",
            "calc(1px + 2px)",
            "var(--color-brand)",
            "var(--spacing)",
            "var(--tw-colors-brand)",
            "var(--tw-borderColor)",
            "var(--missing, red)",
            "var(--x)px",
            "calc(0px * -1)",
            "a_\\_b",
            "",
            "200ms",
            "1 2 3 4",
            "#ABCDEFff",
        ];
        for prefix in [None, Some("tw".into())] {
            theme.prefix = prefix;
            for compatibility in [false, true] {
                for features in 0..=3 {
                    for rem in [None, Some(16.0)] {
                        let options = Options { rem, features };
                        let mut direct_cache = ValueCache::default();
                        let mut arena_cache = ValueCache::default();
                        for seed in 0..256 {
                            let mut random = seed + 1_u32;
                            let mut declarations = Vec::new();
                            for _ in 0..seed % 17 {
                                random = random.wrapping_mul(1664525).wrapping_add(1013904223);
                                let property = properties[random as usize % properties.len()];
                                random = random.wrapping_mul(1664525).wrapping_add(1013904223);
                                let mut node =
                                    AstNode::decl(property, values[random as usize % values.len()]);
                                if let AstNode::Declaration {
                                    important, value, ..
                                } = &mut node
                                {
                                    *important = random & 4 != 0;
                                    if random.is_multiple_of(31) {
                                        *value = None;
                                    }
                                }
                                declarations.push(node);
                            }
                            if seed.is_multiple_of(7)
                                && let Some(shared) = declarations.first().cloned()
                            {
                                declarations.push(shared);
                            }
                            let nodes = vec![rule(".x", declarations)];
                            arena_cache.prepare(&theme);
                            let mut arena = Canonicalizer {
                                arena: Arena::default(),
                                theme: &theme,
                                options,
                                compatibility_theme: compatibility,
                                track_theme_variables: true,
                                collation: &COLLATION,
                                inlined: HashMap::new(),
                                values: &mut arena_cache,
                            };
                            let root = arena.arena.import(nodes.clone());
                            let expected = arena.group(root).map(|()| {
                                let mut variables = Vec::new();
                                arena.variables(root, &mut variables);
                                (arena.arena.into_export(root), variables)
                            });
                            let actual = canonicalize_ast_with_cache(
                                nodes.clone(),
                                &theme,
                                options,
                                compatibility,
                                &mut direct_cache,
                                false,
                            );
                            assert_eq!(
                                actual.as_ref().map(|result| &result.nodes),
                                expected.as_ref().map(|(nodes, _)| nodes),
                            );
                            if let Ok(result) = actual {
                                assert!(result.theme_variables.is_empty());
                            }
                            let actual = canonicalize_ast_with_cache(
                                nodes,
                                &theme,
                                options,
                                compatibility,
                                &mut direct_cache,
                                true,
                            )
                            .map(|result| (result.nodes, result.theme_variables));
                            assert_eq!(
                                actual, expected,
                                "seed={seed}, features={features}, rem={rem:?}, compat={compatibility}"
                            );
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn value_memoization_preserves_aliases_and_observes_theme_and_option_changes() {
        let mut theme = Theme::default();
        theme
            .add("--color-brand", "#ABCDEF", crate::theme::INLINE)
            .unwrap();
        let mut modified = theme.clone();
        modified.add("--color-brand", "#123ABC", 0).unwrap();
        let mut replacement = Theme::default();
        replacement.add("--color-brand", "#F00", 0).unwrap();
        let mut cache = ValueCache::default();
        for mut theme in [theme, modified, replacement] {
            for prefix in [None, Some("tw".into()), None] {
                theme.prefix = prefix;
                for compatibility in [false, true] {
                    for rem in [Some(16.0), Some(10.0), None] {
                        let declaration = AstNode::decl("color", "var(--color-brand)");
                        let nodes = vec![
                            rule(
                                ".a",
                                vec![declaration.clone(), AstNode::decl("width", "1rem")],
                            ),
                            rule(
                                ".b",
                                vec![declaration, AstNode::decl("--custom", "#ABCDEF")],
                            ),
                        ];
                        let options = Options {
                            rem,
                            features: EXPAND_PROPERTIES,
                        };
                        let expected = canonicalize_ast_with_options(
                            nodes.clone(),
                            &theme,
                            options,
                            compatibility,
                        )
                        .map(|result| (crate::ast::to_css(&result.nodes), result.theme_variables));
                        let actual = canonicalize_ast_with_cache(
                            nodes.clone(),
                            &theme,
                            options,
                            compatibility,
                            &mut cache,
                            false,
                        )
                        .map(|result| crate::ast::to_css(&result.nodes));
                        assert_eq!(actual.as_ref(), expected.as_ref().map(|(css, _)| css));
                        let actual = canonicalize_ast_with_cache(
                            nodes,
                            &theme,
                            options,
                            compatibility,
                            &mut cache,
                            true,
                        )
                        .map(|result| (crate::ast::to_css(&result.nodes), result.theme_variables));
                        assert_eq!(actual, expected);
                    }
                }
            }
            theme.clear_namespace("--color", 0);
            let nodes = vec![rule(
                ".x",
                vec![AstNode::decl("color", "var(--color-brand)")],
            )];
            let actual = canonicalize_ast_with_cache(
                nodes,
                &theme,
                Options::default(),
                false,
                &mut cache,
                true,
            )
            .unwrap();
            assert_eq!(actual.theme_variables, [""]);
            assert!(crate::ast::to_css(&actual.nodes).contains("var(--color-brand)"));
        }
    }
}
