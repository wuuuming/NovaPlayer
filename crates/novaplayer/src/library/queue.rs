//! The open list and its play order (spec: order cycle and previous/next in "Transport under the art", queue operations in "Queue page").

use crate::library::store::{List, Origin};
use crate::library::Song;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Order {
    RepeatOne,
    RepeatAll,
    #[default]
    Off,
    Shuffle,
}

impl Order {
    /// one → all → off → shuffle → one; a single song: one ↔ off.
    pub fn next(self, count: usize) -> Order {
        match (self, count > 1) {
            (Order::RepeatOne, true) => Order::RepeatAll,
            (Order::RepeatAll, true) => Order::Off,
            (Order::Off, true) => Order::Shuffle,
            (Order::RepeatOne, false) => Order::Off,
            _ => Order::RepeatOne,
        }
    }
    pub fn key(self) -> &'static str {
        match self {
            Order::RepeatOne => "one",
            Order::RepeatAll => "all",
            Order::Off => "off",
            Order::Shuffle => "shuffle",
        }
    }
    pub fn from_key(k: &str) -> Option<Order> {
        [Order::RepeatOne, Order::RepeatAll, Order::Off, Order::Shuffle].into_iter().find(|o| o.key() == k)
    }
    pub fn wraps(self) -> bool {
        matches!(self, Order::RepeatAll | Order::Shuffle)
    }
}

/// A stable sort of song indices by `key` (ties keep the incoming, added order); `None` last.
fn sort_keys<K: Ord>(order: &[usize], key: impl Fn(usize) -> Option<K>, desc: bool) -> Vec<usize> {
    let mut k: Vec<(usize, Option<K>)> = order.iter().map(|&i| (i, key(i))).collect();
    k.sort_by(|a, b| match (&a.1, &b.1) {
        (Some(x), Some(y)) if desc => y.cmp(x),
        (Some(x), Some(y)) => x.cmp(y),
        (x, y) => x.is_none().cmp(&y.is_none()),
    });
    k.into_iter().map(|(i, _)| i).collect()
}

/// A random number below `n` (n ≥ 1).
fn rand_below(n: u64) -> u64 {
    use std::hash::{BuildHasher, Hasher};
    let r = std::collections::hash_map::RandomState::new().build_hasher().finish();
    r % n.max(1)
}

/// A new round of shuffle: `first`, then the rest of `order` drawn again.
fn round_from(order: &[usize], first: usize) -> Vec<usize> {
    let mut rest: Vec<usize> = order.iter().copied().filter(|&x| x != first).collect();
    shuffle(&mut rest);
    std::iter::once(first).chain(rest).collect()
}

fn shuffle(v: &mut [usize]) {
    use std::hash::{BuildHasher, Hasher};
    let mut st = std::collections::hash_map::RandomState::new().build_hasher().finish() | 1;
    let mut next = move || {
        st = st.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = st;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    };
    for last in (1..v.len()).rev() {
        let pick = (next() % (last as u64 + 1)) as usize;
        v.swap(last, pick);
    }
}

/// The queue's sort keys (the heading's List button).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Sort {
    #[default]
    Added,
    Title,
    Artist,
    Album,
    Duration,
}

impl Sort {
    pub const ALL: [Sort; 5] = [Sort::Added, Sort::Title, Sort::Artist, Sort::Album, Sort::Duration];
    pub fn key(self) -> &'static str {
        match self {
            Sort::Added => "added",
            Sort::Title => "title",
            Sort::Artist => "artist",
            Sort::Album => "album",
            Sort::Duration => "duration",
        }
    }
    pub fn from_key(k: &str) -> Option<Sort> {
        Sort::ALL.into_iter().find(|s| s.key() == k)
    }
}

#[derive(Default)]
pub struct Queue {
    pub list: List,
    /// The kept playlist this is, if any.
    pub id: Option<u32>,
    /// Position in `order` of the current song.
    pub pos: usize,
    /// Changes when the order is rearranged (the side panel dissolves to the new order).
    pub order_version: u64,
    /// Changes on any edit (the side panel rebuilds / refilters).
    pub version: u64,
    pub order_mode: Order,
    pub unsaved: bool,
}

