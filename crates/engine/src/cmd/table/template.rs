//! TABLETEMPLATE: a table laid out by a table template — a JSON file that says how a kind of
//! table looks: its columns (widths, alignment, header text and rotation), a header of one or
//! more rows with merged cells, a title, text style and heights, row heights and how rows grow,
//! and the lineweight of each kind of line (the outline, the lines under and in the header, the
//! lines between columns and between data rows). It holds no data, so one template serves a
//! table drawn by hand and a report (`"template"` in the report definition).
//!
//! ```json
//! {
//!   "name": "GOST 2.106 form 1",
//!   "textStyle": {"name": "GOST", "font": "GOST type A"},
//!   "textHeight": 3.5, "margin": 1, "rowHeight": 8, "rowGrow": 8,
//!   "lines": {"outline": 0.7, "header": 0.7, "vertical": 0.7, "horizontal": 0.25},
//!   "header": {"height": 15, "textHeight": 2.5},
//!   "columns": [
//!     {"header": "Format", "width": 6, "align": "MC", "headerRotation": 90},
//!     {"header": "Name", "width": 63, "align": "ML"}
//!   ]
//! }
//! ```
//!
//! A header of several rows: `"header": {"rows": [["A", "B", "<"], ["^", "B1", "B2"]],
//! "heights": [8, 7]}`, where `<` joins a cell to the one on its left and `^` to the one above.
//!
//! Applying a template makes (or replaces) the table style named after it, whose Title, Header
//! and Data cell styles carry the text and the lines; so rows added later — by hand or by a
//! report update — get the same lines. Columns' alignment and rotation go on the cells.

use cadcraft_doc::{
    CellBorder, CellBorders, CellLook, CellStyle, Drawing, Lineweight, RowGrow, Table, TableCell, TableStyle, TableTemplateLink, TextStyle,
};
use cadcraft_geom::Vec2;
use serde_json::{Value, json};

use super::breaks::read_link;
use super::report::{file_kind, file_picker, locate};
use super::{MAX_COLS, MAX_ROWS, anchors, edit, fit_rows, head_rows, normalize, row_grow_param, set_anchors};
use crate::cmd::{CommandSpec, bad, point_param, str_param};
use crate::{Accept, Input, Interactive, Prompt, Result, Session, Step};
use cadcraft_doc::{EntityKind, Handle};

/// Most header rows a template has.
const MAX_HEADER_ROWS: usize = 20;
/// Largest size a template gives (drawing units).
const MAX_SIZE: f64 = 1e6;

const PARAMS: &str = "{file?: table template (.json), template?: {…} (the template itself), handle? (lay out this table by it; its data rows stay), \
     at?: [x,y] (a new table), rows? (its data rows, default 1)} → {handle}. \
     Template: {name, textStyle?: name | {name, font}, textHeight, margin?, marginV?, rowHeight?, rowGrow?: free|fixed|step, \
     lines?: {outline, header, vertical, horizontal} (mm), title?: text | {text?, height?, textHeight?}, \
     header?: {rows?: [[text | \"<\" | \"^\"]], heights?, height?, textHeight?}, \
     columns: [{header?, width, align?: TL…BR, rotation?, margin?, textHeight?, headerAlign?, headerRotation?}], \
     breaks?: {height (tallest part), heights?: [first parts], spacing?, direction?: right|left|down, repeatTop?}}";

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec::new("tabletemplate", "Table from Template...", run)
            .menu(&["Draw", "Table from Template..."])
            .alias(&["ttpl"])
            .params(PARAMS)
            .interactive(|s| Ok(Box::new(TemplateM::new(s)))),
    ]
}

/// A column of a template.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Column {
    pub header: String,
    pub width: f64,
    pub align: Option<u8>,
    pub rotation: Option<u16>,
    pub margin: Option<f64>,
    pub text_height: Option<f64>,
    pub header_align: Option<u8>,
    pub header_rotation: Option<u16>,
}

/// Lineweights (mm) of the kinds of lines; `None`: as the table's layer draws them.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Lines {
    pub outline: Option<f64>,
    pub header: Option<f64>,
    pub vertical: Option<f64>,
    pub horizontal: Option<f64>,
}

