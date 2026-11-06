//! Non-Unicode ECMAScript syntax and Annex B normalization before matching.
//!
//! The syntax walk follows Node 24.21.0's V8 regexp-parser.cc. Names are
//! registered at group close; references are checked after syntax parsing.
//! Named groups become numbered groups and Annex B escapes become explicit
//! UTF-16 literals. The matcher never receives a missing capture reference.
use std::{collections::HashMap, sync::LazyLock};

#[path = "js_regex/folding.rs"]
mod folding;

type ParseResult<T> = Result<T, &'static str>;

#[derive(serde::Deserialize)]
struct Identifiers {
    start: Vec<[u32; 2]>,
    part: Vec<[u32; 2]>,
}

static IDENTIFIERS: LazyLock<Identifiers> = LazyLock::new(|| {
    serde_json::from_str(include_str!("../data/js-regex-unicode.json"))
        .expect("pinned V8 identifier ranges")
});

fn identifier(code: u32, start: bool) -> bool {
    if matches!(code, 0x24 | 0x5f) || (!start && matches!(code, 0x200c | 0x200d)) {
        return true;
    }
    let ranges = if start {
        &IDENTIFIERS.start
    } else {
        &IDENTIFIERS.part
    };
    let index = ranges.partition_point(|range| range[1] < code);
    ranges.get(index).is_some_and(|range| range[0] <= code)
}

enum Part {
    Text(Vec<u16>),
    NamedReference(String),
}

struct Group {
    capture_index: usize,
    non_participating: (usize, usize),
    capture: Option<usize>,
    name: Option<String>,
    lookbehind: bool,
}

struct NamedCapture {
    index: usize,
}

struct Parser {
    input: Vec<u16>,
    offset: usize,
    output: Vec<Part>,
    groups: Vec<Group>,
    names: HashMap<String, Vec<NamedCapture>>,
    captures: usize,
    total_captures: usize,
    has_names: bool,
}

fn ascii(unit: Option<u16>) -> Option<char> {
    unit.filter(|unit| *unit < 128)
        .map(|unit| unit as u8 as char)
}

fn digit(unit: u16, radix: u32) -> Option<u32> {
    char::from_u32(u32::from(unit))?.to_digit(radix)
}

// Like V8's ScanForCaptures: lexical, including invalid named group headers.
fn scan_captures(input: &[u16]) -> (usize, bool) {
    let mut offset = 0;
    let mut in_class = false;
    let mut captures = 0;
    let mut has_names = false;
    while offset < input.len() {
        match ascii(input.get(offset).copied()) {
            Some('\\') => offset += 1,
            Some('[') if !in_class => in_class = true,
            Some(']') if in_class => in_class = false,
            Some('(') if !in_class => {
                if ascii(input.get(offset + 1).copied()) != Some('?') {
                    captures += 1;
                } else if ascii(input.get(offset + 2).copied()) == Some('<')
                    && !matches!(ascii(input.get(offset + 3).copied()), Some('=' | '!'))
                {
                    captures += 1;
                    has_names = true;
                }
            }
            _ => {}
        }
        offset += 1;
    }
    (captures, has_names)
}

impl Parser {
    fn new(pattern: &str) -> Self {
        let input: Vec<_> = pattern.encode_utf16().collect();
        let (total_captures, has_names) = scan_captures(&input);
        Self {
            input,
            offset: 0,
            output: vec![],
            groups: vec![Group {
                capture_index: 0,
                non_participating: (0, 0),
                capture: None,
                name: None,
                lookbehind: false,
            }],
            names: HashMap::new(),
            captures: 0,
            total_captures,
            has_names,
        }
    }

    fn current(&self) -> Option<char> {
        ascii(self.input.get(self.offset).copied())
    }

    fn emit(output: &mut Vec<Part>, units: &[u16]) {
        if let Some(Part::Text(text)) = output.last_mut() {
            text.extend_from_slice(units);
        } else {
            output.push(Part::Text(units.to_vec()));
        }
    }

    fn text(&mut self, value: &str) {
        Self::emit(&mut self.output, &value.encode_utf16().collect::<Vec<_>>());
    }

    fn literal(&mut self, unit: u16) {
        if (0xd800..=0xdfff).contains(&unit) {
            // regress combines adjacent escaped surrogate pairs even without
            // /u. Raw units retain the two separate non-Unicode atoms.
            Self::emit(&mut self.output, &[unit]);
        } else {
            self.text(&format!(r"\u{unit:04x}"));
        }
    }

