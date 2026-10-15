//! Connectors: named points of a block where wires attach; attached wire ends follow the block.

use cadcraft_engine::Session;
use cadcraft_engine::connect;
use cadcraft_engine::doc::{EntityKind, Handle, RawTag};
use cadcraft_engine::geom::Vec2;
use serde_json::{Value, json};

fn h(v: &Value) -> Handle {
    Handle::parse_hex(v.as_str().unwrap()).unwrap()
}

/// Block "K" (a 10×20 box) with connectors "A1" at (0,10) and "A2" at (10,10), inserted at
/// (100,0). Returns the reference.
fn contactor(s: &mut Session) -> Handle {
    let bx = s.execute("pline", &json!({"vertices": [[0, 0], [10, 0], [10, 20], [0, 20]], "closed": true})).unwrap();
    let a1 = s.execute("connector", &json!({"at": [0, 10], "name": "A1"})).unwrap();
    let a2 = s.execute("connector", &json!({"at": [10, 10], "name": "A2"})).unwrap();
    s.execute("block", &json!({"name": "K", "base": [0, 0], "handles": [bx["handle"], a1["handle"], a2["handle"]], "keep": "delete"})).unwrap();
    let r = s.execute("insert", &json!({"name": "K", "at": [100, 0]})).unwrap();
    h(&r["handle"])
}

fn verts(s: &Session, w: Handle) -> Vec<(f64, f64)> {
    match &s.doc().unwrap().entity(w).unwrap().kind {
        EntityKind::LwPolyline(p) => p.vertices.iter().map(|v| (v.p.x, v.p.y)).collect(),
        EntityKind::Line(l) => vec![(l.a.x, l.a.y), (l.b.x, l.b.y)],
        k => panic!("not a wire: {k:?}"),
    }
}

fn near(a: &[(f64, f64)], b: &[(f64, f64)]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(p, q)| (p.0 - q.0).abs() < 1e-9 && (p.1 - q.1).abs() < 1e-9)
}

fn link(s: &mut Session, w: Handle) -> (Value, Value) {
    let r = s.execute("wire.get", &json!({"handles": [w.hex()]})).unwrap();
    (r[0]["start"].clone(), r[0]["end"].clone())
}

fn wire(s: &mut Session, pts: Value) -> Handle {
    let r = s.execute("pline", &json!({ "vertices": pts })).unwrap();
    h(&r["handle"])
}

#[test]
fn a_connector_is_a_named_point_of_the_block() {
    let mut s = Session::new();
    let k = contactor(&mut s);
    let r = s.execute("connector.list", &json!({"handles": [k.hex()]})).unwrap();
    assert_eq!(r[0]["block"], "K");
    let cs = r[0]["connectors"].as_array().unwrap();
    assert_eq!(cs.len(), 2);
    assert_eq!(cs[0]["name"], "A1");
    assert_eq!(cs[0]["at"], json!([100.0, 10.0]));
    assert_eq!(cs[1]["at"], json!([110.0, 10.0]));
}

#[test]
fn connector_names_count_up() {
    let mut s = Session::new();
    let a = s.execute("connector", &json!({"at": [0, 0]})).unwrap();
    let b = s.execute("connector", &json!({"at": [5, 0]})).unwrap();
    assert_eq!(a["name"], "1");
    assert_eq!(b["name"], "2");
    // Renaming, and back to a plain point.
    s.execute("connector.set", &json!({"handle": b["handle"], "name": "PE"})).unwrap();
    let e = s.doc().unwrap().entity(h(&b["handle"])).unwrap().clone();
    assert_eq!(connect::connector_name(&e).as_deref(), Some("PE"));
    s.execute("connector.set", &json!({"handle": b["handle"], "remove": true})).unwrap();
    let e = s.doc().unwrap().entity(h(&b["handle"])).unwrap().clone();
    assert_eq!(connect::connector_name(&e), None);
    assert!(e.common.xdata.is_empty());
}

#[test]
fn a_wire_drawn_to_a_connector_is_attached() {
    let mut s = Session::new();
    let k = contactor(&mut s);
    let w = wire(&mut s, json!([[50, 30], [50, 10], [100, 10]]));
    let (start, end) = link(&mut s, w);
    assert!(start.is_null());
    assert_eq!(end["handle"], k.hex());
    assert_eq!(end["block"], "K");
    assert_eq!(end["name"], "A1");
}