/// A table template.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Template {
    pub name: String,
    /// Text style name and, to make it when the drawing lacks it, its font.
    pub text_style: Option<(String, Option<String>)>,
    pub text_height: f64,
    pub margin_h: f64,
    pub margin_v: f64,
    pub row_height: Option<f64>,
    pub row_grow: Option<RowGrow>,
    pub lines: Lines,
    /// The title row: text, height, text height.
    pub title: Option<(String, Option<f64>, Option<f64>)>,
    /// Header rows (texts, `<` and `^` join cells), their heights, their text height.
    pub header: Vec<Vec<String>>,
    pub header_heights: Vec<f64>,
    pub header_text_height: Option<f64>,
    pub columns: Vec<Column>,
    /// Parts the table is broken into (one per sheet): `None` keeps the table's own.
    pub breaks: Option<cadcraft_doc::TableBreak>,
}

type Res<T> = std::result::Result<T, String>;

fn text(v: &Value) -> Option<String> {
    match v {
        Value::String(s) => Some(s.chars().take(4096).collect()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

fn size(o: &serde_json::Map<String, Value>, key: &str) -> Res<Option<f64>> {
    match o.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => v
            .as_f64()
            .filter(|x| x.is_finite() && *x > 0.0 && *x <= MAX_SIZE)
            .map(Some)
            .ok_or_else(|| format!("`{key}` must be a number more than 0")),
    }
}

fn margin(o: &serde_json::Map<String, Value>, key: &str) -> Res<Option<f64>> {
    match o.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => v
            .as_f64()
            .filter(|x| x.is_finite() && *x >= 0.0 && *x <= MAX_SIZE)
            .map(Some)
            .ok_or_else(|| format!("`{key}` must be a number, 0 or more")),
    }
}

/// An alignment: `TL` … `BR` or 1 … 9 (MTEXT attachment).
pub fn align(v: &Value) -> Option<u8> {
    const NAMES: [&str; 9] = ["TL", "TC", "TR", "ML", "MC", "MR", "BL", "BC", "BR"];
    if let Some(n) = v.as_u64() {
        return u8::try_from(n).ok().filter(|n| (1..=9).contains(n));
    }
    let s = v.as_str()?.trim().to_ascii_uppercase();
    NAMES.iter().position(|n| *n == s).and_then(|i| u8::try_from(i + 1).ok())
}

fn align_key(o: &serde_json::Map<String, Value>, key: &str) -> Res<Option<u8>> {
    match o.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => align(v).map(Some).ok_or_else(|| format!("`{key}` must be TL, TC, TR, ML, MC, MR, BL, BC or BR")),
    }
}

fn rotation_key(o: &serde_json::Map<String, Value>, key: &str) -> Res<Option<u16>> {
    match o.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => v
            .as_u64()
            .and_then(|r| u16::try_from(r).ok())
            .filter(|r| [0, 90, 180, 270].contains(r))
            .map(Some)
            .ok_or_else(|| format!("`{key}` must be 0, 90, 180 or 270")),
    }
}

fn column(v: &Value) -> Res<Column> {
    let o = v.as_object().ok_or("a column must be {header, width, …}")?;
    Ok(Column {
        header: o.get("header").and_then(text).unwrap_or_default(),
        width: size(o, "width")?.ok_or("every column needs a `width`")?,
        align: align_key(o, "align")?,
        rotation: rotation_key(o, "rotation")?,
        margin: margin(o, "margin")?,
        text_height: size(o, "textHeight")?,
        header_align: align_key(o, "headerAlign")?,
        header_rotation: rotation_key(o, "headerRotation")?,
    })
}

