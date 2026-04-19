/// Layout engine that mirrors the PHP version's pipeline:
///
/// 1. `calculate_x_and_lh`  – x position, display width, line height
/// 2. `calculate_h`         – subtree height (bottom-up)
/// 3. `calculate_y`         – y positions for every node
/// 4. `calculate_height_shift` – vertical centering offset (yo)
/// 5. Build a 2D character grid
/// 6. `draw_connections`    – Unicode box-drawing connectors
/// 7. `add_content`         – place node titles on the grid
use std::collections::BTreeMap;
use unicode_width::UnicodeWidthStr;

use crate::model::{MindMap, NodeId};

// ── Config ───────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct LayoutConfig {
    pub max_parent_width: usize,
    pub max_leaf_width: usize,
    pub line_spacing: usize,
    pub conn_left_len: usize,
    pub conn_right_len: usize,
    pub show_hidden: bool,
    pub left_padding: usize,
}

impl Default for LayoutConfig {
    fn default() -> Self {
        Self {
            max_parent_width: 25,
            max_leaf_width: 55,
            line_spacing: 1,
            conn_left_len: 6,
            conn_right_len: 4,
            show_hidden: false,
            left_padding: 1,
        }
    }
}

// ── Per-node layout data ─────────────────────────────────────

#[derive(Debug, Clone)]
pub struct NodeLayout {
    pub x: i32,
    pub y: i32,
    pub yo: i32,  // vertical centering offset
    pub w: i32,   // display width (columns)
    pub lh: i32,  // line height (number of wrapped lines)
    pub h: i32,   // subtree height
    pub clh: i32, // cumulative line height
    pub collapsed: bool,
    pub visible_children: Vec<NodeId>,
    pub lines: Vec<String>,
}

impl Default for NodeLayout {
    fn default() -> Self {
        Self {
            x: -1,
            y: -1,
            yo: 0,
            w: 0,
            lh: 1,
            h: -1,
            clh: 0,
            collapsed: false,
            visible_children: Vec::new(),
            lines: Vec::new(),
        }
    }
}

// ── Built map (2D grid) ──────────────────────────────────────

/// The final rendered map: a 2D grid of characters indexed by row.
#[derive(Debug, Clone)]
pub struct RenderedMap {
    pub rows: BTreeMap<i32, Vec<char>>,
    pub width: i32,
    pub top: i32,
    pub bottom: i32,
    pub layout: BTreeMap<NodeId, NodeLayout>,
}

impl RenderedMap {
    fn new() -> Self {
        Self {
            rows: BTreeMap::new(),
            width: 0,
            top: 0,
            bottom: 0,
            layout: BTreeMap::new(),
        }
    }

    /// Get a row, creating it if needed.
    fn ensure_row(&mut self, y: i32) {
        if !self.rows.contains_key(&y) {
            let w = (self.width + 40).max(200) as usize;
            self.rows.insert(y, vec![' '; w]);
        }
    }

    /// Put a string at (x, y) on the grid, overwriting existing chars.
    fn put(&mut self, x: i32, y: i32, s: &str) {
        if x < 0 {
            return;
        }
        self.ensure_row(y);
        let row = self.rows.get_mut(&y).unwrap();
        let x = x as usize;

        // extend row if needed
        let needed = x + s.chars().count() + 2;
        if row.len() < needed {
            row.resize(needed, ' ');
        }

        let mut col = x;
        for ch in s.chars() {
            if col < row.len() {
                row[col] = ch;
            }
            // most box-drawing chars are single-width in a monospace terminal
            col += 1;
        }
    }

    /// Read a single char at (x, y).
    fn get_char(&self, x: i32, y: i32) -> char {
        if let Some(row) = self.rows.get(&y) {
            let x = x as usize;
            if x < row.len() {
                row[x]
            } else {
                ' '
            }
        } else {
            ' '
        }
    }

    /// Render row `y` to a String.
    pub fn row_str(&self, y: i32) -> String {
        if let Some(row) = self.rows.get(&y) {
            row.iter().collect()
        } else {
            String::new()
        }
    }
}

