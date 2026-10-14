//! Table cell content: blocks in cells (TINSERT) with attribute values, fitted into the cell, and
//! cells' own borders (`table.cellborders`), drawn, and kept through DXF.

use cadcraft_engine::Session;
use cadcraft_engine::doc::{EntityKind, Handle, Lineweight, Table};
use cadcraft_engine::render;
use serde_json::json;

fn table(s: &Session, h: &str) -> Table {
    match &s.doc().unwrap().entity(Handle::parse_hex(h).unwrap()).unwrap().kind {
        EntityKind::Table(t) => t.clone(),
        _ => panic!("not a table"),
    }
}

/// A 2×2 table with 10-wide, 4-high cells and a block "TAG" (a 2×1 rectangle and an attribute
/// NUMBER), both from the origin.
fn session() -> (Session, String) {
    let mut s = Session::new();
    let r = s.execute("table", &json!({ "at": [0, 0], "rows": 2, "cols": 2, "rowHeight": 4.0, "colWidth": 10.0 })).unwrap();
    let h = r["handle"].as_str().unwrap().to_string();
    let rect = s.execute("rectang", &json!({ "p1": [100, 100], "p2": [102, 101] })).unwrap();
    let att = s.execute("attdef", &json!({ "tag": "NUMBER", "default": "0", "at": [100.5, 100.25], "height": 0.5 })).unwrap();
    let hs: Vec<String> = [rect, att].iter().map(|v| v["handle"].as_str().unwrap().to_string()).collect();
    s.execute("block", &json!({ "name": "TAG", "base": [100, 100], "handles": hs })).unwrap();
    (s, h)
}

#[test]
fn a_block_fills_its_cell_with_its_attribute_values() {
    let (mut s, h) = session();
    s.execute("table.set", &json!({ "handle": h, "row": 0, "col": 1, "text": "gone" })).unwrap();
    s.execute("tinsert", &json!({ "handle": h, "row": 0, "col": 1, "block": "tag", "attribs": { "number": 7 } })).unwrap();
    let t = table(&s, &h);
    let cb = t.cells[0][1].block.as_ref().unwrap();
    assert_eq!(cb.name, "TAG");
    assert_eq!(cb.attrib("NUMBER"), Some("7"));
    // A cell holds a block or text.
    assert!(t.cells[0][1].text.is_empty());
    let g = render::table_geometry(s.doc().unwrap(), &t);
    assert_eq!(g.blocks.len(), 1);
    let b = &g.blocks[0].block.insert;
    // Fitted: the 2 × 1 block in a 10 × 4 cell is limited by the height inside the margins.
    let style = render::table_style_of(s.doc().unwrap(), &t);
    let inner = 4.0 - 2.0 * style.cell_style("Data").margin_v;
    assert!((b.scale.x - inner).abs() < 1e-6, "scale {}", b.scale.x);
    // Inside the cell (x 10..20, y -4..0).
    assert!(b.insert.x >= 10.0 - 1e-9 && b.insert.x <= 20.0 && b.insert.y <= 0.0 && b.insert.y >= -4.0, "{:?}", b.insert);
    assert_eq!(b.attribs.len(), 1);
    assert_eq!(b.attribs[0].text.value, "7");
    // No text is drawn for it.
    assert!(g.texts.iter().all(|x| x.text != "gone"));
}

#[test]
fn a_fixed_scale_is_kept_and_an_empty_name_removes_the_block() {
    let (mut s, h) = session();
    s.execute("tinsert", &json!({ "handle": h, "row": 1, "col": 0, "block": "TAG", "scale": 0.5, "rotation": 90 })).unwrap();
    let t = table(&s, &h);
    let g = render::table_geometry(s.doc().unwrap(), &t);
    let b = &g.blocks[0].block.insert;
    assert_eq!(b.scale.x, 0.5);
    assert!((b.rotation - std::f64::consts::FRAC_PI_2).abs() < 1e-12);
    s.execute("tinsert", &json!({ "handle": h, "row": 1, "col": 0, "block": "" })).unwrap();
    assert!(table(&s, &h).cells[1][0].block.is_none());
    // Typing text into a block's cell replaces the block.
    s.execute("tinsert", &json!({ "handle": h, "row": 1, "col": 0, "block": "TAG" })).unwrap();
    s.execute("tabledit", &json!({ "at": [5, -6], "text": "text" })).unwrap();
    let c = &table(&s, &h).cells[1][0];
    assert_eq!((c.text.as_str(), c.block.is_none()), ("text", true));
}

