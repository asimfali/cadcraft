//! TABLETEMPLATE: tables laid out by a table template (JSON), alone and for a report.

use cadcraft_engine::Session;
use cadcraft_engine::doc::{CellStyle, EntityKind, Handle, Lineweight, Table};
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

/// A parts list after GOST 2.106 form 1.
fn gost() -> Value {
    json!({
        "name": "GOST 2.106",
        "textStyle": {"name": "GOST", "font": "GOST type A"},
        "textHeight": 3.5,
        "margin": 1,
        "rowHeight": 8,
        "rowGrow": 8,
        "lines": {"outline": 0.7, "header": 0.7, "vertical": 0.7, "horizontal": 0.25},
        "header": {"height": 15, "textHeight": 2.5},
        "columns": [
            {"header": "Формат", "width": 6, "align": "MC", "headerRotation": 90},
            {"header": "Зона", "width": 6, "align": "MC", "headerRotation": 90},
            {"header": "Поз.", "width": 8, "align": "MC", "headerRotation": 90},
            {"header": "Обозначение", "width": 70, "align": "ML"},
            {"header": "Наименование", "width": 63, "align": "ML"},
            {"header": "Кол.", "width": 10, "align": "MC", "headerRotation": 90},
            {"header": "Приме-\\Pчание", "width": 22, "align": "ML"}
        ]
    })
}

#[test]
fn a_table_from_a_template() {
    let mut s = Session::new();
    let r = s.execute("tabletemplate", &json!({ "template": gost(), "at": [0, 0], "rows": 5 })).unwrap();
    let h = r["handle"].as_str().unwrap().to_string();
    let t = table(&s, &h);
    assert_eq!(t.col_widths, vec![6.0, 6.0, 8.0, 70.0, 63.0, 10.0, 22.0]);
    assert_eq!(t.row_heights, vec![15.0, 8.0, 8.0, 8.0, 8.0, 8.0]);
    assert_eq!(texts(&t)[0][4], "Наименование");
    assert!(!t.title && t.header);
    assert_eq!(t.style, "GOST 2.106");
    assert_eq!(t.row_grow, cadcraft_engine::doc::RowGrow::Step(8.0));
    // Header: rotated where asked, centred; data: by column.
    assert_eq!(t.cells[0][0].look.and_then(|l| l.rotation), Some(90));
    assert_eq!(t.cells[0][3].look.and_then(|l| l.align), Some(5));
    assert_eq!(t.cells[3][0].look.and_then(|l| l.align), Some(5));
    assert_eq!(t.cells[3][3].look.and_then(|l| l.align), Some(4));
    // The style carries the lines and the text.
    let d = s.doc().unwrap();
    let st = d.table_styles.iter().find(|x| x.name == "GOST 2.106").unwrap();
    let data = st.cell_style(CellStyle::DATA);
    assert_eq!(data.borders.inside_h.lineweight, Lineweight::Mm100(25));
    assert_eq!((data.borders.inside_v.lineweight, data.borders.bottom.lineweight), (Lineweight::Mm100(70), Lineweight::Mm100(70)));
    assert_eq!(data.text_style, "GOST");
    assert_eq!(st.cell_style(CellStyle::HEADER).text_height, 2.5);
    assert_eq!(d.text_style("GOST").map(|x| x.font.clone()).as_deref(), Some("GOST type A"));
    assert!(t.template.is_some());

    // A row added below the last one looks like it.
    s.execute("table.insertrow", &json!({ "handle": h })).unwrap();
    let t = table(&s, &h);
    assert_eq!(t.cells.len(), 7);
    assert_eq!(t.cells[6][0].look.and_then(|l| l.align), Some(5));
    assert_eq!(t.row_heights[6], 8.0);
}

#[test]
fn a_template_lays_out_an_existing_table() {
    let mut s = Session::new();
    let r = s.execute("table", &json!({ "at": [0, 0], "rows": 2, "cols": 2, "header": ["a", "b"], "cells": [["x1", "y1"], ["x2", "y2"]] })).unwrap();
    let h = r["handle"].as_str().unwrap().to_string();
    let tpl = json!({
        "name": "Two rows",
        "textHeight": 2.5,
        "title": {"text": "Wires", "height": 10},
        "header": {"rows": [["No.", "Ends", "<"], ["^", "From", "To"]], "heights": [8, 6]},
        "columns": [{"width": 10}, {"width": 30}, {"width": 30}]
    });
    s.execute("tabletemplate", &json!({ "template": tpl, "handle": h })).unwrap();
    let t = table(&s, &h);
    assert_eq!(texts(&t), vec![vec!["Wires", "", ""], vec!["No.", "Ends", ""], vec!["", "From", "To"], vec!["x1", "y1", ""], vec!["x2", "y2", ""]]);
    assert_eq!(t.cells[0][0].merged, Some((1, 3)));
    assert_eq!(t.cells[1][0].merged, Some((2, 1)));
    assert_eq!(t.cells[1][1].merged, Some((1, 2)));
    assert_eq!(t.row_styles, vec!["Title", "Header", "Header"]);
    assert_eq!(&t.row_heights[..3], &[10.0, 8.0, 6.0]);
    // Formatting a column touches its data rows only, under both header rows.
    s.execute("tableformat", &json!({ "handle": h, "row": 3, "col": 1, "scope": "column", "align": "BR" })).unwrap();
    let t = table(&s, &h);
    assert_eq!(t.cells[2][1].look.and_then(|l| l.align), Some(5));
    assert_eq!(t.cells[4][1].look.and_then(|l| l.align), Some(9));
    // Undo puts the plain table back.
    s.execute("u", &json!({})).unwrap();
    s.execute("u", &json!({})).unwrap();
    assert_eq!(texts(&table(&s, &h))[0], vec!["a", "b"]);
}

