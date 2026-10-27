//! Kana in romaji: how a mora of a name is typed, and kana text spelt out (a typed query, a
//! kanji's Japanese reading).

use super::fold::{is_kana, is_small, Kind, Piece};

/// A kana's spellings: Hepburn, Kunrei, and others people type (`は` as the particle `wa`, `を`
/// as `o`, `ん` as `nn`).
fn mora(c: char) -> Option<(&'static str, &'static str, &'static [&'static str])> {
    let plain = |s| Some((s, s, &[][..]));
    match c {
        'あ' | 'ぁ' => plain("a"),
        'い' | 'ぃ' => plain("i"),
        'う' | 'ぅ' => plain("u"),
        'え' | 'ぇ' => plain("e"),
        'お' | 'ぉ' => plain("o"),
        'か' | 'ゕ' => plain("ka"),
        'き' => plain("ki"),
        'く' => plain("ku"),
        'け' | 'ゖ' => plain("ke"),
        'こ' => plain("ko"),
        'が' => plain("ga"),
        'ぎ' => plain("gi"),
        'ぐ' => plain("gu"),
        'げ' => plain("ge"),
        'ご' => plain("go"),
        'さ' => plain("sa"),
        'し' => Some(("shi", "si", &[])),
        'す' => plain("su"),
        'せ' => plain("se"),
        'そ' => plain("so"),
        'ざ' => plain("za"),
        'じ' => Some(("ji", "zi", &[])),
        'ず' => plain("zu"),
        'ぜ' => plain("ze"),
        'ぞ' => plain("zo"),
        'た' => plain("ta"),
        'ち' => Some(("chi", "ti", &[])),
        'つ' => Some(("tsu", "tu", &[])),
        'て' => plain("te"),
        'と' => plain("to"),
        'だ' => plain("da"),
        'ぢ' => Some(("ji", "zi", &["di"])),
        'づ' => Some(("zu", "zu", &["du"])),
        'で' => plain("de"),
        'ど' => plain("do"),
        'な' => plain("na"),
        'に' => plain("ni"),
        'ぬ' => plain("nu"),
        'ね' => plain("ne"),
        'の' => plain("no"),
        'は' => Some(("ha", "ha", &["wa"])),
        'ひ' => plain("hi"),
        'ふ' => Some(("fu", "hu", &[])),
        'へ' => Some(("he", "he", &["e"])),
        'ほ' => plain("ho"),
        'ば' => plain("ba"),
        'び' => plain("bi"),
        'ぶ' => plain("bu"),
        'べ' => plain("be"),
        'ぼ' => plain("bo"),
        'ぱ' => plain("pa"),
        'ぴ' => plain("pi"),
        'ぷ' => plain("pu"),
        'ぺ' => plain("pe"),
        'ぽ' => plain("po"),
        'ま' => plain("ma"),
        'み' => plain("mi"),
        'む' => plain("mu"),
        'め' => plain("me"),
        'も' => plain("mo"),
        'や' | 'ゃ' => plain("ya"),
        'ゆ' | 'ゅ' => plain("yu"),
        'よ' | 'ょ' => plain("yo"),
        'ら' => plain("ra"),
        'り' => plain("ri"),
        'る' => plain("ru"),
        'れ' => plain("re"),
        'ろ' => plain("ro"),
        'わ' | 'ゎ' => plain("wa"),
        'ゐ' => Some(("i", "i", &["wi"])),
        'ゑ' => Some(("e", "e", &["we"])),
        'を' => Some(("o", "o", &["wo"])),
        'ん' => Some(("n", "n", &["nn"])),
        'ゔ' => plain("vu"),
        _ => None,
    }
}

fn is_vowel(c: char) -> bool {
    matches!(c, 'a' | 'i' | 'u' | 'e' | 'o')
}

/// A kana with a small kana after it (`きょ`, `ふぁ`), spelt from one spelling of the first:
/// its consonant, then the small kana (`ky` + `o`); `sh`, `ch` and `j` also without the `y`
/// (`sho` beside `shyo`).
fn joined(first: &str, base: char, small: char, out: &mut String) -> bool {
    let Some((tail, ..)) = mora(small) else { return false };
    let stem = if base == 'う' { "w" } else { first.trim_end_matches(is_vowel) };
    out.clear();
    out.push_str(stem);
    out.push_str(tail);
    true
}

