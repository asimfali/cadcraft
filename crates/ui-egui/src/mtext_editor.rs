//! The in-place MTEXT editor: while MTEXT asks for its text, a text box opens on the canvas at
//! the text's location and width with a formatting toolbar (font, height, bold, italic,
//! underline, overline, colour, alignment, lists, tabs) and a ruler for the paragraph's indents
//! and tab stops. The text and its formats live in the engine's
//! [`cadcraft_engine::mtext_edit::Formatted`]; this file only draws and forwards edits.

use std::sync::Arc;

use cadcraft_engine::mtext_edit::{CharFmt, Flag, Formatted, ListStyle, MTextColor, ParaAlign, TabKind};
use egui::text::{CCursor, CCursorRange, LayoutJob};
use egui::{Align2, Color32, FontId, Rect, RichText, Sense, Shape, Stroke, TextFormat, pos2, vec2};

use crate::CadApp;
use crate::theme::Tokens;

/// Smallest and largest size of the text in the box, in points: the box stays readable at any zoom.
const MIN_PX: f32 = 13.0;
const MAX_PX: f32 = 36.0;
/// Families the font list offers besides the one set in the text.
const FAMILIES: [&str; 6] = ["Arial", "Times New Roman", "Courier New", "Verdana", "Georgia", "Calibri"];
/// The standard colours (ACI 1..=7) of the colour menu.
const SWATCHES: [u16; 7] = [1, 2, 3, 4, 5, 6, 7];
/// Ruler snap, in text heights.
const SNAP: f64 = 0.25;
/// Size of a stacked fraction's text against the line's.
const STACK_SCALE: f32 = 0.7;

/// The room a stacked fraction takes in the text box: its wider part and a little space.
fn stack_width(fs: &mut egui::epaint::FontsView<'_>, top: &str, bottom: &str, small: &FontId, size: f32) -> f32 {
    let wt = fs.layout_no_wrap(top.to_string(), small.clone(), Color32::WHITE).size().x;
    let wb = fs.layout_no_wrap(bottom.to_string(), small.clone(), Color32::WHITE).size().x;
    wt.max(wb) + size * 0.3
}

#[derive(Clone)]
struct State {
    fmt: Formatted,
    /// Format of the next typed character when nothing is selected.
    typing: CharFmt,
    /// Caret the `typing` format was set for.
    typing_at: usize,
    /// Height of the toolbar and ruler above the text box (last frame).
    top: f32,
    /// Selection (characters) of the last frame, for toolbar actions of this one.
    sel: (usize, usize),
    /// A caret position to set on the text box in the next frame (after a toolbar insertion).
    caret: Option<usize>,
    /// Where the edited text is (its insertion point): another one (TABLEDIT's next cell) starts
    /// the editor afresh.
    at: (f64, f64),
    /// The text box takes the keyboard on the editor's first frame.
    focus: bool,
}

fn state_id() -> egui::Id {
    egui::Id::new("mtext_editor_state")
}

fn edit_id() -> egui::Id {
    egui::Id::new("mtext_editor_text")
}

/// The ACI index or true colour as a screen colour.
fn color32(c: MTextColor) -> Color32 {
    let rgb = match c {
        MTextColor::Aci(v) => cadcraft_color::aci_rgb(u8::try_from(v).unwrap_or(7)),
        MTextColor::Rgb(v) => cadcraft_color::Rgb((v >> 16) as u8, (v >> 8) as u8, v as u8),
    };
    Color32::from_rgb(rgb.0, rgb.1, rgb.2)
}

