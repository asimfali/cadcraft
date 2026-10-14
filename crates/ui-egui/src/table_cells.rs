//! Table cells picked with the mouse, as in AutoCAD: a click inside a cell selects it, Shift+click
//! or a drag across cells selects a range, Esc lets go. The selected cells are highlighted, and
//! the right-click menu acts on them (alignment, borders, text rotation, a block, merging, rows
//! and columns) through the table commands. A click on the table's lines still selects the
//! table itself.

use cadcraft_doc::{EntityKind, Handle, Table};
use cadcraft_geom::{Bounds2, Vec2};
use egui::{Color32, Rect, Stroke};
use serde_json::{Value, json};

use crate::CadApp;
use crate::canvas::Xf;
use crate::menus::Entry;

/// Menu item ids of cell actions start with this ([`act`]).
pub const CELL: &str = "cell:";

/// Selected cells of one table: the cell the selection started at and the one it reaches.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CellSel {
    pub table: Handle,
    pub anchor: (usize, usize),
    pub end: (usize, usize),
}

impl CellSel {
    /// (first row, rows, first column, columns).
    pub fn range(&self) -> (usize, usize, usize, usize) {
        let (r0, r1) = (self.anchor.0.min(self.end.0), self.anchor.0.max(self.end.0));
        let (c0, c1) = (self.anchor.1.min(self.end.1), self.anchor.1.max(self.end.1));
        (r0, r1 - r0 + 1, c0, c1 - c0 + 1)
    }
}

/// The table at `h`, if it still is one.
fn table(app: &CadApp, h: Handle) -> Option<Table> {
    match &app.session.doc().ok()?.entity(h)?.kind {
        EntityKind::Table(t) => Some(t.clone()),
        _ => None,
    }
}

/// The table cell whose inside `p` is in, `tol` (drawing units) away from its lines: clicks on
/// the lines pick the table itself.
pub fn cell_inside(app: &CadApp, p: Vec2, tol: f64) -> Option<(Handle, usize, usize)> {
    let d = app.session.doc().ok()?;
    let space = d.space(&app.session.space())?;
    for e in space.iter() {
        let EntityKind::Table(t) = &e.kind else { continue };
        let Some((r, c)) = t.cell_at_point(p) else { continue };
        let inside = t.cell_rects(r, c).iter().any(|b| p.x >= b.min.x + tol && p.x <= b.max.x - tol && p.y >= b.min.y + tol && p.y <= b.max.y - tol);
        if inside {
            return Some((e.handle, r, c));
        }
    }
    None
}

/// A click at `p` with no command running: select the cell there (Shift extends a selection in
/// the same table). `false` when there is no cell there, and the selection of cells is dropped.
pub fn click(app: &mut CadApp, p: Vec2, tol: f64, shift: bool) -> bool {
    let Some((h, r, c)) = cell_inside(app, p, tol) else {
        app.canvas.cells = None;
        return false;
    };
    app.canvas.cells = match app.canvas.cells {
        Some(sel) if shift && sel.table == h => Some(CellSel { end: (r, c), ..sel }),
        _ => Some(CellSel { table: h, anchor: (r, c), end: (r, c) }),
    };
    app.session.set_selection(Vec::new());
    true
}

/// A drag over the cells reaches the cell at `p` (in the selection's table).
pub fn drag_to(app: &mut CadApp, p: Vec2) {
    let Some(sel) = app.canvas.cells else { return };
    let Some(t) = table(app, sel.table) else { return };
    if let Some(cell) = t.cell_at_point(p) {
        app.canvas.cells = Some(CellSel { end: cell, ..sel });
    }
}

/// Drop a selection whose table is gone or no longer has those cells, or when objects are
/// selected instead.
pub fn validate(app: &mut CadApp) {
    if !app.session.selection().is_empty() {
        app.canvas.cells = None;
    }
    if let Some(sel) = app.canvas.cells {
        let ok = table(app, sel.table).is_some_and(|t| {
            let (r0, rows, c0, cols) = sel.range();
            r0 + rows <= t.row_heights.len() && c0 + cols <= t.col_widths.len()
        });
        if !ok {
            app.canvas.cells = None;
        }
    }
}

