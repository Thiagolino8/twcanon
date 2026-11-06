//! V8 emission copies and deferred continuations for the fixed interpreter reference.
use super::{Action, Block, Class, Graph, TextElement};
use regress::bytecode::Insn;

#[derive(Clone, Copy, Default)]
struct Quick {
    mask: u16,
    value: u16,
    perfect: bool,
    cannot: bool,
}

impl Quick {
    fn merge(mut self, other: Self) -> Self {
        if other.cannot {
            return self;
        }
        if self.cannot {
            return other;
        }
        self.perfect &= self.mask == other.mask && self.value == other.value && other.perfect;
        self.mask &= other.mask;
        self.value &= self.mask;
        self.mask &= !(self.value ^ (other.value & self.mask));
        self.value &= self.mask;
        self
    }
}

#[derive(Clone, Copy, Debug)]
struct Trace {
    offset: i32,
    actions: bool,
    backtrack: bool,
    at_start: i8,
    preloaded: usize,
    checked: usize,
    quick: usize,
    perfect: bool,
    budget: usize,
    positions: Option<usize>,
}

impl Default for Trace {
    fn default() -> Self {
        Self {
            offset: 0,
            actions: false,
            backtrack: false,
            at_start: 0,
            preloaded: 0,
            checked: 0,
            quick: 0,
            perfect: false,
            budget: 100,
            positions: None,
        }
    }
}

impl Trace {
    fn trivial(self) -> bool {
        self.offset == 0
            && !self.actions
            && !self.backtrack
            && self.at_start == 0
            && self.preloaded == 0
            && self.checked == 0
            && self.quick == 0
    }
}

#[derive(Default)]
struct Versions {
    generic: Option<usize>,
    copies: usize,
    queued: bool,
}

struct Emitter<'a> {
    original: Graph,
    output: Graph,
    versions: Vec<Versions>,
    work_list: Vec<usize>,
    flags: u8,
    minimum: Vec<usize>,
    input: &'a [u16],
    classes: &'a [Class],
    one_byte: bool,
    positions: Vec<(usize, i32, Option<usize>)>,
}

enum Work {
    Visit {
        id: usize,
        trace: Trace,
        depth: usize,
        out: usize,
        generic: bool,
    },
    Flags(u8),
}