/// Show the editor while MTEXT collects its text; clears its state when it isn't running.
pub fn show(app: &mut CadApp, ctx: &egui::Context) {
    let mtext = app.session.editing_mtext();
    let asking = app.session.current_prompt().is_some_and(|p| p.accept.text && !p.accept.point);
    let (Some(mt), true) = (mtext, asking) else {
        ctx.data_mut(|d| d.remove::<State>(state_id()));
        return;
    };
    let t = Tokens::get();
    let (rect, scale) = app.canvas.xf.map_or((ctx.content_rect(), 20.0), |x| (x.rect, x.scale.max(1e-6)));
    // A table cell (TABLEDIT): Enter and Tab keep the text and go on to the next cell,
    // Alt+Enter breaks the line, as in AutoCAD's table cells.
    let cell = app.session.running.as_ref().is_some_and(|r| r.id != "mtext");
    let at = (mt.insert.x, mt.insert.y);
    let saved = ctx.data_mut(|d| d.get_temp::<State>(state_id())).filter(|s| s.at == at);
    let mut st = saved.unwrap_or_else(|| {
        // Existing contents (a table cell's) are edited; MTEXT starts empty.
        let mut fmt = if mt.contents.is_empty() { Formatted::default() } else { Formatted::from_contents(&mt.contents) };
        if let Some(f) = app.session.doc().ok().and_then(|d| d.text_style(&mt.style).map(|s| s.font.clone())) {
            // A family name, not a font file (`txt.shx`, `arial.ttf`).
            if !f.is_empty() && !f.contains('.') {
                fmt.base_font = f;
            }
        }
        let end = fmt.len();
        State { fmt, typing: CharFmt::default(), typing_at: 0, top: 70.0, sel: (end, end), caret: Some(end), at, focus: true }
    });
    let base_h = if mt.height > 0.0 { mt.height } else { 0.2 };
    let px = ((base_h * scale) as f32).clamp(MIN_PX, MAX_PX);
    let width = if mt.width > 0.0 { ((mt.width * scale) as f32).clamp(280.0, rect.width().max(280.0) * 0.9) } else { 380.0 };
    // The box's top-left corner on screen (rotation is ignored while editing).
    let code = usize::from(mt.attach.clamp(1, 9) - 1);
    let (col, row) = ((code % 3) as f64, (code / 3) as f64);
    let ww = f64::from(width) / scale;
    let corner = cadcraft_geom::Vec2::new(mt.insert.x - col * ww / 2.0, mt.insert.y + row * base_h * 2.0);
    let at = app.canvas.xf.map_or(rect.center(), |x| x.to_screen(corner));
    let max_x = (rect.right() - width - 8.0).max(rect.left());
    let pos = pos2(at.x.clamp(rect.left(), max_x), at.y.clamp(rect.top() + st.top, (rect.bottom() - 120.0).max(rect.top() + st.top)));

    let mut done = Done::No;
    if cell {
        let next = ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Tab) || i.consume_key(egui::Modifiers::NONE, egui::Key::Enter));
        if next {
            done = Done::Ok;
        }
    }
    let top_start = pos.y - st.top;
    egui::Area::new(egui::Id::new("mtext_editor_area")).order(egui::Order::Foreground).fixed_pos(pos2(pos.x, top_start)).show(ctx, |ui| {
        egui::Frame::NONE.fill(t.panel).stroke(Stroke::new(1.0, t.accent)).inner_margin(egui::Margin::same(0)).show(ui, |ui| {
            ui.set_width(width);
            let y0 = ui.cursor().top();
            toolbar(ui, &mut st, base_h);
            ruler(ui, &mut st, px, width, base_h);
            fraction_row(ui, &mut st);
            st.top = ui.cursor().top() - y0;
            text_box(ui, &mut st, base_h, px, width, &t);
            ui.horizontal(|ui| {
                ui.add_space(4.0);
                if ui.button(crate::tl!("OK")).clicked() {
                    done = Done::Ok;
                }
                if ui.button(crate::tl!("Cancel")).clicked() {
                    done = Done::Cancel;
                }
                let finish = if cell {
                    crate::tl!("Enter or Tab: next cell, Alt+Enter: new line, Esc: done")
                } else if ui.ctx().os().is_mac() {
                    crate::tl!("⌘↩ to finish")
                } else {
                    crate::tl!("Ctrl+Enter to finish")
                };
                ui.label(RichText::new(finish).small().color(t.text_dim));
            });
        });
    });
    if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        done = Done::Cancel;
    } else if ctx.input(|i| i.modifiers.command && i.key_pressed(egui::Key::Enter)) {
        done = Done::Ok;
    }
    match done {
        Done::No => {
            ctx.data_mut(|d| d.insert_temp(state_id(), st));
        }
        Done::Ok => {
            ctx.data_mut(|d| d.remove::<State>(state_id()));
            // The editor holds the whole contents: one input, then Enter ends MTEXT's text entry.
            // A table cell takes its text even when emptied.
            if cell {
                if let Err(e) = app.session.input(cadcraft_engine::Input::Text(st.fmt.to_contents())) {
                    app.session.echo(e.to_string());
                }
            } else if !st.fmt.is_empty() {
                let _ = app.session.input(cadcraft_engine::Input::Text(st.fmt.to_contents()));
            }
            if app.session.running.as_ref().is_some_and(|r| r.id == "mtext") {
                let _ = app.session.input(cadcraft_engine::Input::Enter);
            }
        }
        Done::Cancel => {
            ctx.data_mut(|d| d.remove::<State>(state_id()));
            app.session.cancel();
        }
    }
}