/// Read a table template.
pub fn parse(v: &Value) -> Res<Template> {
    let o = v.as_object().ok_or("a table template must be a JSON object")?;
    let columns = o.get("columns").and_then(Value::as_array).ok_or("a table template needs `columns`: [{header, width}]")?;
    if columns.is_empty() || columns.len() > MAX_COLS {
        return Err(format!("a table template has 1 to {MAX_COLS} columns"));
    }
    let columns = columns.iter().map(column).collect::<Res<Vec<_>>>()?;
    let text_style = match o.get("textStyle") {
        None | Some(Value::Null) => None,
        Some(Value::Object(s)) => {
            let name = s.get("name").and_then(text).filter(|n| !n.trim().is_empty()).ok_or("`textStyle` needs a `name`")?;
            Some((name.trim().to_string(), s.get("font").and_then(text).filter(|f| !f.trim().is_empty())))
        }
        Some(v) => Some((text(v).ok_or("`textStyle` must be a name or {name, font}")?.trim().to_string(), None)),
    };
    let lines = match o.get("lines") {
        None | Some(Value::Null) => Lines::default(),
        Some(Value::Object(l)) => {
            let lw = |k: &str| -> Res<Option<f64>> {
                match l.get(k) {
                    None | Some(Value::Null) => Ok(None),
                    Some(v) => v
                        .as_f64()
                        .filter(|x| x.is_finite() && (0.0..=2.11).contains(x))
                        .map(Some)
                        .ok_or_else(|| format!("`lines.{k}` must be a lineweight in mm (0 … 2.11)")),
                }
            };
            Lines { outline: lw("outline")?, header: lw("header")?, vertical: lw("vertical")?, horizontal: lw("horizontal")? }
        }
        Some(_) => return Err("`lines` must be {outline, header, vertical, horizontal}".into()),
    };
    let title = match o.get("title") {
        None | Some(Value::Null) | Some(Value::Bool(false)) => None,
        Some(Value::Bool(true)) => Some((String::new(), None, None)),
        Some(Value::Object(t)) => Some((t.get("text").and_then(text).unwrap_or_default(), size(t, "height")?, size(t, "textHeight")?)),
        Some(v) => Some((text(v).ok_or("`title` must be a text or {text, height, textHeight}")?, None, None)),
    };
    let mut header: Vec<Vec<String>> = Vec::new();
    let mut header_heights = Vec::new();
    let mut header_text_height = None;
    match o.get("header") {
        None | Some(Value::Null) => {}
        Some(Value::Bool(false)) => header.push(Vec::new()),
        Some(Value::Object(h)) => {
            if let Some(rows) = h.get("rows").filter(|v| !v.is_null()) {
                let rows = rows.as_array().ok_or("`header.rows` must be a list of rows")?;
                if rows.len() > MAX_HEADER_ROWS {
                    return Err(format!("more than {MAX_HEADER_ROWS} header rows"));
                }
                for r in rows {
                    let r = r.as_array().ok_or("a header row must be a list of texts")?;
                    header.push(r.iter().take(MAX_COLS).map(|c| text(c).unwrap_or_default()).collect());
                }
            }
            if let Some(hs) = h.get("heights").and_then(Value::as_array) {
                header_heights = hs
                    .iter()
                    .take(MAX_HEADER_ROWS)
                    .map(|x| x.as_f64().filter(|v| v.is_finite() && *v > 0.0 && *v <= MAX_SIZE).ok_or("`header.heights` must be numbers more than 0"))
                    .collect::<std::result::Result<Vec<_>, _>>()?;
            } else if let Some(hh) = size(h, "height")? {
                header_heights = vec![hh];
            }
            header_text_height = size(h, "textHeight")?;
        }
        Some(_) => return Err("`header` must be {rows?, heights?, height?, textHeight?}".into()),
    }
    // No rows given: one row of the columns' headers (none when no column has one, or `false`).
    if header.len() == 1 && header.first().is_some_and(Vec::is_empty) {
        header.clear();
    } else if header.is_empty() && columns.iter().any(|c| !c.header.is_empty()) {
        header.push(columns.iter().map(|c| c.header.clone()).collect());
    }
    let breaks = match o.get("breaks") {
        None | Some(Value::Null) => None,
        Some(Value::Object(b)) => {
            let direction = match b.get("direction").and_then(Value::as_str).map(str::to_ascii_lowercase).as_deref() {
                None | Some("right") => cadcraft_doc::BreakDirection::Right,
                Some("left") => cadcraft_doc::BreakDirection::Left,
                Some("down") => cadcraft_doc::BreakDirection::Down,
                Some(other) => return Err(format!("`breaks.direction` must be right, left or down, not {other}")),
            };
            Some(cadcraft_doc::TableBreak {
                enabled: true,
                direction,
                repeat_top: b.get("repeatTop").and_then(Value::as_bool).unwrap_or(true),
                repeat_bottom: false,
                height: size(b, "height")?.ok_or("`breaks` needs `height`: the tallest part (a sheet's room for the table)")?,
                // The first parts' own heights (a first sheet with a taller title block).
                heights: match b.get("heights") {
                    None | Some(Value::Null) => Vec::new(),
                    Some(Value::Array(a)) => a
                        .iter()
                        .take(cadcraft_doc::MAX_TABLE_PARTS)
                        .map(|x| {
                            x.as_f64().filter(|v| v.is_finite() && *v > 0.0 && *v <= MAX_SIZE).ok_or("`breaks.heights` must be numbers more than 0")
                        })
                        .collect::<std::result::Result<Vec<_>, _>>()?,
                    Some(_) => return Err("`breaks.heights` must be a list of heights".into()),
                },
                spacing: margin(b, "spacing")?.unwrap_or(10.0),
                offsets: Vec::new(),
            })
        }
        Some(_) => return Err("`breaks` must be {height, spacing?, direction?, repeatTop?}".into()),
    };
    let text_height = size(o, "textHeight")?.unwrap_or(2.5);
    let margin_h = margin(o, "margin")?.unwrap_or(text_height * 0.3);
    Ok(Template {
        name: o
            .get("name")
            .and_then(text)
            .map(|n| n.trim().chars().take(255).collect::<String>())
            .filter(|n| !n.is_empty())
            .unwrap_or_else(|| "Template".into()),
        text_style,
        text_height,
        margin_h,
        margin_v: margin(o, "marginV")?.unwrap_or(margin_h),
        row_height: size(o, "rowHeight")?,
        row_grow: row_grow_param(&Value::Object(o.clone()))?,
        lines,
        title,
        header,
        header_heights,
        header_text_height,
        columns,
        breaks,
    })
}

