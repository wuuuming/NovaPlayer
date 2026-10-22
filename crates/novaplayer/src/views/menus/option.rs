//! The plugin option flyout.
use crate::app::{Models, Views};
use crate::views::menus::*;
use novaui::*;

/// The option flyout, under the line it opens from: every value of the plugin's option the
/// playing song comes in, written as the line itself is (the plugin's `style.res` template:
/// "96 kHz / 24-bit FLAC · 高清臻音"), the one it plays at checked (the output flyout's rows).
pub fn open_option_menu(v: &mut Views, ui: &mut Ui, m: &Models, list: Vec<crate::plugins::client::OptionValue>) {
    let menu = begin(v, ui, Kind::Option { song: m.playback.song_version });
    let now = m.playback.snap.option.as_ref().map(|(id, _)| id.clone());
    // The playing song's plugin's, also while another list is shown.
    let tpl = m.playing_queue().current_plugin().and_then(|(pl, _)| m.plugin.registry.style_res(&pl));
    let rows: Vec<(String, String, bool)> = list
        .iter()
        .map(|q| {
            let (a, b) = crate::plugins::client::res_line(tpl.as_deref(), &q.label, &q.name);
            let label = if tpl.is_some() {
                format!("{a}{b}")
            } else if q.label.is_empty() {
                q.name.clone()
            } else {
                format!("{} {}", q.label, q.name)
            };
            (label, q.id.clone(), now.as_deref() == Some(q.id.as_str()))
        })
        .collect();
    for (k, (label, id, on)) in rows.iter().enumerate() {
        check_item(v, ui, menu, *on, label, "", Some(Command::SetOption(id.clone())), true, &format!("qm{k}"));
    }
    let res = v.id(&v.now_playing.block("res-line"));
    // Over the line, on the artwork it describes, whatever the window's size; under it only when
    // the room above (below the title bar) is shorter than the flyout and the room below is more.
    show(v, ui, Place::on(res, Side::Above, Along::Center, 4.0), Some(res));
}
