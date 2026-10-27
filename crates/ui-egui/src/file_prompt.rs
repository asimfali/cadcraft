//! A small window while the prompt asks for a file to open (REPORT's report file, TABLETEMPLATE's
//! template): the files of the asked kind in the prompt's folders, a click entering one, and
//! Browse... for the host's file dialog. Tab at the command line completes a file name from the
//! same list ([`tab`]). Typing a path works as well, so scripts and the control channel never
//! meet a dialog.

use cadcraft_engine::{Input, Picker};
use egui::pos2;

use crate::CadApp;

/// Most files listed.
const MAX_FILES: usize = 200;
/// Largest file read to tell its kind (bytes).
#[cfg(not(target_arch = "wasm32"))]
const MAX_PEEK: u64 = 1 << 20;

/// Files listed for a prompt: (key of the folders and kind, (name, path) sorted by name).
#[derive(Clone, Debug, Default)]
pub struct FileList {
    key: String,
    files: Vec<(String, String)>,
    /// Tab completion: what was typed, the index completed last, the text it put in.
    tab: Option<(String, usize, String)>,
}

/// The files of `kind` (any file when empty) with one of `exts` in `dirs`; a name in two
/// folders is listed once, from the first.
#[cfg(not(target_arch = "wasm32"))]
fn scan(dirs: &[String], exts: &[String], kind: &str) -> Vec<(String, String)> {
    use std::io::Read;
    let mut out: Vec<(String, String)> = Vec::new();
    for d in dirs {
        let Ok(rd) = std::fs::read_dir(d) else { continue };
        for e in rd.flatten().take(5000) {
            let path = e.path();
            let Some(name) = path.file_name().map(|n| n.to_string_lossy().into_owned()) else { continue };
            let ext_ok = path.extension().is_some_and(|x| exts.iter().any(|w| x.eq_ignore_ascii_case(w.as_str())));
            if !ext_ok || !path.is_file() || out.iter().any(|(n, _)| n.eq_ignore_ascii_case(&name)) {
                continue;
            }
            if !kind.is_empty() {
                let mut text = String::new();
                let Ok(f) = std::fs::File::open(&path) else { continue };
                if f.take(MAX_PEEK).read_to_string(&mut text).is_err() {
                    continue;
                }
                let Ok(v) = serde_json::from_str::<serde_json::Value>(text.trim_start_matches('\u{feff}')) else { continue };
                if cadcraft_engine::cmd::table_file_kind(&v) != Some(kind) {
                    continue;
                }
            }
            out.push((name, path.to_string_lossy().into_owned()));
            if out.len() >= MAX_FILES {
                break;
            }
        }
    }
    out.sort_by_key(|(n, _)| n.to_lowercase());
    out
}

#[cfg(target_arch = "wasm32")]
fn scan(_dirs: &[String], _exts: &[String], _kind: &str) -> Vec<(String, String)> {
    Vec::new()
}

/// The file picker of the prompt on screen: (title, extensions, folders, kind).
fn picker(app: &CadApp) -> Option<(String, Vec<String>, Vec<String>, String)> {
    match app.session.current_prompt()?.picker? {
        Picker::OpenFile { title, extensions, dirs, kind } => Some((title, extensions, dirs, kind)),
        _ => None,
    }
}

/// The files for the prompt on screen, read once per prompt.
fn files(app: &mut CadApp) -> Vec<(String, String)> {
    let Some((_, exts, dirs, kind)) = picker(app) else {
        app.cmd.files = None;
        return Vec::new();
    };
    let key = format!("{}|{}|{}", app.session.current_prompt().map(|p| p.message).unwrap_or_default(), dirs.join("|"), kind);
    if app.cmd.files.as_ref().is_none_or(|f| f.key != key) {
        app.cmd.files = Some(FileList { key, files: scan(&dirs, &exts, &kind), tab: None });
    }
    app.cmd.files.as_ref().map(|f| f.files.clone()).unwrap_or_default()
}

