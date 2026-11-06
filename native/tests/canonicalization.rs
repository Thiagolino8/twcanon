use serde::Deserialize;
use tailwind_canonical_native::{
    ast::AstNode,
    canonicalize::{CanonicalizeOptions, Canonicalizer},
    compile::DesignSystem,
    theme::Theme,
};

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
    nodes: Vec<AstNode>,
}
#[derive(Deserialize)]
struct Scenario {
    name: String,
    entries: Vec<Entry>,
    prefix: Option<String>,
    customs: Vec<Custom>,
    initial: Option<CanonicalizeOptions>,
    queries: Vec<Query>,
    #[serde(default)]
    prelude: Vec<Prime>,
}
#[derive(Deserialize)]
struct Prime {
    input: String,
    options: CanonicalizeOptions,
    expected: Option<String>,
}
#[derive(Deserialize)]
struct Query {
    input: Vec<String>,
    options: CanonicalizeOptions,
    expected: Option<Vec<String>>,
    error: Option<String>,
}

#[test]
fn collapse_observes_utilities_registered_after_a_cached_dynamic_lookup() {
    let mut theme = Theme::default();
    theme.add("--spacing", "0.25rem", 0).unwrap();
    let mut engine = Canonicalizer::new(DesignSystem::new(theme));
    let input = vec!["mt-7".into(), "w-7".into()];
    let options = CanonicalizeOptions {
        rem: Some(16.0),
        collapse: true,
        logical_to_physical: true,
    };
    assert_eq!(engine.canonicalize(&input, options).unwrap(), input);
    engine
        .design_system
        .custom_utility(
            "pair-*",
            &[
                AstNode::decl("margin-top", "--spacing(--value(number))"),
                AstNode::decl("width", "--spacing(--value(number))"),
            ],
        )
        .unwrap();
    assert_eq!(engine.canonicalize(&input, options).unwrap(), ["pair-7"]);
    assert_eq!(
        engine
            .canonicalize(
                &input,
                CanonicalizeOptions {
                    rem: Some(17.0),
                    ..options
                }
            )
            .unwrap(),
        ["pair-7"]
    );
}

#[test]
fn collapse_observes_replacement_of_the_design_system_with_an_equal_sized_registry() {
    let mut theme = Theme::default();
    theme.add("--spacing", "0.25rem", 0).unwrap();
    let mut first = DesignSystem::new(theme.clone());
    first
        .custom_utility(
            "solo-*",
            &[AstNode::decl("margin-top", "--spacing(--value(number))")],
        )
        .unwrap();
    let mut engine = Canonicalizer::new(first);
    let input = vec!["mt-7".into(), "w-7".into()];
    let options = CanonicalizeOptions {
        rem: Some(16.0),
        collapse: true,
        logical_to_physical: true,
    };
    assert_eq!(engine.canonicalize(&input, options).unwrap(), input);
    let mut second = DesignSystem::new(theme);
    second
        .custom_utility(
            "pair-*",
            &[
                AstNode::decl("margin-top", "--spacing(--value(number))"),
                AstNode::decl("width", "--spacing(--value(number))"),
            ],
        )
        .unwrap();
    engine.design_system = second;
    assert_eq!(engine.canonicalize(&input, options).unwrap(), ["pair-7"]);
}

#[test]
fn repeated_collapse_still_populates_the_variant_order_cache() {
    let mut theme = Theme::default();
    theme.add("--spacing", "0.25rem", 0).unwrap();
    theme.add("--breakpoint-sm", "40rem", 0).unwrap();
    theme.add("--breakpoint-md", "48rem", 0).unwrap();
    let mut engine = Canonicalizer::new(DesignSystem::new(theme.clone()));
    engine.design_system.parse_variant("sm").unwrap();
    engine.design_system.parse_variant("md").unwrap();
    let input = vec!["mt-1".into(), "mt-2".into()];
    let options = CanonicalizeOptions {
        rem: Some(16.0),
        collapse: true,
        logical_to_physical: true,
    };
    for _ in 0..3 {
        assert_eq!(engine.canonicalize(&input, options).unwrap(), ["mt-2"]);
    }
    let order = engine.design_system.variant_order().unwrap();
    assert!(order["sm"] < order["md"]);

    // A new parsed variant invalidates the order cache. Replacing the public
    // registry changes the comparison, while the unprefixed classes stay equal.
    engine.design_system.parse_variant("[&:focus]").unwrap();
    let mut reversed = theme.clone();
    reversed.add("--breakpoint-sm", "60rem", 0).unwrap();
    engine.design_system.variants = tailwind_canonical_native::variants::Variants::new(&reversed);
    assert_eq!(engine.canonicalize(&input, options).unwrap(), ["mt-2"]);
    engine.design_system.variants = tailwind_canonical_native::variants::Variants::new(&theme);
    let order = engine.design_system.variant_order().unwrap();
    assert!(order["md"] < order["sm"]);
}

