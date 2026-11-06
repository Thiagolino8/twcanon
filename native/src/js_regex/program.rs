//! Lower the V8-shaped graph to the Rust matcher's instruction stream.
//!
//! Capture and loop identifiers belong to the graph, rather than its emitted
//! copies. This lets subsequent code generation duplicate a continuation without
//! adding source-level captures or changing backreference numbers.
use super::{Class, INFINITY, Kind, TextElement, Tree};
use regress::bytecode::{CompiledRegex, Insn, LoopFields, StartPredicate};
use std::ops::Range;
#[path = "emission.rs"]
mod emission;

#[derive(Clone, Debug)]
enum Block {
    Goal,
    Fail,
    Search {
        first: usize,
        later: usize,
    },
    Quick {
        mask: u16,
        value: u16,
        one_byte: bool,
        next: usize,
    },
    Flags {
        flags: u8,
        next: usize,
    },
    Text {
        elements: Vec<TextElement>,
        possible: bool,
        flags: u8,
        backwards: bool,
        next: usize,
    },
    Other {
        span: Range<usize>,
        flags: u8,
        next: usize,
    },
    Capture {
        id: usize,
        start: bool,
        next: usize,
    },
    // V8 ActionNodes whose runtime effects are already part of EnterLoop.
    Action {
        action: Action,
        next: usize,
    },
    // Positive submatch success reaches the continuation during graph passes,
    // but returns to the lookaround instruction during matching.
    SubmatchEnd {
        clear: bool,
        negative: bool,
        next: usize,
    },
    Choice(Vec<usize>),
    NegativeChoice {
        backwards: bool,
        groups: Range<u16>,
        body: usize,
        next: usize,
    },
    Loop {
        id: u32,
        min: u32,
        max: u32,
        greedy: bool,
        groups: Range<u16>,
        nullable: bool,
        body_min: u32,
        body: usize,
        next: usize,
    },
    LoopAgain(usize),
    Lookaround {
        negative: bool,
        backwards: bool,
        groups: Range<u16>,
        body: usize,
        end: usize,
        next: usize,
    },
}

#[derive(Clone, Copy, Debug)]
enum Action {
    Begin,
    Deferred,
    Initialize,
    Store(usize),
    Empty { position: usize, minimum: bool },
}

#[derive(serde::Deserialize)]
struct AnalysisProfile {
    available: usize,
    analysis_frame: usize,
    stack_check_frame: usize,
    text_choice: usize,
    loop_choice: usize,
    action_assertion_backreference: usize,
    negative_submatch: usize,
}

static ANALYSIS_PROFILE: std::sync::LazyLock<AnalysisProfile> = std::sync::LazyLock::new(|| {
    #[derive(serde::Deserialize)]
    struct Profile {
        analysis_stack: AnalysisProfile,
    }
    serde_json::from_str::<Profile>(include_str!("../../data/js-regex-resources.json"))
        .expect("embedded reference resource profile")
        .analysis_stack
});

struct Graph {
    contains_anchor: bool,
    blocks: Vec<Block>,
    not_at_start: Vec<bool>,
    loops: u32,
    optimize: bool,
}

#[derive(Clone, Copy)]
struct State {
    next: usize,
    flags: u8,
    backwards: bool,
    expansion: u32,
}

fn capture_range(tree: &Tree) -> Range<u16> {
    let mut range: Option<Range<u16>> = None;
    let mut pending = vec![tree];
    while let Some(tree) = pending.pop() {
        if !tree.captures {
            continue;
        }
        match &tree.kind {
            Kind::Capture(id, body) => {
                // compile() checks the complete graph's capture count before
                // lowering any capture identifiers.
                let id = u16::try_from(*id).expect("validated capture count");
                let bounds = range.get_or_insert(id..id + 1);
                bounds.start = bounds.start.min(id);
                bounds.end = bounds.end.max(id + 1);
                pending.push(body);
            }
            Kind::Sequence(children) | Kind::Choice(children) => pending.extend(children),
            Kind::Group(_, body) | Kind::Lookaround { body, .. } | Kind::Repeat { body, .. } => {
                pending.push(body);
            }
            _ => {}
        }
    }
    range.unwrap_or(0..0)
}

