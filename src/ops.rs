/// Tree operations: insert, delete, move, yank, paste, toggle, sort, rank.
///
/// Each mutating operation should call `push_change` first so it can be undone.
use crate::model::{MindMap, Node, NodeId};
use crate::parser;

// ── Undo / Redo ──────────────────────────────────────────────

/// Snapshot-based undo history (same strategy as the PHP version).
#[derive(Debug, Clone)]
pub struct UndoHistory {
    snapshots: Vec<Snapshot>,
    index: usize,
    max_steps: usize,
}

#[derive(Debug, Clone)]
struct Snapshot {
    nodes: std::collections::BTreeMap<NodeId, Node>,
    active_node: NodeId,
}

impl UndoHistory {
    pub fn new(max_steps: usize) -> Self {
        Self {
            snapshots: Vec::new(),
            index: 0,
            max_steps,
        }
    }

    /// Save current state before a mutation.
    pub fn push(&mut self, mm: &MindMap) {
        // discard any redo chain
        self.snapshots.truncate(self.index);

        // cap history size
        if self.snapshots.len() >= self.max_steps {
            self.snapshots.remove(0);
            self.index = self.index.saturating_sub(1);
        }

        self.snapshots.push(Snapshot {
            nodes: mm.nodes.clone(),
            active_node: mm.active_node,
        });
        self.index += 1;
    }

    /// Undo: restore the previous snapshot. Returns true if successful.
    pub fn undo(&mut self, mm: &mut MindMap) -> bool {
        if self.index == 0 {
            return false;
        }
        self.index -= 1;
        let snap = &self.snapshots[self.index];
        mm.nodes = snap.nodes.clone();
        mm.active_node = snap.active_node;
        true
    }

    /// Redo: move forward in the snapshot chain. Returns true if successful.
    pub fn redo(&mut self, mm: &mut MindMap) -> bool {
        if self.index >= self.snapshots.len() {
            return false;
        }
        let snap = &self.snapshots[self.index];
        mm.nodes = snap.nodes.clone();
        mm.active_node = snap.active_node;
        self.index += 1;
        true
    }
}

// ── Insert ───────────────────────────────────────────────────

pub enum InsertKind {
    Sibling,
    Child,
}

/// Insert a new node. Returns the new node's id.
pub fn insert_node(mm: &mut MindMap, kind: InsertKind, title: &str) -> NodeId {
    let mut kind = kind;
    // root can only get children, not siblings
    if mm.active_node == mm.root_id {
        kind = InsertKind::Child;
    }

    let parent_id = match kind {
        InsertKind::Sibling => mm.node(mm.active_node).parent,
        InsertKind::Child => mm.active_node,
    };

    let new_id = mm.alloc_id();
    let node = Node::new(title, parent_id);
    mm.nodes.insert(new_id, node);

    // update parent
    mm.node_mut(parent_id).is_leaf = false;
    mm.node_mut(parent_id).collapsed = false;

    match kind {
        InsertKind::Sibling => {
            // insert right after active node in parent's children
            let children = &mm.node(parent_id).children;
            let mut new_children = Vec::with_capacity(children.len() + 1);
            for &cid in children {
                new_children.push(cid);
                if cid == mm.active_node {
                    new_children.push(new_id);
                }
            }
            mm.node_mut(parent_id).children = new_children;
        }
        InsertKind::Child => {
            mm.node_mut(parent_id).children.push(new_id);
        }
    }

    mm.active_node = new_id;
    new_id
}

// ── Delete ───────────────────────────────────────────────────

/// Collect all descendant ids (including `id` itself).
pub fn subtree_ids(mm: &MindMap, id: NodeId) -> Vec<NodeId> {
    let mut result = vec![id];
    for &cid in &mm.node(id).children {
        result.extend(subtree_ids(mm, cid));
    }
    result
}

