use serde::Deserialize;
use tailwind_canonical_native::{
    canonicalize::Canonicalizer, compile::DesignSystem, signature::Options, theme::Theme,
};

#[derive(Deserialize)]
struct Fixture {
    scenarios: Vec<Scenario>,
}
#[derive(Deserialize)]
struct Custom {
    name: String,
    nodes: Vec<tailwind_canonical_native::ast::AstNode>,
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
    #[serde(default)]
    customs: Vec<Custom>,
    entries: Vec<Entry>,
    prefix: Option<String>,
    variant: Vec<VariantCase>,
    utility: Vec<UtilityCase>,
    variant_indexes: Vec<VariantIndex>,
    utility_indexes: Vec<UtilityIndex>,
}
#[derive(Deserialize)]
struct VariantCase {
    input: String,
    rem: Option<f64>,
    signature: Option<String>,
}
#[derive(Deserialize)]
struct UtilityCase {
    input: String,
    options: Options,
    signature: Option<String>,
    variables: Option<Vec<String>>,
    properties: Vec<(String, Vec<String>)>,
}
#[derive(Deserialize)]
struct VariantIndex {
    rem: Option<f64>,
    expected: Vec<(String, Vec<String>)>,
}
type StaticIndex = Vec<(String, Vec<(String, Vec<String>)>)>;
#[derive(Deserialize)]
struct UtilityIndex {
    options: Options,
    expected: Vec<(String, Vec<String>)>,
    static_expected: StaticIndex,
}

#[test]
fn signatures_and_precomputed_indexes_match_official_including_insertion_order() {
    let fixture: Fixture = serde_json::from_str(include_str!("../fixtures/indexes.json")).unwrap();
    for scenario in fixture.scenarios {
        let mut theme = Theme::default();
        for entry in scenario.entries {
            theme.add(&entry.key, &entry.value, entry.options).unwrap();
        }
        theme.prefix = scenario.prefix;
        let mut system = DesignSystem::new(theme);
        for custom in scenario.customs {
            system.custom_variant(&custom.name, &custom.nodes);
        }
        let mut canonicalizer = Canonicalizer::new(system);
        for case in scenario.variant {
            let actual = canonicalizer.variant_signature(&case.input, case.rem);
            assert_eq!(
                actual, case.signature,
                "{} variant {} {:?}",
                scenario.name, case.input, case.rem
            );
            assert_eq!(
                canonicalizer.variant_signature(&case.input, case.rem),
                actual,
                "variant cache"
            );
        }
        for case in scenario.utility {
            assert_eq!(
                canonicalizer.utility_signature(&case.input, case.options),
                case.signature,
                "{} utility {}",
                scenario.name,
                case.input
            );
            assert_eq!(
                canonicalizer.utility_theme_variables(&case.input, case.options),
                case.variables,
                "{} variables {}",
                scenario.name,
                case.input
            );
            let actual: Vec<_> = canonicalizer
                .utility_properties(&case.input, case.options)
                .into_iter()
                .map(|(property, values)| {
                    (
                        property.into_string(),
                        values.into_iter().collect::<Vec<_>>(),
                    )
                })
                .collect();
            assert_eq!(
                actual, case.properties,
                "{} properties {}",
                scenario.name, case.input
            );
        }
        assert!(!canonicalizer.same_utility("unknown", "unknown", Options::default()));
        for case in scenario.variant_indexes {
            assert_eq!(
                canonicalizer
                    .variant_index(case.rem)
                    .into_iter()
                    .collect::<Vec<_>>(),
                case.expected,
                "{} variant index {:?}",
                scenario.name,
                case.rem
            );
        }
        for case in scenario.utility_indexes {
            assert_eq!(
                canonicalizer
                    .utility_index(case.options)
                    .into_iter()
                    .collect::<Vec<_>>(),
                case.expected,
                "{} utility index {:?}",
                scenario.name,
                case.options
            );
            let actual: StaticIndex = canonicalizer
                .static_utilities(case.options)
                .into_iter()
                .map(|(property, values)| {
                    (
                        property,
                        values
                            .into_iter()
                            .map(|(value, classes)| (value, classes.into_iter().collect()))
                            .collect(),
                    )
                })
                .collect();
            assert_eq!(
                actual, case.static_expected,
                "{} static index {:?}",
                scenario.name, case.options
            );
        }
    }
}
