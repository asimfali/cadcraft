//! What table cells hold beyond text: TINSERT puts a block in a cell (with attribute values and
//! a dynamic block's visibility state); `table.cellborders` gives cells their own borders.

use std::collections::BTreeMap;

use cadcraft_doc::dynblock::{ParameterKind, dynamic_block};
use cadcraft_doc::{CellBlock, CellBorder, Drawing, Handle, MAX_CELL_BLOCK_VALUES, Table};
use cadcraft_geom::Vec2;
use serde_json::{Value, json};

use super::cells::{apply_border, cell_under};
use super::{anchors, cell_at, edit, usize_param};
use crate::cmd::{CommandSpec, bad, str_param};
use crate::{Accept, EngineError, Input, Interactive, Prompt, Result, Session, Step};

const TINSERT_PARAMS: &str = "{handle?, row+col | at: [x,y], block: name (\"\" removes the block), scale?: number | \"fit\" (default fit), \
     rotation?: degrees, attribs?: {TAG: value}, props?: {Visibility1: state}} → {handle}";

const BORDERS_PARAMS: &str = "{handle?, row, col, rows? (default 1), cols? (default 1), \
     edges?: all|outside|inside|top|bottom|left|right|insideH|insideV or a list (default all), \
     visible?: bool, color?, lineweight? (mm or ByBlock/ByLayer), clear?: bool (back to the cell style)} → {handle}";

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec::new("tinsert", "Insert Block in Table Cell", run_tinsert)
            .params(TINSERT_PARAMS)
            .interactive(|_| Ok(Box::new(TinsertM::default()))),
        CommandSpec::new("table.cellborders", "Set Table Cell Borders", run_borders).params(BORDERS_PARAMS),
    ]
}

// ---------------------------------------------------------------- TINSERT

/// A JSON object of strings (numbers and booleans as typed), at most [`MAX_CELL_BLOCK_VALUES`].
fn string_map(p: &Value, key: &str) -> std::result::Result<BTreeMap<String, String>, String> {
    let Some(v) = p.get(key).filter(|v| !v.is_null()) else { return Ok(BTreeMap::new()) };
    let o = v.as_object().ok_or_else(|| format!("`{key}` must be an object of names and values"))?;
    if o.len() > MAX_CELL_BLOCK_VALUES {
        return Err(format!("`{key}` has more than {MAX_CELL_BLOCK_VALUES} entries"));
    }
    Ok(o.iter()
        .map(|(k, v)| (k.trim().to_string(), v.as_str().map(str::to_string).unwrap_or_else(|| v.to_string())))
        .filter(|(k, _)| !k.is_empty())
        .collect())
}

/// Check custom properties `props` against block `name`: each must name a visibility parameter
/// of the dynamic block and one of its states (other properties change the block's geometry,
/// which cells can't show yet).
fn check_props(d: &Drawing, name: &str, props: &BTreeMap<String, String>) -> std::result::Result<(), String> {
    if props.is_empty() {
        return Ok(());
    }
    let m = dynamic_block(d, name).ok_or_else(|| format!("Block \"{name}\" is not a dynamic block."))?;
    let vis: Vec<(&str, Vec<&str>)> = m
        .parameters
        .iter()
        .filter_map(|p| match &p.kind {
            ParameterKind::Visibility { label, states, .. } => Some((label.as_str(), states.iter().map(|s| s.name.as_str()).collect())),
            _ => None,
        })
        .collect();
    for (k, v) in props {
        let found =
            vis.iter().find(|(l, _)| l.eq_ignore_ascii_case(k)).or_else(|| k.eq_ignore_ascii_case("Visibility").then(|| vis.first()).flatten());
        let Some((label, states)) = found else {
            return Err(format!("A table cell can set only a visibility state of \"{name}\"; it has no visibility property \"{k}\"."));
        };
        if !states.iter().any(|s| s.eq_ignore_ascii_case(v.trim())) {
            return Err(format!("\"{v}\" is not a state of {label}: {}.", states.join(", ")));
        }
    }
    Ok(())
}

