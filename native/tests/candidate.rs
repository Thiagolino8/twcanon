use serde::Deserialize;
use serde_json::Value;
use tailwind_canonical_native::{
    candidate::{self, Registry},
    print,
};

#[derive(Deserialize)]
struct Fixture {
    registry: Registry,
    cases: Vec<Case>,
}

#[derive(Deserialize)]
struct Case {
    kind: String,
    input: String,
    prefix: Option<String>,
    expected: Value,
}

#[test]
fn full_candidate_parser_matches_official_tailwind() {
    let Fixture {
        mut registry,
        cases,
    } = serde_json::from_str(include_str!("../fixtures/candidate.json")).unwrap();
    for case in cases {
        registry.prefix = case.prefix;
        let actual = match case.kind.as_str() {
            "candidate" => serde_json::to_value(candidate::parse(&case.input, &registry)).unwrap(),
            "variant" => {
                serde_json::to_value(candidate::parse_variant(&case.input, &registry)).unwrap()
            }
            "printCandidate" => serde_json::to_value(
                candidate::parse(&case.input, &registry)
                    .iter()
                    .map(|c| print::candidate(c, registry.prefix.as_deref()))
                    .collect::<Vec<_>>(),
            )
            .unwrap(),
            "printVariant" => serde_json::to_value(
                candidate::parse_variant(&case.input, &registry)
                    .as_ref()
                    .map(print::variant),
            )
            .unwrap(),
            other => panic!("Unknown fixture kind: {other}"),
        };
        assert_eq!(actual, case.expected, "{}: {:?}", case.kind, case.input);
    }
}