impl Queue {
    pub fn len(&self) -> usize {
        self.list.order.len()
    }
    pub fn is_empty(&self) -> bool {
        self.list.order.is_empty()
    }
    pub fn song_at(&self, pos: usize) -> Option<&Song> {
        self.list.order.get(pos).and_then(|&i| self.list.songs.get(i).map(|s| &**s))
    }
    pub fn current(&self) -> Option<&Song> {
        self.song_at(self.pos)
    }

    /// The current song's plugin and its URI there, when it is a plugin's.
    pub fn current_plugin(&self) -> Option<(String, String)> {
        crate::plugins::client::parse(&self.current()?.path)
    }

    pub fn adopt(&mut self, list: List, id: Option<u32>) {
        let mut list = list;
        if list.order.is_empty() && !list.songs.is_empty() {
            list.order = (0..list.songs.len()).collect();
        }
        // The list's own play mode; a list saved before modes were kept stays in the current one.
        self.order_mode = if list.shuffled { Order::Shuffle } else { list.mode.unwrap_or(self.order_mode) };
        if !list.shuffled && self.order_mode == Order::Shuffle {
            self.order_mode = Order::Off;
        }
        let cur = list.current;
        self.list = list;
        self.find(Some(cur));
        self.id = id;
        self.order_version += 1;
        self.version += 1;
        self.unsaved = true;
    }

    fn touched(&mut self) {
        self.version += 1;
        self.unsaved = true;
    }

    /// The current song, as an index into the songs: the one at `pos`, the one record of which
    /// song is current while the list is here (`List::current` is what a kept list says,
    /// written by `current_from_pos` when it is kept, read by `adopt`).
    pub fn current_index(&self) -> Option<usize> {
        self.list.order.get(self.pos).copied()
    }

    /// The list as kept says its current song is the one at `pos`.
    pub(crate) fn current_from_pos(&mut self) {
        if let Some(i) = self.current_index() {
            self.list.current = i;
        }
    }

    /// After the order changed, `pos` is where song `cur` (the current one before) now stands
    /// (the top when it is gone).
    fn find(&mut self, cur: Option<usize>) {
        self.pos = cur.and_then(|c| self.list.order.iter().position(|&x| x == c)).unwrap_or(0);
    }

    /// A song joining this round of shuffle goes to a random place after the current song.
    fn insert_in_round(&mut self, i: usize) {
        let after = (self.pos + 1).min(self.list.order.len());
        let room = self.list.order.len() - after + 1;
        let at = after + (rand_below(room as u64) as usize);
        self.list.order.insert(at, i);
    }

    /// The order was rearranged: a new order version, and an edit.
    fn rearranged(&mut self) {
        self.order_version += 1;
        self.touched();
    }

    /// Whether the list may be sorted now: not while shuffle owns its order.
    pub fn sortable(&self) -> bool {
        !self.shuffling()
    }

    /// Whether the list is shuffling: its order mode, the one record of it while it is here
    /// (`List::shuffled` is what a kept list says, written by `to_list` and a save's stamp).
    pub fn shuffling(&self) -> bool {
        self.order_mode == Order::Shuffle
    }

    /// The list as it is kept: its current song from `pos`, its play mode from `order_mode`.
    pub fn to_list(&self) -> List {
        let mut l = self.list.clone();
        if let Some(&i) = self.list.order.get(self.pos) {
            l.current = i;
        }
        l.keep_mode(self.order_mode);
        l
    }