#[test]
fn a_report_takes_its_look_from_a_template() {
    let dir = std::env::temp_dir().join(format!("cadcraft-template-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("gost.json"), serde_json::to_string(&gost()).unwrap()).unwrap();
    let report = json!({
        "template": "gost.json",
        "from": "circle",
        "group": ["radius"],
        "row": ["", "", "{#}", "", "Ring R{radius}", "{count}"]
    });
    std::fs::write(dir.join("rings.json"), serde_json::to_string(&report).unwrap()).unwrap();
    let mut s = Session::new();
    for (x, r) in [(0, 1), (5, 1), (10, 2)] {
        s.execute("circle", &json!({ "center": [x, 0], "radius": r })).unwrap();
    }
    let r = s.execute("report", &json!({ "file": dir.join("rings.json").to_string_lossy(), "at": [0, 50] })).unwrap();
    assert_eq!(r["columns"][4], "Наименование");
    let h = r["table"].as_str().unwrap().to_string();
    let t = table(&s, &h);
    assert_eq!(texts(&t)[0][3], "Обозначение");
    assert_eq!(texts(&t)[1], ["", "", "1", "", "Ring R1", "2", ""]);
    assert_eq!(t.col_widths.len(), 7);
    assert_eq!(t.row_heights, vec![15.0, 8.0, 8.0]);
    assert_eq!(t.cells[1][2].look.and_then(|l| l.align), Some(5));
    assert_eq!(t.report.as_ref().map(|r| r.start_row), Some(1));
    // A new circle: the update adds a row with the same look.
    s.execute("circle", &json!({ "center": [20, 0], "radius": 3 })).unwrap();
    s.execute("reportupdate", &json!({})).unwrap();
    let t = table(&s, &h);
    assert_eq!(texts(&t)[3][4], "Ring R3");
    assert_eq!(t.cells[3][2].look.and_then(|l| l.align), Some(5));
    assert_eq!(t.row_heights[3], 8.0);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn typed_tabletemplate_inserts_or_lays_out_the_selected_table() {
    let dir = std::env::temp_dir().join(format!("cadcraft-template-typed-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("gost.json");
    std::fs::write(&file, serde_json::to_string(&gost()).unwrap()).unwrap();
    let mut s = Session::new();
    s.cmdline("tabletemplate").unwrap();
    s.cmdline(&file.to_string_lossy()).unwrap();
    s.cmdline("3").unwrap();
    s.cmdline("0,0").unwrap();
    let h = last_table(&s);
    assert_eq!(table(&s, &h).cells.len(), 4);
    // A plain table selected first is laid out by the template (the file is remembered).
    let r = s.execute("table", &json!({ "at": [0, 100], "rows": 2, "cols": 2 })).unwrap();
    let plain = r["handle"].as_str().unwrap().to_string();
    s.set_selection(vec![Handle::parse_hex(&plain).unwrap()]);
    s.cmdline("tabletemplate").unwrap();
    s.cmdline("").unwrap();
    assert!(!s.prompting());
    assert_eq!(table(&s, &plain).col_widths.len(), 7);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn bad_template_calls_are_errors() {
    let mut s = Session::new();
    for p in [
        json!({}),
        json!({ "template": gost() }),
        json!({ "template": {"columns": []}, "at": [0, 0] }),
        json!({ "template": gost(), "at": [0, 0], "rows": -1 }),
        json!({ "template": gost(), "handle": "FFFF" }),
        json!({ "file": "/no/such/template.json", "at": [0, 0] }),
    ] {
        assert!(s.execute("tabletemplate", &p).is_err(), "{p}");
    }
}

#[test]
fn bare_file_names_are_found_and_wrong_kinds_explained() {
    let dir = std::env::temp_dir().join(format!("cadcraft-template-find-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("gost.json"), serde_json::to_string(&gost()).unwrap()).unwrap();
    std::fs::write(dir.join("rings.json"), r#"{"template": "gost.json", "from": "circle", "row": ["{#}"]}"#).unwrap();
    let mut s = Session::new();
    // The folder of the last report file is searched for a bare name.
    s.last_used.report_file = dir.join("rings.json").to_string_lossy().into_owned();
    assert!(s.execute("report", &json!({ "file": "rings.json" })).is_ok());
    assert!(s.execute("tabletemplate", &json!({ "file": "gost.json", "at": [0, 0] })).is_ok());
    // A template given as a report, and a report as a template, say what they are.
    let e = s.execute("report", &json!({ "file": "gost.json" })).unwrap_err().to_string();
    assert!(e.contains("is a table template, not a report"), "{e}");
    let e = s.execute("tabletemplate", &json!({ "file": "rings.json", "at": [0, 0] })).unwrap_err().to_string();
    assert!(e.contains("is a report definition, not a table template"), "{e}");
    // The prompt offers the folders and the kind.
    s.cmdline("report").unwrap();
    match s.current_prompt().and_then(|p| p.picker) {
        Some(cadcraft_engine::Picker::OpenFile { dirs, kind, .. }) => {
            assert_eq!(kind, "report");
            assert_eq!(dirs.first().map(String::as_str), Some(dir.to_string_lossy().as_ref()));
        }
        other => panic!("{other:?}"),
    }
    std::fs::remove_dir_all(&dir).unwrap();
}

/// The handle of the last table drawn.
fn last_table(s: &Session) -> String {
    s.doc().unwrap().model.iter().filter(|e| matches!(e.kind, EntityKind::Table(_))).map(|e| e.handle).max().unwrap().hex()
}
