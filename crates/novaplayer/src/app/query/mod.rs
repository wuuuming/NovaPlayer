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

use fold::{Kind, Piece};
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

pub struct Query {
    words: Vec<Word>,
    /// The names of the thing looked at, kept between things so a search over a long list
    /// allocates nothing per thing.
    names: RefCell<(Vec<Name>, Vec<bool>)>,
}

/// One typed word: folded, as read (its kana in romaji), and each looked for with typos.
struct Word {
    text: Vec<char>,
    read: Vec<char>,
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
            (!text.is_empty()).then_some(Word { text, read, typo, typo_read })
        });
        Query { words: words.collect(), names: RefCell::default() }
    }

    /// Nothing typed: everything matches.
    pub fn is_empty(&self) -> bool {
        self.words.is_empty()
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
    fn find(&self, names: &mut [Name], tried: &mut Vec<bool>) -> Option<Found> {
        if names.iter().any(|n| contains(&n.text, &self.text)) {
            return Some(Found::Text);
        }
        if names.iter_mut().any(|n| by_reading(&self.read, n, tried)) {
            return Some(Found::Reading);
        }
        let near = |n: &mut Name| {
            self.typo.as_ref().is_some_and(|t| t.near(&n.text))
                || self.typo_read.as_ref().is_some_and(|t| t.near(n.read()))
        };
        names.iter_mut().any(near).then_some(Found::Typo)
    }
}

/// A name folded; its pieces and its reading worked out when a word is not found in it as
/// text. Its buffers are kept for the next name (`set`).
#[derive(Default)]
struct Name {
    text: Vec<char>,
    pieces: Vec<Piece>,
    /// Per piece, what its readings start with (`starts`).
    starts: Vec<u32>,
    read: Vec<char>,
    has_pieces: bool,
    has_read: bool,
}

impl Name {
    fn set(&mut self, s: &str) {
        self.text.clear();
        fold::fold(s, &mut self.text);
        self.has_pieces = false;
        self.has_read = false;
    }

    fn pieces(&mut self) -> &[Piece] {
        if !self.has_pieces {
            self.pieces.clear();
            fold::pieces(&self.text, &mut self.pieces);
            self.starts.clear();
            for i in 0..self.pieces.len() {
                let mut m = 0;
                if self.pieces[i].kind == Kind::Word {
                    m = start(self.text[self.pieces[i].at]);
                } else {
                    readings(&self.text, &self.pieces, i, &mut |r| {
                        m |= r.chars().next().map_or(NOTHING, start);
                        false
                    });
                }
                self.starts.push(m);
            }
            self.has_pieces = true;
        }
        &self.pieces
    }

    /// The name as read: each piece by its first spelling, with nothing between them
    /// (`周杰伦` `zhoujielun`, `Taylor Swift` `taylorswift`).
    fn read(&mut self) -> &[char] {
        if !self.has_read {
            self.pieces();
            let (text, pieces, out) = (&self.text, &self.pieces, &mut self.read);
            out.clear();
            for (i, p) in pieces.iter().enumerate() {
                if p.kind == Kind::Word {
                    out.extend_from_slice(p.chars(text));
                    continue;
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

/// What a reading starting with `c` can be found by: a bit per letter `a`-`z`, one for any
/// other character (`OTHER`); `NOTHING` for a reading that is nothing (`っ`, skipped).
fn start(c: char) -> u32 {
    if c.is_ascii_lowercase() {
        1 << (c as u32 - 'a' as u32)
    } else {
        OTHER
    }
}
const OTHER: u32 = 1 << 30;
const NOTHING: u32 = 1 << 31;

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

    fn found(q: &str, names: &[&str]) -> Option<Found> {
        Query::new(q).find(names.iter().copied())
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

    #[test]
    fn the_least_well_found_word_ranks_the_thing() {
        let song = ["晴天", "周杰伦", "叶惠美"];
        assert_eq!(found("晴天 zjl", &song), Some(Found::Reading));
        assert!(Found::Text < Found::Reading && Found::Reading < Found::Typo);
    }
}