#[test]
fn bad_tinsert_parameters_are_refused() {
    let (mut s, h) = session();
    for p in [
        json!({ "handle": h, "row": 0, "col": 0, "block": "NOPE" }),
        json!({ "handle": h, "row": 9, "col": 0, "block": "TAG" }),
        json!({ "handle": h, "row": 0, "col": 0, "block": "TAG", "scale": -1 }),
        json!({ "handle": h, "row": 0, "col": 0, "block": "TAG", "scale": "huge" }),
        json!({ "handle": h, "row": 0, "col": 0, "block": "TAG", "attribs": [1, 2] }),
        // TAG is not a dynamic block: it has no visibility states.
        json!({ "handle": h, "row": 0, "col": 0, "block": "TAG", "props": { "Visibility1": "A" } }),
        json!({ "at": [500, 500], "block": "TAG" }),
    ] {
        assert!(s.execute("tinsert", &p).is_err(), "{p}");
    }
    assert!(table(&s, &h).cells.iter().flatten().all(|c| c.block.is_none()));
}

#[test]
fn hostile_tinsert_params_never_panic() {
    let (mut s, h) = session();
    let long = "x".repeat(100_000);
    for p in [
        json!({}),
        json!({ "handle": h }),
        json!({ "handle": h, "row": u64::MAX, "col": u64::MAX, "block": "TAG" }),
        json!({ "handle": h, "row": 0, "col": 0, "block": long }),
        json!({ "handle": h, "row": 0, "col": 0, "block": "TAG", "rotation": 1e308, "scale": 1e308 }),
        json!({ "handle": h, "row": 0, "col": 0, "block": "TAG", "attribs": { "NUMBER": long } }),
        json!({ "handle": h, "row": 0, "col": 0, "rows": u64::MAX, "cols": u64::MAX, "edges": ["all"], "lineweight": 99 }),
    ] {
        let _ = s.execute("tinsert", &p);
        let _ = s.execute("table.cellborders", &p);
    }
    let t = table(&s, &h);
    let _ = render::table_geometry(s.doc().unwrap(), &t);
}

#[test]
fn cell_borders_override_the_style_and_the_heavier_wins() {
    let (mut s, h) = session();
    // The whole table's outline heavy, the top row's bottom thin: on the edge between the rows
    // the second row's top (not set) leaves the first row's thin bottom.
    s.execute("table.cellborders", &json!({ "handle": h, "row": 0, "col": 0, "rows": 2, "cols": 2, "edges": "outside", "lineweight": 0.5 })).unwrap();
    s.execute("table.cellborders", &json!({ "handle": h, "row": 0, "col": 0, "cols": 2, "edges": "bottom", "lineweight": 0.18 })).unwrap();
    // The lower left cell's top heavier than the upper left cell's bottom: it wins there.
    s.execute("table.cellborders", &json!({ "handle": h, "row": 1, "col": 0, "edges": "top", "lineweight": 0.7 })).unwrap();
    let t = table(&s, &h);
    let g = render::table_geometry(s.doc().unwrap(), &t);
    // The lineweights of the horizontal segments at height `y` overlapping x0..x1.
    let at = |y: f64, x0: f64, x1: f64| {
        g.lines
            .iter()
            .filter(|l| (l.a.y - y).abs() < 1e-9 && (l.b.y - y).abs() < 1e-9 && l.a.x.min(l.b.x) < x1 - 1e-9 && l.a.x.max(l.b.x) > x0 + 1e-9)
            .map(|l| l.lineweight)
            .collect::<Vec<_>>()
    };
    assert_eq!(at(0.0, 0.0, 20.0), [Lineweight::Mm100(50)], "top");
    assert_eq!(at(-4.0, 0.0, 10.0), [Lineweight::Mm100(70)], "middle, left");
    assert_eq!(at(-4.0, 10.0, 20.0), [Lineweight::Mm100(18)], "middle, right");
    // Clearing goes back to the cell style.
    s.execute("table.cellborders", &json!({ "handle": h, "row": 0, "col": 0, "rows": 2, "cols": 2, "clear": true })).unwrap();
    assert!(table(&s, &h).cells.iter().flatten().all(|c| c.edges.is_none()));
}