    fn copy_to(&mut self, end: usize) {
        Self::emit(&mut self.output, &self.input[self.offset..end]);
        self.offset = end;
    }

    fn hex(&mut self, count: usize) -> Option<u32> {
        let slice = self.input.get(self.offset..self.offset + count)?;
        let mut value = 0;
        for &unit in slice {
            value = value * 16 + digit(unit, 16)?;
        }
        self.offset += count;
        Some(value)
    }

    fn name_codepoint(&mut self) -> ParseResult<u32> {
        let mut code = u32::from(
            *self
                .input
                .get(self.offset)
                .ok_or("Invalid capture group name")?,
        );
        self.offset += 1;
        if code == 0x5c && self.current() == Some('u') {
            self.offset += 1;
            if self.current() == Some('{') {
                self.offset += 1;
                let start = self.offset;
                code = 0;
                while let Some(value) = self
                    .input
                    .get(self.offset)
                    .and_then(|unit| digit(*unit, 16))
                {
                    code = code.saturating_mul(16).saturating_add(value);
                    self.offset += 1;
                }
                if start == self.offset || code > 0x10ffff || self.current() != Some('}') {
                    return Err("Invalid Unicode escape");
                }
                self.offset += 1;
            } else {
                code = self.hex(4).ok_or("Invalid Unicode escape")?;
                if (0xd800..=0xdbff).contains(&code)
                    && self.input.get(self.offset..self.offset + 2) == Some(&[0x5c, 0x75])
                {
                    let saved = self.offset;
                    self.offset += 2;
                    match self.hex(4) {
                        Some(low @ 0xdc00..=0xdfff) => {
                            code = 0x10000 + (code - 0xd800) * 1024 + low - 0xdc00
                        }
                        _ => self.offset = saved,
                    }
                }
            }
        } else if (0xd800..=0xdbff).contains(&code)
            && let Some(&low @ 0xdc00..=0xdfff) = self.input.get(self.offset)
        {
            self.offset += 1;
            code = 0x10000 + (code - 0xd800) * 1024 + u32::from(low) - 0xdc00;
        }
        Ok(code)
    }

    fn name(&mut self) -> ParseResult<String> {
        let mut name = String::new();
        loop {
            if !name.is_empty() && self.current() == Some('>') {
                self.offset += 1;
                return Ok(name);
            }
            let code = self.name_codepoint()?;
            if !identifier(code, name.is_empty()) {
                return Err("Invalid capture group name");
            }
            name.push(char::from_u32(code).ok_or("Invalid capture group name")?);
        }
    }

    fn open_group(&mut self) -> ParseResult<()> {
        let start = self.offset;
        self.offset += 1;
        let mut capturing = true;
        let mut lookbehind = false;
        let mut name = None;
        if self.current() == Some('?') {
            self.offset += 1;
            let mut modifiers = 0_u8;
            let mut dash = false;
            loop {
                match self.current() {
                    Some('-') => {
                        if dash {
                            return Err("Multiple dashes in flag group");
                        }
                        dash = true;
                        self.offset += 1;
                    }
                    Some(flag @ ('i' | 'm' | 's')) => {
                        let bit = match flag {
                            'i' => 1,
                            'm' => 2,
                            _ => 4,
                        };
                        if modifiers & bit != 0 {
                            return Err("Repeated flag in flag group");
                        }
                        modifiers |= bit;
                        self.offset += 1;
                    }
                    Some(':') => {
                        capturing = false;
                        self.offset += 1;
                        break;
                    }
                    Some('=' | '!') if modifiers == 0 && !dash => {
                        capturing = false;
                        self.offset += 1;
                        break;
                    }
                    Some('<') if modifiers == 0 && !dash => {
                        self.offset += 1;
                        if matches!(self.current(), Some('=' | '!')) {
                            capturing = false;
                            lookbehind = true;
                            self.offset += 1;
                        } else {
                            name = Some(self.name()?);
                        }
                        break;
                    }
                    _ => return Err("Invalid group"),
                }
            }
            if dash && modifiers == 0 {
                return Err("Invalid flag group");
            }
        }
        let capture = if capturing {
            if self.captures >= 32767 {
                return Err("Too many captures");
            }
            self.captures += 1;
            Some(self.captures)
        } else {
            None
        };
        if name.is_some() {
            self.text("(");
        } else {
            Self::emit(&mut self.output, &self.input[start..self.offset]);
        }
        self.groups.push(Group {
            capture_index: self.captures,
            non_participating: self.groups.last().unwrap().non_participating,
            capture,
            name,
            lookbehind,
        });
        Ok(())
    }

