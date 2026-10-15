//! Wires: polylines (or lines) carrying a wire list's data, kept as wire-list add-ins keep it in
//! AutoCAD so drawings go back and forth: XRECORDs `L` (length, `;` between the sections of a
//! chain), `c1` and `c2` (the terminals at its ends) and `N` (its number) in the object's
//! extension dictionary; the wire type is the layer, its colour the object's colour.
//!
//! - `wire.set` / `wire.get`: the data of the given (or selected) wires.
//! - WIRE: select wires, then their length, or the other fields by keyword (the Properties
//!   palette's Wire section edits the same).
//! - WIRENUM: number wires by picking them one after another.
//! - `wire.choices`: what the Properties palette offers — the `wires.json` lists (terminals,
//!   types, colours, lengths) found where reports are looked for, and the values the drawing's
//!   wires already have.

use cadcraft_color::Color;
use cadcraft_doc::{EntityKind, Handle};
use serde_json::{Value, json};

use super::machines::{SelOutcome, SelectPhase};
use super::{CommandSpec, bad, targets};
use crate::{Accept, Input, Interactive, Prompt, Result, Session, Step};

/// The XRECORD keys, as the add-ins write them.
pub const LENGTH: &str = "L";
pub const START: &str = "c1";
pub const END: &str = "c2";
pub const NUMBER: &str = "N";

/// Longest value a field takes.
const MAX_VALUE: usize = 1000;
/// Most entries of each `wires.json` list.
const MAX_CHOICES: usize = 1000;
/// The settings file with the lists.
pub const SETTINGS: &str = "wires.json";

const SET_PARAMS: &str = "{handles? (default: the selection), length? (\"40;35\": a chain's sections), c1?, c2? (terminal codes), number?, \
     type? (the layer; made when missing), color? (ACI number or name)} → {wires}";

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec::new("wire", "Wire Data...", run_set)
            .menu(&["Tools", "Wires", "Wire Data..."])
            .alias(&["wd"])
            .params(SET_PARAMS)
            .interactive(|_| Ok(Box::new(WireM::default()))),
        CommandSpec::new("wire.set", "Set Wire Data", run_set).params(SET_PARAMS),
        CommandSpec::new("wire.get", "Wire Data", run_get)
            .params("{handles? (default: the selection)} → [{handle, length, c1, c2, number, type, color}]")
            .noundo(),
        CommandSpec::new("wire.choices", "Wire Data Choices", run_choices)
            .params(
                "{} → {terminals, types, colors: [{aci, name}], lengths, file?} (wires.json where reports are looked for, and the drawing's values)",
            )
            .noundo(),
        CommandSpec::new("wirenum", "Number Wires", run_number)
            .menu(&["Tools", "Wires", "Number Wires"])
            .alias(&["wn"])
            .params("{handles (in order), start? (default: the next free number)} → {last}")
            .interactive(|s| Ok(Box::new(NumberM::new(s)))),
    ]
}

/// Whether `k` can be a wire.
fn is_wire(k: &EntityKind) -> bool {
    matches!(k, EntityKind::LwPolyline(_) | EntityKind::Line(_) | EntityKind::Polyline3d(_))
}

/// One wire's data.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct WireData {
    pub length: String,
    pub c1: String,
    pub c2: String,
    pub number: String,
    pub layer: String,
    pub color: Color,
}

/// Most sections of a chain.
pub const MAX_SECTIONS: usize = 100;

/// A wire as a chain: its nodes (the terminal at each) and the length of each section between
/// them — one section for a plain wire. Kept as the add-ins keep it: `L` the lengths, `c1` the
/// node at the start of each section, `c2` the last node, all `;`-separated.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Chain {
    /// Sections + 1 nodes.
    pub nodes: Vec<String>,
    pub lengths: Vec<String>,
}

impl Chain {
    pub fn of(w: &WireData) -> Chain {
        let split = |s: &str| -> Vec<String> { s.split(';').take(MAX_SECTIONS + 1).map(|x| x.trim().to_string()).collect() };
        let lengths = split(&w.length);
        let n = lengths.len().max(1);
        let mut nodes: Vec<String> = split(&w.c1);
        nodes.resize(n, String::new());
        nodes.push(w.c2.trim().to_string());
        Chain { nodes, lengths }
    }

    pub fn sections(&self) -> usize {
        self.lengths.len()
    }

    /// The `wire.set` fields: length, c1, c2.
    pub fn fields(&self) -> (String, String, String) {
        let n = self.lengths.len();
        let starts: Vec<&str> = self.nodes.iter().take(n).map(String::as_str).collect();
        (self.lengths.join(";"), starts.join(";"), self.nodes.get(n).cloned().unwrap_or_default())
    }

