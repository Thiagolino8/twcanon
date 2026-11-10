// Stable node and child-list identities for the mutating ast.ts passes.
use crate::{ast::AstNode, id_hasher::IdHasher};
use std::{collections::HashMap, hash::BuildHasherDefault};

pub(crate) type NodeId = usize;
pub(crate) type GroupId = usize;

pub(crate) struct Node {
    pub data: AstNode,
    pub children: Option<GroupId>,
}

#[derive(Default)]
pub(crate) struct Arena {
    pub nodes: Vec<Node>,
    pub groups: Vec<Vec<NodeId>>,
    declarations: HashMap<usize, NodeId, BuildHasherDefault<IdHasher>>,
}

impl Arena {
    pub fn group(&mut self, nodes: Vec<NodeId>) -> GroupId {
        let id = self.groups.len();
        self.groups.push(nodes);
        id
    }

    pub fn node(&mut self, data: AstNode, children: Option<GroupId>) -> NodeId {
        let id = self.nodes.len();
        self.nodes.push(Node { data, children });
        id
    }

    pub fn import_node(&mut self, mut data: AstNode) -> NodeId {
        let identity = match &data {
            AstNode::Declaration { identity, .. } => Some(identity.0),
            _ => None,
        };
        if let Some(id) = identity.and_then(|identity| self.declarations.get(&identity)) {
            return *id;
        }
        let children = data
            .children_mut()
            .map(std::mem::take)
            .map(|nodes| self.import(nodes));
        let id = self.node(data, children);
        if let Some(identity) = identity {
            self.declarations.insert(identity, id);
        }
        id
    }

    pub fn import(&mut self, nodes: Vec<AstNode>) -> GroupId {
        if self.nodes.capacity() == 0 && self.groups.capacity() == 0 {
            fn count(nodes: &[AstNode], counts: &mut [usize; 3]) {
                for node in nodes {
                    if counts[0] == 64 {
                        break;
                    }
                    counts[0] += 1;
                    match node {
                        AstNode::Declaration { .. } => counts[2] += 1,
                        AstNode::Rule { nodes, .. }
                        | AstNode::AtRule { nodes, .. }
                        | AstNode::Context { nodes, .. }
                        | AstNode::AtRoot { nodes } => {
                            counts[1] += 1;
                            count(nodes, counts);
                        }
                        _ => {}
                    }
                }
            }
            // Bound the initial reservation, including trees with shared declarations.
            let mut counts = [0, 1, 0];
            count(&nodes, &mut counts);
            self.nodes.reserve(counts[0]);
            self.groups.reserve(counts[1]);
            self.declarations.reserve(counts[2]);
        }
        let nodes = nodes
            .into_iter()
            .map(|node| self.import_node(node))
            .collect();
        self.group(nodes)
    }

    pub fn export(&self, group: GroupId) -> Vec<AstNode> {
        self.groups[group]
            .iter()
            .map(|&id| {
                let mut data = self.nodes[id].data.clone();
                if let Some(children) = self.nodes[id].children {
                    *data.children_mut().unwrap() = self.export(children);
                }
                data
            })
            .collect()
    }

    /// Move the last reachable occurrence of each node into the output. Earlier
    /// occurrences retain the same declaration identity, as with `export`.
    pub fn into_export(mut self, group: GroupId) -> Vec<AstNode> {
        fn count(arena: &Arena, group: GroupId, references: &mut [usize]) {
            for &id in &arena.groups[group] {
                references[id] += 1;
                if let Some(children) = arena.nodes[id].children {
                    count(arena, children, references);
                }
            }
        }
        fn export(arena: &mut Arena, group: GroupId, references: &mut [usize]) -> Vec<AstNode> {
            let mut result = Vec::with_capacity(arena.groups[group].len());
            for index in 0..arena.groups[group].len() {
                let id = arena.groups[group][index];
                let mut data = if references[id] == 1 {
                    std::mem::replace(
                        &mut arena.nodes[id].data,
                        AstNode::AtRoot { nodes: Vec::new() },
                    )
                } else {
                    arena.nodes[id].data.clone()
                };
                references[id] -= 1;
                if let Some(children) = arena.nodes[id].children {
                    *data.children_mut().unwrap() = export(arena, children, references);
                }
                result.push(data);
            }
            result
        }
        let mut references = vec![0; self.nodes.len()];
        count(&self, group, &mut references);
        export(&mut self, group, &mut references)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::rule;

    #[test]
    fn consuming_export_preserves_shared_nodes_and_declaration_identity() {
        let declaration = AstNode::decl("color", "red");
        let mut arena = Arena::default();
        let root = arena.import(vec![
            rule(".a", vec![declaration.clone(), declaration.clone()]),
            rule(".b", vec![declaration]),
        ]);
        // Share an entire rule too, as well as its declaration and child list.
        let first = arena.groups[root][0];
        arena.groups[root].push(first);
        let expected = arena.export(root);
        let actual = arena.into_export(root);
        assert_eq!(actual, expected);
        fn identities(nodes: &[AstNode], output: &mut Vec<usize>) {
            for node in nodes {
                if let AstNode::Declaration { identity, .. } = node {
                    output.push(identity.0);
                }
                match node {
                    AstNode::Rule { nodes, .. }
                    | AstNode::AtRule { nodes, .. }
                    | AstNode::Context { nodes, .. }
                    | AstNode::AtRoot { nodes } => identities(nodes, output),
                    _ => {}
                }
            }
        }
        let mut before = Vec::new();
        identities(&expected, &mut before);
        let mut after = Vec::new();
        identities(&actual, &mut after);
        assert_eq!(after, before);
        assert!(after.iter().all(|identity| *identity == after[0]));
    }
}
