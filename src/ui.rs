/// Terminal UI layer: render the mind map and handle keyboard events.
use std::io;
use std::time::Duration;

use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers};
use crossterm::execute;
use crossterm::terminal::{self, EnterAlternateScreen, LeaveAlternateScreen};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::symbols::border;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};
use ratatui::Terminal;

use crate::layout::{self, LayoutConfig, RenderedMap};
use crate::linear;
use crate::model::{MindMap, NodeId};
use crate::ops::{self, UndoHistory};

// ── App state ────────────────────────────────────────────────

pub struct App {
    pub mm: MindMap,
    pub history: UndoHistory,
    pub cfg: LayoutConfig,
    pub filename: Option<String>,
    pub modified: bool,
    pub message: Option<String>,
    pub query: Option<String>,
    pub workdir: String,
    pub linear_team_slug: Option<String>,
    pub current_git_head: Option<String>,
    pub show_hidden: bool,
    pub center_lock: bool,
    pub focus_lock: bool,
    pub editor: Option<EditorState>,
    pub confirm: Option<ConfirmState>,
    /// Viewport scroll offset.
    pub scroll_y: i32,
    pub scroll_x: i32,
}

#[derive(Debug, Clone)]
pub struct EditorState {
    pub node_id: NodeId,
    pub buffer: String,
    pub cursor: usize,
    pub original_title: String,
    pub is_new_node: bool,
    pub original_modified: bool,
}

#[derive(Debug, Clone)]
pub struct ConfirmState {
    pub action: ConfirmAction,
    pub message: String,
}

#[derive(Debug, Clone, Copy)]
pub enum ConfirmAction {
    ExportCurrent,
    ExportSubtree,
}

impl App {
    pub fn new(mm: MindMap, filename: Option<String>) -> Self {
        let workdir = std::env::current_dir()
            .ok()
            .and_then(|p| p.into_os_string().into_string().ok())
            .unwrap_or_else(|| ".".to_string());

        Self {
            mm,
            history: UndoHistory::new(24),
            cfg: LayoutConfig::default(),
            filename,
            modified: false,
            message: None,
            query: None,
            workdir: workdir.clone(),
            linear_team_slug: None,
            current_git_head: linear::current_git_head(&workdir),
            show_hidden: false,
            center_lock: false,
            focus_lock: false,
            editor: None,
            confirm: None,
            scroll_y: 0,
            scroll_x: 0,
        }
    }
}

// ── Run ──────────────────────────────────────────────────────

pub fn run(mut app: App) -> io::Result<()> {
    // setup terminal
    terminal::enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, crossterm::cursor::Hide)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // initial collapse
    ops::collapse_all(&mut app.mm);
    ops::collapse_level(&mut app.mm, 1);

    let result = main_loop(&mut terminal, &mut app);

    // restore terminal
    terminal::disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        crossterm::cursor::Show
    )?;

    result
}

fn main_loop(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut App,
) -> io::Result<()> {
    loop {
        // draw
        terminal.draw(|f| draw(f, app))?;

        // poll for events
        if event::poll(Duration::from_millis(50))? {
            if let Event::Key(key) = event::read()? {
                app.message = None; // clear status on any key

                if handle_key(app, key) {
                    break; // quit
                }
            }
        }
    }
    Ok(())
}

// ── Draw ─────────────────────────────────────────────────────

