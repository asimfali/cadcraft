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
     Definition: {title?, columns?: [header | {header, width}], rowHeight?, lookups?: {name: {file} | {rows: [{…}]}}, \
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

/// The directory relative file names are read from: the report file's, else the drawing's.
fn base_dir(s: &Session, file: Option<&str>) -> Option<PathBuf> {
    let from_file = file.and_then(|f| Path::new(f).parent()).filter(|p| !p.as_os_str().is_empty()).map(Path::to_path_buf);
    from_file.or_else(|| s.state().ok()?.path.as_deref().and_then(|p| Path::new(p).parent()).map(Path::to_path_buf))
}

fn resolve(base: Option<&Path>, path: &str) -> String {
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
}

fn parse_definition(id: &str, text: &str, from: &str) -> Result<Value> {
    serde_json::from_str(text).map_err(|e| bad(id, format!("{from}: not JSON ({e})")))
}

/// The definition `p` gives: `definition` (an object or JSON text) or `file`.
fn load(s: &Session, id: &str, p: &Value) -> Result<Loaded> {
    let file = str_param(p, "file").map(str::trim).filter(|f| !f.is_empty()).map(|f| resolve(base_dir(s, None).as_deref(), f));
    let value = match p.get("definition") {
        Some(Value::String(t)) => parse_definition(id, t, "definition")?,
        Some(v @ Value::Object(_)) => v.clone(),
        Some(Value::Null) | None => match &file {
            Some(f) => parse_definition(id, &read_link(id, f)?, f)?,
            None => return Err(bad(id, "a report definition is required: `file` (.json) or `definition`")),
        },
        Some(_) => return Err(bad(id, "`definition` must be a JSON object")),
    };
    let report = def::parse(&value).map_err(|m| bad(id, m))?;
    Ok(Loaded { value, report, file })
}

/// The rows of a lookup file: CSV with a header row, or a JSON list of objects.
fn lookup_file(id: &str, path: &str) -> Result<Vec<std::collections::BTreeMap<String, String>>> {
    let text = read_link(id, path)?;
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

/// Put `rows` into `t` from row `start` on, in `cols` columns. The new rows look like the
/// first data row was (its height and its cells' formats); merges below `start` go.
fn fill(t: &mut Table, start: usize, cols: usize, rows: &[Row], row_h: Option<f64>) {
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
    for row in rows {
        let r = t.cells.len();
        match row {
            Row::Data(v) => t.cells.push((0..cols).map(|c| cell(c, v.get(c).cloned().unwrap_or_default())).collect()),
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
    let st = style_of(s);
    let r = &l.report;
    let cols = r.column_count();
    let header = r.has_header().then(|| (0..cols).map(|c| r.columns.get(c).map(|x| x.header.clone()).unwrap_or_default()).collect());
    let title = title.or(r.title.as_deref());
    let mut t = make_table(&st, at, 0, cols, r.row_height, None, title, header, &[]);
    let start = usize::from(t.title) + usize::from(t.header);
    let data_h = r.row_height.unwrap_or(st.text_height * 5.0 / 3.0 + 2.0 * st.margin);
    // `make_table` gave a plain table an empty row: the data replaces it.
    fill(&mut t, start, cols, rows, Some(data_h));
    // Columns as given, else as wide as their longest text.
    for (c, w) in t.col_widths.iter_mut().enumerate() {
        if let Some(given) = r.columns.get(c).and_then(|x| x.width) {
            *w = given;
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
            Row::Data(v) => json!(v),
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
            Row::Data(v) => v.clone(),
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
    let cols = l.report.column_count();
    edit(s, &json!({ "handle": h.hex() }), id, |t| {
        let start = match &t.report {
            Some(r) => r.start_row,
            None => usize::from(t.title) + usize::from(t.header),
        }
        .min(t.cells.len());
        fill(t, start, cols, rows, None);
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
    let columns: Vec<String> = (0..l.report.column_count()).map(|c| l.report.columns.get(c).map(|x| x.header.clone()).unwrap_or_default()).collect();
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
    let mut t = new_table(s, l, rows, at, title);
    let start = usize::from(t.title) + usize::from(t.header);
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
            handles: Vec::new(),
            rows: Vec::new(),
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
    fn prompt(&self, _s: &Session) -> Prompt {
        match self.stage {
            Stage::File => {
                let p = Prompt::new("Enter report file name", Accept::TEXT)
                    .picker(Picker::OpenFile { title: "Report definition".into(), extensions: vec!["json".into()] });
                if self.last.is_empty() { p } else { p.default(self.last.clone()) }
            }
            Stage::Select if self.sel.removing => self.sel.prompt(),
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
                        self.sel = SelectPhase::begin(s);
                        if self.sel.done {
                            let hs = self.sel.picked.clone();
                            return self.objects_chosen(s, hs);
                        }
                        self.stage = Stage::Select;
                    }
                    Err(e) => s.echo(e.to_string()),
                }
                Ok(Step::Continue)
            }
            Stage::Select => match self.sel.feed(s, &i)? {
                SelOutcome::More => Ok(Step::Continue),
                SelOutcome::Empty => self.objects_chosen(s, Vec::new()),
                SelOutcome::Done(hs) => {
                    s.set_selection(Vec::new());
                    self.objects_chosen(s, hs)
                }
            },
            Stage::Point => match i {
                Input::Point(p) => {
                    let Some(l) = &self.loaded else { return Ok(Step::Cancel) };
                    let h = insert(s, l, &self.rows, p, &self.handles, None)?;
                    s.set_selection(vec![h]);
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