impl Template {
    /// Rows above the data: the title and the header rows.
    pub fn head_rows(&self) -> usize {
        usize::from(self.title.is_some()) + self.header.len()
    }
}

fn border(mm: Option<f64>) -> CellBorder {
    match mm {
        Some(v) => CellBorder { lineweight: Lineweight::Mm100((v * 100.0).round().clamp(0.0, 211.0) as u16), ..CellBorder::default() },
        None => CellBorder::default(),
    }
}

/// The table style a template makes: Title, Header and Data cell styles with its text and lines.
fn table_style(tpl: &Template, text_style: &str) -> TableStyle {
    let l = tpl.lines;
    let (outline, head, vert, horiz) = (border(l.outline), border(l.header.or(l.outline)), border(l.vertical), border(l.horizontal));
    let base = CellStyle {
        text_style: text_style.to_string(),
        text_height: tpl.text_height,
        margin_h: tpl.margin_h,
        margin_v: tpl.margin_v,
        ..CellStyle::default()
    };
    let mut st = TableStyle {
        name: tpl.name.clone(),
        text_height: tpl.text_height,
        margin: tpl.margin_h,
        title: tpl.title.is_some(),
        header: !tpl.header.is_empty(),
        cell_styles: Vec::new(),
    };
    let title_h = tpl.title.as_ref().and_then(|t| t.2).unwrap_or(tpl.text_height * 1.4);
    st.set_cell_style(CellStyle {
        name: CellStyle::TITLE.into(),
        text_height: title_h,
        align: 5,
        merge: true,
        borders: CellBorders { top: outline, inside_h: head, bottom: head, left: outline, inside_v: vert, right: outline },
        ..base.clone()
    });
    st.set_cell_style(CellStyle {
        name: CellStyle::HEADER.into(),
        text_height: tpl.header_text_height.unwrap_or(tpl.text_height),
        align: 5,
        borders: CellBorders { top: outline, inside_h: head, bottom: head, left: outline, inside_v: vert, right: outline },
        ..base.clone()
    });
    st.set_cell_style(CellStyle {
        name: CellStyle::DATA.into(),
        align: 4,
        borders: CellBorders { top: head, inside_h: horiz, bottom: outline, left: outline, inside_v: vert, right: outline },
        ..base
    });
    st
}

