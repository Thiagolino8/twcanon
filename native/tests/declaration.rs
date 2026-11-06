use serde::Deserialize;
use serde_json::{Value, json};
use tailwind_canonical_native::{
    ast::AstNode,
    data_type::{self, DataType},
    declaration, numeric,
};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Fixture {
    data_types: Vec<DataType>,
    numeric_cases: Vec<NumericCase>,
    fold_cases: Vec<FoldCase>,
    expansion_cases: Vec<ExpansionCase>,
}
#[derive(Deserialize)]
struct NumericCase {
    input: String,
    expected: Value,
}
#[derive(Deserialize)]
struct FoldCase {
    input: String,
    rem: Option<f64>,
    normalize: bool,
    expected: String,
}
#[derive(Deserialize)]
struct ExpansionCase {
    property: String,
    value: String,
    logical: bool,
    important: bool,
    expected: Value,
}

#[test]
fn numeric_and_declaration_transformations_match_official_tailwind() {
    let fixture: Fixture =
        serde_json::from_str(include_str!("../fixtures/declaration.json")).unwrap();
    for case in fixture.numeric_cases {
        let actual = json!({
            "dimension": numeric::dimension(&case.input).map(|(number, unit)| (serde_json::from_str::<Value>(&numeric::format(number)).unwrap_or(Value::Null), unit)),
            "integer": numeric::is_positive_integer(&case.input),
            "strictInteger": numeric::is_strict_positive_integer(&case.input),
            "multiplier": numeric::is_spacing_multiplier(&case.input),
            "types": fixture.data_types.iter().map(|t| data_type::infer(&case.input, &[*t])).collect::<Vec<_>>()
        });
        assert_eq!(actual, case.expected, "numeric: {:?}", case.input);
    }
    for case in fixture.fold_cases {
        assert_eq!(
            declaration::constant_fold(&case.input, case.rem, case.normalize),
            case.expected,
            "fold {:?} rem={:?}, normalize={}",
            case.input,
            case.rem,
            case.normalize
        );
    }
    for case in fixture.expansion_cases {
        let node = AstNode::Declaration {
            identity: Default::default(),
            property: case.property.clone().into(),
            value: Some(case.value.clone()),
            important: case.important,
        };
        assert_eq!(
            json!(declaration::expand(&node, case.logical)),
            case.expected,
            "expand {:?}: {:?}, logical={}",
            case.property,
            case.value,
            case.logical
        );
    }
}
