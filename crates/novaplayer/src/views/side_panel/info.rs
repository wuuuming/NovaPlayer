//! Side panel: the song info page.
use super::*;
use crate::player_core::MediaTags;

impl SidePanel {
    // ---- info -----------------------------------------------------------------------------

    /// The playing song changed or learnt more (`face` / `song`): the info page follows it, unless
    /// it describes a queue row's song (`info_of`).
    pub fn set_info(&mut self, track: Option<&NowPlayingTrack>, song: Option<&Song>) {
        if self.info_of.is_none() {
            self.fill_info(track, song);
        }
    }

    /// A queue row's song on the info page: the list's words, then what reading its file found
    /// (`tags`), once it did.
    pub fn show_song_info(&mut self, song: &Song, tags: Option<MediaTags>) {
        let w = &song.tags;
        let mut t = tags.unwrap_or_default();
        t.cover = None;
        t.lyrics = None;
        let or = |a: &mut Option<String>, b: &Option<String>| {
            if a.as_deref().is_none_or(|x| x.trim().is_empty()) {
                *a = b.clone();
            }
        };
        or(&mut t.artist, &w.artist);
        or(&mut t.album, &w.album);
        or(&mut t.album_artist, &w.album_artist);
        or(&mut t.track, &w.track);
        or(&mut t.genre, &w.genre);
        or(&mut t.date, &w.date);
        t.duration = t.duration.or(w.duration);
        let face = NowPlayingTrack {
            title: song.title().to_owned(),
            artist: t.artist.clone().unwrap_or_default(),
            album: t.album.clone().unwrap_or_default(),
            res: String::new(),
            tags: Some(t),
            lyrics: None,
            layers: Default::default(),
        };
        self.info_of = Some(song.clone());
        self.fill_info(Some(&face), Some(song));
    }

    /// The info page describes `face` / `song`: its rows are worked out now, their elements made
    /// at the next present.
    fn fill_info(&mut self, track: Option<&NowPlayingTrack>, song: Option<&Song>) {
        self.info_scroll.jump(0.0);
        self.info_stale = true;
        self.info_groups.clear();
        let Some(f) = track else {
            self.shown.edit().info_up = false;
            return;
        };
        let t = f.tags.clone().unwrap_or_default();
        let r = t.resolution.as_ref();
        let (loc, ext) = song.map_or((None, None), |s| {
            (Some(s.path.to_string_lossy().into_owned()), s.path.extension().map(|e| e.to_string_lossy().into_owned()))
        });
        let (artist, album) = if song.is_some_and(|s| s.span.is_some()) {
            (song.and_then(|s| s.tags.artist.clone()), song.and_then(|s| s.tags.album.clone()))
        } else {
            (t.artist.clone(), t.album.clone())
        };
        let dur = song.and_then(crate::library::Song::length).or(t.duration);
        let groups: Vec<Vec<(&'static str, String)>> = vec![
            vec![
                ("info.title-row", f.title.clone()),
                ("info.artist", opt(&artist)),
                ("info.album", opt(&album)),
                ("info.album-artist", opt(&t.album_artist)),
                (
                    "info.track",
                    t.track
                        .as_deref()
                        .map(library::track_text)
                        .or(song.and_then(|s| s.tags.track.clone()))
                        .unwrap_or_else(|| library::MISSING.into()),
                ),
                (
                    "info.year",
                    t.date
                        .as_deref()
                        .and_then(library::year)
                        .map(str::to_string)
                        .unwrap_or_else(|| library::MISSING.into()),
                ),
                ("info.genre", opt(&t.genre)),
                ("info.duration", dur.map(library::clock).unwrap_or_else(|| library::MISSING.into())),
            ],
            vec![
                ("info.codec", r.map(|r| library::codec(&r.codec)).unwrap_or_else(|| library::MISSING.into())),
                ("info.rate-depth", library::rate_depth(r.map(|r| r.sample_rate), r.and_then(|r| r.bits))),
                (
                    "info.bit-rate",
                    r.and_then(|r| r.bit_rate)
                        .map(|b| format!("{} kbps", (b + 500) / 1000))
                        .unwrap_or_else(|| library::MISSING.into()),
                ),
            ],
            vec![
                ("info.kind", library::kind_text(ext.as_deref(), t.container.as_deref())),
                ("info.size", t.file_size.map(library::size_text).unwrap_or_else(|| library::MISSING.into())),
                ("info.location", loc.unwrap_or_else(|| library::MISSING.into())),
            ],
        ];
        self.info_groups = groups;
    }

    /// The info rows' elements follow the rows set since the last present.
    pub(super) fn present_info(&mut self, ui: &mut Ui, doc: &mut Doc) {
        if !std::mem::take(&mut self.info_stale) {
            return;
        }
        let rows = doc.id("info-rows");
        for g in self.info_rows.drain(..) {
            ui.tree.remove(g);
        }
        doc.prune(ui);
        for (gi, g) in self.info_groups.iter().enumerate() {
            let Ok(gid) = doc.instantiate(ui, rows, None, "InfoGroup", &[], &format!("ig{gi}")) else { continue };
            for (ri, (label, value)) in g.iter().enumerate() {
                let _ = doc.instantiate(
                    ui,
                    gid,
                    None,
                    "InfoRow",
                    &[Val::Key((*label).into()), Val::Str(value.clone())],
                    &format!("ig{gi}.{ri}"),
                );
            }
            self.info_rows.push(gid);
        }
    }

    /// A wheel notch over the info page: its rows glide as every scroll does.
    pub fn wheel_info(&mut self, ui: &Ui, doc: &Doc, dy: f32) {
        let (_, ph) = ui.tree.size(doc.id("info-pane"));
        let (_, rh) = ui.tree.size(doc.id("info-rows"));
        self.info_scroll.wheel(dy, ui.now(), 0.0, (rh - ph).max(0.0), crate::views::SCROLL);
        self.clock.frame();
    }

    /// The info page's rows: centred when they fit (the pane's justify), else from the top less
    /// the scroll (a centred overflow would cut both ends).
    pub fn place_info(&mut self, ui: &mut Ui, doc: &Doc) {
        let (pane, rows) = (doc.id("info-pane"), doc.id("info-rows"));
        let (_, ph) = ui.tree.size(pane);
        let (_, rh) = ui.tree.size(rows);
        let over = (rh - ph).max(0.0);
        let now = ui.now();
        self.info_scroll.hold(0.0, over, now, crate::views::SCROLL);
        if !self.info_scroll.done(now) {
            self.clock.frame();
        }
        let y = over * 0.5 - self.info_scroll.at(now);
        if (ui.tree.prop(rows, Prop::Y) - y).abs() > 0.01 {
            ui.tree.set_translate(rows, 0.0, y);
        }
    }
}