/// The table and cell a command names: `at` (a point in a cell), else `handle` + `row` + `col`.
fn target(s: &Session, p: &Value, id: &str) -> Result<Value> {
    let mut q = p.clone();
    if let Some(at) = p.get("at").and_then(|v| serde_json::from_value::<[f64; 2]>(v.clone()).ok()) {
        let (h, r, c) = cell_under(s, Vec2::new(at[0], at[1])).ok_or_else(|| bad(id, "no table cell at that point"))?;
        q["handle"] = json!(h.hex());
        q["row"] = json!(r);
        q["col"] = json!(c);
    }
    Ok(q)
}

/// The anchor of the merge covering cell (`r`, `c`), or the cell itself.
fn anchor_of(t: &Table, r: usize, c: usize) -> (usize, usize) {
    anchors(t).into_iter().find(|&(ar, ac, rs, cs)| r >= ar && r < ar + rs && c >= ac && c < ac + cs).map(|(ar, ac, ..)| (ar, ac)).unwrap_or((r, c))
}

fn run_tinsert(s: &mut Session, p: &Value) -> Result<Value> {
    let q = target(s, p, "tinsert")?;
    let name = str_param(&q, "block").map(str::trim).unwrap_or("").to_string();
    let block = if name.is_empty() {
        None
    } else {
        let d = s.doc()?;
        let real = d.block(&name).map(|b| b.name.clone()).ok_or_else(|| bad("tinsert", format!("Block \"{name}\" not found.")))?;
        let scale = match q.get("scale") {
            None | Some(Value::Null) => None,
            Some(Value::String(f)) if f.trim().eq_ignore_ascii_case("fit") => None,
            Some(v) => {
                Some(v.as_f64().filter(|x| x.is_finite() && *x > 0.0 && *x <= 1e9).ok_or_else(|| bad("tinsert", format!("invalid scale {v}")))?)
            }
        };
        let rotation = match q.get("rotation") {
            None | Some(Value::Null) => 0.0,
            Some(v) => v.as_f64().filter(|x| x.is_finite()).ok_or_else(|| bad("tinsert", format!("invalid rotation {v}")))?.to_radians(),
        };
        let attribs = string_map(&q, "attribs").map_err(|m| bad("tinsert", m))?;
        let props = string_map(&q, "props").map_err(|m| bad("tinsert", m))?;
        check_props(d, &real, &props).map_err(|m| bad("tinsert", m))?;
        Some(CellBlock { name: real, scale, rotation, attribs, props }.sanitized())
    };
    edit(s, &q, "tinsert", |t| {
        let (r, c) = cell_at(&q, t)?;
        let (r, c) = anchor_of(t, r, c);
        if let Some(cell) = t.cells.get_mut(r).and_then(|row| row.get_mut(c)) {
            if block.is_some() {
                // A cell holds a block or text, not both (as in AutoCAD).
                cell.text.clear();
                cell.formula = None;
            }
            cell.block = block;
        }
        Ok(())
    })
}

/// TINSERT at the command line: pick a cell, then name the block (fitted into the cell).
#[derive(Default)]
struct TinsertM {
    cell: Option<(Handle, usize, usize)>,
}

impl Interactive for TinsertM {
    fn name(&self) -> &'static str {
        "TINSERT"
    }
    fn prompt(&self, _s: &Session) -> Prompt {
        match self.cell {
            None => Prompt::new("Pick a table cell", Accept::POINT),
            Some(_) => Prompt::new("Enter block name", Accept::TEXT),
        }
    }
    fn input(&mut self, s: &mut Session, i: Input) -> Result<Step> {
        match (&self.cell, i) {
            (_, Input::Cancel) | (None, Input::Enter) | (Some(_), Input::Enter) => Ok(Step::Cancel),
            (None, Input::Point(p)) => {
                match cell_under(s, p) {
                    Some(c) => self.cell = Some(c),
                    None => s.echo("No table cell there."),
                }
                Ok(Step::Continue)
            }
            (Some((h, r, c)), Input::Text(name) | Input::Keyword(name)) => {
                let q = json!({ "handle": h.hex(), "row": r, "col": c, "block": name });
                match run_tinsert(s, &q) {
                    Ok(_) => Ok(Step::Done),
                    Err(EngineError::BadParams { msg, .. }) => {
                        s.echo(msg);
                        Ok(Step::Continue)
                    }
                    Err(e) => Err(e),
                }
            }
            _ => Ok(Step::Continue),
        }
    }
}

