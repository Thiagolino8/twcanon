use serde::Deserialize;
use tailwind_canonical_native::{
    ast::AstNode,
    breakpoints::{self, Direction},
    candidate::{Candidate, CandidateKind, Kind, parse},
    theme::Theme,
    utilities,
};

#[derive(Deserialize)]
struct Fixture {
    scenarios: Vec<Scenario>,
    alpha_cases: Vec<AlphaCase>,
    breakpoint_cases: Vec<BreakpointCase>,
    pending: Vec<Pending>,
}
#[derive(Deserialize)]
struct Scenario {
    entries: Vec<Entry>,
    prefix: Option<String>,
    cases: Vec<Case>,
}
#[derive(Deserialize)]
struct Entry {
    key: String,
    value: String,
    options: u8,
}
#[derive(Deserialize)]
struct Case {
    candidate: Candidate,
    expected: Vec<Vec<AstNode>>,
}
#[derive(Deserialize)]
struct AlphaCase {
    alpha: String,
    with_alpha: String,
    replace_alpha: String,
}
#[derive(Deserialize)]
struct Pending {
    root: String,
    kind: Kind,
}

#[derive(Deserialize)]
struct BreakpointCase {
    left: String,
    right: String,
    direction: Direction,
    expected: i8,
}

#[test]
fn breakpoint_order_matches_javascript_numeric_and_utf16_comparison() {
    let fixture: Fixture =
        serde_json::from_str(include_str!("../fixtures/utilities.json")).unwrap();
    for case in fixture.breakpoint_cases {
        let actual = breakpoints::compare(&case.left, &case.right, case.direction);
        assert_eq!(
            actual as i8, case.expected,
            "{} / {}",
            case.left, case.right
        );
    }
}

#[test]
fn utility_bodies_match_official_compile_functions_across_themes() {
    let fixture: Fixture =
        serde_json::from_str(include_str!("../fixtures/utilities.json")).unwrap();
    for (scenario_index, scenario) in fixture.scenarios.into_iter().enumerate() {
        let mut theme = Theme::default();
        for entry in scenario.entries {
            theme.add(&entry.key, &entry.value, entry.options).unwrap();
        }
        theme.prefix = scenario.prefix.clone();
        let registry = utilities::registry(scenario.prefix);
        for case in scenario.cases {
            assert!(
                parse(&case.candidate.raw, &registry).contains(&case.candidate),
                "registry: {:?}",
                case.candidate
            );
            let actual = utilities::compile_base(&case.candidate, &theme).unwrap();
            assert_eq!(
                actual, case.expected,
                "scenario {scenario_index}: {:?}",
                case.candidate
            );
        }
    }
}

#[test]
fn color_alpha_preserves_javascript_number_semantics() {
    let fixture: Fixture =
        serde_json::from_str(include_str!("../fixtures/utilities.json")).unwrap();
    for case in fixture.alpha_cases {
        assert_eq!(
            utilities::with_alpha("red", &case.alpha),
            case.with_alpha,
            "{:?}",
            case.alpha
        );
        assert_eq!(
            utilities::replace_alpha("red", &case.alpha),
            case.replace_alpha,
            "{:?}",
            case.alpha
        );
    }
}

#[test]
fn unported_implementations_are_explicit_errors() {
    let fixture: Fixture =
        serde_json::from_str(include_str!("../fixtures/utilities.json")).unwrap();
    for Pending { root, kind } in fixture.pending {
        let candidate = Candidate {
            identity: Default::default(),
            kind: if kind == Kind::Static {
                CandidateKind::Static { root: root.clone() }
            } else {
                CandidateKind::Functional {
                    root: root.clone(),
                    value: None,
                    modifier: None,
                }
            },
            variants: Vec::new(),
            important: false,
            raw: root.clone(),
        };
        assert_eq!(
            utilities::compile_base(&candidate, &Theme::default())
                .unwrap_err()
                .root,
            root
        );
    }
}

#[test]
fn all_builtin_bodies_are_implemented_and_unknown_roots_produce_no_body() {
    let fixture: Fixture =
        serde_json::from_str(include_str!("../fixtures/utilities.json")).unwrap();
    assert!(fixture.pending.is_empty());
    for kind in [
        CandidateKind::Static {
            root: "unknown-native-utility".into(),
        },
        CandidateKind::Functional {
            root: "unknown-native-utility".into(),
            value: None,
            modifier: None,
        },
    ] {
        let candidate = Candidate {
            identity: Default::default(),
            kind,
            variants: Vec::new(),
            important: false,
            raw: "unknown-native-utility".into(),
        };
        assert!(
            utilities::compile_base(&candidate, &Theme::default())
                .unwrap()
                .is_empty()
        );
    }
}