fn draw(f: &mut ratatui::Frame, app: &mut App) {
    let area = f.area();
    let term_w = area.width as usize;
    let status_height = 3;
    let map_height = area.height.saturating_sub(status_height + 1) as i32;

    if app.focus_lock {
        ops::focus(&mut app.mm);
    }

    app.cfg.show_hidden = app.show_hidden;
    let rmap = layout::build_map(&app.mm, &app.cfg);
    if app.center_lock {
        center_active_node_in_view(app, &rmap, area.width as i32, map_height, false);
    }

    // find active node position for auto-scroll
    if let Some(al) = rmap.layout.get(&app.mm.active_node) {
        let ax = al.x;
        let aw = al.w + 2;
        let ay = al.y + al.yo;
        let alh = al.lh;
        if ax < app.scroll_x {
            app.scroll_x = ax;
        }
        if ax + aw > app.scroll_x + area.width as i32 {
            app.scroll_x = ax + aw - area.width as i32;
        }
        if ay < app.scroll_y {
            app.scroll_y = ay;
        }
        if ay + alh > app.scroll_y + map_height {
            app.scroll_y = ay + alh - map_height;
        }
    }

    // get active node region for highlighting
    let (ax1, ax2, ay1, ay2) = if let Some(al) = rmap.layout.get(&app.mm.active_node) {
        let x1 = (al.x - 1 - app.scroll_x).max(0) as usize;
        let x2 = (al.x + al.w + 1 - app.scroll_x).max(0) as usize;
        let y1 = al.y + al.yo;
        let y2 = al.y + al.yo + al.lh;
        (x1, x2, y1, y2)
    } else {
        (0, 0, 0, 0)
    };

    let active_style = Style::default()
        .fg(Color::Black)
        .bg(Color::Rgb(214, 137, 36))
        .add_modifier(Modifier::BOLD);
    let line_style = Style::default().fg(Color::Rgb(120, 80, 80));
    let collapsed_style = Style::default().fg(Color::Rgb(215, 175, 95));
    let dim_style = Style::default().add_modifier(Modifier::DIM);
    let default_style = Style::default();

    let y_min = app.scroll_y;
    let y_max = app.scroll_y + map_height;

    let mut lines: Vec<Line> = Vec::new();

    for row_y in y_min..=y_max {
        let row_str = rmap.row_str(row_y);
        // truncate to terminal width
        let display: String = row_str
            .chars()
            .skip(app.scroll_x.max(0) as usize)
            .take(term_w)
            .collect();

        if row_y >= ay1 && row_y < ay2 {
            // row contains the active node — split into 3 segments
            let chars: Vec<char> = display.chars().collect();
            let len = chars.len();
            let x1 = ax1.min(len);
            let x2 = ax2.min(len);

            let before: String = chars[..x1].iter().collect();
            let highlighted: String = chars[x1..x2].iter().collect();
            let after: String = chars[x2..].iter().collect();

            let mut spans = Vec::new();
            if !before.is_empty() {
                spans.extend(style_line_segment(
                    &before,
                    &line_style,
                    &collapsed_style,
                    &dim_style,
                    &default_style,
                ));
            }
            if !highlighted.is_empty() {
                spans.push(Span::styled(highlighted, active_style));
            }
            if !after.is_empty() {
                spans.extend(style_line_segment(
                    &after,
                    &line_style,
                    &collapsed_style,
                    &dim_style,
                    &default_style,
                ));
            }
            lines.push(Line::from(spans));
        } else {
            let spans = style_line_segment(
                &display,
                &line_style,
                &collapsed_style,
                &dim_style,
                &default_style,
            );
            lines.push(Line::from(spans));
        }
    }

    // pad
    while (lines.len() as i32) < map_height {
        lines.push(Line::from(""));
    }

    let paragraph = Paragraph::new(lines);
    let map_area = Rect::new(
        0,
        0,
        area.width,
        area.height.saturating_sub(status_height + 1),
    );
    f.render_widget(paragraph, map_area);

    // bottom popup/status box
    let panel_margin_x = if area.width > 8 { 2 } else { 0 };
    let status_area = Rect::new(
        panel_margin_x,
        area.height.saturating_sub(status_height),
        area.width.saturating_sub(panel_margin_x * 2),
        status_height,
    );
    let status = Paragraph::new(render_status_lines(app))
        .block(
            Block::default()
                .title(if app.editor.is_some() {
                    " Edit "
                } else if app.confirm.is_some() {
                    " Confirm "
                } else {
                    " Status "
                })
                .title_style(Style::default().fg(Color::Rgb(190, 170, 220)))
                .borders(Borders::ALL)
                .border_set(border::ROUNDED)
                .border_style(Style::default().fg(Color::Rgb(153, 102, 204)))
                .style(Style::default().bg(Color::Rgb(24, 20, 34))),
        )
        .wrap(Wrap { trim: false });
    f.render_widget(status, status_area);
}

fn center_active_node(app: &mut App, only_vertically: bool) {
    let (term_w, term_h) = terminal::size().unwrap_or((80, 24));
    let map_height = (term_h.saturating_sub(4)).max(1) as i32;
    app.cfg.show_hidden = app.show_hidden;
    let rmap = layout::build_map(&app.mm, &app.cfg);
    center_active_node_in_view(app, &rmap, term_w as i32, map_height, only_vertically);
}

