//! V8 13.6 character-class folding during graph analysis.
//!
//! Unlike literal matching, class expansion uses the flags left by a depth-first
//! graph walk. Choice successors share nodes, so restoring flags lexically here
//! would change the behavior of the pinned reference engine.
use std::{collections::BTreeMap, ops::Range, sync::LazyLock};

#[path = "program.rs"]
mod program;

const INFINITY: u32 = i32::MAX as u32;

#[derive(Clone, Debug)]
struct Class {
    span: Range<usize>,
    folded: bool,
    literal: Option<u16>,
}

#[derive(Clone, Debug)]
struct Literal {
    code: u16,
    span: Range<usize>,
}

#[derive(Clone, Debug)]
enum TextElement {
    Skip,
    Literal(Literal),
    Class(Vec<usize>),
    Raw(Range<usize>),
}

enum Kind {
    Empty,
    Text(Vec<TextElement>, bool),
    Other(Range<usize>),
    Sequence(Vec<Tree>),
    Choice(Vec<Tree>),
    Group(u8, Box<Tree>),
    Capture(usize, Box<Tree>),
    Lookaround {
        body: Box<Tree>,
        negative: bool,
        backwards: bool,
    },
    Repeat {
        body: Box<Tree>,
        min: u32,
        max: u32,
        greedy: bool,
    },
}

struct Tree {
    kind: Kind,
    min: u32,
    max: u32,
    captures: bool,
    literal: Option<Vec<Literal>>,
}

impl Drop for Tree {
    fn drop(&mut self) {
        let mut pending = vec![];
        let mut kind = std::mem::replace(&mut self.kind, Kind::Empty);
        loop {
            match kind {
                Kind::Sequence(children) | Kind::Choice(children) => pending.extend(children),
                Kind::Group(_, child) | Kind::Capture(_, child) => pending.push(*child),
                Kind::Lookaround { body, .. } | Kind::Repeat { body, .. } => pending.push(*body),
                Kind::Empty | Kind::Text(..) | Kind::Other(_) => {}
            }
            let Some(mut child) = pending.pop() else {
                break;
            };
            kind = std::mem::replace(&mut child.kind, Kind::Empty);
        }
    }
}

impl Tree {
    fn atom(kind: Kind, min: u32, max: u32) -> Self {
        Self {
            kind,
            min,
            max,
            captures: false,
            literal: None,
        }
    }

    fn literal(code: u16, span: Range<usize>) -> Self {
        let literal = Literal { code, span };
        let mut tree = Self::atom(
            Kind::Text(vec![TextElement::Literal(literal.clone())], code <= 255),
            1,
            1,
        );
        tree.literal = Some(vec![literal]);
        tree
    }
}

struct Parser<'a> {
    input: &'a [u16],
    offset: usize,
    classes: Vec<Class>,
    captures: usize,
    contains_anchor: bool,
}

struct Group {
    flags: u8,
    capture: Option<usize>,
    lookaround: bool,
    negative: bool,
    backwards: bool,
}

struct Frame {
    flags: u8,
    group: Option<Group>,
    terms: Vec<Tree>,
    alternatives: Vec<Tree>,
}

fn sequence(mut terms: Vec<Tree>, input: &[u16]) -> Tree {
    // AssertionSequenceRewriter folds repeated consecutive assertions before
    // lowering. They would otherwise consume extra graph recursion levels.
    let mut seen: Vec<Vec<u16>> = vec![];
    terms.retain(|tree| {
        if let Kind::Other(span) = &tree.kind {
            let value = &input[span.clone()];
            if value == ['^' as u16]
                || value == ['$' as u16]
                || value == ['\\' as u16, 'b' as u16]
                || value == ['\\' as u16, 'B' as u16]
            {
                if seen.iter().any(|old| old == value) {
                    return false;
                }
                seen.push(value.to_vec());
                return true;
            }
        }
        seen.clear();
        true
    });
    if terms.len() == 1 {
        return terms.pop().unwrap();
    }
    Tree {
        min: terms
            .iter()
            .fold(0_u32, |sum, t| sum.saturating_add(t.min).min(INFINITY)),
        max: terms
            .iter()
            .fold(0_u32, |sum, t| sum.saturating_add(t.max).min(INFINITY)),
        captures: terms.iter().any(|t| t.captures),
        literal: None,
        kind: Kind::Sequence(terms),
    }
}