impl Emitter<'_> {
    fn placeholder(&mut self) -> usize {
        self.output.add(Block::Goal)
    }

    fn alias(&mut self, from: usize, to: usize) {
        self.output.blocks[from] = Block::Flags {
            flags: self.flags,
            next: to,
        };
    }

    fn defer(&mut self, id: usize, out: usize) {
        let generic = if let Some(generic) = self.versions[id].generic {
            generic
        } else {
            let label = self.placeholder();
            self.versions[id].generic = Some(label);
            label
        };
        if !self.versions[id].queued {
            self.versions[id].queued = true;
            self.work_list.push(id);
        }
        self.alias(out, generic);
    }

    fn first_check(&self, id: usize, flags: u8, not_at_start: bool, depth: usize) -> Quick {
        self.first_check_in(id, flags, not_at_start, depth, &mut vec![])
    }

    fn first_check_in(
        &self,
        id: usize,
        flags: u8,
        not_at_start: bool,
        depth: usize,
        visited: &mut Vec<usize>,
    ) -> Quick {
        if depth > 100 {
            return Quick::default();
        }
        match &self.original.blocks[id] {
            Block::Goal => Quick::default(),
            Block::Fail => Quick {
                cannot: true,
                ..Quick::default()
            },
            Block::Flags { flags, next } => {
                self.first_check_in(*next, *flags, not_at_start, depth + 1, visited)
            }
            Block::Action {
                action: Action::Initialize,
                next,
            } => {
                if let Block::Loop {
                    min,
                    body_min,
                    body,
                    ..
                } = self.original.blocks[*next]
                    && min > 0
                    && body_min > 0
                {
                    return self.first_check_in(body, flags, not_at_start, depth + 1, visited);
                }
                self.first_check_in(*next, flags, not_at_start, depth + 1, visited)
            }
            Block::Capture { next, .. }
            | Block::Action { next, .. }
            | Block::Lookaround {
                negative: false,
                next,
                ..
            }
            | Block::NegativeChoice { next, .. } => {
                self.first_check_in(*next, flags, not_at_start, depth + 1, visited)
            }
            Block::SubmatchEnd { .. } => Quick::default(),
            Block::Other {
                span,
                next,
                flags: lexical,
            } => {
                if span.len() >= 2
                    && self.input[span.start] == '\\' as u16
                    && (b'1' as u16..=b'9' as u16).contains(&self.input[span.start + 1])
                {
                    Quick::default()
                } else if self.input[span.clone()] == ['^' as u16]
                    && lexical & 2 == 0
                    && not_at_start
                {
                    Quick {
                        cannot: true,
                        ..Quick::default()
                    }
                } else {
                    self.first_check_in(*next, flags, not_at_start, depth + 1, visited)
                }
            }
            Block::Text {
                elements,
                backwards,
                ..
            } => {
                if *backwards {
                    Quick::default()
                } else {
                    self.element_quick(&elements[0], flags)
                }
            }
            Block::Choice(choices) => choices
                .iter()
                .map(|id| self.first_check_in(*id, flags, not_at_start, depth + 1, visited))
                .reduce(Quick::merge)
                .unwrap_or_default(),
            Block::Loop {
                body,
                next,
                nullable,
                ..
            } => {
                if *nullable || visited.contains(&id) {
                    return Quick::default();
                }
                visited.push(id);
                let first = self.first_check_in(*body, flags, not_at_start, depth + 1, visited);
                let second = self.first_check_in(*next, flags, not_at_start, depth + 1, visited);
                visited.pop();
                first.merge(second)
            }
            Block::LoopAgain(next) => {
                self.first_check_in(*next, flags, not_at_start, depth, visited)
            }
            _ => Quick::default(),
        }
    }

    fn element_quick(&self, element: &TextElement, flags: u8) -> Quick {
        let (pattern, bits, literal) = match element {
            TextElement::Literal(literal) => (
                format!(r"\u{:04x}", literal.code)
                    .encode_utf16()
                    .collect::<Vec<_>>(),
                flags,
                true,
            ),
            TextElement::Raw(span) => (self.input[span.clone()].to_vec(), flags, false),
            TextElement::Skip => return Quick::default(),
            TextElement::Class(ids) => {
                let mut pattern = vec!['[' as u16];
                if ids.len() > 1 {
                    for id in ids {
                        pattern.extend(
                            format!(r"\u{:04x}", self.classes[*id].literal.unwrap()).encode_utf16(),
                        );
                    }
                    pattern.push(']' as u16);
                } else {
                    let class = &self.classes[ids[0]];
                    pattern = class.literal.map_or_else(
                        || self.input[class.span.clone()].to_vec(),
                        |code| format!(r"\u{code:04x}").encode_utf16().collect(),
                    );
                }
                (pattern, u8::from(self.classes[ids[0]].folded), false)
            }
        };
        if pattern.starts_with(&['[' as u16, '^' as u16]) {
            return Quick::default();
        }
        let compiled = regress::Regex::from_unicode(
            pattern.iter().copied().map(u32::from),
            regress::Flags {
                icase: bits & 1 != 0,
                dot_all: bits & 4 != 0,
                no_opt: true,
                ..regress::Flags::default()
            },
        )
        .unwrap()
        .into_program();
        let limit = if self.one_byte { 255 } else { 65535 };
        let ranges: Vec<_> = match &compiled.insns[0] {
            Insn::Char(c) => std::iter::once(*c..*c + 1).collect(),
            Insn::CharSet(values) => values.iter().map(|c| *c..*c + 1).collect(),
            Insn::AsciiBracket(bitmap) => {
                let mut ranges = vec![];
                let mut start = None;
                for code in 0..=128_u32 {
                    if code < 128 && bitmap.0[code as usize / 8] & (1 << (code % 8)) != 0 {
                        start.get_or_insert(code);
                    } else if let Some(first) = start.take() {
                        ranges.push(first..code);
                    }
                }
                ranges
            }
            Insn::Bracket(index) => {
                let bracket = &compiled.brackets[*index];
                if bracket.invert {
                    return Quick::default();
                }
                bracket
                    .cps
                    .intervals()
                    .iter()
                    .map(|iv| iv.codepoints())
                    .collect()
            }
            // V8 does not reject empty classes in a quick check. The full
            // text check still emits the continuation after the failing atom.
            Insn::JustFail => return Quick::default(),
            _ => return Quick::default(),
        };
        if ranges.is_empty() && !literal {
            return Quick::default();
        }
        let ranges: Vec<_> = ranges
            .into_iter()
            .filter(|r| r.start <= limit)
            .map(|r| r.start..r.end.min(limit + 1))
            .collect();
        if ranges.is_empty() {
            return Quick {
                cannot: true,
                ..Quick::default()
            };
        }
        if literal {
            let mut codes: Vec<_> = ranges.into_iter().flatten().collect();
            codes.sort();
            codes.dedup();
            let mut mask = limit;
            let mut value = codes[0];
            for code in &codes[1..] {
                mask &= !(code ^ value);
                value &= mask;
            }
            Quick {
                mask: mask as u16,
                value: value as u16,
                perfect: codes.len() == 1 || codes.len() == 2 && (limit ^ mask).is_power_of_two(),
                cannot: false,
            }
        } else {
            let mut result = None;
            for range in ranges {
                let from = range.start;
                let to = range.end - 1;
                let mut smeared = from ^ to;
                smeared |= smeared >> 1;
                smeared |= smeared >> 2;
                smeared |= smeared >> 4;
                smeared |= smeared >> 8;
                smeared |= smeared >> 16;
                let mask = (!smeared & limit) as u16;
                let quick = Quick {
                    mask,
                    value: from as u16 & mask,
                    perfect: (from ^ to) & ((from ^ to) + 1) == 0 && from + (from ^ to) == to,
                    cannot: false,
                };
                result = Some(result.map_or(quick, |old: Quick| {
                    let mut merged = old.merge(quick);
                    merged.perfect = false;
                    merged
                }));
            }
            result.unwrap()
        }
    }

    fn run(&mut self, id: usize, trace: Trace, depth: usize, out: usize, generic: bool) {
        let mut pending = vec![Work::Visit {
            id,
            trace,
            depth,
            out,
            generic,
        }];
        while let Some(work) = pending.pop() {
            let Work::Visit {
                id,
                mut trace,
                depth,
                out,
                generic,
            } = work
            else {
                let Work::Flags(flags) = work else {
                    unreachable!()
                };
                self.flags = flags;
                continue;
            };
            let block = self.original.blocks[id].clone();
            if let Block::LoopAgain(entry) = block {
                let target = self.placeholder();
                self.output.blocks[out] = Block::LoopAgain(target);
                pending.push(Work::Visit {
                    id: entry,
                    trace,
                    depth,
                    out: target,
                    generic: false,
                });
                continue;
            }
            if matches!(&block, Block::Loop { .. }) && !trace.trivial() {
                pending.push(Work::Visit {
                    id,
                    trace: Trace::default(),
                    depth,
                    out,
                    generic: false,
                });
                continue;
            }
            let backreference = matches!(&block,Block::Other {span,..} if span.len()>=2 && self.input[span.start]=='\\' as u16 && (b'1' as u16..=b'9' as u16).contains(&self.input[span.start+1]));
            if (matches!(
                &block,
                Block::Goal | Block::SubmatchEnd { negative: true, .. }
            ) || backreference)
                && !trace.trivial()
            {
                pending.push(Work::Visit {
                    id,
                    trace: Trace::default(),
                    depth,
                    out,
                    generic: false,
                });
                continue;
            }
            let counted = !matches!(
                &block,
                Block::Goal
                    | Block::Fail
                    | Block::Other { .. }
                    | Block::SubmatchEnd { negative: true, .. }
            ) || backreference;
            if counted && !generic {
                if trace.trivial() {
                    if let Some(label) = self.versions[id].generic {
                        self.alias(out, label);
                        continue;
                    }
                    if depth > 100 {
                        self.defer(id, out);
                        continue;
                    }
                    self.versions[id].generic = Some(out);
                    // A bound generic must never be queued again.
                    self.versions[id].queued = true;
                } else {
                    self.versions[id].copies += 1;
                    if depth > 100 || !self.original.optimize || self.versions[id].copies >= 10 {
                        self.defer(id, out);
                        continue;
                    }
                }
            }
            let next_depth = depth + usize::from(counted);
            match block {
                Block::Goal => self.output.blocks[out] = Block::Goal,
                Block::Fail => self.output.blocks[out] = Block::Fail,
                Block::Flags { flags, next } => {
                    self.flags = flags;
                    let target = self.placeholder();
                    self.output.blocks[out] = Block::Flags {
                        flags,
                        next: target,
                    };
                    pending.push(Work::Visit {
                        id: next,
                        trace,
                        depth: next_depth,
                        out: target,
                        generic: false,
                    });
                }
                Block::Text {
                    mut elements,
                    flags,
                    backwards,
                    next,
                    possible,
                } => {
                    let length = elements.len();
                    if trace.quick > 0 && trace.perfect && !backwards {
                        elements[0] = TextElement::Skip;
                    }
                    trace.offset += if backwards {
                        -(length as i32)
                    } else {
                        length as i32
                    };
                    trace.at_start = if backwards { 0 } else { -1 };
                    trace.preloaded = 0;
                    trace.checked = trace.checked.saturating_sub(length);
                    trace.quick = trace.quick.saturating_sub(length);
                    trace.perfect = false;
                    let target = self.placeholder();
                    self.output.blocks[out] = Block::Text {
                        elements,
                        flags: flags & !1 | self.flags & 1,
                        backwards,
                        next: target,
                        possible,
                    };
                    pending.push(Work::Visit {
                        id: next,
                        trace,
                        depth: next_depth,
                        out: target,
                        generic: false,
                    });
                }
                Block::Other { span, flags, next } => {
                    if self.input[span.clone()] == ['^' as u16] && flags & 2 == 0 {
                        if trace.at_start == -1 {
                            self.output.blocks[out] = Block::Fail;
                            continue;
                        }
                        trace.at_start = 1;
                    }
                    let target = self.placeholder();
                    self.output.blocks[out] = Block::Other {
                        span,
                        flags: if backreference { self.flags } else { flags },
                        next: target,
                    };
                    pending.push(Work::Visit {
                        id: next,
                        trace,
                        depth: next_depth,
                        out: target,
                        generic: false,
                    });
                }
                Block::Capture { id, start, next } => {
                    trace.actions = true;
                    let target = self.placeholder();
                    self.output.blocks[out] = Block::Capture {
                        id,
                        start,
                        next: target,
                    };
                    pending.push(Work::Visit {
                        id: next,
                        trace,
                        depth: next_depth,
                        out: target,
                        generic: false,
                    });
                }
                Block::Action { action, next } => {
                    match action {
                        Action::Begin if !trace.trivial() => {
                            pending.push(Work::Visit {
                                id,
                                trace: Trace::default(),
                                depth: next_depth,
                                out,
                                generic: false,
                            });
                            continue;
                        }
                        Action::Begin => {}
                        Action::Empty { position, minimum } => {
                            let mut cursor = trace.positions;
                            let mut stored = None;
                            while let Some(index) = cursor {
                                let (id, offset, previous) = self.positions[index];
                                if id == position {
                                    stored = Some(offset);
                                    break;
                                }
                                cursor = previous;
                            }
                            if stored == Some(trace.offset) && !minimum {
                                self.output.blocks[out] = Block::Fail;
                                continue;
                            }
                            if !stored.is_some_and(|offset| offset < trace.offset)
                                && !trace.trivial()
                            {
                                pending.push(Work::Visit {
                                    id,
                                    trace: Trace::default(),
                                    depth: next_depth,
                                    out,
                                    generic: false,
                                });
                                continue;
                            }
                        }
                        Action::Store(position) => {
                            self.positions
                                .push((position, trace.offset, trace.positions));
                            trace.positions = Some(self.positions.len() - 1);
                            trace.actions = true;
                        }
                        Action::Deferred | Action::Initialize => trace.actions = true,
                    }
                    let target = self.placeholder();
                    self.output.blocks[out] = Block::Action {
                        action,
                        next: target,
                    };
                    pending.push(Work::Visit {
                        id: next,
                        trace,
                        depth: next_depth,
                        out: target,
                        generic: false,
                    });
                }
                Block::Lookaround {
                    negative: false,
                    backwards,
                    groups,
                    body,
                    end,
                    ..
                } => {
                    if !trace.trivial() {
                        pending.push(Work::Visit {
                            id,
                            trace: Trace::default(),
                            depth: next_depth,
                            out,
                            generic: false,
                        });
                        continue;
                    }
                    let target = self.placeholder();
                    self.output.blocks[out] = Block::Lookaround {
                        negative: false,
                        backwards,
                        groups,
                        body: target,
                        end,
                        next: 0,
                    };
                    pending.push(Work::Visit {
                        id: body,
                        trace,
                        depth: next_depth,
                        out: target,
                        generic: false,
                    });
                }
                Block::SubmatchEnd {
                    negative: false,
                    clear,
                    next,
                } => {
                    if !trace.trivial() {
                        pending.push(Work::Visit {
                            id,
                            trace: Trace::default(),
                            depth: next_depth,
                            out,
                            generic: false,
                        });
                        continue;
                    }
                    let target = self.placeholder();
                    self.output.blocks[out] = Block::SubmatchEnd {
                        negative: false,
                        clear,
                        next: target,
                    };
                    trace.backtrack = clear;
                    pending.push(Work::Visit {
                        id: next,
                        trace,
                        depth: next_depth,
                        out: target,
                        generic: false,
                    });
                }
                Block::SubmatchEnd { negative: true, .. } => self.output.blocks[out] = Block::Goal,
                Block::NegativeChoice { body, next, .. } => {
                    self.choice(
                        Work::Visit {
                            id,
                            trace,
                            depth,
                            out,
                            generic: false,
                        },
                        next_depth,
                        vec![body, next],
                        Some(block),
                        &mut pending,
                    );
                }
                Block::Choice(choices) => {
                    self.choice(
                        Work::Visit {
                            id,
                            trace,
                            depth,
                            out,
                            generic: false,
                        },
                        next_depth,
                        choices,
                        None,
                        &mut pending,
                    );
                }
                Block::Loop {
                    body, next, greedy, ..
                } => {
                    let choices = if greedy {
                        vec![body, next]
                    } else {
                        vec![next, body]
                    };
                    self.choice(
                        Work::Visit {
                            id,
                            trace,
                            depth,
                            out,
                            generic: false,
                        },
                        next_depth,
                        choices,
                        Some(block),
                        &mut pending,
                    );
                }
                _ => unreachable!("unsupported emission node"),
            }
        }
    }

    fn choice(
        &mut self,
        work: Work,
        next_depth: usize,
        choices: Vec<usize>,
        loop_node: Option<Block>,
        pending: &mut Vec<Work>,
    ) {
        let Work::Visit {
            id,
            trace,
            depth,
            out,
            ..
        } = work
        else {
            unreachable!("choice emission requires a visit")
        };
        if trace.actions && trace.budget == 0 {
            pending.push(Work::Visit {
                id,
                trace: Trace::default(),
                depth,
                out,
                generic: false,
            });
            return;
        }
        let flags = self.flags;
        let preload = self.minimum[id].min(1);
        let mut children = vec![];
        let mut inline = vec![];
        let mut slow = vec![];
        let mut current = trace.preloaded == preload;
        let mut checked = current;
        for (index, choice) in choices.iter().enumerate() {
            let target = self.placeholder();
            children.push(target);
            let mut child = trace;
            if self.original.not_at_start[id] {
                child.at_start = -1;
            }
            child.preloaded = if current { preload } else { 0 };
            child.checked = if checked { preload } else { trace.checked };
            child.quick = 0;
            if index + 1 < choices.len() {
                child.backtrack = true;
            }
            if trace.actions {
                child.budget = trace.budget / choices.len();
            }
            // The negative body did not contribute to EatsAtLeast;
            // its quick check cannot assume the preloaded width.
            let skip_quick = index == 0 && matches!(loop_node, Some(Block::NegativeChoice { .. }));
            let detail = if preload > 0 && !skip_quick {
                self.first_check(*choice, flags, trace.at_start == -1, 0)
            } else {
                Quick::default()
            };
            if detail.cannot {
                self.output.blocks[target] = Block::Fail;
                continue;
            }
            let quick = detail.mask & 255 != 0;
            if quick {
                let guarded = self.placeholder();
                self.output.blocks[target] = Block::Quick {
                    mask: detail.mask,
                    value: detail.value,
                    one_byte: self.one_byte,
                    next: guarded,
                };
                current = true;
                checked = true;
                child.preloaded = preload;
                child.quick = preload;
                child.perfect = detail.perfect;
                if index + 1 == choices.len() {
                    child.checked = preload;
                    inline.push((*choice, child, guarded));
                    current = false;
                } else {
                    child.backtrack = true;
                    slow.push((*choice, child, guarded));
                }
            } else {
                if index != 0 {
                    child.preloaded = 0;
                }
                inline.push((*choice, child, target));
                current = false;
            }
        }
        self.output.blocks[out] = if let Some(Block::Loop {
            id,
            min,
            max,
            greedy,
            groups,
            nullable,
            body_min,
            ..
        }) = loop_node
        {
            let (body, next) = if greedy {
                (children[0], children[1])
            } else {
                (children[1], children[0])
            };
            Block::Loop {
                id,
                min,
                max,
                greedy,
                groups,
                nullable,
                body_min,
                body,
                next,
            }
        } else if let Some(Block::NegativeChoice {
            backwards, groups, ..
        }) = loop_node
        {
            Block::Lookaround {
                negative: true,
                backwards,
                groups,
                body: children[0],
                end: 0,
                next: children[1],
            }
        } else {
            Block::Choice(children)
        };
        pending.push(Work::Flags(flags));
        for (choice, child, target) in inline
            .into_iter()
            .chain(slow)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
        {
            pending.push(Work::Visit {
                id: choice,
                trace: child,
                depth: next_depth,
                out: target,
                generic: false,
            });
            pending.push(Work::Flags(flags));
        }
    }
}