fn center_active_node_in_view(
    app: &mut App,
    rmap: &RenderedMap,
    view_w: i32,
    view_h: i32,
    only_vertically: bool,
) {
    let Some(al) = rmap.layout.get(&app.mm.active_node) else {
        return;
    };

    let midx = al.x + (al.w / 2);
    let midy = al.y + al.yo + (al.lh / 2);

    if !only_vertically {
        app.scroll_x = (midx - (view_w / 2)).max(0);
    }
    app.scroll_y = midy - (view_h / 2);
}

fn toggle_center_lock(app: &mut App) {
    app.center_lock = !app.center_lock;
    if app.center_lock {
        center_active_node(app, false);
        app.message = Some("center lock on".to_string());
    } else {
        app.message = Some("center lock off".to_string());
    }
}

fn focus_active_node(app: &mut App) {
    ops::focus(&mut app.mm);
    center_active_node(app, false);
}

fn toggle_focus_lock(app: &mut App) {
    app.focus_lock = !app.focus_lock;
    app.message = Some(if app.focus_lock {
        "focus lock on".to_string()
    } else {
        "focus lock off".to_string()
    });
}

fn render_status_lines(app: &App) -> Vec<Line<'static>> {
    let bar_style = Style::default()
        .fg(Color::Rgb(220, 214, 230))
        .bg(Color::Rgb(24, 20, 34));

    if let Some(confirm) = &app.confirm {
        return vec![Line::from(vec![
            Span::styled(
                " action ".to_string(),
                badge_style(Color::Rgb(90, 140, 220)),
            ),
            Span::styled(confirm.message.clone(), bar_style),
            Span::styled(
                "  Enter confirm, Esc cancel".to_string(),
                Style::default()
                    .fg(Color::Rgb(160, 150, 180))
                    .bg(Color::Rgb(24, 20, 34)),
            ),
        ])];
    }

    if let Some(editor) = &app.editor {
        let prompt = " edit> ";
        let chars: Vec<char> = editor.buffer.chars().collect();
        let cursor = editor.cursor.min(chars.len());

        let before: String = chars[..cursor].iter().collect();
        let cursor_ch = chars.get(cursor).copied().unwrap_or(' ');
        let after: String = if cursor < chars.len() {
            chars[cursor + 1..].iter().collect()
        } else {
            String::new()
        };

        return vec![Line::from(vec![
            Span::styled(prompt.to_string(), bar_style),
            Span::styled(before, bar_style),
            Span::styled(
                cursor_ch.to_string(),
                Style::default()
                    .fg(Color::Rgb(24, 20, 34))
                    .bg(Color::Rgb(214, 137, 36))
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(after, bar_style),
            Span::styled(
                "  Enter save, Esc cancel".to_string(),
                Style::default()
                    .fg(Color::Rgb(160, 150, 180))
                    .bg(Color::Rgb(24, 20, 34)),
            ),
        ])];
    }

    if let Some(ref msg) = app.message {
        return vec![Line::from(Span::styled(msg.clone(), bar_style))];
    }

    if let Some(ref q) = app.query {
        return vec![Line::from(Span::styled(format!("/{}", q), bar_style))];
    }

    let fname = app.filename.as_deref().unwrap_or("[new]");
    let mod_indicator = if app.modified { " [+]" } else { "" };
    let active = app.mm.node(app.mm.active_node);
    let issue = active.meta.linear_identifier.as_deref().unwrap_or("-");
    let git = active
        .meta
        .exported_git_head
        .as_deref()
        .or(app.current_git_head.as_deref())
        .unwrap_or("uncommitted");
    let parent_status = parent_status_text(app, app.mm.active_node);
    let push_preview = export_preview_text(app, app.mm.active_node);
    vec![Line::from(vec![
        Span::styled(format!(" {}{} ", fname, mod_indicator), bar_style),
        Span::styled(
            " linear ".to_string(),
            badge_style(Color::Rgb(153, 102, 204)),
        ),
        Span::styled(format!(" {} ", issue), bar_style),
        Span::styled(" git ".to_string(), badge_style(Color::Rgb(110, 110, 140))),
        Span::styled(format!(" {} ", git), bar_style),
        Span::styled(
            " parent ".to_string(),
            badge_style(Color::Rgb(120, 90, 140)),
        ),
        Span::styled(format!(" {} ", parent_status), bar_style),
        Span::styled(
            format!(" push:{} ", push_preview),
            push_badge_style(push_preview),
        ),
        Span::styled(" Ctrl+L open  x/X push ".to_string(), bar_style),
    ])]
}

