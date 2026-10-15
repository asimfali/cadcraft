//! Wire sketches for report tables (a wire list's "Sketch" column): a block drawn from a wire's
//! data — the terminal at each node, the length of each section between them — and shown in the
//! cell. The same data makes the same block, so identical wires share one definition.
//!
//! A node is a terminal code as wire lists write them: the first letter picks the drawing
//! (`Н`/`H` blade, `В`/`B` fork, `К`/`K` ring, `НВН` blade with a second contact), a final `И`
//! adds "(insulated)", `Z12` / `З12` is a stripped end ("strip 12 mm") and anything else is just
//! its label. Two nodes make a plain wire; more make a chain (`7;7;7` with four nodes), the
//! inner terminals standing on the wire. The drawings are CADCraft's own.

use cadcraft_doc::{Block, Circle, Drawing, Entity, EntityKind, HAlign, Line, Text, VAlign};
use cadcraft_geom::Vec3;

/// Most sections of one sketch.
const MAX_SECTIONS: usize = 20;
/// Text height in the sketch (block units, before the cell fits it).
const TEXT_H: f64 = 2.5;
/// Room at each end for the terminal and its notes.
const END: f64 = 24.0;

/// A wire's sketch: its nodes (terminal codes) and the lengths between them.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Sketch {
    pub nodes: Vec<String>,
    pub lengths: Vec<String>,
    /// Written after each length (" см").
    pub unit: String,
    /// The sketch's width (block units; the cell scales it).
    pub width: f64,
    /// Words of the notes: (insulated, strip, mm).
    pub words: (String, String, String),
}

/// What a terminal code draws.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Kind {
    Blade,
    Fork,
    Ring,
    DoubleBlade,
    Strip,
    Plain,
}

/// A code's drawing, label, insulation and strip length.
fn kind_of(code: &str) -> (Kind, String, bool, Option<String>) {
    let c = code.trim();
    let up = c.to_uppercase();
    if let Some(rest) = up.strip_prefix('Z').or_else(|| up.strip_prefix('З'))
        && !rest.is_empty()
        && rest.chars().all(|ch| ch.is_ascii_digit() || ch == ',' || ch == '.')
    {
        return (Kind::Strip, String::new(), false, Some(rest.to_string()));
    }
    let insulated = up.ends_with('И') && up.chars().count() > 1;
    let label: String = if insulated { c.chars().take(c.chars().count() - 1).collect() } else { c.to_string() };
    let l = label.to_uppercase();
    let kind = if l.starts_with("НВН") || l.starts_with("HBH") {
        Kind::DoubleBlade
    } else if l.starts_with('Н') || l.starts_with('H') {
        Kind::Blade
    } else if l.starts_with('В') || l.starts_with('B') {
        Kind::Fork
    } else if l.starts_with('К') || l.starts_with('K') {
        Kind::Ring
    } else {
        Kind::Plain
    };
    (kind, label, insulated, None)
}

impl Sketch {
    /// The sections drawn: as many as lengths (at least one), nodes padded with bare ends.
    fn sections(&self) -> usize {
        self.lengths.len().clamp(1, MAX_SECTIONS)
    }

    /// The name of its block: the same data, the same name.
    pub fn block_name(&self) -> String {
        // FNV-1a over everything that changes the drawing.
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        let key = format!("{:?}|{:?}|{}|{}|{:?}", self.nodes, self.lengths, self.unit, self.width, self.words);
        for b in key.bytes() {
            h ^= u64::from(b);
            h = h.wrapping_mul(0x0100_0000_01b3);
        }
        format!("SKETCH_{h:016X}")
    }

    /// A short text for files and replies: `Н1И —120 см— Н1И`.
    pub fn summary(&self) -> String {
        let n = self.sections();
        let mut out = self.nodes.first().cloned().unwrap_or_default();
        for i in 0..n {
            out.push_str(&format!(" —{}{}— ", self.lengths.get(i).map(String::as_str).unwrap_or(""), self.unit));
            out.push_str(self.nodes.get(i + 1).map(String::as_str).unwrap_or(""));
        }
        out.trim().to_string()
    }