/// Make the template's table style (replacing one of its name) and text style in `d`.
pub fn make_styles(d: &mut Drawing, tpl: &Template) {
    let text_style = match &tpl.text_style {
        Some((name, font)) => match (d.text_style(name).map(|s| s.name.clone()), font) {
            (Some(n), _) => n,
            (None, Some(font)) => {
                d.text_styles.push(TextStyle { name: name.clone(), font: font.clone(), ..TextStyle::default() });
                name.clone()
            }
            (None, None) => "Standard".into(),
        },
        None => "Standard".into(),
    };
    let st = table_style(tpl, &text_style);
    match d.table_styles.iter_mut().find(|s| s.name.eq_ignore_ascii_case(&tpl.name)) {
        Some(s) => *s = st,
        None => d.table_styles.push(st),
    }
}

/// A look with only what the column gives, `None` when it gives nothing.
fn look(align: Option<u8>, rotation: Option<u16>, margin: Option<f64>) -> Option<CellLook> {
    let l = CellLook { align, margin_h: margin, margin_v: None, rotation };
    (!l.is_empty()).then_some(l)
}

/// Give the data cells from row `start` on the columns' alignment, rotation, margin and text
/// height, where they have none of their own.
pub fn data_looks(t: &mut Table, tpl: &Template, start: usize) {
    for row in t.cells.iter_mut().skip(start) {
        for (cell, col) in row.iter_mut().zip(&tpl.columns) {
            if cell.look.is_none() {
                cell.look = look(col.align, col.rotation, col.margin);
            }
            if cell.text_height.is_none() {
                cell.text_height = col.text_height;
            }
        }
    }
}

/// The header rows' cells and merges (row, col, rows, cols), `<` and `^` joining cells.
fn header_cells(tpl: &Template, cols: usize, first: usize) -> (Vec<Vec<TableCell>>, Vec<(usize, usize, usize, usize)>) {
    let at = |r: usize, c: usize| tpl.header.get(r).and_then(|row| row.get(c)).map(|s| s.trim()).unwrap_or("");
    let joined = |r: usize, c: usize| matches!(at(r, c), "<" | "^");
    let mut rows = Vec::new();
    let mut merges = Vec::new();
    for r in 0..tpl.header.len() {
        let mut row = Vec::new();
        for c in 0..cols {
            let col = tpl.columns.get(c);
            let text = if joined(r, c) { String::new() } else { at(r, c).to_string() };
            row.push(TableCell {
                text,
                look: look(Some(col.and_then(|x| x.header_align).unwrap_or(5)), col.and_then(|x| x.header_rotation), None),
                ..TableCell::default()
            });
            if joined(r, c) {
                continue;
            }
            let cs = 1 + (c + 1..cols).take_while(|&k| at(r, k) == "<").count();
            let rs = 1 + (r + 1..tpl.header.len()).take_while(|&k| at(k, c) == "^").count();
            if cs > 1 || rs > 1 {
                merges.push((first + r, c, rs, cs));
            }
        }
        rows.push(row);
    }
    (rows, merges)
}

