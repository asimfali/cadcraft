//! REPORT and REPORTUPDATE: a table made from the drawing's objects by a report definition (a
//! JSON file or object, see [`def`]), and made again when the objects change.
//!
//! A report reads objects (block references, polylines…) and their data (attributes, layer,
//! colour, length, XRECORD values…) as records; each section filters, groups, sorts and sums
//! them into rows, with lookup tables (CSV or JSON) mapping codes to names. The table keeps the
//! definition, its file and the objects it read, so REPORTUPDATE (and the table's right-click
//! menu) fills its data rows again. The rows above the data (title, header) and the look of the
//! first data row stay as the user made them.

mod def;
mod eval;
mod sketch;

use std::path::{Path, PathBuf};

use cadcraft_doc::{Drawing, Entity, EntityKind, Handle, Table, TableCell, TableReport};
use cadcraft_geom::Vec2;
use serde_json::{Value, json};

use super::breaks::{read_link, separator};
use super::{MAX_COLS, MAX_ROWS, anchors, edit, fit_rows, make_table, normalize, parse_csv, set_anchors, style_of};
use crate::cmd::machines::{SelOutcome, SelectPhase};
use crate::cmd::{CommandSpec, bad, point_param, str_param};
use crate::prompt::Picker;
use crate::{Accept, Input, Interactive, Prompt, Result, Session, Step};
pub use def::Report;
use eval::{Lookups, Row};

const PARAMS: &str = "{file?: report definition (.json), definition?: {…} (the definition itself), handles? (objects to read; default: every object), \
     at?: [x,y] (insert a new table), table?: handle (fill this table again), csv?: path (write the rows), title?} → {columns, rows, table?}. \
     Definition: {title?, blockLibrary?: drawing file with sketch blocks, template?: table template file | {…} (the table's look, see tabletemplate), columns?: [header | {header, width}], rowHeight?, lookups?: {name: {file} | {rows: [{…}]}}, \
     sections?: [{title?, from?: \"insert,lwpolyline\", blocks?, layers?, each?: [field], split?, fields?: {name: template | {lookup, match: {col: template}, get, default?}}, \
     where?: {field: pattern (! negates)}, group?: [field], sort?: [field | -field], row: [template per column]}]}. \
     Templates: {field}, {#}, {count}, {sum:field}, {min:…}, {max:…}, {join:…}, {…:N} N decimals. \
     Fields: type, name, layer, color, linetype, handle, length, area, att.TAG, xr.KEY, xd.APP, dyn.NAME, item, index, any property.";

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec::new("report", "Report Table...", run_report)
            .menu(&["Tools", "Report Table..."])
            .alias(&["rpt"])
            .params(PARAMS)
            .interactive(|s| Ok(Box::new(ReportM::new(s)))),
        CommandSpec::new("reportupdate", "Update Report Tables", run_update)
            .alias(&["rptu"])
            .params("{handles?} (report tables; default: every one) → {updated, errors}")
            .interactive(|_| Ok(Box::new(UpdateM::default()))),
    ]
}

/// A height as a prompt default: no trailing zeros.
pub(super) fn fmt_height(v: f64) -> String {
    eval::fmt_num(v, None)
}

/// The directory relative file names are read from: the report file's, else the drawing's.
pub(super) fn base_dir(s: &Session, file: Option<&str>) -> Option<PathBuf> {
    let from_file = file.and_then(|f| Path::new(f).parent()).filter(|p| !p.as_os_str().is_empty()).map(Path::to_path_buf);
    from_file.or_else(|| s.state().ok()?.path.as_deref().and_then(|p| Path::new(p).parent()).map(Path::to_path_buf))
}

/// The folders a bare report or template file name is looked for in: those of the last report
/// and template files, then the drawing's.
pub(crate) fn search_dirs(s: &Session) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    let last = [s.last_used.report_file.as_str(), s.last_used.template_file.as_str()];
    let dirs = last
        .iter()
        .filter_map(|f| Path::new(f.trim()).parent().filter(|p| !p.as_os_str().is_empty()).map(Path::to_path_buf))
        .chain(base_dir(s, None));
    for d in dirs {
        if !out.contains(&d) {
            out.push(d);
        }
    }
    out
}

