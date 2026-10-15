# Wires

A wire is a polyline (or a line) carrying a wire list's data: its length, the terminal at each
end, its number, its type and its colour. Reports read them into wire lists and bills of
materials ([reports](reports.md)).

The data is kept as wire-list add-ins for AutoCAD keep it, so drawings go back and forth:

| Data | Where |
|---|---|
| Length | XRECORD `L` in the object's extension dictionary; `40;35` is a chain of two sections |
| Terminals | XRECORDs `c1` and `c2`: codes such as `Н1И`, `В1`, `Z12` (`;` between a chain's nodes) |
| Number | XRECORD `N` |
| Type | the layer (`МПО 0.5`) |
| Colour | the object's colour (ACI) |

## Entering the data

- **Properties palette.** Select wires: the Wire section (first) shows their length, terminals,
  number, type and colour, `*VARIES*` where they differ. Type a value (Enter or leaving the field
  keeps it) or pick it from the ▾ list; it goes to every selected wire at once.
- **Hover.** A wire with data shows it under the cursor: `Wire 5 · 60 · Н1И → В1 · МПО 0.5 · yellow`.
- **WIRE** (`WD`), Tools ▸ Wires ▸ Wire Data..., or the right-click menu of a polyline or line:
  select wires, then type the length, or `Start`, `End`, `Number`, `Type`, `Color` for the other
  fields; Enter ends. The prompt offers what the wires share.
- **WIRENUM** (`WN`), Tools ▸ Wires ▸ Number Wires: the first number (default: the one after the
  highest the drawing has), then pick the wires one after another; each takes the next number.
- JSON: `wire.set {handles, length, c1, c2, number, type, color}`, `wire.get {handles}`,
  `wirenum {handles, start}`, `wire.choices`.

## Lists

The palette's lists come from `wires.json`, found where report files are looked for (the folders
of the last report and template files, then the drawing's), and from the values the drawing's
wires already have:

```json
{
  "terminals": ["Н1", "Н1И", "В1", "К1", "К2", "НВН", "Z12"],
  "types": ["МПО 0.5", "ПВ3 1.5", "ПВ3 2.5"],
  "colors": [{"aci": 250, "name": "черный"}, {"aci": 2, "name": "желтый"}],
  "lengths": ["0;7;7;7"]
}
```

A colour can be set by its name in this list too (`wire.set {color: "черный"}`).
