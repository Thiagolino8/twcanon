use serde::Deserialize;
use serde_json::Value;
use tailwind_canonical_native::{
    ast, print,
    syntax::{arbitrary, attribute, css, escape, segment, selector, value},
};

#[derive(Deserialize)]
struct Fixture {
    cases: Vec<Case>,
}

#[derive(Deserialize)]
struct Case {
    kind: String,
    input: String,
    separator: Option<String>,
    expected: Value,
}

#[test]
fn syntax_matches_official_tailwind_on_upstream_inputs_and_fuzz_cases() {
    let fixture: Fixture = serde_json::from_str(include_str!("../fixtures/syntax.json")).unwrap();
    for case in fixture.cases {
        let actual = match case.kind.as_str() {
            "value" => serde_json::to_value(value::parse(&case.input)).unwrap(),
            "attribute" => serde_json::to_value(attribute::parse(&case.input)).unwrap(),
            "valid" => Value::Bool(arbitrary::is_valid(&case.input)),
            "decode" => Value::String(arbitrary::decode(&case.input)),
            "printArbitrary" => Value::String(print::arbitrary(&case.input)),
            "selector" => serde_json::to_value(selector::parse(&case.input)).unwrap(),
            "selectorCss" => Value::String(selector::to_css(&selector::parse(&case.input), false)),
            "selectorMinified" => {
                Value::String(selector::to_css(&selector::parse(&case.input), true))
            }
            "escape" => Value::String(escape::escape(&case.input)),
            "unescape" => Value::String(escape::unescape(&case.input)),
            "css" | "cssPrinted" => match css::parse(&case.input) {
                Ok(nodes) if case.kind == "css" => serde_json::to_value(nodes).unwrap(),
                Ok(nodes) => Value::String(ast::to_css(&nodes)),
                Err(error) => serde_json::json!({"error":error.message}),
            },
            "segment" => serde_json::to_value(segment::segment(
                &case.input,
                case.separator.unwrap().as_bytes()[0],
            ))
            .unwrap(),
            other => panic!("Unknown fixture kind: {other}"),
        };
        assert_eq!(actual, case.expected, "{}: {:?}", case.kind, case.input);
    }
}
