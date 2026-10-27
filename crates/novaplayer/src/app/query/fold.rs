//! Text as search compares it, and the pieces a name is read in.

use super::latin::LATIN;

/// Appends `s` as search compares it: lowercase; marks dropped and ligatures spelt out (`é` →
/// `e`, `ß` → `ss`); full-width forms as ASCII; Cyrillic and Greek in Latin letters (one common
/// transliteration); katakana as hiragana. Both what is typed and what it is found in go through
/// this, so the two always agree.
pub(super) fn fold(s: &str, out: &mut Vec<char>) {
    for c in s.chars() {
        // Most of a name: ASCII, and ideographs, kana and Hangul past the scripts with case.
        if c.is_ascii() {
            out.push(c.to_ascii_lowercase());
            continue;
        }
        if is_han(c) || ('\u{3041}'..='\u{3096}').contains(&c) || is_hangul(c) {
            out.push(c);
            continue;
        }
        for c in c.to_lowercase() {
            let c = narrow(c);
            if c.is_ascii() {
                out.push(c);
            } else if let Ok(i) = LATIN.binary_search_by_key(&c, |p| p.0) {
                LATIN[i].1.chars().for_each(|b| letter(b, out));
            } else {
                letter(c, out);
            }
        }
    }
}

/// A full-width ASCII form as ASCII, the ideographic space as a space.
fn narrow(c: char) -> char {
    match c {
        '\u{FF01}'..='\u{FF5E}' => char::from_u32(c as u32 - 0xFEE0).unwrap_or(c),
        '\u{3000}' => ' ',
        _ => c,
    }
}

/// A letter without marks: Cyrillic and Greek spelt in Latin letters, katakana as hiragana.
fn letter(c: char, out: &mut Vec<char>) {
    if let Some(latin) = cyrillic(c).or_else(|| greek(c)) {
        out.extend(latin.chars());
    } else {
        out.push(match c {
            '\u{30A1}'..='\u{30F6}' | '\u{30FD}' | '\u{30FE}' => char::from_u32(c as u32 - 0x60).unwrap_or(c),
            _ => c,
        });
    }
}

/// Russian, Ukrainian, Belarusian, Bulgarian, Serbian and Macedonian letters in Latin (the
/// spellings English text uses: `ж` zh, `х` kh, `щ` shch).
fn cyrillic(c: char) -> Option<&'static str> {
    Some(match c {
        'а' => "a",
        'б' => "b",
        'в' => "v",
        'г' | 'ґ' => "g",
        'д' => "d",
        'е' | 'ё' | 'э' => "e",
        'є' => "ye",
        'ж' => "zh",
        'з' => "z",
        'и' | 'і' => "i",
        'ї' => "yi",
        'й' => "y",
        'к' => "k",
        'л' => "l",
        'м' => "m",
        'н' => "n",
        'о' => "o",
        'п' => "p",
        'р' => "r",
        'с' => "s",
        'т' => "t",
        'у' | 'ў' => "u",
        'ф' => "f",
        'х' => "kh",
        'ц' => "ts",
        'ч' => "ch",
        'ш' => "sh",
        'щ' => "shch",
        'ъ' | 'ь' => "",
        'ы' => "y",
        'ю' => "yu",
        'я' => "ya",
        'ђ' => "dj",
        'ј' => "j",
        'љ' => "lj",
        'њ' => "nj",
        'ћ' => "c",
        'џ' => "dz",
        'ѓ' => "gj",
        'ќ' => "kj",
        'ѕ' => "dz",
        _ => return None,
    })
}

/// Greek letters (their marks already dropped) in Latin, as ELOT 743 spells them.
fn greek(c: char) -> Option<&'static str> {
    Some(match c {
        'α' => "a",
        'β' => "v",
        'γ' => "g",
        'δ' => "d",
        'ε' => "e",
        'ζ' => "z",
        'η' | 'ι' => "i",
        'θ' => "th",
        'κ' => "k",
        'λ' => "l",
        'μ' => "m",
        'ν' => "n",
        'ξ' => "x",
        'ο' | 'ω' => "o",
        'π' => "p",
        'ρ' => "r",
        'σ' | 'ς' => "s",
        'τ' => "t",
        'υ' => "y",
        'φ' => "f",
        'χ' => "ch",
        'ψ' => "ps",
        _ => return None,
    })
}

