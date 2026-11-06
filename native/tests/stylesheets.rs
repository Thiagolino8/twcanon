use serde::Deserialize;
use serde_json::Value;
use tailwind_canonical_native::{ast, syntax::css};

#[derive(Deserialize)]
struct Fixture {
    cases: Vec<Case>,
}
#[derive(Deserialize)]
struct Case {
    source: String,
    input: String,
    expected: Value,
    printed: String,
}

#[test]
fn official_tailwind_stylesheets_match_official_parser() {
    let fixture: Fixture =
        serde_json::from_str(include_str!("../fixtures/stylesheets.json")).unwrap();
    for case in fixture.cases {
        let nodes = css::parse(&case.input).unwrap_or_else(|e| panic!("{}: {e}", case.source));
        assert_eq!(
            serde_json::to_value(&nodes).unwrap(),
            case.expected,
            "AST: {}",
            case.source
        );
        assert_eq!(ast::to_css(&nodes), case.printed, "CSS: {}", case.source);
    }
}