    /// The objects of the sketch, in block units, the wire along y = 0 from x = 0.
    pub fn entities(&self, style: &str) -> Vec<EntityKind> {
        let n = self.sections();
        let width = if self.width.is_finite() && self.width > 2.0 * END + 10.0 { self.width.min(10_000.0) } else { 120.0 };
        let (x0, x1) = (END, width - END);
        let x_at = |i: usize| x0 + (x1 - x0) * i as f64 / n as f64;
        let mut out = Vec::new();
        let text = |at: (f64, f64), value: &str, h: HAlign, v: VAlign| {
            let p = Vec3::new(at.0, at.1, 0.0);
            EntityKind::Text(Text {
                insert: p,
                align_pt: Some(p),
                height: TEXT_H,
                value: value.to_string(),
                rotation: 0.0,
                width_factor: 1.0,
                oblique: 0.0,
                style: style.to_string(),
                halign: h,
                valign: v,
            })
        };
        // The wire and its lengths.
        out.push(line((x_at(0), 0.0), (x_at(n), 0.0)));
        for i in 0..n {
            if let Some(l) = self.lengths.get(i).filter(|l| !l.trim().is_empty()) {
                let mid = (x_at(i) + x_at(i + 1)) / 2.0;
                out.push(text((mid, 0.8), &format!("{}{}", l.trim(), self.unit), HAlign::Center, VAlign::Bottom));
            }
        }
        // The terminals.
        for i in 0..=n {
            let code = self.nodes.get(i).map(String::as_str).unwrap_or("");
            if code.trim().is_empty() {
                continue;
            }
            let x = x_at(i);
            let dir = if i == 0 {
                -1.0
            } else if i == n {
                1.0
            } else {
                0.0
            };
            self.terminal(&mut out, code, x, dir, &text);
        }
        out
    }

    /// Draw terminal `code` at node `x`: pointing left (`dir` -1), right (1) or up (0).
    fn terminal(&self, out: &mut Vec<EntityKind>, code: &str, x: f64, dir: f64, text: &dyn Fn((f64, f64), &str, HAlign, VAlign) -> EntityKind) {
        let (kind, label, insulated, strip) = kind_of(code);
        let (ins_w, strip_w, mm) = (&self.words.0, &self.words.1, &self.words.2);
        if dir == 0.0 {
            // On the wire, pointing up: a sleeve and the contact above it.
            out.extend(rect((x - 0.6, 0.0), (x + 0.6, 1.5)));
            match kind {
                Kind::Ring => out.push(circle((x, 3.5), 2.0)),
                Kind::Fork => {
                    out.push(line((x - 1.5, 1.5), (x + 1.5, 1.5)));
                    out.push(line((x - 1.5, 1.5), (x - 1.5, 5.0)));
                    out.push(line((x + 1.5, 1.5), (x + 1.5, 5.0)));
                }
                Kind::Strip | Kind::Plain => {}
                Kind::Blade | Kind::DoubleBlade => out.extend(rect((x - 1.5, 1.5), (x + 1.5, 5.5))),
            }
            if !label.is_empty() {
                out.push(text((x, 6.2), &label, HAlign::Center, VAlign::Bottom));
            }
            if insulated {
                out.push(text((x + 2.5, -1.0), ins_w, HAlign::Left, VAlign::Top));
            }
            return;
        }
        // At an end: the contact points away from the wire.
        let p = |dx: f64| x + dir * dx;
        let mut far = 0.0;
        match kind {
            Kind::Strip => {
                out.push(line((p(-2.0), -1.2), (p(-2.0), 1.2)));
                let t = format!("{strip_w} {} {mm}", strip.unwrap_or_default());
                let h = if dir < 0.0 { HAlign::Right } else { HAlign::Left };
                out.push(text((p(1.0), -TEXT_H / 2.0), t.trim(), h, VAlign::Bottom));
                return;
            }
            Kind::Plain => {}
            _ => {
                out.extend(rect((p(0.0), -0.6), (p(1.5), 0.6)));
                far = 1.5;
            }
        }
        match kind {
            Kind::Blade => {
                out.extend(rect((p(1.5), -1.5), (p(5.5), 1.5)));
                far = 5.5;
            }
            Kind::DoubleBlade => {
                out.extend(rect((p(1.5), -1.5), (p(5.5), 1.5)));
                out.extend(rect((p(5.5), -0.8), (p(7.5), 0.8)));
                far = 7.5;
            }
            Kind::Fork => {
                out.push(line((p(1.5), -1.5), (p(1.5), 1.5)));
                out.push(line((p(1.5), 1.5), (p(5.5), 1.5)));
                out.push(line((p(1.5), -1.5), (p(5.5), -1.5)));
                far = 5.5;
            }
            Kind::Ring => {
                out.push(circle((p(3.5), 0.0), 2.0));
                far = 5.5;
            }
            Kind::Strip | Kind::Plain => {}
        }
        let mid = p(far / 2.0);
        if !label.is_empty() {
            out.push(text((mid, 2.2), &label, HAlign::Center, VAlign::Bottom));
        }
        if insulated {
            let h = if dir < 0.0 { HAlign::Right } else { HAlign::Left };
            out.push(text((p(far + 1.0), -TEXT_H / 2.0), ins_w, h, VAlign::Bottom));
        }
    }
}

