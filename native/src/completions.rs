// Port of Tailwind CSS 4.3.4 suggestion resolution and intellisense::getClassList.
// Captured data describes suggestions and namespaces, never canonicalization results.
use crate::{
    candidate::{Kind, Registry},
    compare, js_sort,
    theme::Theme,
    utilities,
};
use indexmap::{IndexMap, IndexSet};
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::sync::LazyLock;

#[derive(Deserialize)]
struct Data {
    registrations: Vec<Registration>,
}

#[derive(Deserialize)]
struct Registration {
    root: String,
    definitions: Vec<Definition>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum Definition {
    Value(String),
    Group(Description),
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Description {
    supports_negative: Option<bool>,
    #[serde(default)]
    supports_fractions: bool,
    #[serde(default)]
    values: Vec<String>,
    #[serde(default)]
    modifiers: Vec<String>,
    #[serde(default)]
    value_theme_keys: Vec<String>,
    #[serde(default)]
    modifier_theme_keys: Vec<String>,
    #[serde(default)]
    has_default_value: bool,
    has_default_theme_key: Option<String>,
    #[serde(default)]
    spacing_values: bool,
}

static DEFINITIONS: LazyLock<IndexMap<String, Vec<Definition>>> = LazyLock::new(|| {
    let data: Data = serde_json::from_str(include_str!("../data/completions.json"))
        .expect("pinned completion descriptions must be valid");
    let mut definitions: IndexMap<String, Vec<Definition>> = IndexMap::new();
    for registration in data.registrations {
        definitions
            .entry(registration.root)
            .or_default()
            .extend(registration.definitions);
    }
    definitions
});

static LEGACY_NUMERIC_KEY: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"([0-9]+)_([0-9]+)").unwrap());
static FRACTION: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[0-9]+/[0-9]+$").unwrap());
static REGISTRY: LazyLock<Registry> = LazyLock::new(|| utilities::registry(None));

pub(crate) const SPACING_VALUES: &[&str] = &[
    "0", "0.5", "1", "1.5", "2", "2.5", "3", "3.5", "4", "5", "6", "7", "8", "9", "10", "11", "12",
    "14", "16", "20", "24", "28", "32", "36", "40", "44", "48", "52", "56", "60", "64", "72", "80",
    "96",
];
const FRACTIONS: &[&str] = &[
    "1/2", "1/3", "2/3", "1/4", "2/4", "3/4", "1/5", "2/5", "3/5", "4/5", "1/6", "2/6", "3/6",
    "4/6", "5/6", "1/12", "2/12", "3/12", "4/12", "5/12", "6/12", "7/12", "8/12", "9/12", "10/12",
    "11/12",
];

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Group {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub supports_negative: Option<bool>,
    pub values: Vec<Option<String>>,
    pub modifiers: Vec<String>,
}

pub(crate) fn resolve(theme: &Theme, namespaces: &[String]) -> Vec<String> {
    theme
        .keys_in_namespaces(namespaces)
        .into_iter()
        .map(|value| LEGACY_NUMERIC_KEY.replace_all(&value, "$1.$2").into_owned())
        .collect()
}

pub(crate) fn has_registered_groups(root: &str) -> bool {
    DEFINITIONS.contains_key(root)
}

