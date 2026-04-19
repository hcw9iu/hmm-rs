/// .hmm file format parser and serializer.
///
/// The format is dead-simple: plain text with tab indentation representing
/// the tree hierarchy.  This module mirrors the PHP `list_to_map` and
/// `map_to_list` functions.
use std::collections::BTreeMap;

use crate::model::{MindMap, Node, NodeId, NodeMeta};

// ── Constants ────────────────────────────────────────────────

const HIDDEN_PREFIX: &str = "[HIDDEN] ";

// ── Parse ────────────────────────────────────────────────────

/// Parse the content of a `.hmm` file into a `MindMap`.
pub fn parse(text: &str) -> MindMap {
    let lines = sanitize_lines(text);

    if lines.is_empty() {
        return MindMap::new("root");
    }

    let (new_nodes, first_level) = build_tree(&lines);

    if new_nodes.is_empty() {
        return MindMap::new("root");
    }

    let mut mm = MindMap::new("root");
    // clear the default root we just created
    mm.nodes.clear();

    // super-root (id 0) — never rendered
    mm.nodes.insert(
        NodeId(0),
        Node {
            title: "X".into(),
            meta: NodeMeta::default(),
            parent: NodeId(usize::MAX),
            children: Vec::new(),
            is_leaf: false,
            collapsed: false,
            hidden: false,
        },
    );

    if first_level.len() > 1 {
        // multiple top-level nodes → inject a synthetic "root" node at id 1
        mm.root_id = NodeId(1);

        let root = Node {
            title: "root".into(),
            meta: NodeMeta::default(),
            parent: NodeId(0),
            children: first_level.clone(),
            is_leaf: false,
            collapsed: false,
            hidden: false,
        };
        mm.nodes.insert(NodeId(1), root);
        mm.node_mut(NodeId(0)).children = vec![NodeId(1)];

        // reparent first-level nodes
        let mut patched = new_nodes;
        for &fid in &first_level {
            if let Some(n) = patched.get_mut(&fid) {
                n.parent = NodeId(1);
            }
        }
        mm.bulk_insert(patched);
        mm.active_node = NodeId(1);
    } else {
        mm.node_mut(NodeId(0)).children = first_level.clone();
        mm.bulk_insert(new_nodes);
        mm.root_id = first_level[0];
        mm.active_node = first_level[0];
    }

    mm
}

/// Clean up raw lines: expand tabs, strip BOM, normalize bullets, compute
/// minimum indentation shift.  Returns `(title, indentation)` pairs for
/// non-empty lines only.
fn sanitize_lines(text: &str) -> Vec<(String, usize)> {
    let mut raw: Vec<String> = text
        .lines()
        .map(|l| {
            let s = l.replace('\t', "  ");
            // strip BOM
            let s = s.strip_prefix('\u{FEFF}').unwrap_or(&s).to_string();
            // strip control chars except space/newline
            let s: String = s.chars().filter(|&c| c == ' ' || !c.is_control()).collect();
            s
        })
        .collect();

    // normalise bullets:  "  * foo" → "    foo",  "  - foo" → "    foo"
    for line in &mut raw {
        let trimmed = line.trim_start();
        if trimmed.starts_with("* ") || trimmed.starts_with("- ") {
            let indent = line.len() - trimmed.len();
            *line = format!("{}{}", " ".repeat(indent + 2), &trimmed[2..]);
        }
    }

    // compute minimum indentation across non-empty lines
    let min_indent = raw
        .iter()
        .filter(|l| !l.trim().is_empty())
        .map(|l| l.len() - l.trim_start().len())
        .min()
        .unwrap_or(0);

    raw.iter()
        .filter(|l| !l.trim().is_empty())
        .map(|l| {
            let indent = (l.len() - l.trim_start().len()).saturating_sub(min_indent);
            (l.trim().to_string(), indent)
        })
        .collect()
}

/// Build the node tree from sanitized `(title, indentation)` pairs.
/// Returns `(nodes_map, first_level_ids)`.
fn build_tree(lines: &[(String, usize)]) -> (BTreeMap<NodeId, Node>, Vec<NodeId>) {
    let start_id: usize = 2; // leave room for super-root(0) and possible synthetic root(1)
    let root_parent = NodeId(0);

    let mut nodes: BTreeMap<NodeId, Node> = BTreeMap::new();
    let mut id = start_id;

    let mut prev_level: usize = 1;
    let mut prev_indent: usize = 0;

    // level → parent node id
    let mut level_parent: BTreeMap<usize, NodeId> = BTreeMap::new();
    // level → indentation value
    let mut level_indent: BTreeMap<usize, usize> = BTreeMap::new();

    level_parent.insert(1, root_parent);
    level_indent.insert(1, 0);

    for (title, indent) in lines {
        let indent = *indent;

        // determine current level
        let level = if indent > prev_indent {
            let l = prev_level + 1;
            level_indent.insert(l, indent);
            l
        } else if indent < prev_indent {
            // go up — find the matching level
            level_indent
                .iter()
                .find(|(_, &v)| v == indent)
                .map(|(&k, _)| k)
                .unwrap_or(1)
        } else {
            prev_level
        };

        // going deeper → the previous node becomes parent of this level
        if level > prev_level {
            level_parent.insert(level, NodeId(id - 1));
        }

        let parent = level_parent[&level];
        let (title, meta) = parse_title_and_meta(title);
        let hidden = title.starts_with(HIDDEN_PREFIX);

        let node = Node {
            title,
            meta,
            parent,
            children: Vec::new(),
            is_leaf: true,
            collapsed: false,
            hidden,
        };
        nodes.insert(NodeId(id), node);

        prev_indent = indent;
        prev_level = level;
        id += 1;
    }

    // wire children / is_leaf
    let ids: Vec<NodeId> = nodes.keys().copied().collect();
    for nid in &ids {
        let parent = nodes[nid].parent;
        if let Some(p) = nodes.get_mut(&parent) {
            p.is_leaf = false;
            p.children.push(*nid);
        }
    }

    // collect first-level ids (those whose parent is root_parent)
    let first_level: Vec<NodeId> = nodes
        .iter()
        .filter(|(_, n)| n.parent == root_parent)
        .map(|(&id, _)| id)
        .collect();

    (nodes, first_level)
}