fn badge_style(bg: Color) -> Style {
    Style::default()
        .fg(Color::Rgb(245, 242, 250))
        .bg(bg)
        .add_modifier(Modifier::BOLD)
}

fn push_badge_style(kind: &str) -> Style {
    let bg = match kind {
        "update" => Color::Rgb(61, 122, 72),
        "create-main" => Color::Rgb(80, 120, 190),
        "create-sub" => Color::Rgb(88, 100, 190),
        "blocked" => Color::Rgb(150, 64, 64),
        "root" => Color::Rgb(110, 110, 110),
        _ => Color::Rgb(80, 80, 80),
    };
    badge_style(bg)
}

fn parent_status_text(app: &App, id: NodeId) -> String {
    if id == app.mm.root_id {
        return "root not exportable".to_string();
    }

    let parent_id = app.mm.node(id).parent;
    if parent_id == app.mm.root_id {
        return "main issue".to_string();
    }

    app.mm
        .node(parent_id)
        .meta
        .linear_identifier
        .as_ref()
        .map(|v| format!("parent:{}", v))
        .unwrap_or_else(|| "parent not exported".to_string())
}

fn export_preview_text(app: &App, id: NodeId) -> &'static str {
    if id == app.mm.root_id {
        return "root";
    }

    let node = app.mm.node(id);
    if node.meta.linear_identifier.is_some() {
        return "update";
    }

    if node.parent == app.mm.root_id {
        return "create-main";
    }

    if app.mm.node(node.parent).meta.linear_identifier.is_some() {
        "create-sub"
    } else {
        "blocked"
    }
}

/// Style a line segment: colour box-drawing chars, dim codes, highlight [+].
fn style_line_segment<'a>(
    text: &str,
    line_style: &Style,
    collapsed_style: &Style,
    _dim_style: &Style,
    default_style: &Style,
) -> Vec<Span<'a>> {
    let mut spans = Vec::new();
    let mut current = String::new();
    let mut current_is_line = false;

    let line_chars = ['─', '│', '╭', '╰', '├', '┤', '┬', '┼', '╮', '╯', '╫'];

    for ch in text.chars() {
        let is_line = line_chars.contains(&ch);

        if is_line != current_is_line && !current.is_empty() {
            let style = if current_is_line {
                *line_style
            } else {
                *default_style
            };
            spans.push(Span::styled(current.clone(), style));
            current.clear();
        }
        current.push(ch);
        current_is_line = is_line;
    }

    if !current.is_empty() {
        let style = if current_is_line {
            *line_style
        } else {
            *default_style
        };
        spans.push(Span::styled(current, style));
    }

    // check for [+] and re-style it
    let mut result = Vec::new();
    for span in spans {
        if span.content.contains("[+]") {
            let parts: Vec<&str> = span.content.splitn(2, "[+]").collect();
            if !parts[0].is_empty() {
                result.push(Span::styled(parts[0].to_string(), span.style));
            }
            result.push(Span::styled("[+]".to_string(), *collapsed_style));
            if parts.len() > 1 && !parts[1].is_empty() {
                result.push(Span::styled(parts[1].to_string(), span.style));
            }
        } else {
            result.push(span);
        }
    }

    result
}

// ── Key handling ─────────────────────────────────────────────

