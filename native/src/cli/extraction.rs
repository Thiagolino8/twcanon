use oxc_allocator::Allocator;
use oxc_ast::ast::{
    Argument, CallExpression, Expression, JSXAttribute, JSXAttributeName, JSXAttributeValue,
    JSXExpression, StringLiteral, TemplateLiteral,
};
use oxc_ast_visit::{Visit, walk};
use oxc_parser::Parser;
use oxc_span::{SourceType, Span};
use serde::Serialize;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum TextKind {
    String,
    Template,
    Attribute,
}

#[derive(Debug, Serialize)]
pub struct ClassText {
    pub start: u32,
    pub end: u32,
    pub value: String,
    pub kind: TextKind,
}

#[derive(Default)]
struct Collector {
    texts: Vec<ClassText>,
    error: Option<u32>,
}

impl Collector {
    fn push(&mut self, span: Span, value: Option<&str>, kind: TextKind) {
        if let Some(value) = value {
            self.texts.push(ClassText {
                start: span.start,
                end: span.end,
                value: value.into(),
                kind,
            });
        } else {
            self.error.get_or_insert(span.start);
        }
    }

    fn string(&mut self, literal: &StringLiteral<'_>, kind: TextKind) {
        self.push(literal.span, literal.value.as_str(), kind);
    }

    fn template(&mut self, literal: &TemplateLiteral<'_>) {
        if literal.expressions.is_empty() {
            self.push(
                literal.span,
                literal.quasis[0]
                    .value
                    .cooked
                    .and_then(|value| value.as_str()),
                TextKind::Template,
            );
        }
    }
}

impl<'a> Visit<'a> for Collector {
    fn visit_jsx_attribute(&mut self, attribute: &JSXAttribute<'a>) {
        if let JSXAttributeName::Identifier(name) = &attribute.name
            && matches!(name.name.as_str(), "class" | "className")
        {
            match &attribute.value {
                Some(JSXAttributeValue::StringLiteral(literal)) => {
                    self.string(literal, TextKind::Attribute);
                }
                Some(JSXAttributeValue::ExpressionContainer(container)) => {
                    match &container.expression {
                        JSXExpression::StringLiteral(literal) => {
                            self.string(literal, TextKind::String);
                        }
                        JSXExpression::TemplateLiteral(literal) => self.template(literal),
                        _ => {}
                    }
                }
                _ => {}
            }
        }
        walk::walk_jsx_attribute(self, attribute);
    }

    fn visit_call_expression(&mut self, call: &CallExpression<'a>) {
        if let Expression::Identifier(identifier) = &call.callee
            && matches!(
                identifier.name.as_str(),
                "mergeClassName" | "twMerge" | "cn"
            )
        {
            for argument in &call.arguments {
                match argument {
                    Argument::StringLiteral(literal) => self.string(literal, TextKind::String),
                    Argument::TemplateLiteral(literal) => self.template(literal),
                    _ => {}
                }
            }
        }
        walk::walk_call_expression(self, call);
    }
}

pub fn line_at(source: &str, end: usize) -> usize {
    let mut line = 1;
    let mut chars = source[..end].chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\r' {
            if chars.peek() == Some(&'\n') {
                chars.next();
            }
            line += 1;
        } else if matches!(c, '\n' | '\u{2028}' | '\u{2029}') {
            line += 1;
        }
    }
    line
}

pub fn collect(filename: &str, source: &str) -> Result<Vec<ClassText>, String> {
    let allocator = Allocator::default();
    let source_type = if filename.ends_with('x') {
        SourceType::tsx()
    } else {
        SourceType::ts()
    };
    let result = Parser::new(&allocator, source, source_type).parse();
    if !result.diagnostics.is_empty() {
        return Err(format!(
            "Sintaxe inválida em {filename}:\n{}",
            result
                .diagnostics
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join("\n")
        ));
    }
    let mut collector = Collector::default();
    collector.visit_program(&result.program);
    collector.error.map_or(Ok(collector.texts), |position| {
        Err(format!(
            "{filename}:L{}: texto de classe contém surrogate UTF-16 isolado.",
            line_at(source, position as usize)
        ))
    })
}
