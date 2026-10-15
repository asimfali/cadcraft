//! Wires: a wire list's data on polylines (XRECORDs L, c1, c2, N; type = layer, colour), set and
//! read by WIRE / wire.set, numbered by WIRENUM, kept in DXF and read by reports.

use cadcraft_engine::Session;
use cadcraft_engine::doc::Handle;
use serde_json::{Value, json};

fn pline(s: &mut Session, y: f64) -> String {
    let r = s.execute("pline", &json!({"vertices": [[0, y], [50, y], [50, y + 5.0]]})).unwrap();
    r["handle"].as_str().unwrap().to_string()
}

fn get(s: &mut Session, h: &str) -> Value {
    s.execute("wire.get", &json!({ "handles": [h] })).unwrap()[0].clone()
}

#[test]
fn wire_data_is_set_read_undone_and_kept_in_dxf() {
    let mut s = Session::new();
    let (a, b) = (pline(&mut s, 0.0), pline(&mut s, 10.0));
    s.execute("wire.set", &json!({ "handles": [a, b], "length": "60", "c1": "Н1И", "c2": "В1", "type": "МПО 0.5", "color": 2 })).unwrap();
    s.execute("wire.set", &json!({ "handles": [b], "length": "40;35" })).unwrap();
    assert_eq!(get(&mut s, &a), json!({"handle": a, "length": "60", "c1": "Н1И", "c2": "В1", "number": "", "type": "МПО 0.5", "color": 2}));
    assert_eq!(get(&mut s, &b)["length"], "40;35");
    assert!(s.doc().unwrap().layer("МПО 0.5").is_some(), "the type's layer is made");
    // Undo takes the last change back.
    s.execute("u", &json!({})).unwrap();
    assert_eq!(get(&mut s, &b)["length"], "60");
    // Saved and read back: the XRECORDs are there, under the object, as an add-in reads them.
    let text = String::from_utf8(cadcraft_io::write(s.doc().unwrap(), "w.dxf").unwrap()).unwrap();
    assert!(text.contains("ACAD_XDICTIONARY") && text.contains("AcDbXrecord"));
    let back = cadcraft_io::read(text.as_bytes(), "w.dxf").unwrap();
    let ha = Handle::parse_hex(&a).unwrap();
    assert_eq!(back.xrecord_text(ha, "L").as_deref(), Some("60"));
    assert_eq!(back.xrecord_text(ha, "c1").as_deref(), Some("Н1И"));
    // Saved again: still one dictionary per wire, no lost records.
    let again = cadcraft_io::read(&cadcraft_io::write(&back, "w.dxf").unwrap(), "w.dxf").unwrap();
    assert_eq!(again.xrecord_text(ha, "c2").as_deref(), Some("В1"));
    // Not wires: refused.
    let c = s.execute("circle", &json!({"center": [0, 0], "radius": 1})).unwrap()["handle"].as_str().unwrap().to_string();
    assert!(s.execute("wire.set", &json!({ "handles": [c], "length": "1" })).is_err());
}

#[test]
fn typed_wire_and_wirenum() {
    let mut s = Session::new();
    let hs: Vec<String> = (0..3).map(|i| pline(&mut s, i as f64 * 10.0)).collect();
    // WIRE on the selection: a value is the length, keywords the other fields.
    s.set_selection(hs.iter().map(|h| Handle::parse_hex(h).unwrap()).collect());
    s.cmdline("wd").unwrap();
    s.cmdline("75").unwrap();
    s.cmdline("s").unwrap();
    s.cmdline("Н1И").unwrap();
    s.cmdline("color").unwrap();
    s.cmdline("green").unwrap();
    // The prompt shows what they share.
    assert_eq!(s.current_prompt().and_then(|p| p.default).as_deref(), Some("75"));
    s.cmdline("").unwrap();
    assert!(!s.prompting());
    for h in &hs {
        let w = get(&mut s, h);
        assert_eq!((w["length"].as_str(), w["c1"].as_str(), w["color"].as_i64()), (Some("75"), Some("Н1И"), Some(3)));
    }
    // WIRENUM: the first number, then one pick per wire.
    s.set_selection(Vec::new());
    s.cmdline("wn").unwrap();
    assert_eq!(s.current_prompt().and_then(|p| p.default).as_deref(), Some("1"));
    s.cmdline("5").unwrap();
    for h in [&hs[2], &hs[0]] {
        s.input(cadcraft_engine::Input::Pick(vec![Handle::parse_hex(h).unwrap()])).unwrap();
    }
    s.cmdline("").unwrap();
    assert_eq!(get(&mut s, &hs[2])["number"], "5");
    assert_eq!(get(&mut s, &hs[0])["number"], "6");
    // The next free number follows the highest.
    s.cmdline("wn").unwrap();
    assert_eq!(s.current_prompt().and_then(|p| p.default).as_deref(), Some("7"));
    s.cmdline("").unwrap();
    s.input(cadcraft_engine::Input::Pick(vec![Handle::parse_hex(&hs[1]).unwrap()])).unwrap();
    s.cmdline("").unwrap();
    assert_eq!(get(&mut s, &hs[1])["number"], "7");
}

#[test]
fn choices_come_from_wires_json_and_the_drawing() {
    let dir = std::env::temp_dir().join(format!("cadcraft-wires-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("wires.json"),
        json!({"terminals": ["Н1", "В1"], "types": ["МПО 0.5"], "colors": [{"aci": 250, "name": "черный"}], "lengths": ["0;7;7;7"]}).to_string(),
    )
    .unwrap();
    let mut s = Session::new();
    s.last_used.report_file = dir.join("r.json").to_string_lossy().into_owned();
    let h = pline(&mut s, 0.0);
    s.execute("wire.set", &json!({ "handles": [h], "length": "10", "c2": "К2;Z12", "color": "черный" })).unwrap();
    assert_eq!(get(&mut s, &h)["color"], 250, "a colour by its wires.json name");
    let c = s.execute("wire.choices", &json!({})).unwrap();
    assert_eq!(c["terminals"], json!(["Н1", "В1", "К2", "Z12"]));
    assert_eq!(c["types"], json!(["МПО 0.5", "0"]));
    assert_eq!(c["colors"], json!([{"aci": 250, "name": "черный"}]));
    assert_eq!(c["lengths"], json!(["0;7;7;7"]));
    std::fs::remove_dir_all(&dir).unwrap();
}