/// What a piece of a name is read as (`Piece`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Kind {
    /// A run of letters or digits (a Latin word, a Thai phrase): read as itself.
    Word,
    /// A CJK ideograph: its Mandarin, Cantonese and Japanese readings (`han.rs`).
    Han,
    /// A kana mora (one kana, or one with its small kana: `きょ`): its romaji (`kana.rs`).
    Kana,
    /// A Hangul syllable: its romanisations and its first consonant (`hangul.rs`).
    Hangul,
}

/// A piece of a folded name: `len` characters from `at`. Spaces and punctuation are no piece,
/// so the pieces either side of them are next to each other.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Piece {
    pub at: usize,
    pub len: usize,
    pub kind: Kind,
}

impl Piece {
    pub fn chars<'a>(&self, text: &'a [char]) -> &'a [char] {
        &text[self.at..self.at + self.len]
    }
}

pub(super) fn is_han(c: char) -> bool {
    matches!(c, '\u{3005}' | '\u{3007}' | '\u{3400}'..='\u{4DBF}' | '\u{4E00}'..='\u{9FFF}' | '\u{F900}'..='\u{FAFF}' | '\u{20000}'..='\u{3134F}')
}

/// Hiragana (katakana is folded into it), the iteration marks and the long-vowel mark.
pub(super) fn is_kana(c: char) -> bool {
    matches!(c, '\u{3041}'..='\u{3096}' | '\u{309D}' | '\u{309E}' | '\u{30FC}')
}

/// A small kana that joins the kana before it into one mora (`き` + `ょ`).
pub(super) fn is_small(c: char) -> bool {
    matches!(c, 'ぁ' | 'ぃ' | 'ぅ' | 'ぇ' | 'ぉ' | 'ゃ' | 'ゅ' | 'ょ' | 'ゎ')
}

fn is_hangul(c: char) -> bool {
    matches!(c, '\u{AC00}'..='\u{D7A3}')
}

/// Splits a folded name into its pieces.
pub(super) fn pieces(text: &[char], out: &mut Vec<Piece>) {
    let mut i = 0;
    while i < text.len() {
        let c = text[i];
        let (len, kind) = if is_han(c) {
            (1, Kind::Han)
        } else if is_hangul(c) {
            (1, Kind::Hangul)
        } else if is_kana(c) {
            let joins = c != 'っ' && c != 'ー' && !is_small(c) && text.get(i + 1).is_some_and(|&n| is_small(n));
            (if joins { 2 } else { 1 }, Kind::Kana)
        } else if c.is_alphanumeric() {
            let n = text[i..]
                .iter()
                .take_while(|&&c| c.is_alphanumeric() && !is_han(c) && !is_kana(c) && !is_hangul(c))
                .count();
            (n, Kind::Word)
        } else {
            i += 1;
            continue;
        };
        out.push(Piece { at: i, len, kind });
        i += len;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn folded(s: &str) -> String {
        let mut out = Vec::new();
        fold(s, &mut out);
        out.into_iter().collect()
    }

    #[test]
    fn text_folds_to_what_a_plain_keyboard_types() {
        assert_eq!(folded("Beyoncé"), "beyonce");
        assert_eq!(folded("Mötley Crüe"), "motley crue");
        assert_eq!(folded("Straße"), "strasse");
        assert_eq!(folded("Sigur Rós Ágætis"), "sigur ros agaetis");
        assert_eq!(folded("Sơn Tùng M-TP"), "son tung m-tp");
        assert_eq!(folded("ＡＢＣ１２３"), "abc123");
        assert_eq!(folded("Кино"), "kino");
        assert_eq!(folded("Земфира"), "zemfira");
        assert_eq!(folded("Щедрик"), "shchedrik");
        assert_eq!(folded("Βαγγέλης"), "vaggelis");
        assert_eq!(folded("カタカナ"), "かたかな");
    }

    #[test]
    fn a_name_splits_into_words_characters_and_morae() {
        let mut text = Vec::new();
        fold("周杰伦 Jay キョウ", &mut text);
        let mut p = Vec::new();
        pieces(&text, &mut p);
        let kinds: Vec<_> = p.iter().map(|p| (p.chars(&text).iter().collect::<String>(), p.kind)).collect();
        assert_eq!(
            kinds,
            [
                ("周".into(), Kind::Han),
                ("杰".into(), Kind::Han),
                ("伦".into(), Kind::Han),
                ("jay".into(), Kind::Word),
                ("きょ".into(), Kind::Kana),
                ("う".into(), Kind::Kana)
            ]
        );
    }
}