/// Delete a node and its subtree. Moves active node to the next sibling or parent.
/// Returns the serialized subtree text (for clipboard).
pub fn delete_node(mm: &mut MindMap, id: NodeId) -> Option<String> {
    if id == mm.root_id || id == NodeId(0) {
        return None;
    }

    let text = parser::serialize(mm, id);

    let parent_id = mm.node(id).parent;

    // find next active node: next sibling, prev sibling, or parent
    let siblings = mm.node(parent_id).children.clone();
    let pos = siblings.iter().position(|&c| c == id);
    let next_active = if let Some(p) = pos {
        if p + 1 < siblings.len() {
            siblings[p + 1]
        } else if p > 0 {
            siblings[p - 1]
        } else {
            parent_id
        }
    } else {
        parent_id
    };

    // remove from parent's children
    mm.node_mut(parent_id).children.retain(|&cid| cid != id);

    if mm.node(parent_id).children.is_empty() {
        mm.node_mut(parent_id).is_leaf = true;
    }

    // remove all descendant nodes
    for nid in subtree_ids(mm, id) {
        mm.nodes.remove(&nid);
    }

    mm.active_node = next_active;
    Some(text)
}

/// Delete only the children of a node.
pub fn delete_children(mm: &mut MindMap, id: NodeId) -> Option<String> {
    let children = mm.node(id).children.clone();
    if children.is_empty() {
        return None;
    }

    let mut text = String::new();
    for &cid in &children {
        text.push_str(&parser::serialize(mm, cid));
    }

    for &cid in &children {
        for nid in subtree_ids(mm, cid) {
            mm.nodes.remove(&nid);
        }
    }

    mm.node_mut(id).children.clear();
    mm.node_mut(id).is_leaf = true;

    Some(text)
}

// ── Move ─────────────────────────────────────────────────────

/// Move active node down among its siblings.
pub fn move_node_down(mm: &mut MindMap, show_hidden: bool) -> bool {
    let id = mm.active_node;
    if id == NodeId(0) {
        return false;
    }
    let parent_id = mm.node(id).parent;
    let children = mm.node(parent_id).children.clone();
    let pos = match children.iter().position(|&c| c == id) {
        Some(p) => p,
        None => return false,
    };

    // find next visible sibling
    let next = children[pos + 1..]
        .iter()
        .find(|&&cid| show_hidden || !mm.node(cid).hidden);

    if let Some(&swap_id) = next {
        let swap_pos = children.iter().position(|&c| c == swap_id).unwrap();
        let mut new_children = children;
        new_children.swap(pos, swap_pos);
        mm.node_mut(parent_id).children = new_children;
        true
    } else {
        false
    }
}

/// Move active node up among its siblings.
pub fn move_node_up(mm: &mut MindMap, show_hidden: bool) -> bool {
    let id = mm.active_node;
    if id == NodeId(0) {
        return false;
    }
    let parent_id = mm.node(id).parent;
    let children = mm.node(parent_id).children.clone();
    let pos = match children.iter().position(|&c| c == id) {
        Some(p) => p,
        None => return false,
    };

    // find previous visible sibling
    let prev = children[..pos]
        .iter()
        .rev()
        .find(|&&cid| show_hidden || !mm.node(cid).hidden);

    if let Some(&swap_id) = prev {
        let swap_pos = children.iter().position(|&c| c == swap_id).unwrap();
        let mut new_children = children;
        new_children.swap(pos, swap_pos);
        mm.node_mut(parent_id).children = new_children;
        true
    } else {
        false
    }
}

// ── Yank (copy) ──────────────────────────────────────────────

/// Serialize a node and its subtree for the clipboard.
pub fn yank_node(mm: &MindMap, id: NodeId) -> String {
    parser::serialize(mm, id)
}

/// Serialize only the children of a node.
pub fn yank_children(mm: &MindMap, id: NodeId) -> String {
    let mut buf = String::new();
    for &cid in &mm.node(id).children {
        buf.push_str(&parser::serialize(mm, cid));
    }
    buf
}

// ── Paste ────────────────────────────────────────────────────