// ---------------------------------------------------------------- cell borders

/// Which edges of a range `edges` names: (top, bottom, left, right, inside horizontal, inside
/// vertical).
fn edge_set(v: Option<&Value>) -> std::result::Result<[bool; 6], String> {
    let names: Vec<String> = match v {
        None | Some(Value::Null) => vec!["all".into()],
        Some(Value::String(s)) => s.split(',').map(|x| x.trim().to_ascii_lowercase()).collect(),
        Some(Value::Array(a)) => a.iter().map(|x| x.as_str().unwrap_or("").trim().to_ascii_lowercase()).collect(),
        Some(v) => return Err(format!("invalid edges {v}")),
    };
    let mut out = [false; 6];
    for n in names {
        let set: &[usize] = match n.as_str() {
            "all" => &[0, 1, 2, 3, 4, 5],
            "outside" => &[0, 1, 2, 3],
            "inside" => &[4, 5],
            "top" => &[0],
            "bottom" => &[1],
            "left" => &[2],
            "right" => &[3],
            "insideh" => &[4],
            "insidev" => &[5],
            _ => return Err(format!("unknown edge {n} (all, outside, inside, top, bottom, left, right, insideH, insideV)")),
        };
        for i in set {
            if let Some(x) = out.get_mut(*i) {
                *x = true;
            }
        }
    }
    Ok(out)
}

fn run_borders(s: &mut Session, p: &Value) -> Result<Value> {
    let which = edge_set(p.get("edges")).map_err(|m| bad("table.cellborders", m))?;
    let clear = p.get("clear").and_then(Value::as_bool).unwrap_or(false);
    let mut border = CellBorder::default();
    if !clear {
        apply_border(&mut border, p).map_err(|m| bad("table.cellborders", m))?;
    }
    edit(s, p, "table.cellborders", |t| {
        let (r0, c0) = cell_at(p, t)?;
        let rows = usize_param(p, "rows").unwrap_or(1).max(1).min(t.cells.len() - r0);
        let cols = usize_param(p, "cols").unwrap_or(1).max(1).min(t.col_widths.len() - c0);
        let [top, bottom, left, right, inside_h, inside_v] = which;
        let value = (!clear).then_some(border);
        for r in r0..r0 + rows {
            for c in c0..c0 + cols {
                let (ar, ac) = anchor_of(t, r, c);
                let Some(cell) = t.cells.get_mut(ar).and_then(|row| row.get_mut(ac)) else { continue };
                let mut e = cell.edges.unwrap_or_default();
                if (r == r0 && top) || (r > r0 && inside_h) {
                    e.top = value;
                }
                if (r + 1 == r0 + rows && bottom) || (r + 1 < r0 + rows && inside_h) {
                    e.bottom = value;
                }
                if (c == c0 && left) || (c > c0 && inside_v) {
                    e.left = value;
                }
                if (c + 1 == c0 + cols && right) || (c + 1 < c0 + cols && inside_v) {
                    e.right = value;
                }
                cell.edges = (!e.is_empty()).then_some(e);
            }
        }
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edge_names() {
        assert_eq!(edge_set(None), Ok([true; 6]));
        assert_eq!(edge_set(Some(&json!("outside"))), Ok([true, true, true, true, false, false]));
        assert_eq!(edge_set(Some(&json!(["top", "insideV"]))), Ok([true, false, false, false, false, true]));
        assert!(edge_set(Some(&json!("diagonal"))).is_err());
        assert!(edge_set(Some(&json!(3))).is_err());
    }

    #[test]
    fn string_maps_are_bounded() {
        let mut big = serde_json::Map::new();
        for i in 0..=MAX_CELL_BLOCK_VALUES {
            big.insert(format!("T{i}"), json!(i));
        }
        assert!(string_map(&json!({ "attribs": big }), "attribs").is_err());
        let m = string_map(&json!({ "attribs": { "NUMBER": 5, " ": "x", "VAL": "a" } }), "attribs").unwrap_or_default();
        assert_eq!(m.get("NUMBER").map(String::as_str), Some("5"));
        assert_eq!(m.len(), 2);
        assert!(string_map(&json!({ "attribs": [1] }), "attribs").is_err());
    }
}