/// A file the user names: as given when it has a folder (relative to `base`, else the
/// drawing's), else the first of `base` and the [`search_dirs`] that has it.
pub(super) fn locate(s: &Session, base: Option<&Path>, name: &str) -> String {
    let p = Path::new(name.trim());
    let bare = p.components().count() == 1 && p.is_relative();
    if bare {
        let dirs = base.map(Path::to_path_buf).into_iter().chain(search_dirs(s));
        for d in dirs {
            let f = d.join(p);
            if exists(&f) {
                return f.to_string_lossy().into_owned();
            }
        }
    }
    resolve(base.map(Path::to_path_buf).or_else(|| base_dir(s, None)).as_deref(), name)
}

fn exists(p: &Path) -> bool {
    #[cfg(not(target_arch = "wasm32"))]
    {
        p.is_file()
    }
    #[cfg(target_arch = "wasm32")]
    {
        let _ = p;
        false
    }
}

/// What a JSON table file holds: `"report"` (it has `sections` or `row`), `"template"` (it has
/// `columns` with widths and no rows) or nothing.
pub fn file_kind(v: &Value) -> Option<&'static str> {
    let o = v.as_object()?;
    if o.contains_key("sections") || o.contains_key("row") {
        Some("report")
    } else if o.get("columns").and_then(Value::as_array).is_some_and(|c| c.iter().any(|x| x.get("width").is_some())) {
        Some("template")
    } else {
        None
    }
}

/// The picker of a prompt for a report or template file.
pub(super) fn file_picker(s: &Session, title: &str, kind: &str) -> Picker {
    Picker::OpenFile {
        title: title.into(),
        extensions: vec!["json".into()],
        dirs: search_dirs(s).iter().map(|d| d.to_string_lossy().into_owned()).collect(),
        kind: kind.into(),
    }
}

pub(super) fn resolve(base: Option<&Path>, path: &str) -> String {
    let p = Path::new(path.trim());
    match base {
        Some(b) if p.is_relative() => b.join(p).to_string_lossy().into_owned(),
        _ => p.to_string_lossy().into_owned(),
    }
}

/// A definition and the file it came from.
struct Loaded {
    value: Value,
    report: Report,
    file: Option<String>,
    /// The table template the report names.
    template: Option<super::template::Loaded>,
}

fn parse_definition(id: &str, text: &str, from: &str) -> Result<Value> {
    serde_json::from_str(text).map_err(|e| bad(id, format!("{from}: not JSON ({e})")))
}

/// The definition `p` gives: `definition` (an object or JSON text) or `file`.
fn load(s: &Session, id: &str, p: &Value) -> Result<Loaded> {
    let file = str_param(p, "file").map(str::trim).filter(|f| !f.is_empty()).map(|f| locate(s, None, f));
    let value = match p.get("definition") {
        Some(Value::String(t)) => parse_definition(id, t, "definition")?,
        Some(v @ Value::Object(_)) => v.clone(),
        Some(Value::Null) | None => match &file {
            Some(f) => parse_definition(id, &read_link(id, std::path::Path::new(f))?, f)?,
            None => return Err(bad(id, "a report definition is required: `file` (.json) or `definition`")),
        },
        Some(_) => return Err(bad(id, "`definition` must be a JSON object")),
    };
    if file_kind(&value) == Some("template") {
        let name = file.as_deref().unwrap_or("the definition");
        return Err(bad(id, format!("{name} is a table template, not a report: choose a report definition (it has `sections` or `row`)")));
    }
    let report = def::parse(&value).map_err(|m| bad(id, m))?;
    let template = match &report.template {
        None => None,
        Some(t) => {
            let base = base_dir(s, file.as_deref());
            let p = match t {
                Value::String(f) => json!({ "file": f }),
                other => json!({ "template": other }),
            };
            Some(super::template::load(s, id, &p, base.as_deref())?)
        }
    };
    Ok(Loaded { value, report, file, template })
}

/// The rows of a lookup file: CSV with a header row, or a JSON list of objects.
fn lookup_file(id: &str, path: &str) -> Result<Vec<std::collections::BTreeMap<String, String>>> {
    let text = read_link(id, std::path::Path::new(path))?;
    if path.to_ascii_lowercase().ends_with(".json") {
        let v: Value = parse_definition(id, &text, path)?;
        let a = v.as_array().ok_or_else(|| bad(id, format!("{path}: a lookup file must hold a list of objects")))?;
        return Ok(a.iter().take(def::MAX_LOOKUP_ROWS).map(def::row_of).collect());
    }
    let text = text.trim_start_matches('\u{feff}');
    let rows = parse_csv(text, separator(path, text));
    let Some((head, body)) = rows.split_first() else { return Ok(Vec::new()) };
    let head: Vec<String> = head.iter().map(|h| h.trim().to_lowercase()).collect();
    Ok(body.iter().map(|r| head.iter().cloned().zip(r.iter().cloned()).collect()).collect())
}

