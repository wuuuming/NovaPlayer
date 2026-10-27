//! A word found with a typo: somewhere in a text, within a few edits, two letters swapped
//! counting as one (Myers' bit-parallel approximate search with Hyyrö's transpositions, one pass
//! over the text).

/// How many edits a word of `len` characters may be off by: none under 4 characters (a short
/// word one letter off is another word), 1 up to 7, then 2.
pub(super) fn allowed(len: usize) -> usize {
    match len {
        0..=3 => 0,
        4..=7 => 1,
        _ => 2,
    }
}

/// A word looked for with typos: each distinct character with the places it is at in the word.
pub(super) struct Typo {
    len: usize,
    /// The edits allowed (`allowed`).
    k: usize,
    /// ASCII characters' places by code; the others' in `places`.
    ascii: [u64; 128],
    places: Vec<(char, u64)>,
}

impl Typo {
    /// `None` for a word that may not be off by any edit, or is longer than 64 characters.
    pub fn new(word: &[char]) -> Option<Typo> {
        let (len, k) = (word.len(), allowed(word.len()));
        if k == 0 || len > 64 {
            return None;
        }
        let (mut ascii, mut places) = ([0u64; 128], Vec::<(char, u64)>::new());
        for (i, &c) in word.iter().enumerate() {
            if c.is_ascii() {
                ascii[c as usize] |= 1 << i;
            } else {
                match places.iter_mut().find(|p| p.0 == c) {
                    Some(p) => p.1 |= 1 << i,
                    None => places.push((c, 1 << i)),
                }
            }
        }
        Some(Typo { len, k, ascii, places })
    }

    /// The edits the word may be off by.
    pub fn allowed(&self) -> usize {
        self.k
    }

    /// The word is within its allowed insertions, deletions, substitutions or swaps of two
    /// neighbours of some part of `text`.
    pub fn near(&self, text: &[char]) -> bool {
        self.near_within(text, self.k)
    }

    /// The places of `c` in the word.
    fn at(&self, c: char) -> u64 {
        if c.is_ascii() {
            self.ascii[c as usize]
        } else {
            self.places.iter().find(|p| p.0 == c).map_or(0, |p| p.1)
        }
    }

    /// The word is within `k` edits of some part of `text`.
    fn near_within(&self, text: &[char], k: usize) -> bool {
        let last = 1u64 << (self.len - 1);
        let (mut vp, mut vn, mut d0, mut eq_before, mut score) = (!0u64, 0u64, 0u64, 0u64, self.len);
        for &c in text {
            let eq = self.at(c);
            let swapped = ((!d0 & eq) << 1) & eq_before;
            d0 = ((eq & vp).wrapping_add(vp) ^ vp) | eq | vn | swapped;
            let hp = vn | !(d0 | vp);
            let hn = d0 & vp;
            if hp & last != 0 {
                score += 1;
            } else if hn & last != 0 {
                score -= 1;
            }
            let (hp, hn) = (hp << 1, hn << 1);
            vp = hn | !(d0 | hp);
            vn = d0 & hp;
            eq_before = eq;
            if score <= k {
                return true;
            }
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn near_in(word: &str, text: &str, k: usize) -> bool {
        let word: Vec<char> = word.chars().collect();
        let t = Typo::new(&word).expect("a word long enough for a typo");
        t.near_within(&text.chars().collect::<Vec<_>>(), k)
    }

    #[test]
    fn a_word_is_found_within_its_edits() {
        assert!(near_in("beatels", "the beatles", 1));
        assert!(!near_in("beatels", "the beatles", 0));
        assert!(!near_in("bteaels", "the beatles", 1));
        assert!(near_in("queen", "quen", 1));
        assert!(near_in("radiohed", "radiohead - creep", 1));
        assert!(near_in("metalica", "metallica", 1));
        assert!(!near_in("metalica", "megadeth", 2));
        assert!(near_in("zhoujielum", "zhoujielun", 1));
        assert_eq!(allowed(3), 0);
        assert_eq!(allowed(4), 1);
        assert_eq!(allowed(8), 2);
    }
}
