use serde::Deserialize;
use tailwind_canonical_native::{
    compare::{self, Collation},
    declaration,
};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Fixture {
    locale: String,
    locales: Vec<String>,
    calc_cases: Vec<CalcCase>,
    compare_cases: Vec<CompareCase>,
}
#[derive(Deserialize)]
struct CalcCase {
    input: String,
    expected: String,
}
#[derive(Deserialize)]
struct CompareCase {
    left: String,
    right: String,
    natural: i8,
    locale: Vec<i8>,
}

#[test]
fn ordering_and_calc_canonicalization_match_official_tailwind() {
    let fixture: Fixture = serde_json::from_str(include_str!("../fixtures/compare.json")).unwrap();
    let collation = Collation::new(&fixture.locale).unwrap();
    for case in fixture.calc_cases {
        assert_eq!(
            declaration::canonicalize_calc(&case.input, &collation),
            case.expected,
            "calc: {:?}",
            case.input
        );
    }
    let collations: Vec<_> = fixture
        .locales
        .iter()
        .map(|locale| Collation::new(locale).unwrap())
        .collect();
    for case in fixture.compare_cases {
        assert_eq!(case.locale.len(), collations.len());
        assert_eq!(
            compare::natural(&case.left, &case.right) as i8,
            case.natural,
            "natural: {:?} vs {:?}",
            case.left,
            case.right
        );
        for (i, (collation, expected)) in collations.iter().zip(case.locale).enumerate() {
            assert_eq!(
                collation.compare(&case.left, &case.right) as i8,
                expected,
                "locale {}: {:?} vs {:?}",
                fixture.locales[i],
                case.left,
                case.right
            );
        }
    }
}