#[test]
fn moving_the_block_drags_the_wire_keeping_right_angles() {
    let mut s = Session::new();
    let k = contactor(&mut s);
    let w = wire(&mut s, json!([[50, 30], [50, 10], [100, 10]]));
    s.execute("move", &json!({"handles": [k.hex()], "delta": [20, 5]})).unwrap();
    // The end is on A1 again; the vertex before it moved up with it, so the last segment stays
    // horizontal and the one before it vertical.
    assert!(near(&verts(&s, w), &[(50.0, 30.0), (50.0, 15.0), (120.0, 15.0)]), "{:?}", verts(&s, w));
    // One undo puts both back.
    s.execute("u", &json!({})).unwrap();
    assert!(near(&verts(&s, w), &[(50.0, 30.0), (50.0, 10.0), (100.0, 10.0)]), "{:?}", verts(&s, w));
}

#[test]
fn rotating_the_block_turns_the_connector_and_the_wire_follows() {
    let mut s = Session::new();
    let k = contactor(&mut s);
    let w = wire(&mut s, json!([[50, 10], [100, 10]]));
    s.execute("rotate", &json!({"handles": [k.hex()], "base": [100, 0], "angle": 90})).unwrap();
    // A1 (0,10) of a reference at (100,0) turned 90°: (90, 0).
    assert!(near(&verts(&s, w), &[(50.0, 10.0), (90.0, 0.0)]), "{:?}", verts(&s, w));
    assert_eq!(link(&mut s, w).1["name"], "A1");
}

#[test]
fn a_wire_between_two_blocks_follows_both() {
    let mut s = Session::new();
    let k = contactor(&mut s);
    let k2 = h(&s.execute("insert", &json!({"name": "K", "at": [200, 0]})).unwrap()["handle"]);
    // From K's A2 (110,10) to the second one's A1 (200,10).
    let w = wire(&mut s, json!([[110, 10], [150, 10], [150, 10], [200, 10]]));
    let (start, end) = link(&mut s, w);
    assert_eq!((start["handle"].clone(), start["name"].clone()), (json!(k.hex()), json!("A2")));
    assert_eq!((end["handle"].clone(), end["name"].clone()), (json!(k2.hex()), json!("A1")));
    s.execute("move", &json!({"handles": [k2.hex()], "delta": [0, 30]})).unwrap();
    assert!(near(&verts(&s, w), &[(110.0, 10.0), (150.0, 10.0), (150.0, 40.0), (200.0, 40.0)]), "{:?}", verts(&s, w));
}

#[test]
fn moving_the_end_off_the_connector_lets_go() {
    let mut s = Session::new();
    let _k = contactor(&mut s);
    let w = wire(&mut s, json!([[50, 10], [100, 10]]));
    s.execute("move", &json!({"handles": [w.hex()], "delta": [0, -5]})).unwrap();
    assert!(link(&mut s, w).1.is_null());
    assert!(s.doc().unwrap().entity(w).unwrap().common.xdata.is_empty());
}

#[test]
fn moving_block_and_wire_together_keeps_the_link() {
    let mut s = Session::new();
    let k = contactor(&mut s);
    let w = wire(&mut s, json!([[50, 10], [100, 10]]));
    s.execute("move", &json!({"handles": [k.hex(), w.hex()], "delta": [7, 7]})).unwrap();
    assert!(near(&verts(&s, w), &[(57.0, 17.0), (107.0, 17.0)]), "{:?}", verts(&s, w));
    assert_eq!(link(&mut s, w).1["handle"], k.hex());
}

#[test]
fn a_copied_wire_alone_is_not_attached_but_a_copy_with_its_block_is() {
    let mut s = Session::new();
    let k = contactor(&mut s);
    let w = wire(&mut s, json!([[50, 10], [100, 10]]));
    let c = s.execute("copy", &json!({"handles": [w.hex()], "delta": [0, 50]})).unwrap();
    let wc = h(&c["handles"][0]);
    assert!(link(&mut s, wc).1.is_null());
    let c = s.execute("copy", &json!({"handles": [k.hex(), w.hex()], "delta": [0, 100]})).unwrap();
    let copies: Vec<Handle> = c["handles"].as_array().unwrap().iter().map(h).collect();
    let (kc, wc) = (copies[0], copies[1]);
    assert_eq!(link(&mut s, wc).1["handle"], kc.hex());
    // The original stays on the original.
    assert_eq!(link(&mut s, w).1["handle"], k.hex());
}

