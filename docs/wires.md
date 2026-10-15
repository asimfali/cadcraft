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
- **Chains.** A wire through more than two terminals is a chain of sections (`7;7;7` with four
  terminals). With one wire selected (or wires with the same sections), the palette lists the
  chain: each node's terminal and each section's length, with **+ Section** and **− Section**; a
  plain wire becomes a chain with + Section.
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

## Connectors

A block can have **connectors**: named points where wires attach — a contactor's terminals
(`A1`, `A2`, `13`, `14`), a tee's three ends. A wire end put on a connector is attached to it, and
when the block reference moves, turns, scales or mirrors, the attached ends go with it.

- **CONNECTOR** (`CONN`), Tools ▸ Wires ▸ Connector: pick a point, then type its name (Enter: the
  next number); again until Enter. Place connectors in the Block Editor, or among the objects
  you then make into a block. A connector is a POINT carrying xdata `CADCRAFT_CONNECTOR`
  (`1000` its name), so AutoCAD and other programs keep it as a point.
- **Snap.** Whenever object snaps are on (F3), the cursor goes to a block's connector before any
  other snap point near it; the tip names it (`Connector A1`). The connectors of selected block
  references, and those in the Block Editor, show as small rings with their names.
- **Attaching.** Drawing a polyline or line to a connector, or dragging a wire's end grip onto
  one, attaches that end. Taking the end off the connector lets go. The Properties palette's Wire
  section shows what each end is attached to (`Start  KM : 14`, `End  KM : A1`).
- **Following.** When an attached block reference moves (MOVE, ROTATE, SCALE, MIRROR, its grips,
  or its block definition changes in the Block Editor), the wire end follows its connector in
  the same undo step. When the segment at that end was horizontal or vertical, the vertex next to
  it moves along, so the wire keeps its right angles.
- **Copies.** Copying a block together with its wires attaches the copies to each other; a wire
  copied alone is not attached. Erasing a block leaves its wires where they are, unattached.
- The link is xdata `CADCRAFT_LINK` on the wire: `1000 start|end`, `1005` the block reference,
  `1000` the connector, for each attached end.
- JSON: `connector {at, name?}`, `connector.set {handle, name? | remove}`,
  `connector.list {handles?}`; `wire.get` returns `start` and `end` (`{handle, block, name}` or
  null).