/// Returns `true` if the app should quit.
fn handle_key(app: &mut App, key: KeyEvent) -> bool {
    if app.editor.is_some() {
        handle_editor_key(app, key);
        return false;
    }

    if app.confirm.is_some() {
        handle_confirm_key(app, key);
        return false;
    }

    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);

    match key.code {
        // ── Quit ──
        KeyCode::Char('q') if !ctrl => {
            if app.modified {
                app.message = Some("Unsaved changes. Shift+Q to force quit.".into());
                return false;
            }
            return true;
        }
        KeyCode::Char('Q') => return true,
        KeyCode::Char('c') if ctrl => return true,
        KeyCode::Char('l') if ctrl => open_current_issue(app),

        // ── Navigation (ijkl = up/left/down/right) ──
        KeyCode::Char('j') | KeyCode::Left => nav_left(app),
        KeyCode::Char('l') | KeyCode::Right => nav_right(app),
        KeyCode::Char('k') | KeyCode::Down => nav_down(app),
        KeyCode::Char('i') | KeyCode::Up => nav_up(app),
        KeyCode::Char('g') => go_to_top(app),
        KeyCode::Char('G') => go_to_bottom(app),
        KeyCode::Char('m') | KeyCode::Char('~') => {
            app.mm.active_node = app.mm.root_id;
        }
        KeyCode::Char('c') => center_active_node(app, false),
        KeyCode::Char('C') => toggle_center_lock(app),
        KeyCode::Char('f') => focus_active_node(app),
        KeyCode::Char('F') => toggle_focus_lock(app),

        // ── Collapse / Expand ──
        KeyCode::Char(' ') => ops::toggle_node(&mut app.mm),
        KeyCode::Char('v') => ops::collapse_all(&mut app.mm),
        KeyCode::Char('b') => ops::expand_all(&mut app.mm),
        KeyCode::Char(c @ '1'..='9') => {
            let level = (c as usize) - ('0' as usize);
            ops::collapse_level(&mut app.mm, level);
        }

        // ── Insert ──
        KeyCode::Char('o') | KeyCode::Enter => {
            let original_modified = app.modified;
            app.history.push(&app.mm);
            ops::insert_node(&mut app.mm, ops::InsertKind::Sibling, "");
            app.modified = true;
            start_editing(app, app.mm.active_node, true, original_modified);
        }
        KeyCode::Char('O') | KeyCode::Tab => {
            let original_modified = app.modified;
            app.history.push(&app.mm);
            ops::insert_node(&mut app.mm, ops::InsertKind::Child, "");
            app.modified = true;
            start_editing(app, app.mm.active_node, true, original_modified);
        }

        // ── Edit ──
        KeyCode::Char('e') => {
            let id = app.mm.active_node;
            start_editing(app, id, false, app.modified);
        }

        // ── Delete ──
        KeyCode::Char('d') => {
            app.history.push(&app.mm);
            let id = app.mm.active_node;
            ops::delete_node(&mut app.mm, id);
            app.modified = true;
        }
        KeyCode::Char('D') => {
            app.history.push(&app.mm);
            let id = app.mm.active_node;
            ops::delete_children(&mut app.mm, id);
            app.modified = true;
        }

        // ── Move ──
        KeyCode::Char('J') => {
            app.history.push(&app.mm);
            if ops::move_node_down(&mut app.mm, app.show_hidden) {
                app.modified = true;
            }
        }
        KeyCode::Char('K') => {
            app.history.push(&app.mm);
            if ops::move_node_up(&mut app.mm, app.show_hidden) {
                app.modified = true;
            }
        }

        // ── Sort ──
        KeyCode::Char('T') => {
            app.history.push(&app.mm);
            ops::sort_siblings(&mut app.mm);
            app.modified = true;
        }

        // ── Undo / Redo ──
        KeyCode::Char('u') => {
            if app.history.undo(&mut app.mm) {
                app.modified = true;
            }
        }

        // ── Save ──
        KeyCode::Char('s') => save(app),

        // ── Toggle symbol ──
        KeyCode::Char('t') => {
            app.history.push(&app.mm);
            ops::toggle_symbol(&mut app.mm, "✓", "✗");
            app.modified = true;
        }

        // ── Linear export ──
        KeyCode::Char('x') => begin_export_confirm(app, ConfirmAction::ExportCurrent),
        KeyCode::Char('X') => begin_export_confirm(app, ConfirmAction::ExportSubtree),

        _ => {}
    }

    false
}

fn begin_export_confirm(app: &mut App, action: ConfirmAction) {
    let subject = if action_is_recursive(action) {
        if app.mm.active_node == app.mm.root_id {
            "all root children"
        } else {
            "current subtree"
        }
    } else {
        "current node"
    };

    let push_preview = export_preview_text(app, app.mm.active_node);
    app.confirm = Some(ConfirmState {
        action,
        message: format!("push {} to Linear? preview={}", subject, push_preview),
    });
}