    /// One more section at the end, like the last one (its length, its end terminal).
    pub fn add(&mut self) {
        if self.lengths.len() >= MAX_SECTIONS {
            return;
        }
        let len = self.lengths.last().cloned().unwrap_or_default();
        let end = self.nodes.last().cloned().unwrap_or_default();
        self.lengths.push(len);
        self.nodes.push(end);
    }

    /// The last section off (a wire keeps one).
    pub fn remove(&mut self) {
        if self.lengths.len() > 1 {
            self.lengths.pop();
            self.nodes.pop();
        }
    }
}

/// The data of wire `h` (`None`: not a wire).
pub fn data(d: &cadcraft_doc::Drawing, h: Handle) -> Option<WireData> {
    let e = d.entity(h).filter(|e| is_wire(&e.kind))?;
    let x = |k: &str| d.xrecord_text(h, k).unwrap_or_default();
    Some(WireData { length: x(LENGTH), c1: x(START), c2: x(END), number: x(NUMBER), layer: e.common.layer.clone(), color: e.common.color })
}

/// A text field of `p`: text, or a number as typed.
fn field(p: &Value, key: &str) -> Option<String> {
    let v = p.get(key)?;
    let t = match v {
        Value::String(s) => s.trim().to_string(),
        Value::Number(n) => n.to_string(),
        _ => return None,
    };
    Some(t.chars().take(MAX_VALUE).collect())
}

/// A colour by ACI number, AutoCAD name or a `wires.json` colour name.
fn color_of(s: &Session, v: &Value) -> Option<Color> {
    if let Some(i) = v.as_i64() {
        return i16::try_from(i).ok().filter(|i| (0..=256).contains(i)).map(Color::from_aci);
    }
    let t = v.as_str()?.trim();
    if let Some(c) = choices(s).colors.iter().find(|(_, n)| n.eq_ignore_ascii_case(t) || n.to_lowercase() == t.to_lowercase()) {
        return Some(Color::from_aci(c.0));
    }
    Color::parse(t)
}

fn run_set(s: &mut Session, p: &Value) -> Result<Value> {
    let id = "wire.set";
    let hs = targets(s, p)?;
    let color = match p.get("color") {
        None | Some(Value::Null) => None,
        Some(v) => Some(color_of(s, v).ok_or_else(|| bad(id, format!("unknown colour {v}")))?),
    };
    let layer = field(p, "type").filter(|l| !l.is_empty());
    let fields: Vec<(&str, String)> = [(LENGTH, "length"), (START, "c1"), (END, "c2"), (NUMBER, "number")]
        .into_iter()
        .filter_map(|(k, name)| field(p, name).map(|v| (k, v)))
        .collect();
    let d = s.doc()?;
    let wires: Vec<Handle> = hs.into_iter().filter(|h| d.entity(*h).is_some_and(|e| is_wire(&e.kind))).collect();
    if wires.is_empty() {
        return Err(bad(id, "no wires: select polylines or lines"));
    }
    if wires.iter().any(|h| super::curves::is_locked(s, *h)) {
        return Err(bad(id, "a wire is on a locked layer"));
    }
    let d = s.doc_mut()?;
    if let Some(l) = &layer {
        d.ensure_layer(l);
    }
    for h in &wires {
        for (k, v) in &fields {
            d.set_xrecord_text(*h, k, v);
        }
        if layer.is_some() || color.is_some() {
            d.modify_entity(*h, |e| {
                if let Some(l) = &layer {
                    e.common.layer = l.clone();
                }
                if let Some(c) = color {
                    e.common.color = c;
                }
            })?;
        }
    }
    Ok(json!({ "wires": wires.len() }))
}

fn run_get(s: &mut Session, p: &Value) -> Result<Value> {
    let hs = targets(s, p)?;
    let d = s.doc()?;
    let out: Vec<Value> = hs
        .iter()
        .filter_map(|h| {
            let w = data(d, *h)?;
            Some(json!({
                "handle": h.hex(), "length": w.length, "c1": w.c1, "c2": w.c2, "number": w.number,
                "type": w.layer, "color": w.color.to_aci(),
            }))
        })
        .collect();
    Ok(Value::Array(out))
}

/// What the wire fields offer to pick from.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Choices {
    pub terminals: Vec<String>,
    pub types: Vec<String>,
    /// (ACI, name).
    pub colors: Vec<(i16, String)>,
    pub lengths: Vec<String>,
    /// The `wires.json` they came from.
    pub file: Option<String>,
}

