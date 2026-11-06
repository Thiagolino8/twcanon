use serde::Deserialize;
use serde_json::Value;
use tailwind_canonical_native::{ast::AstNode, compile::DesignSystem, syntax::css, theme::Theme};

#[derive(Deserialize)]
struct Fixture {
    scenarios: Vec<Scenario>,
}
#[derive(Deserialize)]
struct Entry {
    key: String,
    value: String,
    options: u8,
}
#[derive(Deserialize)]
struct Custom {
    name: String,
    css: String,
}
#[derive(Deserialize)]
struct Scenario {
    name: String,
    entries: Vec<Entry>,
    prefix: Option<String>,
    important: bool,
    customs: Vec<Custom>,
    prime_variants: Vec<String>,
    invalid_candidates: Vec<String>,
    cases: Vec<Case>,
}
#[derive(Deserialize)]
struct Case {
    input: Vec<String>,
    respect_important: bool,
    prime_variants: Vec<String>,
    expected: Option<Value>,
    error: Option<String>,
}

#[test]
fn compile_candidates_matches_official_ast_metadata_invalids_and_stateful_order() {
    let fixture: Fixture =
        serde_json::from_str(include_str!("../fixtures/compilation.json")).unwrap();
    for scenario in fixture.scenarios {
        let mut theme = Theme::default();
        for entry in scenario.entries {
            theme.add(&entry.key, &entry.value, entry.options).unwrap();
        }
        theme.prefix = scenario.prefix;
        let mut system = DesignSystem::new(theme);
        system.important = scenario.important;
        for custom in scenario.customs {
            system.custom_variant(&custom.name, &css::parse(&custom.css).unwrap());
        }
        for name in scenario.prime_variants {
            system.parse_variant(&name);
        }
        system
            .invalid_candidates
            .extend(scenario.invalid_candidates);
        for (index, case) in scenario.cases.into_iter().enumerate() {
            for name in case.prime_variants {
                system.parse_variant(&name);
            }
            let actual = system.compile_candidates(&case.input, case.respect_important);
            match case.expected {
                Some(expected) => {
                    let actual = serde_json::to_value(actual.unwrap_or_else(|error| {
                        panic!("{} batch {index}: {error}", scenario.name)
                    }))
                    .unwrap();
                    // Compare records one at a time to keep any failure readable.
                    assert_eq!(
                        actual["invalid"], expected["invalid"],
                        "{} batch {index}: invalid",
                        scenario.name
                    );
                    assert_eq!(
                        actual["nodes"].as_array().unwrap().len(),
                        expected["nodes"].as_array().unwrap().len(),
                        "{} batch {index}: nodes",
                        scenario.name
                    );
                    for (record, (actual, expected)) in actual["nodes"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .zip(expected["nodes"].as_array().unwrap())
                        .enumerate()
                    {
                        // The JS walker can leave fields from a former node kind
                        // on an at-root object. Compare the documented AST shape.
                        let actual_node: AstNode =
                            serde_json::from_value(actual["node"].clone()).unwrap();
                        let expected_node: AstNode =
                            serde_json::from_value(expected["node"].clone()).unwrap();
                        assert_eq!(
                            actual_node, expected_node,
                            "{} batch {index} record {record}: AST",
                            scenario.name
                        );
                        for field in ["properties", "variants", "candidate"] {
                            assert_eq!(
                                actual[field], expected[field],
                                "{} batch {index} record {record}: {field}",
                                scenario.name
                            );
                        }
                    }
                }
                None => assert_eq!(actual.err(), case.error, "{} batch {index}", scenario.name),
            }
        }
    }
}