fn line(a: (f64, f64), b: (f64, f64)) -> EntityKind {
    EntityKind::Line(Line { a: Vec3::new(a.0, a.1, 0.0), b: Vec3::new(b.0, b.1, 0.0) })
}

fn circle(c: (f64, f64), r: f64) -> EntityKind {
    EntityKind::Circle(Circle { center: Vec3::new(c.0, c.1, 0.0), radius: r })
}

fn rect(a: (f64, f64), b: (f64, f64)) -> [EntityKind; 4] {
    [line(a, (b.0, a.1)), line((b.0, a.1), b), line(b, (a.0, b.1)), line((a.0, b.1), a)]
}

/// Make the block of `s` in `d` unless it is there; its name.
pub fn ensure_block(d: &mut Drawing, s: &Sketch, style: &str) -> String {
    let name = s.block_name();
    if d.block(&name).is_none() {
        let mut b = Block::new(&name);
        b.description = s.summary();
        for k in s.entities(style) {
            let h = d.new_handle();
            b.entities.push(Entity::new(h, k));
        }
        d.blocks.insert(name.clone(), std::sync::Arc::new(b));
    }
    name
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sk(nodes: &[&str], lengths: &[&str]) -> Sketch {
        Sketch {
            nodes: nodes.iter().map(|s| s.to_string()).collect(),
            lengths: lengths.iter().map(|s| s.to_string()).collect(),
            unit: " см".into(),
            width: 120.0,
            words: ("(изолир)".into(), "зачист".into(), "мм".into()),
        }
    }

    fn texts(s: &Sketch) -> Vec<String> {
        s.entities("Standard")
            .into_iter()
            .filter_map(|k| match k {
                EntityKind::Text(t) => Some(t.value),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn codes_pick_the_terminal() {
        assert_eq!(kind_of("Н1И"), (Kind::Blade, "Н1".into(), true, None));
        assert_eq!(kind_of("В1"), (Kind::Fork, "В1".into(), false, None));
        assert_eq!(kind_of("K2").0, Kind::Ring);
        assert_eq!(kind_of("НВН").0, Kind::DoubleBlade);
        assert_eq!(kind_of("Z12"), (Kind::Strip, String::new(), false, Some("12".into())));
        assert_eq!(kind_of("НКИ-6").0, Kind::Blade);
        assert_eq!(kind_of("X1:L1").0, Kind::Plain);
    }

    #[test]
    fn a_wire_shows_its_ends_and_length() {
        let t = texts(&sk(&["Н1И", "В1"], &["60"]));
        assert_eq!(t, ["60 см", "Н1", "(изолир)", "В1"]);
        let t = texts(&sk(&["Z12", "Н1"], &["19"]));
        assert!(t.contains(&"зачист 12 мм".to_string()), "{t:?}");
        // A chain: three sections, four terminals.
        let chain = sk(&["В1", "В1", "В1", "В1"], &["7", "7", "7"]);
        assert_eq!(texts(&chain).iter().filter(|x| *x == "7 см").count(), 3);
        assert_eq!(texts(&chain).iter().filter(|x| *x == "В1").count(), 4);
        assert_eq!(chain.summary(), "В1 —7 см— В1 —7 см— В1 —7 см— В1");
        // Same data, same block; other data, another.
        assert_eq!(sk(&["Н1"], &["5"]).block_name(), sk(&["Н1"], &["5"]).block_name());
        assert_ne!(sk(&["Н1"], &["5"]).block_name(), sk(&["Н1"], &["6"]).block_name());
        // Hostile: no lengths, many sections.
        let many: Vec<String> = (0..500).map(|i| i.to_string()).collect();
        let s = Sketch { lengths: many, ..sk(&[], &[]) };
        assert!(s.entities("Standard").len() < 200);
    }
}