/// Highlight the selected cells.
pub fn paint(app: &CadApp, painter: &egui::Painter, xf: &Xf) {
    let Some(sel) = app.canvas.cells else { return };
    let Some(t) = table(app, sel.table) else { return };
    let tk = crate::theme::Tokens::get();
    let stroke = Stroke::new(2.0, tk.window_stroke);
    let fill = Color32::from_rgba_unmultiplied(tk.window_stroke.r(), tk.window_stroke.g(), tk.window_stroke.b(), 40);
    let (r0, rows, c0, cols) = sel.range();
    for r in r0..r0 + rows {
        for c in c0..c0 + cols {
            for b in t.cell_rects(r, c) {
                let rect = Rect::from_two_pos(xf.to_screen(b.min), xf.to_screen(b.max));
                painter.rect_filled(rect, 0.0, fill);
            }
        }
    }
    // The outline of the whole range in each part.
    let mut outline = Bounds2::EMPTY;
    for r in r0..r0 + rows {
        for c in c0..c0 + cols {
            for b in t.cell_rects(r, c) {
                outline = outline.union(&b);
            }
        }
    }
    if !outline.is_empty() && !t.is_broken() {
        let rect = Rect::from_two_pos(xf.to_screen(outline.min), xf.to_screen(outline.max));
        painter.rect_stroke(rect, 0.0, stroke, egui::StrokeKind::Middle);
    }
}

fn item(label: &str, id: String) -> Entry {
    Entry::Item { label: label.into(), id, shortcut: None, enabled: true }
}

fn sub(label: &str, children: Vec<Entry>) -> Entry {
    Entry::Sub { label: label.into(), children }
}

/// The right-click menu's groups for the selected cells (`None` without a selection).
pub fn entries(app: &CadApp) -> Option<Vec<Vec<Entry>>> {
    let sel = app.canvas.cells?;
    let one = sel.anchor == sel.end;
    let align = [
        ("Top Left", 1),
        ("Top Center", 2),
        ("Top Right", 3),
        ("Middle Left", 4),
        ("Middle Center", 5),
        ("Middle Right", 6),
        ("Bottom Left", 7),
        ("Bottom Center", 8),
        ("Bottom Right", 9),
    ]
    .into_iter()
    .map(|(l, a)| item(l, format!("{CELL}align:{a}")))
    .collect();
    let rotation = [("Horizontal", 0), ("Vertical, Upwards", 90), ("Vertical, Downwards", 270)]
        .into_iter()
        .map(|(l, r)| item(l, format!("{CELL}rotate:{r}")))
        .collect();
    let weights = |edges: &str| -> Vec<Entry> {
        let mut v: Vec<Entry> =
            ["0.18", "0.25", "0.35", "0.5", "0.7", "1.0"].iter().map(|w| item(&format!("{w} mm"), format!("{CELL}borders:{edges}:{w}"))).collect();
        v.push(item("ByBlock", format!("{CELL}borders:{edges}:byblock")));
        v.push(item("Hidden", format!("{CELL}borders:{edges}:hidden")));
        v.push(item("As the Cell Style", format!("{CELL}borders:{edges}:clear")));
        v
    };
    let borders = [
        ("All Borders", "all"),
        ("Outside Borders", "outside"),
        ("Inside Borders", "inside"),
        ("Inside Horizontal", "insideH"),
        ("Inside Vertical", "insideV"),
        ("Top Border", "top"),
        ("Bottom Border", "bottom"),
        ("Left Border", "left"),
        ("Right Border", "right"),
    ]
    .into_iter()
    .map(|(l, e)| sub(l, weights(e)))
    .collect();
    let mut cell = vec![
        sub("Alignment", align),
        sub("Borders", borders),
        sub("Text Rotation", rotation),
        item("Remove Formatting", format!("{CELL}clearformat")),
    ];
    if one {
        cell.insert(0, item("Edit Text", format!("{CELL}edit")));
    }
    let content = vec![item("Insert Block...", format!("{CELL}block")), item("Delete Content", format!("{CELL}delete"))];
    let merge = vec![item("Merge Cells", format!("{CELL}merge")), item("Unmerge Cells", format!("{CELL}unmerge"))];
    let rows = vec![
        item("Insert Above", format!("{CELL}row:above")),
        item("Insert Below", format!("{CELL}row:below")),
        item("Delete Rows", format!("{CELL}row:delete")),
    ];
    let cols = vec![
        item("Insert Left", format!("{CELL}col:left")),
        item("Insert Right", format!("{CELL}col:right")),
        item("Delete Columns", format!("{CELL}col:delete")),
    ];
    Some(vec![cell, content, merge, vec![sub("Rows", rows), sub("Columns", cols)]])
}