#[test]
fn repeated_line_height_preparation_observes_theme_names_and_signature_options() {
    use tailwind_canonical_native::{signature, theme::INLINE};

    let mut theme = Theme::default();
    theme.add("--spacing", "0.25rem", 0).unwrap();
    theme.add("--text-sm", "0.875rem", INLINE).unwrap();
    let mut engine = Canonicalizer::new(DesignSystem::new(theme));
    let input = vec!["leading-4".into(), "block".into()];
    let options = CanonicalizeOptions {
        rem: Some(16.0),
        collapse: true,
        logical_to_physical: true,
    };
    let properties = |engine: &Canonicalizer, rem, features| {
        engine
            .static_utilities(signature::Options { rem, features })
            .into_values()
            .flat_map(|values| values.into_values().flatten())
            .collect::<std::collections::BTreeSet<_>>()
    };
    let expected = engine.canonicalize(&input, options).unwrap();
    assert!(properties(&engine, options.rem, 3).contains("text-sm/4"));
    assert_eq!(engine.canonicalize(&input, options).unwrap(), expected);

    engine
        .design_system
        .theme
        .add("--text-new", "1rem", INLINE)
        .unwrap();
    assert_eq!(engine.canonicalize(&input, options).unwrap(), expected);
    assert!(properties(&engine, options.rem, 3).contains("text-new/4"));

    engine.design_system.theme.clear_namespace("--text", 0);
    engine
        .design_system
        .theme
        .add("--text-next", "1.125rem", 0)
        .unwrap();
    assert_eq!(engine.canonicalize(&input, options).unwrap(), expected);
    assert!(properties(&engine, options.rem, 3).contains("text-next/4"));

    let mut replacement = engine.design_system.theme.clone();
    replacement.clear_namespace("--text", 0);
    replacement.add("--text-clone", "1.25rem", 0).unwrap();
    engine.design_system.theme = replacement;
    assert_eq!(engine.canonicalize(&input, options).unwrap(), expected);
    assert!(properties(&engine, options.rem, 3).contains("text-clone/4"));

    let alternate = CanonicalizeOptions {
        logical_to_physical: false,
        ..options
    };
    assert_eq!(engine.canonicalize(&input, alternate).unwrap(), expected);
    assert!(properties(&engine, alternate.rem, 1).contains("text-clone/4"));
    let alternate = CanonicalizeOptions {
        rem: Some(20.0),
        ..options
    };
    assert_eq!(engine.canonicalize(&input, alternate).unwrap(), expected);
    assert!(properties(&engine, alternate.rem, 3).contains("text-clone/5"));
}

#[test]
fn complete_candidate_pipeline_and_collapse_match_official() {
    let fixture: Fixture =
        serde_json::from_str(include_str!("../fixtures/canonicalization.json")).unwrap();
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
        let mut canonicalizer = match scenario.initial {
            Some(options) => Canonicalizer::with_options(
                system,
                tailwind_canonical_native::signature::Options {
                    rem: options.rem,
                    features: u8::from(options.collapse)
                        | (u8::from(options.logical_to_physical) * 2),
                },
            ),
            None => Canonicalizer::new(system),
        };
        for prime in scenario.prelude {
            let options = tailwind_canonical_native::signature::Options {
                rem: prime.options.rem,
                features: u8::from(prime.options.collapse)
                    | (u8::from(prime.options.logical_to_physical) * 2),
            };
            assert_eq!(
                canonicalizer.utility_signature(&prime.input, options),
                prime.expected,
                "{} prelude",
                scenario.name
            );
        }
        for (index, query) in scenario.queries.into_iter().enumerate() {
            let actual = canonicalizer.canonicalize(&query.input, query.options);
            assert_eq!(
                actual,
                query.error.map_or_else(|| Ok(query.expected.unwrap()), Err),
                "{} query {} {:?} {:?}",
                scenario.name,
                index,
                query.options,
                query.input
            );
        }
    }
}
