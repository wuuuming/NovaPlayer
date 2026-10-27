//! What the user typed to find something, wherever they type it: the queue's search box, a
//! dropdown's box. One rule, so they never disagree: every word is found in one of the thing's
//! names (a song's title, artist, album; a font's names), and how it was found ranks the thing.
//!
//! A word is found, best first (`Found`):
//! - **as text**: in the name, both folded (`fold::fold`: case, marks, full width, Cyrillic and
//!   Greek in Latin letters, katakana as hiragana);
//! - **by reading**: as the starts of the readings of pieces of the name in a row
//!   (`fold::pieces`): an ideograph's pinyin, jyutping or Japanese readings (`han`), a kana's
//!   romaji (`kana`), a Hangul syllable's romanisations or first consonant (`hangul`), and a
//!   word of letters whole or by its initial. `zjl`, `zhoujl` and `zhoujielun` find 周杰伦,
//!   `tokyo` finds 東京 and とうきょう, `tswift` finds Taylor Swift, `ㅎㄱ` finds 한국. Kana typed
//!   is read as its romaji;
//! - **with a typo**: within `typo::allowed` edits (a swap of two letters is one) of the name or
//!   of its reading.

mod fold;
mod han;
mod hangul;
mod kana;
mod latin;
mod typo;

use fold::{letters, start, Kind, Piece, NOTHING, OTHER};
use std::cell::RefCell;

pub use han::warm;

/// How a query was found in a thing: the way its least well found word was. Better first, so
/// sorting by it ranks.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Found {
    Text,
    Reading,
    Typo,
}

/// What a thing's names are made of, worked out once for every query typed over them
/// (`Query::find_signed`): the letters in them, the other characters (hashed into 64 bits), what
/// the readings of their pieces start with and the letters of their reading. A word that cannot be
/// found in names with this signature is known not to be without folding them again: a search
/// over a long list keeps one per thing (28 bytes) while it is typed.
#[derive(Clone, Copy, Debug, Default)]
pub struct Signature {
    letters: u64,
    others: u64,
    read_letters: u64,
    starts: u32,
}

impl Signature {
    pub fn of<'a>(names: impl IntoIterator<Item = &'a str>) -> Signature {
        let mut name = Name::default();
        let mut sig = Signature::default();
        for s in names {
            name.set(s);
            sig.letters |= name.letters;
            sig.others |= others(name.text.iter().copied());
            sig.starts |= name.starts_any();
            sig.read_letters |= name.read_letters();
        }
        sig
    }
}

/// A bit (of 64, by a multiplicative hash) for each character of `chars` that `fold::letters`
/// has none for.
fn others(chars: impl IntoIterator<Item = char>) -> u64 {
    chars
        .into_iter()
        .filter(|c| !c.is_ascii_alphanumeric())
        .fold(0, |m, c| m | 1 << ((c as u32).wrapping_mul(0x9E37_79B9) >> 26))
}

pub struct Query {
    words: Vec<Word>,
    /// The names of the thing looked at, kept between things so a search over a long list
    /// allocates nothing per thing.
    names: RefCell<(Vec<Name>, Vec<bool>)>,
}

/// One typed word: folded, as read (its kana in romaji), each with its letters
/// (`fold::letters`) and as looked for with typos.
struct Word {
    text: Vec<char>,
    read: Vec<char>,
    letters: u64,
    /// `others` of `text`.
    others: u64,
    read_letters: u64,
    typo: Option<typo::Typo>,
    typo_read: Option<typo::Typo>,
}

impl Query {
    pub fn new(text: &str) -> Self {
        let words = text.split_whitespace().filter_map(|w| {
            let mut text = Vec::new();
            fold::fold(w, &mut text);
            let read: Vec<char> = kana::romaji(&text, false).chars().collect();
            let (typo, typo_read) = (typo::Typo::new(&text), typo::Typo::new(&read));
            let (letters, read_letters) = (letters(text.iter().copied()), letters(read.iter().copied()));
            let others = others(text.iter().copied());
            (!text.is_empty()).then_some(Word { text, read, letters, others, read_letters, typo, typo_read })
        });
        Query { words: words.collect(), names: RefCell::default() }
    }

