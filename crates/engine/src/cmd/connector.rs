//! Connectors (see [`crate::connect`]): the named points of a block where wires attach.
//!
//! - CONNECTOR: place connectors (in the Block Editor, or in a drawing that becomes a block),
//!   a point and a name each, until Enter.
//! - `connector.set`: rename a connector point, make a point one, or (`remove`) make it a plain
//!   point again.
//! - `connector.list`: the connectors of block references, and those placed in the current space.

use cadcraft_doc::{EntityKind, Handle, Point};
use cadcraft_geom::Vec2;
use serde_json::{Value, json};

use super::helpers::v3;
use super::{CommandSpec, bad, point_param, targets};
use crate::connect::{self, MAX_NAME};
use crate::{Accept, Input, Interactive, Prompt, Result, Session, Step};

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec::new("connector", "Connector", run_add)
            .menu(&["Tools", "Wires", "Connector"])
            .alias(&["conn"])
            .params("{at: [x,y], name? (default: the next number)} → {handle, name}")
            .interactive(|_| Ok(Box::new(ConnectorM::default()))),
        CommandSpec::new("connector.set", "Set Connector", run_set)
            .params("{handle (a point), name?, remove? (true: a plain point again)} → {handle, name}"),
        CommandSpec::new("connector.list", "List Connectors", run_list)
            .params(
                "{handles? (block references; default: the selection, or every connector placed in the current space when nothing is selected)} \
                 → [{handle, block?, connectors: [{name, at}]}]",
            )
            .noundo(),
    ]
}

/// A connector name from user text: trimmed, at most [`MAX_NAME`] characters.
fn clean(name: &str) -> String {
    name.trim().chars().take(MAX_NAME).collect()
}

/// The connector names placed in the current space.
fn names_here(s: &Session) -> Vec<String> {
    let space = s.space();
    let Ok(d) = s.doc() else { return Vec::new() };
    d.space(&space).map(|st| st.iter().filter_map(|e| connect::connector_name(e)).collect()).unwrap_or_default()
}

/// The name the next connector gets: one more than the highest number used, else the count + 1.
fn next_name(s: &Session) -> String {
    let names = names_here(s);
    let top = names.iter().filter_map(|n| n.parse::<u64>().ok()).max();
    match top {
        Some(n) => n.saturating_add(1).to_string(),
        None => (names.len() + 1).to_string(),
    }
}

fn add(s: &mut Session, at: Vec2, name: &str) -> Result<Handle> {
    if !at.is_finite() {
        return Err(bad("connector", "the point must be finite"));
    }
    let h = s.add_entity(EntityKind::Point(Point { p: v3(at), angle: 0.0 }))?;
    let name = clean(name);
    s.doc_mut()?.modify_entity(h, |e| connect::set_connector_name(&mut e.common.xdata, &name))?;
    Ok(h)
}

fn run_add(s: &mut Session, p: &Value) -> Result<Value> {
    let at = point_param(p, "at").ok_or_else(|| bad("connector", "`at` is required"))?;
    let name = match p.get("name").and_then(Value::as_str).map(clean).filter(|n| !n.is_empty()) {
        Some(n) => n,
        None => next_name(s),
    };
    let h = add(s, at, &name)?;
    Ok(json!({ "handle": h.hex(), "name": name }))
}

fn run_set(s: &mut Session, p: &Value) -> Result<Value> {
    let h = p.get("handle").and_then(Value::as_str).and_then(Handle::parse_hex).ok_or_else(|| bad("connector.set", "`handle` is required"))?;
    let d = s.doc()?;
    let e = d.entity(h).ok_or_else(|| bad("connector.set", "no such object"))?;
    if !matches!(e.kind, EntityKind::Point(_)) {
        return Err(bad("connector.set", "a connector is a point"));
    }
    let current = connect::connector_name(e);
    if p.get("remove").and_then(Value::as_bool).unwrap_or(false) {
        s.doc_mut()?.modify_entity(h, |e| connect::clear_connector(&mut e.common.xdata))?;
        return Ok(json!({ "handle": h.hex(), "name": Value::Null }));
    }
    let name = match p.get("name").and_then(Value::as_str).map(clean).filter(|n| !n.is_empty()) {
        Some(n) => n,
        None => current.unwrap_or_else(|| next_name(s)),
    };
    s.doc_mut()?.modify_entity(h, |e| connect::set_connector_name(&mut e.common.xdata, &name))?;
    Ok(json!({ "handle": h.hex(), "name": name }))
}

fn run_list(s: &mut Session, p: &Value) -> Result<Value> {
    let hs = targets(s, p)?;
    let d = s.doc()?;
    let pt = |q: Vec2| json!([q.x, q.y]);
    if hs.is_empty() {
        let space = s.space();
        let here: Vec<Value> = d
            .space(&space)
            .map(|st| {
                st.iter()
                    .filter_map(|e| {
                        let name = connect::connector_name(e)?;
                        let EntityKind::Point(q) = &e.kind else { return None };
                        Some(json!({ "handle": e.handle.hex(), "connectors": [{ "name": name, "at": pt(q.p.xy()) }] }))
                    })
                    .collect()
            })
            .unwrap_or_default();
        return Ok(Value::Array(here));
    }
    let out: Vec<Value> = hs
        .iter()
        .filter_map(|h| {
            let e = d.entity(*h)?;
            let EntityKind::Insert(ins) = &e.kind else { return None };
            let cs: Vec<Value> = connect::connectors_of(d, e).into_iter().map(|(n, q)| json!({ "name": n, "at": pt(q) })).collect();
            Some(json!({ "handle": h.hex(), "block": ins.block, "connectors": cs }))
        })
        .collect();
    Ok(Value::Array(out))
}

/// What a wire end is attached to, for `wire.get` and the Properties palette: the block
/// reference, its block's name and the connector.
pub fn link_json(d: &cadcraft_doc::Drawing, l: &connect::Link) -> Value {
    let block = d.entity(l.insert).and_then(|e| match &e.kind {
        EntityKind::Insert(i) => Some(i.block.clone()),
        _ => None,
    });
    json!({ "handle": l.insert.hex(), "block": block, "name": l.name })
}

/// CONNECTOR: a point, then its name; again until Enter.
#[derive(Default)]
struct ConnectorM {
    at: Option<Vec2>,
}

impl Interactive for ConnectorM {
    fn name(&self) -> &'static str {
        "CONNECTOR"
    }
    fn prompt(&self, s: &Session) -> Prompt {
        match self.at {
            None => Prompt::new("Specify connector point", Accept::POINT),
            Some(_) => Prompt::new("Enter connector name", Accept::TEXT).default(next_name(s)),
        }
    }
    fn input(&mut self, s: &mut Session, i: Input) -> Result<Step> {
        match (self.at, i) {
            (None, Input::Point(p)) => {
                self.at = Some(p);
                Ok(Step::Continue)
            }
            (None, Input::Enter) => Ok(Step::Done),
            (Some(at), Input::Text(t) | Input::Keyword(t)) if !clean(&t).is_empty() => {
                let name = clean(&t);
                add(s, at, &name)?;
                s.echo(format!("Connector {name} placed."));
                self.at = None;
                Ok(Step::Continue)
            }
            (Some(at), Input::Enter | Input::Text(_)) => {
                let name = next_name(s);
                add(s, at, &name)?;
                s.echo(format!("Connector {name} placed."));
                self.at = None;
                Ok(Step::Continue)
            }
            _ => Ok(Step::Continue),
        }
    }
}