    fn close_group(&mut self) -> ParseResult<bool> {
        if self.groups.len() == 1 {
            return Err("Unmatched ')'");
        }
        let group = self.groups.pop().unwrap();
        if let Some(name) = group.name {
            let previous = self.names.entry(name).or_default();
            for capture in previous.iter() {
                if capture.index < group.non_participating.0
                    || capture.index > group.non_participating.1
                {
                    return Err("Duplicate capture group name");
                }
            }
            previous.push(NamedCapture {
                index: group.capture.unwrap(),
            });
        }
        self.copy_to(self.offset + 1);
        Ok(group.lookbehind)
    }

    // Returns the end and clamped bounds without consuming an incomplete form.
    fn quantifier(&self) -> Option<(usize, u32, u32)> {
        let mut offset = self.offset;
        match self.current()? {
            '*' => return Some((offset + 1, 0, i32::MAX as u32)),
            '+' => return Some((offset + 1, 1, i32::MAX as u32)),
            '?' => return Some((offset + 1, 0, 1)),
            '{' => offset += 1,
            _ => return None,
        }
        let start = offset;
        let number = |offset: &mut usize| {
            let mut value = 0_u32;
            while let Some(digit) = self.input.get(*offset).and_then(|unit| digit(*unit, 10)) {
                value = value
                    .saturating_mul(10)
                    .saturating_add(digit)
                    .min(i32::MAX as u32);
                *offset += 1;
            }
            value
        };
        let min = number(&mut offset);
        if offset == start {
            return None;
        }
        let max = if ascii(self.input.get(offset).copied()) == Some(',') {
            offset += 1;
            if ascii(self.input.get(offset).copied()) == Some('}') {
                i32::MAX as u32
            } else {
                number(&mut offset)
            }
        } else {
            min
        };
        (ascii(self.input.get(offset).copied()) == Some('}')).then_some((offset + 1, min, max))
    }

    // Character-class escapes return None; other escapes return their UTF-16
    // unit for range comparisons. Named/numbered references are atoms too.
    fn escape(&mut self, in_class: bool) -> ParseResult<Option<u16>> {
        self.offset += 1;
        let unit = *self.input.get(self.offset).ok_or("\\ at end of pattern")?;
        let escape = ascii(Some(unit));
        if matches!(escape, Some('d' | 'D' | 's' | 'S' | 'w' | 'W')) {
            Self::emit(&mut self.output, &[0x5c, unit]);
            self.offset += 1;
            return Ok(None);
        }
        if escape == Some('k') && self.has_names {
            if in_class {
                return Err("Invalid escape");
            }
            self.offset += 1;
            if self.current() != Some('<') {
                return Err("Invalid named reference");
            }
            self.offset += 1;
            let name = self.name()?;
            if self
                .groups
                .iter()
                .any(|group| group.name.as_ref() == Some(&name))
            {
                self.text("(?:)");
            } else {
                self.output.push(Part::NamedReference(name));
            }
            return Ok(None);
        }
        if !in_class && matches!(escape, Some('1'..='9')) {
            let mut end = self.offset;
            let mut number = 0_usize;
            while let Some(digit) = self.input.get(end).and_then(|unit| digit(*unit, 10)) {
                number = number.saturating_mul(10).saturating_add(digit as usize);
                end += 1;
            }
            if number <= self.total_captures {
                if self
                    .groups
                    .iter()
                    .any(|group| group.capture == Some(number))
                {
                    self.text("(?:)");
                } else {
                    self.text(&format!(r"(?:\{number})"));
                }
                self.offset = end;
                return Ok(None);
            }
        }
        self.offset += 1;
        let value = match escape {
            Some('b') if in_class => 8,
            Some('f') => 12,
            Some('n') => 10,
            Some('r') => 13,
            Some('t') => 9,
            Some('v') => 11,
            Some('c') => {
                let control = self.input.get(self.offset).copied();
                if control.is_some_and(|code| {
                    matches!(ascii(Some(code)), Some('a'..='z' | 'A'..='Z'))
                        || (in_class && matches!(ascii(Some(code)), Some('0'..='9' | '_')))
                }) {
                    self.offset += 1;
                    control.unwrap() & 31
                } else {
                    // Invalid control escapes consume only the backslash.
                    self.offset -= 1;
                    0x5c
                }
            }
            Some('0'..='7') => {
                let mut value = unit - 0x30;
                for _ in 0..if value < 4 { 2 } else { 1 } {
                    if let Some(next @ 0x30..=0x37) = self.input.get(self.offset).copied() {
                        value = value * 8 + next - 0x30;
                        self.offset += 1;
                    } else {
                        break;
                    }
                }
                value
            }
            Some('x') => self.hex(2).map_or(0x78, |code| code as u16),
            Some('u') => self.hex(4).map_or(0x75, |code| code as u16),
            _ => unit,
        };
        self.literal(value);
        Ok(Some(value))
    }

