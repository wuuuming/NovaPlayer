//! The layout's inputs. The layout itself is written in ui/player.nui; this feeds it the model
//! facts it reads. `Doc::set` drops a value equal to the one it holds, so nothing is cached here.
use crate::app::*;
use novaui::*;

/// The state the page's layout reads (ui/player.nui).
pub(crate) fn layout_state(v: &mut Views, ui: &mut Ui) {
    // The face's facts move only when the face model notifies (`Msg::Track`).
    if !std::mem::replace(&mut v.now_playing.layout_current, true) {
        let f = v.now_playing.track.as_ref();
        let (artist, album, res) = (
            f.is_some_and(|f| !f.artist.is_empty()),
            f.is_some_and(|f| !f.album.is_empty()),
            f.is_some_and(|f| !f.res.is_empty()),
        );
        let face = f.is_some();
        v.doc.set(ui, "has-artist", artist);
        v.doc.set(ui, "has-album", album);
        v.doc.set(ui, "has-res", res);
        v.doc.set(ui, "has-top-row", face);
    }
    let rows = title_rows(v, ui);
    v.doc.set(ui, "title-rows", rows as f32);
    v.doc.set(ui, "panel-open", v.side_panel.open_amount);
    crate::views::update_doc(v.doc, v.doc_clock, ui);
    v.side_panel.place(ui, v.doc);
}

/// Title lines at the column's width (1 or 2): the layout's words block depends on it. NovaUI
/// breaks the title as its layout will (its size, line, weight and line limit are the sheet's),
/// from its text cache.
fn title_rows(v: &mut Views, ui: &mut Ui) -> usize {
    let t = v.id(&v.now_playing.block("title"));
    let col = v.doc.num("col_w").max(1.0);
    ui.tree.text_lines(t, col).clamp(1, 2)
}

/// The layout's scales as Rust uses them (`ui/player.nui`'s size rules: `text_scale`,
/// `chrome_scale`), read and clamped in one place.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Scale {
    pub text: f32,
    pub chrome: f32,
}

impl Default for Scale {
    fn default() -> Self {
        Scale { text: 1.0, chrome: 1.0 }
    }
}

impl Scale {
    pub fn of(doc: &novaui::dsl::Doc) -> Self {
        Scale { text: doc.num("text_scale").max(0.5), chrome: doc.num("chrome_scale").max(0.5) }
    }
}