#[test]
fn block_cells_and_borders_survive_dxf() {
    let (mut s, h) = session();
    s.execute("tinsert", &json!({ "handle": h, "row": 0, "col": 0, "block": "TAG", "attribs": { "NUMBER": "12" } })).unwrap();
    s.execute("table.cellborders", &json!({ "handle": h, "row": 1, "col": 1, "edges": "left", "lineweight": 0.5, "color": 1 })).unwrap();
    s.execute("table.set", &json!({ "handle": h, "row": 1, "col": 0, "text": "a long text that wraps in its cell" })).unwrap();
    let d = s.doc().unwrap();
    let dxf = cadcraft_io::write_dxf(d);
    // AutoCAD sees a block cell: type 2 with the block's record and the attribute value.
    let lines: Vec<&str> = dxf.lines().map(str::trim).collect();
    let pair = |code: &str, value: &str| lines.windows(2).any(|w| w[0] == code && w[1] == value);
    assert!(pair("171", "2"), "block cell type");
    assert!(pair("300", "12"), "attribute value");
    let back = cadcraft_io::read_dxf(dxf.as_bytes()).unwrap();
    let t2 = back
        .model
        .iter()
        .find_map(|e| match &e.kind {
            EntityKind::Table(t) => Some(t.clone()),
            _ => None,
        })
        .unwrap();
    let t = table(&s, &h);
    assert_eq!(t2.cells[0][0].block, t.cells[0][0].block);
    assert_eq!(t2.cells[1][1].edges, t.cells[1][1].edges);
    // The wrapped text keeps its cell width.
    let g = render::table_geometry(&back, &t2);
    let w = g.texts.iter().find(|x| x.text.starts_with("a long")).unwrap().width;
    assert!(w > 0.0 && w < 10.0, "{w}");
}

