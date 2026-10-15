//! A wire end dragged near a block's connector snaps onto it and attaches; moving the block
//! then drags the wire along.

use cadcraft_engine::Session;
use cadcraft_engine::doc::{EntityKind, Handle};
use cadcraft_engine::geom::Vec2;
use cadcraft_ui_egui::{CadApp, Services};
use egui::{Event, Modifiers, PointerButton, Pos2, vec2};
use egui_kittest::Harness;
use serde_json::json;

fn h(v: &serde_json::Value) -> Handle {
    Handle::parse_hex(v.as_str().unwrap()).unwrap()
}

/// Block "K" with connector "A1" at (0,10), inserted at (100,0); a wire (50,10)–(90,10), selected.
fn harness() -> (Harness<'static, CadApp>, Handle, Handle) {
    let mut app = CadApp::new(Session::new(), Services::default());
    let bx = app.run("pline", json!({"vertices": [[0, 0], [10, 0], [10, 20], [0, 20]], "closed": true})).unwrap();
    let a1 = app.run("connector", json!({"at": [0, 10], "name": "A1"})).unwrap();
    app.run("block", json!({"name": "K", "base": [0, 0], "handles": [bx["handle"], a1["handle"]], "keep": "delete"})).unwrap();
    let k = h(&app.run("insert", json!({"name": "K", "at": [100, 0]})).unwrap()["handle"]);
    let w = h(&app.run("pline", json!({"vertices": [[50, 10], [90, 10]]})).unwrap()["handle"]);
    let mut hh = Harness::builder().with_size(vec2(1400.0, 900.0)).with_step_dt(1.0 / 60.0).build_ui_state(
        |ui, app: &mut CadApp| {
            app.logic(ui.ctx());
            app.ui(ui);
        },
        app,
    );
    hh.run_steps(3);
    hh.state_mut().session.execute("view.set", &json!({ "center": [80, 10], "height": 60 })).unwrap();
    hh.state_mut().canvas.zoom_pending = false;
    hh.state_mut().session.set_selection(vec![w]);
    hh.run_steps(3);
    (hh, k, w)
}

fn end(hh: &Harness<'_, CadApp>, w: Handle) -> (f64, f64) {
    match &hh.state().session.doc().unwrap().entity(w).unwrap().kind {
        EntityKind::LwPolyline(p) => p.vertices.last().map(|v| (v.p.x, v.p.y)).unwrap(),
        _ => panic!("not a polyline"),
    }
}

fn screen(hh: &Harness<'_, CadApp>, x: f64, y: f64) -> Pos2 {
    hh.state().canvas.xf.unwrap().to_screen(Vec2::new(x, y))
}

fn button(hh: &mut Harness<'_, CadApp>, pos: Pos2, pressed: bool) {
    hh.event(Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE });
    hh.run_steps(1);
}

#[test]
fn a_wire_end_dragged_near_a_connector_attaches_to_it() {
    let (mut hh, k, w) = harness();
    let g = screen(&hh, 90.0, 10.0);
    hh.event(Event::PointerMoved(g));
    hh.run_steps(2);
    button(&mut hh, g, true);
    // Let go a little off the connector (100,10): the snap puts the end on it.
    let to = screen(&hh, 99.6, 10.3);
    for i in 1..=10 {
        hh.event(Event::PointerMoved(g + (to - g) * (i as f32 / 10.0)));
        hh.run_steps(1);
    }
    button(&mut hh, to, false);
    hh.run_steps(2);
    let e = end(&hh, w);
    assert!((e.0 - 100.0).abs() < 1e-9 && (e.1 - 10.0).abs() < 1e-9, "{e:?}");
    let r = hh.state_mut().session.execute("wire.get", &json!({"handles": [w.hex()]})).unwrap();
    assert_eq!(r[0]["end"]["name"], "A1");
    // The block moves: the wire follows.
    hh.state_mut().session.execute("move", &json!({"handles": [k.hex()], "delta": [5, 5]})).unwrap();
    assert_eq!(end(&hh, w), (105.0, 15.0));
}
