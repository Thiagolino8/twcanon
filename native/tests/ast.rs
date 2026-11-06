use serde::Deserialize;
use tailwind_canonical_native::{
    ast::{self, AstNode},
    compile::DesignSystem,
    nesting, optimize,
    theme::Theme,
};

#[derive(Deserialize)]
struct Fixture {
    node_pool: Vec<serde_json::Value>,
    theme_states: Vec<serde_json::Value>,
    scenarios: Vec<Scenario>,
    variables: Vec<Variable>,
    apis: Vec<ApiScenario>,
}
#[derive(Deserialize)]
struct Variable {
    input: String,
    expected: Vec<String>,
}
#[derive(Deserialize)]
struct Entry {
    key: String,
    value: String,
    options: u8,
}
#[derive(Deserialize)]
struct Scenario {
    name: String,
    entries: Vec<Entry>,
    prefix: Option<String>,
    #[serde(default)]
    customs: Vec<Custom>,
    cases: Vec<Case>,
}
#[derive(Deserialize)]
struct Custom {
    name: String,
    css: String,
}
#[derive(Deserialize)]
struct ApiScenario {
    name: String,
    entries: Vec<Entry>,
    prefix: Option<String>,
    customs: Vec<Custom>,
    cases: Vec<ApiCase>,
}
#[derive(Deserialize)]
struct ApiCase {
    input: Vec<String>,
    expected: Option<Vec<Vec<usize>>>,
    css: Option<Vec<Option<String>>>,
    order: Option<Vec<(String, Option<usize>)>>,
    theme: Option<usize>,
    error: Option<String>,
}
#[derive(Deserialize)]
struct Case {
    kind: String,
    input: serde_json::Value,
    expected: Option<Vec<usize>>,
    css: Option<String>,
    error: Option<String>,
    polyfills: Option<u8>,
    theme: Option<usize>,
}

fn stored_node(id: usize, pool: &[serde_json::Value]) -> serde_json::Value {
    let mut node = pool[id].clone();
    if let Some(children) = node.get_mut("nodes") {
        *children = serde_json::Value::Array(
            children
                .as_array()
                .unwrap()
                .iter()
                .map(|child| stored_node(child.as_u64().unwrap() as usize, pool))
                .collect(),
        );
    }
    node
}

fn read_ast(ids: &[usize], pool: &[serde_json::Value]) -> Vec<AstNode> {
    ids.iter()
        .map(|id| serde_json::from_value(stored_node(*id, pool)).unwrap())
        .collect()
}

#[test]
fn optimization_matches_official_theme_dependencies_polyfills_and_compiled_output() {
    let fixture: Fixture = serde_json::from_str(include_str!("../fixtures/ast.json")).unwrap();
    for scenario in fixture.scenarios {
        for (index, case) in scenario.cases.into_iter().enumerate() {
            if case.kind == "nesting" {
                continue;
            }
            let mut theme = Theme::default();
            for entry in &scenario.entries {
                theme.add(&entry.key, &entry.value, entry.options).unwrap();
            }
            theme.prefix = scenario.prefix.clone();
            let actual = if case.kind == "compiled" {
                let input: Vec<String> = serde_json::from_value(case.input).unwrap();
                let mut system = DesignSystem::new(theme);
                for custom in &scenario.customs {
                    system.custom_variant(
                        &custom.name,
                        &tailwind_canonical_native::syntax::css::parse(&custom.css).unwrap(),
                    );
                }
                let result = system
                    .compile_candidates(&input, true)
                    .and_then(|compiled| {
                        optimize::optimize_ast(
                            compiled.nodes.into_iter().map(|node| node.node).collect(),
                            &mut system.theme,
                            case.polyfills.unwrap(),
                        )
                    });
                theme = system.theme;
                result
            } else {
                optimize::optimize_ast(
                    read_ast(
                        &serde_json::from_value::<Vec<usize>>(case.input).unwrap(),
                        &fixture.node_pool,
                    ),
                    &mut theme,
                    case.polyfills.unwrap(),
                )
            };
            if let Some(expected) = case.expected {
                let expected = read_ast(&expected, &fixture.node_pool);
                let actual = actual
                    .unwrap_or_else(|error| panic!("{} case {index}: {error}", scenario.name));
                assert_eq!(actual, expected, "{} case {index}", scenario.name);
                assert_eq!(
                    ast::to_css(&actual),
                    case.css.unwrap(),
                    "{} case {index}: CSS",
                    scenario.name
                );
                assert_eq!(
                    serde_json::to_value(theme.entries().unwrap()).unwrap(),
                    fixture.theme_states[case.theme.unwrap()],
                    "{} case {index}: theme",
                    scenario.name
                );
            } else {
                assert_eq!(
                    actual.unwrap_err(),
                    case.error.unwrap(),
                    "{} case {index}: error",
                    scenario.name
                );
            }
        }
    }
}