fn action_is_recursive(action: ConfirmAction) -> bool {
    matches!(action, ConfirmAction::ExportSubtree)
}

fn handle_confirm_key(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Esc => {
            app.confirm = None;
        }
        KeyCode::Enter => {
            let Some(confirm) = app.confirm.take() else {
                return;
            };
            push_current_node(app, action_is_recursive(confirm.action));
        }
        _ => {}
    }
}

fn open_current_issue(app: &mut App) {
    let Some(identifier) = app
        .mm
        .node(app.mm.active_node)
        .meta
        .linear_identifier
        .clone()
    else {
        app.message = Some("current node has no linked Linear issue".to_string());
        return;
    };

    let url = match linear::issue_url(&app.workdir, &identifier) {
        Ok(url) => url,
        Err(err) => {
            app.message = Some(err);
            return;
        }
    };

    let result = if cfg!(target_os = "macos") {
        std::process::Command::new("open")
            .arg(&url)
            .current_dir(&app.workdir)
            .output()
    } else if cfg!(target_os = "linux") {
        std::process::Command::new("xdg-open")
            .arg(&url)
            .current_dir(&app.workdir)
            .output()
    } else {
        app.message = Some(format!("issue URL: {}", url));
        return;
    };

    match result {
        Ok(output) if output.status.success() => {
            app.message = Some(format!("opened {}", identifier));
        }
        Ok(output) => {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            app.message = Some(if stderr.is_empty() {
                format!("failed to open {}", identifier)
            } else {
                stderr
            });
        }
        Err(err) => {
            app.message = Some(format!("failed to open issue: {}", err));
        }
    }
}

fn push_current_node(app: &mut App, recursive: bool) {
    let team_slug = match ensure_linear_team(app) {
        Ok(team_slug) => team_slug,
        Err(err) => {
            app.message = Some(err);
            return;
        }
    };

    let result = if recursive && app.mm.active_node == app.mm.root_id {
        export_root_children(app, &team_slug)
    } else {
        export_node(app, app.mm.active_node, recursive, &team_slug)
    };

    match result {
        Ok(count) => match persist_to_disk(app) {
            Ok(()) => {
                app.modified = false;
                app.current_git_head = linear::current_git_head(&app.workdir);
                app.message = Some(format!("exported {} node(s) to Linear", count));
            }
            Err(err) => {
                app.modified = true;
                app.message = Some(format!("exported {} node(s), save failed: {}", count, err));
            }
        },
        Err(err) => app.message = Some(err),
    }
}

fn ensure_linear_team(app: &mut App) -> Result<String, String> {
    if let Some(team) = &app.linear_team_slug {
        return Ok(team.clone());
    }

    let team = linear::detect_team_slug(&app.workdir)?;
    app.linear_team_slug = Some(team.clone());
    Ok(team)
}

fn export_root_children(app: &mut App, team_slug: &str) -> Result<usize, String> {
    let children = app.mm.node(app.mm.root_id).children.clone();
    let mut count = 0;
    for child in children {
        count += export_node(app, child, true, team_slug)?;
    }
    Ok(count)
}

fn export_node(
    app: &mut App,
    id: NodeId,
    recursive: bool,
    team_slug: &str,
) -> Result<usize, String> {
    if id == app.mm.root_id {
        return Err("root not exportable".to_string());
    }

    let parent_id = app.mm.node(id).parent;
    let parent_identifier = if parent_id == app.mm.root_id {
        None
    } else {
        Some(
            app.mm
                .node(parent_id)
                .meta
                .linear_identifier
                .clone()
                .ok_or_else(|| "parent not exported".to_string())?,
        )
    };

    let title = app.mm.node(id).title.trim().to_string();
    if title.is_empty() {
        return Err("cannot export empty title".to_string());
    }

    let issue = if let Some(existing) = app.mm.node(id).meta.linear_identifier.clone() {
        linear::update_issue(
            &app.workdir,
            &existing,
            &title,
            parent_identifier.as_deref(),
        )?
    } else {
        linear::create_issue(
            &app.workdir,
            team_slug,
            &title,
            parent_identifier.as_deref(),
        )?
    };

    let git_head = linear::current_git_head(&app.workdir);
    {
        let node = app.mm.node_mut(id);
        node.meta.linear_identifier = Some(issue.identifier);
        node.meta.exported_git_head = git_head;
    }

    let mut count = 1;
    if recursive {
        let children = app.mm.node(id).children.clone();
        for child in children {
            count += export_node(app, child, true, team_slug)?;
        }
    }

    Ok(count)
}

