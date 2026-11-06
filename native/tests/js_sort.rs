use serde::Deserialize;
use std::cmp::Ordering;
use tailwind_canonical_native::js_sort;

#[derive(Deserialize)]
struct Fixture {
    cases: Vec<Case>,
}
#[derive(Deserialize)]
struct Case {
    input: Vec<usize>,
    matrix: Vec<Vec<i8>>,
    expected: Vec<usize>,
    calls: Option<Vec<[usize; 2]>>,
}

#[test]
fn sort_matches_v8_outputs_and_comparison_schedule_even_for_inconsistent_comparators() {
    let fixture: Fixture = serde_json::from_str(include_str!("../fixtures/sort.json")).unwrap();
    for (index, case) in fixture.cases.into_iter().enumerate() {
        let mut calls = vec![];
        let mut actual = (0..case.input.len()).collect::<Vec<_>>();
        js_sort::sort_by(&mut actual, |&a, &z| {
            if case.calls.is_some() {
                calls.push([a, z]);
            }
            match case.matrix[case.input[a]][case.input[z]] {
                -1 => Ordering::Less,
                0 => Ordering::Equal,
                1 => Ordering::Greater,
                _ => unreachable!(),
            }
        });
        assert_eq!(actual, case.expected, "output case {index}");
        if let Some(expected) = case.calls {
            assert_eq!(calls, expected, "schedule case {index}");
        }
    }
}