    /// Appends songs (to the found order and the play order). While shuffling, they join this
    /// round's order at random places after the current song, not as a block at its end. The
    /// songs come shared: whoever found them made them (off the UI thread for a large add).
    pub fn append(&mut self, songs: Vec<std::sync::Arc<Song>>) {
        let shuffling = self.shuffling();
        self.list.songs_mut().reserve(songs.len());
        self.list.order.reserve(songs.len());
        for s in songs {
            let i = self.list.songs.len();
            self.list.songs_mut().push(s);
            if shuffling {
                self.insert_in_round(i);
            } else {
                self.list.order.push(i);
            }
            if !self.list.own.is_empty() || shuffling {
                self.list.own.push(i);
            }
        }
        self.touched();
        // Songs added to a sorted list take their place in it.
        if let Some((k, desc)) = self.sorted_by() {
            self.resort(k, desc);
        }
    }

    /// After the list was taken again from its source (adopted): a round of shuffle `old` was
    /// in carries on over the songs; a list that was not shuffling plays on in `mode`, the
    /// user's play mode, unless that is shuffle.
    pub fn carry_round(&mut self, old: Option<&List>, mode: Order) {
        match old.filter(|l| l.shuffled) {
            Some(o) => self.carry_shuffle(o),
            None if mode != Order::Shuffle => self.order_mode = mode,
            None => {}
        }
    }

    /// A refreshed list (rescan, re-import) taking the place of `old`: a round of shuffle in
    /// progress goes on. The songs still there keep their places in it; new ones join at random
    /// after the current song; the user's arrangement underneath follows the refreshed order.
    pub fn carry_shuffle(&mut self, old: &List) {
        if !old.shuffled {
            return;
        }
        let key = |s: &Song| (s.path.to_string_lossy().to_lowercase(), s.span.map(|p| p.start));
        let at: std::collections::HashMap<_, usize> =
            self.list.songs.iter().enumerate().map(|(i, s)| (key(s), i)).collect();
        let mut taken = vec![false; self.list.songs.len()];
        let mut order = Vec::with_capacity(self.list.songs.len());
        for &o in &old.order {
            if let Some(&i) = old.songs.get(o).and_then(|s| at.get(&key(s))) {
                if !taken[i] {
                    taken[i] = true;
                    order.push(i);
                }
            }
        }
        let cur = old.songs.get(old.current).and_then(|s| at.get(&key(s))).copied();
        self.list.own = self.list.order.clone();
        self.list.order = order;
        self.pos = cur.and_then(|c| self.list.order.iter().position(|&x| x == c)).unwrap_or(0);
        self.order_mode = Order::Shuffle;
        let fresh: Vec<usize> = (0..taken.len()).filter(|&i| !taken[i]).collect();
        for i in fresh {
            self.insert_in_round(i);
        }
        self.rearranged();
    }

    pub fn set_pos(&mut self, pos: usize) {
        if pos < self.len() {
            self.pos = pos;
            self.touched();
        }
    }

    /// Takes the song at `pos` out of the list. The current song neither goes nor moves (one rule
    /// for the list that plays and one that waits where it will go on from).
    pub fn remove(&mut self, pos: usize) {
        if pos >= self.len() || pos == self.pos {
            return;
        }
        let i = self.list.order.remove(pos);
        self.list.own.retain(|&x| x != i);
        if pos < self.pos {
            self.pos -= 1;
        }
        self.touched();
    }

    /// Moves the song at `from` to stand at `to` of what is left.
    pub fn move_to(&mut self, from: usize, to: usize) {
        if from >= self.len() || from == to {
            return;
        }
        let cur = self.current_index();
        let i = self.list.order.remove(from);
        self.list.sort = None;
        let to = to.min(self.list.order.len());
        self.list.order.insert(to, i);
        self.find(cur);
        if !self.shuffling() {
            self.list.own.clear();
        }
        self.touched();
    }

    /// A new shuffle of the shuffled list: the song that plays first, the rest drawn again.
    /// The user's own arrangement (`own`) is kept for when the shuffle ends.
    pub fn reshuffle(&mut self) -> bool {
        if self.order_mode != Order::Shuffle || self.list.order.len() < 2 {
            return false;
        }
        let cur = self.current_index().unwrap_or(self.list.current);
        self.list.order = round_from(&self.list.order, cur);
        self.pos = 0;
        self.rearranged();
        true
    }

