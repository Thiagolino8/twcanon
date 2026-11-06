use indexmap::IndexSet;
use serde::Deserialize;
use tailwind_canonical_native::{
    apply,
    ast::{self, AstNode},
    compile::DesignSystem,
    signature::{self, Options},
    theme::Theme,
};

#[derive(Deserialize)]
struct Fixture {
    scenarios: Vec<Scenario>,
    error_sequences: Vec<ErrorSequence>,
}
#[derive(Deserialize)]
struct ErrorSequence {
    name: String,
    entries: Vec<Entry>,
    prefix: Option<String>,
    failed: UtilityCase,
    after_apply: ApplyCase,
    after_utility: UtilityCase,
}
#[derive(Deserialize)]
struct Scenario {
    name: String,
    entries: Vec<Entry>,
    prefix: Option<String>,
    resolve: Vec<Resolution>,
    ast: Vec<AstCase>,
    apply: Vec<ApplyCase>,
    utility: Vec<UtilityCase>,
    cache: Vec<CacheAction>,
}
#[derive(Deserialize)]
struct Entry {
    key: String,
    value: String,
    options: u8,
}
#[derive(Deserialize)]
struct Resolution {
    input: String,
    expected: String,
    inlined: Vec<String>,
}
#[derive(Deserialize)]
struct AstCase {
    input: Vec<AstNode>,
    options: Options,
    nodes: Vec<AstNode>,
    theme_variables: Vec<String>,
    css: String,
}

#[derive(Deserialize)]
struct ApplyCase {
    input: Vec<AstNode>,
    nodes: Option<Vec<AstNode>>,
    changed: Option<bool>,
    error: Option<String>,
}

#[derive(Deserialize)]
struct UtilityCase {
    input: String,
    options: Options,
    nodes: Option<Vec<AstNode>>,
    theme_variables: Option<Vec<String>>,
    css: Option<String>,
    error: Option<String>,
}

#[derive(Deserialize)]
#[serde(tag = "op", rename_all = "lowercase")]
enum CacheAction {
    Compile {
        input: String,
        respect: bool,
        fresh: bool,
        nodes: Vec<AstNode>,
    },
    Order {
        expected: Vec<(String, usize)>,
    },
    Important {
        value: bool,
    },
    Theme {
        key: String,
        value: String,
        options: u8,
    },
    Parse {
        input: String,
    },
}

