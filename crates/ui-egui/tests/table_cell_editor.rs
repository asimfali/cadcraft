//! Double-clicking a table cell opens the in-place text editor on the cell with its text, as in
//! AutoCAD: Enter or Tab keeps the text and goes on to the next cell, Esc ends; a stacked
//! fraction shows stacked.

use cadcraft_engine::Session;
use cadcraft_engine::doc::{EntityKind, Handle, Table};
use cadcraft_engine::geom::Vec2;
use cadcraft_ui_egui::{CadApp, Services};
use egui::{Event, Key, Modifiers, PointerButton, Pos2, vec2};
use egui_kittest::Harness;
use serde_json::json;

const FRACTION: &str = "Лист \\SБ-ПН-2,0 ГОСТ 19903-2015/Ст3сп ГОСТ 14637-89;";

fn harness() -> (Harness<'static, CadApp>, Handle) {
    let mut app = CadApp::new(Session::new(), Services::default());
    let r = app
        .run("table", json!({ "at": [0, 0], "rows": 2, "cols": 2, "rowHeight": 8.0, "colWidth": 80.0, "cells": [[FRACTION, "1,2"], ["", ""]] }))
        .unwrap();
    let h = Handle::parse_hex(r["handle"].as_str().unwrap()).unwrap();
    app.session.execute("tableformat", &json!({ "handle": h.hex(), "row": 0, "col": 0, "scope": "table", "textHeight": 3.5 })).unwrap();
    app.session.execute("view.set", &json!({ "center": [45, -10], "height": 40 })).unwrap();
    app.canvas.zoom_pending = false;
    let mut harness = Harness::builder().with_size(vec2(1600.0, 1000.0)).with_step_dt(1.0 / 60.0).build_ui_state(
        |ui, app: &mut CadApp| {
            app.logic(ui.ctx());
            app.ui(ui);
        },
        app,
    );
    harness.run_steps(5);
    harness.state_mut().session.execute("view.set", &json!({ "center": [45, -10], "height": 40 })).unwrap();
    harness.state_mut().canvas.zoom_pending = false;
    harness.run_steps(3);
    (harness, h)
}

fn double_click(h: &mut Harness<'_, CadApp>, at: Vec2) {
    let pos: Pos2 = h.state().canvas.xf.unwrap().to_screen(at);
    h.event(Event::PointerMoved(pos));
    h.run_steps(2);
    for _ in 0..2 {
        for pressed in [true, false] {
            h.event(Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE });
            h.run_steps(1);
        }
    }
    h.run_steps(3);
}

fn table(h: &Harness<'_, CadApp>, t: Handle) -> Table {
    match &h.state().session.doc().unwrap().entity(t).unwrap().kind {
        EntityKind::Table(x) => x.clone(),
        _ => panic!("not a table"),
    }
}

fn key(h: &mut Harness<'_, CadApp>, k: Key) {
    h.event(Event::Key { key: k, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE });
    h.event(Event::Key { key: k, physical_key: None, pressed: false, repeat: false, modifiers: Modifiers::NONE });
    h.run_steps(3);
}

#[test]
fn a_cell_is_edited_in_place_and_enter_goes_on() {
    let (mut h, t) = harness();
    double_click(&mut h, Vec2::new(40.0, -4.0));
    let s = &h.state().session;
    assert_eq!(s.running.as_ref().map(|r| r.id.as_str()), Some("tabledit"));
    let m = s.editing_mtext().expect("the editor has the cell");
    assert_eq!(m.contents, FRACTION);
    assert!((m.height - 3.5).abs() < 1e-9, "{}", m.height);
    if let Ok(dir) = std::env::var("CRAFT_UI_VISUAL_DIR") {
        std::fs::create_dir_all(&dir).unwrap();
        h.render().unwrap().save(format!("{dir}/table-cell-editor.png")).unwrap();
    }
    // Enter keeps the text as it was (the fraction too) and edits the next cell.
    key(&mut h, Key::Enter);
    assert_eq!(table(&h, t).cells[0][0].text, FRACTION);
    let m = h.state().session.editing_mtext().expect("next cell");
    assert_eq!(m.contents, "1,2");
    // Tab on; Esc ends.
    key(&mut h, Key::Tab);
    assert_eq!(h.state().session.editing_mtext().map(|m| m.contents), Some(String::new()));
    key(&mut h, Key::Escape);
    assert!(h.state().session.running.is_none());
    assert_eq!(table(&h, t).cells[0][1].text, "1,2");
}