pub fn utility_groups(root: &str, theme: &Theme) -> Vec<Group> {
    let Some(definitions) = DEFINITIONS.get(root) else {
        return if REGISTRY.has_utility(root, Kind::Static) {
            vec![Group {
                supports_negative: Some(false),
                ..Group::default()
            }]
        } else {
            Vec::new()
        };
    };
    definitions
        .iter()
        .map(|definition| {
            let Definition::Group(description) = definition else {
                let Definition::Value(value) = definition else {
                    unreachable!()
                };
                return Group {
                    values: vec![Some(value.clone())],
                    ..Group::default()
                };
            };
            let mut values = description.values.clone();
            if description.spacing_values
                && theme
                    .get(&["--spacing".into()])
                    .is_some_and(|value| !value.is_empty())
            {
                values.extend(SPACING_VALUES.iter().map(|value| (*value).into()));
            }
            values.extend(resolve(theme, &description.value_theme_keys));
            if description.supports_fractions {
                values.extend(FRACTIONS.iter().map(|value| (*value).into()));
            }
            let mut values: Vec<_> = values.into_iter().map(Some).collect();
            if description.has_default_value
                || description
                    .has_default_theme_key
                    .as_ref()
                    .is_some_and(|key| theme.get(std::slice::from_ref(key)).is_some())
            {
                values.insert(0, None);
            }
            let mut modifiers = description.modifiers.clone();
            modifiers.extend(resolve(theme, &description.modifier_theme_keys));
            Group {
                supports_negative: description.supports_negative,
                values,
                modifiers,
            }
        })
        .collect()
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Metadata {
    pub modifiers: Vec<String>,
}

pub type ClassEntry = (String, Metadata);

#[derive(Default)]
struct Item {
    name: String,
    utility: String,
    fraction: bool,
    modifiers: Vec<String>,
}

fn item<'a>(items: &'a mut IndexMap<String, Item>, name: &str) -> &'a mut Item {
    items.entry(name.into()).or_insert_with(|| Item {
        name: name.into(),
        utility: name.into(),
        ..Item::default()
    })
}

pub fn class_list(theme: &Theme) -> Vec<ClassEntry> {
    class_list_with(
        utilities::keys(Kind::Static),
        utilities::keys(Kind::Functional),
        |root| utility_groups(root, theme),
        |_| true,
    )
}

pub(crate) fn class_list_with(
    static_keys: Vec<String>,
    functional_keys: Vec<String>,
    mut groups: impl FnMut(&str) -> Vec<Group>,
    include_modifier: impl Fn(&str) -> bool,
) -> Vec<ClassEntry> {
    let mut items = IndexMap::new();
    for root in static_keys {
        if groups(&root).is_empty() {
            continue;
        }
        let entry = item(&mut items, &root);
        entry.fraction = false;
        entry.modifiers.clear();
    }
    for root in functional_keys {
        for mut group in groups(&root) {
            group
                .modifiers
                .retain(|modifier| include_modifier(modifier));
            for value in group.values {
                let fraction = value.as_ref().is_some_and(|value| FRACTION.is_match(value));
                let name = value.map_or_else(|| root.clone(), |value| format!("{root}-{value}"));
                let entry = item(&mut items, &name);
                entry.utility = root.clone();
                entry.fraction |= fraction;
                entry.modifiers.extend(group.modifiers.iter().cloned());
                if group.supports_negative == Some(true) {
                    let negative = item(&mut items, &format!("-{name}"));
                    negative.utility = format!("-{root}");
                    negative.fraction |= fraction;
                    negative.modifiers.extend(group.modifiers.iter().cloned());
                }
                let entry = items.get_mut(&name).unwrap();
                entry.modifiers = entry
                    .modifiers
                    .drain(..)
                    .collect::<IndexSet<_>>()
                    .into_iter()
                    .collect();
            }
        }
    }
    let mut list: Vec<_> = items.into_values().collect();
    js_sort::sort_by(&mut list, |a, b| compare::natural(&a.name, &b.name));
    // Fractions move to the last contiguous bucket for their utility.
    let mut buckets: Vec<Vec<Item>> = Vec::new();
    let mut current: Option<String> = None;
    let mut last: IndexMap<String, usize> = IndexMap::new();
    let mut fractions: IndexMap<String, Vec<Item>> = IndexMap::new();
    for entry in list {
        if current.as_ref() != Some(&entry.utility) {
            current = Some(entry.utility.clone());
            last.insert(entry.utility.clone(), buckets.len());
            buckets.push(Vec::new());
        }
        if entry.fraction {
            fractions
                .entry(entry.utility.clone())
                .or_default()
                .push(entry);
        } else {
            buckets.last_mut().unwrap().push(entry);
        }
    }
    for (utility, entries) in fractions {
        buckets[last[&utility]].extend(entries);
    }
    buckets
        .into_iter()
        .flatten()
        .map(|entry| {
            (
                entry.name,
                Metadata {
                    modifiers: entry.modifiers,
                },
            )
        })
        .collect()
}
