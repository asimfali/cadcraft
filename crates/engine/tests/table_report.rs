//! REPORT and REPORTUPDATE: tables made from the drawing's objects by a report definition.
//!
//! The scenario is a materials list: parts from block attributes, wire ends (terminal codes in
//! XRECORDs of the wires) and wire lengths by layer and colour, with lookup tables naming the
//! materials.

use cadcraft_engine::Session;
use cadcraft_engine::doc::foreign::{ForeignExtra, ForeignObject};
use cadcraft_engine::doc::{EntityKind, Handle, RawTag, Table};
use serde_json::{Value, json};

fn table(s: &Session, h: &str) -> Table {
    match &s.doc().unwrap().entity(Handle::parse_hex(h).unwrap()).unwrap().kind {
        EntityKind::Table(t) => t.clone(),
        _ => panic!("not a table"),
    }
}

fn texts(t: &Table) -> Vec<Vec<String>> {
    t.cells.iter().map(|r| r.iter().map(|c| c.text.clone()).collect()).collect()
}

fn block(s: &mut Session, name: &str, tags: &[&str]) {
    for (i, t) in tags.iter().enumerate() {
        s.execute("attdef", &json!({"tag": t, "default": "", "at": [0, i as f64], "height": 0.5})).unwrap();
    }
    s.execute("circle", &json!({"center": [0, 0], "radius": 1})).unwrap();
    let hs: Vec<String> = s.doc().unwrap().model.iter().map(|e| e.handle.hex()).collect();
    s.execute("block", &json!({"name": name, "base": [0, 0], "handles": hs, "keep": "delete"})).unwrap();
}

fn insert(s: &mut Session, name: &str, x: f64, attribs: Value) -> String {
    let r = s.execute("insert", &json!({"name": name, "at": [x, 50], "attribs": attribs})).unwrap();
    r["handle"].as_str().unwrap().to_string()
}

/// A wire: a polyline on layer `layer` in colour `color`, with XRECORDs (key, text) in its
/// extension dictionary, as an add-in in another CAD program writes them.
fn wire(s: &mut Session, layer: &str, color: i64, y: f64, data: &[(&str, &str)]) -> Handle {
    if s.doc().unwrap().layer(layer).is_none() {
        s.execute("layer.new", &json!({"name": layer})).unwrap();
    }
    let r = s.execute("pline", &json!({"vertices": [[0, y], [10, y]]})).unwrap();
    let h = Handle::parse_hex(r["handle"].as_str().unwrap()).unwrap();
    s.execute("properties.set", &json!({"handles": [h.hex()], "layer": layer, "color": color})).unwrap();
    let f = std::sync::Arc::make_mut(&mut s.doc_mut().unwrap().foreign);
    let base = 0x10000 + f.objects.len() as u64 * 16;
    let dict = Handle(base);
    let mut dict_tags = vec![RawTag { code: 100, value: "AcDbDictionary".into() }];
    for (i, (k, v)) in data.iter().enumerate() {
        let x = Handle(base + 1 + i as u64);
        dict_tags.push(RawTag { code: 3, value: k.to_string() });
        dict_tags.push(RawTag { code: 350, value: x.hex() });
        f.objects.push(ForeignObject {
            handle: x,
            kind: "XRECORD".into(),
            tags: vec![
                RawTag { code: 330, value: dict.hex() },
                RawTag { code: 100, value: "AcDbXrecord".into() },
                RawTag { code: 280, value: "1".into() },
                RawTag { code: 1000, value: v.to_string() },
            ],
        });
    }
    f.objects.push(ForeignObject { handle: dict, kind: "DICTIONARY".into(), tags: dict_tags });
    f.entities.insert(h, ForeignExtra { xdict: Some(dict), ..Default::default() });
    h
}

