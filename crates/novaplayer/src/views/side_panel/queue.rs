//! Side panel: queue rows, thumbs, autoscroll, drag and rails.
use super::*;
use crate::sync::Locked;

impl SidePanel {
    // ---- queue ----------------------------------------------------------------------------

    pub(super) fn positions(&mut self, q: &Queue) -> &std::rc::Rc<Vec<usize>> {
        let key = (q.version, q.order_version, self.narrow.clone());
        if self.positions_key != key {
            self.positions = std::rc::Rc::new(match &self.narrow {
                None => (0..q.len()).collect(),
                Some(n) => q.filter(&n.text, n.artist.as_deref(), n.album.as_deref()),
            });
            self.positions_key = key;
        }
        &self.positions
    }

    /// Every live row is written again (another list, or its songs changed under it).
    fn refill_rows(&mut self) {
        for r in self.rows.values_mut() {
            r.idx = usize::MAX;
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn sync(
        &mut self,
        ui: &mut Ui,
        doc: &mut Doc,
        q: &Queue,
        list_version: u64,
        store: &Store,
        inbox: &crate::app::JobSender,
        assets: &novaui::Queue,
        force: bool,
    ) {
        let rh = self.row_h();
        // Heading and caption.
        let name = q.id.and_then(|id| store.entry(id)).map(|e| e.name.clone()).unwrap_or_else(|| q.list.name.clone());
        doc.set(ui, "list-name", if name.is_empty() { ui.t("list.default", &[]) } else { name });
        let n_pos = self.positions(q).len();
        let caption = match &self.narrow {
            None if q.is_empty() => String::new(),
            None if !self.shows_playing => ui.t("queue.count", &[("n", &q.len().to_string())]),
            None => ui.t("queue.at", &[("at", &(q.pos + 1).to_string()), ("n", &q.len().to_string())]),
            Some(n) if n.is_search() => ui.t("queue.found", &[("n", &n_pos.to_string())]),
            Some(n) => match (&n.artist, &n.album) {
                (Some(a), _) => ui.t("queue.of", &[("of", a), ("n", &n_pos.to_string())]),
                (_, Some(a)) => ui.t("queue.of", &[("of", a), ("n", &n_pos.to_string())]),
                _ => ui.t("songs.n", &[("n", &n_pos.to_string())]),
            },
        };
        doc.set(ui, "queue-caption", caption);
        let empty = doc.id("q-empty");
        let list = doc.id("q-list");
        ui.tree.set_layout(empty, |l| l.display = if q.is_empty() { Display::Flex } else { Display::None });
        ui.tree.set_layout(list, |l| l.display = if q.is_empty() { Display::None } else { Display::Flex });
        if self.version != list_version || force {
            if self.drag.is_some() {
                ui.cancel_drag();
                self.end_drag();
            }
            self.invalidate_rows();
            self.version = list_version;
            self.scroll = 0.0;
            self.target = 0.0;
            self.follow_current = true;
        }
        if self.listed_version != q.version {
            self.listed_version = q.version;
            // Rows are re-filled (order or content changed).
            self.refill_rows();
        }
        if !self.shown.shows(Page::Queue) {
            self.wanted.locked().2.clear();
            return;
        }
        let (_, lh) = ui.tree.size(list);
        let positions = self.positions(q).clone();
        let reach = (positions.len() as f32 * rh - lh).max(0.0);
        // A resize is not a scroll: the row at the top stays the row at the top. The pitch
        // scales with the window (the layout's text_scale), so the offset is carried in rows; the reach
        // clamp is applied to what is shown only, so a shrink and a grow back return the list to
        // where it was. During it the list does not ease: it stands.
        let resized = (self.last_rh > 0.0 && (rh - self.last_rh).abs() > 1e-4)
            || (self.last_lh > 0.0 && (lh - self.last_lh).abs() > 0.5);
        if self.last_rh > 0.0 && (rh - self.last_rh).abs() > 1e-4 {
            let k = rh / self.last_rh;
            self.scroll *= k;
            self.target *= k;
        }
        self.last_rh = rh;
        self.last_lh = lh;
        if self.queue_rail.grabbed() {
            self.follow_current = false;
            self.last_pos = q.pos;
        }
        // Follow the playing song: into view when opened / the song changed; the locate button
        // eases it to the same place even when it is in view already.
        let locate = std::mem::take(&mut self.locate);
        if (self.follow_current || locate || self.last_pos != q.pos) && lh > 0.0 {
            if let Ok(place) = positions.binary_search(&q.pos) {
                let top = place as f32 * rh;
                let shown_t = self.target.clamp(0.0, reach);
                let t = if locate || top < shown_t || top + rh > shown_t + lh {
                    (top - lh * 0.3).clamp(0.0, reach)
                } else {
                    self.target
                };
                // The asked offset may lie past this list's reach (kept from a longer list or a
                // larger window): the list is put where it is shown, never past its end, or a
                // click on a row in view would sweep every row in from far away.
                if self.follow_current {
                    self.scroll = t.min(reach);
                }
                self.target = t;
            }
            self.follow_current = false;
            self.last_pos = q.pos;
        }
        // The wanted offset stays as asked; what is shown is held inside the list's reach.
        self.target = self.target.max(0.0);
        let goal = self.target.min(reach);
        // A new search or narrowing: the list is a different list, not a scroll of the old one.
        if self.listed_narrow.as_ref() != Some(&self.narrow) {
            if self.listed_narrow.is_some() {
                self.scroll = goal;
                self.target = goal;
                self.glide.jump(goal);
                self.easing = false;

                let searching = self.narrow.as_ref().is_some_and(|n| !n.text.trim().is_empty() && n.is_search());

                ui.tree.stop(list, Prop::Y);
                ui.tree.set_prop(list, Prop::Y, if searching { RISE_DIST * self.scale.text } else { 0.0 });

                if searching {
                    ui.tree.animate(list, Prop::Y, 0.0, TURN_SECS, SHIFT_EASE, ui.now());
                }

                self.clock.frame();
            }
            self.listed_narrow = Some(self.narrow.clone());
        }
        let now = ui.now();
        self.due_thumb_retries(now);
        if self.auto.is_some() {
            self.autoscroll_tick(ui, doc, reach);
        } else if resized && !self.easing {
            // No motion is started by the window's size: the list stands at the same rows.
            self.scroll = goal;
        } else if (self.scroll - goal).abs() > 0.5 {
            // The scroll every list has (`Glide`, WinUI's offset change): a new end starts a
            // new glide from where the list stands; its length follows the distance.
            if (self.glide.target() - goal).abs() > 0.5 || self.glide.done(now) {
                self.glide.jump(self.scroll);
                self.glide.to(goal, now, crate::views::SCROLL);
            }
            self.scroll = self.glide.at(now);
            self.easing = true;
            if self.glide.done(now) {
                self.scroll = goal;
            }
            self.clock.frame();
        } else {
            self.easing = false;
            self.glide.jump(goal);
            self.scroll = goal;
        }
        // The rows in view (+1 either side), on the framework's virtual list: rows leaving are
        // parked and come back for places entering; a carried row stays live wherever it is,
        // and the rows it passes give way.
        let list_el = list;
        let vl = self.list.get_or_insert_with(|| novaui::widgets::VirtualList::new(list_el, rh));
        vl.pitch = rh;
        vl.lead = 2.0 * self.scale.text;
        vl.offset = self.scroll;
        match self.drag {
            Some((from, y, _)) => vl.carry(from, y),
            None if vl.carried().is_some() => vl.end_carry(),
            None => {}
        }
        let (els, row_gen) = (&mut self.rows, &mut self.row_gen);
        vl.sync(
            ui,
            positions.len(),
            |ui| {
                *row_gen += 1;
                let prefix = format!("qr{row_gen}");
                let args = [Val::Str(String::new()), Val::Str(String::new())];
                let made = doc.instantiate(ui, list_el, None, "QueueRow", &args, &prefix);
                let g = |n: &str| doc.id(&format!("{prefix}.{n}"));
                let r = RowEl {
                    root: g("row"),
                    name: g("name"),
                    artist: g("artist"),
                    sep: g("sep"),
                    album: g("album"),
                    len: g("len"),
                    remove: g("remove"),
                    grip: g("grip"),
                    thumb: g("thumb"),
                    rule: g("rule"),
                    idx: usize::MAX,
                };
                if made.is_ok() {
                    for (id, a) in [
                        (r.root, "row"),
                        (r.artist, "row-artist"),
                        (r.album, "row-album"),
                        (r.remove, "row-remove"),
                        (r.grip, "row-grip"),
                    ] {
                        ui.tree.set_action(id, Some(a));
                    }
                }
                let root = r.root;
                els.insert(root, r);
                root
            },
            |_, _, _| {},
        );
        if vl.moving(ui.now()) {
            self.clock.frame();
        }
        let live: Vec<(usize, Id)> = vl.rows().collect();
        // A parked row shows nothing: the next place it serves is written whole, and a song's
        // news (`row_tags`, `forget_thumb`) finds only the row in view.
        for (root, r) in self.rows.iter_mut() {
            if !live.iter().any(|(_, l)| l == root) {
                r.idx = usize::MAX;
            }
        }
        *self.wanted.locked() = (
            list_version,
            self.row_version,
            live.iter().filter_map(|&(p, _)| positions.get(p).and_then(|&p| q.list.order.get(p))).copied().collect(),
        );
        for (place, root) in live {
            let pos = positions[place];
            let Some(&idx) = q.list.order.get(pos) else { continue };
            let song = &q.list.songs[idx];
            if let Some(at) = self.thumb_lru.iter().position(|&old| old == idx) {
                self.thumb_lru.remove(at);
                self.thumb_lru.push(idx);
            }
            let Some(r) = self.rows.get_mut(&root) else { continue };
            let playing = self.shows_playing && pos == q.pos;
            let dragged = self.drag.map(|d| d.0) == Some(place);
            let name = r.name;
            for &id in &[r.root, r.artist, r.album, r.remove, r.grip] {
                ui.tree.set_tag(id, place as u64);
            }
            if r.idx != idx {
                r.idx = idx;
                ui.tree.set_text(name, song.title());
                let artist = song.tags.artist.clone().filter(|a| !a.is_empty());
                let album = song.tags.album.clone().filter(|a| !a.is_empty());
                ui.tree.set_text(r.artist, artist.as_deref().unwrap_or(""));
                ui.tree.set_text(r.album, album.as_deref().unwrap_or(""));
                for (id, on) in [
                    (r.artist, artist.is_some()),
                    (r.album, album.is_some()),
                    (r.sep, artist.is_some() && album.is_some()),
                ] {
                    ui.tree.set_layout(id, |l| l.display = if on { Display::Flex } else { Display::None });
                }
                ui.tree.set_text(r.len, &song.tags.duration.map(library::clock).unwrap_or_default());
                let thumb = self.thumbs.get(&idx).cloned();
                ui.tree.set_content(
                    r.thumb,
                    match thumb {
                        Some(t) => {
                            Content::Image(ImageContent { image: t, opacity: 1.0, fit_square: true, hdr: false })
                        }
                        None => Content::Icon(IconContent {
                            icon: Icon::MusicNote1,
                            size: 20.0 * self.scale.text,
                            cut: None,
                            named: true,
                        }),
                    },
                );
                ui.tree.set_visual(r.thumb, |v| v.fg = argb(0x87FFFFFF));
            }
            // Playing / hover state. The title's size is the sheet's; its weight says
            // which one plays.
            if let Content::Text(t) = &ui.tree.el(name).content() {
                let weight = if playing { 700 } else { 400 };
                if t.weight != weight || t.text != song.title() {
                    let st = TextContent { text: song.title().to_string(), weight, ..t.clone() };
                    ui.tree.set_content(name, Content::Text(st));
                }
            }
            ui.tree.set_lit(root, playing || dragged);
            ui.tree.set_visual(root, |v| {
                v.shadow = if dragged { Some(Shadow { sigma: 12.0, dy: 8.0, color: argb(0x47000000) }) } else { None };
            });
            let hovered = ui.tree.el(root).hovered();
            // The list's current song neither moves nor goes, playing or not (`Queue::remove`).
            let show_remove = hovered && pos != q.pos && self.drag.is_none();
            ui.tree.set_layout(r.remove, |l| l.display = if show_remove { Display::Flex } else { Display::None });
            ui.tree.set_layout(r.len, |l| l.display = if show_remove { Display::None } else { Display::Flex });
            ui.tree.set_visible(r.rule, !dragged);
            // Tags / thumbnail wanted for rows in view.
            let needs_tags = song.unread();
            let needs_thumb = !self.thumbs.contains_key(&idx) && !self.no_thumb.contains(&idx);
            if (needs_tags || needs_thumb) && self.requested.insert(idx) {
                let (path, list, inbox) = (song.path.clone(), list_version, inbox.clone());
                let wanted = self.wanted.clone();
                let version = self.row_version;
                let admitted = assets.try_background(move || {
                    let wanted_now = || {
                        let w = wanted.locked();
                        w.0 == list && w.1 == version && w.2.contains(&idx)
                    };
                    if !wanted_now() {
                        #[cfg(any(test, feature = "test-support"))]
                        crate::library::row_cache::skipped();
                        inbox.send(Job::RowSkipped { list, idx, version });
                    } else {
                        let (tags, thumb) = crate::library::row_cache::load(&path);
                        if wanted_now() {
                            inbox.send(Job::RowTags {
                                list,
                                idx,
                                version,
                                tags: if needs_tags { tags } else { None },
                                thumb,
                            });
                        } else {
                            // Retain the useful local result, not an offscreen GPU upload.
                            #[cfg(any(test, feature = "test-support"))]
                            crate::library::row_cache::skipped();
                            inbox.send(Job::RowSkipped { list, idx, version });
                        }
                    }
                });
                if !admitted {
                    self.requested.remove(&idx);
                }
            }
        }
        // Dragged row on top.
        if let Some(r) = self.drag.and_then(|(from, ..)| self.list.as_ref()?.row(from)) {
            let kids = ui.tree.children(list);
            if kids.last() != Some(&r) {
                ui.tree.reorder(r, kids.len());
            }
        }
    }

    pub fn row_version(&self) -> u64 {
        self.row_version
    }
    pub fn row_skipped(&mut self, idx: usize) {
        self.requested.remove(&idx);
    }
    /// A song's picture became known after its row asked (an online song's face landed when it
    /// played, its row having asked first and come back empty): the row asks again, now from
    /// the cache. Before, the row kept its note until the list was switched away and back.
    pub fn forget_thumb(&mut self, idx: usize) {
        if self.no_thumb.remove(&idx) || !self.thumbs.contains_key(&idx) {
            self.requested.remove(&idx);
            self.row_changed(idx);
        }
    }

    /// An online row came back without a picture and without a kept face: the fetch failed, not
    /// the song's lack of a cover. It asks again in a few seconds (three tries, 3 / 9 / 27 s).
    pub fn retry_thumb(&mut self, now: f64, idx: usize) {
        let tries = self.thumb_retry.get(&idx).map_or(0, |t| t.1);
        if tries >= 3 {
            return;
        }
        let at = now + 3f64.powi(tries as i32 + 1);
        self.thumb_retry.insert(idx, (at, tries + 1));
        self.clock.at(at);
    }

    /// Due retries go back to asking (from `sync`).
    pub(super) fn due_thumb_retries(&mut self, now: f64) {
        let due: Vec<usize> = self.thumb_retry.iter().filter(|(_, (t, _))| *t <= now).map(|(i, _)| *i).collect();
        for i in due {
            if let Some(e) = self.thumb_retry.get_mut(&i) {
                e.0 = f64::INFINITY;
            }
            self.forget_thumb(i);
        }
    }

    /// Every row asks again (its thumbnail too).
    pub fn invalidate_rows(&mut self) {
        self.row_version = self.row_version.wrapping_add(1);
        self.wanted.locked().2.clear();
        self.requested.clear();
        self.no_thumb.clear();
        self.thumb_retry.clear();
        self.thumb_lru.clear();
        self.thumbs.clear();
        self.refill_rows();
    }

    /// A row's tags (and thumbnail) landed; its row is written again at the next sync.
    pub fn row_tags(&mut self, idx: usize, thumb: Option<novaui::ImageHandle>) {
        self.requested.remove(&idx);
        if thumb.is_none() {
            self.no_thumb.insert(idx);
        }
        if let Some(image) = thumb {
            self.no_thumb.remove(&idx);
            if self.thumbs.remove(&idx).is_some() {
                self.thumb_lru.retain(|&i| i != idx);
            }
            if self.thumbs.len() >= THUMB_SLOTS {
                // The least recently made goes; its row (if one shows it) goes back to the note
                // and asks again when it is in view.
                let old = self.thumb_lru.remove(0);
                self.row_changed(old);
                self.thumbs.remove(&old);
            }
            self.thumbs.insert(idx, image);
            self.thumb_lru.push(idx);
        }
        self.row_changed(idx);
    }

    /// How far the queue list scrolls with `n` rows in a list `lh` high: never below 0.
    fn reach(&self, n: usize, lh: f32) -> f32 {
        (n as f32 * self.row_h() - lh).max(0.0)
    }

    /// The queue list's height and how far it scrolls: its rows (all of them while it is not
    /// narrowed, the found ones when it is).
    fn queue_extent(&self, q: &Queue, ui: &Ui, doc: &Doc) -> (f32, f32) {
        let (_, lh) = ui.tree.size(doc.id("q-list"));
        let n = self.positions.len().max(if self.narrow.is_none() { q.len() } else { 0 });
        (lh, self.reach(n, lh))
    }

    /// PgUp / PgDn: a page less one row (WinUI ScrollViewer's page step leaves the edge row in
    /// view); Home / End: the ends. Animated as a wheel notch is.
    pub fn page_queue(&mut self, down: bool, to_end: bool, q: &Queue, ui: &Ui, doc: &Doc) {
        let (lh, reach) = self.queue_extent(q, ui, doc);
        let at = self.target.min(reach);
        self.target = if to_end {
            if down {
                reach
            } else {
                0.0
            }
        } else {
            let page = (lh - self.row_h()).max(self.row_h());
            (at + if down { page } else { -page }).clamp(0.0, reach)
        };
        self.easing = true;
        self.follow_current = false;
    }

    pub fn autoscrolling(&self) -> bool {
        self.auto.is_some()
    }

    /// A middle press over the list: the list scrolls toward the pointer, faster the further
    /// it is from where the press began (player.nui autoscroll_*).
    pub fn autoscroll_start(&mut self, ui: &mut Ui, doc: &Doc, y: i32) {
        if self.auto.is_some_and(|a| a.latched) {
            // A latched autoscroll ends on the next press (browsers').
            self.autoscroll_stop(ui);
            return;
        }
        let _ = doc;
        self.auto = Some(Autoscroll { anchor: y, pointer: y, dragged: false, latched: false, last: ui.now() });
        self.glide.jump(self.scroll);
        self.easing = false;
        self.follow_current = false;
        ui.set_cursor_override(Some(novaui::Cursor::ScrollNS));
        self.clock.frame();
    }

    pub fn autoscroll_steer(&mut self, ui: &mut Ui, y: i32) {
        let dead = (doc_dead(self.scale.text) * ui.scale()) as i32;
        if let Some(a) = self.auto.as_mut() {
            a.pointer = y;
            if (y - a.anchor).abs() > dead {
                a.dragged = true;
            }
            self.clock.frame();
        }
    }

    /// The middle button let go: a held, moved autoscroll ends; a click without a move latches
    /// it on (until the next press, Esc or a wheel notch).
    pub fn autoscroll_release(&mut self, ui: &mut Ui) {
        match self.auto {
            Some(a) if a.dragged => self.autoscroll_stop(ui),
            Some(mut a) => {
                a.latched = true;
                self.auto = Some(a);
            }
            None => {}
        }
    }

    pub fn autoscroll_stop(&mut self, ui: &mut Ui) {
        if self.auto.take().is_some() {
            ui.set_cursor_override(None);
            ui.refresh_hover();
            self.target = self.scroll;
        }
    }

    /// One turn of autoscroll: speed grows with the pointer's distance past the dead zone.
    pub(super) fn autoscroll_tick(&mut self, ui: &mut Ui, doc: &Doc, reach: f32) {
        let Some(mut a) = self.auto else { return };
        let s = ui.scale();
        let dt = ((ui.now() - a.last) as f32).clamp(0.0, 0.1);
        a.last = ui.now();
        self.auto = Some(a);
        let dead = doc.num("autoscroll_dead");
        let off = (a.pointer - a.anchor) as f32 / s;
        let past = (off.abs() - dead).max(0.0);
        // dips per second: gain × distance, capped.
        let speed = (past * doc.num("autoscroll_gain")).min(doc.num("autoscroll_max")) * off.signum();
        if speed != 0.0 {
            self.scroll = (self.scroll + speed * dt).clamp(0.0, reach);
            self.target = self.scroll;
        }
        self.clock.frame();
    }

    #[cfg(all(feature = "test-support", windows))] // the screenshot suites (Windows only)
    pub fn last_tick(&self) -> f64 {
        self.last_tick
    }

    /// A wheel notch over the queue; one ends an autoscroll first, as a press does.
    pub fn wheel_queue(&mut self, dy: f32, q: &Queue, ui: &mut Ui, doc: &Doc) {
        self.autoscroll_stop(ui);
        let (_, reach) = self.queue_extent(q, ui, doc);
        // Quick notches the same way build speed (`Glide::notch`).
        let by = self.glide.notch(dy, ui.now(), crate::views::SCROLL);
        self.target = (self.target.min(reach) - by).clamp(0.0, reach);
        self.easing = true;
        self.follow_current = false;
    }

    /// Scrolls the list so the row at `place` is whole in view (the keyboard focus came to it), as
    /// a wheel notch scrolls.
    pub fn reveal_row(&mut self, place: usize, q: &Queue, ui: &Ui, doc: &Doc) {
        let (lh, reach) = self.queue_extent(q, ui, doc);
        let (top, h) = (place as f32 * self.row_h(), self.row_h());
        let at = self.target.min(reach);
        let to = if top < at {
            top
        } else if top + h > at + lh {
            top + h - lh
        } else {
            return;
        };
        self.target = to.clamp(0.0, reach);
        self.easing = true;
        self.follow_current = false;
    }

    /// Play position of the row at list place `place`.
    pub fn pos_of(&self, place: usize) -> Option<usize> {
        self.positions.get(place).copied()
    }

    pub fn drag_start(&mut self, place: usize, grab_y: f32) {
        self.drag = Some((place, place as f32 * self.row_h(), grab_y));
    }

    /// A drag that ended without a move: the rows go back to their places on the next sync.
    pub fn end_drag(&mut self) {
        self.drag = None;
        self.relayout = true;
    }

    /// Pointer at `y` dips from the list top: returns true if the list should keep scrolling.
    pub fn drag_to(&mut self, y: f32, lh: f32, dt: f32) -> bool {
        let Some((from, _, grab)) = self.drag else { return false };
        let n = self.positions.len();
        let edge = 48.0;
        let pull = if y < edge {
            -(1.0 - y / edge).clamp(0.0, 1.0)
        } else if y > lh - edge {
            (1.0 - (lh - y) / edge).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let reach = self.reach(n, lh);
        if pull != 0.0 {
            self.target = (self.target.min(reach) + pull * 12.0 * self.row_h() * dt).clamp(0.0, reach);
            self.scroll = self.target;
        }
        // The row stays in the list's view while it is carried (it follows the pointer within
        // it; past either end the list scrolls under it instead).
        let last = n.saturating_sub(1) as f32 * self.row_h();
        let ry = (y + self.scroll - grab)
            .clamp(self.scroll, (self.scroll + lh - self.row_h()).max(self.scroll))
            .clamp(0.0, last);
        self.drag = Some((from, ry, grab));
        pull != 0.0
    }

    /// Drop: (from, to) in play positions.
    pub fn drag_end(&mut self) -> Option<(usize, usize)> {
        let (from, y, _) = self.drag.take()?;
        let n = self.positions.len();
        let to = ((y + self.row_h() * 0.5) / self.row_h()).floor().clamp(0.0, n.saturating_sub(1) as f32) as usize;
        if to == from {
            None
        } else {
            Some((*self.positions.get(from)?, *self.positions.get(to)?))
        }
    }

    pub fn rail_event(&mut self, ui: &mut Ui, doc: &Doc, q: &Queue, event: &Event) -> bool {
        let (id, y, kind) = match *event {
            Event::Press { id, y, .. } => (id, y, 0),
            Event::Drag { id, y, .. } => (id, y, 1),
            Event::DragEnd { id, y, .. } => (id, y, 2),
            Event::DragCancel { id } => (id, 0, 3),
            Event::Wheel { id, dy } => {
                if id == doc.id("queue-rail") {
                    self.wheel_queue(dy, q, ui, doc);
                    return true;
                }
                if id == doc.id("lyrics-rail") {
                    self.wheel_lyrics(dy, ui.now());
                    return true;
                }
                return false;
            }
            _ => return false,
        };
        let queue = id == doc.id("queue-rail");
        if !queue && id != doc.id("lyrics-rail") {
            return false;
        }
        let y = (y - ui.tree.rect(id).y) as f32 / ui.scale();
        let rail = if queue { &mut self.queue_rail } else { &mut self.lyrics_rail };
        let offset = match kind {
            0 => {
                rail.press(y);
                None
            }
            1 => rail.drag(y),
            2 => rail.release(y),
            _ => {
                rail.cancel();
                None
            }
        };
        if let Some(offset) = offset {
            if queue {
                self.target = offset;
                if kind == 1 || self.queue_rail.grabbed() {
                    // A dragged thumb carries the list with it (WinUI's ScrollBar thumb).
                    self.scroll = offset;
                    self.glide.jump(offset);
                    self.easing = false;
                } else {
                    // A press on the track pages, animated as a wheel notch is.
                    self.easing = true;
                }
                self.follow_current = false;
            } else if kind == 1 || self.lyrics_rail.grabbed() {
                self.browse_glide.jump(offset - self.lyric_scroll);
            } else {
                // A press on the track pages, as a wheel notch glides.
                self.browse_glide.to(offset - self.lyric_scroll, ui.now(), crate::views::SCROLL);
            }
        }
        if !queue {
            self.browse_until = ui.now() + BROWSE_SECS;
            self.clock.at(self.browse_until);
        }
        true
    }

    pub(super) fn sync_rails(&mut self, ui: &mut Ui, doc: &Doc) {
        use novaui::widgets::scroll::{Parts, Range};
        let skin = crate::views::rail_skin(doc);
        let parts = |name: &str| Parts {
            root: doc.id(name),
            track: doc.id(&format!("{name}-track")),
            thumb: doc.id(&format!("{name}-thumb")),
        };
        let showing = self.shown.open && !self.shown.info_up;
        let h = doc.num("queue_view_h");
        let content = self.positions.len() as f32 * self.row_h();
        let wake = self.queue_rail.sync(
            ui,
            parts("queue-rail"),
            Range { viewport: h, content, offset: self.scroll },
            skin,
            showing && self.shown.page == Page::Queue,
            self.open_amount,
        );
        self.rail_wake(ui, wake);
        let h = doc.num("panel_h");
        let last = self.anchors.last().copied().unwrap_or(0.0);
        let wake = self.lyrics_rail.sync(
            ui,
            parts("lyrics-rail"),
            Range { viewport: h, content: h + last, offset: self.lyric_scroll + self.browse },
            skin,
            showing && self.shown.page == Page::Lyrics && self.heights.iter().sum::<f32>() > h,
            self.open_amount,
        );
        self.rail_wake(ui, wake);
    }

    /// A rail moving asks for frames, a rail waiting on its hover delay for that time.
    fn rail_wake(&mut self, ui: &Ui, wake: Option<f64>) {
        crate::views::rail_wake(&self.clock, ui, wake);
    }
}