    /// Everything this query finds, `wider` finds too: it was typed on from it (each of its
    /// words, as typed and as read, goes on from `wider`'s, or is one more) and no word may be
    /// off by more typos. A longer text holds the shorter, a longer reading walk passes the
    /// shorter, a word near a text has its start near that text's start. So a search typed on
    /// needs to look only at what `wider` found.
    pub fn narrows(&self, wider: &Query) -> bool {
        let allowed = |t: &Option<typo::Typo>| t.as_ref().map(typo::Typo::allowed);
        wider.words.len() <= self.words.len()
            && wider.words.iter().zip(&self.words).all(|(w, n)| {
                n.text.starts_with(&w.text)
                    && n.read.starts_with(&w.read)
                    && allowed(&n.typo) == allowed(&w.typo)
                    && allowed(&n.typo_read) == allowed(&w.typo_read)
            })
    }

    /// Nothing typed: everything matches.
    pub fn is_empty(&self) -> bool {
        self.words.is_empty()
    }

    /// `find`, for names whose `Signature` is `sig`: `None` at once when some word cannot be
    /// found in them at all.
    pub fn find_signed<'a>(&self, sig: &Signature, names: impl IntoIterator<Item = &'a str>) -> Option<Found> {
        if self.words.iter().all(|w| w.may_be_in(sig)) {
            self.find(names)
        } else {
            None
        }
    }

    /// How every word is found in one of `names`; `None` when one is not.
    pub fn find<'a>(&self, names: impl IntoIterator<Item = &'a str>) -> Option<Found> {
        if self.words.is_empty() {
            return Some(Found::Text);
        }
        let mut kept = self.names.borrow_mut();
        let (kept, tried) = &mut *kept;
        let mut n = 0;
        for s in names {
            if n == kept.len() {
                kept.push(Name::default());
            }
            kept[n].set(s);
            n += 1;
        }
        let names = &mut kept[..n];
        let mut worst = Found::Text;
        for w in &self.words {
            worst = worst.max(w.find(names, tried)?);
        }
        Some(worst)
    }
}

impl Word {
    /// Some way could find the word in names with signature `sig`: as text (its letters and
    /// other characters all there), by reading (some reading starts as it does), or with a
    /// typo (no more of its letters missing than it may be off by).
    fn may_be_in(&self, sig: &Signature) -> bool {
        let lacks = |word: u64, names: u64| (word & !names).count_ones() as usize;
        (lacks(self.letters, sig.letters) == 0 && self.others & !sig.others == 0)
            || self.read.first().is_some_and(|&c| sig.starts & start(c) != 0)
            || self.typo.as_ref().is_some_and(|t| lacks(self.letters, sig.letters) <= t.allowed())
            || self.typo_read.as_ref().is_some_and(|t| lacks(self.read_letters, sig.read_letters) <= t.allowed())
    }

    /// Each way is tried only where the name's masks allow it: most names of a long list are
    /// passed over on a few bit operations, never walked or spelt out.
    fn find(&self, names: &mut [Name], tried: &mut Vec<bool>) -> Option<Found> {
        if names.iter().any(|n| self.letters & !n.letters == 0 && contains(&n.text, &self.text)) {
            return Some(Found::Text);
        }
        let first = self.read.first().map_or(0, |&c| start(c));
        if names.iter_mut().any(|n| n.starts_any() & first != 0 && by_reading(&self.read, n, tried)) {
            return Some(Found::Reading);
        }
        // A word lacking more of the name's letters than it may be off by is not near it.
        let lacks = |word: u64, name: u64, k: usize| (word & !name).count_ones() as usize > k;
        let near = |n: &mut Name| {
            self.typo.as_ref().is_some_and(|t| !lacks(self.letters, n.letters, t.allowed()) && t.near(&n.text))
                || self
                    .typo_read
                    .as_ref()
                    .is_some_and(|t| !lacks(self.read_letters, n.read_letters(), t.allowed()) && t.near(n.read()))
        };
        names.iter_mut().any(near).then_some(Found::Typo)
    }
}