#[derive(PartialEq)]
enum Done {
    No,
    Ok,
    Cancel,
}

/// Change the selected characters' format, or the next typed ones' when nothing is selected.
fn change(st: &mut State, f: impl Fn(&mut CharFmt)) {
    let (a, b) = st.sel;
    if a == b {
        f(&mut st.typing);
    } else {
        st.fmt.edit(a.min(b)..a.max(b), f);
    }
}

fn current(st: &State) -> CharFmt {
    let (a, b) = st.sel;
    if a == b { st.typing.clone() } else { st.fmt.fmt_at(a.min(b)) }
}

fn toggle(st: &mut State, flag: Flag) {
    let (a, b) = st.sel;
    if a == b {
        let on = !flag_of(&st.typing, flag);
        change(st, |c| set_flag(c, flag, on));
    } else {
        st.fmt.toggle(a.min(b)..a.max(b), flag);
    }
}

fn flag_of(c: &CharFmt, f: Flag) -> bool {
    match f {
        Flag::Bold => c.bold,
        Flag::Italic => c.italic,
        Flag::Underline => c.underline,
        Flag::Overline => c.overline,
    }
}

fn set_flag(c: &mut CharFmt, f: Flag, on: bool) {
    match f {
        Flag::Bold => c.bold = on,
        Flag::Italic => c.italic = on,
        Flag::Underline => c.underline = on,
        Flag::Overline => c.overline = on,
    }
}

