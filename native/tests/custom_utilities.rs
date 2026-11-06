use serde::Deserialize;
use tailwind_canonical_native::{
    ast::AstNode,
    canonicalize::{CanonicalizeOptions, Canonicalizer},
    compile::DesignSystem,
    completions::{ClassEntry, Group},
    custom_utilities,
    syntax::{css, escape::unescape},
    theme::Theme,
};

#[derive(Deserialize)]
struct Fixture {
    names: Vec<Name>,
    scenarios: Vec<Scenario>,
}
#[derive(Deserialize)]
struct Name {
    input: String,
    expected: bool,
}
#[derive(Deserialize)]
struct Entry {
    key: String,
    value: String,
    options: u8,
}
#[derive(Deserialize)]
struct Definition {
    name: String,
    css: String,
}
#[derive(Deserialize)]
struct Scenario {
    name: String,
    entries: Vec<Entry>,
    prefix: Option<String>,
    definitions: Vec<Definition>,
    groups: Vec<(String, Vec<Group>)>,
    classes: Vec<ClassEntry>,
    queries: Vec<Query>,
    css: Css,
    #[serde(rename = "canonicalQueries")]
    canonical_queries: Vec<CanonicalQuery>,
}
#[derive(Deserialize)]
struct Expected {
    node: AstNode,
    #[serde(rename = "propertySort")]
    property_sort: PropertySort,
}
#[derive(Deserialize)]
struct PropertySort {
    order: Vec<usize>,
    count: usize,
}
#[derive(Deserialize)]
struct Query {
    input: String,
    expected: Option<Vec<Expected>>,
    error: Option<String>,
}
#[derive(Deserialize)]
struct Css {
    input: Vec<String>,
    expected: Option<Vec<Option<String>>>,
    error: Option<String>,
}
#[derive(Deserialize)]
struct CanonicalQuery {
    input: Vec<String>,
    options: CanonicalizeOptions,
    expected: Option<Vec<String>>,
    error: Option<String>,
}

#[test]
fn css_utilities_match_official_names_resolution_suggestions_and_integration() {
    verify(include_str!("../fixtures/custom-utilities.json"), false);
}

#[test]
fn legacy_utilities_match_official_compilation_suggestions_and_custom_overlays() {
    verify(include_str!("../fixtures/legacy-utilities.json"), true);
}

fn verify(source: &str, legacy: bool) {
    let fixture: Fixture = serde_json::from_str(source).unwrap();
    for name in fixture.names {
        let value = unescape(&name.input);
        assert_eq!(
            custom_utilities::valid_functional_name(&value)
                || custom_utilities::valid_static_name(&value),
            name.expected,
            "name {:?}",
            name.input
        );
    }
    for scenario in fixture.scenarios {
        let mut theme = Theme::default();
        for entry in scenario.entries {
            theme.add(&entry.key, &entry.value, entry.options).unwrap();
        }
        theme.prefix = scenario.prefix;
        let mut system = DesignSystem::new(theme);
        if legacy {
            system.register_legacy_utilities();
        }
        for definition in scenario.definitions {
            system
                .custom_utility(&definition.name, &css::parse(&definition.css).unwrap())
                .unwrap();
        }
        for (root, groups) in scenario.groups {
            assert_eq!(
                system.utility_groups(&root),
                groups,
                "{} groups {root}",
                scenario.name
            );
        }
        assert_eq!(
            system.get_class_list(),
            scenario.classes,
            "{} classes",
            scenario.name
        );
        for (index, query) in scenario.queries.into_iter().enumerate() {
            let result = system.parse_candidate(&query.input).into_iter().try_fold(
                Vec::new(),
                |mut all, candidate| {
                    all.extend(system.compile_ast_nodes(&candidate, true)?);
                    Ok::<_, String>(all)
                },
            );
            match result {
                Err(error) => assert_eq!(
                    Some(error),
                    query.error,
                    "{} query {index} {}",
                    scenario.name,
                    query.input
                ),
                Ok(actual) => {
                    assert!(
                        query.error.is_none(),
                        "{} query {index} {} expected error {:?}",
                        scenario.name,
                        query.input,
                        query.error
                    );
                    let expected = query.expected.unwrap();
                    assert_eq!(
                        actual.iter().map(|rule| &rule.node).collect::<Vec<_>>(),
                        expected.iter().map(|rule| &rule.node).collect::<Vec<_>>(),
                        "{} query {index} {}",
                        scenario.name,
                        query.input
                    );
                    for (actual, expected) in actual.iter().zip(expected) {
                        assert_eq!(actual.property_sort.order, expected.property_sort.order);
                        assert_eq!(actual.property_sort.count, expected.property_sort.count);
                    }
                }
            }
        }
        assert_eq!(
            system.candidates_to_css(&scenario.css.input),
            scenario
                .css
                .error
                .map_or_else(|| Ok(scenario.css.expected.unwrap()), Err),
            "{} css",
            scenario.name
        );
        let mut canonicalizer = Canonicalizer::new(system);
        for query in scenario.canonical_queries {
            assert_eq!(
                canonicalizer.canonicalize(&query.input, query.options),
                query.error.map_or_else(|| Ok(query.expected.unwrap()), Err),
                "{} canonical {:?}",
                scenario.name,
                query.input
            );
        }
    }
}
