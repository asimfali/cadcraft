//! The report definition: JSON read into [`Report`], with every size capped.
//!
//! ```json
//! {
//!   "title": "Materials",
//!   "columns": [{"header": "No.", "width": 10}, {"header": "Name", "width": 120}, "Qty"],
//!   "lookups": {"wires": {"file": "wires.csv"}, "codes": {"rows": [{"code": "A1", "name": "…"}]}},
//!   "sections": [{
//!     "title": "Blocks",
//!     "from": "insert", "blocks": "*", "layers": "*",
//!     "each": ["xr.c1", "xr.c2"], "split": ";",
//!     "fields": {"name": "{att.PART}", "erp": {"lookup": "codes", "match": {"code": "{item}"}, "get": "name"}},
//!     "where": {"name": "?*", "item": "!Z*"},
//!     "group": ["name"], "sort": ["name"],
//!     "row": ["{#}", "{name}", "{count}"]
//!   }]
//! }
//! ```
//!
//! A definition without `sections` is one section itself (`from`, `row`… at the top).

use std::collections::BTreeMap;

use serde_json::Value;

/// Most sections, columns, fields, filters and group/sort keys a definition holds.
const MAX_SECTIONS: usize = 100;
pub(super) const MAX_COLUMNS: usize = 200;
const MAX_FIELDS: usize = 200;
const MAX_KEYS: usize = 50;
/// Longest text taken from a definition (a template, a pattern, a name).
const MAX_TEXT: usize = 4096;
/// Most lookup tables and rows in each.
const MAX_LOOKUPS: usize = 50;
pub(super) const MAX_LOOKUP_ROWS: usize = 100_000;

/// A report column: its header text and width (`None`: as wide as its text).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Column {
    pub header: String,
    pub width: Option<f64>,
}

/// Where a lookup table's rows come from.
#[derive(Clone, Debug, PartialEq)]
pub enum LookupSource {
    /// A CSV (comma, semicolon or tab separated, header first) or JSON (array of objects) file.
    File(String),
    /// Rows given in the definition: column (lower case) → value.
    Rows(Vec<BTreeMap<String, String>>),
}

/// A field a section computes for each record.
#[derive(Clone, Debug, PartialEq)]
pub enum Field {
    /// A template (`"{att.PART} {layer}"`).
    Template(String),
    /// The `get` column of the first row of lookup table `table` whose `matches` columns equal
    /// the templates' values; `default` when no row matches.
    Lookup { table: String, matches: Vec<(String, String)>, get: String, default: String },
}

/// A record filter: the field's value must match the wildcard pattern (or not, with `!`).
#[derive(Clone, Debug, PartialEq)]
pub struct Filter {
    pub field: String,
    pub pattern: String,
    pub negate: bool,
}

/// One part of a report: the objects it reads and how they become rows.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Section {
    /// A row across the table above the section's rows.
    pub title: Option<String>,
    /// Object types (`insert`, `lwpolyline,line`, wildcards), as DXF names, any case.
    pub from: String,
    /// Block names (wildcards) for block references; `None`: any.
    pub blocks: Option<String>,
    /// Layer names (wildcards); `None`: any.
    pub layers: Option<String>,
    /// Fields whose values (split by `split`) make one record each, with the value as `item`.
    pub each: Vec<String>,
    pub split: String,
    /// Computed fields by name (lower case).
    pub fields: BTreeMap<String, Field>,
    pub filters: Vec<Filter>,
    /// Records with equal values of these fields make one row; `None`: a row per record; empty:
    /// one row for all of them.
    pub group: Option<Vec<String>>,
    /// Row order: field names or templates, `-` first for descending; `None`: by the group.
    pub sort: Option<Vec<String>>,
    /// A template per column.
    pub row: Vec<String>,
}

/// A report definition.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Report {
    pub title: Option<String>,
    pub columns: Vec<Column>,
    pub lookups: BTreeMap<String, LookupSource>,
    pub sections: Vec<Section>,
    pub row_height: Option<f64>,
}

impl Report {
    /// Whether the table gets a header row (some column has a header).
    pub fn has_header(&self) -> bool {
        self.columns.iter().any(|c| !c.header.is_empty())
    }
    /// How many columns the table has: the columns, or the widest row.
    pub fn column_count(&self) -> usize {
        self.sections.iter().map(|s| s.row.len()).chain(std::iter::once(self.columns.len())).max().unwrap_or(0).clamp(1, MAX_COLUMNS)
    }
}

type Res<T> = std::result::Result<T, String>;

fn text(v: &Value) -> Option<String> {
    let s = match v {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        Value::Bool(b) => b.to_string(),
        _ => return None,
    };
    Some(s.chars().take(MAX_TEXT).collect())
}

fn opt_text(o: &serde_json::Map<String, Value>, key: &str) -> Res<Option<String>> {
    match o.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => text(v).map(Some).ok_or_else(|| format!("`{key}` must be text")),
    }
}

