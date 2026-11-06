// Port of Tailwind CSS 4.3.4 selector-parser.ts. See LICENSE.tailwind.
use super::is_js_whitespace;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Selector {
    List { nodes: Vec<Selector> },
    Combinator { value: String },
    Complex { nodes: Vec<Selector> },
    Compound { nodes: Vec<Selector> },
    Selector { value: String },
    Function { value: String, nodes: Vec<Selector> },
    Value { value: String },
}

impl Selector {
    pub fn children_mut(&mut self) -> Option<&mut Vec<Self>> {
        match self {
            Self::List { nodes }
            | Self::Complex { nodes }
            | Self::Compound { nodes }
            | Self::Function { nodes, .. } => Some(nodes),
            _ => None,
        }
    }

    pub fn is_pseudo_element(&self) -> bool {
        let value = match self {
            Self::Selector { value } | Self::Function { value, .. } => value,
            _ => return false,
        };
        value.starts_with("::")
            || matches!(
                value.to_ascii_lowercase().as_str(),
                ":before" | ":after" | ":first-line" | ":first-letter"
            )
    }
}

pub fn to_css(nodes: &[Selector], minify: bool) -> String {
    let mut output = String::new();
    for node in nodes {
        match node {
            Selector::Selector { value } | Selector::Value { value } => output.push_str(value),
            Selector::Combinator { value } => {
                if !minify && value != " " {
                    output.push(' ');
                }
                output.push_str(value);
                if !minify && value != " " {
                    output.push(' ');
                }
            }
            Selector::Function { value, nodes } => {
                output.push_str(value);
                output.push('(');
                output.push_str(&to_css(nodes, minify));
                output.push(')');
            }
            Selector::List { nodes } => {
                for (i, node) in nodes.iter().enumerate() {
                    if i > 0 {
                        output.push_str(if minify { "," } else { ", " });
                    }
                    output.push_str(&to_css(std::slice::from_ref(node), minify));
                }
            }
            Selector::Complex { nodes } | Selector::Compound { nodes } => {
                output.push_str(&to_css(nodes, minify))
            }
        }
    }
    output
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    List,
    Combinator,
    Complex,
    Compound,
    Selector,
    Function,
    Value,
}

struct Node {
    kind: Kind,
    value: String,
    children: usize,
}

#[derive(Default)]
struct Arena {
    nodes: Vec<Node>,
    groups: Vec<Vec<usize>>,
}

impl Arena {
    fn group(&mut self, nodes: Vec<usize>) -> usize {
        let index = self.groups.len();
        self.groups.push(nodes);
        index
    }

    fn node(&mut self, kind: Kind, value: String, children: Vec<usize>) -> usize {
        let children = self.group(children);
        let index = self.nodes.len();
        self.nodes.push(Node {
            kind,
            value,
            children,
        });
        index
    }

    fn current(&mut self, group: usize, combinator: bool) -> usize {
        let mut nodes = std::mem::take(&mut self.groups[group]);
        if nodes.len() == 1 {
            nodes.pop().unwrap()
        } else {
            self.node(
                if combinator {
                    Kind::Complex
                } else {
                    Kind::Compound
                },
                String::new(),
                nodes,
            )
        }
    }

    fn append(&mut self, group: usize, node: usize) {
        if let Some(&last) = self.groups[group].last() {
            match self.nodes[last].kind {
                Kind::Compound => {
                    let children = self.nodes[last].children;
                    self.groups[children].push(node);
                    return;
                }
                Kind::List | Kind::Combinator => {}
                _ => {
                    let compound = self.node(Kind::Compound, String::new(), vec![last, node]);
                    *self.groups[group].last_mut().unwrap() = compound;
                    return;
                }
            }
        }
        self.groups[group].push(node);
    }

    fn flush(&mut self, group: usize, buffer: &mut String) {
        if !buffer.is_empty() {
            let node = self.node(Kind::Selector, std::mem::take(buffer), Vec::new());
            self.append(group, node);
        }
    }

    fn finish(&mut self, target: usize, list: Option<usize>, combinator: bool) {
        if let Some(list) = list {
            let item = self.current(target, combinator);
            let children = self.nodes[list].children;
            self.groups[children].push(item);
        } else if combinator {
            let nodes = std::mem::take(&mut self.groups[target]);
            let node = self.node(Kind::Complex, String::new(), nodes);
            self.groups[target].push(node);
        }
    }

    fn take(&mut self, index: usize) -> Selector {
        let node = &mut self.nodes[index];
        let kind = node.kind;
        let value = std::mem::take(&mut node.value);
        let indices = std::mem::take(&mut self.groups[node.children]);
        let nodes = indices.into_iter().map(|i| self.take(i)).collect();
        match kind {
            Kind::List => Selector::List { nodes },
            Kind::Combinator => Selector::Combinator { value },
            Kind::Complex => Selector::Complex { nodes },
            Kind::Compound => Selector::Compound { nodes },
            Kind::Selector => Selector::Selector { value },
            Kind::Function => Selector::Function { value, nodes },
            Kind::Value => Selector::Value { value },
        }
    }
}

const fn is_combinator(c: char) -> bool {
    matches!(c, '>' | '\n' | ' ' | '+' | '\t' | '~')
}