impl Parser<'_> {
    fn current(&self) -> Option<char> {
        self.input
            .get(self.offset)
            .and_then(|c| char::from_u32(u32::from(*c)))
    }

    // Input has already passed the ECMAScript syntax validator and normalizer.
    fn expression(&mut self, flags: u8) -> Tree {
        let mut frames = vec![Frame {
            flags,
            group: None,
            terms: vec![],
            alternatives: vec![],
        }];
        loop {
            let mut term = if self.offset == self.input.len() || self.current() == Some(')') {
                let mut frame = frames.pop().unwrap();
                frame.alternatives.push(sequence(frame.terms, self.input));
                let body = self.disjunction(frame.alternatives, frame.flags);
                let Some(group) = frame.group else {
                    return body;
                };
                self.offset += 1;
                // Keep syntactic group boundaries through assertion rewriting.
                // Groups without effects disappear only during graph lowering.
                Tree {
                    min: if group.lookaround { 0 } else { body.min },
                    max: if group.lookaround { 0 } else { body.max },
                    captures: group.capture.is_some() || body.captures,
                    literal: None,
                    kind: if group.lookaround {
                        Kind::Lookaround {
                            body: Box::new(body),
                            negative: group.negative,
                            backwards: group.backwards,
                        }
                    } else if let Some(id) = group.capture {
                        Kind::Capture(id, Box::new(body))
                    } else {
                        Kind::Group(group.flags, Box::new(body))
                    },
                }
            } else if self.current() == Some('|') {
                let frame = frames.last_mut().unwrap();
                frame
                    .alternatives
                    .push(sequence(std::mem::take(&mut frame.terms), self.input));
                self.offset += 1;
                continue;
            } else if self.current() == Some('(') {
                let group = self.group(frames.last().unwrap().flags);
                frames.push(Frame {
                    flags: group.flags,
                    group: Some(group),
                    terms: vec![],
                    alternatives: vec![],
                });
                continue;
            } else {
                let start = self.offset;
                match self.current() {
                    Some('[') => {
                        self.offset += 1;
                        while let Some(c) = self.input.get(self.offset).copied() {
                            self.offset += 1;
                            if c == '\\' as u16 {
                                self.offset += 1;
                            } else if c == ']' as u16 {
                                break;
                            }
                        }
                        let possible = latin1_class(&self.input[start..self.offset]);
                        let id = self.classes.len();
                        self.classes.push(Class {
                            span: start..self.offset,
                            folded: false,
                            literal: None,
                        });
                        Tree::atom(
                            Kind::Text(vec![TextElement::Class(vec![id])], possible),
                            1,
                            1,
                        )
                    }
                    Some('\\') => {
                        self.offset += 1;
                        let escape = self.current().expect("normalized escape");
                        self.offset += 1;
                        match escape {
                            'b' | 'B' => Tree::atom(Kind::Other(start..self.offset), 0, 0),
                            '1'..='9' => {
                                while self.current().is_some_and(|c| c.is_ascii_digit()) {
                                    self.offset += 1;
                                }
                                Tree::atom(Kind::Other(start..self.offset), 0, INFINITY)
                            }
                            'u' | 'x' => {
                                let length = if escape == 'u' { 4 } else { 2 };
                                let mut code = 0;
                                for _ in 0..length {
                                    code = code * 16
                                        + self
                                            .current()
                                            .and_then(|c| c.to_digit(16))
                                            .expect("normalized hex");
                                    self.offset += 1;
                                }
                                Tree::literal(code as u16, start..self.offset)
                            }
                            _ => Tree::atom(
                                Kind::Text(vec![TextElement::Raw(start..self.offset)], true),
                                1,
                                1,
                            ),
                        }
                    }
                    Some('^' | '$') => {
                        self.contains_anchor |= self.current() == Some('^');
                        self.offset += 1;
                        Tree::atom(Kind::Other(start..self.offset), 0, 0)
                    }
                    Some('.') => {
                        self.offset += 1;
                        Tree::atom(
                            Kind::Text(vec![TextElement::Raw(start..self.offset)], true),
                            1,
                            1,
                        )
                    }
                    _ => {
                        let code = self.input[self.offset];
                        self.offset += 1;
                        Tree::literal(code, start..self.offset)
                    }
                }
            };
            let terms = &mut frames.last_mut().unwrap().terms;
            if let Some((min, max, greedy)) = self.quantifier() {
                // V8 drops a zero-width atom with zero minimum in the parser,
                // before it closes the preceding text. Other {0} atoms remain
                // syntactic barriers until graph lowering.
                if term.max == 0 && min == 0 {
                    continue;
                }
                if max == 0 {
                    term = Tree::atom(Kind::Empty, 0, 0);
                } else if term.max != 0 {
                    term = Tree {
                        min: min.saturating_mul(term.min).min(INFINITY),
                        max: max.saturating_mul(term.max).min(INFINITY),
                        captures: term.captures,
                        literal: None,
                        kind: Kind::Repeat {
                            body: Box::new(term),
                            min,
                            max,
                            greedy,
                        },
                    };
                }
            }
            if let (
                Some(Tree {
                    kind: Kind::Text(previous, possible),
                    min,
                    max,
                    literal,
                    ..
                }),
                Kind::Text(classes, next_possible),
            ) = (terms.last_mut(), &term.kind)
            {
                previous.extend(classes.iter().cloned());
                *possible &= next_possible;
                *min = min.saturating_add(term.min).min(INFINITY);
                *max = max.saturating_add(term.max).min(INFINITY);
                match (literal.as_mut(), term.literal.take()) {
                    (Some(previous), Some(next)) => previous.extend(next),
                    _ => *literal = None,
                }
            } else {
                terms.push(term);
            }
        }
    }

    fn disjunction(&mut self, alternatives: Vec<Tree>, flags: u8) -> Tree {
        enum Work {
            Visit(Vec<Tree>),
            Ready(Tree),
            Prefix(Tree),
            Finish(usize),
        }
        let mut pending = vec![Work::Visit(alternatives)];
        let mut results = vec![];
        while let Some(work) = pending.pop() {
            match work {
                Work::Ready(tree) => results.push(tree),
                Work::Prefix(prefix) => {
                    let suffix = results.pop().unwrap();
                    results.push(sequence(vec![prefix, suffix], self.input));
                }
                Work::Visit(mut alternatives) => {
                    if alternatives.len() <= 2 {
                        results.push(choice(alternatives));
                        continue;
                    }
                    let mut offset = 0;
                    while offset < alternatives.len() {
                        let start = offset;
                        while offset < alternatives.len() && alternatives[offset].literal.is_some()
                        {
                            offset += 1;
                        }
                        alternatives[start..offset].sort_by_cached_key(|tree| {
                            fold_key(tree.literal.as_ref().unwrap()[0].code, flags)
                        });
                        offset += 1;
                    }
                    let mut tasks = vec![];
                    let mut count = 0;
                    let mut input = alternatives.into_iter().peekable();
                    while let Some(first) = input.next() {
                        let Some(atom) = &first.literal else {
                            tasks.push(Work::Ready(first));
                            count += 1;
                            continue;
                        };
                        let key = fold_key(atom[0].code, flags);
                        let mut run = vec![first];
                        while input.peek().is_some_and(|next| {
                            next.literal
                                .as_ref()
                                .is_some_and(|atom| fold_key(atom[0].code, flags) == key)
                        }) {
                            run.push(input.next().unwrap());
                        }
                        if run.len() <= 2 {
                            count += run.len();
                            tasks.extend(run.into_iter().map(Work::Ready));
                            continue;
                        }
                        let mut length = run
                            .iter()
                            .map(|tree| tree.literal.as_ref().unwrap().len())
                            .min()
                            .unwrap();
                        for tree in &run[1..] {
                            for index in 1..length {
                                if fold_key(run[0].literal.as_ref().unwrap()[index].code, flags)
                                    != fold_key(tree.literal.as_ref().unwrap()[index].code, flags)
                                {
                                    length = index;
                                    break;
                                }
                            }
                        }
                        let prefix =
                            literal_tree(run[0].literal.as_ref().unwrap()[..length].to_vec());
                        let suffixes = run
                            .into_iter()
                            .map(|mut tree| {
                                literal_tree(tree.literal.take().unwrap()[length..].to_vec())
                            })
                            .collect();
                        count += 1;
                        tasks.push(Work::Visit(suffixes));
                        tasks.push(Work::Prefix(prefix));
                    }
                    pending.push(Work::Finish(count));
                    pending.extend(tasks.into_iter().rev());
                }
                Work::Finish(count) => {
                    let factored = results.split_off(results.len() - count);
                    let mut alternatives = vec![];
                    let mut input = factored.into_iter().peekable();
                    while let Some(first) = input.next() {
                        if first.literal.as_ref().is_none_or(|atom| atom.len() != 1) {
                            alternatives.push(first);
                            continue;
                        }
                        let mut run = vec![first];
                        while input.peek().is_some_and(|next| {
                            next.literal.as_ref().is_some_and(|atom| atom.len() == 1)
                        }) {
                            run.push(input.next().unwrap());
                        }
                        if run.len() == 1 {
                            alternatives.extend(run);
                            continue;
                        }
                        let mut classes = vec![];
                        let mut possible = false;
                        for mut tree in run {
                            let atom = tree.literal.take().unwrap().pop().unwrap();
                            possible |= atom.code <= 255;
                            classes.push(self.classes.len());
                            self.classes.push(Class {
                                span: atom.span,
                                folded: false,
                                literal: Some(atom.code),
                            });
                        }
                        alternatives.push(Tree::atom(
                            Kind::Text(vec![TextElement::Class(classes)], possible),
                            1,
                            1,
                        ));
                    }
                    results.push(choice(alternatives));
                }
            }
        }
        debug_assert_eq!(results.len(), 1);
        results.pop().unwrap()
    }

    fn group(&mut self, mut flags: u8) -> Group {
        self.offset += 1;
        let mut capture = None;
        let mut lookaround = false;
        let mut negative = false;
        let mut backwards = false;
        if self.current() == Some('?') {
            self.offset += 1;
            if self.current() == Some('<') {
                backwards = true;
                self.offset += 1;
            }
            if matches!(self.current(), Some('=' | '!')) {
                lookaround = true;
                negative = self.current() == Some('!');
                self.offset += 1;
            } else {
                let mut remove = false;
                while let Some(c) = self.current() {
                    self.offset += 1;
                    if c == ':' {
                        break;
                    }
                    if c == '-' {
                        remove = true;
                        continue;
                    }
                    let bit = match c {
                        'i' => 1,
                        'm' => 2,
                        's' => 4,
                        _ => unreachable!("normalized modifier"),
                    };
                    if remove {
                        flags &= !bit;
                    } else {
                        flags |= bit;
                    }
                }
            }
        } else {
            capture = Some(self.captures);
            self.captures += 1;
        }
        Group {
            flags,
            capture,
            lookaround,
            negative,
            backwards,
        }
    }

    fn number(&mut self) -> u32 {
        let mut value = 0_u32;
        while let Some(c) = self.current().filter(char::is_ascii_digit) {
            value = value
                .saturating_mul(10)
                .saturating_add(c as u32 - '0' as u32)
                .min(INFINITY);
            self.offset += 1;
        }
        value
    }

    fn quantifier(&mut self) -> Option<(u32, u32, bool)> {
        let start = self.offset;
        let (min, max) = match self.current()? {
            '*' => {
                self.offset += 1;
                (0, INFINITY)
            }
            '+' => {
                self.offset += 1;
                (1, INFINITY)
            }
            '?' => {
                self.offset += 1;
                (0, 1)
            }
            '{' => {
                self.offset += 1;
                if !self.current().is_some_and(|c| c.is_ascii_digit()) {
                    self.offset = start;
                    return None;
                }
                let min = self.number();
                let max = if self.current() == Some(',') {
                    self.offset += 1;
                    if self.current() == Some('}') {
                        INFINITY
                    } else {
                        self.number()
                    }
                } else {
                    min
                };
                if self.current() != Some('}') {
                    self.offset = start;
                    return None;
                }
                self.offset += 1;
                (min, max)
            }
            _ => return None,
        };
        let greedy = self.current() != Some('?');
        if !greedy {
            self.offset += 1;
        }
        Some((min, max, greedy))
    }
}