fn anchored(tree: &Tree, input: &[u16], flags: u8, depth: usize) -> bool {
    if depth > 100 {
        return false;
    }
    match &tree.kind {
        Kind::Other(span) => input[span.clone()] == ['^' as u16] && flags & 2 == 0,
        Kind::Group(flags, body) => anchored(body, input, *flags, depth + 1),
        Kind::Capture(_, body) => anchored(body, input, flags, depth + 1),
        Kind::Lookaround {
            negative: false,
            backwards: false,
            body,
        } => anchored(body, input, flags, depth + 1),
        Kind::Choice(choices) => {
            !choices.is_empty()
                && choices
                    .iter()
                    .all(|tree| anchored(tree, input, flags, depth + 1))
        }
        Kind::Sequence(terms) => {
            for term in terms {
                if anchored(term, input, flags, depth + 1) {
                    return true;
                }
                if term.max > 0 {
                    return false;
                }
            }
            false
        }
        _ => false,
    }
}

impl Graph {
    fn add(&mut self, block: Block) -> usize {
        self.blocks.push(block);
        self.not_at_start.push(false);
        self.blocks.len() - 1
    }

    fn build(&mut self, tree: &Tree, next: usize) -> usize {
        enum Work<'a> {
            Build(&'a Tree, State),
            Sequence(&'a [Tree], State),
            SequenceNext(&'a [Tree], State),
            Choice(usize),
            Capture(usize),
            Flags(u8),
            Repeat {
                body: &'a Tree,
                min: u32,
                max: u32,
                greedy: bool,
                state: State,
                not_at_start: bool,
            },
            Required {
                body: &'a Tree,
                count: u32,
                state: State,
            },
            Optional {
                body: &'a Tree,
                count: u32,
                greedy: bool,
                state: State,
                not_at_start: bool,
            },
            OptionalNext {
                body: &'a Tree,
                count: u32,
                greedy: bool,
                state: State,
                not_at_start: bool,
            },
            Loop {
                block: usize,
                id: u32,
                min: u32,
                max: u32,
                greedy: bool,
                groups: Range<u16>,
                nullable: bool,
                body_min: u32,
                next: usize,
            },
            Lookaround {
                negative: bool,
                backwards: bool,
                groups: Range<u16>,
                end: usize,
                next: usize,
            },
        }
        let mut pending = vec![Work::Build(
            tree,
            State {
                next,
                flags: 0,
                backwards: false,
                expansion: 1,
            },
        )];
        let mut results = vec![];
        while let Some(work) = pending.pop() {
            match work {
                Work::Build(tree, state) => match &tree.kind {
                    Kind::Empty => results.push(state.next),
                    Kind::Text(elements, possible) => results.push(self.add(Block::Text {
                        elements: elements.clone(),
                        possible: *possible,
                        flags: state.flags,
                        backwards: state.backwards,
                        next: state.next,
                    })),
                    Kind::Other(span) => results.push(self.add(Block::Other {
                        span: span.clone(),
                        flags: state.flags,
                        next: state.next,
                    })),
                    Kind::Sequence(terms) => pending.push(Work::Sequence(terms, state)),
                    Kind::Choice(alternatives) => {
                        pending.push(Work::Choice(alternatives.len()));
                        pending.extend(
                            alternatives
                                .iter()
                                .rev()
                                .map(|tree| Work::Build(tree, state)),
                        );
                    }
                    Kind::Group(flags, body) => {
                        if *flags == state.flags {
                            pending.push(Work::Build(body, state));
                            continue;
                        }
                        let next = self.add(Block::Flags {
                            flags: state.flags,
                            next: state.next,
                        });
                        pending.push(Work::Flags(*flags));
                        pending.push(Work::Build(
                            body,
                            State {
                                flags: *flags,
                                next,
                                ..state
                            },
                        ));
                    }
                    Kind::Capture(id, body) => {
                        let next = self.add(Block::Capture {
                            id: *id,
                            start: false,
                            next: state.next,
                        });
                        pending.push(Work::Capture(*id));
                        pending.push(Work::Build(body, State { next, ..state }));
                    }
                    Kind::Repeat {
                        body,
                        min,
                        max,
                        greedy,
                    } => {
                        pending.push(Work::Repeat {
                            body,
                            min: *min,
                            max: *max,
                            greedy: *greedy,
                            state,
                            not_at_start: false,
                        });
                    }
                    Kind::Lookaround {
                        body,
                        negative,
                        backwards,
                    } => {
                        let end = self.add(Block::SubmatchEnd {
                            clear: body.captures,
                            negative: *negative,
                            next: state.next,
                        });
                        pending.push(Work::Lookaround {
                            negative: *negative,
                            backwards: *backwards,
                            groups: capture_range(body),
                            end,
                            next: state.next,
                        });
                        pending.push(Work::Build(
                            body,
                            State {
                                next: end,
                                backwards: *backwards,
                                ..state
                            },
                        ));
                    }
                },
                Work::Sequence(terms, state) => {
                    let split = if state.backwards {
                        terms.split_first()
                    } else {
                        terms.split_last()
                    };
                    if let Some((term, rest)) = split {
                        pending.push(Work::SequenceNext(rest, state));
                        pending.push(Work::Build(term, state));
                    } else {
                        results.push(state.next);
                    }
                }
                Work::SequenceNext(rest, state) => {
                    let next = results.pop().unwrap();
                    pending.push(Work::Sequence(rest, State { next, ..state }));
                }
                Work::Choice(count) => {
                    let choices = results.split_off(results.len() - count);
                    results.push(self.add(Block::Choice(choices)));
                }
                Work::Capture(id) => {
                    let next = results.pop().unwrap();
                    // Both capture instructions use the cursor's direction in
                    // regress, so their order remains begin, contents, end.
                    results.push(self.add(Block::Capture {
                        id,
                        start: true,
                        next,
                    }));
                }
                Work::Flags(flags) => {
                    let next = results.pop().unwrap();
                    results.push(self.add(Block::Flags { flags, next }));
                }
                Work::Repeat {
                    body,
                    min,
                    max,
                    greedy,
                    state,
                    not_at_start,
                } => {
                    if max == 0 {
                        results.push(state.next);
                        continue;
                    }
                    // RegExpQuantifier::ToNode unrolls short nonempty bodies
                    // without captures. These distinct source nodes matter to
                    // the subsequent emission budget, even when their matching
                    // behavior is the same as a counted loop.
                    if self.optimize && body.min > 0 && !body.captures {
                        let factor = state
                            .expansion
                            .saturating_mul(min.saturating_add(u32::from(max != min)));
                        if min > 0 && min <= 3 && factor <= 6 {
                            let state = State {
                                expansion: factor,
                                ..state
                            };
                            pending.push(Work::Required {
                                body,
                                count: min,
                                state,
                            });
                            pending.push(Work::Repeat {
                                body,
                                min: 0,
                                max: if max == INFINITY { max } else { max - min },
                                greedy,
                                state,
                                not_at_start: true,
                            });
                            continue;
                        }
                        let factor = state.expansion.saturating_mul(max);
                        if min == 0 && max <= 3 && factor <= 6 {
                            results.push(state.next);
                            pending.push(Work::Optional {
                                body,
                                count: max,
                                greedy,
                                state: State {
                                    expansion: factor,
                                    ..state
                                },
                                not_at_start,
                            });
                            continue;
                        }
                    }
                    let id = self.loops;
                    self.loops += 1;
                    let block = self.add(Block::Goal);
                    self.not_at_start[block] = not_at_start && !state.backwards;
                    let mut next = self.add(Block::LoopAgain(block));
                    if min > 0 || max < INFINITY {
                        next = self.add(Block::Action {
                            action: Action::Deferred,
                            next,
                        });
                    }
                    if body.min == 0 {
                        next = self.add(Block::Action {
                            action: Action::Empty {
                                position: block,
                                minimum: min > 0,
                            },
                            next,
                        });
                    }
                    pending.push(Work::Loop {
                        block,
                        id,
                        min,
                        max,
                        greedy,
                        groups: capture_range(body),
                        nullable: body.min == 0,
                        body_min: body.min,
                        next: state.next,
                    });
                    pending.push(Work::Build(body, State { next, ..state }));
                }
                Work::Required { body, count, state } => {
                    if count != 0 {
                        let next = results.pop().unwrap();
                        pending.push(Work::Required {
                            body,
                            count: count - 1,
                            state,
                        });
                        pending.push(Work::Build(body, State { next, ..state }));
                    }
                }
                Work::Optional {
                    body,
                    count,
                    greedy,
                    state,
                    not_at_start,
                } => {
                    if count != 0 {
                        let next = results.pop().unwrap();
                        pending.push(Work::OptionalNext {
                            body,
                            count,
                            greedy,
                            state,
                            not_at_start,
                        });
                        pending.push(Work::Build(body, State { next, ..state }));
                    }
                }
                Work::OptionalNext {
                    body,
                    count,
                    greedy,
                    state,
                    not_at_start,
                } => {
                    let body_node = results.pop().unwrap();
                    let choice = self.add(Block::Choice(if greedy {
                        vec![body_node, state.next]
                    } else {
                        vec![state.next, body_node]
                    }));
                    self.not_at_start[choice] = not_at_start && !state.backwards;
                    results.push(choice);
                    pending.push(Work::Optional {
                        body,
                        count: count - 1,
                        greedy,
                        state,
                        not_at_start,
                    });
                }
                Work::Loop {
                    block,
                    id,
                    min,
                    max,
                    greedy,
                    groups,
                    nullable,
                    body_min,
                    next,
                } => {
                    let mut body = results.pop().unwrap();
                    if nullable {
                        body = self.add(Block::Action {
                            action: Action::Store(block),
                            next: body,
                        });
                    }
                    if !groups.is_empty() {
                        body = self.add(Block::Action {
                            action: Action::Deferred,
                            next: body,
                        });
                    }
                    self.blocks[block] = Block::Loop {
                        id,
                        min,
                        max,
                        greedy,
                        groups,
                        nullable,
                        body_min,
                        body,
                        next,
                    };
                    results.push(if min > 0 || max < INFINITY {
                        self.add(Block::Action {
                            action: Action::Initialize,
                            next: block,
                        })
                    } else {
                        block
                    });
                }
                Work::Lookaround {
                    negative,
                    backwards,
                    groups,
                    end,
                    next,
                } => {
                    let body = results.pop().unwrap();
                    results.push(self.add(Block::Lookaround {
                        negative,
                        backwards,
                        groups,
                        body,
                        end,
                        next,
                    }));
                }
            }
        }
        debug_assert_eq!(results.len(), 1);
        results.pop().unwrap()
    }

    fn analysis_fits(&self, start: usize, anchored: bool) -> bool {
        // Analysis::EnsureAnalyzed checks the stack before its visited guard.
        // Costs come from the fixed Linux x64 reference ABI, not the host stack.
        let profile = &*ANALYSIS_PROFILE;
        let root_frame = if anchored {
            0
        } else if self.contains_anchor {
            profile.text_choice
        } else {
            profile.loop_choice
        };
        let mut pending = vec![(start, profile.analysis_frame + root_frame)];
        let mut visited = vec![false; self.blocks.len()];
        while let Some((id, cost)) = pending.pop() {
            if cost + profile.stack_check_frame > profile.available {
                return false;
            }
            if visited[id] {
                continue;
            }
            visited[id] = true;
            let frame = match &self.blocks[id] {
                Block::Text { .. } | Block::Choice(_) => profile.text_choice,
                Block::Loop { .. } => profile.loop_choice,
                Block::Goal | Block::Fail | Block::LoopAgain(_) => 0,
                Block::Lookaround { negative: true, .. } => profile.negative_submatch,
                _ => profile.action_assertion_backreference,
            };
            let cost = cost + frame;
            match &self.blocks[id] {
                Block::Text { next, .. }
                | Block::Other { next, .. }
                | Block::Capture { next, .. }
                | Block::Flags { next, .. }
                | Block::Action { next, .. }
                | Block::SubmatchEnd {
                    negative: false,
                    next,
                    ..
                }
                | Block::LoopAgain(next) => pending.push((*next, cost)),
                Block::Choice(choices) => {
                    pending.extend(choices.iter().rev().map(|id| (*id, cost)))
                }
                Block::Loop { body, next, .. } => {
                    pending.push((*body, cost));
                    pending.push((*next, cost));
                }
                Block::Lookaround {
                    negative,
                    body,
                    next,
                    ..
                } => {
                    if *negative {
                        pending.push((*next, cost));
                    }
                    pending.push((*body, cost));
                }
                _ => {}
            }
        }
        true
    }

    fn analyze(&self, start: usize, classes: &mut [Class]) {
        enum Work {
            Visit(usize),
            LoopBody { body: usize, original: u8 },
            Restore(u8),
        }
        let mut pending = vec![Work::Visit(start)];
        let mut visited = vec![false; self.blocks.len()];
        let mut flags = 0;
        while let Some(work) = pending.pop() {
            let id = match work {
                Work::Visit(id) => id,
                Work::LoopBody { body, original } => {
                    pending.push(Work::Restore(flags));
                    flags = original;
                    pending.push(Work::Visit(body));
                    continue;
                }
                Work::Restore(value) => {
                    flags = value;
                    continue;
                }
            };
            if visited[id] {
                continue;
            }
            visited[id] = true;
            match &self.blocks[id] {
                Block::Text { elements, next, .. } => {
                    for element in elements {
                        if let TextElement::Class(ids) = element {
                            for id in ids {
                                classes[*id].folded |= flags & 1 != 0;
                            }
                        }
                    }
                    pending.push(Work::Visit(*next));
                }
                Block::Flags { flags: value, next } => {
                    flags = *value;
                    pending.push(Work::Visit(*next));
                }
                Block::Other { next, .. }
                | Block::Capture { next, .. }
                | Block::Action { next, .. }
                | Block::SubmatchEnd {
                    negative: false,
                    next,
                    ..
                }
                | Block::LoopAgain(next) => pending.push(Work::Visit(*next)),
                Block::Choice(choices) => {
                    pending.extend(choices.iter().rev().map(|id| Work::Visit(*id)))
                }
                Block::Loop { body, next, .. } => {
                    pending.push(Work::LoopBody {
                        body: *body,
                        original: flags,
                    });
                    pending.push(Work::Visit(*next));
                }
                Block::Lookaround {
                    negative,
                    body,
                    next,
                    ..
                } => {
                    if *negative {
                        pending.push(Work::Visit(*next));
                    }
                    pending.push(Work::Visit(*body));
                }
                Block::Goal | Block::Fail | Block::SubmatchEnd { negative: true, .. } => {}
                Block::Search { .. } | Block::Quick { .. } | Block::NegativeChoice { .. } => {
                    unreachable!()
                }
            }
        }
    }

    fn filter(
        &mut self,
        id: usize,
        depth: i32,
        active: &mut [bool],
        replacements: &mut [Option<Option<usize>>],
    ) -> Option<usize> {
        if let Some(result) = replacements[id] {
            return result;
        }
        if depth < 0 || active[id] {
            return Some(id);
        }
        active[id] = true;
        let mut block = self.blocks[id].clone();
        let mut replacement_block = None;
        let result = match &mut block {
            Block::Goal => Some(id),
            Block::Fail
            | Block::Text {
                possible: false, ..
            } => None,
            Block::Flags { next, .. }
            | Block::Text { next, .. }
            | Block::Other { next, .. }
            | Block::Capture { next, .. }
            | Block::Action { next, .. }
            | Block::SubmatchEnd {
                negative: false,
                next,
                ..
            } => self
                .filter(*next, depth - 2, active, replacements)
                .map(|replacement| {
                    *next = replacement;
                    id
                }),
            Block::Choice(choices) => {
                let mut survivors = Vec::with_capacity(choices.len());
                for next in choices.iter_mut() {
                    if let Some(replacement) = self.filter(*next, depth - 1, active, replacements) {
                        *next = replacement;
                        survivors.push(replacement);
                    }
                }
                match survivors.as_slice() {
                    [] => None,
                    [only] => Some(*only),
                    _ => {
                        *choices = survivors;
                        Some(id)
                    }
                }
            }
            Block::Loop {
                min,
                max,
                body,
                next,
                ..
            } => self
                .filter(*next, depth - 1, active, replacements)
                .and_then(|replacement| {
                    if *min > 0 || *max < INFINITY {
                        // LoopChoice checks whether its continuation can survive,
                        // but guarded ChoiceNode returns before replacing edges.
                        return Some(id);
                    }
                    *next = replacement;
                    self.filter(*body, depth - 2, active, replacements).map_or(
                        Some(*next),
                        |replacement| {
                            *body = replacement;
                            Some(id)
                        },
                    )
                }),
            Block::LoopAgain(next) => {
                self.filter(*next, depth, active, replacements)
                    .map(|replacement| {
                        *next = replacement;
                        id
                    })
            }
            Block::Lookaround {
                negative: false,
                body,
                end,
                next,
                ..
            } => self
                .filter(*body, depth - 2, active, replacements)
                .map(|replacement| {
                    *body = replacement;
                    if let Block::SubmatchEnd {
                        next: continuation, ..
                    } = self.blocks[*end]
                    {
                        *next = continuation;
                    }
                    id
                }),
            Block::Lookaround {
                negative: true,
                body,
                next,
                ..
            } => self
                // BeginNegativeSubmatch (-2), then its special Choice (-1).
                .filter(*next, depth - 3, active, replacements)
                .map(|replacement| {
                    *next = replacement;
                    match self.filter(*body, depth - 3, active, replacements) {
                        Some(replacement) => {
                            *body = replacement;
                            id
                        }
                        None => {
                            // The projected BeginNegativeSubmatch survives even
                            // when its Choice is replaced by the continuation.
                            // Guarded loops can retain an edge to this original
                            // action rather than to its cached replacement.
                            replacement_block = Some(Block::Action {
                                action: Action::Begin,
                                next: *next,
                            });
                            id
                        }
                    }
                }),
            Block::SubmatchEnd { negative: true, .. } => Some(id),
            Block::Search { .. } | Block::Quick { .. } | Block::NegativeChoice { .. } => {
                unreachable!()
            }
        };
        self.blocks[id] = replacement_block.unwrap_or(block);
        active[id] = false;
        replacements[id] = Some(result);
        result
    }
}

struct Assembler<'a> {
    input: &'a [u16],
    classes: &'a [Class],
    result: CompiledRegex,
    labels: Vec<usize>,
    jumps: Vec<(usize, usize)>,
}

fn flags(bits: u8) -> regress::Flags {
    regress::Flags {
        icase: bits & 1 != 0,
        multiline: bits & 2 != 0,
        dot_all: bits & 4 != 0,
        ..regress::Flags::default()
    }
}

impl Assembler<'_> {
    fn jump(&mut self, block: usize) {
        self.jumps.push((self.result.insns.len(), block));
        self.result.insns.push(Insn::Jump { target: 0 });
    }

