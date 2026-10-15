//! Running a report: the drawing's objects become records, records are filtered, grouped and
//! sorted, and each group becomes a row through the column templates.
//!
//! Templates hold `{…}` references:
//! - `{field}`: the field's value (in a group, the first record's that has one);
//! - `{#}`: the row number, counted on through the sections;
//! - `{count}`: the records in the row; `{count:field}` those with a value of `field`;
//! - `{sum:field}`, `{min:field}`, `{max:field}`: numbers (a `;` list counts as its sum);
//! - `{join:field}`: the different values, joined with ", ";
//! - a last `:N` rounds a number to N decimals (`{sum:length:0}`);
//! - `{{` and `}}` are braces.
//!
//! Fields of an object: `type` (its DXF name), `name` (a block reference's block, the dynamic
//! block's own name for its anonymous copies), `layer`, `color` (the ACI number; 256 ByLayer,
//! 0 ByBlock), `linetype`, `handle`, `length`, `area`, `att.TAG` (attribute values), `xr.KEY`
//! (the first value of the XRECORD `KEY` in its extension dictionary), `xd.APP` (the texts of
//! application `APP`'s xdata, joined with `;`), `dyn.NAME` (a dynamic block property, e.g.
//! `dyn.Visibility`), `item` and `index` (with `each`), and any property the Properties
//! palette shows (`radius`, `rotation`…). A section's `fields` come first.

use std::cmp::Ordering;
use std::collections::{BTreeMap, HashMap};

use cadcraft_doc::foreign::ForeignObject;
use cadcraft_doc::{Drawing, Entity, EntityKind, Handle};
use serde_json::Value;

use super::def::{Field, Report, RowCell, Section};
use crate::cmd::qselect::{properties, wildcard};

/// Most records a report reads (objects times their `each` items).
pub const MAX_RECORDS: usize = 500_000;
/// How deep fields may refer to fields.
const MAX_DEPTH: usize = 16;
/// Most parts of a `;` list summed as a number.
const MAX_LIST: usize = 1000;

/// Lookup tables by name (lower case): rows of column (lower case) → value.
pub type Lookups = BTreeMap<String, Vec<BTreeMap<String, String>>>;

/// A row the report makes.
#[derive(Clone, Debug, PartialEq)]
pub enum Row {
    /// A section's title, across the table.
    Title(String),
    /// A cell per column.
    Data(Vec<Out>),
}

/// A cell the report fills: its text, or a wire sketch (then the text sums it up).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Out {
    pub text: String,
    pub sketch: Option<super::sketch::Sketch>,
    /// A block named for the cell instead of the drawn sketch.
    pub block: Option<cadcraft_doc::CellBlock>,
}

/// One record: an object and, with `each`, one of its items (text, 1-based index).
struct Rec {
    obj: usize,
    item: Option<(String, usize)>,
    memo: HashMap<String, String>,
}

/// What a section's records are evaluated with.
struct Ctx<'a> {
    d: &'a Drawing,
    objs: &'a [&'a Entity],
    xr: &'a HashMap<Handle, &'a ForeignObject>,
    fields: &'a BTreeMap<String, Field>,
    lookups: &'a Lookups,
}

/// The text of a number: up to 4 decimals (or `decimals`), no trailing zeros, never `-0`.
pub fn fmt_num(v: f64, decimals: Option<usize>) -> String {
    let t = format!("{v:.*}", decimals.unwrap_or(4).min(12));
    let t = if decimals.is_none() && t.contains('.') { t.trim_end_matches('0').trim_end_matches('.').to_string() } else { t };
    if t.trim_start_matches('-').chars().all(|c| c == '0' || c == '.') { t.trim_start_matches('-').to_string() } else { t }
}

/// A value as a number: decimal comma or point; a `;` list counts as its sum.
pub fn num(s: &str) -> Option<f64> {
    let parse = |p: &str| p.trim().replace(',', ".").parse::<f64>().ok().filter(|v| v.is_finite());
    if s.contains(';') {
        let mut sum = 0.0;
        for p in s.split(';').take(MAX_LIST).filter(|p| !p.trim().is_empty()) {
            sum += parse(p)?;
        }
        return Some(sum).filter(|v: &f64| v.is_finite());
    }
    parse(s)
}