fn lookups(s: &Session, id: &str, r: &Report, file: Option<&str>) -> Result<Lookups> {
    let base = base_dir(s, file);
    let mut out = Lookups::new();
    for (name, src) in &r.lookups {
        let rows = match src {
            def::LookupSource::Rows(rows) => rows.clone(),
            def::LookupSource::File(f) => lookup_file(id, &resolve(base.as_deref(), f))?,
        };
        out.insert(name.clone(), rows);
    }
    Ok(out)
}

/// The objects a report reads: `handles` (those that still exist), or every object.
fn objects<'a>(d: &'a Drawing, handles: &[Handle]) -> Vec<&'a Entity> {
    if handles.is_empty() {
        std::iter::once(&d.model).chain(d.layouts.iter().map(|l| &l.entities)).flat_map(|st| st.iter()).map(|e| e.as_ref()).collect()
    } else {
        handles.iter().filter_map(|h| d.entity(*h)).map(|e| e.as_ref()).collect()
    }
}

/// Run the report: its rows.
fn rows(s: &Session, id: &str, l: &Loaded, handles: &[Handle]) -> Result<Vec<Row>> {
    let lk = lookups(s, id, &l.report, l.file.as_deref())?;
    let d = s.doc()?;
    let objs = objects(d, handles);
    let rows = eval::run(d, &l.report, &objs, &lk).map_err(|m| bad(id, m))?;
    if rows.len().saturating_add(2) > MAX_ROWS {
        return Err(bad(id, format!("the report makes {} rows; a table holds at most {MAX_ROWS}", rows.len())));
    }
    Ok(rows)
}

/// The widths (at scale 1) of the blocks the rows name, by upper-case name.
type BlockSizes = std::collections::HashMap<String, f64>;

fn block_sizes(d: &Drawing, rows: &[Row]) -> BlockSizes {
    let mut out = BlockSizes::new();
    for r in rows {
        if let Row::Data(v) = r {
            for b in v.iter().filter_map(|o| o.block.as_ref()) {
                let key = b.name.to_uppercase();
                if !out.contains_key(&key)
                    && let Some(sz) = cadcraft_doc::cell_block_size(d, b)
                {
                    out.insert(key, sz.x);
                }
            }
        }
    }
    out
}

/// Make the blocks of the rows' wire sketches, their text in `style`.
/// Blocks named in place of sketches that the drawing lacks are copied from the report's block
/// library; any still missing are reported at the command line (their cells stay empty).
fn make_sketches(s: &mut Session, l: &Loaded, rows: &[Row], style: &str) -> Result<()> {
    let d = s.doc_mut()?;
    let mut named: Vec<String> = Vec::new();
    for r in rows {
        if let Row::Data(v) = r {
            for o in v {
                if let Some(sk) = &o.sketch {
                    sketch::ensure_block(d, sk, style);
                }
                if let Some(b) = &o.block
                    && d.block(&b.name).is_none()
                    && !named.iter().any(|n| n.eq_ignore_ascii_case(&b.name))
                {
                    named.push(b.name.clone());
                }
            }
        }
    }
    if named.is_empty() {
        return Ok(());
    }
    let mut missing = named.clone();
    if let Some(lib) = &l.report.block_library {
        let path = locate(s, base_dir(s, l.file.as_deref()).as_deref(), lib);
        match crate::cmd::blocks::insert_file::import_blocks(s, &path, &named) {
            Ok(m) => missing = m,
            Err(e) => s.echo(e.to_string()),
        }
    }
    if !missing.is_empty() {
        s.echo(format!("Blocks not found for the sketches: {}.", missing.join(", ")));
    }
    Ok(())
}

/// The text style a report's sketches use: its template's, else Standard.
fn sketch_style(s: &Session, l: &Loaded) -> String {
    l.template
        .as_ref()
        .and_then(|tl| tl.template.text_style.as_ref())
        .and_then(|(n, _)| s.doc().ok()?.text_style(n).map(|st| st.name.clone()))
        .unwrap_or_else(|| "Standard".into())
}