/// The spelling without the `y` of a joined `sh`/`ch`/`j` mora (`sha` for `shya`), if any.
fn without_y(s: &str) -> Option<String> {
    ["shy", "chy", "jy"].iter().find(|p| s.starts_with(**p)).map(|p| format!("{}{}", &p[..p.len() - 1], &s[p.len()..]))
}

/// Calls `f` with each way the kana piece `i` of a name is typed, until it returns true (then
/// true). `っ` is typed as nothing or as the next mora's consonant (`きっと` `kitto`, `kito`);
/// `ー`, and `う` / `お` lengthening a vowel, as nothing or as the vowel (`とうきょう` `tokyo`).
pub(super) fn readings(text: &[char], pieces: &[Piece], i: usize, f: &mut dyn FnMut(&str) -> bool) -> bool {
    let p = pieces[i];
    let kana = |j: usize| pieces.get(j).filter(|q| q.kind == Kind::Kana);
    let prev = i.checked_sub(1).and_then(kana).filter(|q| q.at + q.len == p.at);
    let next = kana(i + 1).filter(|q| q.at == p.at + p.len);
    let c = text[p.at];
    match c {
        'っ' => {
            if f("") {
                return true;
            }
            let Some(n) = next else { return false };
            let mut firsts = Vec::new();
            spellings(n.chars(text), &mut |s| {
                if let Some(c) = s.chars().next().filter(|c| !is_vowel(*c)) {
                    if !firsts.contains(&c) {
                        firsts.push(c);
                    }
                }
                false
            });
            firsts.into_iter().any(|c| f(c.encode_utf8(&mut [0; 4])))
        }
        'ー' | 'ゝ' | 'ゞ' => {
            if f("") {
                return true;
            }
            let vowel = prev.and_then(|q| hepburn(q.chars(text)).chars().last()).filter(|c| is_vowel(*c));
            c == 'ー' && vowel.is_some_and(|v| f(v.encode_utf8(&mut [0; 4])))
        }
        _ => {
            let lengthens = match c {
                'う' => prev.map(|q| hepburn(q.chars(text))).is_some_and(|h| h.ends_with('o') || h.ends_with('u')),
                'お' => prev.map(|q| hepburn(q.chars(text))).is_some_and(|h| h.ends_with('o')),
                _ => false,
            };
            if p.len == 1 && lengthens && f("") {
                return true;
            }
            spellings(p.chars(text), f)
        }
    }
}

/// What the ways the kana piece `i` is typed start with, and the letters of the first that is not
/// nothing (the one a name's reading takes): at least what `readings` gives (`っ` as the next
/// mora's consonants, `ー` as any vowel, `う` / `お` as nothing too), without working out which
/// vowel or consonant it takes.
pub(super) fn masks(text: &[char], pieces: &[Piece], i: usize) -> (u32, u64) {
    use super::fold::{letters, start, NOTHING};
    let p = pieces[i];
    let (mut starts, mut first) = (0, None);
    let mut add = |s: &str| {
        starts |= s.chars().next().map_or(NOTHING, start);
        if first.is_none() && !s.is_empty() {
            first = Some(letters(s.chars()));
        }
        false
    };
    match text[p.at] {
        '\u{3063}' => {
            let next = pieces.get(i + 1).filter(|q| q.kind == Kind::Kana && q.at == p.at + p.len);
            if let Some(n) = next {
                spellings(n.chars(text), &mut add);
            }
            starts |= NOTHING;
        }
        '\u{30FC}' | '\u{309D}' | '\u{309E}' => {
            for v in ["", "a", "i", "u", "e", "o"] {
                add(v);
            }
            first = Some(letters("aiueo".chars()));
        }
        c => {
            spellings(p.chars(text), &mut add);
            if p.len == 1 && matches!(c, '\u{3046}' | '\u{304A}') {
                starts |= NOTHING;
            }
        }
    }
    (starts, first.unwrap_or(0))
}