/// A list of texts (a single text counts as a list of one).
fn texts(v: &Value, key: &str, max: usize) -> Res<Vec<String>> {
    match v {
        Value::Array(a) => {
            if a.len() > max {
                return Err(format!("`{key}` has more than {max} entries"));
            }
            a.iter().map(|x| text(x).ok_or_else(|| format!("`{key}` must hold texts"))).collect()
        }
        other => text(other).map(|t| vec![t]).ok_or_else(|| format!("`{key}` must be a text or a list of texts")),
    }
}

fn positive(o: &serde_json::Map<String, Value>, key: &str) -> Res<Option<f64>> {
    match o.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => {
            v.as_f64().filter(|x| x.is_finite() && *x > 0.0 && *x <= 1e6).map(Some).ok_or_else(|| format!("`{key}` must be a number more than 0"))
        }
    }
}

fn columns(v: &Value) -> Res<Vec<Column>> {
    let a = v.as_array().ok_or("`columns` must be a list")?;
    if a.len() > MAX_COLUMNS {
        return Err(format!("more than {MAX_COLUMNS} columns"));
    }
    a.iter()
        .map(|c| match c {
            Value::Object(o) => Ok(Column { header: opt_text(o, "header")?.unwrap_or_default(), width: positive(o, "width")? }),
            other => text(other)
                .map(|header| Column { header, width: None })
                .ok_or_else(|| "a column must be a header text or {header, width}".to_string()),
        })
        .collect()
}

fn lookup(name: &str, v: &Value) -> Res<LookupSource> {
    let bad = || format!("lookup `{name}` must be {{file}} or {{rows: [{{column: value}}]}}");
    let o = v.as_object().ok_or_else(bad)?;
    if let Some(f) = opt_text(o, "file")? {
        return Ok(LookupSource::File(f));
    }
    let rows = o.get("rows").and_then(Value::as_array).ok_or_else(bad)?;
    if rows.len() > MAX_LOOKUP_ROWS {
        return Err(format!("lookup `{name}` has more than {MAX_LOOKUP_ROWS} rows"));
    }
    Ok(LookupSource::Rows(rows.iter().map(row_of).collect()))
}

/// A lookup row from a JSON object: its scalar values by lower-case key.
pub(super) fn row_of(v: &Value) -> BTreeMap<String, String> {
    v.as_object().map(|o| o.iter().take(MAX_COLUMNS).filter_map(|(k, v)| Some((k.trim().to_lowercase(), text(v)?))).collect()).unwrap_or_default()
}

fn field(name: &str, v: &Value) -> Res<Field> {
    if let Some(t) = text(v) {
        return Ok(Field::Template(t));
    }
    let o = v.as_object().ok_or_else(|| format!("field `{name}` must be a template or {{lookup, match, get}}"))?;
    let table = opt_text(o, "lookup")?.ok_or_else(|| format!("field `{name}`: `lookup` (a lookup table) is required"))?.trim().to_lowercase();
    let get = opt_text(o, "get")?.ok_or_else(|| format!("field `{name}`: `get` (the column to take) is required"))?.trim().to_lowercase();
    let m = o.get("match").and_then(Value::as_object).ok_or_else(|| format!("field `{name}`: `match` ({{column: template}}) is required"))?;
    let matches = m
        .iter()
        .take(MAX_KEYS)
        .map(|(k, v)| text(v).map(|t| (k.trim().to_lowercase(), t)).ok_or_else(|| format!("field `{name}`: `match` values must be templates")))
        .collect::<Res<Vec<_>>>()?;
    Ok(Field::Lookup { table, matches, get, default: opt_text(o, "default")?.unwrap_or_default() })
}

fn filters(v: &Value) -> Res<Vec<Filter>> {
    let o = v.as_object().ok_or("`where` must be {field: pattern}")?;
    if o.len() > MAX_KEYS {
        return Err(format!("`where` has more than {MAX_KEYS} entries"));
    }
    o.iter()
        .map(|(k, v)| {
            let p = text(v).ok_or_else(|| format!("`where`: the pattern of `{k}` must be text"))?;
            let (negate, pattern) = match p.strip_prefix('!') {
                Some(rest) => (true, rest.to_string()),
                None => (false, p),
            };
            Ok(Filter { field: k.trim().to_lowercase(), pattern, negate })
        })
        .collect()
}

