//! A Hangul syllable in Latin letters: the Revised Romanization, the spellings names are often
//! written in (`김` kim, `정` jung, `박` park), and its first consonant as a Korean keyboard
//! types it (`ㅎㄱ` for `한국`).

const INITIAL: [&str; 19] =
    ["g", "kk", "n", "d", "tt", "r", "m", "b", "pp", "s", "ss", "", "j", "jj", "ch", "k", "t", "p", "h"];
/// The initial as names are often written (the older voiceless spellings).
const INITIAL_ALSO: [&str; 19] =
    ["k", "kk", "n", "t", "tt", "l", "m", "p", "pp", "s", "ss", "", "ch", "jj", "ch", "k", "t", "p", "h"];
/// The compatibility jamo a Korean keyboard types for each initial.
const INITIAL_JAMO: [char; 19] =
    ['ㄱ', 'ㄲ', 'ㄴ', 'ㄷ', 'ㄸ', 'ㄹ', 'ㅁ', 'ㅂ', 'ㅃ', 'ㅅ', 'ㅆ', 'ㅇ', 'ㅈ', 'ㅉ', 'ㅊ', 'ㅋ', 'ㅌ', 'ㅍ', 'ㅎ'];
const MEDIAL: [&[&str]; 21] = [
    &["a"],
    &["ae"],
    &["ya"],
    &["yae"],
    &["eo", "u", "o"],
    &["e"],
    &["yeo", "yo"],
    &["ye"],
    &["o"],
    &["wa"],
    &["wae"],
    &["oe", "oi"],
    &["yo"],
    &["u", "oo"],
    &["wo"],
    &["we"],
    &["wi"],
    &["yu", "yoo"],
    &["eu", "u"],
    &["ui", "eui"],
    &["i"],
];
const FINAL: [&[&str]; 28] = [
    &[""],
    &["k", "g"],
    &["k"],
    &["k"],
    &["n"],
    &["n"],
    &["n"],
    &["t", "d"],
    &["l", "r"],
    &["k"],
    &["m"],
    &["l"],
    &["l"],
    &["l"],
    &["p"],
    &["l"],
    &["m"],
    &["p", "b"],
    &["p"],
    &["t", "s"],
    &["t"],
    &["ng"],
    &["t"],
    &["t"],
    &["k"],
    &["t"],
    &["p"],
    &["t"],
];

/// Surnames whose usual spelling no rule gives.
const SURNAMES: [(char, &[&str]); 9] = [
    ('이', &["lee", "yi", "rhee"]),
    ('박', &["park"]),
    ('최', &["choi"]),
    ('오', &["oh"]),
    ('노', &["noh", "roh"]),
    ('류', &["yoo"]),
    ('임', &["lim"]),
    ('서', &["suh"]),
    ('손', &["sohn"]),
];

/// What the spellings of syllable `c` start with (its jamo is `OTHER`), and the letters of the
/// first (the Revised Romanization), from the tables, without spelling them out (`readings`).
pub(super) fn masks(c: char) -> (u32, u64) {
    use super::fold::{letters, start, OTHER};
    let Some(s) = (c as u32).checked_sub(0xAC00).filter(|s| *s < 11172) else { return (0, 0) };
    let (i, m, t) = ((s / 588) as usize, (s % 588 / 28) as usize, (s % 28) as usize);
    let mut starts = OTHER;
    let sh = i == 9 && MEDIAL[m][0].starts_with(['i', 'y']);
    for a in [INITIAL[i], INITIAL_ALSO[i]].into_iter().chain(sh.then_some("sh")) {
        match a.chars().next() {
            Some(c) => starts |= start(c),
            None => MEDIAL[m].iter().filter_map(|b| b.chars().next()).for_each(|c| starts |= start(c)),
        }
    }
    for n in SURNAMES.iter().filter(|n| n.0 == c).flat_map(|n| n.1.iter()) {
        starts |= n.chars().next().map_or(0, start);
    }
    (starts, letters(INITIAL[i].chars().chain(MEDIAL[m][0].chars()).chain(FINAL[t][0].chars())))
}

/// Calls `f` with each way the syllable `c` is typed, until it returns true (then true): its
/// first consonant's jamo, then its spellings.
pub(super) fn readings(c: char, f: &mut dyn FnMut(&str) -> bool) -> bool {
    let Some(s) = (c as u32).checked_sub(0xAC00).filter(|s| *s < 11172) else { return false };
    let (i, m, t) = ((s / 588) as usize, (s % 588 / 28) as usize, (s % 28) as usize);
    if f(INITIAL_JAMO[i].encode_utf8(&mut [0; 4])) {
        return true;
    }
    let medial = MEDIAL[m];
    // `시`, `샤`, `셔`, `쇼`, `슈`: also `sh` (`shin`).
    let sh = i == 9 && medial[0].starts_with(['i', 'y']);
    let initials = [Some(INITIAL[i]), (INITIAL_ALSO[i] != INITIAL[i]).then_some(INITIAL_ALSO[i]), sh.then_some("sh")];
    // At most 2 + 3 + 2 ASCII letters: spelt in place, no allocation per spelling.
    let mut buf = [0u8; 8];
    for a in initials.into_iter().flatten() {
        for b in medial {
            for z in FINAL[t] {
                let n = a.len() + b.len() + z.len();
                buf[..a.len()].copy_from_slice(a.as_bytes());
                buf[a.len()..a.len() + b.len()].copy_from_slice(b.as_bytes());
                buf[a.len() + b.len()..n].copy_from_slice(z.as_bytes());
                if f(std::str::from_utf8(&buf[..n]).unwrap_or_default()) {
                    return true;
                }
            }
        }
    }
    SURNAMES.iter().find(|n| n.0 == c).is_some_and(|n| n.1.iter().any(|s| f(s)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn all(c: char) -> Vec<String> {
        let mut v = Vec::new();
        readings(c, &mut |s| {
            v.push(s.to_owned());
            false
        });
        v
    }

    #[test]
    fn a_syllable_is_found_by_its_romanisations_and_first_consonant() {
        assert_eq!(all('한'), ["ㅎ", "han"]);
        assert_eq!(all('국'), ["ㄱ", "guk", "gug", "gook", "goog", "kuk", "kug", "kook", "koog"]);
        assert!(all('김').contains(&"kim".to_owned()));
        assert!(all('정').contains(&"jung".to_owned()) && all('정').contains(&"chung".to_owned()));
        assert!(all('신').contains(&"shin".to_owned()));
        assert!(all('이').contains(&"lee".to_owned()));
        assert!(all('박').contains(&"park".to_owned()));
        assert!(all('a').is_empty());
    }
}