fn choice(mut alternatives: Vec<Tree>) -> Tree {
    if alternatives.len() == 1 {
        return alternatives.pop().unwrap();
    }
    Tree {
        min: alternatives.iter().map(|tree| tree.min).min().unwrap(),
        max: alternatives.iter().map(|tree| tree.max).max().unwrap(),
        captures: alternatives.iter().any(|tree| tree.captures),
        literal: None,
        kind: Kind::Choice(alternatives),
    }
}

fn literal_tree(atom: Vec<Literal>) -> Tree {
    if atom.is_empty() {
        return Tree::atom(Kind::Empty, 0, 0);
    }
    let length = atom.len() as u32;
    Tree {
        kind: Kind::Text(
            atom.iter().cloned().map(TextElement::Literal).collect(),
            atom.iter().all(|unit| unit.code <= 255),
        ),
        min: length,
        max: length,
        captures: false,
        literal: Some(atom),
    }
}

fn fold_key(code: u16, flags: u8) -> Vec<u16> {
    static FOLDING: LazyLock<BTreeMap<u16, Vec<u16>>> = LazyLock::new(|| {
        let bytes = miniz_oxide::inflate::decompress_to_vec(include_bytes!(concat!(
            env!("OUT_DIR"),
            "/CaseFolding-17.0.0.txt.deflate"
        )))
        .expect("falha ao descomprimir os dados Unicode de case folding");
        std::str::from_utf8(&bytes)
            .expect("os dados Unicode de case folding devem ser UTF-8")
            .lines()
            .filter(|line| !line.starts_with('#') && !line.is_empty())
            .filter_map(|line| {
                let mut parts = line.split(';').map(str::trim);
                let code = u32::from_str_radix(parts.next().unwrap(), 16).unwrap();
                let status = parts.next().unwrap();
                if code > u16::MAX as u32 || !matches!(status, "C" | "F") {
                    return None;
                }
                let folded = parts
                    .next()
                    .unwrap()
                    .split_whitespace()
                    .flat_map(|unit| {
                        let code = u32::from_str_radix(unit, 16).unwrap();
                        char::from_u32(code)
                            .unwrap()
                            .to_string()
                            .encode_utf16()
                            .collect::<Vec<_>>()
                    })
                    .collect();
                Some((code as u16, folded))
            })
            .collect()
    });
    if flags & 1 != 0
        && let Some(folded) = FOLDING.get(&code)
    {
        return folded.clone();
    }
    vec![code]
}