#[test]
fn erasing_the_block_leaves_the_wire_unattached() {
    let mut s = Session::new();
    let k = contactor(&mut s);
    let w = wire(&mut s, json!([[50, 10], [100, 10]]));
    s.execute("erase", &json!({"handles": [k.hex()]})).unwrap();
    assert!(near(&verts(&s, w), &[(50.0, 10.0), (100.0, 10.0)]));
    assert!(link(&mut s, w).1.is_null());
}

#[test]
fn a_line_is_a_wire_too() {
    let mut s = Session::new();
    let k = contactor(&mut s);
    let w = h(&s.execute("line", &json!({"points": [[50, 10], [100, 10]]})).unwrap()["handles"][0]);
    s.execute("move", &json!({"handles": [k.hex()], "delta": [0, 3]})).unwrap();
    assert!(near(&verts(&s, w), &[(50.0, 10.0), (100.0, 13.0)]), "{:?}", verts(&s, w));
}

#[test]
fn connectors_and_links_survive_dxf() {
    let mut s = Session::new();
    let k = contactor(&mut s);
    let w = wire(&mut s, json!([[50, 10], [100, 10]]));
    let dxf = cadcraft_io::write_dxf(s.doc().unwrap());
    let back = cadcraft_io::read_dxf(dxf.as_bytes()).unwrap();
    let blk = connect::block_connectors(&back, "K");
    assert_eq!(blk.iter().map(|(n, _)| n.as_str()).collect::<Vec<_>>(), ["A1", "A2"]);
    let we = back.entity(w).unwrap();
    let [_, end] = connect::links(we);
    assert_eq!(end, Some(connect::Link { insert: k, name: "A1".into() }));
}

#[test]
fn a_block_saved_in_the_block_editor_moves_its_wires() {
    let mut s = Session::new();
    let _k = contactor(&mut s);
    let w = wire(&mut s, json!([[50, 10], [100, 10]]));
    s.execute("bedit", &json!({"name": "K"})).unwrap();
    let list = s.execute("connector.list", &json!({})).unwrap();
    let a1 = list.as_array().unwrap().iter().find(|c| c["connectors"][0]["name"] == "A1").unwrap()["handle"].clone();
    s.execute("move", &json!({"handles": [a1], "delta": [0, 4]})).unwrap();
    s.execute("bclose", &json!({"save": true})).unwrap();
    assert!(near(&verts(&s, w), &[(50.0, 10.0), (100.0, 14.0)]), "{:?}", verts(&s, w));
    assert!(!link(&mut s, w).1.is_null());
}

#[test]
fn snapping_finds_a_connector_before_other_points() {
    let mut s = Session::new();
    let _k = contactor(&mut s);
    let d = s.doc().unwrap();
    let mode = cadcraft_engine::snap::mode::END | cadcraft_engine::snap::mode::CONN;
    // (100,9) is 1 from A1 and 9 from the box corner (100,0): the connector wins well within reach.
    let hit = cadcraft_engine::snap::osnap(d, &Default::default(), Vec2::new(100.5, 9.0), 3.0, mode, None, false).unwrap();
    assert_eq!(hit.mode, cadcraft_engine::snap::mode::CONN);
    assert_eq!(hit.point, Vec2::new(100.0, 10.0));
    assert_eq!(hit.name, "Connector");
}

#[test]
fn hostile_link_data_does_not_panic() {
    let mut s = Session::new();
    let _k = contactor(&mut s);
    let w = wire(&mut s, json!([[50, 10], [100, 10]]));
    let junk = vec![
        RawTag { code: 1001, value: connect::LINK_APP.into() },
        RawTag { code: 1000, value: "end".into() },
        RawTag { code: 1005, value: "ZZZZ".into() },
        RawTag { code: 1000, value: "start".into() },
        RawTag { code: 1000, value: "end".into() },
        RawTag { code: 1005, value: "FFFFFFFFFFFF".into() },
        RawTag { code: 1000, value: "nowhere".into() },
        RawTag { code: 1000, value: "start".into() },
    ];
    s.doc_mut().unwrap().modify_entity(w, |e| e.common.xdata = junk).unwrap();
    let _ = s.execute("wire.get", &json!({"handles": [w.hex()]})).unwrap();
    s.execute("move", &json!({"handles": [w.hex()], "delta": [1, 1]})).unwrap();
    s.execute("move", &json!({"handles": [w.hex()], "delta": [-1, -1]})).unwrap();
    assert_eq!(link(&mut s, w).1["name"], "A1");
}
