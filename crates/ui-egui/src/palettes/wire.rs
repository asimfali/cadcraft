//! The Properties palette's Wire section: with wires selected (polylines or lines), their wire
//! list data — length, the terminals at both ends, number, type and colour — shown and edited for
//! all of them at once (`*VARIES*` where they differ), as WIRE does at the command line. The
//! lists come from `wires.json` and the drawing (`wire.choices`).

use cadcraft_engine::cmd::wire::{self, Chain, Choices, WireData};
use cadcraft_engine::doc::Handle;
use serde_json::{Value, json};

use super::prop_row;
use crate::CadApp;

const VARIES: &str = "*VARIES*";

/// The choices, worked out once per drawing revision.
fn choices(ctx: &egui::Context, app: &CadApp) -> Choices {
    let Ok(st) = app.session.state() else { return Choices::default() };
    let id = egui::Id::unique(("wire-choices", st.uid, st.revision));
    if let Some(c) = ctx.data(|d| d.get_temp::<Choices>(id)) {
        return c;
    }
    let c = wire::choices(&app.session);
    ctx.data_mut(|d| d.insert_temp(id, c.clone()));
    c
}

/// The hover tip of wire `h`: its number, length, terminals, type and colour (`None` for an
/// object without wire data).
pub(crate) fn tip(app: &CadApp, ctx: &egui::Context, h: Handle) -> Option<String> {
    let w = wire::data(app.session.doc().ok()?, h)?;
    if w.length.is_empty() && w.c1.is_empty() && w.c2.is_empty() && w.number.is_empty() {
        return None;
    }
    let c = choices(ctx, app);
    let mut parts = Vec::new();
    let head = if w.number.is_empty() { crate::i18n::t("Wire").to_string() } else { format!("{} {}", crate::i18n::t("Wire"), w.number) };
    parts.push(head);
    if !w.length.is_empty() {
        parts.push(w.length.clone());
    }
    if !w.c1.is_empty() || !w.c2.is_empty() {
        parts.push(format!("{} → {}", w.c1, w.c2));
    }
    parts.push(w.layer.clone());
    parts.push(c.color_name(w.color));
    Some(parts.join(" · "))
}

/// What a wire end is attached to: "block : connector", or "—".
fn link_text(d: &cadcraft_engine::doc::Drawing, l: Option<&cadcraft_engine::connect::Link>) -> String {
    let Some(l) = l else { return "—".into() };
    match d.entity(l.insert).map(|e| &e.kind) {
        Some(cadcraft_engine::doc::EntityKind::Insert(i)) => format!("{} : {}", i.block, l.name),
        _ => l.name.clone(),
    }
}

/// A text field with a ▾ list beside it: typed text commits on Enter or leaving the field, a
/// list item at once.
pub(super) fn combo_field(ui: &mut egui::Ui, id: egui::Id, value: &str, items: &[String]) -> Option<String> {
    let mut buf = ui.data_mut(|d| d.get_temp::<String>(id)).unwrap_or_else(|| value.to_string());
    let te = id.with("te");
    let w = (ui.available_width() - 38.0).max(40.0);
    let resp = ui.add_sized([w, 20.0], egui::TextEdit::singleline(&mut buf).id(te).font(crate::theme::body()));
    if ui.memory(|m| m.has_focus(te)) {
        ui.data_mut(|d| d.insert_temp(id, buf.clone()));
    } else {
        ui.data_mut(|d| d.remove::<String>(id));
    }
    let mut out = (resp.lost_focus() && buf != value && buf != VARIES).then(|| buf.trim().to_string());
    if !items.is_empty() {
        ui.menu_button("▾", |ui| {
            egui::ScrollArea::vertical().max_height(320.0).show(ui, |ui| {
                for it in items {
                    if ui.button(it).clicked() {
                        out = Some(it.clone());
                        ui.close();
                    }
                }
            });
        });
    }
    out
}

/// What the wires share for one field, or `*VARIES*`.
fn shared(ws: &[WireData], f: impl Fn(&WireData) -> String) -> String {
    let vals: Vec<String> = ws.iter().map(f).collect();
    match vals.first() {
        Some(v) if vals.iter().all(|x| x == v) => v.clone(),
        _ => VARIES.into(),
    }
}