/// Lay `t` out by `tpl`: its rows above the data become the template's title and header, its
/// data rows (texts, blocks, formulas) stay, in the template's columns.
pub fn apply(t: &mut Table, tpl: &Template, link: Option<TableTemplateLink>) {
    let cols = tpl.columns.len().clamp(1, MAX_COLS);
    let old_head = head_rows(t);
    let old_title = if t.title { t.cells.first().and_then(|r| r.first()).map(|c| c.text.clone()) } else { None };
    let head = tpl.head_rows();
    // Data rows and their merges, moved below the new head.
    let data_merges: Vec<_> = anchors(t)
        .into_iter()
        .filter(|(r, ..)| *r >= old_head)
        .filter(|&(_, c, ..)| c < cols)
        .map(|(r, c, rs, cs)| (r - old_head + head, c, rs, cs.min(cols - c)))
        .collect();
    let mut data: Vec<Vec<TableCell>> = t.cells.drain(old_head..).collect();
    let mut data_h: Vec<f64> = t.row_heights.get(old_head..).map(<[f64]>::to_vec).unwrap_or_default();
    let data_min: Vec<f64> = t.row_min.get(old_head..).map(<[f64]>::to_vec).unwrap_or_default();
    for (i, h) in data_h.iter_mut().enumerate() {
        *h = tpl.row_height.or(data_min.get(i).copied()).unwrap_or(*h);
    }
    for row in &mut data {
        row.resize_with(cols, TableCell::default);
        for cell in row.iter_mut() {
            cell.edges = None;
            cell.style.clear();
        }
    }
    // The head.
    let mut cells = Vec::new();
    let mut heights = Vec::new();
    let mut styles = Vec::new();
    let mut merges = Vec::new();
    if let Some((text, h, th)) = &tpl.title {
        let text = if text.is_empty() { old_title.unwrap_or_default() } else { text.clone() };
        let mut row = vec![TableCell::default(); cols];
        if let Some(c) = row.first_mut() {
            c.text = text;
        }
        cells.push(row);
        heights.push(h.unwrap_or(th.unwrap_or(tpl.text_height * 1.4) * 2.0 + 2.0 * tpl.margin_v));
        styles.push(CellStyle::TITLE.to_string());
        if cols > 1 {
            merges.push((0, 0, 1, cols));
        }
    }
    let (hrows, hmerges) = header_cells(tpl, cols, cells.len());
    let default_h = tpl.row_height.unwrap_or(tpl.text_height * 5.0 / 3.0 + 2.0 * tpl.margin_v);
    for (i, row) in hrows.into_iter().enumerate() {
        cells.push(row);
        heights.push(tpl.header_heights.get(i).or(tpl.header_heights.last()).copied().unwrap_or(default_h));
        styles.push(CellStyle::HEADER.to_string());
    }
    merges.extend(hmerges);
    merges.extend(data_merges);
    // A table keeps a row.
    if data.is_empty() && cells.is_empty() {
        data.push(vec![TableCell::default(); cols]);
        data_h.push(default_h);
    }
    let mins: Vec<f64> = heights.iter().copied().chain(data_h.iter().copied()).collect();
    cells.extend(data);
    t.cells = cells;
    t.row_heights = mins.clone();
    t.row_min = mins;
    t.row_styles = styles;
    t.col_widths = tpl.columns.iter().map(|c| c.width).collect();
    t.title = tpl.title.is_some();
    t.header = !tpl.header.is_empty();
    t.style = tpl.name.clone();
    t.text_height = tpl.text_height;
    if let Some(g) = tpl.row_grow {
        t.row_grow = g;
    }
    if let Some(b) = &tpl.breaks {
        t.breaks = Some(b.clone());
    }
    set_anchors(t, &merges);
    data_looks(t, tpl, head);
    if let Some(r) = &mut t.report {
        r.start_row = head;
    }
    if link.is_some() {
        t.template = link;
    }
}

/// A new table laid out by `tpl` at `at`, with `rows` empty data rows.
pub fn new_table(tpl: &Template, at: Vec2, rows: usize, link: Option<TableTemplateLink>) -> Table {
    let cols = tpl.columns.len().clamp(1, MAX_COLS);
    let h = tpl.row_height.unwrap_or(tpl.text_height * 5.0 / 3.0 + 2.0 * tpl.margin_v);
    let mut t = Table {
        insert: cadcraft_geom::Vec3::new(at.x, at.y, 0.0),
        col_widths: vec![1.0; cols],
        row_heights: vec![h; rows],
        cells: vec![vec![TableCell::default(); cols]; rows],
        ..Table::default()
    };
    apply(&mut t, tpl, link);
    t
}

/// A loaded template, the link a table keeps to it.
pub struct Loaded {
    pub template: Template,
    pub link: TableTemplateLink,
}

/// The template `p` gives: `template` (an object or JSON text) or `file` (relative to `base`).
pub fn load(s: &Session, id: &str, p: &Value, base: Option<&std::path::Path>) -> Result<Loaded> {
    let file = str_param(p, "file").map(str::trim).filter(|f| !f.is_empty()).map(|f| locate(s, base, f));
    let value: Value = match p.get("template") {
        Some(Value::String(t)) => serde_json::from_str(t).map_err(|e| bad(id, format!("template: not JSON ({e})")))?,
        Some(v @ Value::Object(_)) => v.clone(),
        Some(Value::Null) | None => match &file {
            Some(f) => serde_json::from_str(&read_link(id, f)?).map_err(|e| bad(id, format!("{f}: not JSON ({e})")))?,
            None => return Err(bad(id, "a table template is required: `file` (.json) or `template`")),
        },
        Some(_) => return Err(bad(id, "`template` must be a JSON object")),
    };
    if file_kind(&value) == Some("report") {
        let name = file.as_deref().unwrap_or("the template");
        return Err(bad(id, format!("{name} is a report definition, not a table template: choose a template (it has `columns` with widths)")));
    }
    let template = parse(&value).map_err(|m| bad(id, m))?;
    let definition = serde_json::to_string(&value).unwrap_or_default();
    let definition = if definition.len() > cadcraft_doc::MAX_REPORT_BYTES { String::new() } else { definition };
    Ok(Loaded { template, link: TableTemplateLink { file: file.unwrap_or_default(), definition } })
}