// ── Serialize ────────────────────────────────────────────────

/// Serialize a subtree rooted at `id` back to the `.hmm` tab-indented format.
pub fn serialize(mm: &MindMap, id: NodeId) -> String {
    let mut buf = String::new();
    serialize_rec(mm, id, 0, &mut buf);
    buf
}

fn serialize_rec(mm: &MindMap, id: NodeId, depth: usize, buf: &mut String) {
    let node = mm.node(id);
    buf.push_str(&"\t".repeat(depth));
    buf.push_str(&serialize_title_and_meta(&node.title, &node.meta));
    buf.push('\n');
    for &cid in &node.children {
        serialize_rec(mm, cid, depth + 1, buf);
    }
}

fn parse_title_and_meta(raw: &str) -> (String, NodeMeta) {
    let trimmed = raw.trim_end();
    let Some(open_idx) = trimmed.rfind(" {") else {
        return (trimmed.to_string(), NodeMeta::default());
    };
    if !trimmed.ends_with('}') {
        return (trimmed.to_string(), NodeMeta::default());
    }

    let title = trimmed[..open_idx].to_string();
    let meta_inner = &trimmed[open_idx + 2..trimmed.len() - 1];
    let mut meta = NodeMeta::default();

    for token in meta_inner.split_whitespace() {
        let Some((key, value)) = token.split_once('=') else {
            continue;
        };
        match key {
            "linear" => meta.linear_identifier = Some(value.to_string()),
            "git" => meta.exported_git_head = Some(value.to_string()),
            _ => {}
        }
    }

    if meta.is_empty() {
        (trimmed.to_string(), NodeMeta::default())
    } else {
        (title, meta)
    }
}

fn serialize_title_and_meta(title: &str, meta: &NodeMeta) -> String {
    if meta.is_empty() {
        return title.to_string();
    }

    let mut tokens = Vec::new();
    if let Some(linear) = &meta.linear_identifier {
        tokens.push(format!("linear={}", linear));
    }
    if let Some(git) = &meta.exported_git_head {
        tokens.push(format!("git={}", git));
    }

    format!("{} {{{}}}", title, tokens.join(" "))
}

/// Serialize starting from the user root (skipping the super-root).
pub fn serialize_map(mm: &MindMap) -> String {
    serialize(mm, mm.root_id)
}

// ── Tests ────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_simple() {
        let input = "\
root
\tA
\tB
\t\tBa
\t\tBb
\tC
";
        let mm = parse(input);
        let output = serialize_map(&mm);
        assert_eq!(output, input);
    }

    #[test]
    fn round_trip_single_root() {
        let input = "hello\n";
        let mm = parse(input);
        assert_eq!(mm.root_id, NodeId(2));
        assert_eq!(mm.node(mm.root_id).title, "hello");
        let output = serialize_map(&mm);
        assert_eq!(output, input);
    }

    #[test]
    fn multiple_top_level_creates_synthetic_root() {
        let input = "A\nB\nC\n";
        let mm = parse(input);
        // synthetic root at id 1
        assert_eq!(mm.root_id, NodeId(1));
        assert_eq!(mm.node(NodeId(1)).title, "root");
        assert_eq!(mm.node(NodeId(1)).children.len(), 3);
    }

    #[test]
    fn bullet_normalization() {
        let input = "* root\n  * child\n";
        let mm = parse(input);
        assert_eq!(mm.node(mm.root_id).title, "root");
        let children = &mm.node(mm.root_id).children;
        assert_eq!(children.len(), 1);
        assert_eq!(mm.node(children[0]).title, "child");
    }

    #[test]
    fn empty_input() {
        let mm = parse("");
        assert_eq!(mm.node(mm.root_id).title, "root");
    }

    #[test]
    fn hidden_node() {
        let input = "root\n\t[HIDDEN] secret\n\tvisible\n";
        let mm = parse(input);
        let children = &mm.node(mm.root_id).children;
        assert_eq!(children.len(), 2);
        assert!(mm.node(children[0]).hidden);
        assert!(!mm.node(children[1]).hidden);
    }

    #[test]
    fn deeper_nesting() {
        let input = "\
top
\ta
\t\tb
\t\t\tc
\ta2
";
        let mm = parse(input);
        let output = serialize_map(&mm);
        assert_eq!(output, input);
    }

    #[test]
    fn parses_metadata_suffix() {
        let input = "root\n\tchild {linear=HCW-123 git=abc123}\n";
        let mm = parse(input);
        let child = mm.node(mm.node(mm.root_id).children[0]);
        assert_eq!(child.title, "child");
        assert_eq!(child.meta.linear_identifier.as_deref(), Some("HCW-123"));
        assert_eq!(child.meta.exported_git_head.as_deref(), Some("abc123"));
    }

    #[test]
    fn serializes_metadata_suffix() {
        let input = "root\n\tchild {linear=HCW-123 git=abc123}\n";
        let mm = parse(input);
        let output = serialize_map(&mm);
        assert_eq!(output, input);
    }
}