fn toolbar(ui: &mut egui::Ui, st: &mut State, base_h: f64) {
    let cur = current(st);
    let range = st.sel.0.min(st.sel.1)..st.sel.0.max(st.sel.1);
    let mut refocus = false;
    ui.horizontal_wrapped(|ui| {
        ui.add_space(4.0);
        // Font
        let shown = cur.font.clone().unwrap_or_else(|| st.fmt.base_font.clone());
        egui::ComboBox::from_id_salt("mtext_font").width(120.0).selected_text(shown.clone()).show_ui(ui, |ui| {
            for fam in FAMILIES.iter().map(|s| s.to_string()).chain(cur.font.clone().filter(|f| !FAMILIES.contains(&f.as_str()))) {
                if ui.selectable_label(shown == fam, &fam).clicked() {
                    change(st, |c| c.font = Some(fam.clone()));
                    refocus = true;
                }
            }
        });
        // Height
        let mut h = cur.height.unwrap_or(base_h);
        if ui
            .add(egui::DragValue::new(&mut h).speed(base_h * 0.02).range(base_h * 0.05..=base_h * 50.0).max_decimals(4))
            .on_hover_text(crate::tl!("Text height"))
            .changed()
        {
            let none = (h - base_h).abs() < 1e-9;
            change(st, |c| c.height = (!none).then_some(h));
        }
        ui.separator();
        for (flag, label, tip) in [
            (Flag::Bold, RichText::new("B").strong(), crate::tl!("Bold")),
            (Flag::Italic, RichText::new("I").italics(), crate::tl!("Italic")),
            (Flag::Underline, RichText::new("U").underline(), crate::tl!("Underline")),
            (Flag::Overline, RichText::new("O").strikethrough(), crate::tl!("Overline")),
        ] {
            if ui.selectable_label(flag_of(&cur, flag), label).on_hover_text(tip).clicked() {
                toggle(st, flag);
                refocus = true;
            }
        }
        // Colour
        let swatch = cur.color.map_or(ui.visuals().text_color(), color32);
        ui.menu_button(RichText::new("A▾").color(swatch).strong(), |ui| {
            if ui.button(crate::tl!("ByLayer")).clicked() {
                change(st, |c| c.color = None);
                refocus = true;
                ui.close();
            }
            ui.horizontal(|ui| {
                for v in SWATCHES {
                    let (r, resp) = ui.allocate_exact_size(vec2(18.0, 18.0), Sense::click());
                    ui.painter().rect_filled(r, 2.0, color32(MTextColor::Aci(v)));
                    if resp.clicked() {
                        change(st, |c| c.color = Some(MTextColor::Aci(v)));
                        refocus = true;
                        ui.close();
                    }
                }
            });
            let mut rgb = cur.color.map_or([255, 255, 255], |c| {
                let k = color32(c);
                [k.r(), k.g(), k.b()]
            });
            if ui.color_edit_button_srgb(&mut rgb).changed() {
                let v = (u32::from(rgb[0]) << 16) | (u32::from(rgb[1]) << 8) | u32::from(rgb[2]);
                change(st, |c| c.color = Some(MTextColor::Rgb(v)));
            }
        })
        .response
        .on_hover_text(crate::tl!("Text color"));
        ui.separator();
        // Paragraph alignment
        let para = st.fmt.para(st.fmt.paragraph_of(range.start));
        for (al, glyph, tip) in [
            (ParaAlign::Left, "⇤", crate::tl!("Left")),
            (ParaAlign::Center, "↔", crate::tl!("Center")),
            (ParaAlign::Right, "⇥", crate::tl!("Right")),
            (ParaAlign::Justify, "≡", crate::tl!("Justify")),
        ] {
            let on = para.align == al;
            if ui.selectable_label(on, glyph).on_hover_text(tip).clicked() {
                st.fmt.edit_paras(range.clone(), |p| p.align = if on { ParaAlign::Default } else { al });
                refocus = true;
            }
        }
        ui.separator();
        // Lists and special characters
        if ui.button("•≡").on_hover_text(crate::tl!("Bulleted list")).clicked() {
            st.fmt.make_list(range.clone(), ListStyle::Bullet, 3.0 * base_h);
            refocus = true;
        }
        if ui.button("1≡").on_hover_text(crate::tl!("Numbered list")).clicked() {
            st.fmt.make_list(range.clone(), ListStyle::Number, 3.0 * base_h);
            refocus = true;
        }
        // Stack: the selected `a/b` (or `a#b`, `a^b`) becomes a fraction, a selected fraction
        // (or the one before the caret) text again.
        if ui.button("a⁄b").on_hover_text(crate::tl!("Stack")).clicked() {
            let chars: Vec<char> = st.fmt.text().chars().collect();
            let stacked: Vec<usize> = if range.is_empty() {
                range.start.checked_sub(1).filter(|i| chars.get(*i).is_some_and(|c| st.fmt.stack_of(*c).is_some())).into_iter().collect()
            } else {
                range.clone().filter(|i| chars.get(*i).is_some_and(|c| st.fmt.stack_of(*c).is_some())).collect()
            };
            if stacked.is_empty() {
                if st.fmt.stack(range.clone()) {
                    st.sel = (range.start + 1, range.start + 1);
                    st.caret = Some(range.start + 1);
                }
            } else {
                for i in stacked.into_iter().rev() {
                    st.fmt.unstack(i);
                }
                let end = st.fmt.len().min(range.end.max(range.start + 1) + 64);
                st.caret = Some(end.min(st.fmt.len()));
            }
            refocus = true;
        }
        for (glyph, tip, code) in [
            ("⇥|", crate::tl!("Insert tab"), "\t"),
            ("°", crate::tl!("Degree"), "°"),
            ("±", crate::tl!("Plus/minus"), "±"),
            ("⌀", crate::tl!("Diameter"), "⌀"),
        ] {
            if ui.button(glyph).on_hover_text(tip).clicked() {
                insert(st, code);
                refocus = true;
            }
        }
    });
    if refocus {
        ui.ctx().memory_mut(|m| m.request_focus(edit_id()));
    }
}