/// Run cell action `action` (an item id without [`CELL`]) on the selected cells.
pub fn act(app: &mut CadApp, action: &str) {
    let Some(sel) = app.canvas.cells else { return };
    let (r0, rows, c0, cols) = sel.range();
    let h = sel.table.hex();
    let range = json!({ "handle": h, "row": r0, "col": c0, "scope": "range", "rows": rows, "cols": cols });
    let with = |extra: Value| {
        let mut q = range.clone();
        if let (Some(o), Some(e)) = (q.as_object_mut(), extra.as_object()) {
            o.extend(e.clone());
        }
        q
    };
    // The first cell's centre, for the commands that pick a cell.
    let centre = table(app, sel.table).and_then(|t| t.cell_rects(r0, c0).first().map(|b| (b.min + b.max) * 0.5));
    let parts: Vec<&str> = action.split(':').collect();
    let result: Result<Value, String> = match parts.as_slice() {
        ["align", a] => app.run("tableformat", with(json!({ "align": a.parse::<u8>().unwrap_or(5) }))),
        ["rotate", r] => app.run("tableformat", with(json!({ "rotation": r.parse::<u16>().unwrap_or(0) }))),
        ["clearformat"] => {
            let _ = app.run("tableborders", with(json!({ "clear": true })));
            app.run("tableformat", with(json!({ "clear": true })))
        }
        ["borders", edges, w] => {
            let extra = match *w {
                "clear" => json!({ "edges": edges, "clear": true }),
                "hidden" => json!({ "edges": edges, "visible": false }),
                "byblock" => json!({ "edges": edges, "lineweight": "ByBlock" }),
                mm => json!({ "edges": edges, "lineweight": mm.parse::<f64>().unwrap_or(0.25) }),
            };
            app.run("tableborders", with(extra))
        }
        ["edit"] => {
            start_at(app, "tabledit", centre);
            Ok(Value::Null)
        }
        ["block"] => {
            start_at(app, "tinsert", centre);
            Ok(Value::Null)
        }
        ["delete"] => {
            let mut last = Ok(Value::Null);
            for r in r0..r0 + rows {
                for c in c0..c0 + cols {
                    let _ = app.run("tinsert", json!({ "handle": h, "row": r, "col": c, "block": "" }));
                    last = app.run("table.set", json!({ "handle": h, "row": r, "col": c, "text": "" }));
                }
            }
            last
        }
        ["merge"] => app.run("table.merge", json!({ "handle": h, "row": r0, "col": c0, "rows": rows, "cols": cols })),
        ["unmerge"] => app.run("table.unmerge", json!({ "handle": h, "row": r0, "col": c0 })),
        ["row", "above"] => app.run("table.insertrow", json!({ "handle": h, "row": r0 })),
        ["row", "below"] => app.run("table.insertrow", json!({ "handle": h, "row": r0 + rows })),
        ["row", "delete"] => {
            app.canvas.cells = None;
            let mut last = Ok(Value::Null);
            for r in (r0..r0 + rows).rev() {
                last = app.run("table.deleterow", json!({ "handle": h, "row": r }));
            }
            last
        }
        ["col", "left"] => app.run("table.insertcol", json!({ "handle": h, "col": c0 })),
        ["col", "right"] => app.run("table.insertcol", json!({ "handle": h, "col": c0 + cols })),
        ["col", "delete"] => {
            app.canvas.cells = None;
            let mut last = Ok(Value::Null);
            for c in (c0..c0 + cols).rev() {
                last = app.run("table.deletecol", json!({ "handle": h, "col": c }));
            }
            last
        }
        _ => Ok(Value::Null),
    };
    if let Err(e) = result {
        app.session.echo(e);
    }
    validate(app);
}

