/// Core data model for the mind map.
///
/// The PHP version stores everything in `$mm['nodes'][$id]` —
/// an associative array keyed by integer id.
/// We mirror that with `MindMap` holding a `BTreeMap<NodeId, Node>`.
use std::collections::BTreeMap;

// ── Node id ──────────────────────────────────────────────────

/// Lightweight, copyable node identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NodeId(pub usize);

// ── Node ─────────────────────────────────────────────────────

/// A single node in the mind-map tree.
#[derive(Debug, Clone)]
pub struct Node {
    pub title: String,
    pub meta: NodeMeta,
    pub parent: NodeId,
    pub children: Vec<NodeId>,
    pub is_leaf: bool,
    pub collapsed: bool,
    /// Nodes whose title starts with "[HIDDEN] " are logically hidden.
    pub hidden: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NodeMeta {
    pub linear_identifier: Option<String>,
    pub exported_git_head: Option<String>,
}

impl NodeMeta {
    pub fn is_empty(&self) -> bool {
        self.linear_identifier.is_none() && self.exported_git_head.is_none()
    }
}

impl Node {
    pub fn new(title: impl Into<String>, parent: NodeId) -> Self {
        Self {
            title: title.into(),
            meta: NodeMeta::default(),
            parent,
            children: Vec::new(),
            is_leaf: true,
            collapsed: false,
            hidden: false,
        }
    }
}

// ── MindMap ──────────────────────────────────────────────────

/// The top-level mind-map state.
///
/// `root_id` is the user-visible root (usually `NodeId(1)` or `NodeId(2)`).
/// `NodeId(0)` is a synthetic super-root that is never rendered, matching
/// the PHP version's `$mm['nodes'][0]`.
#[derive(Debug, Clone)]
pub struct MindMap {
    pub nodes: BTreeMap<NodeId, Node>,
    pub root_id: NodeId,
    pub active_node: NodeId,
    next_id: usize,
}

impl MindMap {
    /// Create a brand-new, empty mind map with a single root node.
    pub fn new(root_title: impl Into<String>) -> Self {
        let mut nodes = BTreeMap::new();

        // super-root (invisible)
        let super_root = Node {
            title: "X".into(),
            meta: NodeMeta::default(),
            parent: NodeId(usize::MAX), // sentinel
            children: vec![NodeId(1)],
            is_leaf: false,
            collapsed: false,
            hidden: false,
        };
        nodes.insert(NodeId(0), super_root);

        // user root
        let root = Node {
            title: root_title.into(),
            meta: NodeMeta::default(),
            parent: NodeId(0),
            children: Vec::new(),
            is_leaf: true,
            collapsed: false,
            hidden: false,
        };
        nodes.insert(NodeId(1), root);

        Self {
            nodes,
            root_id: NodeId(1),
            active_node: NodeId(1),
            next_id: 2,
        }
    }

    /// Allocate the next available `NodeId`.
    pub fn alloc_id(&mut self) -> NodeId {
        let id = NodeId(self.next_id);
        self.next_id += 1;
        id
    }

    /// Convenience: get a ref to a node (panics on missing id — bug).
    pub fn node(&self, id: NodeId) -> &Node {
        &self.nodes[&id]
    }

    /// Convenience: get a mut ref to a node.
    pub fn node_mut(&mut self, id: NodeId) -> &mut Node {
        self.nodes.get_mut(&id).expect("node not found")
    }

    /// Insert multiple nodes parsed from external data.
    /// Adjusts `next_id` to stay above the highest inserted key.
    pub fn bulk_insert(&mut self, new_nodes: BTreeMap<NodeId, Node>) {
        if let Some((&max_id, _)) = new_nodes.last_key_value() {
            if max_id.0 >= self.next_id {
                self.next_id = max_id.0 + 1;
            }
        }
        self.nodes.extend(new_nodes);
    }

    /// List of visible children for a node
    /// (excludes hidden nodes when `show_hidden` is false).
    pub fn visible_children(&self, id: NodeId, show_hidden: bool) -> Vec<NodeId> {
        let node = self.node(id);
        if show_hidden {
            node.children.clone()
        } else {
            node.children
                .iter()
                .filter(|&&cid| !self.node(cid).hidden)
                .copied()
                .collect()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_map_has_root() {
        let mm = MindMap::new("root");
        assert_eq!(mm.root_id, NodeId(1));
        assert_eq!(mm.node(NodeId(1)).title, "root");
        assert_eq!(mm.node(NodeId(0)).children, vec![NodeId(1)]);
    }

    #[test]
    fn alloc_id_increments() {
        let mut mm = MindMap::new("r");
        let a = mm.alloc_id();
        let b = mm.alloc_id();
        assert_eq!(a, NodeId(2));
        assert_eq!(b, NodeId(3));
    }
}