/// Lay table `h` out by `l`.
fn apply_to(s: &mut Session, id: &str, h: Handle, l: &Loaded) -> Result<Value> {
    make_styles(s.doc_mut()?, &l.template);
    edit(s, &json!({ "handle": h.hex() }), id, |t| {
        apply(t, &l.template, Some(l.link.clone()));
        Ok(())
    })
}

/// Insert a new table laid out by `l`.
fn insert(s: &mut Session, l: &Loaded, at: Vec2, rows: usize) -> Result<Handle> {
    make_styles(s.doc_mut()?, &l.template);
    let mut t = new_table(&l.template, at, rows, Some(l.link.clone()));
    normalize(&mut t);
    fit_rows(s.doc()?, &mut t);
    s.add_entity(EntityKind::Table(t))
}

fn run(s: &mut Session, p: &Value) -> Result<Value> {
    let id = "tabletemplate";
    let l = load(s, id, p, None)?;
    if let Some(h) = p.get("handle").filter(|v| !v.is_null()) {
        let h =
            h.as_str().and_then(Handle::parse_hex).or_else(|| h.as_u64().map(Handle)).ok_or_else(|| bad(id, "`handle` must be a table's handle"))?;
        if !matches!(s.doc()?.entity(h).map(|e| &e.kind), Some(EntityKind::Table(_))) {
            return Err(bad(id, "object is not a table"));
        }
        return apply_to(s, id, h, &l);
    }
    let at = point_param(p, "at").ok_or_else(|| bad(id, "`at` ([x, y]) or `handle` (a table) is required"))?;
    let rows = match p.get("rows") {
        None | Some(Value::Null) => 1,
        Some(v) => v
            .as_u64()
            .and_then(|n| usize::try_from(n).ok())
            .filter(|n| *n < MAX_ROWS - MAX_HEADER_ROWS)
            .ok_or_else(|| bad(id, format!("`rows` must be 0 … {}", MAX_ROWS - MAX_HEADER_ROWS - 1)))?,
    };
    let h = insert(s, &l, at, rows)?;
    Ok(json!({ "handle": h.hex() }))
}

// ---------------- command line ----------------

#[derive(Clone, Copy, Debug, Default, PartialEq)]
enum Stage {
    #[default]
    File,
    Rows,
    Point,
}

/// TABLETEMPLATE at the command line: the template file, then the number of data rows and the
/// point; with a table selected, that table is laid out by the template.
struct TemplateM {
    stage: Stage,
    last: String,
    target: Option<Handle>,
    loaded: Option<Loaded>,
    rows: usize,
}

const DEFAULT_ROWS: usize = 10;

impl TemplateM {
    fn new(s: &Session) -> Self {
        TemplateM { stage: Stage::File, last: s.last_used.template_file.clone(), target: None, loaded: None, rows: DEFAULT_ROWS }
    }
}