/// Paste clipboard text as children of the active node.
pub fn paste_as_children(mm: &mut MindMap, text: &str) {
    paste_subtree(mm, text, false);
}

/// Paste clipboard text as siblings after the active node.
pub fn paste_as_siblings(mm: &mut MindMap, text: &str) {
    if mm.active_node == mm.root_id {
        return;
    }
    paste_subtree(mm, text, true);
}

fn paste_subtree(mm: &mut MindMap, text: &str, as_sibling: bool) {
    let parent_id = if as_sibling {
        mm.node(mm.active_node).parent
    } else {
        mm.active_node
    };

    mm.node_mut(parent_id).collapsed = false;
    mm.node_mut(parent_id).is_leaf = false;

    let start_id = mm.alloc_id().0;

    // reuse the parser to build a subtree
    let temp = parser::parse(text);
    // the parser gives us nodes keyed from 2 upward;
    // we need to shift them into our id-space.

    let temp_root = temp.root_id;
    let temp_children = temp.node(temp_root).children.clone();

    // collect all nodes except super-root(0) and possibly synthetic root(1)
    let mut to_import: Vec<(NodeId, Node)> = Vec::new();

    fn collect(mm: &MindMap, id: NodeId, out: &mut Vec<(NodeId, Node)>) {
        out.push((id, mm.node(id).clone()));
        for &cid in &mm.node(id).children {
            collect(mm, cid, out);
        }
    }

    // if the parsed result has a synthetic root, import its children as top-level
    let import_roots = if temp.root_id == NodeId(1) && temp.node(NodeId(1)).title == "root" {
        // synthetic root — import each child tree
        let mut roots = Vec::new();
        for &cid in &temp_children {
            collect(&temp, cid, &mut to_import);
            roots.push(cid);
        }
        roots
    } else {
        // single root — import the whole subtree
        collect(&temp, temp_root, &mut to_import);
        vec![temp_root]
    };

    if to_import.is_empty() {
        return;
    }

    // compute id offset
    let min_import_id = to_import.iter().map(|(id, _)| id.0).min().unwrap();
    let offset = start_id.saturating_sub(min_import_id);

    // remap ids
    let mut first_new_id = None;
    let mut sub_roots = Vec::new();

    for (old_id, mut node) in to_import {
        let new_id = NodeId(old_id.0 + offset);
        if first_new_id.is_none() {
            first_new_id = Some(new_id);
        }

        // remap parent
        if import_roots.contains(&old_id) {
            node.parent = parent_id;
            sub_roots.push(new_id);
        } else {
            node.parent = NodeId(node.parent.0 + offset);
        }

        // remap children
        node.children = node.children.iter().map(|c| NodeId(c.0 + offset)).collect();

        mm.nodes.insert(new_id, node);
    }

    // update next_id
    let max_inserted = mm.nodes.keys().last().map(|k| k.0 + 1).unwrap_or(start_id);
    while mm.alloc_id().0 < max_inserted {}

    // wire into parent
    if as_sibling {
        let children = mm.node(parent_id).children.clone();
        let mut new_children = Vec::with_capacity(children.len() + sub_roots.len());
        for &cid in &children {
            if !sub_roots.contains(&cid) {
                new_children.push(cid);
            }
            if cid == mm.active_node {
                new_children.extend(&sub_roots);
            }
        }
        mm.node_mut(parent_id).children = new_children;
    } else {
        for &sr in &sub_roots {
            mm.node_mut(parent_id).children.push(sr);
        }
    }

    if let Some(fid) = first_new_id {
        mm.active_node = fid;
    }
}

// ── Toggle ───────────────────────────────────────────────────

/// Toggle collapsed state of active node.
pub fn toggle_node(mm: &mut MindMap) {
    let id = mm.active_node;
    if mm.node(id).is_leaf {
        return;
    }
    let cur = mm.node(id).collapsed;
    mm.node_mut(id).collapsed = !cur;
}

