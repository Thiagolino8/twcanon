use serde::Deserialize;
use tailwind_canonical_native::{
    candidate::{Candidate, Modifier, Variant},
    canonicalize::{
        Canonicalizer,
        converter::{self, Convert},
    },
    compat,
    compile::DesignSystem,
    numeric, print,
    signature::Options,
    theme::Theme,
};

#[derive(Deserialize)]
struct Fixture {
    scenarios: Vec<Scenario>,
    key_paths: Vec<KeyPath>,
    numeric: Vec<Numeric>,
    utility_scenarios: Vec<UtilityScenario>,
    variant_scenarios: Vec<VariantScenario>,
}
#[derive(Deserialize)]
struct VariantScenario {
    name: String,
    entries: Vec<Entry>,
    prefix: Option<String>,
    options: Options,
    cases: Vec<VariantCase>,
    customs: Vec<Custom>,
}
#[derive(Deserialize)]
struct Custom {
    name: String,
    nodes: Vec<tailwind_canonical_native::ast::AstNode>,
}
#[derive(Deserialize)]
struct VariantCase {
    input: String,
    expected: Option<Vec<VariantResult>>,
    error: Option<String>,
}
#[derive(Debug, Deserialize, PartialEq)]
struct VariantResult {
    variant: Variant,
    printed: String,
}
#[derive(Deserialize)]
struct UtilityScenario {
    name: String,
    entries: Vec<Entry>,
    prefix: Option<String>,
    options: Options,
    cases: Vec<UtilityCase>,
}
#[derive(Deserialize)]
struct UtilityCase {
    input: String,
    expected: Option<Vec<UtilityResult>>,
    error: Option<String>,
}
#[derive(Debug, Deserialize, PartialEq)]
struct UtilityResult {
    candidate: Candidate,
    printed: String,
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
    convert: Vec<Conversion>,
    spacing: Vec<Spacing>,
}
#[derive(Deserialize)]
struct Conversion {
    input: String,
    mode: u8,
    expected: Option<(String, Option<Modifier>)>,
    error: Option<String>,
}
#[derive(Deserialize)]
struct Spacing {
    input: String,
    expected: Option<String>,
    error: Option<String>,
}
#[derive(Deserialize)]
struct KeyPath {
    input: String,
    parts: Vec<String>,
    expected: Option<String>,
    error: Option<String>,
}
#[derive(Deserialize)]
struct Numeric {
    input: f64,
    digits: u32,
    expected: f64,
}

#[test]
fn converter_and_spacing_match_official() {
    let fixture: Fixture =
        serde_json::from_str(include_str!("../fixtures/transforms.json")).unwrap();
    for case in fixture.key_paths {
        assert_eq!(
            compat::to_key_path(&case.input),
            case.parts,
            "{}",
            case.input
        );
        let actual = compat::key_path_to_css_property(&case.parts);
        assert_eq!(
            actual,
            case.error.map_or_else(|| Ok(case.expected), Err),
            "{}",
            case.input
        );
    }
    for case in fixture.numeric {
        assert_eq!(
            numeric::round_fixed(case.input, case.digits),
            case.expected,
            "{} {}",
            case.input,
            case.digits
        )
    }
    assert_eq!(numeric::round_fixed(-0.0, 10).to_bits(), 0.0_f64.to_bits());
    for scenario in fixture.scenarios {
        let mut theme = Theme::default();
        for entry in scenario.entries {
            theme.add(&entry.key, &entry.value, entry.options).unwrap();
        }
        theme.prefix = scenario.prefix;
        for case in scenario.convert {
            let mode = match case.mode {
                0 => Convert::All,
                1 => Convert::MigrateModifier,
                2 => Convert::MigrateThemeOnly,
                _ => panic!("invalid mode"),
            };
            let actual = converter::convert(&theme, &case.input, mode);
            assert_eq!(
                actual,
                case.error.map_or_else(|| Ok(case.expected.unwrap()), Err),
                "{} convert {} mode {}",
                scenario.name,
                case.input,
                case.mode
            );
        }
        for case in scenario.spacing {
            assert!(case.error.is_none());
            assert_eq!(
                converter::spacing_calc(&case.input, &theme),
                case.expected.unwrap(),
                "{} spacing {}",
                scenario.name,
                case.input
            );
        }
    }
    for scenario in fixture.utility_scenarios {
        let mut theme = Theme::default();
        for entry in scenario.entries {
            theme.add(&entry.key, &entry.value, entry.options).unwrap();
        }
        theme.prefix = scenario.prefix.clone();
        let mut canonicalizer =
            Canonicalizer::with_options(DesignSystem::new(theme), scenario.options);
        for case in scenario.cases {
            let candidates = canonicalizer.design_system.parse_candidate(&case.input);
            let actual: Result<Vec<_>, _> = candidates
                .into_iter()
                .map(|candidate| {
                    let candidate = canonicalizer.transform_utility(candidate, scenario.options)?;
                    let printed = print::candidate(&candidate, scenario.prefix.as_deref());
                    Ok(UtilityResult { candidate, printed })
                })
                .collect();
            assert_eq!(
                actual,
                case.error.map_or_else(|| Ok(case.expected.unwrap()), Err),
                "{} {:?} {}",
                scenario.name,
                scenario.options,
                case.input
            );
        }
    }
    for scenario in fixture.variant_scenarios {
        let mut theme = Theme::default();
        for entry in scenario.entries {
            theme.add(&entry.key, &entry.value, entry.options).unwrap();
        }
        theme.prefix = scenario.prefix;
        let mut system = DesignSystem::new(theme);
        for custom in scenario.customs {
            system.custom_variant(&custom.name, &custom.nodes);
        }
        let mut canonicalizer = Canonicalizer::with_options(system, scenario.options);
        for case in scenario.cases {
            let actual = canonicalizer
                .design_system
                .parse_variant(&case.input)
                .map(|variant| {
                    canonicalizer
                        .transform_variant(variant, scenario.options)
                        .map(|variants| {
                            variants
                                .into_iter()
                                .map(|variant| {
                                    let printed = print::variant(&variant);
                                    VariantResult { variant, printed }
                                })
                                .collect::<Vec<_>>()
                        })
                })
                .transpose();
            assert_eq!(
                actual,
                case.error.map_or_else(|| Ok(case.expected), Err),
                "{} {:?} variant {}",
                scenario.name,
                scenario.options,
                case.input
            );
        }
    }
}
