# Report tables (REPORT)

A report table is a table whose data rows come from the drawing's objects. A report definition
(JSON) says which objects to read, what to read from them, and how records become rows:
filtered, grouped, sorted, counted and summed, with lookup tables that map codes to names. The
table keeps its definition, so `REPORTUPDATE` (or **Update Report** in the table's right-click
menu) makes the rows again when the drawing changes.

The engine knows no standard: a parts list, a bill of materials for an ERP system or a wire
list is a definition file, and it can be kept in a company folder and versioned like code.

## Commands

| Command | What it does |
|---|---|
| `REPORT` (`RPT`), Tools ▸ Report Table... | Asks for the report file (Browse... opens a file dialog), the objects to read (Enter: all of them) and the insertion point. |
| `REPORTUPDATE` (`RPTU`) | Makes the selected report tables again (Enter: every report table). The rows above the data (title, header) stay as you edited them, and new rows take the look of the first data row. |

The JSON form (`engine.execute`, scripts, MCP):

```json
{"command": "report", "params": {"file": "materials.json", "at": [0, 0]}}
```

| Parameter | |
|---|---|
| `file` | The definition file (`.json`). Relative paths are read from the drawing's folder. |
| `definition` | The definition itself (an object), instead of `file`. |
| `handles` | The objects to read; default: every object of the drawing. |
| `at` | Insert a new table there. |
| `table` | Fill this table again (a handle). |
| `csv` | Also write the rows to this file (`.csv`; `.txt`/`.tsv` tab-separated). |

Without `at` or `table` the command only returns `{columns, rows}`.

## The definition

```json
{
  "title": "Materials",
  "columns": [{"header": "No.", "width": 10}, {"header": "Name", "width": 120}, "Qty"],
  "rowHeight": 8,
  "lookups": {
    "codes": {"file": "codes.csv"},
    "colors": {"rows": [{"aci": "1", "name": "red"}, {"aci": "5", "name": "blue"}]}
  },
  "sections": [
    {
      "title": "Parts",
      "from": "insert",
      "blocks": "*",
      "layers": "*",
      "fields": {"part": "{att.PART}"},
      "where": {"part": "?*"},
      "group": ["part"],
      "sort": ["part"],
      "row": ["{#}", "{part}", "{count}"]
    }
  ]
}
```

| Key | |
|---|---|
| `title` | The table's title row. |
| `columns` | Header texts, or `{header, width}`. No headers: no header row. A column without a width is as wide as its longest text. |
| `rowHeight` | Height of the data rows. |
| `lookups` | Tables by name: `{"file": "x.csv"}` (comma, semicolon or tab separated, header row first) or `.json` (a list of objects), or `{"rows": [...]}`. |
| `sections` | Parts of the report, one after the other. A definition without `sections` is one section. |

A section:

| Key | |
|---|---|
| `title` | A row across the table above the section's rows (left out when the section has none). |
| `from` | Object types, as DXF names: `insert`, `lwpolyline,line`, `*` (default). |
| `blocks`, `layers` | Block and layer names, with wildcards (`*`, `?`, `#`), comma separated. |
| `each` | Fields whose values make one record each: `["xr.c1", "xr.c2"]` reads both ends of a wire. A value is split at `split` (default `;`); the part is the field `item`, its number `index`. |
| `fields` | Fields the section computes: a template (`"{att.PART} {layer}"`) or a lookup: `{"lookup": "codes", "match": {"code": "{item}"}, "get": "name", "default": ""}` takes column `get` of the first row whose `match` columns equal the templates' values (any case; `02` equals `2`). |
| `where` | `{field: pattern}`: every field must match its wildcard pattern; `!` before the pattern negates it; `""` matches an empty value, `?*` any value. |
| `group` | Records with equal values of these fields make one row. Without `group`, each record is a row; `[]` makes one row of all of them (a total). |
| `sort` | Fields or templates, `-` first for descending. Numbers sort by value, text in natural order (`X2` before `X10`). Default: by the group. |
| `row` | A template per column. |

### Templates

| Reference | |
|---|---|
| `{field}` | The value (in a group: the first record's that has one). |
| `{#}` | The row number, counted on through the sections. |
| `{count}` | Records in the row; `{count:field}` those with a value of `field`. |
| `{sum:field}`, `{min:field}`, `{max:field}` | Numbers; a `;` list (`40;35`) counts as its sum; a decimal comma is read. |
| `{join:field}` | The different values, joined with `, `. |
| `{…:N}` | Rounds a number to N decimals: `{sum:length:0}`. |
| `{{`, `}}` | Braces. |

### Fields of an object

| Field | |
|---|---|
| `type` | DXF type (`INSERT`, `LWPOLYLINE`…). |
| `name` | A block reference's block (a dynamic block's own name for its anonymous copies). |
| `layer`, `linetype`, `handle` | |
| `color` | ACI number (256 ByLayer, 0 ByBlock). |
| `length`, `area` | Lines and polylines; circles' `area`. |
| `att.TAG` | Attribute value (constant attributes too). |
| `xr.KEY` | The first value of XRECORD `KEY` in the object's extension dictionary. |
| `xd.APP` | Application `APP`'s xdata texts, joined with `;`. |
| `dyn.NAME` | A dynamic block property (`dyn.Visibility` finds `Visibility1`). |
| `item`, `index` | With `each`. |
| anything else | A property the Properties palette shows (`radius`, `rotation`, `centerX`…). |

## Example: a materials list

Parts from block attributes, wire ends from XRECORDs on the wires, insulators counted from
codes ending in `И`, and wire lengths by layer (the wire type) and colour:

```json
{
  "title": "Materials",
  "columns": [{"header": "No.", "width": 10}, {"header": "Name", "width": 120}, {"header": "Qty", "width": 25}],
  "lookups": {"wires": {"file": "wires.csv"}, "ends": {"file": "terminals.csv"}},
  "sections": [
    {"from": "insert", "fields": {"name": "{att.PART}"}, "where": {"name": "?*"}, "group": ["name"],
     "row": ["{#}", "{name}", "{count}"]},
    {"from": "lwpolyline", "each": ["xr.c1", "xr.c2"],
     "fields": {"mat": {"lookup": "ends", "match": {"val": "{item}"}, "get": "name"}},
     "where": {"mat": "?*"}, "group": ["mat"], "row": ["{#}", "{mat}", "{count}"]},
    {"from": "lwpolyline", "each": ["xr.c1", "xr.c2"], "where": {"item": "*И"}, "group": [],
     "row": ["{#}", "Insulator", "{count}"]},
    {"from": "lwpolyline", "where": {"xr.L": "?*", "mat": "?*"},
     "fields": {"mat": {"lookup": "wires", "match": {"type": "{layer}", "color": "{color}"}, "get": "name"}},
     "group": ["mat"], "row": ["{#}", "{mat}", "{sum:xr.L} cm"]}
  ]
}
```

## What the table keeps

The table keeps the definition file's path, a copy of the definition, the first data row and
the objects it read (none: the whole drawing), in CADCraft's table xdata (`TABLEX`). AutoCAD
sees an ordinary table. On update the file is read again; when it is gone, the kept copy is
used (a copy larger than 8 KB is not saved next to its file, as AutoCAD keeps at most 16 KB of
xdata on an object).

Limits: 500,000 records per section, 2,000 rows, 100 sections, 200 columns, 100,000 lookup rows.
