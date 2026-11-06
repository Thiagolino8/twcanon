use indexmap::IndexMap;
use serde::Deserialize;
use tailwind_canonical_native::{
    ast::AstNode,
    canonicalize::{CanonicalizeOptions, Canonicalizer},
    configuration::{self, Root, Source},
    imports::LoadedStylesheet,
    theme::Entry,
};

#[derive(Deserialize)]
struct Fixture {
    scenarios: Vec<Scenario>,
}
#[derive(Deserialize)]
struct Scenario {
    input: String,
    base: String,
    files: IndexMap<String, File>,
    probes: Vec<String>,
    expected: Option<Expected>,
    error: Option<String>,
}
#[derive(Deserialize)]
struct File {
    path: String,
    base: String,
    content: String,
}
#[derive(Deserialize)]
struct Expected {
    configuration: ExpectedConfiguration,
    css: Vec<Option<String>>,
    canonicalization: Vec<CanonicalQuery>,
}
#[derive(Deserialize)]
struct CanonicalQuery {
    input: Vec<String>,
    expected: Option<Vec<String>>,
    error: Option<String>,
}
#[derive(Deserialize)]
struct ExpectedConfiguration {
    ast: Vec<AstNode>,
    sources: Vec<Source>,
    root: Option<Root>,
    inline_candidates: Vec<String>,
    has_utilities: bool,
    important: bool,
    prefix: Option<String>,
    theme: Vec<(String, Entry)>,
    invalid: Vec<String>,
}

#[test]
fn css_directives_imports_custom_variants_and_apply_match_official() {
    let fixture: Fixture =
        serde_json::from_str(include_str!("../fixtures/configuration.json")).unwrap();
    for (index, scenario) in fixture.scenarios.into_iter().enumerate() {
        let result = configuration::load(&scenario.input, &scenario.base, &mut |id, base| {
            let key = format!("{base}/{id}");
            let file = scenario
                .files
                .get(&key)
                .ok_or_else(|| format!("Missing stylesheet: {key}"))?;
            Ok(LoadedStylesheet {
                path: file.path.clone(),
                base: file.base.clone(),
                content: file.content.clone(),
            })
        });
        match result {
            Err(error) => assert_eq!(
                Some(error),
                scenario.error,
                "scenario {index} {}",
                scenario.input
            ),
            Ok(mut actual) => {
                assert!(
                    scenario.error.is_none(),
                    "scenario {index}: expected {:?}",
                    scenario.error
                );
                let expected = scenario.expected.unwrap();
                let config = expected.configuration;
                assert_eq!(
                    actual.ast, config.ast,
                    "scenario {index}: AST {}",
                    scenario.input
                );
                assert_eq!(actual.sources, config.sources, "scenario {index}: sources");
                assert_eq!(actual.root, config.root, "scenario {index}: root");
                assert_eq!(
                    actual.inline_candidates, config.inline_candidates,
                    "scenario {index}: inline"
                );
                assert_eq!(
                    actual.has_utilities, config.has_utilities,
                    "scenario {index}: utilities"
                );
                assert_eq!(
                    actual.design_system.important, config.important,
                    "scenario {index}: important"
                );
                assert_eq!(
                    actual.design_system.theme.prefix, config.prefix,
                    "scenario {index}: prefix"
                );
                assert_eq!(
                    serde_json::to_value(actual.design_system.theme.entries().unwrap()).unwrap(),
                    serde_json::to_value(config.theme).unwrap(),
                    "scenario {index}: theme"
                );
                assert_eq!(
                    actual
                        .design_system
                        .invalid_candidates
                        .iter()
                        .cloned()
                        .collect::<Vec<_>>(),
                    config.invalid,
                    "scenario {index}: invalid"
                );
                assert_eq!(
                    actual
                        .design_system
                        .candidates_to_css(&scenario.probes)
                        .unwrap(),
                    expected.css,
                    "scenario {index}: CSS"
                );
                let mut canonicalizer = Canonicalizer::new(actual.design_system);
                for query in expected.canonicalization {
                    assert_eq!(
                        canonicalizer.canonicalize(
                            &query.input,
                            CanonicalizeOptions {
                                rem: Some(16.0),
                                ..Default::default()
                            }
                        ),
                        query.error.map_or_else(|| Ok(query.expected.unwrap()), Err),
                        "scenario {index}: canonicalization"
                    );
                }
            }
        }
    }
}