/// Tab at a file prompt: complete the typed name from the listed files, the next one on each
/// Tab. Returns false when the prompt asks for no file.
pub fn tab(app: &mut CadApp) -> bool {
    if picker(app).is_none() {
        return false;
    }
    let list = files(app);
    let buffer = app.cmd.buffer.clone();
    let Some(fl) = app.cmd.files.as_mut() else { return true };
    // Tab again on what Tab put in: the next match of what was typed.
    let (typed, from) = match &fl.tab {
        Some((typed, i, put)) if *put == buffer => (typed.clone(), i + 1),
        _ => (buffer.clone(), 0),
    };
    let t = typed.to_lowercase();
    let matches: Vec<&(String, String)> = {
        let starts: Vec<_> = list.iter().filter(|(n, _)| n.to_lowercase().starts_with(&t)).collect();
        if starts.is_empty() { list.iter().filter(|(n, _)| n.to_lowercase().contains(&t)).collect() } else { starts }
    };
    if matches.is_empty() {
        return true;
    }
    let i = from % matches.len();
    if let Some((name, _)) = matches.get(i) {
        fl.tab = Some((typed, i, name.clone()));
        app.cmd.buffer = name.clone();
    }
    true
}

pub fn show(app: &mut CadApp, ctx: &egui::Context) {
    let Some((title, extensions, _, _)) = picker(app) else {
        app.cmd.files = None;
        return;
    };
    let list = files(app);
    let default = app.session.current_prompt().and_then(|p| p.default).filter(|d| !d.is_empty());
    let pick = app.services.pick_file.as_ref();
    let mut chosen = None;
    let area = ctx.content_rect();
    egui::Window::new(crate::i18n::t(&title))
        .id(egui::Id::unique("prompt_open_file"))
        .collapsible(false)
        .resizable(false)
        .default_pos(pos2(area.center().x - 160.0, area.max.y - 320.0))
        .show(ctx, |ui| {
            egui::ScrollArea::vertical().max_height(220.0).show(ui, |ui| {
                for (name, path) in &list {
                    let current = default.as_deref().is_some_and(|d| d.eq_ignore_ascii_case(path));
                    if ui.selectable_label(current, name).on_hover_text(path).clicked() {
                        chosen = Some(path.clone());
                    }
                }
            });
            if list.is_empty() {
                ui.label(crate::tl!("No files in the usual folders: browse for one."));
            }
            ui.add_space(4.0);
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Services;
    use cadcraft_engine::Session;

    #[test]
    fn a_file_prompt_lists_and_tab_completes_files_of_its_kind() {
        let dir = std::env::temp_dir().join(format!("cadcraft-file-prompt-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let _ = std::fs::write(dir.join("materials-1c.json"), r#"{"row": ["{#}"]}"#);
        let _ = std::fs::write(dir.join("materials-wires.json"), r#"{"sections": []}"#);
        let _ = std::fs::write(dir.join("materials-template.json"), r#"{"columns": [{"width": 5}]}"#);
        let _ = std::fs::write(dir.join("notes.txt"), "x");
        let mut a = CadApp::new(Session::new(), Services::default());
        a.session.last_used.report_file = dir.join("old.json").to_string_lossy().into_owned();
        a.start("report");
        let names: Vec<String> = files(&mut a).into_iter().map(|(n, _)| n).collect();
        assert_eq!(names, ["materials-1c.json", "materials-wires.json"], "reports only");
        a.cmd.buffer = "mat".into();
        assert!(tab(&mut a));
        assert_eq!(a.cmd.buffer, "materials-1c.json");
        assert!(tab(&mut a));
        assert_eq!(a.cmd.buffer, "materials-wires.json");
        assert!(tab(&mut a));
        assert_eq!(a.cmd.buffer, "materials-1c.json");
        // Typing something else starts again from it.
        a.cmd.buffer = "WIRES".into();
        assert!(tab(&mut a));
        assert_eq!(a.cmd.buffer, "materials-wires.json");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