    fn leaf(&mut self, pattern: &[u16], bits: u8) -> Result<(), regress::Error> {
        let leaf =
            regress::Regex::from_unicode(pattern.iter().copied().map(u32::from), flags(bits))?
                .into_program();
        debug_assert_eq!(leaf.groups, 0);
        debug_assert_eq!(leaf.loops, 0);
        let brackets = self.result.brackets.len();
        self.result.brackets.extend(leaf.brackets);
        for mut insn in leaf.insns {
            match &mut insn {
                Insn::Goal => continue,
                Insn::Bracket(index) => *index += brackets,
                _ => {}
            }
            self.result.insns.push(insn);
        }
        Ok(())
    }

    fn text(&mut self, element: &TextElement, bits: u8) -> Result<(), regress::Error> {
        match element {
            TextElement::Skip => self.result.insns.push(Insn::MatchAny {}),
            TextElement::Literal(literal) => {
                self.leaf(
                    &format!(r"\u{:04x}", literal.code)
                        .encode_utf16()
                        .collect::<Vec<_>>(),
                    bits,
                )?;
            }
            TextElement::Raw(span) => self.leaf(&self.input[span.clone()], bits)?,
            TextElement::Class(ids) => {
                // FixSingleCharacterDisjunctions combines literals into one
                // class. All of its source literals share the analysis flags.
                if ids.len() > 1 {
                    let mut pattern = vec!['[' as u16];
                    for id in ids {
                        let class = &self.classes[*id];
                        let code = class.literal.expect("combined literal class");
                        pattern.extend(format!(r"\u{code:04x}").encode_utf16());
                    }
                    pattern.push(']' as u16);
                    self.leaf(&pattern, u8::from(self.classes[ids[0]].folded))?;
                } else {
                    let class = &self.classes[ids[0]];
                    let pattern = if let Some(code) = class.literal {
                        format!(r"\u{code:04x}").encode_utf16().collect::<Vec<_>>()
                    } else {
                        self.input[class.span.clone()].to_vec()
                    };
                    self.leaf(&pattern, u8::from(class.folded))?;
                }
            }
        }
        Ok(())
    }

