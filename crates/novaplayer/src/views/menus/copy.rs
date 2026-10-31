//! The 复制 menus: the song's words under the artwork (复制 › what), and a lyric line's.
use crate::app::command::CopyWhat;
use crate::app::Views;
use crate::views::menus::*;
use novaui::*;

/// The words under the artwork, pressed with the secondary button: 复制 ›, what to copy picked
/// beside it (`open_copy_pick`).
pub(crate) fn open_words_menu(v: &mut Views, ui: &mut Ui, x: i32, y: i32) {
    if v.now_playing.track.is_none() {
        return close_menus(v, ui);
    }
    let menu = begin(v, ui, Kind::Words);
    if let Some(row) =
        item(v, ui, menu, Icon::Copy, &ui.t("menu.copy", &[]), "", Some(Command::CopyPick), true, "wm-copy")
    {
        let _ = v.doc.instantiate(ui, row, None, "MenuChevron", &[], "wm-copy.more");
    }
    let s = ui.scale();
    show(v, ui, Place::at(x as f32 / s, y as f32 / s), None);
}

/// Beside 复制: the title, the artist, the album, the title with the artist; those the song
/// has. Pressed again, it goes.
pub(crate) fn open_copy_pick(v: &mut Views, ui: &mut Ui) {
    let (Some(row), true) = (v.doc.get("wm-copy.item"), is_open(v, |k| *k == Kind::Words)) else { return };
    let again = v.menus.copying;
    close_plugin_flyout(v, ui);
    ui.tree.set_lit(row, !again);
    if again {
        return;
    }
    let Some(t) = v.now_playing.track.as_ref() else { return };
    let host = v.id("plugin-flyout");
    let panel = v.menus.side.reset(ui, host);
    v.doc.prune(ui);
    v.menus.copying = true;
    let rows = [
        ("menu.copy.title", CopyWhat::Title, !t.title.is_empty(), "cp-title"),
        ("menu.copy.artist", CopyWhat::Artist, !t.artist.is_empty(), "cp-artist"),
        ("menu.copy.album", CopyWhat::Album, !t.album.is_empty(), "cp-album"),
        ("menu.copy.title-artist", CopyWhat::TitleArtist, !t.title.is_empty() && !t.artist.is_empty(), "cp-both"),
    ];
    for (words, what, has, name) in rows {
        let _ = item(v, ui, panel, Icon::Copy, &ui.t(words, &[]), "", Some(Command::Copy(what)), has, name);
    }
    show_side(v, ui, row);
}

/// A lyric line pressed with the secondary button: copy it (with the layers shown under it),
/// or every line.
pub(crate) fn open_lyric_menu(v: &mut Views, ui: &mut Ui, group: usize, x: i32, y: i32) {
    let menu = begin(v, ui, Kind::Lyric);
    let _ = item(
        v,
        ui,
        menu,
        Icon::Copy,
        &ui.t("menu.copy.line", &[]),
        "",
        Some(Command::Copy(CopyWhat::Line(group))),
        true,
        "lm-line",
    );
    let _ = item(
        v,
        ui,
        menu,
        Icon::Copy,
        &ui.t("menu.copy.lyrics", &[]),
        "",
        Some(Command::Copy(CopyWhat::Lyrics)),
        true,
        "lm-all",
    );
    let s = ui.scale();
    show(v, ui, Place::at(x as f32 / s, y as f32 / s), None);
}