/// The stacked fraction the caret is at (just before or after it) or the selection is.
fn fraction_at(st: &State) -> Option<usize> {
    let chars: Vec<char> = st.fmt.text().chars().collect();
    let is = |i: usize| chars.get(i).is_some_and(|c| st.fmt.stack_of(*c).is_some());
    let (a, b) = (st.sel.0.min(st.sel.1), st.sel.0.max(st.sel.1));
    if b == a + 1 && is(a) {
        return Some(a);
    }
    if a != b {
        return None;
    }
    a.checked_sub(1).filter(|i| is(*i)).or_else(|| is(a).then_some(a))
}

/// With the caret at a stacked fraction: its upper and lower text and its separator, to edit
/// apart (AutoCAD's Stack Properties).
fn fraction_row(ui: &mut egui::Ui, st: &mut State) {
    let Some(i) = fraction_at(st) else { return };
    let Some((mut top, mut sep, mut bottom)) = st.fmt.text().chars().nth(i).and_then(|c| st.fmt.stack_of(c)) else { return };
    let w = ((ui.available_width() - 170.0) / 2.0).max(80.0);
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.add_space(4.0);
        ui.label(crate::tl!("Fraction:"));
        changed |= ui
            .add(egui::TextEdit::singleline(&mut top).id_salt("mtext_stack_top").desired_width(w))
            .on_hover_text(crate::tl!("Upper text"))
            .changed();
        egui::ComboBox::from_id_salt("mtext_stack_sep").width(36.0).selected_text(sep.to_string()).show_ui(ui, |ui| {
            for (c, tip) in [('/', crate::tl!("Horizontal bar")), ('#', crate::tl!("Diagonal bar")), ('^', crate::tl!("No bar (tolerance)"))] {
                changed |= ui.selectable_value(&mut sep, c, format!("{c}  {tip}")).changed();
            }
        });
        changed |= ui
            .add(egui::TextEdit::singleline(&mut bottom).id_salt("mtext_stack_bottom").desired_width(w))
            .on_hover_text(crate::tl!("Lower text"))
            .changed();
    });
    if changed {
        st.fmt.set_stack(i, &top, sep, &bottom);
    }
}

/// Insert `s` at the caret (over the selection).
fn insert(st: &mut State, s: &str) {
    let (a, b) = (st.sel.0.min(st.sel.1), st.sel.0.max(st.sel.1));
    let chars: Vec<char> = st.fmt.text().chars().collect();
    let new: String = chars.iter().take(a).chain(s.chars().collect::<Vec<_>>().iter()).chain(chars.iter().skip(b)).collect();
    if st.fmt.set_text(&new, &st.typing.clone()) {
        let at = a + s.chars().count();
        st.sel = (at, at);
        st.typing_at = at;
        st.caret = Some(at);
    }
}