    fn assemble(mut self, graph: Graph, start: usize) -> Result<regress::Regex, regress::Error> {
        self.jump(start);
        for (id, block) in graph.blocks.iter().enumerate() {
            self.labels[id] = self.result.insns.len();
            match block {
                Block::NegativeChoice { .. } => unreachable!(),
                Block::Goal | Block::SubmatchEnd { negative: true, .. } => {
                    self.result.insns.push(Insn::Goal)
                }
                Block::SubmatchEnd {
                    negative: false,
                    next,
                    ..
                } => {
                    self.jumps.push((self.result.insns.len(), *next));
                    self.result.insns.push(Insn::GoalAt { continuation: 0 });
                }
                Block::Action { next, .. } => self.jump(*next),
                Block::Search { first, later } => {
                    self.result.insns.push(Insn::Alt {
                        secondary: (self.result.insns.len() + 3) as u32,
                    });
                    self.result
                        .insns
                        .push(Insn::StartOfLine { multiline: false });
                    self.jump(*first);
                    self.jumps.push((self.result.insns.len(), *later));
                    self.result.insns.push(Insn::Lookahead {
                        negate: true,
                        start_group: 0,
                        end_group: 0,
                        continuation: 0,
                    });
                    self.result
                        .insns
                        .push(Insn::StartOfLine { multiline: false });
                    self.result.insns.push(Insn::Goal);
                }
                Block::Fail => {
                    self.leaf(&"[]".encode_utf16().collect::<Vec<_>>(), 0)?;
                }
                Block::Quick {
                    mask,
                    value,
                    one_byte,
                    next,
                } => {
                    self.jumps.push((self.result.insns.len(), *next));
                    self.result.insns.push(Insn::Lookahead {
                        negate: false,
                        start_group: 0,
                        end_group: 0,
                        continuation: 0,
                    });
                    let mut template = regress::Regex::with_flags(
                        "[\u{100}-\u{110}a-z]",
                        regress::Flags {
                            no_opt: true,
                            ..regress::Flags::default()
                        },
                    )?
                    .into_program();
                    let mut bracket = template.brackets.pop().expect("template bracket");
                    bracket.cps = Default::default();
                    let limit = if *one_byte { 255_u32 } else { 65535 };
                    for code in 0..=limit {
                        if code & u32::from(*mask) == u32::from(*value) {
                            bracket.cps.add_one(code);
                        }
                    }
                    self.result
                        .insns
                        .push(Insn::Bracket(self.result.brackets.len()));
                    self.result.brackets.push(bracket);
                    self.result.insns.push(Insn::Goal);
                }
                Block::Flags { flags, next } => {
                    debug_assert_eq!(flags & !7, 0, "normalized modifier flags");
                    self.jump(*next);
                }
                Block::Text {
                    elements,
                    flags,
                    backwards,
                    next,
                    ..
                } => {
                    if *backwards {
                        for element in elements.iter().rev() {
                            self.text(element, *flags)?;
                        }
                    } else {
                        for element in elements {
                            self.text(element, *flags)?;
                        }
                    }
                    self.jump(*next);
                }
                Block::Other { span, flags, next } => {
                    let pattern = &self.input[span.clone()];
                    if pattern.len() >= 2
                        && pattern[0] == '\\' as u16
                        && (b'1' as u16..=b'9' as u16).contains(&pattern[1])
                    {
                        let group = pattern[1..]
                            .iter()
                            .fold(0, |n, digit| n * 10 + u32::from(*digit - '0' as u16));
                        self.result.insns.push(Insn::BackRef {
                            group: group - 1,
                            icase: flags & 1 != 0,
                        });
                    } else {
                        self.leaf(pattern, *flags)?;
                    }
                    self.jump(*next);
                }
                Block::Capture { id, start, next } => {
                    if *id == usize::MAX {
                        self.jump(*next);
                        continue;
                    }
                    let id = u16::try_from(*id).expect("validated capture count");
                    self.result.insns.push(if *start {
                        Insn::BeginCaptureGroup(id)
                    } else {
                        Insn::EndCaptureGroup(id)
                    });
                    self.jump(*next);
                }
                Block::Choice(choices) => {
                    for (index, choice) in choices.iter().enumerate() {
                        if index + 1 < choices.len() {
                            self.result.insns.push(Insn::Alt {
                                secondary: (self.result.insns.len() + 2) as u32,
                            });
                        }
                        self.jump(*choice);
                    }
                }
                Block::Loop {
                    id,
                    min,
                    max,
                    greedy,
                    groups,
                    body,
                    next,
                    ..
                } => {
                    self.jumps.push((self.result.insns.len(), *next));
                    self.result.insns.push(Insn::EnterLoop(LoopFields {
                        loop_id: u16::try_from(*id).map_err(|_| regress::Error {
                            text: "Regular expression too large".into(),
                        })?,
                        min_iters: *min as usize,
                        max_iters: if *max == INFINITY {
                            usize::MAX
                        } else {
                            *max as usize
                        },
                        greedy: *greedy,
                        exit: 0,
                    }));
                    for group in groups.clone() {
                        self.result.insns.push(Insn::ResetCaptureGroup(group));
                    }
                    self.jump(*body);
                }
                Block::LoopAgain(enter) => {
                    self.jumps.push((self.result.insns.len(), *enter));
                    self.result.insns.push(Insn::LoopAgain { begin: 0 });
                }
                Block::Lookaround {
                    negative,
                    backwards,
                    groups,
                    body,
                    next,
                    ..
                } => {
                    self.jumps.push((self.result.insns.len(), *next));
                    self.result.insns.push(if *backwards {
                        Insn::Lookbehind {
                            negate: *negative,
                            start_group: groups.start,
                            end_group: groups.end,
                            continuation: 0,
                        }
                    } else {
                        Insn::Lookahead {
                            negate: *negative,
                            start_group: groups.start,
                            end_group: groups.end,
                            continuation: 0,
                        }
                    });
                    self.jump(*body);
                }
            }
        }
        for (offset, block) in self.jumps {
            let label = u32::try_from(self.labels[block]).expect("V8 graph instruction limit");
            match &mut self.result.insns[offset] {
                Insn::GoalAt { continuation } => *continuation = label,
                Insn::Jump { target } => *target = label,
                Insn::EnterLoop(fields) => fields.exit = label,
                Insn::LoopAgain { begin } => *begin = label,
                Insn::Lookahead { continuation, .. } | Insn::Lookbehind { continuation, .. } => {
                    *continuation = label
                }
                _ => unreachable!("jump fixup instruction"),
            }
        }
        let mut loop_targets = vec![];
        for (offset, insn) in self.result.insns.iter().enumerate() {
            if let Insn::LoopAgain { begin } = insn {
                let mut target = *begin;
                let mut hops = 0;
                while let Insn::Jump { target: next } = self.result.insns[target as usize] {
                    target = next;
                    hops += 1;
                    if hops > self.result.insns.len() {
                        return Err(regress::Error {
                            text: "Cyclic loop label".into(),
                        });
                    }
                }
                loop_targets.push((offset, target));
            }
        }
        for (offset, begin) in loop_targets {
            self.result.insns[offset] = Insn::LoopAgain { begin };
        }
        Ok(regress::Regex::from(self.result))
    }
}