/// Put `rows` into `t` from row `start` on, in `cols` columns. The new rows look like the
/// first data row was (its height and its cells' formats); merges below `start` go.
fn fill(t: &mut Table, start: usize, cols: usize, rows: &[Row], row_h: Option<f64>, sizes: &BlockSizes) {
    let cols = cols.clamp(1, MAX_COLS);
    let start = start.min(t.cells.len());
    let model: Vec<TableCell> = t.cells.get(start).cloned().unwrap_or_default();
    let model_h = t.row_min.get(start).or(t.row_heights.get(start)).copied();
    let h = row_h.or(model_h).or_else(|| t.row_heights.last().copied()).unwrap_or(t.text_height * 5.0 / 3.0);
    let keep_min = t.row_min.len() == t.row_heights.len() && !t.row_min.is_empty();
    let mut a: Vec<_> = anchors(t).into_iter().filter(|(r, ..)| *r < start).collect();
    t.cells.truncate(start);
    t.row_heights.truncate(start);
    t.row_min.truncate(start);
    t.row_styles.truncate(start);
    let cell = |c: usize, text: String| TableCell { text, merged: None, formula: None, block: None, ..model.get(c).cloned().unwrap_or_default() };
    // A sketch: its block (made by `make_sketches`), every one of a column at one scale: its
    // width across the cell, so the texts of all rows are alike.
    let widths = t.col_widths.clone();
    let out = |c: usize, o: Option<&eval::Out>| match o {
        Some(eval::Out { sketch: Some(sk), .. }) => {
            let scale = widths.get(c).map(|w| w * 0.94 / sk.width.max(1.0)).filter(|k| k.is_finite() && *k > 0.0);
            TableCell { block: Some(cadcraft_doc::CellBlock { name: sk.block_name(), scale, ..Default::default() }), ..cell(c, String::new()) }
        }
        // A block named for the cell: as drawn, made smaller only to fit the column; the row
        // grows for its height.
        Some(eval::Out { block: Some(b), .. }) => {
            let fit = sizes.get(&b.name.to_uppercase()).zip(widths.get(c)).map(|(w, cw)| if *w > 0.0 { (cw * 0.94 / w).min(1.0) } else { 1.0 });
            TableCell {
                block: Some(cadcraft_doc::CellBlock { scale: fit.filter(|k| k.is_finite() && *k > 0.0), ..b.clone() }),
                ..cell(c, String::new())
            }
        }
        Some(o) => cell(c, o.text.clone()),
        None => cell(c, String::new()),
    };
    for row in rows {
        let r = t.cells.len();
        match row {
            Row::Data(v) => t.cells.push((0..cols).map(|c| out(c, v.get(c))).collect()),
            Row::Title(text) => {
                t.cells.push((0..cols).map(|c| cell(c, if c == 0 { text.clone() } else { String::new() })).collect());
                if cols > 1 {
                    a.push((r, 0, 1, cols));
                }
            }
        }
        t.row_heights.push(h);
        if keep_min {
            t.row_min.push(h);
        }
    }
    // A table keeps a row.
    if t.cells.is_empty() {
        t.cells.push((0..cols).map(|c| cell(c, String::new())).collect());
        t.row_heights.push(h);
    }
    if !keep_min {
        t.row_min.clear();
    }
    let w = t.col_widths.last().copied().unwrap_or(2.5);
    t.col_widths.resize(cols, w);
    for row in &mut t.cells {
        row.resize_with(cols, TableCell::default);
    }
    // A title merged across the table stays across.
    if t.title && start > 0 && cols > 1 {
        a.retain(|x| x.0 != 0);
        a.push((0, 0, 1, cols));
    }
    set_anchors(t, &a);
}