fn drawing() -> Session {
    let mut s = Session::new();
    block(&mut s, "KM", &["VAL1C"]);
    block(&mut s, "SB", &["VAL1C"]);
    block(&mut s, "FRAME", &[]);
    insert(&mut s, "KM", 0.0, json!({"VAL1C": "Контактор КМ-1"}));
    insert(&mut s, "KM", 10.0, json!({"VAL1C": "Контактор КМ-1"}));
    insert(&mut s, "SB", 20.0, json!({"VAL1C": "Кнопка"}));
    insert(&mut s, "FRAME", 30.0, json!({}));
    for y in [0.0, 2.0, 4.0] {
        wire(&mut s, "МПО 0.5", 2, y, &[("L", "120"), ("c1", "Н1"), ("c2", "Н1И"), ("N", "1")]);
    }
    wire(&mut s, "МПО 0.5", 2, 6.0, &[("L", "80;40"), ("c1", "К1"), ("c2", "Z12")]);
    wire(&mut s, "ПВ3 1.5", 1, 8.0, &[("L", "50")]);
    s
}

fn materials() -> Value {
    json!({
        "title": "Материалы",
        "columns": [{"header": "№", "width": 10}, {"header": "Наименование", "width": 100}, {"header": "Кол.", "width": 20}],
        "lookups": {
            "wires": {"rows": [
                {"type": "МПО 0.5", "color": "02", "val1c": "Провод МПО 0,5 желтый"},
                {"type": "ПВ3 1.5", "color": "01", "val1c": "Провод ПВ3 1,5 красный"}
            ]},
            "ends": {"rows": [
                {"val": "Н1", "val1C": "Наконечник Н1"},
                {"val": "Н1И", "val1C": "Наконечник Н1"},
                {"val": "К1", "val1C": "Наконечник К1"}
            ]}
        },
        "sections": [
            {"from": "insert", "fields": {"name": "{att.VAL1C}"}, "where": {"name": "?*"}, "group": ["name"], "row": ["{#}", "{name}", "{count}"]},
            {"from": "lwpolyline", "each": ["xr.c1", "xr.c2"],
             "fields": {"mat": {"lookup": "ends", "match": {"val": "{item}"}, "get": "val1c"}},
             "where": {"mat": "?*"}, "group": ["mat"], "row": ["{#}", "{mat}", "{count}"]},
            {"from": "lwpolyline", "each": ["xr.c1", "xr.c2"], "where": {"item": "*И"}, "group": [], "row": ["{#}", "Изолятор наконечника", "{count}"]},
            {"from": "lwpolyline", "fields": {"mat": {"lookup": "wires", "match": {"type": "{layer}", "color": "{color}"}, "get": "val1c"}},
             "where": {"xr.L": "?*"}, "group": ["mat"], "row": ["{#}", "{mat}", "{sum:xr.L} см"]}
        ]
    })
}

fn expected() -> Value {
    json!([
        ["1", "Кнопка", "1"],
        ["2", "Контактор КМ-1", "2"],
        ["3", "Наконечник К1", "1"],
        ["4", "Наконечник Н1", "6"],
        ["5", "Изолятор наконечника", "3"],
        ["6", "Провод МПО 0,5 желтый", "480 см"],
        ["7", "Провод ПВ3 1,5 красный", "50 см"]
    ])
}

#[test]
fn a_materials_list_from_attributes_xrecords_and_lookups() {
    let mut s = drawing();
    let undo = s.state().unwrap().undo.len();
    let r = s.execute("report", &json!({ "definition": materials() })).unwrap();
    assert_eq!(r["columns"], json!(["№", "Наименование", "Кол."]));
    assert_eq!(r["rows"], expected());
    // Without `at` or `table` nothing changes.
    assert_eq!(s.state().unwrap().undo.len(), undo);
}

