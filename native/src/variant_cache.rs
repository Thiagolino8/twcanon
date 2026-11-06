// Interned variant objects preserve the child references copied by replaceObject.
use crate::candidate::{self, Registry, Variant};
use indexmap::IndexMap;

pub(crate) type Identity = usize;

struct Node {
    value: Variant,
    child: Option<Identity>,
}

#[derive(Default)]
pub(crate) struct VariantCache {
    pub(crate) parsed: IndexMap<String, Option<Identity>>,
    nodes: Vec<Node>,
}

impl VariantCache {
    pub(crate) fn parse(&mut self, input: &str, registry: &Registry) -> Option<Variant> {
        let identity = self.parse_identity(input, registry)?;
        Some(self.snapshot(identity))
    }

    pub(crate) fn parse_identity(&mut self, input: &str, registry: &Registry) -> Option<Identity> {
        if let Some(value) = self.parsed.get(input) {
            return *value;
        }
        let mut child = None;
        let value = candidate::parse_variant_with(input, registry, &mut |raw| {
            child = self.parse_identity(raw, registry);
            child.map(|identity| self.snapshot(identity))
        });
        let identity = value.map(|value| {
            let identity = self.nodes.len();
            self.nodes.push(Node { value, child });
            identity
        });
        // DefaultMap inserts the parent only after evaluating its children.
        self.parsed.insert(input.into(), identity);
        identity
    }

    pub(crate) fn snapshot(&self, identity: Identity) -> Variant {
        let node = &self.nodes[identity];
        let mut value = node.value.clone();
        if let Variant::Compound { variant, .. } = &mut value {
            **variant = self.snapshot(node.child.expect("compound has an interned child"));
        }
        value
    }

    pub(crate) fn value(&self, identity: Identity) -> &Variant {
        &self.nodes[identity].value
    }

    pub(crate) fn child(&self, identity: Identity) -> Option<Identity> {
        self.nodes[identity].child
    }

    pub(crate) fn replace(&mut self, identity: Identity, value: &Variant, child: Option<Identity>) {
        self.nodes[identity] = Node {
            value: value.clone(),
            child,
        };
    }
}