    pub fn set_order(&mut self, o: Order) {
        let cur = self.current_index();
        let was = self.order_mode;
        self.order_mode = o;
        if (o == Order::Shuffle) != (was == Order::Shuffle) {
            if o == Order::Shuffle {
                // The sort is kept (unused while shuffled): it is back when the shuffle ends.
                // Keep the user's arrangement, shuffle the rest after the current song.
                self.list.own = self.list.order.clone();
                self.list.order = round_from(&self.list.order, cur.unwrap_or(self.list.current));
            } else {
                // The user's arrangement comes back, less the songs removed meanwhile.
                let member = members(&self.list.order, self.list.songs.len());
                let has = |x: &usize| member.get(*x).copied().unwrap_or(false);
                let own = std::mem::take(&mut self.list.own);
                self.list.order = if own.is_empty() {
                    (0..self.list.songs.len()).filter(has).collect()
                } else {
                    own.into_iter().filter(has).collect()
                };
            }
            self.find(cur);
            self.order_version += 1;
            if let Some((k, d)) = self.sorted_by() {
                self.resort(k, d);
            }
        }
        self.touched();
    }

    /// Rearranges the play order by `key` (stable; ties keep the order the songs were added in).
    /// `Sort::Added` is the order they were added in. The arrangement becomes the user's own: a
    /// shuffle ends, the current song stays current.
    pub fn sort(&mut self, key: Sort, desc: bool) {
        if self.list.sort == Some((key, desc)) && !self.shuffling() && self.sorted(key, desc) == self.list.order {
            return;
        }
        self.resort(key, desc);
    }

    /// Puts the play order in `key` order (again, after tags arrived or songs were added).
    pub fn resort(&mut self, key: Sort, desc: bool) {
        let cur = self.current_index();
        let order = self.sorted(key, desc);
        self.list.sort = Some((key, desc));
        if order == self.list.order && !self.shuffling() {
            return;
        }
        self.list.order = order;
        self.list.own.clear();
        if self.shuffling() {
            self.order_mode = Order::Off;
        }
        self.find(cur);
        self.rearranged();
    }

    /// The play order `key` gives (the songs in the list now, stable on the added order).
    /// Text compares case-insensitively; a missing field sorts after every present one in both
    /// directions.
    pub fn sorted(&self, key: Sort, desc: bool) -> Vec<usize> {
        let mut order: Vec<usize> = self.list.order.clone();
        order.sort_unstable();
        let songs = &self.list.songs;
        let text = |s: Option<&str>| s.map(str::trim).filter(|s| !s.is_empty()).map(str::to_lowercase);
        let length = |i: usize| songs[i].length();
        match key {
            Sort::Added => {
                if desc {
                    order.reverse();
                }
                order
            }
            Sort::Title => sort_keys(&order, |i| text(Some(songs[i].title())), desc),
            Sort::Artist => sort_keys(&order, |i| text(songs[i].tags.artist.as_deref()), desc),
            Sort::Album => sort_keys(&order, |i| text(songs[i].tags.album.as_deref()), desc),
            Sort::Duration => sort_keys(&order, length, desc),
        }
    }

    /// The sort the play order is in (a shuffle or a hand arrangement is none).
    pub fn sorted_by(&self) -> Option<(Sort, bool)> {
        if self.shuffling() {
            return None;
        }
        self.list.sort
    }

    /// Songs whose tags a sort by `key` still needs read (not yet probed).
    pub fn unprobed(&self, key: Sort) -> Vec<usize> {
        if key == Sort::Added {
            return Vec::new();
        }
        let s = &self.list.songs;
        self.list.order.iter().copied().filter(|&i| s[i].unread()).collect()
    }

    /// The song that plays when the current one ends: itself on repeat one, else the one after
    /// it per the order. Next and previous move to the neighbours on every order (`next_pos`,
    /// `prev_pos`); only the end of a song repeats it.
    pub fn follow_pos(&self) -> Option<usize> {
        match self.order_mode {
            Order::RepeatOne if self.current().is_some() => Some(self.pos),
            _ => self.next_pos(),
        }
    }