/// A name folded; its pieces and its reading worked out when a word is not found in it as
/// text. Its buffers are kept for the next name (`set`).
#[derive(Default)]
struct Name {
    text: Vec<char>,
    /// The letters of `text` (`fold::letters`).
    letters: u64,
    pieces: Vec<Piece>,
    /// Per piece, what its readings start with (`fold::start`), and all of them together.
    starts: Vec<u32>,
    starts_any: u32,
    /// The letters of `read`, from each piece's facts (`fold::letters`; a long-vowel mark counts
    /// every vowel).
    read_letters: u64,
    read: Vec<char>,
    has_pieces: bool,
    has_read: bool,
}

impl Name {
    fn set(&mut self, s: &str) {
        self.text.clear();
        fold::fold(s, &mut self.text);
        self.letters = letters(self.text.iter().copied());
        self.has_pieces = false;
        self.has_read = false;
    }

    fn pieces(&mut self) -> &[Piece] {
        if !self.has_pieces {
            self.pieces.clear();
            fold::pieces(&self.text, &mut self.pieces);
            self.starts.clear();
            (self.starts_any, self.read_letters) = (0, 0);
            for i in 0..self.pieces.len() {
                let p = self.pieces[i];
                let c = self.text[p.at];
                // From each character's facts, not by spelling its readings out.
                let (starts, letters) = match p.kind {
                    Kind::Word => (start(c), letters(p.chars(&self.text).iter().copied())),
                    Kind::Han => {
                        let (s, l) = han::table().masks(c);
                        (s | OTHER, l)
                    }
                    Kind::Hangul => hangul::masks(c),
                    Kind::Kana => kana::masks(&self.text, &self.pieces, i),
                };
                self.starts.push(starts);
                self.starts_any |= starts;
                self.read_letters |= letters;
            }
            self.has_pieces = true;
        }
        &self.pieces
    }

    fn starts_any(&mut self) -> u32 {
        self.pieces();
        self.starts_any
    }

    fn read_letters(&mut self) -> u64 {
        self.pieces();
        self.read_letters
    }

    /// The name as read: each piece by its first spelling, with nothing between them
    /// (`周杰伦` `zhoujielun`, `Taylor Swift` `taylorswift`).
    fn read(&mut self) -> &[char] {
        if !self.has_read {
            self.pieces();
            let (text, pieces, out) = (&self.text, &self.pieces, &mut self.read);
            out.clear();
            for (i, p) in pieces.iter().enumerate() {
                match p.kind {
                    Kind::Word => {
                        out.extend_from_slice(p.chars(text));
                        continue;
                    }
                    Kind::Han => {
                        match han::table().first(text[p.at]) {
                            Some(r) => out.extend(r.chars()),
                            None => out.push(text[p.at]),
                        }
                        continue;
                    }
                    Kind::Kana | Kind::Hangul => {}
                }
                // A Han or Hangul piece's first spelling is itself or its jamo: not a reading.
                let mut first = |r: &str| {
                    let spelt = r.starts_with(|c: char| c.is_ascii_alphabetic());
                    if spelt {
                        out.extend(r.chars());
                    }
                    spelt
                };
                if !readings(text, pieces, i, &mut first) {
                    out.extend_from_slice(p.chars(text));
                }
            }
            self.has_read = true;
        }
        &self.read
    }
}

fn contains(text: &[char], word: &[char]) -> bool {
    word.len() <= text.len() && text.windows(word.len()).any(|w| w == word)
}

/// Calls `f` with each reading of the (not `Word`) piece `i` until it returns true (then true).
fn readings(text: &[char], pieces: &[Piece], i: usize, f: &mut dyn FnMut(&str) -> bool) -> bool {
    let p = pieces[i];
    let c = text[p.at];
    match p.kind {
        Kind::Word => false,
        Kind::Kana => kana::readings(text, pieces, i, f),
        Kind::Han => f(c.encode_utf8(&mut [0; 4])) || han::table().readings(c, f),
        Kind::Hangul => f(c.encode_utf8(&mut [0; 4])) || hangul::readings(c, f),
    }
}

