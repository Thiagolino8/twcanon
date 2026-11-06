use std::collections::BTreeMap;

use serde::Deserialize;
use tailwind_canonical_native::{
    ast::{self, AstNode},
    imports::{LoadedStylesheet, substitute_at_imports},
    syntax::css,
};

#[derive(Deserialize)]
struct Fixture {
    files: BTreeMap<String, File>,
    cases: Vec<Case>,
    #[serde(rename = "utf16BoundaryInputs")]
    utf16_boundary_inputs: Vec<String>,
}
#[derive(Deserialize)]
struct File {
    path: String,
    base: String,
    content: String,
}
#[derive(Deserialize)]
struct Case {
    input: String,
    base: String,
    expected: Option<Expected>,
    error: Option<String>,
}
#[derive(Debug, Deserialize, PartialEq, Eq)]
struct Expected {
    recognized: bool,
    nodes: Vec<AstNode>,
    css: String,
    calls: Vec<[String; 2]>,
}

#[test]
fn imports_match_official_expansion_conditions_errors_and_loaded_bases() {
    let fixture: Fixture = serde_json::from_str(include_str!("../fixtures/imports.json")).unwrap();
    for (index, case) in fixture.cases.into_iter().enumerate() {
        let result = css::parse(&case.input)
            .map_err(|error| error.message)
            .and_then(|mut nodes| {
                let mut calls = Vec::new();
                let recognized = substitute_at_imports(&mut nodes, &case.base, &mut |id, base| {
                    calls.push([id.to_owned(), base.to_owned()]);
                    let loaded = fixture
                        .files
                        .get(&format!("{base}/{id}"))
                        .or_else(|| fixture.files.get(id))
                        .ok_or_else(|| format!("Cannot resolve {id} from {base}"))?;
                    Ok(LoadedStylesheet {
                        path: loaded.path.clone(),
                        base: loaded.base.clone(),
                        content: loaded.content.clone(),
                    })
                })?;
                calls.sort_by_key(|call| serde_json::to_string(call).unwrap());
                Ok(Expected {
                    recognized,
                    css: ast::to_css(&nodes),
                    nodes,
                    calls,
                })
            });
        assert_eq!(
            result,
            case.error.map_or_else(|| Ok(case.expected.unwrap()), Err),
            "case {index}: {}",
            case.input
        );
    }
    for input in fixture.utf16_boundary_inputs {
        let mut nodes = css::parse(&input).unwrap();
        let result = substitute_at_imports(&mut nodes, "/root", &mut |_, _| {
            panic!("Unrepresentable URI must fail before the loader")
        });
        assert_eq!(
            result,
            Err("Import URI contains an unpaired UTF-16 surrogate".into()),
            "{input}"
        );
    }
}