    /// The song after the current one, per the order; None at the end of a list that stops.
    pub fn next_pos(&self) -> Option<usize> {
        if self.pos + 1 < self.len() {
            Some(self.pos + 1)
        } else if self.order_mode.wraps() && self.len() > 1 {
            Some(0)
        } else {
            None
        }
    }

    pub fn prev_pos(&self) -> Option<usize> {
        match self.pos.checked_sub(1) {
            Some(p) => Some(p),
            None if self.order_mode.wraps() && self.len() > 1 => Some(self.len() - 1),
            None => None,
        }
    }

    pub fn new_empty(name: &str) -> List {
        List { name: name.to_owned(), origin: Origin::Made, ..Default::default() }
    }

    /// Positions of the songs of the given artist / album (exactly) where every word of `q` is
    /// found in the title, artist or album (`Query`): the best found first, each rank in play
    /// order.
    pub fn filter(&self, q: &str, artist: Option<&str>, album: Option<&str>) -> Vec<usize> {
        filter(&self.list.songs, &self.list.order, q, artist, album, &|| false)
    }
}

/// `Queue::filter` over `songs` in play order `order`, on any thread (a search over a long list
/// runs on a worker). `stop` is asked now and then: true gives up, with what was found so far.
pub fn filter(
    songs: &[std::sync::Arc<Song>],
    order: &[usize],
    q: &str,
    artist: Option<&str>,
    album: Option<&str>,
    stop: &dyn Fn() -> bool,
) -> Vec<usize> {
    let q = crate::app::query::Query::new(q);
    let mut out = Vec::new();
    for (p, &i) in order.iter().enumerate() {
        if p % 1024 == 1023 && stop() {
            break;
        }
        let Some(s) = songs.get(i) else { continue };
        if artist.is_some_and(|a| s.tags.artist.as_deref() != Some(a))
            || album.is_some_and(|a| s.tags.album.as_deref() != Some(a))
        {
            continue;
        }
        let names = [Some(s.title()), s.tags.artist.as_deref(), s.tags.album.as_deref()];
        if let Some(found) = q.find(names.into_iter().flatten()) {
            out.push((found, p));
        }
    }
    // Stable: play order within a rank.
    out.sort_by_key(|f| f.0);
    out.into_iter().map(|f| f.1).collect()
}

/// Which of `len` songs a play order holds, by song index: one look at each of its entries.
fn members(order: &[usize], len: usize) -> Vec<bool> {
    #[cfg(test)]
    tests::LOOKED.with(|n| n.set(n.get() + order.len()));
    let mut m = vec![false; len];
    for &x in order {
        if let Some(v) = m.get_mut(x) {
            *v = true;
        }
    }
    m
}

#[cfg(test)]
mod tests {
    use super::*;

    thread_local! {
        /// Entries of a play order looked at (`members`).
        pub(super) static LOOKED: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    }