/// The text of an XRECORD or xdata group worth showing.
fn tag_text(t: &cadcraft_doc::RawTag) -> Option<String> {
    matches!(t.code, 1..=9 | 10..=59 | 60..=99 | 140..=149 | 170..=179 | 270..=289 | 300..=309 | 1000 | 1040..=1071)
        .then(|| t.value.trim().to_string())
}

/// A block reference's block name: the dynamic block's for its anonymous copies.
fn block_name(d: &Drawing, e: &Entity) -> String {
    let EntityKind::Insert(ins) = &e.kind else { return String::new() };
    if let Some(r) = cadcraft_doc::dynblock::reference(d, e.handle) {
        return r.definition;
    }
    d.block(&ins.block).map(|b| b.name.clone()).unwrap_or_else(|| ins.block.clone())
}

impl Ctx<'_> {
    /// The value of field `name` (lower case) for record `r`.
    fn value(&self, r: &mut Rec, name: &str, depth: usize) -> String {
        if let Some(v) = r.memo.get(name) {
            return v.clone();
        }
        let v = if let Some(f) = self.fields.get(name).filter(|_| depth < MAX_DEPTH) {
            match f {
                Field::Template(t) => self.record_text(r, t, depth + 1),
                Field::Lookup { table, matches, get, default } => {
                    let wanted: Vec<(&String, String)> = matches.iter().map(|(col, t)| (col, self.record_text(r, t, depth + 1))).collect();
                    let hit = self
                        .lookups
                        .get(table)
                        .and_then(|rows| rows.iter().find(|row| wanted.iter().all(|(col, v)| row.get(*col).is_none_or(|x| row_matches(x, v)))))
                        .and_then(|row| row.get(get).cloned());
                    // No row: the default, a template too (`{layer}`).
                    match hit {
                        Some(v) => v,
                        None => self.record_text(r, default, depth + 1),
                    }
                }
            }
        } else {
            self.builtin(r, name)
        };
        r.memo.insert(name.to_string(), v.clone());
        v
    }

    fn builtin(&self, r: &Rec, name: &str) -> String {
        let Some(e) = self.objs.get(r.obj) else { return String::new() };
        let key_of = |prefix: &str| name.strip_prefix(prefix).map(str::trim).filter(|k| !k.is_empty());
        match name {
            "item" => return r.item.as_ref().map(|i| i.0.clone()).unwrap_or_default(),
            "index" => return r.item.as_ref().map(|i| i.1.to_string()).unwrap_or_default(),
            "type" => return e.kind.dxf_name().to_string(),
            "name" | "block" => return block_name(self.d, e),
            "layer" => return e.common.layer.clone(),
            "color" => return e.common.color.to_aci().to_string(),
            "linetype" => return e.common.linetype.clone(),
            "handle" => return e.handle.hex(),
            _ => {}
        }
        if let Some(tag) = key_of("att.") {
            let EntityKind::Insert(ins) = &e.kind else { return String::new() };
            if let Some(a) = ins.attribs.iter().find(|a| a.tag.eq_ignore_ascii_case(tag)) {
                return a.text.value.clone();
            }
            // A constant attribute lives in the block definition.
            return self
                .d
                .block(&ins.block)
                .and_then(|b| {
                    b.entities.iter().find_map(|x| match &x.kind {
                        EntityKind::AttDef(a) if a.constant && a.tag.eq_ignore_ascii_case(tag) => Some(a.text.value.clone()),
                        _ => None,
                    })
                })
                .unwrap_or_default();
        }
        if let Some(key) = key_of("xr.") {
            return self
                .d
                .foreign
                .xrecords(self.xr, e.handle)
                .into_iter()
                .find(|(k, _)| k.eq_ignore_ascii_case(key))
                .and_then(|(_, tags)| tags.iter().find_map(tag_text))
                .unwrap_or_default();
        }
        if let Some(app) = key_of("xd.") {
            let mut out = Vec::new();
            let mut on = false;
            for t in &e.common.xdata {
                if t.code == 1001 {
                    on = t.value.trim().eq_ignore_ascii_case(app);
                } else if on && let Some(v) = tag_text(t) {
                    out.push(v);
                }
            }
            return out.join(";");
        }
        if let Some(prop) = key_of("dyn.") {
            return cadcraft_doc::dynblock::custom_properties(self.d, e.handle)
                .and_then(|(_, ps)| {
                    // `dyn.Visibility` also finds "Visibility1" (the default label).
                    let i = ps.iter().position(|p| p.name.eq_ignore_ascii_case(prop)).or_else(|| {
                        let pre = prop.to_lowercase();
                        ps.iter().position(|p| p.name.to_lowercase().starts_with(&pre))
                    })?;
                    ps.into_iter().nth(i).map(|p| p.value)
                })
                .unwrap_or_default();
        }
        // Anything the Properties palette shows.
        let want: String = name.chars().filter(|c| c.is_alphanumeric()).collect();
        properties(self.d, e)
            .into_iter()
            .find(|(k, _)| k.chars().filter(|c| c.is_alphanumeric()).flat_map(char::to_lowercase).collect::<String>() == want)
            .map(|(_, v)| match v {
                Value::String(s) => s,
                Value::Number(n) => n.as_f64().map(|x| fmt_num(x, None)).unwrap_or_else(|| n.to_string()),
                other => other.to_string(),
            })
            .unwrap_or_default()
    }

    /// A template for one record (aggregates see just that record).
    fn record_text(&self, r: &mut Rec, t: &str, depth: usize) -> String {
        let mut out = String::new();
        for p in parse(t) {
            match p {
                Part::Lit(s) => out.push_str(&s),
                Part::Number => {}
                Part::Ref { agg: Agg::Count, field, .. } => {
                    out.push_str(if field.is_empty() || !self.value(r, &field, depth).is_empty() { "1" } else { "0" })
                }
                Part::Ref { field, decimals, .. } => {
                    let v = self.value(r, &field, depth);
                    out.push_str(&rounded(&v, decimals));
                }
            }
        }
        out
    }

    /// A template for a group of records, with its row number.
    fn group_text(&self, recs: &mut [Rec], group: &[usize], t: &str, number: usize) -> String {
        let mut out = String::new();
        for p in parse(t) {
            match p {
                Part::Lit(s) => out.push_str(&s),
                Part::Number => out.push_str(&number.to_string()),
                Part::Ref { agg, field, decimals } => {
                    let mut vals = Vec::new();
                    if !(agg == Agg::Count && field.is_empty()) {
                        for &i in group {
                            if let Some(r) = recs.get_mut(i) {
                                vals.push(self.value(r, &field, 0));
                            }
                        }
                    }
                    out.push_str(&aggregate(agg, &vals, group.len(), decimals));
                }
            }
        }
        out
    }
}