/// A new table for report `l` at `at`.
fn new_table(s: &Session, l: &Loaded, rows: &[Row], at: Vec2, title: Option<&str>) -> Table {
    let r = &l.report;
    if let Some(tl) = &l.template {
        // The template gives the head, the columns and the look; the report the rows.
        let tpl = &tl.template;
        let mut t = super::template::new_table(tpl, at, 0, Some(tl.link.clone()));
        if let Some(text) = title.or(r.title.as_deref())
            && t.title
            && let Some(c) = t.cells.first_mut().and_then(|row| row.first_mut())
        {
            c.text = text.to_string();
        }
        let head = tpl.head_rows();
        let sizes = s.doc().map(|d| block_sizes(d, rows)).unwrap_or_default();
        fill(&mut t, head, r.column_count().max(tpl.columns.len()), rows, tpl.row_height.or(r.row_height), &sizes);
        super::template::data_looks(&mut t, tpl, head);
        return t;
    }
    let st = style_of(s);
    let cols = r.column_count();
    let header = r.has_header().then(|| (0..cols).map(|c| r.columns.get(c).map(|x| x.header.clone()).unwrap_or_default()).collect());
    let title = title.or(r.title.as_deref());
    let mut t = make_table(&st, at, 0, cols, r.row_height, None, title, header, &[]);
    let start = usize::from(t.title) + usize::from(t.header);
    let data_h = r.row_height.unwrap_or(st.text_height * 5.0 / 3.0 + 2.0 * st.margin);
    // Widths known before the data (sketches are scaled to them): as given, else a sketch's own.
    let sketch_w = |c: usize| {
        rows.iter().find_map(|row| match row {
            Row::Data(v) => v.get(c).and_then(|o| o.sketch.as_ref()).map(|sk| sk.width),
            Row::Title(_) => None,
        })
    };
    let fixed: Vec<Option<f64>> = (0..t.col_widths.len()).map(|c| r.columns.get(c).and_then(|x| x.width).or_else(|| sketch_w(c))).collect();
    for (w, f) in t.col_widths.iter_mut().zip(&fixed) {
        if let Some(f) = f {
            *w = *f;
        }
    }
    // `make_table` gave a plain table an empty row: the data replaces it.
    let sizes = s.doc().map(|d| block_sizes(d, rows)).unwrap_or_default();
    fill(&mut t, start, cols, rows, Some(data_h), &sizes);
    // The other columns as wide as their longest text.
    for (c, w) in t.col_widths.iter_mut().enumerate() {
        if fixed.get(c).copied().flatten().is_some() {
            continue;
        }
        let chars = t
            .cells
            .iter()
            .skip(usize::from(t.title))
            .filter_map(|row| row.get(c))
            .filter(|x| x.merged.is_none())
            .map(|x| x.text.chars().count())
            .max()
            .unwrap_or(0)
            .clamp(1, 80);
        *w = chars as f64 * st.text_height * 0.9 + 2.0 * st.margin + st.text_height;
    }
    t
}

/// What the table keeps of its report.
fn link(l: &Loaded, start_row: usize, handles: &[Handle]) -> TableReport {
    let definition = serde_json::to_string(&l.value).unwrap_or_default();
    let definition = if definition.len() > cadcraft_doc::MAX_REPORT_BYTES { String::new() } else { definition };
    TableReport {
        file: l.file.clone().unwrap_or_default(),
        definition,
        start_row,
        handles: handles.iter().take(cadcraft_doc::MAX_REPORT_HANDLES).copied().collect(),
    }
}

fn rows_json(rows: &[Row]) -> Vec<Value> {
    rows.iter()
        .map(|r| match r {
            Row::Data(v) => json!(v.iter().map(|o| o.text.clone()).collect::<Vec<_>>()),
            Row::Title(t) => json!({ "title": t }),
        })
        .collect()
}

fn csv_field(text: &str, sep: char) -> String {
    if text.contains([sep, '"', '\n', '\r']) { format!("\"{}\"", text.replace('"', "\"\"")) } else { text.to_string() }
}

fn write_csv(id: &str, path: &str, columns: &[String], rows: &[Row]) -> Result<()> {
    let sep = if path.to_ascii_lowercase().ends_with(".txt") || path.to_ascii_lowercase().ends_with(".tsv") { '\t' } else { ',' };
    let line = |cells: &[String]| cells.iter().map(|c| csv_field(c, sep)).collect::<Vec<_>>().join(&sep.to_string());
    let mut out = String::new();
    if columns.iter().any(|c| !c.is_empty()) {
        out.push_str(&line(columns));
        out.push_str("\r\n");
    }
    for r in rows {
        let cells = match r {
            Row::Data(v) => v.iter().map(|o| o.text.clone()).collect(),
            Row::Title(t) => vec![t.clone()],
        };
        out.push_str(&line(&cells));
        out.push_str("\r\n");
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        std::fs::write(path, out).map_err(|e| bad(id, format!("{path}: {e}")))
    }
    #[cfg(target_arch = "wasm32")]
    {
        let _ = out;
        Err(bad(id, format!("{path}: files can't be written in the browser")))
    }
}

