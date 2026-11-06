// Port of Tailwind CSS 4.3.4 at-import.ts. See LICENSE.tailwind.
use crate::{
    ast::{AstNode, ContextValue},
    syntax::{css, value},
};
use indexmap::IndexMap;

/// A stylesheet returned by the caller's resolver. Resolution and I/O stay with
/// the host; imported CSS is parsed and expanded by this library.
#[derive(Clone, Debug)]
pub struct LoadedStylesheet {
    pub path: String,
    pub base: String,
    pub content: String,
}

#[derive(Default)]
struct ImportParams {
    uri: Option<String>,
    layer: Option<String>,
    supports: Option<String>,
    media: Option<String>,
}

fn parse_params(params: &[value::Value]) -> Result<Option<ImportParams>, String> {
    use value::Value;
    let mut result = ImportParams::default();
    for (index, node) in params.iter().enumerate() {
        if matches!(node, Value::Separator { .. }) {
            continue;
        }
        if let Value::Word { value } = node
            && result.uri.as_ref().is_none_or(|s| s.is_empty())
        {
            if !matches!(value.as_bytes().first(), Some(b'\'' | b'"')) {
                return Ok(None);
            }
            let units: Vec<_> = value.encode_utf16().collect();
            result.uri = Some(
                String::from_utf16(&units[1..units.len().saturating_sub(1).max(1)])
                    .map_err(|_| "Import URI contains an unpaired UTF-16 surrogate".to_owned())?,
            );
            continue;
        }
        if let Value::Function { value, .. } = node
            && value.eq_ignore_ascii_case("url")
        {
            return Ok(None);
        }
        if result.uri.as_ref().is_none_or(|s| s.is_empty()) {
            return Ok(None);
        }
        let (name, arguments) = match node {
            Value::Word { value } => (value.as_str(), None),
            Value::Function { value, nodes } => (value.as_str(), Some(nodes)),
            Value::Separator { .. } => unreachable!(),
        };
        if name.eq_ignore_ascii_case("layer") {
            if result.layer.as_ref().is_some_and(|s| !s.is_empty()) {
                return Ok(None);
            }
            if result.supports.as_ref().is_some_and(|s| !s.is_empty()) {
                return Err("`layer(…)` in an `@import` should come before any other functions or conditions".into());
            }
            result.layer = Some(arguments.map_or_else(String::new, |v| value::to_css(v)));
            continue;
        }
        if name.eq_ignore_ascii_case("supports") && arguments.is_some() {
            if result.supports.as_ref().is_some_and(|s| !s.is_empty()) {
                return Ok(None);
            }
            result.supports = arguments.map(|v| value::to_css(v));
            continue;
        }
        result.media = Some(value::to_css(&params[index..]));
        break;
    }
    Ok(result
        .uri
        .as_ref()
        .is_some_and(|s| !s.is_empty())
        .then_some(result))
}

fn context(base: String, nodes: Vec<AstNode>) -> AstNode {
    AstNode::Context {
        context: IndexMap::from([("base".into(), ContextValue::String(base))]),
        nodes,
    }
}

fn wrap(params: ImportParams, node: AstNode) -> Vec<AstNode> {
    let mut nodes = vec![node];
    if let Some(layer) = params.layer {
        nodes = vec![AstNode::AtRule {
            name: "@layer".into(),
            params: layer,
            nodes,
        }];
    }
    if let Some(media) = params.media {
        nodes = vec![AstNode::AtRule {
            name: "@media".into(),
            params: media,
            nodes,
        }];
    }
    if let Some(supports) = params.supports {
        nodes = vec![AstNode::AtRule {
            name: "@supports".into(),
            params: if supports.starts_with('(') {
                supports
            } else {
                format!("({supports})")
            },
            nodes,
        }];
    }
    nodes
}

/// Substitute local `@import` and `@reference` rules. Remote and `url(...)`
/// imports remain in the AST. Returns whether an import was recognized.
pub fn substitute_at_imports(
    ast: &mut [AstNode],
    base: &str,
    load: &mut impl FnMut(&str, &str) -> Result<LoadedStylesheet, String>,
) -> Result<bool, String> {
    fn visit(
        ast: &mut [AstNode],
        base: &str,
        load: &mut impl FnMut(&str, &str) -> Result<LoadedStylesheet, String>,
        depth: usize,
    ) -> Result<bool, String> {
        let mut recognized = false;
        for node in ast {
            if let AstNode::AtRule { name, params, .. } = node
                && matches!(name.as_str(), "@import" | "@reference")
                && let Some(mut parsed) = parse_params(&value::parse(params))?
            {
                if name == "@reference" {
                    parsed.media = Some("reference".into());
                }
                recognized = true;
                let uri = parsed.uri.as_deref().unwrap();
                if !["data:", "http://", "https://"]
                    .iter()
                    .any(|prefix| uri.starts_with(prefix))
                {
                    if depth > 100 {
                        return Err(format!(
                            "Exceeded maximum recursion depth while resolving `{uri}` in `{base}`)"
                        ));
                    }
                    let loaded = load(uri, base)?;
                    let mut imported = css::parse(&loaded.content).map_err(|e| e.message)?;
                    visit(&mut imported, &loaded.base, load, depth + 1)?;
                    *node = AstNode::Context {
                        context: IndexMap::new(),
                        nodes: wrap(parsed, context(loaded.base, imported)),
                    };
                    continue;
                }
            }
            if let Some(children) = node.children_mut() {
                recognized |= visit(children, base, load, depth)?;
            }
        }
        Ok(recognized)
    }
    visit(ast, base, load, 0)
}
