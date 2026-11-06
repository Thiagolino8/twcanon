use serde::Deserialize;
use std::collections::BTreeMap;
use tailwind_canonical_native::{
    ast::AstNode,
    candidate::{self, Registry, Variant, VariantDefinition},
    theme::Theme,
    variants::Variants,
};

#[derive(Deserialize)]
struct Fixture {
    scenarios: Vec<Scenario>,
}
#[derive(Deserialize)]
struct Scenario {
    name: String,
    entries: Vec<Entry>,
    prefix: Option<String>,
    registry: BTreeMap<String, VariantDefinition>,
    customs: Vec<Custom>,
    cases: Vec<Case>,
    comparisons: Vec<Comparison>,
    orders: Vec<Order>,
}
#[derive(Deserialize)]
struct Entry {
    key: String,
    value: String,
    options: u8,
}
#[derive(Deserialize)]
struct Case {
    input: String,
    body: Vec<AstNode>,
    variant: Option<Variant>,
    expected: Option<AstNode>,
    error: Option<String>,
}
#[derive(Deserialize)]
struct Custom {
    name: String,
    nodes: Vec<AstNode>,
}
#[derive(Deserialize)]
struct Comparison {
    left: String,
    right: String,
    expected: i8,
}
#[derive(Deserialize)]
struct Order {
    input: Vec<String>,
    expected: Vec<String>,
}

#[test]
fn variants_match_official_registry_application_and_order_across_themes() {
    let fixture: Fixture = serde_json::from_str(include_str!("../fixtures/variants.json")).unwrap();
    for scenario in fixture.scenarios {
        let mut theme = Theme::default();
        for entry in scenario.entries {
            theme.add(&entry.key, &entry.value, entry.options).unwrap();
        }
        theme.prefix = scenario.prefix;
        let mut variants = Variants::new(&theme);
        for custom in scenario.customs {
            variants.from_ast(&custom.name, &custom.nodes);
        }
        assert_eq!(
            serde_json::to_value(variants.registry()).unwrap(),
            serde_json::to_value(scenario.registry).unwrap(),
            "registry: {}",
            scenario.name
        );
        let mut registry = Registry::default();
        variants.extend_registry(&mut registry);
        for case in scenario.cases {
            let parsed = candidate::parse_variant(&case.input, &registry);
            assert_eq!(
                parsed, case.variant,
                "parse / {}: {}",
                scenario.name, case.input
            );
            let actual = parsed
                .map(|variant| {
                    let mut node = AstNode::Rule {
                        selector: ".fixture".into(),
                        nodes: case.body,
                    };
                    variants
                        .apply(&mut node, &variant)
                        .map(|applied| applied.then_some(node))
                })
                .unwrap_or(Ok(None));
            let expected = case.error.map_or(Ok(case.expected), Err);
            assert_eq!(
                actual, expected,
                "apply / {}: {}",
                scenario.name, case.input
            );
        }
        for case in scenario.comparisons {
            let left = candidate::parse_variant(&case.left, &registry);
            let right = candidate::parse_variant(&case.right, &registry);
            assert_eq!(
                variants.compare_parsed(
                    left.as_ref().map(|ast| (case.left.as_str(), ast)),
                    right.as_ref().map(|ast| (case.right.as_str(), ast))
                ) as i8,
                case.expected,
                "compare / {}: {} / {}",
                scenario.name,
                case.left,
                case.right
            );
        }
        for case in scenario.orders {
            let mut actual = case
                .input
                .into_iter()
                .map(|input| {
                    let parsed = candidate::parse_variant(&input, &registry).unwrap();
                    (input, parsed)
                })
                .collect::<Vec<_>>();
            tailwind_canonical_native::js_sort::sort_by(&mut actual, |(a, a_ast), (z, z_ast)| {
                variants.compare_parsed(Some((a, a_ast)), Some((z, z_ast)))
            });
            for (index, ((actual, _), expected)) in actual.iter().zip(&case.expected).enumerate() {
                assert_eq!(
                    actual, expected,
                    "sorted / {} / index {index}",
                    scenario.name
                );
            }
        }
    }
}
