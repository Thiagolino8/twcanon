use serde::Deserialize;
use tailwind_canonical_native::{ast::AstNode, css_functions, theme::Theme};

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
struct Scenario {
    name: String,
    entries: Vec<Entry>,
    prefix: Option<String>,
    cases: Vec<Case>,
    asts: Vec<AstCase>,
}
#[derive(Deserialize)]
struct Case {
    input: String,
    at_rule: bool,
    expected: Option<String>,
    error: Option<String>,
}
#[derive(Deserialize)]
struct AstCase {
    input: Vec<AstNode>,
    expected: Option<Vec<AstNode>>,
    error: Option<String>,
    changed: Option<bool>,
}

#[test]
fn css_functions_match_official_values_errors_and_ast_contexts() {
    let fixture: Fixture =
        serde_json::from_str(include_str!("../fixtures/css-functions.json")).unwrap();
    for scenario in fixture.scenarios {
        let mut theme = Theme::default();
        for entry in scenario.entries {
            theme.add(&entry.key, &entry.value, entry.options).unwrap();
        }
        theme.prefix = scenario.prefix;
        for case in scenario.cases {
            let actual = css_functions::substitute_in_value(&case.input, case.at_rule, &theme);
            let expected = case.expected.ok_or_else(|| case.error.unwrap());
            assert_eq!(
                actual, expected,
                "{}: {:?}, at_rule={}",
                scenario.name, case.input, case.at_rule
            );
        }
        for case in scenario.asts {
            let mut nodes = case.input;
            let actual = css_functions::substitute(&mut nodes, &theme);
            match case.expected {
                Some(expected) => {
                    assert_eq!(actual, Ok(case.changed.unwrap()), "{}", scenario.name);
                    assert_eq!(nodes, expected, "{}", scenario.name);
                }
                None => assert_eq!(actual, Err(case.error.unwrap()), "{}", scenario.name),
            }
        }
    }
}
