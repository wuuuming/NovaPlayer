//! Multi-select of the queue's rows: which songs are chosen, Shift's range, a press swept across
//! rows. A row is a place in the rows shown (`positions`: the play positions, narrowed by a
//! search); a choice is kept by the song (an index into the list's songs), so it holds while the
//! rows move under it (a sort, a song moved, a search typed).
use super::*;
use std::collections::BTreeSet;

#[derive(Clone, Debug, Default)]
pub struct Selection {
    songs: BTreeSet<usize>,
    /// The song a Shift+press ranges from: the last row pressed alone.
    anchor: Option<usize>,
    /// A press swept across rows: the row it began on, whether it chooses or lets go (the
    /// opposite of what that row was), and the choice before it (going back unsweeps).
    sweep: Option<(usize, bool, BTreeSet<usize>)>,
    /// A row pressed and not yet let go (with Shift?): it chooses on release, unless the press
    /// became a sweep (a row takes drags while selecting: its release is a drag's end).
    pressed: Option<(usize, bool)>,
}

impl Selection {
    pub fn has(&self, song: usize) -> bool {
        self.songs.contains(&song)
    }
}

/// The song at row `place`.
fn song_at(q: &Queue, positions: &[usize], place: usize) -> Option<usize> {
    positions.get(place).and_then(|&pos| q.list.order.get(pos)).copied()
}

impl SidePanel {
    /// Multi-select is on (the toggle beside the search, Shift+Ctrl+A).
    pub fn selecting(&self) -> bool {
        self.select.is_some()
    }

    /// Turns multi-select on (nothing chosen: every row dims) or off (the choice goes).
    pub fn set_selecting(&mut self, on: bool) {
        if on != self.selecting() {
            self.select = on.then(Selection::default);
            self.relayout = true;
        }
    }

    pub fn selection(&self) -> Option<&Selection> {
        self.select.as_ref()
    }

    /// A press on row `place`: it is chosen or let go; with Shift, every row from the last row
    /// pressed alone to it is chosen.
    pub fn select_press(&mut self, q: &Queue, place: usize, shift: bool) {
        let positions = self.positions.clone();
        let Some(s) = self.select.as_mut() else { return };
        let Some(song) = song_at(q, &positions, place) else { return };
        let from = shift
            .then_some(s.anchor)
            .flatten()
            .and_then(|a| (0..positions.len()).find(|&k| song_at(q, &positions, k) == Some(a)));
        match from {
            Some(from) => {
                for k in from.min(place)..=from.max(place) {
                    s.songs.extend(song_at(q, &positions, k));
                }
            }
            None => {
                if !s.songs.remove(&song) {
                    s.songs.insert(song);
                }
                s.anchor = Some(song);
            }
        }
        self.relayout = true;
    }

    /// The button went down on row `place` (`select_release` ends it).
    pub fn select_down(&mut self, place: usize, shift: bool) {
        if let Some(s) = self.select.as_mut() {
            s.pressed = Some((place, shift));
        }
    }

    /// The button came up: a press that did not sweep chooses its row (`select_press`). True
    /// when it was this list's press or sweep.
    pub fn select_release(&mut self, q: &Queue) -> bool {
        let pressed = self.select.as_mut().and_then(|s| s.pressed.take());
        if self.sweep_end() {
            return true;
        }
        match pressed {
            Some((place, shift)) => {
                self.select_press(q, place, shift);
                true
            }
            None => false,
        }
    }

    /// Every row shown chosen (Ctrl+A); when they all are, none.
    pub fn select_all(&mut self, q: &Queue) {
        let positions = self.positions.clone();
        let Some(s) = self.select.as_mut() else { return };
        let shown: BTreeSet<usize> = (0..positions.len()).filter_map(|k| song_at(q, &positions, k)).collect();
        if shown.is_subset(&s.songs) {
            s.songs.retain(|x| !shown.contains(x));
        } else {
            s.songs.extend(shown);
        }
        self.relayout = true;
    }

    /// A press on row `place` became a drag: it sweeps, choosing (or letting go of, when that
    /// row was chosen) every row it crosses.
    pub fn sweep_start(&mut self, q: &Queue, place: usize) {
        let positions = self.positions.clone();
        let Some(s) = self.select.as_mut() else { return };
        let Some(song) = song_at(q, &positions, place) else { return };
        s.sweep = Some((place, !s.songs.contains(&song), s.songs.clone()));
        s.pressed = None;
        s.anchor = Some(song);
        self.sweep_to(q, place);
    }

    /// The sweep is over row `place` (dips `y` in the list, scroll included, made a place by the
    /// caller): the rows from where it began to here take its choice; those it went back over,
    /// what they were.
    pub fn sweep_to(&mut self, q: &Queue, place: usize) {
        let positions = self.positions.clone();
        let Some(s) = self.select.as_mut() else { return };
        let Some((from, on, before)) = s.sweep.clone() else { return };
        let place = place.min(positions.len().saturating_sub(1));
        s.songs = before;
        for k in from.min(place)..=from.max(place) {
            if let Some(song) = song_at(q, &positions, k) {
                if on {
                    s.songs.insert(song);
                } else {
                    s.songs.remove(&song);
                }
            }
        }
        self.relayout = true;
    }