/// The handles `p` names in `handles` (none: every object).
fn handles_param(id: &str, p: &Value) -> Result<Vec<Handle>> {
    match p.get("handles") {
        None | Some(Value::Null) => Ok(Vec::new()),
        Some(Value::Array(a)) => a
            .iter()
            .take(cadcraft_doc::MAX_REPORT_HANDLES)
            .map(|v| {
                v.as_str()
                    .and_then(Handle::parse_hex)
                    .or_else(|| v.as_u64().map(Handle))
                    .ok_or_else(|| bad(id, format!("`handles`: {v} is not a handle")))
            })
            .collect(),
        Some(_) => Err(bad(id, "`handles` must be a list of object handles")),
    }
}

fn table_handle(p: &Value) -> Option<Handle> {
    p.get("table").and_then(|v| v.as_str().and_then(Handle::parse_hex).or_else(|| v.as_u64().map(Handle)))
}

/// Fill the data rows of report table `h` again with `rows`, keeping what it knows of `l`.
fn refill(s: &mut Session, id: &str, h: Handle, l: &Loaded, rows: &[Row], handles: &[Handle]) -> Result<()> {
    let cols = l.report.column_count().max(l.template.as_ref().map_or(0, |tl| tl.template.columns.len()));
    // The template's lines and text as they are now (its head stays as the user left it).
    if let Some(tl) = &l.template {
        super::template::make_styles(s.doc_mut()?, &tl.template);
    }
    let style = sketch_style(s, l);
    make_sketches(s, l, rows, &style)?;
    let sizes = block_sizes(s.doc()?, rows);
    edit(s, &json!({ "handle": h.hex() }), id, |t| {
        let start = super::head_rows(t);
        fill(t, start, cols, rows, None, &sizes);
        if let Some(tl) = &l.template {
            super::template::data_looks(t, &tl.template, start);
            t.template = Some(tl.link.clone());
        }
        t.report = Some(link(l, start, handles));
        Ok(())
    })?;
    Ok(())
}

fn run_report(s: &mut Session, p: &Value) -> Result<Value> {
    let id = "report";
    let l = load(s, id, p)?;
    let handles = handles_param(id, p)?;
    let rows = rows(s, id, &l, &handles)?;
    // Column names: the template's headers when it has them, else the report's.
    let tcols = l.template.as_ref().map(|tl| &tl.template.columns).filter(|c| c.iter().any(|x| !x.header.is_empty()));
    let ncols = l.report.column_count().max(tcols.map_or(0, Vec::len));
    let columns: Vec<String> = (0..ncols)
        .map(|c| match tcols {
            Some(tc) => tc.get(c).map(|x| x.header.clone()).unwrap_or_default(),
            None => l.report.columns.get(c).map(|x| x.header.clone()).unwrap_or_default(),
        })
        .collect();
    let mut out = json!({ "columns": columns, "rows": rows_json(&rows) });
    if let Some(path) = str_param(p, "csv").map(str::trim).filter(|f| !f.is_empty()) {
        let path = resolve(base_dir(s, None).as_deref(), path);
        write_csv(id, &path, &columns, &rows)?;
        out["csv"] = json!(path);
    }
    if let Some(h) = table_handle(p) {
        refill(s, id, h, &l, &rows, &handles)?;
        out["table"] = json!(h.hex());
    } else if p.get("at").is_some_and(|v| !v.is_null()) {
        let at = point_param(p, "at").ok_or_else(|| bad(id, "`at` must be a point [x, y]"))?;
        let h = insert(s, &l, &rows, at, &handles, str_param(p, "title"))?;
        out["table"] = json!(h.hex());
    }
    Ok(out)
}

/// Insert a new report table.
fn insert(s: &mut Session, l: &Loaded, rows: &[Row], at: Vec2, handles: &[Handle], title: Option<&str>) -> Result<Handle> {
    if let Some(tl) = &l.template {
        super::template::make_styles(s.doc_mut()?, &tl.template);
    }
    let style = sketch_style(s, l);
    make_sketches(s, l, rows, &style)?;
    let mut t = new_table(s, l, rows, at, title);
    let start = l.template.as_ref().map_or(usize::from(t.title) + usize::from(t.header), |tl| tl.template.head_rows());
    t.report = Some(link(l, start, handles));
    normalize(&mut t);
    fit_rows(s.doc()?, &mut t);
    s.add_entity(EntityKind::Table(t))
}