impl Choices {
    /// The name `wires.json` gives colour `c`, else AutoCAD's.
    pub fn color_name(&self, c: Color) -> String {
        let aci = c.to_aci();
        self.colors.iter().find(|(i, _)| *i == aci).map(|(_, n)| n.clone()).unwrap_or_else(|| c.name())
    }
}

fn push_unique(v: &mut Vec<String>, t: &str) {
    let t = t.trim();
    if !t.is_empty() && v.len() < MAX_CHOICES && !v.iter().any(|x| x.eq_ignore_ascii_case(t)) {
        v.push(t.to_string());
    }
}

/// The lists of the first `wires.json` where reports are looked for, then the values of the
/// drawing's wires.
pub fn choices(s: &Session) -> Choices {
    let mut c = Choices::default();
    #[cfg(not(target_arch = "wasm32"))]
    for dir in super::table::search_dirs(s) {
        let path = dir.join(SETTINGS);
        let Ok(meta) = std::fs::metadata(&path) else { continue };
        if meta.len() > 4 << 20 {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&path) else { continue };
        let Ok(v) = serde_json::from_str::<Value>(text.trim_start_matches('\u{feff}')) else { continue };
        let list = |k: &str| -> Vec<String> {
            v.get(k)
                .and_then(Value::as_array)
                .map(|a| a.iter().take(MAX_CHOICES).filter_map(|x| x.as_str().map(str::to_string)).collect())
                .unwrap_or_default()
        };
        for t in list("terminals") {
            push_unique(&mut c.terminals, &t);
        }
        for t in list("types") {
            push_unique(&mut c.types, &t);
        }
        for t in list("lengths") {
            push_unique(&mut c.lengths, &t);
        }
        if let Some(a) = v.get("colors").and_then(Value::as_array) {
            for x in a.iter().take(MAX_CHOICES) {
                let aci = x.get("aci").and_then(Value::as_i64).and_then(|i| i16::try_from(i).ok()).filter(|i| (0..=256).contains(i));
                let name = x.get("name").and_then(Value::as_str).map(str::trim).filter(|n| !n.is_empty());
                if let (Some(aci), Some(name)) = (aci, name)
                    && !c.colors.iter().any(|(i, _)| *i == aci)
                {
                    c.colors.push((aci, name.to_string()));
                }
            }
        }
        c.file = Some(path.to_string_lossy().into_owned());
        break;
    }
    if let Ok(d) = s.doc() {
        let index = d.foreign.object_index();
        for e in d.model.iter().take(200_000) {
            if !is_wire(&e.kind) || !d.foreign.entities.get(&e.handle).is_some_and(|x| x.xdict.is_some()) {
                continue;
            }
            let recs = d.foreign.xrecords(&index, e.handle);
            let get = |k: &str| recs.iter().find(|(n, _)| n.eq_ignore_ascii_case(k)).and_then(|(_, t)| t.first()).map(|t| t.value.clone());
            for k in [START, END] {
                if let Some(v) = get(k) {
                    for part in v.split(';') {
                        push_unique(&mut c.terminals, part);
                    }
                }
            }
            if get(LENGTH).is_some() {
                push_unique(&mut c.types, &e.common.layer);
            }
        }
    }
    c
}

fn run_choices(s: &mut Session, _p: &Value) -> Result<Value> {
    let c = choices(s);
    Ok(json!({
        "terminals": c.terminals, "types": c.types, "lengths": c.lengths, "file": c.file,
        "colors": c.colors.iter().map(|(a, n)| json!({ "aci": a, "name": n })).collect::<Vec<_>>(),
    }))
}

/// The number after the highest one the drawing's wires have.
fn next_number(s: &Session) -> u64 {
    let Ok(d) = s.doc() else { return 1 };
    let index = d.foreign.object_index();
    let mut top = 0u64;
    for e in d.model.iter().take(200_000).filter(|e| is_wire(&e.kind)) {
        if let Some(n) = d
            .foreign
            .xrecords(&index, e.handle)
            .into_iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(NUMBER))
            .and_then(|(_, t)| t.first().and_then(|t| t.value.trim().parse::<u64>().ok()))
        {
            top = top.max(n);
        }
    }
    top.saturating_add(1)
}