// ── Public entry point ───────────────────────────────────────

/// Build the full rendered map from a mind-map.
pub fn build_map(mm: &MindMap, cfg: &LayoutConfig) -> RenderedMap {
    let mut nl: BTreeMap<NodeId, NodeLayout> = BTreeMap::new();
    let root = mm.root_id;

    // initialise layout for every node
    for (&id, node) in &mm.nodes {
        let mut l = NodeLayout::default();
        l.collapsed = node.collapsed;
        l.visible_children = if cfg.show_hidden {
            node.children.clone()
        } else {
            node.children
                .iter()
                .filter(|&&cid| !mm.node(cid).hidden)
                .copied()
                .collect()
        };
        nl.insert(id, l);
    }

    // super-root (id 0) bootstrap
    if let Some(l0) = nl.get_mut(&NodeId(0)) {
        l0.x = 0;
        l0.yo = 0;
        l0.w = cfg.left_padding as i32;
        l0.lh = 1;
    }

    let mut map_width: i32 = 0;

    // Phase 1: x and lh
    calc_x_and_lh(mm, &mut nl, root, cfg, &mut map_width);

    // Phase 2: h (bottom-up)
    calc_h(&mut nl, mm, cfg);

    // Phase 3: y + yo
    if let Some(l0) = nl.get_mut(&NodeId(0)) {
        l0.y = 0;
    }
    let mut map_top: i32 = 0;
    let mut map_bottom: i32 = 0;
    calc_children_y(&mut nl, mm, NodeId(0), cfg, &mut map_top, &mut map_bottom);

    // Phase 4: height shift (vertical centering)
    calc_height_shift(&mut nl, mm, root, 0);

    // Phase 5: build 2D grid
    let mut rmap = RenderedMap::new();
    rmap.width = map_width;
    rmap.top = map_top;
    rmap.bottom = map_bottom;

    let height = map_bottom.max(50);
    for y in map_top..=height {
        rmap.rows
            .insert(y, vec![' '; (map_width + 40).max(200) as usize]);
    }

    // Phase 6: draw connections
    draw_connections(mm, &nl, root, cfg, &mut rmap);

    // Phase 7: add text content
    add_content(mm, &nl, root, cfg, &mut rmap);

    rmap.layout = nl;
    rmap
}

// ── Phase 1: x and lh ───────────────────────────────────────

fn calc_x_and_lh(
    mm: &MindMap,
    nl: &mut BTreeMap<NodeId, NodeLayout>,
    id: NodeId,
    cfg: &LayoutConfig,
    map_width: &mut i32,
) {
    let node = mm.node(id);
    let parent = node.parent;

    // x = parent.x + parent.w + connectors
    let (px, pw) = if let Some(pl) = nl.get(&parent) {
        (pl.x, pl.w)
    } else {
        (0, 0)
    };

    let is_child_of_superroot = parent == NodeId(0);
    let x = if is_child_of_superroot {
        // PHP: parent.x + parent.w + 1 + (1 - conn_right - conn_left)
        // For the user root, this simplifies to left_padding + 1
        (px + pw + 1).max(0)
    } else {
        px + pw + cfg.conn_left_len as i32 + cfg.conn_right_len as i32 + 1
    };

    let vis = nl
        .get(&id)
        .map(|l| l.visible_children.clone())
        .unwrap_or_default();
    let at_end = node.is_leaf || node.collapsed || vis.is_empty();

    let max_w = if at_end {
        cfg.max_leaf_width
    } else {
        cfg.max_parent_width
    };

    let lines = wrap_title(&node.title, max_w);
    let w: i32 = lines
        .iter()
        .map(|l| UnicodeWidthStr::width(l.as_str()) as i32)
        .max()
        .unwrap_or(0);
    let lh = lines.len() as i32;

    if let Some(l) = nl.get_mut(&id) {
        l.x = x;
        l.w = w;
        l.lh = lh;
        l.lines = lines;
        l.clh = if at_end { lh } else { 0 };
    }

    *map_width = (*map_width).max(x + w);

    let vis_clone = vis.clone();
    for &cid in &vis_clone {
        calc_x_and_lh(mm, nl, cid, cfg, map_width);
        let child_clh = nl.get(&cid).map(|l| l.clh).unwrap_or(0);
        if let Some(l) = nl.get_mut(&id) {
            l.clh += child_clh;
        }
    }
}

