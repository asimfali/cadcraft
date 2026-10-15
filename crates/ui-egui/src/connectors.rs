//! Block connectors on the canvas (see `cadcraft_engine::connect`): a small ring with its name at
//! each connector of the selected block references, and at each connector placed in the space
//! being edited (the Block Editor shows the ones it is making).

use egui::{Color32, Painter, Pos2, Rect, Stroke, vec2};

use crate::CadApp;
use crate::canvas::Xf;
use crate::theme::Tokens;
use cadcraft_engine::connect;
use cadcraft_engine::doc::EntityKind;

/// Most connectors drawn (a selection of many references).
const MAX_DRAWN: usize = 2000;

/// The ring and name of one connector.
fn mark(p: &Painter, at: Pos2, name: &str, color: Color32) {
    let r = crate::theme::snap_marker_half() * 0.8;
    p.circle_stroke(at, r, Stroke::new(1.5, color));
    p.circle_filled(at, r * 0.35, color);
    if !name.is_empty() {
        p.text(at + vec2(r + 3.0, -r - 1.0), egui::Align2::LEFT_BOTTOM, name, crate::theme::small(), color);
    }
}

/// Draw the connectors of the selected references and of the space being edited.
pub(crate) fn draw(app: &CadApp, painter: &Painter, xf: &Xf, rect: Rect) {
    let Ok(d) = app.session.doc() else { return };
    let space = app.session.space();
    let Some(store) = d.space(&space) else { return };
    let color = Tokens::get().snap;
    let mut left = MAX_DRAWN;
    let mut put = |name: &str, w: cadcraft_engine::geom::Vec2| {
        let at = xf.to_screen(w);
        if left > 0 && rect.contains(at) {
            left -= 1;
            mark(painter, at, name, color);
        }
    };
    for h in app.session.selection() {
        let Some(e) = store.get(h) else { continue };
        for (name, w) in connect::connectors_of(d, e) {
            put(&name, w);
        }
    }
    // Connector points of this space: only looked for while a few are drawn anyway (the Block
    // Editor's content, or a drawing being made into a block).
    if store.len() <= 20_000 {
        for e in store.iter() {
            if let (Some(name), EntityKind::Point(pt)) = (connect::connector_name(e), &e.kind) {
                put(&name, pt.p.xy());
            }
        }
    }
}

/// The snap tip of a connector at `w`: "Connector A1".
pub(crate) fn snap_label(app: &CadApp, w: cadcraft_engine::geom::Vec2, aperture: f64) -> Option<String> {
    let d = app.session.doc().ok()?;
    let c = connect::connector_at(d, &app.session.space(), w, aperture.max(1e-9) * 1e-3)?;
    Some(format!("{} {}", crate::i18n::t("Connector"), c.name))
}