fn latin1_class(pattern: &[u16]) -> bool {
    let regex = regress::Regex::from_unicode(
        pattern.iter().copied().map(u32::from),
        regress::Flags::default(),
    )
    .expect("normalized class");
    (0..=255).any(|c| regex.find_from_ucs2(&[c], 0).next().is_some())
}

pub(super) fn compile(
    input: &[u16],
    one_byte: bool,
    optimize: bool,
) -> Result<regress::Regex, regress::Error> {
    // Syntax has already been checked by the iterative ECMAScript parser.
    // Only individual atoms go through regress; its recursive whole-pattern
    // parser has a lower nesting limit than the reference engine.
    let mut parser = Parser {
        input,
        offset: 0,
        classes: vec![],
        captures: 0,
        contains_anchor: false,
    };
    let tree = parser.expression(0);
    let classes = parser.classes;
    let captures = parser.captures;
    program::compile(
        input,
        &tree,
        &classes,
        captures,
        one_byte,
        optimize,
        parser.contains_anchor,
    )
}

#[cfg(test)]
pub(super) fn rewrite(input: &[u16], one_byte: bool) -> Vec<u16> {
    let mut parser = Parser {
        input,
        offset: 0,
        classes: vec![],
        captures: 0,
        contains_anchor: false,
    };
    let tree = parser.expression(0);
    let mut classes = parser.classes;
    program::analyzed_classes(input, &tree, &mut classes, one_byte, parser.contains_anchor);
    let mut output = vec![];
    let mut from = 0;
    classes.sort_by_key(|class| class.span.start);
    for class in classes {
        output.extend(&input[from..class.span.start]);
        output.extend(if class.folded { "(?i:" } else { "(?-i:" }.encode_utf16());
        if let Some(code) = class.literal {
            output.extend(format!(r"\u{code:04x}").encode_utf16());
        } else {
            output.extend(&input[class.span.clone()]);
        }
        output.push(')' as u16);
        from = class.span.end;
    }
    output.extend(&input[from..]);
    output
}