fn run_number(s: &mut Session, p: &Value) -> Result<Value> {
    let hs = targets(s, p)?;
    let mut n = p.get("start").and_then(Value::as_u64).unwrap_or_else(|| next_number(s));
    let d = s.doc()?;
    let wires: Vec<Handle> = hs.into_iter().filter(|h| d.entity(*h).is_some_and(|e| is_wire(&e.kind))).collect();
    if wires.is_empty() {
        return Err(bad("wirenum", "no wires: select polylines or lines"));
    }
    let d = s.doc_mut()?;
    for h in &wires {
        d.set_xrecord_text(*h, NUMBER, &n.to_string());
        n = n.saturating_add(1);
    }
    Ok(json!({ "last": n.saturating_sub(1) }))
}

// ---------------- command line ----------------

#[derive(Clone, Copy, Debug, Default, PartialEq)]
enum Field {
    #[default]
    Length,
    Start,
    End,
    Number,
    Type,
    Color,
}

impl Field {
    fn key(self) -> &'static str {
        match self {
            Field::Length => "length",
            Field::Start => "c1",
            Field::End => "c2",
            Field::Number => "number",
            Field::Type => "type",
            Field::Color => "color",
        }
    }
    fn message(self) -> &'static str {
        match self {
            Field::Length => "Enter wire length",
            Field::Start => "Enter terminal at the start",
            Field::End => "Enter terminal at the end",
            Field::Number => "Enter wire number",
            Field::Type => "Enter wire type",
            Field::Color => "Enter wire color",
        }
    }
}

/// WIRE at the command line: select wires (or have them selected), then "Enter wire length or
/// [Start/End/Number/Type/Color]": a value sets the field, a keyword asks for that field; Enter
/// ends.
#[derive(Default)]
struct WireM {
    sel: SelectPhase,
    wires: Vec<Handle>,
    /// The field asked for (`None`: the main prompt).
    asking: Option<Field>,
}

impl WireM {
    /// The value all the wires share for `f` (empty when they differ).
    fn current(&self, s: &Session, f: Field) -> String {
        let Ok(d) = s.doc() else { return String::new() };
        let c = choices(s);
        let vals: Vec<String> = self
            .wires
            .iter()
            .filter_map(|h| data(d, *h))
            .map(|w| match f {
                Field::Length => w.length,
                Field::Start => w.c1,
                Field::End => w.c2,
                Field::Number => w.number,
                Field::Type => w.layer,
                Field::Color => c.color_name(w.color),
            })
            .collect();
        match vals.first() {
            Some(v) if vals.iter().all(|x| x == v) => v.clone(),
            _ => String::new(),
        }
    }

    fn set(&mut self, s: &mut Session, f: Field, v: &str) -> Result<()> {
        let hs: Vec<String> = self.wires.iter().map(|h| h.hex()).collect();
        let mut p = json!({ "handles": hs });
        p[f.key()] = json!(v);
        run_set(s, &p)?;
        Ok(())
    }
}

impl Interactive for WireM {
    fn name(&self) -> &'static str {
        "WIRE"
    }
    fn begin(&mut self, s: &mut Session) -> Result<Step> {
        self.sel = SelectPhase::begin(s);
        if self.sel.done {
            self.wires = self.sel.picked.clone();
        }
        Ok(Step::Continue)
    }
    fn prompt(&self, s: &Session) -> Prompt {
        if self.wires.is_empty() {
            return if self.sel.removing { self.sel.prompt() } else { Prompt::new("Select wires", Accept::SELECT) };
        }
        let f = self.asking.unwrap_or(Field::Length);
        let p = Prompt::new(if self.asking.is_none() { "Enter wire length or" } else { f.message() }, Accept::TEXT);
        let p = if self.asking.is_none() { p.kw(&["Start", "End", "Number", "Type", "Color"]) } else { p };
        let cur = self.current(s, f);
        if cur.is_empty() { p } else { p.default(cur) }
    }
    fn input(&mut self, s: &mut Session, i: Input) -> Result<Step> {
        if matches!(i, Input::Cancel) {
            return Ok(Step::Cancel);
        }
        if self.wires.is_empty() {
            return match self.sel.feed(s, &i)? {
                SelOutcome::More => Ok(Step::Continue),
                SelOutcome::Empty => Ok(Step::Cancel),
                SelOutcome::Done(hs) => {
                    let d = s.doc()?;
                    self.wires = hs.into_iter().filter(|h| d.entity(*h).is_some_and(|e| is_wire(&e.kind))).collect();
                    if self.wires.is_empty() {
                        s.echo("No wires selected: wires are polylines or lines.");
                        return Ok(Step::Cancel);
                    }
                    Ok(Step::Continue)
                }
            };
        }
        let keyword = |t: &str| match t.trim().to_lowercase().as_str() {
            "start" | "s" => Some(Field::Start),
            "end" | "e" => Some(Field::End),
            "number" | "n" => Some(Field::Number),
            "type" | "t" => Some(Field::Type),
            "color" | "c" => Some(Field::Color),
            _ => None,
        };
        match (self.asking, i) {
            (None, Input::Enter) => Ok(Step::Done),
            (Some(_), Input::Enter) => {
                self.asking = None;
                Ok(Step::Continue)
            }
            (None, Input::Keyword(t)) | (None, Input::Text(t)) if keyword(&t).is_some() => {
                self.asking = keyword(&t);
                Ok(Step::Continue)
            }
            (asking, Input::Text(t) | Input::Keyword(t)) => {
                let f = asking.unwrap_or(Field::Length);
                if let Err(e) = self.set(s, f, &t) {
                    s.echo(e.to_string());
                }
                self.asking = None;
                Ok(Step::Continue)
            }
            _ => Ok(Step::Continue),
        }
    }
}