/// Make report table `h` again from the drawing as it is now.
fn update(s: &mut Session, h: Handle) -> Result<usize> {
    let id = "reportupdate";
    let r = match s.doc()?.entity(h).map(|e| &e.kind) {
        Some(EntityKind::Table(t)) => t.report.clone().ok_or_else(|| bad(id, "the table is not a report table"))?,
        _ => return Err(bad(id, "object is not a table")),
    };
    // The file as it is now, else the definition the table kept.
    let from_file = (!r.file.trim().is_empty()).then(|| load(s, id, &json!({ "file": r.file })));
    let l = match from_file {
        Some(Ok(l)) => l,
        Some(Err(e)) if r.definition.trim().is_empty() => return Err(e),
        _ => {
            let mut l = load(s, id, &json!({ "definition": r.definition }))?;
            l.file = (!r.file.trim().is_empty()).then(|| r.file.clone());
            l
        }
    };
    let rows = rows(s, id, &l, &r.handles)?;
    refill(s, id, h, &l, &rows, &r.handles)?;
    Ok(rows.len())
}

/// Every report table of the drawing.
fn report_tables(d: &Drawing) -> Vec<Handle> {
    std::iter::once(&d.model)
        .chain(d.layouts.iter().map(|l| &l.entities))
        .flat_map(|st| st.iter())
        .filter(|e| matches!(&e.kind, EntityKind::Table(t) if t.report.is_some()))
        .map(|e| e.handle)
        .collect()
}

fn run_update(s: &mut Session, p: &Value) -> Result<Value> {
    let given = handles_param("reportupdate", p)?;
    let hs: Vec<Handle> = {
        let d = s.doc()?;
        if given.is_empty() {
            report_tables(d)
        } else {
            given.into_iter().filter(|h| matches!(d.entity(*h).map(|e| &e.kind), Some(EntityKind::Table(t)) if t.report.is_some())).collect()
        }
    };
    let mut updated = 0;
    let mut errors = Vec::new();
    for h in hs {
        match update(s, h) {
            Ok(_) => updated += 1,
            Err(e) => errors.push(e.to_string()),
        }
    }
    let mut msg = format!("{updated} report table(s) updated.");
    for e in &errors {
        msg.push('\n');
        msg.push_str(e);
    }
    Ok(json!({ "updated": updated, "errors": errors, "message": msg }))
}

// ---------------- command line ----------------

#[derive(Clone, Copy, Debug, Default, PartialEq)]
enum Stage {
    #[default]
    File,
    Select,
    Point,
}

/// REPORT at the command line: the report file, the objects (Enter: all of them), then where
/// the table goes.
struct ReportM {
    stage: Stage,
    last: String,
    loaded: Option<Loaded>,
    sel: SelectPhase,
    /// What was selected before the command: offered as the Selected option.
    pre: Vec<Handle>,
    handles: Vec<Handle>,
    rows: Vec<Row>,
}

impl ReportM {
    fn new(s: &Session) -> Self {
        ReportM {
            stage: Stage::File,
            last: s.last_used.report_file.clone(),
            loaded: None,
            sel: SelectPhase::default(),
            pre: Vec::new(),
            handles: Vec::new(),
            rows: Vec::new(),
        }
    }

    /// Picks at "Select objects": Enter with nothing picked takes what was selected before the
    /// command, else every object.
    fn select(&mut self, s: &mut Session, i: Input) -> Result<Step> {
        match self.sel.feed(s, &i)? {
            SelOutcome::More => Ok(Step::Continue),
            SelOutcome::Empty => {
                let hs = std::mem::take(&mut self.pre);
                s.set_selection(Vec::new());
                self.objects_chosen(s, hs)
            }
            SelOutcome::Done(hs) => {
                s.set_selection(Vec::new());
                self.objects_chosen(s, hs)
            }
        }
    }