fn section(v: &Value) -> Res<Section> {
    let o = v.as_object().ok_or("a section must be an object")?;
    let keys = |key: &str| -> Res<Option<Vec<String>>> {
        match o.get(key) {
            None | Some(Value::Null) => Ok(None),
            Some(v) => texts(v, key, MAX_KEYS).map(|k| Some(k.into_iter().map(|x| x.trim().to_string()).collect())),
        }
    };
    let mut fields = BTreeMap::new();
    if let Some(f) = o.get("fields").filter(|v| !v.is_null()) {
        let f = f.as_object().ok_or("`fields` must be {name: template or lookup}")?;
        if f.len() > MAX_FIELDS {
            return Err(format!("more than {MAX_FIELDS} fields"));
        }
        for (k, v) in f {
            fields.insert(k.trim().to_lowercase(), field(k, v)?);
        }
    }
    let row = match o.get("row") {
        Some(v) => texts(v, "row", MAX_COLUMNS)?,
        None => return Err("a section needs `row`: a template per column".into()),
    };
    Ok(Section {
        title: opt_text(o, "title")?,
        from: opt_text(o, "from")?.filter(|f| !f.trim().is_empty()).unwrap_or_else(|| "*".into()),
        blocks: opt_text(o, "blocks")?.filter(|f| !f.trim().is_empty()),
        layers: opt_text(o, "layers")?.filter(|f| !f.trim().is_empty()),
        each: keys("each")?.unwrap_or_default().into_iter().map(|k| k.to_lowercase()).collect(),
        split: opt_text(o, "split")?.unwrap_or_else(|| ";".into()),
        fields,
        filters: o.get("where").filter(|v| !v.is_null()).map(filters).transpose()?.unwrap_or_default(),
        group: keys("group")?.map(|g| g.into_iter().map(|k| k.to_lowercase()).collect()),
        sort: keys("sort")?,
        row,
    })
}

/// Read a report definition.
pub fn parse(v: &Value) -> Res<Report> {
    let o = v.as_object().ok_or("a report definition must be a JSON object")?;
    let sections = match o.get("sections") {
        Some(Value::Array(a)) => {
            if a.len() > MAX_SECTIONS {
                return Err(format!("more than {MAX_SECTIONS} sections"));
            }
            a.iter().map(section).collect::<Res<Vec<_>>>()?
        }
        Some(_) => return Err("`sections` must be a list".into()),
        None => vec![section(v)?],
    };
    if sections.is_empty() {
        return Err("a report needs at least one section".into());
    }
    let mut lookups = BTreeMap::new();
    if let Some(l) = o.get("lookups").filter(|v| !v.is_null()) {
        let l = l.as_object().ok_or("`lookups` must be {name: {file} or {rows}}")?;
        if l.len() > MAX_LOOKUPS {
            return Err(format!("more than {MAX_LOOKUPS} lookup tables"));
        }
        for (k, v) in l {
            lookups.insert(k.trim().to_lowercase(), lookup(k, v)?);
        }
    }
    for s in &sections {
        for f in s.fields.values() {
            if let Field::Lookup { table, .. } = f
                && !lookups.contains_key(table)
            {
                return Err(format!("no lookup table `{table}` in `lookups`"));
            }
        }
    }
    Ok(Report {
        title: opt_text(o, "title")?,
        columns: o.get("columns").filter(|v| !v.is_null()).map(columns).transpose()?.unwrap_or_default(),
        lookups,
        sections,
        row_height: positive(o, "rowHeight")?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_definition_without_sections_is_one_section() {
        let r = parse(&json!({"columns": ["A", {"header": "B", "width": 30}], "from": "insert", "group": "name", "row": ["{name}", "{count}"]}));
        let r = r.unwrap_or_default();
        assert_eq!(r.columns, vec![Column { header: "A".into(), width: None }, Column { header: "B".into(), width: Some(30.0) }]);
        assert_eq!(r.sections.len(), 1);
        assert_eq!(r.sections.first().and_then(|s| s.group.clone()), Some(vec!["name".to_string()]));
        assert_eq!(r.column_count(), 2);
    }

    #[test]
    fn bad_definitions_say_what_is_wrong() {
        assert!(parse(&json!([])).is_err());
        assert!(parse(&json!({"from": "insert"})).is_err_and(|e| e.contains("row")));
        assert!(
            parse(&json!({"row": ["{x}"], "fields": {"x": {"lookup": "nope", "match": {"a": "{b}"}, "get": "c"}}}))
                .is_err_and(|e| e.contains("nope"))
        );
        assert!(parse(&json!({"row": ["{x}"], "where": {"x": ["a"]}})).is_err());
        assert!(parse(&json!({"row": ["{x}"], "rowHeight": -1})).is_err());
        let many: Vec<Value> = (0..MAX_COLUMNS + 1).map(|_| json!("x")).collect();
        assert!(parse(&json!({"row": many})).is_err());
    }

    #[test]
    fn filters_negate_with_an_exclamation_mark() {
        let r = parse(&json!({"row": ["{item}"], "where": {"Item": "!Z*"}})).unwrap_or_default();
        let f = r.sections.first().map(|s| s.filters.clone()).unwrap_or_default();
        assert_eq!(f, vec![Filter { field: "item".into(), pattern: "Z*".into(), negate: true }]);
    }
}