impl Interactive for TemplateM {
    fn name(&self) -> &'static str {
        "TABLETEMPLATE"
    }
    fn begin(&mut self, s: &mut Session) -> Result<Step> {
        if s.settings.pickfirst {
            let d = s.doc()?;
            self.target = s.selection().into_iter().find(|h| matches!(d.entity(*h).map(|e| &e.kind), Some(EntityKind::Table(_))));
        }
        Ok(Step::Continue)
    }
    fn prompt(&self, s: &Session) -> Prompt {
        match self.stage {
            Stage::File => {
                let p = Prompt::new("Enter table template file", Accept::TEXT).picker(file_picker(s, "Table template", "template"));
                if self.last.is_empty() { p } else { p.default(self.last.clone()) }
            }
            Stage::Rows => Prompt::new("Enter number of data rows", Accept::NUMBER).default(DEFAULT_ROWS.to_string()),
            Stage::Point => Prompt::new("Specify insertion point", Accept::POINT),
        }
    }
    fn input(&mut self, s: &mut Session, i: Input) -> Result<Step> {
        if matches!(i, Input::Cancel) {
            return Ok(Step::Cancel);
        }
        match self.stage {
            Stage::File => {
                let f = match &i {
                    Input::Text(t) if !t.trim().is_empty() => t.trim().trim_matches('"').to_string(),
                    Input::Text(_) | Input::Enter if !self.last.is_empty() => self.last.clone(),
                    _ => return Ok(Step::Continue),
                };
                match load(s, "tabletemplate", &json!({ "file": f }), None) {
                    Ok(l) => {
                        s.last_used.template_file = l.link.file.clone();
                        if let Some(h) = self.target {
                            apply_to(s, "tabletemplate", h, &l)?;
                            return Ok(Step::Done);
                        }
                        self.loaded = Some(l);
                        self.stage = Stage::Rows;
                    }
                    Err(e) => s.echo(e.to_string()),
                }
                Ok(Step::Continue)
            }
            Stage::Rows => {
                match &i {
                    Input::Enter => self.stage = Stage::Point,
                    Input::Text(t) => match t.trim().parse::<usize>() {
                        Ok(n) if n < MAX_ROWS - MAX_HEADER_ROWS => {
                            self.rows = n;
                            self.stage = Stage::Point;
                        }
                        _ => s.echo(format!("Requires an integer between 0 and {}.", MAX_ROWS - MAX_HEADER_ROWS - 1)),
                    },
                    _ => {}
                }
                Ok(Step::Continue)
            }
            Stage::Point => match i {
                Input::Point(p) => {
                    let Some(l) = &self.loaded else { return Ok(Step::Cancel) };
                    insert(s, l, p, self.rows)?;
                    Ok(Step::Done)
                }
                Input::Enter => Ok(Step::Cancel),
                _ => Ok(Step::Continue),
            },
        }
    }
    fn preview(&self, _s: &Session, c: Vec2) -> Vec<EntityKind> {
        match (&self.loaded, self.stage) {
            (Some(l), Stage::Point) => vec![EntityKind::Table(new_table(&l.template, c, self.rows, None))],
            _ => Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn headers_join_cells_left_and_up() {
        let tpl = parse(&json!({
            "columns": [{"width": 10}, {"width": 10}, {"width": 10}],
            "header": {"rows": [["A", "B", "<"], ["^", "B1", "B2"]], "heights": [8, 7]}
        }))
        .unwrap_or_default();
        let (rows, merges) = header_cells(&tpl, 3, 0);
        assert_eq!(
            rows.iter().map(|r| r.iter().map(|c| c.text.as_str()).collect::<Vec<_>>()).collect::<Vec<_>>(),
            vec![vec!["A", "B", ""], vec!["", "B1", "B2"]]
        );
        assert_eq!(merges, vec![(0, 0, 2, 1), (0, 1, 1, 2)]);
        assert_eq!(tpl.header_heights, vec![8.0, 7.0]);
    }

    #[test]
    fn column_headers_make_one_header_row() {
        let tpl = parse(&json!({"columns": [{"header": "A", "width": 5}, {"width": 5}]})).unwrap_or_default();
        assert_eq!(tpl.header, vec![vec!["A".to_string(), String::new()]]);
        let none = parse(&json!({"columns": [{"width": 5}]})).unwrap_or_default();
        assert!(none.header.is_empty());
        let off = parse(&json!({"columns": [{"header": "A", "width": 5}], "header": false})).unwrap_or_default();
        assert!(off.header.is_empty());
    }

    #[test]
    fn bad_templates_say_what_is_wrong() {
        for v in [
            json!([]),
            json!({}),
            json!({"columns": []}),
            json!({"columns": [{"header": "A"}]}),
            json!({"columns": [{"width": -1}]}),
            json!({"columns": [{"width": 5, "align": "XX"}]}),
            json!({"columns": [{"width": 5, "rotation": 45}]}),
            json!({"columns": [{"width": 5}], "lines": {"outline": 9}}),
            json!({"columns": [{"width": 5}], "rowGrow": "sometimes"}),
        ] {
            assert!(parse(&v).is_err(), "{v}");
        }
    }
}