    /// Leaving shuffle looks at each entry of the order once, however long the list (it looked
    /// through the whole order once per song: 4·10⁸ steps for 20 000 songs, on a press).
    #[test]
    fn leaving_shuffle_looks_at_each_song_once() {
        let mut x = q(20_000);
        x.remove(7);
        let own = x.list.order.clone();
        x.set_order(Order::Shuffle);
        LOOKED.with(|n| n.set(0));
        x.set_order(Order::Off);
        assert!(LOOKED.with(|n| n.get()) <= 20_000, "one look per song");
        assert_eq!(x.list.order, own, "the user's arrangement is back, the removed song left out");
    }
    use std::path::PathBuf;
    fn q(n: usize) -> Queue {
        let mut q = Queue::default();
        q.adopt(Queue::new_empty("x"), None);
        q.append((0..n).map(|i| std::sync::Arc::new(Song::new(PathBuf::from(format!("C:\\{i}.flac"))))).collect());
        q
    }
    #[test]
    fn order_cycle() {
        assert_eq!(Order::RepeatOne.next(3), Order::RepeatAll);
        assert_eq!(Order::Off.next(3), Order::Shuffle);
        assert_eq!(Order::Shuffle.next(3), Order::RepeatOne);
        assert_eq!(Order::RepeatOne.next(1), Order::Off);
        assert_eq!(Order::Off.next(1), Order::RepeatOne);
    }
    #[test]
    fn next_prev_wrap() {
        let mut x = q(3);
        x.set_pos(2);
        assert_eq!(x.next_pos(), None);
        x.set_order(Order::RepeatAll);
        assert_eq!(x.next_pos(), Some(0));
        x.set_pos(0);
        assert_eq!(x.prev_pos(), Some(2));
    }
    #[test]
    fn shuffle_round_lifecycle() {
        // A round is kept through save/load; leaving shuffle ends it; entering again is a new one.
        let mut x = q(12);
        x.set_pos(5);
        x.set_order(Order::Shuffle);
        let round = x.list.order.clone();
        let back = crate::library::store::decode_list(&crate::library::store::encode_list(&x.to_list())).unwrap();
        let mut y = Queue::default();
        y.adopt(back, None);
        assert_eq!((y.order_mode, &y.list.order), (Order::Shuffle, &round));
        assert_eq!(y.list.order[y.pos], x.list.order[x.pos]);
        y.set_order(Order::RepeatOne);
        assert_eq!(y.list.order, (0..12).collect::<Vec<_>>());
        assert!(!y.shuffling() && y.list.own.is_empty());
        // Twelve songs: the chance a fresh round equals the old one (after the same first) is nil.
        let mut fresh = false;
        for _ in 0..4 {
            y.set_order(Order::Shuffle);
            fresh |= y.list.order != round;
            y.set_order(Order::Off);
        }
        assert!(fresh, "entering shuffle again reused the old round");
    }
    #[test]
    fn songs_added_mid_round_join_it_and_survive_a_refresh() {
        let mut x = q(6);
        x.set_order(Order::Shuffle);
        x.set_pos(2);
        let cur = x.list.order[x.pos];
        let played: Vec<usize> = x.list.order[..=x.pos].to_vec();
        x.append((6..10).map(|i| std::sync::Arc::new(Song::new(PathBuf::from(format!("C:\\{i}.flac"))))).collect());
        assert_eq!(x.list.order.len(), 10);
        assert_eq!(&x.list.order[..=x.pos], &played[..], "what was played stays before");
        assert_eq!(x.list.order[x.pos], cur);
        // Leaving shuffle: the user's order, with the new songs at its end.
        let mut z = Queue::default();
        z.adopt(x.to_list(), None);
        z.set_order(Order::Off);
        assert_eq!(z.list.order, (0..10).collect::<Vec<_>>());
        // A refresh (rescan) of the same files in another found order keeps the round.
        let old = x.to_list();
        let mut r = Queue::default();
        let mut l = Queue::new_empty("x");
        for i in (0..10).rev() {
            l.songs_mut().push(std::sync::Arc::new(Song::new(PathBuf::from(format!("C:\\{i}.flac")))));
        }
        r.adopt(l, None);
        r.carry_shuffle(&old);
        let name = |q: &Queue, p: usize| q.song_at(p).unwrap().path.clone();
        assert_eq!(r.order_mode, Order::Shuffle);
        assert_eq!((0..10).map(|p| name(&r, p)).collect::<Vec<_>>(), (0..10).map(|p| name(&x, p)).collect::<Vec<_>>());
        assert_eq!(name(&r, r.pos), name(&x, x.pos));
    }
    #[test]
    fn reshuffle_draws_again_and_keeps_the_playing_song() {
        let mut q = Queue::default();
        let mut l = Queue::new_empty("x");
        l.songs = std::sync::Arc::new(
            (0..40).map(|k| std::sync::Arc::new(crate::library::Song::new(format!("s{k}.flac").into()))).collect(),
        );
        q.adopt(l, None);
        q.set_pos(7);
        let playing = q.current().unwrap().path.clone();
        assert!(!q.reshuffle(), "only a shuffled list reshuffles");
        q.set_order(Order::Shuffle);
        let own = q.list.own.clone();
        let first = q.list.order.clone();
        let mut drew_again = false;
        for _ in 0..5 {
            assert!(q.reshuffle());
            assert_eq!(q.current().unwrap().path, playing, "the playing song stays");
            assert_eq!(q.pos, 0);
            let mut sorted = q.list.order.clone();
            sorted.sort_unstable();
            assert_eq!(sorted, (0..40).collect::<Vec<_>>(), "every song once");
            drew_again |= q.list.order != first;
        }
        assert!(drew_again, "a new order");
        assert_eq!(q.list.own, own, "the user's arrangement kept");
        q.set_order(Order::Off);
        assert_eq!(q.list.order, own);
    }

