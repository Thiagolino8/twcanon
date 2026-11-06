use serde::Deserialize;
use tailwind_canonical_native::{
    canonicalize::{CanonicalizeOptions, Canonicalizer},
    compile::DesignSystem,
    theme::Theme,
};

#[derive(Deserialize)]
struct Fixture {
    cases: Vec<Case>,
}

#[derive(Deserialize)]
struct Case {
    input: Vec<String>,
    official: Vec<String>,
}

#[test]
fn regex_flag_regressions_match_tailwind_canonicalization() {
    let fixture: Fixture =
        serde_json::from_str(include_str!("../fixtures/regex-limitations.json")).unwrap();
    for case in fixture.cases {
        let mut canonicalizer = Canonicalizer::new(DesignSystem::new(Theme::default()));
        let actual = canonicalizer
            .canonicalize(&case.input, CanonicalizeOptions::default())
            .unwrap();
        assert_eq!(actual, case.official, "{:?}", case.input);
    }
}