/// A session with a table and a dynamic block "Door" (kept foreign data, built by hand): a
/// visibility parameter whose state Open shows its line and Closed hides it; a circle is shown
/// in both.
fn dynamic_session() -> (Session, String, Handle) {
    use cadcraft_engine::doc::RawTag;
    use cadcraft_engine::doc::foreign::{Foreign, ForeignExtra, ForeignObject};
    let obj = |handle: u64, kind: &str, tags: &[(i32, &str)]| ForeignObject {
        handle: Handle(handle),
        kind: kind.into(),
        tags: tags.iter().map(|(c, v)| RawTag { code: *c, value: (*v).into() }).collect(),
    };
    let mut s = Session::new();
    let h = s.execute("table", &json!({ "at": [0, 0], "rows": 2, "cols": 1, "rowHeight": 4.0, "colWidth": 10.0 })).unwrap()["handle"]
        .as_str()
        .unwrap()
        .to_string();
    let l = s.execute("line", &json!({ "points": [[0, 0], [4, 0]] })).unwrap()["handles"][0].clone();
    let c = s.execute("circle", &json!({ "center": [0, 0], "radius": 1 })).unwrap()["handle"].clone();
    s.execute("block", &json!({ "name": "Door", "base": [0, 0], "handles": [l, c] })).unwrap();
    let blk = s.doc().unwrap().block("Door").unwrap().clone();
    let line = blk.entities.iter().find(|e| matches!(e.kind, EntityKind::Line(_))).unwrap().handle;
    let lh = line.hex();
    let mut f = Foreign::default();
    f.records.insert(("BLOCK_RECORD".into(), "DOOR".into()), ForeignExtra { xdict: Some(Handle(0xF00)), ..Default::default() });
    f.objects.push(obj(0xF00, "DICTIONARY", &[(100, "AcDbDictionary"), (3, "ACAD_ENHANCEDBLOCK"), (360, "F01")]));
    f.objects.push(obj(0xF01, "ACAD_EVALUATION_GRAPH", &[(100, "AcDbEvalGraph"), (91, "0"), (95, "1"), (360, "F02")]));
    f.objects.push(obj(
        0xF02,
        "BLOCKVISIBILITYPARAMETER",
        &[
            (100, "AcDbEvalExpr"),
            (90, "1"),
            (100, "AcDbBlockElement"),
            (300, "Visibility State"),
            (100, "AcDbBlockParameter"),
            (280, "1"),
            (100, "AcDbBlock1PtParameter"),
            (1010, "2"),
            (1020, "3"),
            (100, "AcDbBlockVisibilityParameter"),
            (301, "Visibility1"),
            (302, ""),
            (93, "1"),
            (331, &lh),
            (92, "2"),
            (303, "Open"),
            (94, "1"),
            (332, &lh),
            (95, "0"),
            (303, "Closed"),
            (94, "0"),
            (95, "0"),
        ],
    ));
    let mut d = s.doc().unwrap().clone();
    d.foreign = std::sync::Arc::new(f);
    s.open_drawing(d, "dynamic", None);
    (s, h, line)
}

#[test]
fn a_dynamic_block_shows_the_cells_visibility_state() {
    let (mut s, h, line) = dynamic_session();
    s.execute("tinsert", &json!({ "handle": h, "row": 0, "col": 0, "block": "Door", "props": { "Visibility1": "open" } })).unwrap();
    s.execute("tinsert", &json!({ "handle": h, "row": 1, "col": 0, "block": "Door", "props": { "visibility": "Closed" } })).unwrap();
    // An unknown state is refused with the states it may be.
    let e = s.execute("tinsert", &json!({ "handle": h, "row": 1, "col": 0, "block": "Door", "props": { "Visibility1": "Ajar" } })).unwrap_err();
    assert!(e.to_string().contains("Open, Closed"), "{e}");
    let t = table(&s, &h);
    let g = render::table_geometry(s.doc().unwrap(), &t);
    let blk = s.doc().unwrap().block("Door").unwrap().clone();
    let shown = |row: usize| {
        let v = &g.blocks.iter().find(|b| b.row == row).unwrap().block.visibility;
        blk.entities.iter().filter(|e| v.shows(e)).map(|e| e.handle).collect::<Vec<_>>()
    };
    assert!(shown(0).contains(&line), "Open shows the line");
    assert!(!shown(1).contains(&line), "Closed hides the line");
    assert_eq!(shown(1).len(), 1, "the circle stays");
    // The *T block AutoCAD draws shows the states too: switching the closed door open adds its
    // line there.
    // (LINEs, CIRCLEs) in the written file: the door's definition has one of each.
    let lines_in_t = |s: &Session| {
        let dxf = cadcraft_io::write_dxf(s.doc().unwrap());
        let lines: Vec<&str> = dxf.lines().map(str::trim).collect();
        let n = |kind: &str| lines.windows(2).filter(|w| w[0] == "0" && w[1] == kind).count();
        (n("LINE"), n("CIRCLE"))
    };
    let (closed, circles) = lines_in_t(&s);
    assert_eq!(circles, 3);
    s.execute("tinsert", &json!({ "handle": h, "row": 1, "col": 0, "block": "Door", "props": { "Visibility1": "Open" } })).unwrap();
    assert_eq!(lines_in_t(&s), (closed + 1, 3));
}