/// Collapse everything except first-level nodes.
pub fn collapse_all(mm: &mut MindMap) {
    let ids: Vec<NodeId> = mm.nodes.keys().copied().collect();
    for id in ids {
        if !mm.node(id).is_leaf && id != NodeId(0) && id != mm.root_id {
            mm.node_mut(id).collapsed = true;
        }
    }
    mm.active_node = mm.root_id;
}

/// Expand all nodes.
pub fn expand_all(mm: &mut MindMap) {
    let ids: Vec<NodeId> = mm.nodes.keys().copied().collect();
    for id in ids {
        mm.node_mut(id).collapsed = false;
    }
}

/// Collapse to a specific depth level.
pub fn collapse_level(mm: &mut MindMap, level: usize) {
    collapse_rec(mm, mm.root_id, level);

    // if active node is now inside a collapsed subtree, move it up
    let mut current = mm.active_node;
    while current != mm.root_id {
        if mm.node(current).collapsed {
            mm.active_node = current;
        }
        current = mm.node(current).parent;
    }
}

/// Focus the tree around the active node.
///
/// Collapse sibling branches along the active path, then fully expand the
/// active node's subtree.
pub fn focus(mm: &mut MindMap) {
    collapse_siblings(mm, mm.active_node);
    expand_subtree(mm, mm.active_node);
}

fn collapse_siblings(mm: &mut MindMap, id: NodeId) {
    if id <= mm.root_id {
        return;
    }

    let parent_id = mm.node(id).parent;
    let siblings = mm.node(parent_id).children.clone();
    for cid in siblings {
        if cid != id {
            mm.node_mut(cid).collapsed = true;
        }
    }

    collapse_siblings(mm, parent_id);
}

fn expand_subtree(mm: &mut MindMap, id: NodeId) {
    if mm.node(id).is_leaf {
        return;
    }

    mm.node_mut(id).collapsed = false;
    let children = mm.node(id).children.clone();
    for cid in children {
        expand_subtree(mm, cid);
    }
}

fn collapse_rec(mm: &mut MindMap, id: NodeId, keep: usize) {
    if mm.node(id).is_leaf {
        return;
    }
    if keep == 0 {
        mm.node_mut(id).collapsed = true;
    } else {
        mm.node_mut(id).collapsed = false;
        let children: Vec<NodeId> = mm.node(id).children.clone();
        for cid in children {
            collapse_rec(mm, cid, keep - 1);
        }
    }
}

// ── Sort ─────────────────────────────────────────────────────

/// Sort siblings of the active node alphabetically.
pub fn sort_siblings(mm: &mut MindMap) {
    let parent_id = mm.node(mm.active_node).parent;
    let mut children = mm.node(parent_id).children.clone();
    children.sort_by(|&a, &b| mm.node(a).title.cmp(&mm.node(b).title));
    mm.node_mut(parent_id).children = children;
}

// ── Toggle symbols (✓/✗) ────────────────────────────────────

pub fn toggle_symbol(mm: &mut MindMap, sym1: &str, sym2: &str) {
    let id = mm.active_node;
    let title = mm.node(id).title.clone();

    let s1 = format!("{} ", sym1);
    let s2 = format!("{} ", sym2);

    let new_title = if title.starts_with(&s1) {
        format!("{}{}", s2, &title[s1.len()..])
    } else if title.starts_with(&s2) {
        title[s2.len()..].to_string()
    } else {
        format!("{}{}", s1, title)
    };

    mm.node_mut(id).title = new_title;
}

// ── Search ───────────────────────────────────────────────────