/// `word` is the starts of the readings of pieces of `name` in a row.
fn by_reading(word: &[char], name: &mut Name, tried: &mut Vec<bool>) -> bool {
    name.pieces();
    let (text, pieces, starts) = (&name.text, &name.pieces, &name.starts);
    if word.is_empty() || pieces.is_empty() {
        return false;
    }
    // (piece, typed so far) pairs already tried and not found from.
    tried.clear();
    tried.resize((pieces.len() + 1) * (word.len() + 1), false);
    let mut walk = Walk { word, text, pieces, starts, tried };
    (0..pieces.len()).any(|start| walk.from(start, 0))
}

struct Walk<'a> {
    word: &'a [char],
    text: &'a [char],
    pieces: &'a [Piece],
    starts: &'a [u32],
    tried: &'a mut Vec<bool>,
}

impl Walk<'_> {
    /// The rest of the word from `at` is the starts of readings of the pieces from `piece` on.
    /// A word of letters is typed whole, by its initial, or cut where the typing ends; any other
    /// piece by any start of one of its readings, or skipped where a reading is nothing (`っ`).
    fn from(&mut self, piece: usize, at: usize) -> bool {
        if piece == self.pieces.len() {
            return false;
        }
        // No reading of the piece starts with what is typed next, and none is nothing.
        if self.starts[piece] & (start(self.word[at]) | NOTHING) == 0 {
            return false;
        }
        let key = piece * (self.word.len() + 1) + at;
        if self.tried[key] {
            return false;
        }
        let rest = &self.word[at..];
        let p = self.pieces[piece];
        let found = if p.kind == Kind::Word {
            let w = p.chars(self.text);
            let same = rest.iter().zip(w).take_while(|(a, b)| a == b).count();
            same > 0
                && (same == rest.len()
                    || (same == w.len() && self.from(piece + 1, at + same))
                    || self.from(piece + 1, at + 1))
        } else {
            let (text, pieces) = (self.text, self.pieces);
            readings(text, pieces, piece, &mut |r| {
                if r.is_empty() {
                    return at > 0 && self.from(piece + 1, at);
                }
                let same = rest.iter().zip(r.chars()).take_while(|(a, b)| **a == *b).count();
                (1..=same).rev().any(|k| at + k == self.word.len() || self.from(piece + 1, at + k))
            })
        };
        if !found {
            self.tried[key] = true;
        }
        found
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// How `q` is found in `names`; the signature's shortcut never decides otherwise.
    fn found(q: &str, names: &[&str]) -> Option<Found> {
        let query = Query::new(q);
        let found = query.find(names.iter().copied());
        let sig = Signature::of(names.iter().copied());
        assert_eq!(query.find_signed(&sig, names.iter().copied()), found, "{q} in {names:?}, by signature");
        found
    }

    #[test]
    fn every_word_is_found_as_text_in_some_name() {
        let song = ["Shape of You", "Ed Sheeran", "÷"];
        assert_eq!(found("", &song), Some(Found::Text));
        assert_eq!(found("shape SHEERAN", &song), Some(Found::Text));
        assert_eq!(found("ape", &song), Some(Found::Text));
        assert_eq!(found("shape adele", &song), None);
        assert_eq!(found("beyonce", &["Halo", "Beyoncé"]), Some(Found::Text));
        assert_eq!(found("ビートルズ", &["びーとるず"]), Some(Found::Text));
        assert_eq!(found("kino", &["Группа крови", "Кино"]), Some(Found::Text));
    }

    #[test]
    fn chinese_is_found_by_pinyin_whole_initials_or_mixed() {
        let song = ["晴天", "周杰伦", "叶惠美"];
        for q in ["zjl", "zhoujielun", "zhoujl", "zhou jie lun", "qingtian", "qt", "周jl", "jielun", "yhm"] {
            assert_eq!(found(q, &song), Some(Found::Reading), "{q}");
        }
        assert_eq!(found("zhoujielun", &["周杰倫"]), Some(Found::Reading)); // traditional
        assert_eq!(found("chongqing", &["重庆森林"]), Some(Found::Reading)); // a second reading
        assert_eq!(found("lvse", &["绿色"]), Some(Found::Reading));
        assert_eq!(found("lyt", &["周杰伦"]), None);
    }

    #[test]
    fn cantonese_is_found_by_jyutping() {
        assert_eq!(found("hoifut", &["海闊天空"]), Some(Found::Reading));
    }

    #[test]
    fn japanese_is_found_by_romaji_with_or_without_long_vowels() {
        for q in ["tokyo", "toukyou", "とうきょう"] {
            assert_eq!(found(q, &["東京"]), Some(Found::Reading), "{q}");
        }
        for q in ["tokyo", "toukyou"] {
            assert!(found(q, &["とうきょう"]).is_some(), "{q}");
        }
        assert_eq!(found("kitto", &["きっと"]), Some(Found::Reading));
        assert_eq!(found("shashin", &["シャシン"]), Some(Found::Reading));
        assert_eq!(found("syasin", &["写真"]), Some(Found::Reading));
        assert_eq!(found("yorunikakeru", &["夜に駆ける"]), Some(Found::Reading));
    }

    #[test]
    fn korean_is_found_by_romanisation_or_first_consonants() {
        assert_eq!(found("hanguk", &["한국"]), Some(Found::Reading));
        assert_eq!(found("ㅎㄱ", &["한국"]), Some(Found::Reading));
        assert_eq!(found("한ㄱ", &["한국"]), Some(Found::Reading));
        assert_eq!(found("bangtan", &["방탄소년단"]), Some(Found::Reading));
    }

    #[test]
    fn latin_words_are_found_whole_or_by_initials() {
        assert_eq!(found("tswift", &["Taylor Swift"]), Some(Found::Reading));
        assert_eq!(found("ts", &["Taylor Swift"]), Some(Found::Reading));
        assert_eq!(found("eds", &["Ed Sheeran"]), Some(Found::Reading));
        assert_eq!(found("ws", &["Taylor Swift"]), None);
    }

    #[test]
    fn a_typo_is_found_last() {
        assert_eq!(found("beatels", &["The Beatles"]), Some(Found::Typo));
        assert_eq!(found("metalica", &["Metallica"]), Some(Found::Typo));
        assert_eq!(found("zhoujielum", &["周杰伦"]), Some(Found::Typo));
        assert_eq!(found("abc", &["abd"]), None); // too short for a typo
        assert_eq!(found("metalica", &["Megadeth"]), None);
    }

    /// A query typed on from another finds nothing the other did not, unless a word now may be
    /// off by a typo more (or its romaji is not the shorter's typed on: `し` `shi`, `しょ` `sho`).
    #[test]
    fn a_query_typed_on_narrows_the_one_before() {
        let q = |s: &str| Query::new(s);
        assert!(q("lov").narrows(&q("lo")));
        assert!(q("love song").narrows(&q("love")));
        assert!(q("zhoujl").narrows(&q("zhou")));
        assert!(q("とうきょう").narrows(&q("とうきょ")));
        assert!(!q("とうき").narrows(&q("とう"))); // `touki`: five letters, a typo allowed
        assert!(!q("love").narrows(&q("lov"))); // 4 letters: one typo allowed now
        assert!(!q("しょ").narrows(&q("し")));
        assert!(!q("lo").narrows(&q("la")));
        assert!(!q("lo").narrows(&q("lo song")));
    }

    #[test]
    fn the_least_well_found_word_ranks_the_thing() {
        let song = ["晴天", "周杰伦", "叶惠美"];
        assert_eq!(found("晴天 zjl", &song), Some(Found::Reading));
        assert!(Found::Text < Found::Reading && Found::Reading < Found::Typo);
    }
}