/// Calls `f` with each spelling of one mora (one kana, or one with its small kana).
fn spellings(m: &[char], f: &mut dyn FnMut(&str) -> bool) -> bool {
    let Some((hep, kun, more)) = mora(m[0]) else { return false };
    let all = std::iter::once(hep).chain((kun != hep).then_some(kun)).chain(more.iter().copied());
    match m.get(1) {
        None => {
            for s in all {
                if f(s) {
                    return true;
                }
            }
            false
        }
        Some(&small) => {
            let mut buf = String::new();
            for s in all {
                if joined(s, m[0], small, &mut buf) {
                    // `sho` before `shyo`: the first is the one a name's reading takes.
                    if without_y(&buf).is_some_and(|short| f(&short)) || f(&buf) {
                        return true;
                    }
                }
            }
            false
        }
    }
}

/// One mora in Hepburn (`きょ` `kyo`, `しゃ` `sha`).
fn hepburn(m: &[char]) -> String {
    let mut out = String::new();
    spellings(m, &mut |s| {
        out = s.to_owned();
        true
    });
    out
}

/// Kana text spelt out in Hepburn (`kunrei` false) or Kunrei; anything else kept as it is.
/// `っ` doubles the next consonant, `ー` repeats the vowel before it.
pub(super) fn romaji(text: &[char], kunrei: bool) -> String {
    let mut out = String::new();
    let mut i = 0;
    while i < text.len() {
        let c = text[i];
        if !is_kana(c) {
            out.push(c);
            i += 1;
            continue;
        }
        let len = if c != 'っ' && c != 'ー' && !is_small(c) && text.get(i + 1).is_some_and(|&n| is_small(n)) {
            2
        } else {
            1
        };
        let m = &text[i..i + len];
        match c {
            'っ' => {
                let next = text.get(i + 1).copied().filter(|&n| is_kana(n)).map(|n| spelt(&[n], kunrei));
                if let Some(c) = next.and_then(|s| s.chars().next()).filter(|c| !is_vowel(*c)) {
                    out.push(c);
                }
            }
            'ー' => {
                if let Some(v) = out.chars().last().filter(|c| is_vowel(*c)) {
                    out.push(v);
                }
            }
            _ => out.push_str(&spelt(m, kunrei)),
        }
        i += len;
    }
    out
}

/// One mora in Hepburn or Kunrei.
fn spelt(m: &[char], kunrei: bool) -> String {
    if !kunrei {
        return hepburn(m);
    }
    let Some((_, kun, _)) = mora(m[0]) else { return m.iter().collect() };
    match m.get(1) {
        None => kun.to_owned(),
        Some(&small) => {
            let mut buf = String::new();
            if joined(kun, m[0], small, &mut buf) {
                buf
            } else {
                kun.to_owned()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chars(s: &str) -> Vec<char> {
        s.chars().collect()
    }

    #[test]
    fn kana_is_spelt_in_hepburn_and_kunrei() {
        assert_eq!(romaji(&chars("とうきょう"), false), "toukyou");
        assert_eq!(romaji(&chars("しゃしん"), false), "shashin");
        assert_eq!(romaji(&chars("しゃしん"), true), "syasin");
        assert_eq!(romaji(&chars("きっと"), false), "kitto");
        assert_eq!(romaji(&chars("らーめん"), false), "raamen");
        assert_eq!(romaji(&chars("ちょっと"), true), "tyotto");
        assert_eq!(romaji(&chars("ふぁん"), false), "fan");
        assert_eq!(romaji(&chars("周jl"), false), "周jl");
    }

    #[test]
    fn a_mora_is_found_by_each_way_it_is_typed() {
        let text = chars("きっと");
        let mut p = Vec::new();
        super::super::fold::pieces(&text, &mut p);
        let all = |i| {
            let mut v = Vec::new();
            readings(&text, &p, i, &mut |s| {
                v.push(s.to_owned());
                false
            });
            v
        };
        assert_eq!(all(0), ["ki"]);
        assert_eq!(all(1), ["", "t"]);
        assert_eq!(all(2), ["to"]);
        let text = chars("しょう");
        let mut p = Vec::new();
        super::super::fold::pieces(&text, &mut p);
        let mut first = Vec::new();
        readings(&text, &p, 0, &mut |s| {
            first.push(s.to_owned());
            false
        });
        assert_eq!(first, ["sho", "shyo", "syo"]);
        let mut second = Vec::new();
        readings(&text, &p, 1, &mut |s| {
            second.push(s.to_owned());
            false
        });
        assert_eq!(second, ["", "u"]);
    }
}