/// Lookup values match when equal as text (any case, trimmed) or as numbers ("02" = "2").
/// A lookup row's value against the wanted one: empty or `*` is any value, a value with `*` or
/// `?` a pattern, else [`same`].
fn row_matches(row: &str, v: &str) -> bool {
    let r = row.trim();
    if r.is_empty() || r == "*" {
        return true;
    }
    if r.contains(['*', '?']) {
        return wildcard(r, v.trim());
    }
    same(r, v)
}

fn same(a: &str, b: &str) -> bool {
    let (a, b) = (a.trim(), b.trim());
    a.eq_ignore_ascii_case(b)
        || a.to_lowercase() == b.to_lowercase()
        || matches!((num(a), num(b)), (Some(x), Some(y)) if !a.contains(';') && !b.contains(';') && x == y)
}

fn rounded(v: &str, decimals: Option<usize>) -> String {
    match (decimals, num(v)) {
        (Some(_), Some(x)) if !v.contains(';') => fmt_num(x, decimals),
        _ => v.to_string(),
    }
}

fn aggregate(agg: Agg, vals: &[String], records: usize, decimals: Option<usize>) -> String {
    let nonempty = || vals.iter().filter(|v| !v.trim().is_empty());
    match agg {
        Agg::First => nonempty().next().map(|v| rounded(v, decimals)).unwrap_or_default(),
        Agg::Count if vals.is_empty() => records.to_string(),
        Agg::Count => nonempty().count().to_string(),
        Agg::Sum => fmt_num(nonempty().filter_map(|v| num(v)).sum(), decimals),
        Agg::Min | Agg::Max => {
            let nums: Vec<f64> = nonempty().filter_map(|v| num(v)).collect();
            let pick = |a: f64, b: f64| if agg == Agg::Min { a.min(b) } else { a.max(b) };
            nums.iter().copied().reduce(pick).map(|v| fmt_num(v, decimals)).unwrap_or_default()
        }
        Agg::Join => {
            let mut seen: Vec<&str> = Vec::new();
            for v in nonempty() {
                if !seen.contains(&v.as_str()) {
                    seen.push(v);
                }
            }
            seen.join(", ")
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Agg {
    First,
    Count,
    Sum,
    Min,
    Max,
    Join,
}

#[derive(Clone, Debug, PartialEq)]
enum Part {
    Lit(String),
    Number,
    Ref { agg: Agg, field: String, decimals: Option<usize> },
}

/// A template's parts.
fn parse(t: &str) -> Vec<Part> {
    let mut out = Vec::new();
    let mut lit = String::new();
    let mut rest = t;
    while let Some(c) = rest.chars().next() {
        if rest.starts_with("{{") || rest.starts_with("}}") {
            lit.push(c);
            rest = rest.get(2..).unwrap_or("");
            continue;
        }
        if c == '{'
            && let Some(end) = rest.find('}')
        {
            let inner = rest.get(1..end).unwrap_or("").trim();
            if !lit.is_empty() {
                out.push(Part::Lit(std::mem::take(&mut lit)));
            }
            out.push(reference(inner));
            rest = rest.get(end + 1..).unwrap_or("");
            continue;
        }
        lit.push(c);
        rest = rest.get(c.len_utf8()..).unwrap_or("");
    }
    if !lit.is_empty() {
        out.push(Part::Lit(lit));
    }
    out
}

fn reference(inner: &str) -> Part {
    if inner == "#" {
        return Part::Number;
    }
    let mut parts: Vec<&str> = inner.split(':').map(str::trim).collect();
    let decimals = match parts.last().and_then(|p| p.parse::<usize>().ok()) {
        Some(n) if parts.len() > 1 => {
            parts.pop();
            Some(n.min(12))
        }
        _ => None,
    };
    let agg = match parts.first().map(|p| p.to_lowercase()).as_deref() {
        Some("first") => Some(Agg::First),
        Some("count") => Some(Agg::Count),
        Some("sum") => Some(Agg::Sum),
        Some("min") => Some(Agg::Min),
        Some("max") => Some(Agg::Max),
        Some("join") => Some(Agg::Join),
        _ => None,
    };
    match agg {
        Some(agg) => Part::Ref { agg, field: parts.get(1..).unwrap_or(&[]).join(":").to_lowercase(), decimals },
        None => Part::Ref { agg: Agg::First, field: parts.join(":").to_lowercase(), decimals },
    }
}

/// Texts in natural order: runs of digits by their value, the rest by letters in any case.
pub fn natural(a: &str, b: &str) -> Ordering {
    let chunks = |s: &str| -> Vec<(bool, String)> {
        let mut v: Vec<(bool, String)> = Vec::new();
        for c in s.chars().take(1024) {
            let d = c.is_ascii_digit();
            match v.last_mut() {
                Some((ld, t)) if *ld == d => t.push(c),
                _ => v.push((d, c.to_string())),
            }
        }
        v
    };
    let (ca, cb) = (chunks(a), chunks(b));
    for (x, y) in ca.iter().zip(cb.iter()) {
        let o = match (x, y) {
            ((true, p), (true, q)) => {
                let (p, q) = (p.trim_start_matches('0'), q.trim_start_matches('0'));
                p.len().cmp(&q.len()).then_with(|| p.cmp(q))
            }
            ((_, p), (_, q)) => p.to_lowercase().cmp(&q.to_lowercase()),
        };
        if o != Ordering::Equal {
            return o;
        }
    }
    ca.len().cmp(&cb.len())
}

/// Numbers by value (a `;` list by its sum), anything else in natural order.
fn by_value(a: &str, b: &str) -> Ordering {
    match (num(a), num(b)) {
        (Some(x), Some(y)) => x.total_cmp(&y),
        _ => natural(a, b),
    }
}

/// Whether object `e` is one section `s` reads.
fn takes(d: &Drawing, s: &Section, e: &Entity) -> bool {
    let any = |spec: &str, text: &str| spec.split(',').any(|p| wildcard(p.trim(), text));
    if matches!(e.kind, EntityKind::Viewport(_)) {
        return false;
    }
    let ty = e.kind.dxf_name();
    if !(any(&s.from, ty) || any(&s.from, e.kind.type_name())) {
        return false;
    }
    if let Some(l) = &s.layers
        && !any(l, &e.common.layer)
    {
        return false;
    }
    if let Some(b) = &s.blocks {
        return matches!(e.kind, EntityKind::Insert(_)) && any(b, &block_name(d, e));
    }
    true
}

/// The rows of report `r` over objects `objs`. Errors when it reads too many records.
pub fn run(d: &Drawing, r: &Report, objs: &[&Entity], lookups: &Lookups) -> Result<Vec<Row>, String> {
    let xr = d.foreign.object_index();
    let mut rows = Vec::new();
    let mut number = 0usize;
    let mut total = 0usize;
    for s in &r.sections {
        let ctx = Ctx { d, objs, xr: &xr, fields: &s.fields, lookups };
        // Records: an object each, or one per item of its `each` fields.
        let mut recs: Vec<Rec> = Vec::new();
        for (i, e) in objs.iter().enumerate() {
            if !takes(d, s, e) {
                continue;
            }
            if s.each.is_empty() {
                recs.push(Rec { obj: i, item: None, memo: HashMap::new() });
            } else {
                let mut probe = Rec { obj: i, item: None, memo: HashMap::new() };
                let mut n = 0;
                for f in &s.each {
                    let v = ctx.value(&mut probe, f, 0);
                    let items: Vec<String> =
                        if s.split.is_empty() { vec![v] } else { v.split(s.split.as_str()).map(|x| x.trim().to_string()).collect() };
                    for item in items.into_iter().filter(|x| !x.is_empty()).take(MAX_LIST) {
                        n += 1;
                        recs.push(Rec { obj: i, item: Some((item, n)), memo: HashMap::new() });
                    }
                }
            }
            total = total.saturating_add(recs.len());
            if recs.len() > MAX_RECORDS || total > MAX_RECORDS.saturating_mul(4) {
                return Err(format!("the report reads more than {MAX_RECORDS} records"));
            }
        }
        // Filters.
        let mut kept: Vec<usize> = Vec::new();
        for i in 0..recs.len() {
            let Some(rec) = recs.get_mut(i) else { continue };
            let ok = s.filters.iter().all(|f| {
                let v = ctx.value(rec, &f.field, 0);
                let hit = if f.pattern.is_empty() { v.trim().is_empty() } else { wildcard(&f.pattern, v.trim()) };
                hit != f.negate
            });
            if ok {
                kept.push(i);
            }
        }
        // Groups, in the order they first appear.
        let mut groups: Vec<(Vec<String>, Vec<usize>)> = Vec::new();
        match &s.group {
            None => groups.extend(kept.iter().map(|&i| (Vec::new(), vec![i]))),
            Some(keys) => {
                let mut at: HashMap<Vec<String>, usize> = HashMap::new();
                for &i in &kept {
                    let Some(rec) = recs.get_mut(i) else { continue };
                    let key: Vec<String> = keys.iter().map(|k| ctx.value(rec, k, 0).trim().to_string()).collect();
                    match at.get(&key).and_then(|&g| groups.get_mut(g)) {
                        Some(g) => g.1.push(i),
                        None => {
                            at.insert(key.clone(), groups.len());
                            groups.push((key, vec![i]));
                        }
                    }
                }
            }
        }
        // Order.
        match &s.sort {
            Some(keys) if !keys.is_empty() => {
                let mut keyed: Vec<(Vec<(bool, String)>, (Vec<String>, Vec<usize>))> = Vec::new();
                for g in groups {
                    let k = keys
                        .iter()
                        .map(|k| {
                            let (desc, k) = match k.strip_prefix('-') {
                                Some(rest) => (true, rest.trim()),
                                None => (false, k.as_str()),
                            };
                            let t = if k.contains('{') { k.to_string() } else { format!("{{{k}}}") };
                            (desc, ctx.group_text(&mut recs, &g.1, &t, 0))
                        })
                        .collect();
                    keyed.push((k, g));
                }
                keyed.sort_by(|a, b| {
                    a.0.iter()
                        .zip(b.0.iter())
                        .map(|((desc, x), (_, y))| if *desc { by_value(y, x) } else { by_value(x, y) })
                        .find(|o| *o != Ordering::Equal)
                        .unwrap_or(Ordering::Equal)
                });
                groups = keyed.into_iter().map(|(_, g)| g).collect();
            }
            Some(_) => {}
            None => {
                if s.group.as_ref().is_some_and(|g| !g.is_empty()) {
                    groups.sort_by(|a, b| {
                        a.0.iter().zip(b.0.iter()).map(|(x, y)| natural(x, y)).find(|o| *o != Ordering::Equal).unwrap_or(Ordering::Equal)
                    });
                }
            }
        }
        if let Some(t) = &s.title
            && !groups.is_empty()
        {
            rows.push(Row::Title(t.clone()));
        }
        for (_, g) in &groups {
            number += 1;
            let mut cells = Vec::new();
            for rc in &s.row {
                cells.push(match rc {
                    RowCell::Text(t) => Out { text: ctx.group_text(&mut recs, g, t, number), sketch: None, block: None },
                    RowCell::Sketch(sd)
                        if let Some(name) =
                            sd.block.iter().map(|t| ctx.group_text(&mut recs, g, t, number).trim().to_string()).find(|n| !n.is_empty()) =>
                    {
                        let mut eval_map = |m: &BTreeMap<String, String>| -> BTreeMap<String, String> {
                            m.iter().map(|(k, t)| (k.clone(), ctx.group_text(&mut recs, g, t, number))).collect()
                        };
                        let attribs = eval_map(&sd.attribs);
                        let props = eval_map(&sd.props);
                        let b = cadcraft_doc::CellBlock { name: name.chars().take(255).collect(), attribs, props, ..Default::default() };
                        Out { text: name, sketch: None, block: Some(b) }
                    }
                    RowCell::Sketch(sd) => {
                        let split = |v: String| -> Vec<String> { v.split(';').map(|x| x.trim().to_string()).collect() };
                        let ends = split(ctx.group_text(&mut recs, g, &sd.ends, number));
                        let lengths: Vec<String> =
                            split(ctx.group_text(&mut recs, g, &sd.lengths, number)).into_iter().filter(|x| !x.is_empty()).collect();
                        let sk = super::sketch::Sketch { nodes: ends, lengths, unit: sd.unit.clone(), width: sd.width, words: sd.words.clone() };
                        Out { text: sk.summary(), sketch: Some(sk), block: None }
                    }
                });
            }
            rows.push(Row::Data(cells));
        }
    }
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn templates_parse_references_aggregates_and_braces() {
        assert_eq!(parse("{#}. {{x}}"), vec![Part::Number, Part::Lit(". {x}".into())]);
        assert_eq!(parse("{sum:xr.L:0} cm"), vec![Part::Ref { agg: Agg::Sum, field: "xr.l".into(), decimals: Some(0) }, Part::Lit(" cm".into())]);
        assert_eq!(parse("{Att.PART}"), vec![Part::Ref { agg: Agg::First, field: "att.part".into(), decimals: None }]);
        assert_eq!(parse("{count}"), vec![Part::Ref { agg: Agg::Count, field: String::new(), decimals: None }]);
        assert_eq!(parse("open { brace"), vec![Part::Lit("open { brace".into())]);
        assert_eq!(parse("кол-во {count}"), vec![Part::Lit("кол-во ".into()), Part::Ref { agg: Agg::Count, field: String::new(), decimals: None }]);
    }

    #[test]
    fn numbers_read_lists_and_commas() {
        assert_eq!(num("120;80"), Some(200.0));
        assert_eq!(num(" 2,5 "), Some(2.5));
        assert_eq!(num("abc"), None);
        assert_eq!(num("1;x"), None);
        assert_eq!(fmt_num(2.50, None), "2.5");
        assert_eq!(fmt_num(-0.00001, None), "0");
        assert_eq!(fmt_num(2.456, Some(1)), "2.5");
        assert_eq!(fmt_num(3.0, Some(0)), "3");
    }

    #[test]
    fn natural_order_counts_digits_by_value() {
        let mut v = vec!["X10", "x2", "X1", "Y", "a"];
        v.sort_by(|a, b| natural(a, b));
        assert_eq!(v, vec!["a", "X1", "x2", "X10", "Y"]);
    }

    #[test]
    fn lookups_match_numbers_by_value() {
        assert!(same("02", "2"));
        assert!(same("Провод", "провод"));
        assert!(!same("1;2", "3"));
    }
}