#[test]
fn the_report_table_keeps_its_report_and_updates() {
    let mut s = drawing();
    let r = s.execute("report", &json!({ "definition": materials(), "at": [100, 0] })).unwrap();
    let h = r["table"].as_str().unwrap().to_string();
    let t = table(&s, &h);
    let rows = texts(&t);
    assert_eq!(rows[0][0], "Материалы");
    assert_eq!(rows[1], ["№", "Наименование", "Кол."]);
    assert_eq!(rows[2], ["1", "Кнопка", "1"]);
    assert_eq!(rows.len(), 2 + 7);
    assert_eq!(t.col_widths, vec![10.0, 100.0, 20.0]);
    assert_eq!(t.report.as_ref().map(|r| r.start_row), Some(2));

    // The user retitles the header and formats the first data row; then the drawing changes.
    s.execute("table.set", &json!({ "handle": h, "row": 1, "col": 2, "text": "Количество" })).unwrap();
    s.execute("tableformat", &json!({ "handle": h, "row": 2, "col": 1, "align": "ML" })).unwrap();
    insert(&mut s, "SB", 40.0, json!({"VAL1C": "Кнопка"}));
    insert(&mut s, "SB", 50.0, json!({"VAL1C": "Лампа"}));
    let r = s.execute("reportupdate", &json!({})).unwrap();
    assert_eq!(r["updated"], 1);
    let t = table(&s, &h);
    let rows = texts(&t);
    assert_eq!(rows[1], ["№", "Наименование", "Количество"]);
    assert_eq!(rows[2], ["1", "Кнопка", "2"]);
    assert_eq!(rows[4], ["3", "Лампа", "1"]);
    assert_eq!(rows.len(), 2 + 8);
    // Every new row takes the first data row's look.
    assert!(t.cells.iter().skip(2).all(|r| r[1].look.is_some_and(|l| l.align.is_some())));
    // Undo puts the old rows back.
    s.execute("u", &json!({})).unwrap();
    assert_eq!(texts(&table(&s, &h)).len(), 2 + 7);
}

