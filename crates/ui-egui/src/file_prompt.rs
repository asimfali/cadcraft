//! A small window while the prompt asks for a file to open (REPORT's report file): Browse...
//! opens the host's file dialog and enters the chosen path at the prompt. Typing the path at
//! the command line works as well, so scripts and the control channel never meet a dialog.

use cadcraft_engine::{Input, Picker};
use egui::pos2;

use crate::CadApp;

pub fn show(app: &mut CadApp, ctx: &egui::Context) {
    let Some(prompt) = app.session.current_prompt() else { return };
    let Some(Picker::OpenFile { title, extensions }) = prompt.picker else { return };
    let pick = app.services.pick_file.as_ref();
    let mut chosen = None;
    let area = ctx.content_rect();
    egui::Window::new(crate::i18n::t(&title))
        .id(egui::Id::new("prompt_open_file"))
        .collapsible(false)
        .resizable(false)
        .default_pos(pos2(area.center().x - 160.0, area.max.y - 220.0))
        .show(ctx, |ui| {
            if let Some(d) = prompt.default.as_deref().filter(|d| !d.is_empty()) {
                ui.label(d);
            }
            let browse = ui.add_enabled(pick.is_some(), egui::Button::new(crate::tl!("Browse...")));
            if browse.clicked() {
                chosen = pick.and_then(|f| f(crate::i18n::t(&title), &extensions));
            }
        });
    if let Some(path) = chosen {
        app.cmd.buffer.clear();
        if let Err(e) = app.session.input(Input::Text(path)) {
            app.session.echo(e.to_string());
        }
    }
}