pub fn parse(input: &str) -> Vec<Selector> {
    let input = input.replace("\r\n", "\n");
    let chars: Vec<char> = input.chars().collect();
    let mut arena = Arena::default();
    let mut target = arena.group(Vec::new());
    let mut contains_combinator = false;
    let mut current_list: Option<usize> = None;
    let mut contexts = Vec::new();
    let mut buffer = String::new();
    let mut i = 0;
    while i < chars.len() {
        match chars[i] {
            ',' => {
                arena.flush(target, &mut buffer);
                while i + 1 < chars.len() && matches!(chars[i + 1], '\n' | ' ' | '\t') {
                    i += 1;
                }
                let item = arena.current(target, contains_combinator);
                if let Some(list) = current_list {
                    let children = arena.nodes[list].children;
                    arena.groups[children].push(item);
                } else {
                    let list = arena.node(Kind::List, String::new(), vec![item]);
                    arena.groups[target].push(list);
                    current_list = Some(list);
                }
                target = arena.group(Vec::new());
                contains_combinator = false;
            }
            c if is_combinator(c) => {
                arena.flush(target, &mut buffer);
                let start = i;
                while i + 1 < chars.len() && is_combinator(chars[i + 1]) {
                    i += 1;
                }
                let raw: String = chars[start..=i].iter().collect();
                let value = raw.trim_matches(is_js_whitespace);
                let end = i + 1;
                if value.is_empty()
                    && (arena.groups[target].is_empty() || end >= chars.len() || chars[end] == ',')
                {
                    i += 1;
                    continue;
                }
                let node = arena.node(
                    Kind::Combinator,
                    if value.is_empty() {
                        " ".into()
                    } else {
                        value.into()
                    },
                    Vec::new(),
                );
                arena.groups[target].push(node);
                contains_combinator = true;
            }
            '(' => {
                let name = std::mem::take(&mut buffer);
                let node = arena.node(Kind::Function, name.clone(), Vec::new());
                if !matches!(
                    name.as_str(),
                    ":not" | ":where" | ":has" | ":is" | ":-moz-any" | ":-webkit-any"
                ) {
                    let start = i + 1;
                    let mut nesting = 0;
                    for (j, c) in chars.iter().enumerate().skip(start) {
                        if *c == '(' {
                            nesting += 1;
                        } else if *c == ')' {
                            if nesting == 0 {
                                i = j;
                                break;
                            }
                            nesting -= 1;
                        }
                    }
                    let contents: String = chars[start..i.max(start)].iter().collect();
                    let mut children = Vec::new();
                    if matches!(name.as_str(), ":nth-child" | ":nth-last-child")
                        && let Some(index) = contents.find("of ")
                    {
                        children.push(Selector::Value {
                            value: contents[..index + 3].into(),
                        });
                        children.extend(parse(&contents[index + 3..]));
                    } else {
                        children.push(Selector::Value { value: contents });
                    }
                    for child in children {
                        let index = insert(&mut arena, child);
                        let group = arena.nodes[node].children;
                        arena.groups[group].push(index);
                    }
                    arena.append(target, node);
                } else {
                    arena.append(target, node);
                    contexts.push((target, current_list, contains_combinator));
                    target = arena.nodes[node].children;
                    current_list = None;
                    contains_combinator = false;
                }
            }
            ')' => {
                arena.flush(target, &mut buffer);
                arena.finish(target, current_list, contains_combinator);
                (target, current_list, contains_combinator) =
                    contexts.pop().unwrap_or((0, None, false));
            }
            '.' | ':' | '#' => {
                if chars[i] == ':' && buffer == ":" {
                    buffer.push(':');
                } else {
                    arena.flush(target, &mut buffer);
                    buffer.push(chars[i]);
                }
            }
            '[' => {
                arena.flush(target, &mut buffer);
                let start = i;
                let mut nesting = 0;
                for (j, c) in chars.iter().enumerate().skip(i + 1) {
                    if *c == '[' {
                        nesting += 1;
                    } else if *c == ']' {
                        if nesting == 0 {
                            i = j;
                            break;
                        }
                        nesting -= 1;
                    }
                }
                let node = arena.node(
                    Kind::Selector,
                    chars[start..=i].iter().collect(),
                    Vec::new(),
                );
                arena.append(target, node);
            }
            '\'' | '"' => {
                let start = i;
                let quote = chars[i];
                let mut j = i + 1;
                while j < chars.len() {
                    if chars[j] == '\\' {
                        j += 2;
                        continue;
                    }
                    if chars[j] == quote {
                        i = j;
                        break;
                    }
                    j += 1;
                }
                buffer.extend(chars[start..=i].iter());
            }
            '&' | '*' => {
                arena.flush(target, &mut buffer);
                let node = arena.node(Kind::Selector, chars[i].to_string(), Vec::new());
                arena.append(target, node);
            }
            '\\' => {
                buffer.push('\\');
                if let Some(c) = chars.get(i + 1) {
                    buffer.push(*c);
                } else {
                    buffer.push_str("undefined");
                }
                i += 1;
            }
            c => buffer.push(c),
        }
        i += 1;
    }
    arena.flush(target, &mut buffer);
    arena.finish(target, current_list, contains_combinator);
    let roots = std::mem::take(&mut arena.groups[0]);
    roots.into_iter().map(|i| arena.take(i)).collect()
}

fn insert(arena: &mut Arena, node: Selector) -> usize {
    let (kind, value, nodes) = match node {
        Selector::List { nodes } => (Kind::List, String::new(), nodes),
        Selector::Complex { nodes } => (Kind::Complex, String::new(), nodes),
        Selector::Compound { nodes } => (Kind::Compound, String::new(), nodes),
        Selector::Function { value, nodes } => (Kind::Function, value, nodes),
        Selector::Combinator { value } => (Kind::Combinator, value, Vec::new()),
        Selector::Selector { value } => (Kind::Selector, value, Vec::new()),
        Selector::Value { value } => (Kind::Value, value, Vec::new()),
    };
    let children = nodes.into_iter().map(|node| insert(arena, node)).collect();
    arena.node(kind, value, children)
}