pub(super) fn emit(
    mut graph: Graph,
    start: usize,
    input: &[u16],
    classes: &[Class],
    one_byte: bool,
    anchored: bool,
) -> (Graph, usize) {
    // Split BeginNegativeSubmatch from its special Choice for version counting.
    for id in 0..graph.blocks.len() {
        if let Block::Lookaround {
            negative: true,
            backwards,
            ref groups,
            body,
            next,
            ..
        } = graph.blocks[id].clone()
        {
            let choice = graph.add(Block::NegativeChoice {
                backwards,
                groups: groups.clone(),
                body,
                next,
            });
            graph.blocks[id] = Block::Action {
                action: Action::Begin,
                next: choice,
            };
        }
    }
    let mut minimum = vec![0; graph.blocks.len()];
    let mut pending = vec![(start, false)];
    let mut done = vec![false; graph.blocks.len()];
    let mut active = vec![false; graph.blocks.len()];
    while let Some((id, exit)) = pending.pop() {
        if done[id] {
            continue;
        }
        if !exit {
            if active[id] {
                continue;
            }
            active[id] = true;
            pending.push((id, true));
            match &graph.blocks[id] {
                Block::Text { next, .. }
                | Block::Other { next, .. }
                | Block::Capture { next, .. }
                | Block::Flags { next, .. }
                | Block::Action { next, .. }
                | Block::SubmatchEnd {
                    negative: false,
                    next,
                    ..
                } => pending.push((*next, false)),
                Block::Loop { next, body, .. } => {
                    pending.push((*next, false));
                    pending.push((*body, false));
                }
                Block::Choice(choices) => pending.extend(choices.iter().map(|id| (*id, false))),
                Block::Lookaround { body, next, .. } | Block::NegativeChoice { body, next, .. } => {
                    pending.push((*next, false));
                    pending.push((*body, false));
                }
                _ => {}
            }
        } else {
            minimum[id] = match &graph.blocks[id] {
                Block::Text { elements, next, .. } => {
                    elements.len().saturating_add(minimum[*next]).min(4)
                }
                Block::Capture { next, .. }
                | Block::Flags { next, .. }
                | Block::Lookaround {
                    negative: false,
                    next,
                    ..
                }
                | Block::NegativeChoice { next, .. } => minimum[*next],
                Block::Action {
                    action: Action::Initialize,
                    next,
                } => {
                    if let Block::Loop {
                        min,
                        body_min,
                        next: rest,
                        ..
                    } = graph.blocks[*next]
                    {
                        ((min as usize).saturating_mul(body_min as usize) + minimum[rest]).min(4)
                    } else {
                        minimum[*next]
                    }
                }
                Block::Action { next, .. } | Block::Loop { next, .. } => minimum[*next],
                Block::Other { span, next, .. } => {
                    if span.len() >= 2
                        && input[span.start] == '\\' as u16
                        && (b'1' as u16..=b'9' as u16).contains(&input[span.start + 1])
                    {
                        0
                    } else {
                        minimum[*next]
                    }
                }
                Block::Choice(choices) => choices.iter().map(|id| minimum[*id]).min().unwrap_or(0),
                _ => 0,
            };
            done[id] = true;
        }
    }
    let count = graph.blocks.len();
    let optimize = graph.optimize;
    let loops = graph.loops;
    let contains_anchor = graph.contains_anchor;
    let mut emitter = Emitter {
        original: graph,
        output: Graph {
            contains_anchor,
            blocks: vec![Block::Goal],
            not_at_start: vec![false],
            loops,
            optimize,
        },
        versions: (0..count).map(|_| Versions::default()).collect(),
        work_list: vec![],
        flags: 0,
        minimum,
        input,
        classes,
        one_byte,
        positions: vec![],
    };
    let out = emitter.placeholder();
    if anchored {
        emitter.run(start, Trace::default(), 0, out, false);
    } else if contains_anchor {
        let loop_version = emitter.placeholder();
        let first_version = emitter.placeholder();
        let preload = emitter.minimum[start].min(1);
        let first_quick = emitter.first_check(start, 0, false, 0);
        // PreprocessRegExp unrolls the virtual search loop whenever the parser
        // saw ^, including multiline assertions and assertions removed by {0}.
        // ChoiceNode emits the first alternative after the search loop only
        // when its quick check branches to an out-of-line continuation.
        let first_inline = preload == 0 || first_quick.mask & 255 == 0;
        let emit_first = |emitter: &mut Emitter<'_>| {
            let quick = first_quick;
            let mut trace = Trace {
                backtrack: true,
                ..Trace::default()
            };
            let guarded = if preload > 0 && quick.mask & 255 != 0 {
                let guarded = emitter.placeholder();
                emitter.output.blocks[first_version] = Block::Quick {
                    mask: quick.mask,
                    value: quick.value,
                    one_byte,
                    next: guarded,
                };
                trace.preloaded = preload;
                trace.quick = preload;
                trace.perfect = quick.perfect;
                guarded
            } else {
                first_version
            };
            emitter.run(start, trace, 1, guarded, false);
        };
        if first_inline {
            emit_first(&mut emitter);
            emitter.flags = 0;
        }
        let quick = emitter.first_check(start, 0, true, 0);
        let mut trace = Trace {
            backtrack: true,
            at_start: -1,
            ..Trace::default()
        };
        let guarded = if preload > 0 && quick.mask & 255 != 0 {
            let guarded = emitter.placeholder();
            emitter.output.blocks[loop_version] = Block::Quick {
                mask: quick.mask,
                value: quick.value,
                one_byte,
                next: guarded,
            };
            trace.preloaded = preload;
            trace.quick = preload;
            trace.perfect = quick.perfect;
            guarded
        } else {
            loop_version
        };
        emitter.run(start, trace, 3, guarded, false);
        emitter.flags = 0;
        if !first_inline {
            emit_first(&mut emitter);
        }
        emitter.output.blocks[out] = Block::Search {
            first: first_version,
            later: loop_version,
        };
    } else {
        let quick = emitter.first_check(start, 0, false, 0);
        let preload = emitter.minimum[start].min(1);
        let mut trace = Trace {
            backtrack: true,
            ..Trace::default()
        };
        let guarded = if preload > 0 && quick.mask & 255 != 0 {
            let guarded = emitter.placeholder();
            emitter.output.blocks[out] = Block::Quick {
                mask: quick.mask,
                value: quick.value,
                one_byte,
                next: guarded,
            };
            trace.preloaded = preload;
            trace.quick = preload;
            trace.perfect = quick.perfect;
            guarded
        } else {
            out
        };
        emitter.run(start, trace, 1, guarded, false);
    }
    if !anchored {
        emitter.flags = 0;
    }
    while let Some(node) = emitter.work_list.pop() {
        let label = emitter.versions[node].generic.unwrap();
        emitter.run(node, Trace::default(), 0, label, true);
    }
    (emitter.output, out)
}