/// The ruler: `px` points per text height (`base_h` drawing units); paragraph indents and tab
/// stops are in drawing units, as `\p` codes are.
fn ruler(ui: &mut egui::Ui, st: &mut State, px: f32, width: f32, base_h: f64) {
    let t = Tokens::get();
    let range = st.sel.0.min(st.sel.1)..st.sel.0.max(st.sel.1);
    let p = st.fmt.para(st.fmt.paragraph_of(range.start));
    let (rect, bg) = ui.allocate_exact_size(vec2(width, 20.0), Sense::click());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 0.0, t.control);
    let tick = |u: f64| rect.left() + (u as f32) * px;
    let x_of = |v: f64| tick(v / base_h);
    let units = (f64::from(width) / f64::from(px)).floor() as i32;
    for u in 0..=units.clamp(0, 400) {
        let x = tick(f64::from(u));
        let major = u % 5 == 0;
        painter.line_segment([pos2(x, rect.bottom() - if major { 7.0 } else { 3.0 }), pos2(x, rect.bottom())], Stroke::new(1.0, t.text_faint));
        if major && u > 0 {
            painter.text(pos2(x + 2.0, rect.top() + 1.0), Align2::LEFT_TOP, u.to_string(), FontId::proportional(8.0), t.text_faint);
        }
    }
    let to_units = |x: f32| ((f64::from(x - rect.left()) / f64::from(px) / SNAP).round() * SNAP).clamp(0.0, 400.0) * base_h;
    // Tab stops: an L at the stop; a right-click removes one.
    for (pos, _kind) in &p.tabs {
        let x = x_of(*pos);
        painter.add(Shape::line(
            vec![pos2(x, rect.bottom() - 9.0), pos2(x, rect.bottom() - 2.0), pos2(x + 4.0, rect.bottom() - 2.0)],
            Stroke::new(1.5, t.accent),
        ));
    }
    let hover = bg.hover_pos();
    // Indent markers: the first line's above, the left indent's below.
    let first_abs = p.left + p.first;
    let mut drag = None;
    for (k, (u, up)) in [(first_abs, false), (p.left, true)].into_iter().enumerate() {
        let x = x_of(u);
        let y = if up { rect.bottom() - 1.0 } else { rect.top() + 1.0 };
        let dir = if up { -1.0 } else { 1.0 };
        let hit = Rect::from_center_size(pos2(x, y + dir * 4.0), vec2(10.0, 10.0));
        let r = ui.interact(hit, ui.id().with(("mtext_indent", k)), Sense::drag());
        let col = if r.hovered() || r.dragged() { t.accent } else { t.icon };
        painter.add(Shape::convex_polygon(vec![pos2(x - 4.0, y), pos2(x + 4.0, y), pos2(x, y + dir * 8.0)], col, Stroke::NONE));
        if r.dragged()
            && let Some(m) = r.interact_pointer_pos()
        {
            drag = Some((k, to_units(m.x)));
        }
    }
    if let Some((k, u)) = drag {
        if k == 0 {
            st.fmt.set_indents(range.clone(), u - p.left, p.left);
        } else {
            st.fmt.set_indents(range.clone(), p.first, u);
        }
    } else if bg.clicked()
        && let Some(m) = hover
    {
        st.fmt.add_tab(range.clone(), to_units(m.x), TabKind::Left);
    } else if bg.secondary_clicked()
        && let Some(m) = hover
    {
        st.fmt.remove_tab(range, to_units(m.x), 0.6 * base_h);
    }
    bg.on_hover_text(crate::tl!("Click the ruler to add a tab stop, right-click to remove one"));
}