/// Start command `id` and give it the point `at` (the cell it asks for first).
fn start_at(app: &mut CadApp, id: &str, at: Option<Vec2>) {
    app.start(id);
    if let Some(p) = at
        && let Err(e) = app.session.input(cadcraft_engine::Input::Point(p))
    {
        app.session.echo(e.to_string());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Services;
    use cadcraft_engine::Session;

    /// An app with a 3×3 table of 10 × 4 cells at the origin (no title or header).
    fn app() -> (CadApp, Handle) {
        let mut a = CadApp::new(Session::new(), Services::default());
        let r = a.run("table", json!({ "at": [0, 0], "rows": 3, "cols": 3, "rowHeight": 4.0, "colWidth": 10.0 })).unwrap_or_default();
        let h = r["handle"].as_str().and_then(Handle::parse_hex).unwrap_or(Handle(0));
        (a, h)
    }

    fn cells(a: &CadApp, h: Handle) -> Vec<Vec<cadcraft_doc::TableCell>> {
        table(a, h).map(|t| t.cells).unwrap_or_default()
    }

    #[test]
    fn clicks_pick_cells_and_shift_extends_the_range() {
        let (mut a, h) = app();
        // Inside cell (0, 0); on a line (x 10) there is no cell, and the selection goes.
        assert!(click(&mut a, Vec2::new(5.0, -2.0), 0.1, false));
        assert_eq!(a.canvas.cells.map(|s| s.range()), Some((0, 1, 0, 1)));
        assert!(click(&mut a, Vec2::new(25.0, -10.0), 0.1, true));
        assert_eq!(a.canvas.cells.map(|s| s.range()), Some((0, 3, 0, 3)));
        assert_eq!(a.canvas.cells.map(|s| s.table), Some(h));
        assert!(!click(&mut a, Vec2::new(10.0, -2.0), 0.1, false));
        assert!(a.canvas.cells.is_none());
        // A drag reaches the cell under the pointer.
        assert!(click(&mut a, Vec2::new(15.0, -6.0), 0.1, false));
        drag_to(&mut a, Vec2::new(25.0, -2.0));
        assert_eq!(a.canvas.cells.map(|s| s.range()), Some((0, 2, 1, 2)));
        // Selecting objects lets the cells go.
        a.session.set_selection(vec![h]);
        validate(&mut a);
        assert!(a.canvas.cells.is_none());
    }

    #[test]
    fn the_menu_formats_and_borders_the_selected_range() {
        let (mut a, h) = app();
        assert!(entries(&a).is_none(), "no menu without cells");
        click(&mut a, Vec2::new(5.0, -6.0), 0.1, false);
        click(&mut a, Vec2::new(15.0, -10.0), 0.1, true);
        assert!(entries(&a).is_some());
        act(&mut a, "align:5");
        act(&mut a, "rotate:90");
        act(&mut a, "borders:outside:0.7");
        let c = cells(&a, h);
        let l = c[1][0].look.unwrap_or_default();
        assert_eq!((l.align, l.rotation), (Some(5), Some(90)));
        assert!(c[0][0].look.is_none() && c[1][2].look.is_none(), "outside the range");
        let top = c[1][0].edges.and_then(|e| e.top).map(|b| b.lineweight);
        assert_eq!(top, Some(cadcraft_doc::Lineweight::Mm100(70)));
        assert!(c[1][0].edges.and_then(|e| e.bottom).is_none(), "an inside edge");
        act(&mut a, "clearformat");
        let c = cells(&a, h);
        assert!(c.iter().flatten().all(|x| x.look.is_none() && x.edges.is_none()));
    }

    #[test]
    fn the_menu_merges_inserts_and_deletes() {
        let (mut a, h) = app();
        a.run("table.set", json!({ "handle": h.hex(), "row": 0, "col": 0, "text": "x" })).unwrap_or_default();
        click(&mut a, Vec2::new(5.0, -2.0), 0.1, false);
        act(&mut a, "delete");
        assert!(cells(&a, h)[0][0].text.is_empty());
        click(&mut a, Vec2::new(15.0, -2.0), 0.1, true);
        act(&mut a, "merge");
        assert_eq!(cells(&a, h)[0][0].merged, Some((1, 2)));
        act(&mut a, "unmerge");
        assert_eq!(cells(&a, h)[0][0].merged, None);
        act(&mut a, "row:below");
        act(&mut a, "col:left");
        let t = table(&a, h).unwrap_or_default();
        assert_eq!((t.row_heights.len(), t.col_widths.len()), (4, 4));
        // Deleting the rows of the selection (row 0) lets the cells go.
        act(&mut a, "row:delete");
        assert_eq!(table(&a, h).map(|t| t.row_heights.len()), Some(3));
        assert!(a.canvas.cells.is_none());
        // Unknown actions do nothing.
        act(&mut a, "nonsense:1");
    }
}