    #[test]
    fn shuffle_keeps_current_first_and_restores() {
        let mut x = q(10);
        x.set_pos(4);
        let cur = x.list.order[x.pos];
        x.set_order(Order::Shuffle);
        assert_eq!(x.list.order[0], cur);
        assert_eq!(x.pos, 0);
        x.set_order(Order::RepeatOne);
        assert_eq!(x.list.order, (0..10).collect::<Vec<_>>());
        assert_eq!(x.list.order[x.pos], cur);
    }
    #[test]
    fn sort_keeps_current_and_restores_added_order() {
        let mut x = q(4);
        for (i, t) in ["c", "A", "b", "a"].iter().enumerate() {
            x.list.song_mut(i).unwrap().tags.title = Some(t.to_string());
        }
        x.set_pos(2);
        let cur = x.list.order[x.pos];
        x.sort(Sort::Title, false);
        assert_eq!(x.list.order, vec![1, 3, 2, 0]);
        assert_eq!(x.sorted_by(), Some((Sort::Title, false)));
        assert_eq!(x.list.order[x.pos], cur);
        x.sort(Sort::Title, true);
        assert_eq!(x.list.order, vec![0, 2, 1, 3]);
        assert_eq!(x.sorted_by(), Some((Sort::Title, true)));
        x.set_order(Order::Shuffle);
        assert_eq!(x.sorted_by(), None);
        x.sort(Sort::Added, false);
        assert_eq!(x.list.order, vec![0, 1, 2, 3]);
        assert_ne!(x.order_mode, Order::Shuffle);
        assert_eq!(x.list.order[x.pos], cur);
    }
    #[test]
    fn sort_fields_missing_last_both_ways() {
        let mut x = q(4);
        for (i, a) in [Some("b"), None, Some("A"), Some("c")].iter().enumerate() {
            x.list.song_mut(i).unwrap().tags.artist = a.map(String::from);
            x.list.song_mut(i).unwrap().tags.duration = a.map(|_| std::time::Duration::from_secs(10 - i as u64));
        }
        x.sort(Sort::Artist, false);
        assert_eq!(x.list.order, vec![2, 0, 3, 1]);
        x.sort(Sort::Artist, true);
        assert_eq!(x.list.order, vec![3, 0, 2, 1]);
        x.sort(Sort::Duration, false);
        assert_eq!(x.list.order, vec![3, 2, 0, 1]);
        x.sort(Sort::Duration, true);
        assert_eq!(x.list.order, vec![0, 2, 3, 1]);
        x.move_to(0, 2);
        assert_eq!(x.sorted_by(), None);
    }
    #[test]
    fn move_and_remove() {
        let mut x = q(4);
        x.move_to(3, 0);
        assert_eq!(x.list.order, vec![3, 0, 1, 2]);
        assert_eq!(x.pos, 1);
        x.remove(1);
        assert_eq!(x.list.order, vec![3, 0, 1, 2]);
        x.remove(0);
        assert_eq!(x.list.order, vec![0, 1, 2]);
        assert_eq!(x.pos, 0);
    }
}