    fn class_atom(&mut self) -> ParseResult<Option<u16>> {
        if self.current() == Some('\\') {
            return self.escape(true);
        }
        let unit = *self
            .input
            .get(self.offset)
            .ok_or("Unterminated character class")?;
        self.copy_to(self.offset + 1);
        Ok(Some(unit))
    }

    fn class(&mut self) -> ParseResult<()> {
        self.copy_to(self.offset + 1);
        if self.current() == Some('^') {
            self.copy_to(self.offset + 1);
        }
        while self.offset < self.input.len() && self.current() != Some(']') {
            let left = self.class_atom()?;
            if self.current() == Some('-') {
                self.copy_to(self.offset + 1);
                if self.offset == self.input.len() || self.current() == Some(']') {
                    break;
                }
                let right = self.class_atom()?;
                if let (Some(left), Some(right)) = (left, right)
                    && left > right
                {
                    return Err("Range out of order in character class");
                }
            }
        }
        if self.current() != Some(']') {
            return Err("Unterminated character class");
        }
        self.copy_to(self.offset + 1);
        Ok(())
    }

    fn parse(mut self) -> ParseResult<Vec<u16>> {
        while self.offset < self.input.len() {
            let mut lookbehind = false;
            match self.current() {
                Some('(') => {
                    self.open_group()?;
                    continue;
                }
                Some(')') => lookbehind = self.close_group()?,
                Some('|') => {
                    let group = self.groups.last_mut().unwrap();
                    if group.non_participating.1 != 0 {
                        group.non_participating.1 = self.captures;
                    } else {
                        group.non_participating = (group.capture_index, self.captures);
                    }
                    self.copy_to(self.offset + 1);
                    continue;
                }
                Some('*' | '+' | '?') => return Err("Nothing to repeat"),
                Some('{') if self.quantifier().is_some() => return Err("Nothing to repeat"),
                Some('^' | '$') => {
                    self.copy_to(self.offset + 1);
                    continue;
                }
                Some('[') => self.class()?,
                Some('\\')
                    if matches!(
                        ascii(self.input.get(self.offset + 1).copied()),
                        Some('b' | 'B')
                    ) =>
                {
                    self.copy_to(self.offset + 2);
                    continue;
                }
                Some('\\') => {
                    self.escape(false)?;
                }
                _ => self.copy_to(self.offset + 1),
            }
            if let Some((end, min, max)) = self.quantifier() {
                if min > max {
                    return Err("numbers out of order in {} quantifier");
                }
                if lookbehind {
                    return Err("Invalid quantifier");
                }
                if self.current() == Some('{') {
                    self.text(&format!("{{{min},{max}}}"));
                    self.offset = end;
                } else {
                    self.copy_to(end);
                }
                if self.current() == Some('?') {
                    self.copy_to(self.offset + 1);
                }
            }
        }
        if self.groups.len() != 1 {
            return Err("Unterminated group");
        }
        let mut output = vec![];
        for part in self.output {
            match part {
                Part::Text(text) => output.extend(text),
                Part::NamedReference(name) => {
                    let captures = self
                        .names
                        .get(&name)
                        .ok_or("Invalid named capture referenced")?;
                    output.extend("(?:".encode_utf16());
                    for capture in captures {
                        output.extend(format!(r"\{}", capture.index).encode_utf16());
                    }
                    output.extend(")".encode_utf16());
                }
            }
        }
        Ok(output)
    }
}

pub(crate) struct Regex {
    one_byte: Result<regress::Regex, String>,
    two_byte: Result<regress::Regex, String>,
}