#[test]
fn signatures_resolve_theme_variables_and_canonicalize_ast_like_official() {
    let fixture: Fixture =
        serde_json::from_str(include_str!("../fixtures/signatures.json")).unwrap();
    for scenario in fixture.scenarios {
        let mut theme = Theme::default();
        for entry in scenario.entries {
            theme.add(&entry.key, &entry.value, entry.options).unwrap();
        }
        theme.prefix = scenario.prefix;
        for (index, case) in scenario.resolve.into_iter().enumerate() {
            let mut inlined = IndexSet::new();
            assert_eq!(
                signature::resolve_variables(&case.input, &theme, &mut inlined),
                case.expected,
                "{} resolve {index}: {}",
                scenario.name,
                case.input
            );
            assert_eq!(
                inlined.into_iter().collect::<Vec<_>>(),
                case.inlined,
                "{} resolve {index}: inlined",
                scenario.name
            );
        }
        for (index, case) in scenario.ast.into_iter().enumerate() {
            let result = signature::canonicalize_ast(case.input, &theme, case.options);
            assert_eq!(result.nodes, case.nodes, "{} AST {index}", scenario.name);
            assert_eq!(
                result.theme_variables, case.theme_variables,
                "{} AST {index}: variables",
                scenario.name
            );
            assert_eq!(
                ast::to_css(&result.nodes),
                case.css,
                "{} AST {index}: CSS",
                scenario.name
            );
        }
        let mut cached_system = DesignSystem::new(theme.clone());
        for (index, action) in scenario.cache.into_iter().enumerate() {
            match action {
                CacheAction::Compile {
                    input,
                    respect,
                    fresh,
                    nodes,
                } => {
                    let candidate = cached_system.parse_candidate(&input).remove(0);
                    let candidate = if fresh {
                        candidate.clone_fresh()
                    } else {
                        candidate
                    };
                    let actual = cached_system
                        .compile_ast_nodes(&candidate, respect)
                        .unwrap()
                        .into_iter()
                        .map(|rule| rule.node)
                        .collect::<Vec<_>>();
                    assert_eq!(actual, nodes, "{} cache {index}", scenario.name);
                }
                CacheAction::Order { expected } => assert_eq!(
                    cached_system
                        .variant_order()
                        .unwrap()
                        .into_iter()
                        .collect::<Vec<_>>(),
                    expected,
                    "{} cache {index}: order",
                    scenario.name
                ),
                CacheAction::Important { value } => cached_system.important = value,
                CacheAction::Theme {
                    key,
                    value,
                    options,
                } => cached_system.theme.add(&key, &value, options).unwrap(),
                CacheAction::Parse { input } => {
                    cached_system.parse_variant(&input);
                }
            }
        }
        let mut system = DesignSystem::new(theme);
        for (index, case) in scenario.apply.into_iter().enumerate() {
            let mut nodes = case.input;
            let result = apply::substitute(&mut nodes, &mut system);
            if let Some(expected) = case.nodes {
                assert_eq!(
                    result.unwrap(),
                    case.changed.unwrap(),
                    "{} apply {index}: features",
                    scenario.name
                );
                assert_eq!(nodes, expected, "{} apply {index}: AST", scenario.name);
            } else {
                assert_eq!(
                    result.unwrap_err(),
                    case.error.unwrap(),
                    "{} apply {index}: error",
                    scenario.name
                );
            }
        }
        for (index, case) in scenario.utility.into_iter().enumerate() {
            let result = signature::compile_utility(&mut system, &case.input, case.options);
            if let Some(expected) = case.nodes {
                let result = result.unwrap_or_else(|error| {
                    panic!("{} utility {index}: {}: {error}", scenario.name, case.input)
                });
                assert_eq!(
                    result.nodes, expected,
                    "{} utility {index}: {}",
                    scenario.name, case.input
                );
                assert_eq!(
                    result.theme_variables,
                    case.theme_variables.unwrap(),
                    "{} utility {index}: variables",
                    scenario.name
                );
                assert_eq!(
                    ast::to_css(&result.nodes),
                    case.css.unwrap(),
                    "{} utility {index}: CSS",
                    scenario.name
                );
            } else {
                assert_eq!(
                    result.unwrap_err(),
                    case.error.unwrap(),
                    "{} utility {index}: error",
                    scenario.name
                );
            }
        }
    }
}

#[test]
fn invalid_apply_stops_before_later_candidates_contaminate_compilation_cache() {
    let fixture: Fixture =
        serde_json::from_str(include_str!("../fixtures/signatures.json")).unwrap();
    for sequence in fixture.error_sequences {
        let mut theme = Theme::default();
        for entry in sequence.entries {
            theme.add(&entry.key, &entry.value, entry.options).unwrap();
        }
        theme.prefix = sequence.prefix;
        let mut system = DesignSystem::new(theme);
        assert_eq!(
            signature::compile_utility(
                &mut system,
                &sequence.failed.input,
                sequence.failed.options
            )
            .unwrap_err(),
            sequence.failed.error.unwrap(),
            "{} failure",
            sequence.name
        );
        let mut ast = sequence.after_apply.input;
        let changed = apply::substitute(&mut ast, &mut system).unwrap();
        assert_eq!(
            changed,
            sequence.after_apply.changed.unwrap(),
            "{} after_apply features",
            sequence.name
        );
        assert_eq!(
            ast,
            sequence.after_apply.nodes.unwrap(),
            "{} after_apply AST",
            sequence.name
        );
        let result = signature::compile_utility(
            &mut system,
            &sequence.after_utility.input,
            sequence.after_utility.options,
        )
        .unwrap();
        assert_eq!(
            result.nodes,
            sequence.after_utility.nodes.unwrap(),
            "{} after_utility AST",
            sequence.name
        );
        assert_eq!(
            result.theme_variables,
            sequence.after_utility.theme_variables.unwrap(),
            "{} after_utility variables",
            sequence.name
        );
        assert_eq!(
            ast::to_css(&result.nodes),
            sequence.after_utility.css.unwrap(),
            "{} after_utility CSS",
            sequence.name
        );
    }
}