#[test]
fn reports_read_definition_and_lookup_files() {
    let dir = std::env::temp_dir().join(format!("cadcraft-report-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("data")).unwrap();
    std::fs::write(dir.join("data/ends.csv"), "\u{feff}val;val1C\nН1;Наконечник Н1\nН1И;Наконечник Н1\nК1;Наконечник К1\n").unwrap();
    std::fs::write(dir.join("data/wires.json"), serde_json::to_string(&materials()["lookups"]["wires"]["rows"]).unwrap()).unwrap();
    let mut def = materials();
    def["lookups"] = json!({"ends": {"file": "data/ends.csv"}, "wires": {"file": "data/wires.json"}});
    let file = dir.join("materials.json");
    std::fs::write(&file, serde_json::to_string_pretty(&def).unwrap()).unwrap();

    let mut s = drawing();
    let csv = dir.join("out.csv");
    let r = s.execute("report", &json!({ "file": file.to_string_lossy(), "csv": csv.to_string_lossy() })).unwrap();
    assert_eq!(r["rows"], expected());
    let written = std::fs::read_to_string(&csv).unwrap();
    assert!(written.starts_with("№,Наименование,Кол.\r\n1,Кнопка,1\r\n"), "{written}");

    // Typed: the file, Enter for every object, the point.
    s.cmdline("report").unwrap();
    s.cmdline(&file.to_string_lossy()).unwrap();
    s.cmdline("").unwrap();
    s.cmdline("200,0").unwrap();
    let h = last_table(&s);
    let t = table(&s, &h);
    assert_eq!(texts(&t)[8], ["7", "Провод ПВ3 1,5 красный", "50 см"]);
    assert_eq!(t.report.as_ref().map(|r| r.file.clone()), Some(file.to_string_lossy().into_owned()));
    // The file changes: the update reads it again.
    def["sections"][3]["row"] = json!(["{#}", "{mat}", "{sum:xr.L:1} см"]);
    std::fs::write(&file, serde_json::to_string(&def).unwrap()).unwrap();
    s.execute("reportupdate", &json!({ "handles": [h] })).unwrap();
    assert_eq!(texts(&table(&s, &h))[8], ["7", "Провод ПВ3 1,5 красный", "50.0 см"]);
    // The file is gone: the definition the table kept is used.
    std::fs::remove_file(&file).unwrap();
    let r = s.execute("reportupdate", &json!({ "handles": [h] })).unwrap();
    assert_eq!(r["errors"], json!([]));
    // Without its lookup files the update says so and leaves the table as it was.
    std::fs::remove_dir_all(&dir).unwrap();
    let r = s.execute("reportupdate", &json!({ "handles": [h] })).unwrap();
    assert_eq!(r["updated"], 0);
    assert!(r["errors"][0].as_str().unwrap().contains("ends.csv"));
    assert_eq!(texts(&table(&s, &h))[8], ["7", "Провод ПВ3 1,5 красный", "50.0 см"]);
}

#[test]
fn typed_report_asks_for_the_objects_even_with_a_selection() {
    let mut s = drawing();
    let sb: Vec<Handle> =
        s.doc().unwrap().model.iter().filter(|e| matches!(&e.kind, EntityKind::Insert(i) if i.block == "SB")).map(|e| e.handle).collect();
    let def = json!({"from": "insert", "fields": {"name": "{att.VAL1C}"}, "where": {"name": "?*"}, "group": ["name"], "row": ["{name}", "{count}"]});
    let dir = std::env::temp_dir().join(format!("cadcraft-report-typed-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("parts.json");
    std::fs::write(&file, def.to_string()).unwrap();
    let rows = |s: &Session| texts(&table(s, &last_table(s))).len();
    // Selected before: offered, Enter takes it.
    s.set_selection(sb.clone());
    s.cmdline("report").unwrap();
    s.cmdline(&file.to_string_lossy()).unwrap();
    assert!(s.current_prompt().is_some_and(|p| p.message.starts_with("Select objects") && p.keywords == ["Selected"]));
    s.cmdline("").unwrap();
    s.cmdline("100,0").unwrap();
    assert_eq!(rows(&s), 1);
    assert!(s.selection().is_empty(), "the new table is not left selected");
    // All, though something is selected.
    s.set_selection(sb.clone());
    s.cmdline("report").unwrap();
    s.cmdline("").unwrap();
    // ALL selects every object, as at any Select objects prompt.
    s.cmdline("all").unwrap();
    s.cmdline("").unwrap();
    s.cmdline("200,0").unwrap();
    assert_eq!(rows(&s), 2);
    // The Selected option, from the menu.
    s.set_selection(sb.clone());
    s.cmdline("report").unwrap();
    s.cmdline("").unwrap();
    s.input(cadcraft_engine::Input::Keyword("Selected".into())).unwrap();
    s.cmdline("250,0").unwrap();
    assert_eq!(rows(&s), 1);
    // Nothing selected: pick objects, or Enter for all.
    s.cmdline("report").unwrap();
    s.cmdline("").unwrap();
    s.input(cadcraft_engine::Input::Pick(sb[..1].to_vec())).unwrap();
    s.cmdline("").unwrap();
    s.cmdline("300,0").unwrap();
    assert_eq!(rows(&s), 1);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn reports_read_only_the_given_objects() {
    let mut s = drawing();
    let km: Vec<String> = s
        .doc()
        .unwrap()
        .model
        .iter()
        .filter(|e| matches!(&e.kind, EntityKind::Insert(i) if i.block == "KM"))
        .map(|e| e.handle.hex())
        .take(1)
        .collect();
    let r = s.execute("report", &json!({ "definition": materials(), "handles": km })).unwrap();
    assert_eq!(r["rows"], json!([["1", "Контактор КМ-1", "1"]]));
}

#[test]
fn rows_without_grouping_sort_and_aggregate() {
    let mut s = drawing();
    let def = json!({
        "sections": [
            {"title": "Провода", "from": "lwpolyline", "sort": ["-xr.L"], "row": ["{xr.N}", "{layer}", "{xr.L}", "{length:1}"]},
            {"from": "lwpolyline", "group": [], "row": ["", "итого", "{sum:xr.L}", "{max:length}"]},
            {"from": "circle", "title": "Нет таких", "row": ["{#}"]}
        ]
    });
    let r = s.execute("report", &json!({ "definition": def })).unwrap();
    assert_eq!(
        r["rows"],
        json!([
            {"title": "Провода"},
            ["1", "МПО 0.5", "120", "10.0"],
            ["1", "МПО 0.5", "120", "10.0"],
            ["1", "МПО 0.5", "120", "10.0"],
            ["", "МПО 0.5", "80;40", "10.0"],
            ["", "ПВ3 1.5", "50", "10.0"],
            ["", "итого", "530", "10"]
        ])
    );
}

#[test]
fn bad_reports_are_errors() {
    let mut s = drawing();
    for p in [
        json!({}),
        json!({ "definition": "not json" }),
        json!({ "definition": {"row": "{x}", "fields": {"x": {"lookup": "none", "match": {}, "get": "y"}}} }),
        json!({ "file": "/no/such/report.json" }),
        json!({ "definition": {"row": ["{x}"]}, "at": "here" }),
        json!({ "definition": {"row": ["{x}"]}, "handles": "all" }),
    ] {
        assert!(s.execute("report", &p).is_err(), "{p}");
    }
    // Fields that refer to each other stop.
    let r = s.execute("report", &json!({ "definition": {"from": "insert", "fields": {"a": "{b}", "b": "{a}"}, "row": ["{a}"]} })).unwrap();
    assert_eq!(r["rows"].as_array().map(Vec::len), Some(4));
}

/// The handle of the last table drawn.
fn last_table(s: &Session) -> String {
    s.doc().unwrap().model.iter().filter(|e| matches!(e.kind, EntityKind::Table(_))).map(|e| e.handle).max().unwrap().hex()
}

#[test]
fn a_wire_list_with_sketches() {
    let mut s = drawing();
    let def = json!({
        "columns": [{"header": "No.", "width": 15}, {"header": "Sketch", "width": 100}, {"header": "Qty", "width": 15}],
        "from": "lwpolyline",
        "where": {"xr.L": "?*"},
        "group": ["layer", "color", "xr.L", "xr.c1", "xr.c2"],
        "sort": ["{min:xr.N}"],
        "row": ["{#}", {"sketch": {"ends": "{xr.c1};{xr.c2}", "lengths": "{xr.L}", "unit": " см", "insulated": "(изолир)"}}, "{count}"]
    });
    let r = s.execute("report", &json!({ "definition": def, "at": [0, -100] })).unwrap();
    // Replies and files get the sketch as text; identical wires make one row.
    let rows = r["rows"].as_array().unwrap().clone();
    assert!(rows.contains(&json!(["3", "Н1 —120 см— Н1И", "3"])), "{rows:?}");
    // Two sections, two nodes given: the last end is bare.
    assert!(rows.iter().any(|r| r[1] == "К1 —80 см— Z12 —40 см—"), "{rows:?}");
    let t = table(&s, r["table"].as_str().unwrap());
    let i = rows.iter().position(|r| r[2] == "3").unwrap();
    let cell = &t.cells[1 + i][1];
    let b = cell.block.as_ref().expect("a sketch block");
    assert!(cell.text.is_empty());
    assert!(b.scale.is_some_and(|k| (k - 100.0 * 0.94 / 120.0).abs() < 1e-9), "{:?}", b.scale);
    let d = s.doc().unwrap();
    let def_block = d.block(&b.name).expect("the sketch's block is made");
    assert!(def_block.entities.iter().any(|e| matches!(&e.kind, EntityKind::Text(x) if x.value == "(изолир)")));
    // The same wires again: the same block, not a new one.
    let blocks = d.blocks.len();
    s.execute("reportupdate", &json!({})).unwrap();
    assert_eq!(s.doc().unwrap().blocks.len(), blocks);
}