// ── Phase 2: h (bottom-up iterative) ────────────────────────

fn calc_h(nl: &mut BTreeMap<NodeId, NodeLayout>, mm: &MindMap, cfg: &LayoutConfig) {
    let mut unfinished = true;
    while unfinished {
        unfinished = false;
        let ids: Vec<NodeId> = nl.keys().copied().collect();
        for id in ids {
            let node = mm.node(id);
            let vis = nl[&id].visible_children.clone();
            let at_end = node.is_leaf || vis.is_empty() || nl[&id].collapsed;

            if at_end {
                nl.get_mut(&id).unwrap().h = cfg.line_spacing as i32 + nl[&id].lh;
            } else {
                let mut h: i32 = 0;
                let mut ready = true;
                for &cid in &vis {
                    let ch = nl[&cid].h;
                    if ch >= 0 {
                        h += ch;
                    } else {
                        ready = false;
                        break;
                    }
                }
                if ready {
                    let lh = nl[&id].lh;
                    nl.get_mut(&id).unwrap().h = h.max(lh + cfg.line_spacing as i32);
                } else {
                    unfinished = true;
                }
            }
        }
    }
}

// ── Phase 3: y and yo ────────────────────────────────────────

fn calc_children_y(
    nl: &mut BTreeMap<NodeId, NodeLayout>,
    mm: &MindMap,
    pid: NodeId,
    cfg: &LayoutConfig,
    map_top: &mut i32,
    map_bottom: &mut i32,
) {
    let py = nl[&pid].y;
    let ph = nl[&pid].h;
    let plh = nl[&pid].lh;

    // yo = center within own subtree height
    let yo = ((ph - plh) as f64 / 2.0).round() as i32;
    nl.get_mut(&pid).unwrap().yo = yo;

    let vis = nl[&pid].visible_children.clone();
    let collapsed = nl[&pid].collapsed;

    if !collapsed {
        let mut y = py;
        for &cid in &vis {
            nl.get_mut(&cid).unwrap().y = y;
            let clh = nl[&cid].lh;
            *map_bottom = (*map_bottom).max(clh + cfg.line_spacing as i32 + y);
            *map_top = (*map_top).min(y);
            y += nl[&cid].h;
            calc_children_y(nl, mm, cid, cfg, map_top, map_bottom);
        }
    }
}

// ── Phase 4: height shift ────────────────────────────────────

fn calc_height_shift(
    nl: &mut BTreeMap<NodeId, NodeLayout>,
    mm: &MindMap,
    id: NodeId,
    mut shift: i32,
) {
    nl.get_mut(&id).unwrap().yo += shift;

    let lh = nl[&id].lh as f64;
    let clh = nl[&id].clh as f64;
    shift += ((lh - clh) / 2.0 - 0.9).floor().max(0.0) as i32;

    let vis = nl[&id].visible_children.clone();
    let collapsed = nl[&id].collapsed;
    if !collapsed {
        for &cid in &vis {
            calc_height_shift(nl, mm, cid, shift);
        }
    }
}

// ── Phase 6: draw connections ────────────────────────────────