fn start_editing(app: &mut App, node_id: NodeId, is_new_node: bool, original_modified: bool) {
    let original_title = app.mm.node(node_id).title.clone();
    let cursor = original_title.chars().count();
    app.editor = Some(EditorState {
        node_id,
        buffer: original_title.clone(),
        cursor,
        original_title,
        is_new_node,
        original_modified,
    });
}

fn handle_editor_key(app: &mut App, key: KeyEvent) {
    let mut commit = false;
    let mut cancel = false;

    if let Some(editor) = app.editor.as_mut() {
        match key.code {
            KeyCode::Esc => cancel = true,
            KeyCode::Enter => commit = true,
            KeyCode::Left => {
                editor.cursor = editor.cursor.saturating_sub(1);
            }
            KeyCode::Right => {
                editor.cursor = (editor.cursor + 1).min(editor.buffer.chars().count());
            }
            KeyCode::Home => editor.cursor = 0,
            KeyCode::End => editor.cursor = editor.buffer.chars().count(),
            KeyCode::Backspace => {
                if editor.cursor > 0 {
                    remove_char_at(&mut editor.buffer, editor.cursor - 1);
                    editor.cursor -= 1;
                }
            }
            KeyCode::Delete => {
                if editor.cursor < editor.buffer.chars().count() {
                    remove_char_at(&mut editor.buffer, editor.cursor);
                }
            }
            KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                insert_char_at(&mut editor.buffer, editor.cursor, c);
                editor.cursor += 1;
            }
            KeyCode::Tab => {
                insert_str_at(&mut editor.buffer, editor.cursor, "  ");
                editor.cursor += 2;
            }
            _ => {}
        }
    }

    if commit {
        commit_editor(app);
    } else if cancel {
        cancel_editor(app);
    }
}

fn commit_editor(app: &mut App) {
    let Some(editor) = app.editor.take() else {
        return;
    };

    let new_title = sanitize_editor_text(&editor.buffer);
    if editor.is_new_node || new_title != editor.original_title {
        if !editor.is_new_node {
            app.history.push(&app.mm);
        }
        app.mm.node_mut(editor.node_id).title = new_title;
        app.modified = true;
    }
}

fn cancel_editor(app: &mut App) {
    let Some(editor) = app.editor.take() else {
        return;
    };

    if editor.is_new_node {
        let _ = app.history.undo(&mut app.mm);
        app.modified = editor.original_modified;
    }
}

fn sanitize_editor_text(text: &str) -> String {
    let cleaned = text.replace(['\n', '\r'], " ").replace('\t', "  ");
    cleaned
}

fn insert_char_at(buf: &mut String, char_idx: usize, ch: char) {
    let byte_idx = char_to_byte_idx(buf, char_idx);
    buf.insert(byte_idx, ch);
}

fn insert_str_at(buf: &mut String, char_idx: usize, s: &str) {
    let byte_idx = char_to_byte_idx(buf, char_idx);
    buf.insert_str(byte_idx, s);
}

fn remove_char_at(buf: &mut String, char_idx: usize) {
    let start = char_to_byte_idx(buf, char_idx);
    let end = char_to_byte_idx(buf, char_idx + 1);
    buf.replace_range(start..end, "");
}

fn char_to_byte_idx(s: &str, char_idx: usize) -> usize {
    s.char_indices()
        .nth(char_idx)
        .map(|(idx, _)| idx)
        .unwrap_or_else(|| s.len())
}

fn persist_to_disk(app: &App) -> Result<(), String> {
    let Some(filename) = app.filename.as_ref() else {
        return Err("no filename; exported metadata is only in memory".to_string());
    };

    let text = crate::parser::serialize_map(&app.mm);
    std::fs::write(filename, &text).map_err(|e| e.to_string())
}

// ── Navigation helpers ──────────────────────────────────────

