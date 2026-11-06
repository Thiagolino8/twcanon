use serde::Deserialize;
use serde_json::{Value, json};
use tailwind_canonical_native::theme::Theme;

#[derive(Deserialize)]
struct Fixture {
    cases: Vec<Case>,
}
#[derive(Deserialize)]
struct Case {
    steps: Vec<Step>,
    expected: Vec<Value>,
}
#[derive(Debug, Deserialize)]
struct Step {
    action: String,
    key: Option<String>,
    value: Option<String>,
    options: Option<u8>,
    #[serde(default)]
    keys: Vec<String>,
    #[serde(default)]
    nested: Vec<String>,
    candidate: Option<String>,
}

#[test]
fn theme_state_transitions_match_official_tailwind() {
    let fixture: Fixture = serde_json::from_str(include_str!("../fixtures/theme.json")).unwrap();
    for case in fixture.cases {
        assert_eq!(case.steps.len(), case.expected.len());
        let mut theme = Theme::default();
        for (i, (step, expected)) in case.steps.into_iter().zip(case.expected).enumerate() {
            let key = step.key.as_deref().unwrap_or("");
            let options = step.options.unwrap_or(0);
            let result = match step.action.as_str() {
                "add" => match theme.add(key, step.value.as_deref().unwrap_or(""), options) {
                    Ok(()) => Value::Null,
                    Err(error) => json!({"error":error}),
                },
                "clear" => {
                    theme.clear_namespace(key, options);
                    Value::Null
                }
                "prefix" => {
                    theme.prefix = step.value.clone();
                    Value::Null
                }
                "keys" => json!(theme.keys_in_namespaces(&step.keys)),
                "get" => json!(theme.get(&step.keys)),
                "options" => json!(theme.get_options(key)),
                "default" => json!(theme.has_default(key)),
                "used" => json!(theme.mark_used_variable(key)),
                "resolve" => json!(theme.resolve(step.candidate.as_deref(), &step.keys, options)),
                "value" => json!(theme.resolve_value(step.candidate.as_deref(), &step.keys)),
                "with" => json!(theme.resolve_with(
                    step.candidate.as_deref().unwrap_or(""),
                    &step.keys,
                    &step.nested
                )),
                "namespace" => json!(theme.namespace(key)),
                other => panic!("Unknown operation: {other}"),
            };
            assert_eq!(
                json!({"result": result, "entries":theme.entries().unwrap()}),
                expected,
                "step {i}: {step:?}"
            );
        }
    }
}

#[test]
fn keyframes_preserve_set_identity_and_insertion_order() {
    use std::sync::Arc;
    use tailwind_canonical_native::ast::AstNode;
    let first = Arc::new(AstNode::AtRule {
        name: "@keyframes".into(),
        params: "spin".into(),
        nodes: Vec::new(),
    });
    let same_content = Arc::new(first.as_ref().clone());
    let mut theme = Theme::default();
    theme.add_keyframes(first.clone());
    theme.add_keyframes(first.clone());
    theme.add_keyframes(same_content.clone());
    assert_eq!(theme.keyframes().len(), 2);
    assert!(Arc::ptr_eq(&theme.keyframes()[0], &first));
    assert!(Arc::ptr_eq(&theme.keyframes()[1], &same_content));
}

#[test]
fn prefix_slicing_uses_utf16_and_rejects_unrepresentable_keys() {
    let mut theme = Theme::default();
    theme.prefix = Some("tw".into());
    assert_eq!(theme.prefix_key("😀abc").unwrap(), "--tw-abc");
    assert!(theme.prefix_key("a😀bc").is_err());
    theme.add("a😀bc", "x", 0).unwrap();
    assert!(theme.entries().is_err());
}