    /// The sweep ended (released or called off: what it chose stands); a press called off
    /// chooses nothing.
    pub fn sweep_end(&mut self) -> bool {
        let Some(s) = self.select.as_mut() else { return false };
        s.pressed = None;
        s.sweep.take().is_some()
    }

    pub fn sweeping(&self) -> bool {
        self.select.as_ref().is_some_and(|s| s.sweep.is_some())
    }

    /// The row under dips `y` from the list's top (the scroll added here).
    pub fn place_at(&self, y: f32) -> usize {
        ((y + self.scroll) / self.row_h()).floor().max(0.0) as usize
    }

    /// The chosen songs' play positions, in play order.
    pub fn selected_positions(&self, q: &Queue) -> Vec<usize> {
        let Some(s) = &self.select else { return Vec::new() };
        (0..q.list.order.len()).filter(|&pos| s.songs.contains(&q.list.order[pos])).collect()
    }

    /// The choice is let go (its songs moved or went), multi-select stays on.
    pub fn select_none(&mut self) {
        if let Some(s) = self.select.as_mut() {
            *s = Selection::default();
            self.relayout = true;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn panel(n: usize) -> (SidePanel, Queue) {
        let mut ui = novaui::Ui::new(400, 400, 1.0);
        let mut p = SidePanel::new(&mut ui, Default::default());
        let mut list = Queue::new_empty("t");
        list.songs = std::sync::Arc::new(
            (0..n).map(|k| std::sync::Arc::new(crate::library::Song::new(format!("{k}.flac").into()))).collect(),
        );
        list.order = (0..n).collect();
        let mut q = Queue::default();
        q.adopt(list, None);
        p.positions = std::rc::Rc::new((0..n).collect());
        p.set_selecting(true);
        (p, q)
    }

    fn chosen(p: &SidePanel, q: &Queue) -> Vec<usize> {
        p.selected_positions(q)
    }

    /// A press chooses a row or lets it go; Shift+press chooses the rows from the last one
    /// pressed alone; Ctrl+A chooses every row shown, and again none.
    #[test]
    fn presses_shift_and_all_choose_rows() {
        let (mut p, q) = panel(8);
        p.select_press(&q, 2, false);
        p.select_press(&q, 5, false);
        assert_eq!(chosen(&p, &q), [2, 5]);
        p.select_press(&q, 2, false);
        assert_eq!(chosen(&p, &q), [5], "pressed again, it goes");
        p.select_press(&q, 4, true);
        assert_eq!(chosen(&p, &q), [2, 3, 4, 5], "from the last row pressed alone (2) to 4");
        p.select_all(&q);
        assert_eq!(chosen(&p, &q).len(), 8);
        p.select_all(&q);
        assert!(chosen(&p, &q).is_empty(), "all chosen: Ctrl+A again chooses none");
    }

    /// A press let go without a sweep chooses its row; one that swept chooses what it crossed
    /// only; one called off chooses nothing.
    #[test]
    fn a_release_chooses_the_row_pressed_unless_it_swept() {
        let (mut p, q) = panel(6);
        p.select_down(2, false);
        assert!(p.select_release(&q));
        assert_eq!(chosen(&p, &q), [2]);
        p.select_down(4, false);
        p.sweep_start(&q, 4);
        p.sweep_to(&q, 5);
        assert!(p.select_release(&q));
        assert_eq!(chosen(&p, &q), [2, 4, 5], "the sweep's rows, the press not again");
        p.select_down(0, false);
        p.sweep_end();
        assert!(!p.select_release(&q));
        assert_eq!(chosen(&p, &q), [2, 4, 5], "called off: nothing");
    }

    /// A sweep chooses what it crosses, going back lets go of what it chose only; begun on a
    /// chosen row it lets go.
    #[test]
    fn a_sweep_chooses_what_it_crosses_and_takes_back_what_it_goes_back_over() {
        let (mut p, q) = panel(10);
        p.select_press(&q, 7, false);
        p.sweep_start(&q, 2);
        p.sweep_to(&q, 5);
        assert_eq!(chosen(&p, &q), [2, 3, 4, 5, 7]);
        p.sweep_to(&q, 3);
        assert_eq!(chosen(&p, &q), [2, 3, 7], "back over 4 and 5: as they were");
        p.sweep_to(&q, 0);
        assert_eq!(chosen(&p, &q), [0, 1, 2, 7], "past where it began, the other way");
        assert!(p.sweep_end());
        p.sweep_start(&q, 1);
        p.sweep_to(&q, 9);
        assert_eq!(chosen(&p, &q), [0], "begun on a chosen row, it lets go");
    }

    /// The choice is the songs: it holds while the rows move under it, and a search shows it.
    #[test]
    fn the_choice_follows_its_songs_when_the_rows_move() {
        let (mut p, mut q) = panel(6);
        p.select_press(&q, 1, false);
        p.select_press(&q, 4, false);
        q.list.order.reverse();
        assert_eq!(chosen(&p, &q), [1, 4], "song 4 is now at 1, song 1 at 4");
        p.positions = std::rc::Rc::new(vec![1, 3]);
        p.select_all(&q);
        assert_eq!(chosen(&p, &q), [1, 3, 4], "Ctrl+A chooses the rows shown");
    }
}