#[cfg(test)]
mod tests {
    use super::rewrite;

    #[test]
    fn rewrites_long_sequences_without_recursive_graph_traversal() {
        let input: Vec<_> = format!("(?i:{})", "(?:a|b)".repeat(100_000))
            .encode_utf16()
            .collect();
        for one_byte in [true, false] {
            assert_eq!(rewrite(&input, one_byte), input);
        }
    }

    #[test]
    fn rewrites_deep_groups_without_recursive_parse_build_or_drop() {
        for prefix in [
            "(?:".repeat(20_000),
            "(".repeat(20_000),
            (0..20_000)
                .map(|i| if i % 2 == 0 { "(?-i:" } else { "(?i:" })
                .collect(),
        ] {
            let suffix = ")".repeat(20_000);
            let input: Vec<_> = format!("(?i:{prefix}[a]{suffix})").encode_utf16().collect();
            let expected: Vec<_> = format!("(?i:{prefix}(?i:[a]){suffix})")
                .encode_utf16()
                .collect();
            for one_byte in [true, false] {
                assert_eq!(rewrite(&input, one_byte), expected);
            }
        }
    }

    #[test]
    fn factors_nested_prefixes_without_recursive_disjunction_optimization() {
        let alternatives = (1..=128)
            .map(|length| "a".repeat(length))
            .collect::<Vec<_>>()
            .join("|");
        let input: Vec<_> = format!("(?i:{alternatives})").encode_utf16().collect();
        for one_byte in [true, false] {
            assert_eq!(rewrite(&input, one_byte), input);
        }
    }
}