fn draw_connections(
    mm: &MindMap,
    nl: &BTreeMap<NodeId, NodeLayout>,
    id: NodeId,
    cfg: &LayoutConfig,
    rmap: &mut RenderedMap,
) {
    let l = &nl[&id];
    let vis = &l.visible_children;
    let all_children = &mm.node(id).children;
    let num_vis = vis.len();
    let num_all = all_children.len();

    // collapsed
    if l.collapsed && num_all > 0 {
        let cx = l.x + l.w + 1;
        let cy = l.y + l.yo;
        if num_vis == num_all {
            rmap.put(cx, cy, " [+]");
        } else {
            rmap.put(cx, cy, "─╫─ [+]");
        }
        return;
    }

    // no visible children
    if num_vis == 0 {
        if num_all > 0 {
            // has hidden children only
            let cx = l.x + l.w + 1;
            let cy = l.y + l.yo + (l.lh as f64 / 2.0 - 0.6).round() as i32;
            rmap.put(cx, cy, "─╫─");
        }
        return;
    }

    let conn_right = "─".repeat(cfg.conn_right_len - 2);
    let conn_left = "─".repeat(cfg.conn_left_len - 2);
    let conn_single = "─".repeat(cfg.conn_left_len + cfg.conn_right_len - 3);
    let has_hidden = num_vis != num_all;

    // single child
    if num_vis == 1 {
        let cid = vis[0];
        let cl = &nl[&cid];

        let y1 = l.y + l.yo + (l.lh as f64 / 2.0 - 0.6).round() as i32;
        let y2 = cl.y + cl.yo + (cl.lh as f64 / 2.0 - 0.6).round() as i32;

        // draw horizontal line from parent end to child start
        let hx = l.x + l.w + 1;
        let target_x = cl.x; // line reaches child's x
        let gap = (target_x - hx).max(0) as usize;
        let prefix = if has_hidden { "─╫" } else { "──" };
        if gap >= 2 {
            let line = format!("{}{}", prefix, "─".repeat(gap - 2));
            rmap.put(hx, y1.min(y2), &line);
        } else {
            rmap.put(hx, y1.min(y2), prefix);
        }

        if y1 != y2 {
            let (top, bot) = (y1.min(y2), y1.max(y2));
            for yy in top..bot {
                rmap.put(cl.x - 2, yy, "│");
            }
            if y2 > y1 {
                rmap.put(cl.x - 2, y2, "╰");
                rmap.put(cl.x - 2, y1, "╮");
            } else {
                rmap.put(cl.x - 2, y2, "╭");
                rmap.put(cl.x - 2, y1, "╯");
            }
        }

        draw_connections(mm, nl, cid, cfg, rmap);
        return;
    }

    // multiple children
    let mut top_y = i32::MAX;
    let mut bot_y = i32::MIN;
    let mut top_child = vis[0];
    let mut bot_child = vis[0];

    for &cid in vis {
        let cl = &nl[&cid];
        let cy = cl.y + cl.yo;
        if cy < top_y {
            top_y = cy;
            top_child = cid;
        }
        if cy > bot_y {
            bot_y = cy;
            bot_child = cid;
        }
    }

    let middle = l.y + l.yo + (l.lh as f64 / 2.0 - 0.6).round() as i32;
    let tc = &nl[&top_child];
    let vert_x = tc.x - cfg.conn_right_len as i32;

    // horizontal line from parent to vertical bar
    // line should reach exactly up to vert_x (not past it)
    let hx = l.x + l.w + 1;
    let gap = (vert_x - hx).max(0) as usize;
    let prefix = if has_hidden { "─╫" } else { "──" };
    if gap >= 2 {
        let hline = format!("{}{}", prefix, "─".repeat(gap - 2));
        rmap.put(hx, middle, &hline);
    } else {
        rmap.put(hx, middle, prefix);
    }

    // vertical bar
    for yy in top_y..bot_y {
        rmap.put(vert_x, yy, "│");
    }

    // top corner ╭── and bottom corner ╰──
    rmap.put(vert_x, top_y, &format!("╭{}", conn_right));
    rmap.put(vert_x, bot_y, &format!("╰{}", conn_right));

    // middle children get ├──
    if num_vis > 2 {
        for &cid in vis {
            if cid != top_child && cid != bot_child {
                let cl = &nl[&cid];
                let cy = cl.y + cl.yo + (cl.lh as f64 / 2.0 - 0.2).round() as i32;
                rmap.put(vert_x, cy, &format!("├{}", conn_right));
            }
        }
    }

    // fix junction where horizontal meets vertical
    let junction_char = rmap.get_char(vert_x, middle);
    let replacement = match junction_char {
        '│' => Some('┤'),
        '╭' => Some('┬'),
        '├' => Some('┼'),
        _ => None,
    };
    if let Some(ch) = replacement {
        rmap.put(vert_x, middle, &ch.to_string());
    }

    // recurse
    for &cid in vis {
        draw_connections(mm, nl, cid, cfg, rmap);
    }
}