/// Draw the Wire section for the selected `wires` (header drawn by `group`); returns the
/// `wire.set` parameters of a change.
pub(super) fn section(app: &CadApp, ui: &mut egui::Ui, wires: &[Handle], group: &dyn Fn(&mut egui::Ui, &str)) -> Option<Value> {
    let d = app.session.doc().ok()?;
    let ws: Vec<WireData> = wires.iter().filter_map(|h| wire::data(d, *h)).collect();
    if ws.is_empty() {
        return None;
    }
    let c = choices(ui.ctx(), app);
    let ids: Vec<String> = wires.iter().map(|h| h.hex()).collect();
    let key = |k: &str| egui::Id::unique(("wire-field", k, ids.clone()));
    let mut set: Option<Value> = None;
    let mut put = |k: &str, v: Value| set = Some(json!({ "handles": ids, k: v }));
    group(ui, "Wire");
    // What one wire's ends are attached to (block connectors).
    if let [h] = wires
        && let Some(e) = d.entity(*h)
    {
        let [start, end] = cadcraft_engine::connect::links(e);
        for (label, l) in [("Start", start), ("End", end)] {
            prop_row(ui, label, |ui| {
                ui.label(link_text(d, l.as_ref()));
            });
        }
    }
    // One chain the wires share (all the same sections), else `None`.
    let chain = ws.first().map(Chain::of).filter(|c0| ws.iter().all(|w| Chain::of(w) == *c0));
    let mut new_chain: Option<Chain> = None;
    match &chain {
        // Several sections: node, section length, node… with + and − for sections.
        Some(ch) if ch.sections() > 1 => {
            let mut edited = ch.clone();
            let mut changed = false;
            for i in 0..ch.nodes.len() {
                let node = ch.nodes.get(i).cloned().unwrap_or_default();
                prop_row(ui, &format!("{} {}", crate::i18n::t("Node"), i + 1), |ui| {
                    if let Some(v) = combo_field(ui, key(&format!("node{i}")), &node, &c.terminals)
                        && let Some(n) = edited.nodes.get_mut(i)
                    {
                        *n = v;
                        changed = true;
                    }
                });
                if let Some(len) = ch.lengths.get(i).cloned() {
                    prop_row(ui, &format!("↓ {}", crate::i18n::t("Length")), |ui| {
                        if let Some(v) = combo_field(ui, key(&format!("len{i}")), &len, &[])
                            && let Some(l) = edited.lengths.get_mut(i)
                        {
                            *l = v;
                            changed = true;
                        }
                    });
                }
            }
            prop_row(ui, "Sections", |ui| {
                if ui.button(crate::tl!("+ Section")).clicked() {
                    edited.add();
                    changed = true;
                }
                if ui.button(crate::tl!("− Section")).clicked() {
                    edited.remove();
                    changed = true;
                }
            });
            if changed {
                new_chain = Some(edited);
            }
        }
        _ => {
            let fields: [(&str, &str, String, &[String]); 3] = [
                ("Length", "length", shared(&ws, |w| w.length.clone()), &c.lengths),
                ("Terminal 1", "c1", shared(&ws, |w| w.c1.clone()), &c.terminals),
                ("Terminal 2", "c2", shared(&ws, |w| w.c2.clone()), &c.terminals),
            ];
            for (label, k, value, items) in fields {
                prop_row(ui, label, |ui| {
                    if let Some(v) = combo_field(ui, key(k), &value, items) {
                        put(k, json!(v));
                    }
                });
            }
            // A plain wire becomes a chain: one more section.
            if let Some(ch) = &chain {
                prop_row(ui, "Sections", |ui| {
                    if ui.button(crate::tl!("+ Section")).on_hover_text(crate::tl!("More than two terminals: a chain of sections")).clicked() {
                        let mut more = ch.clone();
                        more.add();
                        new_chain = Some(more);
                    }
                });
            }
        }
    }
    let number = shared(&ws, |w| w.number.clone());
    prop_row(ui, "Number", |ui| {
        if let Some(v) = combo_field(ui, key("number"), &number, &[]) {
            put("number", json!(v));
        }
    });
    // Type: the layer (the list's types, else the drawing's layers).
    let types: Vec<String> = if c.types.is_empty() { d.layers.iter().map(|l| l.name.clone()).collect() } else { c.types.clone() };
    let ty = shared(&ws, |w| w.layer.clone());
    prop_row(ui, "Type", |ui| {
        if let Some(v) = combo_field(ui, key("type"), &ty, &types) {
            put("type", json!(v));
        }
    });
    // Colour: the list's names, else the standard colours.
    let colour = shared(&ws, |w| c.color_name(w.color));
    let names: Vec<(i16, String)> = if c.colors.is_empty() {
        [(1, "Red"), (2, "Yellow"), (3, "Green"), (4, "Cyan"), (5, "Blue"), (6, "Magenta"), (7, "White"), (250, "Color 250")]
            .into_iter()
            .map(|(a, n)| (a, crate::i18n::t(n).to_string()))
            .collect()
    } else {
        c.colors.clone()
    };
    prop_row(ui, "Color", |ui| {
        ui.menu_button(if colour == VARIES { crate::i18n::t(VARIES).to_string() } else { colour.clone() }, |ui| {
            egui::ScrollArea::vertical().max_height(320.0).show(ui, |ui| {
                for (aci, n) in &names {
                    if ui.button(n).clicked() {
                        put("color", json!(aci));
                        ui.close();
                    }
                }
            });
        });
    });
    // A chain's sections changed: all three fields at once.
    if let Some(ch) = new_chain {
        let (length, c1, c2) = ch.fields();
        return Some(json!({ "handles": ids, "length": length, "c1": c1, "c2": c2 }));
    }
    set
}
