# Table templates (TABLETEMPLATE)

A table template is a JSON file that says how a kind of table looks: its columns, a header of
one or more rows, a title, the text style and heights, row heights and how rows grow, and how
heavy each kind of line is. It holds no data, so the same template serves a table filled by
hand and a [report](reports.md) (`"template": "gost-2.106.json"` in the report definition).
A standard (GOST 2.106, a DIN/ISO parts list, an ASME BOM) is a template file, not code.

## Commands

| Command | What it does |
|---|---|
| `TABLETEMPLATE` (`TTPL`), Draw ▸ Table from Template... | Asks for the template file (listed in a window and completed on Tab like REPORT's, or Browse...), the number of data rows and the insertion point. With a table selected, it lays that table out by the template instead: its data rows stay. |
| Right-click a table or its cells ▸ Apply Table Template... | The same for that table. |

JSON form: `{"file": "gost.json", "at": [0, 0], "rows": 10}` or `{"file": "gost.json", "handle": "1A2"}`
(`template` instead of `file` gives the template itself).

## The template

```json
{
  "name": "GOST 2.106 form 1",
  "textStyle": {"name": "GOST", "font": "GOST type A"},
  "textHeight": 3.5,
  "margin": 1,
  "rowHeight": 8,
  "rowGrow": 8,
  "lines": {"outline": 0.7, "header": 0.7, "vertical": 0.7, "horizontal": 0.25},
  "header": {"height": 15, "textHeight": 2.5},
  "columns": [
    {"header": "Format", "width": 6, "align": "MC", "headerRotation": 90},
    {"header": "Zone", "width": 6, "align": "MC", "headerRotation": 90},
    {"header": "Item", "width": 8, "align": "MC", "headerRotation": 90},
    {"header": "Designation", "width": 70, "align": "ML"},
    {"header": "Name", "width": 63, "align": "ML"},
    {"header": "Qty", "width": 10, "align": "MC", "headerRotation": 90},
    {"header": "Note", "width": 22, "align": "ML"}
  ]
}
```

| Key | |
|---|---|
| `name` | The table style the template makes (replaced when the template is applied again). |
| `textStyle` | A text style name, or `{name, font}` to make it when the drawing has none of that name. |
| `textHeight` | Data text height (default 2.5). |
| `margin`, `marginV` | Cell margins along and across the text (default 0.3 × the text height). |
| `rowHeight` | Data row height. |
| `rowGrow` | How rows grow for their text: `free`, `fixed` or a step (`8`: multiples of 8, as GOST 2.106 asks). |
| `lines` | Lineweights in mm: `outline` (the table's outer edges), `header` (under and inside the header), `vertical` (between columns), `horizontal` (between data rows). Missing: as the layer draws them. |
| `title` | A title row: a text, or `{text, height, textHeight}`. Without text a table keeps its own title. |
| `header` | `{height, textHeight}` for one header row of the columns' `header` texts, or `{rows, heights, textHeight}` for several rows: in `rows`, `<` joins a cell to the one on its left and `^` to the one above. `false`: no header. |
| `columns` | `{header, width, align, rotation, margin, textHeight, headerAlign, headerRotation}`; alignments `TL` … `BR`, rotations 0, 90, 180, 270. |
| `breaks` | A long table in parts, one per sheet: `{height, spacing, direction, repeatTop}` — the tallest part (header included), the space between parts (default 10), `right` (default), `left` or `down`, and whether each part repeats the header (default yes). |

A header of two rows:

```json
"header": {"rows": [["No.", "Ends", "<"], ["^", "From", "To"]], "heights": [8, 6]}
```

## How it is applied

The lines and the text go into the table style's Title, Header and Data cell styles, so rows
added later (Insert Above/Below, a report update) get the same lines. Columns' alignment,
rotation, margin and text height go on the cells; an inserted row copies them from its
neighbouring data row. Applying a template to a table replaces its title and header rows with
the template's and keeps its data rows (in the template's columns); the cells' own borders are
removed, as the template draws the lines.

The table keeps the template's file and a copy of it in CADCraft's table xdata (`TABLEX`).