/// Find all node ids whose title contains `query` (case-insensitive).
pub fn search(mm: &MindMap, query: &str) -> Vec<NodeId> {
    let q = query.to_lowercase();
    mm.nodes
        .iter()
        .filter(|(&id, _)| id != NodeId(0))
        .filter(|(_, n)| n.title.to_lowercase().contains(&q))
        .map(|(&id, _)| id)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_map() -> MindMap {
        parser::parse("root\n\tA\n\tB\n\tC\n")
    }

    #[test]
    fn insert_child() {
        let mut mm = sample_map();
        mm.active_node = mm.root_id;
        let new_id = insert_node(&mut mm, InsertKind::Child, "D");
        assert_eq!(mm.node(new_id).title, "D");
        assert!(mm.node(mm.root_id).children.contains(&new_id));
    }

    #[test]
    fn insert_sibling() {
        let mut mm = sample_map();
        let first_child = mm.node(mm.root_id).children[0];
        mm.active_node = first_child;
        let new_id = insert_node(&mut mm, InsertKind::Sibling, "A2");
        let children = &mm.node(mm.root_id).children;
        let pos_a = children.iter().position(|&c| c == first_child).unwrap();
        let pos_a2 = children.iter().position(|&c| c == new_id).unwrap();
        assert_eq!(pos_a2, pos_a + 1);
    }

    #[test]
    fn delete_and_yank() {
        let mut mm = sample_map();
        let first_child = mm.node(mm.root_id).children[0];
        mm.active_node = first_child;
        let text = delete_node(&mut mm, first_child);
        assert!(text.is_some());
        assert!(!mm.nodes.contains_key(&first_child));
        assert_eq!(mm.node(mm.root_id).children.len(), 2);
    }

    #[test]
    fn move_down_up() {
        let mut mm = sample_map();
        let children = mm.node(mm.root_id).children.clone();
        mm.active_node = children[0]; // A
        assert!(move_node_down(&mut mm, true));
        assert_eq!(mm.node(mm.root_id).children[0], children[1]); // B is now first
        assert!(move_node_up(&mut mm, true));
        assert_eq!(mm.node(mm.root_id).children[0], children[0]); // A is back
    }

    #[test]
    fn collapse_expand() {
        let mut mm = parser::parse("root\n\tA\n\t\tA1\n\tB\n");
        expand_all(&mut mm);
        for (_, n) in &mm.nodes {
            assert!(!n.collapsed);
        }
        collapse_all(&mut mm);
        // root should not be collapsed, but its children should be
        assert!(!mm.node(mm.root_id).collapsed);
    }

    #[test]
    fn focus_collapses_other_branches_and_expands_active_subtree() {
        let mut mm = parser::parse("root\n\tA\n\t\tA1\n\tB\n\t\tB1\n");
        let a = mm.node(mm.root_id).children[0];
        let b = mm.node(mm.root_id).children[1];
        let b1 = mm.node(b).children[0];

        mm.node_mut(a).collapsed = false;
        mm.node_mut(b).collapsed = true;
        mm.active_node = b;

        focus(&mut mm);

        assert!(mm.node(a).collapsed);
        assert!(!mm.node(b).collapsed);
        assert!(!mm.node(b1).collapsed);
    }

    #[test]
    fn search_finds_nodes() {
        let mm = sample_map();
        let results = search(&mm, "b");
        assert_eq!(results.len(), 1);
    }

    #[test]
    fn toggle_symbol_cycle() {
        let mut mm = sample_map();
        mm.active_node = mm.node(mm.root_id).children[0];
        toggle_symbol(&mut mm, "✓", "✗");
        assert!(mm.node(mm.active_node).title.starts_with("✓ "));
        toggle_symbol(&mut mm, "✓", "✗");
        assert!(mm.node(mm.active_node).title.starts_with("✗ "));
        toggle_symbol(&mut mm, "✓", "✗");
        assert_eq!(mm.node(mm.active_node).title, "A");
    }

    #[test]
    fn undo_restores() {
        let mut mm = sample_map();
        let mut history = UndoHistory::new(24);

        // save state before mutation
        history.push(&mm);
        let first_child = mm.node(mm.root_id).children[0];
        mm.active_node = first_child;
        delete_node(&mut mm, first_child);
        assert_eq!(mm.node(mm.root_id).children.len(), 2);

        // undo should restore the 3-children state
        assert!(history.undo(&mut mm));
        assert_eq!(mm.node(mm.root_id).children.len(), 3);
    }
}