fn text_box(ui: &mut egui::Ui, st: &mut State, base_h: f64, px: f32, width: f32, t: &Tokens) {
    let id = edit_id();
    if let Some(at) = st.caret.take()
        && let Some(mut s) = egui::TextEdit::load_state(ui.ctx(), id)
    {
        s.cursor.set_char_range(Some(CCursorRange::one(CCursor::new(at))));
        s.store(ui.ctx(), id);
    }
    let fmt = st.fmt.clone();
    let default_color = t.text;
    let stack_fmt = st.fmt.clone();
    let mut layouter = move |ui: &egui::Ui, text: &dyn egui::TextBuffer, wrap: f32| -> Arc<egui::Galley> {
        let mut job = LayoutJob::default();
        job.wrap.max_width = wrap;
        let s = text.as_str();
        let mut run = String::new();
        let mut cur: Option<CharFmt> = None;
        let flush = |job: &mut LayoutJob, run: &mut String, f: &CharFmt| {
            if run.is_empty() {
                return;
            }
            let size = f.height.map_or(px, |h| (px * (h / base_h) as f32).clamp(6.0, 120.0));
            let color = f.color.map_or(default_color, color32);
            job.append(
                run,
                0.0,
                TextFormat {
                    font_id: FontId::proportional(size),
                    color,
                    italics: f.italic,
                    underline: if f.underline { Stroke::new(1.0, color) } else { Stroke::NONE },
                    // The editor has one weight of each font: bold is shown wider.
                    extra_letter_spacing: if f.bold { size * 0.06 } else { 0.0 },
                    ..TextFormat::default()
                },
            );
            run.clear();
        };
        for (i, c) in s.chars().enumerate() {
            let f = fmt.fmt_at(i);
            // A stacked fraction: its placeholder made invisible and as wide as the fraction,
            // which is painted over it.
            if let Some((top, _, bottom)) = fmt.stack_of(c) {
                if let Some(p) = &cur {
                    flush(&mut job, &mut run, p);
                }
                cur = None;
                let size = f.height.map_or(px, |h| (px * (h / base_h) as f32).clamp(6.0, 120.0));
                let small = FontId::proportional(size * STACK_SCALE);
                let width = ui.fonts_mut(|fs| stack_width(fs, &top, &bottom, &small, size));
                // The room goes before the (invisible) placeholder: the fraction ends where it starts.
                job.append(
                    &c.to_string(),
                    width,
                    TextFormat { font_id: FontId::proportional(size), color: Color32::TRANSPARENT, ..TextFormat::default() },
                );
                continue;
            }
            if cur.as_ref() != Some(&f) {
                if let Some(p) = &cur {
                    flush(&mut job, &mut run, p);
                }
                cur = Some(f);
            }
            run.push(c);
        }
        if let Some(p) = &cur {
            flush(&mut job, &mut run, p);
        }
        ui.fonts_mut(|f| f.layout_job(job))
    };
    let mut buf = st.fmt.text().to_string();
    let out = egui::TextEdit::multiline(&mut buf)
        .id(id)
        .desired_width(width)
        .desired_rows(5)
        .margin(egui::Margin::symmetric(4, 4))
        .lock_focus(true)
        // Opaque: the text being edited (a table cell's) doesn't show through.
        .background_color(t.panel)
        .char_limit(cadcraft_engine::mtext_edit::MAX_CHARS)
        .layouter(&mut layouter)
        .show(ui);
    // Stacked fractions: upper text, a bar, lower text, in the room the layout left for them.
    let painter = ui.painter_at(out.response.rect);
    for (i, c) in buf.chars().enumerate() {
        let Some((top, sep, bottom)) = stack_fmt.stack_of(c) else { continue };
        let f = stack_fmt.fmt_at(i);
        let size = f.height.map_or(px, |h| (px * (h / base_h) as f32).clamp(6.0, 120.0));
        let color = f.color.map_or(default_color, color32);
        let small = FontId::proportional(size * STACK_SCALE);
        let (w, own) = ui.fonts_mut(|fs| {
            (stack_width(fs, &top, &bottom, &small, size), fs.layout_no_wrap(c.to_string(), FontId::proportional(size), Color32::WHITE).size().x)
        });
        // The fraction is the room laid out just before the placeholder glyph, which ends where
        // the next character starts.
        let a = out.galley.pos_from_cursor(CCursor::new(i)).translate(out.galley_pos.to_vec2());
        let b = out.galley.pos_from_cursor(CCursor::new(i + 1)).translate(out.galley_pos.to_vec2());
        let end = if (b.min.y - a.min.y).abs() < 1.0 { b.min.x - own } else { a.min.x + w };
        let (x0, x1) = (end - w, end);
        let mid = (a.min.y + a.max.y) / 2.0;
        let cx = (x0 + x1) / 2.0;
        painter.text(pos2(cx, mid - 1.0), Align2::CENTER_BOTTOM, &top, small.clone(), color);
        painter.text(pos2(cx, mid + 1.0), Align2::CENTER_TOP, &bottom, small, color);
        if sep != '^' {
            painter.line_segment([pos2(x0 + 1.0, mid), pos2(x1 - 1.0, mid)], Stroke::new(1.5, color));
        }
    }
    if !out.response.has_focus() && (st.focus || (buf.is_empty() && st.fmt.is_empty())) {
        out.response.request_focus();
    }
    st.focus = false;
    if out.response.changed() {
        let typing = st.typing.clone();
        st.fmt.set_text(&buf, &typing);
    }
    if let Some(r) = out.cursor_range {
        let (a, b) = (r.primary.index.0, r.secondary.index.0);
        st.sel = (a, b);
        if a == b {
            if out.response.changed() {
                st.typing_at = a;
            } else if a != st.typing_at {
                // The caret moved: typing continues the formatting of the character before it.
                st.typing = st.fmt.fmt_at(a.saturating_sub(1));
                st.typing_at = a;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cadcraft_engine::Session;

    use crate::Services;

    #[test]
    fn the_fraction_row_follows_the_caret() {
        let fmt = Formatted::from_contents("Круг \\SВ20/45; кг");
        let mut st = State { fmt, typing: CharFmt::default(), typing_at: 0, top: 0.0, sel: (0, 0), caret: None, at: (0.0, 0.0), focus: false };
        assert_eq!(fraction_at(&st), None);
        for (sel, want) in [((5, 5), Some(5)), ((6, 6), Some(5)), ((5, 6), Some(5)), ((7, 7), None), ((4, 7), None)] {
            st.sel = sel;
            assert_eq!(fraction_at(&st), want, "{sel:?}");
        }
    }

    #[test]
    fn editor_opens_only_for_mtext_text_entry_and_finishes_with_formatted_contents() {
        let ctx = egui::Context::default();
        let mut app = CadApp::new(Session::new(), Services::default());
        let frame = |app: &mut CadApp| {
            let input = egui::RawInput { screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(1200.0, 800.0))), ..Default::default() };
            for _ in 0..2 {
                let mut out = ctx.run_ui(input.clone(), |ui| show(app, ui.ctx()));
                out.textures_delta.clear();
            }
        };
        frame(&mut app);
        assert!(ctx.data_mut(|d| d.get_temp::<State>(state_id())).is_none());
        app.start("mtext");
        assert!(app.session.editing_mtext().is_none(), "no box yet");
        app.session.input(cadcraft_engine::Input::Point(cadcraft_geom::Vec2::new(1.0, 1.0))).unwrap();
        app.session.input(cadcraft_engine::Input::Point(cadcraft_geom::Vec2::new(6.0, 0.0))).unwrap();
        let m = app.session.editing_mtext().expect("box known");
        assert!((m.width - 5.0).abs() < 1e-9, "{}", m.width);
        frame(&mut app);
        let mut st = ctx.data_mut(|d| d.get_temp::<State>(state_id())).expect("editor open");
        st.fmt.set_text("hello", &CharFmt::default());
        st.fmt.toggle(0..5, Flag::Bold);
        let contents = st.fmt.to_contents();
        app.session.input(cadcraft_engine::Input::Text(contents.clone())).unwrap();
        app.session.input(cadcraft_engine::Input::Enter).unwrap();
        let d = app.session.doc().unwrap();
        let found = d.model.iter().any(|e| matches!(&e.kind, cadcraft_engine::doc::EntityKind::MText(m) if m.contents == contents));
        assert!(found, "{contents}");
    }
}
