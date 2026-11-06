use serde::Deserialize;
use tailwind_canonical_native::{
    candidate::Kind,
    completions::{self, ClassEntry, Group},
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
    #[serde(default)]
    customs: Vec<Custom>,
    entries: Vec<Entry>,
    prefix: Option<String>,
    classes: Vec<ClassEntry>,
    utilities: Vec<Utility>,
    variants: Vec<Variant>,
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
struct Utility {
    root: String,
    groups: Vec<Group>,
}
#[derive(Deserialize)]
struct Variant {
    root: String,
    kind: Kind,
    values: Vec<String>,
}

#[test]
fn completions_and_class_lists_match_official() {
    let fixture: Fixture =
        serde_json::from_str(include_str!("../fixtures/completions.json")).unwrap();
    for scenario in fixture.scenarios {
        let mut theme = Theme::default();
        for entry in scenario.entries {
            theme.add(&entry.key, &entry.value, entry.options).unwrap();
        }
        theme.prefix = scenario.prefix;
        for utility in scenario.utilities {
            assert_eq!(
                completions::utility_groups(&utility.root, &theme),
                utility.groups,
                "{} utility {}",
                scenario.name,
                utility.root
            );
        }
        assert_eq!(
            completions::class_list(&theme),
            scenario.classes,
            "{} classes",
            scenario.name
        );
        let mut variants = Variants::new(&theme);
        for custom in scenario.customs {
            variants.from_ast(&custom.name, &custom.nodes);
        }
        let actual: Vec<_> = variants
            .entries()
            .map(|(root, definition)| {
                (root.to_owned(), definition.kind, variants.completions(root))
            })
            .collect();
        let expected: Vec<_> = scenario
            .variants
            .into_iter()
            .map(|variant| (variant.root, variant.kind, variant.values))
            .collect();
        assert_eq!(actual, expected, "{} variants", scenario.name);
    }
}