#[test]
fn design_system_ast_css_and_class_order_match_official_api_with_customs() {
    let fixture: Fixture = serde_json::from_str(include_str!("../fixtures/ast.json")).unwrap();
    for scenario in fixture.apis {
        for (index, case) in scenario.cases.into_iter().enumerate() {
            let mut theme = Theme::default();
            for entry in &scenario.entries {
                theme.add(&entry.key, &entry.value, entry.options).unwrap();
            }
            theme.prefix = scenario.prefix.clone();
            let mut system = DesignSystem::new(theme);
            for custom in &scenario.customs {
                system.custom_variant(
                    &custom.name,
                    &tailwind_canonical_native::syntax::css::parse(&custom.css).unwrap(),
                );
            }
            let actual = system.candidates_to_ast(&case.input);
            if let Some(expected) = case.expected {
                let expected: Vec<_> = expected
                    .iter()
                    .map(|ids| read_ast(ids, &fixture.node_pool))
                    .collect();
                assert_eq!(
                    actual.unwrap(),
                    expected,
                    "{} case {index}: AST",
                    scenario.name
                );
                assert_eq!(
                    system.candidates_to_css(&case.input).unwrap(),
                    case.css.unwrap(),
                    "{} case {index}: CSS",
                    scenario.name
                );
                assert_eq!(
                    system.get_class_order(&case.input).unwrap(),
                    case.order.unwrap(),
                    "{} case {index}: order",
                    scenario.name
                );
                assert_eq!(
                    serde_json::to_value(system.theme.entries().unwrap()).unwrap(),
                    fixture.theme_states[case.theme.unwrap()],
                    "{} case {index}: theme",
                    scenario.name
                );
            } else {
                assert_eq!(
                    actual.unwrap_err(),
                    case.error.unwrap(),
                    "{} case {index}: error",
                    scenario.name
                );
            }
        }
    }
}

#[test]
fn variable_references_match_official_parser_including_fallback_words_and_duplicates() {
    let fixture: Fixture = serde_json::from_str(include_str!("../fixtures/ast.json")).unwrap();
    for case in fixture.variables {
        assert_eq!(
            optimize::extract_used_variables(&case.input),
            case.expected,
            "{}",
            case.input
        );
    }
}

#[test]
fn nesting_matches_official_selectors_scopes_hoisting_and_deduplication() {
    let fixture: Fixture = serde_json::from_str(include_str!("../fixtures/ast.json")).unwrap();
    for scenario in fixture.scenarios {
        for (index, case) in scenario.cases.into_iter().enumerate() {
            if case.kind != "nesting" {
                continue;
            }
            let input = read_ast(
                &serde_json::from_value::<Vec<usize>>(case.input).unwrap(),
                &fixture.node_pool,
            );
            let actual = nesting::handle_nesting(input);
            if let Some(expected) = case.expected {
                let expected = read_ast(&expected, &fixture.node_pool);
                let actual = actual
                    .unwrap_or_else(|error| panic!("{} case {index}: {error}", scenario.name));
                assert_eq!(actual, expected, "{} case {index}", scenario.name);
                assert_eq!(
                    ast::to_css(&actual),
                    case.css.unwrap(),
                    "{} case {index}: CSS",
                    scenario.name
                );
            } else {
                assert_eq!(
                    actual.unwrap_err(),
                    case.error.unwrap(),
                    "{} case {index}: error",
                    scenario.name
                );
            }
        }
    }
}