    /// The objects are known: run the report, then ask for the point.
    fn objects_chosen(&mut self, s: &mut Session, hs: Vec<Handle>) -> Result<Step> {
        let Some(l) = &self.loaded else { return Ok(Step::Cancel) };
        self.rows = rows(s, "report", l, &hs)?;
        self.handles = hs;
        let n = self.rows.iter().filter(|r| matches!(r, Row::Data(_))).count();
        s.echo(format!("{n} row{} in the report.", if n == 1 { "" } else { "s" }));
        self.stage = Stage::Point;
        Ok(Step::Continue)
    }
}

impl Interactive for ReportM {
    fn name(&self) -> &'static str {
        "REPORT"
    }
    fn begin(&mut self, s: &mut Session) -> Result<Step> {
        // The objects are always asked for (the file comes first): a selection made before is
        // the default, not taken silently.
        if s.settings.pickfirst {
            self.pre = s.selection();
        }
        Ok(Step::Continue)
    }
    fn prompt(&self, s: &Session) -> Prompt {
        match self.stage {
            Stage::File => {
                let p = Prompt::new("Enter report file name", Accept::TEXT).picker(file_picker(s, "Report definition", "report"));
                if self.last.is_empty() { p } else { p.default(self.last.clone()) }
            }
            Stage::Select if self.sel.removing => self.sel.prompt(),
            // ALL (any Select objects prompt has it) takes every object.
            Stage::Select if !self.pre.is_empty() && self.sel.picked.is_empty() => {
                Prompt::new("Select objects to report on", Accept::SELECT).kw(&["Selected"]).default("Selected")
            }
            Stage::Select => Prompt::new("Select objects to report on <All>", Accept::SELECT),
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
                match load(s, "report", &json!({ "file": f })) {
                    Ok(l) => {
                        s.last_used.report_file = l.file.clone().unwrap_or(f);
                        self.loaded = Some(l);
                        self.sel = SelectPhase::default();
                        self.stage = Stage::Select;
                    }
                    Err(e) => s.echo(e.to_string()),
                }
                Ok(Step::Continue)
            }
            Stage::Select => match &i {
                Input::Keyword(k) | Input::Text(k) if ["selected", "s"].contains(&k.trim().to_ascii_lowercase().as_str()) && !self.pre.is_empty() => {
                    let hs = self.pre.clone();
                    s.set_selection(Vec::new());
                    self.objects_chosen(s, hs)
                }
                _ => self.select(s, i),
            },
            Stage::Point => match i {
                Input::Point(p) => {
                    let Some(l) = &self.loaded else { return Ok(Step::Cancel) };
                    insert(s, l, &self.rows, p, &self.handles, None)?;
                    Ok(Step::Done)
                }
                Input::Enter => Ok(Step::Cancel),
                _ => Ok(Step::Continue),
            },
        }
    }
    fn preview(&self, s: &Session, c: Vec2) -> Vec<EntityKind> {
        match (&self.loaded, self.stage) {
            (Some(l), Stage::Point) => vec![EntityKind::Table(new_table(s, l, &self.rows, c, None))],
            _ => Vec::new(),
        }
    }
}

/// REPORTUPDATE at the command line: "Select report tables <All>".
#[derive(Default)]
struct UpdateM {
    sel: SelectPhase,
}

impl UpdateM {
    fn run(s: &mut Session, hs: &[Handle]) -> Result<Step> {
        let r = run_update(s, &json!({ "handles": hs.iter().map(|h| h.hex()).collect::<Vec<_>>() }))?;
        if let Some(m) = r.get("message").and_then(Value::as_str) {
            s.echo(m.to_string());
        }
        Ok(Step::Done)
    }
}

impl Interactive for UpdateM {
    fn name(&self) -> &'static str {
        "REPORTUPDATE"
    }
    fn begin(&mut self, s: &mut Session) -> Result<Step> {
        self.sel = SelectPhase::begin(s);
        if self.sel.done {
            let hs = self.sel.picked.clone();
            return Self::run(s, &hs);
        }
        Ok(Step::Continue)
    }
    fn prompt(&self, _s: &Session) -> Prompt {
        if self.sel.removing { self.sel.prompt() } else { Prompt::new("Select report tables to update <All>", Accept::SELECT) }
    }
    fn input(&mut self, s: &mut Session, i: Input) -> Result<Step> {
        match self.sel.feed(s, &i)? {
            SelOutcome::More => Ok(Step::Continue),
            SelOutcome::Empty => Self::run(s, &[]),
            SelOutcome::Done(hs) => {
                s.set_selection(Vec::new());
                Self::run(s, &hs)
            }
        }
    }
}