fn nav_left(app: &mut App) {
    let parent = app.mm.node(app.mm.active_node).parent;
    if parent != NodeId(0) || app.mm.active_node != app.mm.root_id {
        if parent != NodeId(usize::MAX) && parent != NodeId(0) {
            app.mm.active_node = parent;
        }
    }
}

fn nav_right(app: &mut App) {
    let children = app.mm.visible_children(app.mm.active_node, app.show_hidden);
    if children.is_empty() {
        return;
    }
    // auto-expand if collapsed
    if app.mm.node(app.mm.active_node).collapsed {
        ops::toggle_node(&mut app.mm);
    }
    let children = app.mm.visible_children(app.mm.active_node, app.show_hidden);
    if !children.is_empty() {
        app.mm.active_node = children[children.len() / 2];
    }
}

fn nav_down(app: &mut App) {
    let parent = app.mm.node(app.mm.active_node).parent;
    if parent == NodeId(usize::MAX) {
        return;
    }
    let siblings = app.mm.visible_children(parent, app.show_hidden);
    if let Some(pos) = siblings.iter().position(|&c| c == app.mm.active_node) {
        if pos + 1 < siblings.len() {
            app.mm.active_node = siblings[pos + 1];
        }
    }
}

fn nav_up(app: &mut App) {
    let parent = app.mm.node(app.mm.active_node).parent;
    if parent == NodeId(usize::MAX) {
        return;
    }
    let siblings = app.mm.visible_children(parent, app.show_hidden);
    if let Some(pos) = siblings.iter().position(|&c| c == app.mm.active_node) {
        if pos > 0 {
            app.mm.active_node = siblings[pos - 1];
        }
    }
}

fn go_to_top(app: &mut App) {
    let children = app.mm.visible_children(app.mm.root_id, app.show_hidden);
    if let Some(&first) = children.first() {
        app.mm.active_node = first;
    }
}

fn go_to_bottom(app: &mut App) {
    let children = app.mm.visible_children(app.mm.root_id, app.show_hidden);
    if let Some(&last) = children.last() {
        app.mm.active_node = last;
    }
}

// ── Save ─────────────────────────────────────────────────────

fn save(app: &mut App) {
    if let Some(ref filename) = app.filename {
        match persist_to_disk(app) {
            Ok(()) => {
                app.modified = false;
                app.message = Some(format!("Saved {}", filename));
            }
            Err(e) => {
                app.message = Some(format!("Error saving: {}", e));
            }
        }
    } else {
        app.message = Some("No filename. Use S to save as (not yet implemented).".into());
    }
}

#[cfg(test)]
mod ui_tests {
    use super::*;
    use crate::model::MindMap;

    #[test]
    fn insert_and_remove_chars_by_char_index() {
        let mut s = String::from("ab");
        insert_char_at(&mut s, 1, 'X');
        assert_eq!(s, "aXb");
        remove_char_at(&mut s, 1);
        assert_eq!(s, "ab");
    }

    #[test]
    fn sanitize_editor_text_flattens_whitespace() {
        assert_eq!(sanitize_editor_text("a\nb\tc\r"), "a b  c ");
    }

    #[test]
    fn render_status_line_handles_cursor_at_end() {
        let mut app = App::new(MindMap::new("root"), None);
        app.editor = Some(EditorState {
            node_id: app.mm.root_id,
            buffer: "hello".to_string(),
            cursor: 5,
            original_title: "hello".to_string(),
            is_new_node: false,
            original_modified: false,
        });

        let lines = render_status_lines(&app);
        assert!(!lines[0].spans.is_empty());
    }

    #[test]
    fn render_status_line_handles_confirm_state() {
        let mut app = App::new(MindMap::new("root"), None);
        app.confirm = Some(ConfirmState {
            action: ConfirmAction::ExportCurrent,
            message: "push current node to Linear? preview=create-main".to_string(),
        });

        let lines = render_status_lines(&app);
        assert!(!lines[0].spans.is_empty());
    }

    #[test]
    fn render_status_lines_show_dash_for_unexported_issue_details() {
        let app = App::new(MindMap::new("root"), None);
        let lines = render_status_lines(&app);
        let rendered = lines
            .into_iter()
            .flat_map(|l| l.spans.into_iter().map(|s| s.content.to_string()))
            .collect::<Vec<_>>()
            .join(" ");
        assert!(rendered.contains(" - "));
    }
}