fn prepare_graph(
    input: &[u16],
    tree: &Tree,
    one_byte: bool,
    optimize: bool,
    contains_anchor: bool,
) -> (Graph, usize) {
    let mut graph = Graph {
        contains_anchor,
        blocks: vec![Block::Goal],
        not_at_start: vec![false],
        loops: 0,
        optimize,
    };
    let end = graph.add(Block::Capture {
        id: usize::MAX,
        start: false,
        next: 0,
    });
    let body = graph.build(tree, end);
    let start = graph.add(Block::Capture {
        id: usize::MAX,
        start: true,
        next: body,
    });
    let start = if one_byte {
        let count = graph.blocks.len();
        let mut active = vec![false; count];
        let mut replacements = vec![None; count];
        // PreprocessRegExp wraps capture zero in the unanchored .*? loop.
        // Its continue edge costs one FilterOneByte level before capture zero.
        let depth = if anchored(tree, input, 0, 0) { 100 } else { 99 };
        graph
            .filter(start, depth, &mut active, &mut replacements)
            .and_then(|start| graph.filter(start, depth, &mut active, &mut replacements))
            .unwrap_or_else(|| graph.add(Block::Fail))
    } else {
        start
    };
    (graph, start)
}

#[cfg(test)]
pub(super) fn analyzed_classes(
    input: &[u16],
    tree: &Tree,
    classes: &mut [Class],
    one_byte: bool,
    contains_anchor: bool,
) {
    let (graph, start) = prepare_graph(
        input,
        tree,
        one_byte,
        input.len() <= 20 * 1024,
        contains_anchor,
    );
    graph.analyze(start, classes);
}