/// WIRENUM at the command line: the first number (default: the next free one), then pick the
/// wires one after another; each gets the next number. Enter ends.
struct NumberM {
    next: Option<u64>,
    default: u64,
}

impl NumberM {
    fn new(s: &Session) -> Self {
        NumberM { next: None, default: next_number(s) }
    }
}

impl Interactive for NumberM {
    fn name(&self) -> &'static str {
        "WIRENUM"
    }
    fn prompt(&self, _s: &Session) -> Prompt {
        match self.next {
            None => Prompt::new("Enter first wire number", Accept::NUMBER).default(self.default.to_string()),
            Some(_) => Prompt::new("Select wire", Accept::SELECT),
        }
    }
    fn input(&mut self, s: &mut Session, i: Input) -> Result<Step> {
        match (self.next, i) {
            (_, Input::Cancel) => Ok(Step::Done),
            (None, Input::Enter) => {
                self.next = Some(self.default);
                s.echo(format!("Next number: {}", self.default));
                Ok(Step::Continue)
            }
            (None, Input::Text(t)) => {
                match t.trim().parse::<u64>() {
                    Ok(n) => {
                        self.next = Some(n);
                        s.echo(format!("Next number: {n}"));
                    }
                    Err(_) => s.echo("Requires a whole number."),
                }
                Ok(Step::Continue)
            }
            (Some(_), Input::Enter) => Ok(Step::Done),
            (Some(n), Input::Pick(hs)) => {
                let d = s.doc()?;
                let wires: Vec<Handle> = hs.into_iter().filter(|h| d.entity(*h).is_some_and(|e| is_wire(&e.kind))).collect();
                if wires.is_empty() {
                    s.echo("Not a wire.");
                    return Ok(Step::Continue);
                }
                let r = run_number(s, &json!({ "handles": wires.iter().map(|h| h.hex()).collect::<Vec<_>>(), "start": n }))?;
                let last = r["last"].as_u64().unwrap_or(n);
                s.echo(if last == n { format!("Wire {n}.") } else { format!("Wires {n}–{last}.") });
                s.set_selection(Vec::new());
                self.next = Some(last.saturating_add(1));
                Ok(Step::Continue)
            }
            _ => Ok(Step::Continue),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn w(length: &str, c1: &str, c2: &str) -> WireData {
        WireData { length: length.into(), c1: c1.into(), c2: c2.into(), ..Default::default() }
    }

    #[test]
    fn chains_read_and_write_the_add_in_fields() {
        let c = Chain::of(&w("7;7;7", "В1;В1;В1", "В1"));
        assert_eq!((c.nodes.len(), c.sections()), (4, 3));
        assert_eq!(c.fields(), ("7;7;7".into(), "В1;В1;В1".into(), "В1".into()));
        // A plain wire is one section.
        let mut p = Chain::of(&w("60", "Н1И", "В1"));
        assert_eq!(p.nodes, ["Н1И", "В1"]);
        p.add();
        assert_eq!(p.fields(), ("60;60".into(), "Н1И;В1".into(), "В1".into()));
        p.remove();
        p.remove();
        assert_eq!(p.fields(), ("60".into(), "Н1И".into(), "В1".into()));
        // Too few starts: padded with bare nodes.
        assert_eq!(Chain::of(&w("19;16", "Z12", "Н1И")).nodes, ["Z12", "", "Н1И"]);
        assert_eq!(Chain::of(&w("", "", "")).fields(), (String::new(), String::new(), String::new()));
    }
}