// ── Phase 7: add content ─────────────────────────────────────

fn add_content(
    mm: &MindMap,
    nl: &BTreeMap<NodeId, NodeLayout>,
    id: NodeId,
    cfg: &LayoutConfig,
    rmap: &mut RenderedMap,
) {
    let l = &nl[&id];

    for (i, line) in l.lines.iter().enumerate() {
        let px = l.x;
        let py = l.y + l.yo + i as i32;
        rmap.put(px, py, &format!("{} ", line));
    }

    if !l.collapsed {
        for &cid in &l.visible_children {
            add_content(mm, nl, cid, cfg, rmap);
        }
    }
}

// ── Word wrap ────────────────────────────────────────────────

fn wrap_title(title: &str, max_w: usize) -> Vec<String> {
    if max_w == 0 {
        return vec![title.to_string()];
    }
    let display_w = UnicodeWidthStr::width(title);
    if display_w <= (max_w as f64 * 1.3) as usize {
        return vec![title.to_string()];
    }

    let mut lines = Vec::new();
    let mut current = String::new();
    let mut current_w: usize = 0;

    for word in title.split(' ') {
        let word_w = UnicodeWidthStr::width(word);
        if current.is_empty() {
            current = word.to_string();
            current_w = word_w;
        } else if current_w + 1 + word_w <= max_w {
            current.push(' ');
            current.push_str(word);
            current_w += 1 + word_w;
        } else {
            lines.push(current);
            current = word.to_string();
            current_w = word_w;
        }
    }
    if !current.is_empty() {
        lines.push(current);
    }
    if lines.is_empty() {
        lines.push(title.to_string());
    }
    lines
}

// ── Tests ────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser;

    #[test]
    fn builds_without_panic() {
        let mm = parser::parse("root\n\tA\n\tB\n\tC\n");
        let cfg = LayoutConfig::default();
        let rmap = build_map(&mm, &cfg);
        assert!(!rmap.rows.is_empty());
    }

    #[test]
    fn root_text_appears_in_grid() {
        let mm = parser::parse("root\n\tA\n\tB\n");
        let cfg = LayoutConfig::default();
        let rmap = build_map(&mm, &cfg);

        let has_root = rmap.rows.values().any(|row| {
            let s: String = row.iter().collect();
            s.contains("root")
        });
        assert!(has_root, "grid should contain 'root'");
    }

    #[test]
    fn connections_present() {
        let mm = parser::parse("root\n\tA\n\tB\n\tC\n");
        let cfg = LayoutConfig::default();
        let rmap = build_map(&mm, &cfg);

        let all: String = rmap
            .rows
            .values()
            .map(|r| r.iter().collect::<String>())
            .collect::<Vec<_>>()
            .join("\n");

        assert!(all.contains('╭'), "should have top corner");
        assert!(all.contains('╰'), "should have bottom corner");
        assert!(all.contains('│'), "should have vertical bar");
        assert!(all.contains('─'), "should have horizontal line");
    }

    #[test]
    fn collapsed_shows_marker() {
        let mut mm = parser::parse("root\n\tA\n\t\tA1\n\tB\n");
        // collapse A
        let a_id = mm.node(mm.root_id).children[0];
        mm.node_mut(a_id).collapsed = true;

        let cfg = LayoutConfig::default();
        let rmap = build_map(&mm, &cfg);

        let all: String = rmap
            .rows
            .values()
            .map(|r| r.iter().collect::<String>())
            .collect::<Vec<_>>()
            .join("\n");

        assert!(all.contains("[+]"), "collapsed node should show [+]");
    }

    #[test]
    fn wrap_short() {
        let lines = wrap_title("hello", 50);
        assert_eq!(lines, vec!["hello"]);
    }

    #[test]
    fn wrap_long() {
        let lines = wrap_title(
            "this is a longer title that definitely should be wrapped into multiple lines",
            20,
        );
        assert!(lines.len() > 1);
    }
}