pub(super) fn compile(
    input: &[u16],
    tree: &Tree,
    classes: &[Class],
    captures: usize,
    one_byte: bool,
    optimize: bool,
    contains_anchor: bool,
) -> Result<regress::Regex, regress::Error> {
    if captures > 32767 {
        return Err(regress::Error {
            text: "Too many captures".into(),
        });
    }
    let (graph, start) = prepare_graph(input, tree, one_byte, optimize, contains_anchor);
    if !graph.analysis_fits(start, anchored(tree, input, 0, 0)) {
        return Err(regress::Error {
            text: "Stack overflow".into(),
        });
    }
    let mut classes = classes.to_vec();
    graph.analyze(start, &mut classes);
    let (graph, start) = emission::emit(
        graph,
        start,
        input,
        &classes,
        one_byte,
        anchored(tree, input, 0, 0),
    );
    let result = CompiledRegex {
        insns: vec![],
        brackets: vec![],
        start_pred: StartPredicate::Arbitrary,
        loops: graph.loops,
        groups: u32::try_from(captures).expect("validated capture count"),
        group_names: Box::new([]),
        flags: regress::Flags::default(),
    };
    let assembler = Assembler {
        input,
        classes: &classes,
        result,
        labels: vec![0; graph.blocks.len()],
        jumps: vec![],
    };
    assembler.assemble(graph, start)
}