impl Regex {
    pub(crate) fn find(&self, input: &[u16]) -> Result<Option<regress::Match>, String> {
        let regex = if input.iter().any(|unit| *unit > 255) {
            &self.two_byte
        } else {
            &self.one_byte
        };
        Ok(regex
            .as_ref()
            .map_err(Clone::clone)?
            .find_from_ucs2(input, 0)
            .next())
    }
}

pub(crate) fn compile(pattern: &str) -> Result<Regex, String> {
    let error = |detail: &str| format!("Invalid regular expression: /{pattern}/g: {detail}");
    // V8's TooMuchRegExpCode uses the original source, before Annex B rewriting.
    let optimize = pattern.encode_utf16().count() <= 20 * 1024;
    let normalized = Parser::new(pattern).parse().map_err(error)?;
    // V8 compiles lazily for the subject representation. Keep resource errors
    // until execution so an impossible one-byte graph can still return no match.
    Ok(Regex {
        one_byte: folding::compile(&normalized, true, optimize)
            .map_err(|failure| error(&failure.text)),
        two_byte: folding::compile(&normalized, false, optimize)
            .map_err(|failure| error(&failure.text)),
    })
}

#[cfg(test)]
mod tests {
    use super::compile;

    #[test]
    fn matches_node_non_unicode_regex_fixtures() {
        let fixture: serde_json::Value =
            serde_json::from_str(include_str!("../fixtures/js-regex.json")).unwrap();
        assert_eq!(
            fixture["oracle_flags"],
            serde_json::json!(["--regexp-interpret-all"])
        );
        assert_eq!(fixture["versions"]["node"], "24.21.0");
        assert_eq!(fixture["versions"]["v8"], "13.6.233.17-node.53");
        let mut failed_patterns = 0;
        let mut mismatches = 0;
        let mut comparisons = 0;
        let mut examples = vec![];
        for case in fixture["construction_cases"].as_array().unwrap() {
            let result = compile(case["pattern"].as_str().unwrap());
            assert_eq!(result.is_ok(), case["accepted"].as_bool().unwrap());
            if let Err(error) = result {
                assert_eq!(Some(error.as_str()), case["error"].as_str());
            }
        }
        for case in fixture["cases"]
            .as_array()
            .unwrap()
            .iter()
            .chain(fixture["resource_cases"].as_array().unwrap())
            .chain(fixture["filter_cases"].as_array().unwrap())
        {
            let pattern = case["pattern"].as_str().unwrap();
            assert!(
                case.get("known_match_differences").is_none(),
                "unexpected waiver"
            );
            let before = mismatches;
            let actual = match compile(pattern) {
                Err(error) => serde_json::json!({"error": error, "error_phase": "construction"}),
                Ok(expression) => {
                    let results: Result<Vec<_>, String> = case["haystacks"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|haystack| {
                            let input: Vec<_> = haystack.as_str().unwrap().encode_utf16().collect();
                            std::panic::catch_unwind(|| expression.find(&input))
                                .unwrap_or_else(|_| panic!("regex matcher panicked"))
                                .map(|found| {
                                    found.map(|found| [found.range.start, found.range.end])
                                })
                        })
                        .collect();
                    match results {
                        Ok(matches) => {
                            comparisons += matches.len();
                            serde_json::json!({"matches": matches})
                        }
                        Err(error) => {
                            serde_json::json!({"error": error, "error_phase": "execution"})
                        }
                    }
                }
            };
            let expected = if case.get("error").is_some() {
                serde_json::json!({"error": case["error"], "error_phase": case["error_phase"]})
            } else {
                serde_json::json!({"matches": case["matches"]})
            };
            if actual != expected {
                mismatches += 1;
                if examples.len() < 8 {
                    let label: String = pattern.chars().take(120).collect();
                    examples.push(format!("{label:?} ({} UTF-16 units): actual error {:?}, phase {:?}; expected error {:?}, phase {:?}; matches {:?} != {:?}",
                        pattern.encode_utf16().count(),
                        actual["error"].as_str().map(|e| e.rsplit("/g: ").next().unwrap()), actual["error_phase"],
                        expected["error"].as_str().map(|e| e.rsplit("/g: ").next().unwrap()), expected["error_phase"],
                        actual["matches"], expected["matches"]));
                }
            }
            failed_patterns += usize::from(mismatches != before);
        }
        assert_eq!(
            mismatches, 0,
            "{failed_patterns} patterns differ across {comparisons} match comparisons; examples: {examples:#?}"
        );
    }
}
