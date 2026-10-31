//! What the extra lines of a lyric file are. A file may stamp, under each line, more lines at the
//! same time (spec "Lyrics": original + readings/translations): a romanisation of the line (romaji,
//! pinyin, a Korean romanisation) or a translation (another language, or the same in another
//! script, as simplified under traditional Chinese). Each such line, by its place in the group,
//! is laid under the line as the layer it reads as (`roma`, `trans`) across the whole file.

use super::Group;

/// The script a line is mostly written in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Script {
    /// Any kana: Japanese (with its kanji).
    Kana,
    Han,
    Hangul,
    Latin,
    Other,
}

pub(super) fn script(line: &str) -> Option<Script> {
    let (mut kana, mut han, mut hangul, mut latin, mut other) = (0, 0, 0, 0, 0);
    for c in line.chars().filter(|c| c.is_alphabetic()) {
        match c as u32 {
            0x3040..=0x30FF | 0x31F0..=0x31FF | 0xFF66..=0xFF9F => kana += 1,
            0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xF900..=0xFAFF | 0x20000..=0x2FFFF => han += 1,
            0xAC00..=0xD7AF | 0x1100..=0x11FF | 0x3130..=0x318F => hangul += 1,
            0..=0x24F | 0x1E00..=0x1EFF => latin += 1,
            _ => other += 1,
        }
    }
    if kana > 0 {
        return Some(Script::Kana);
    }
    let best = [(han, Script::Han), (hangul, Script::Hangul), (latin, Script::Latin), (other, Script::Other)]
        .into_iter()
        .max_by_key(|(n, _)| *n)?;
    (best.0 > 0).then_some(best.1)
}

/// The words of `line` that read as a romanisation of a song in `main`, and its words.
fn romanised(line: &str, main: Script) -> (usize, usize) {
    let (mut ok, mut all) = (0, 0);
    for raw in line.split(|c: char| !(c.is_alphanumeric() || c == '\'' || c == '’')) {
        let w = fold(raw);
        if w.is_empty() || w.chars().all(|c| c.is_ascii_digit()) {
            continue;
        }
        all += 1;
        let read = match main {
            Script::Kana => segments(&w, &romaji()),
            Script::Han => {
                // Tone numbers (pinyin's, Jyutping's): a stamped syllable reads as one.
                let bare = w.trim_end_matches(|c: char| c.is_ascii_digit());
                (bare.len() < w.len() && !bare.is_empty() && bare.chars().all(|c| c.is_ascii_lowercase()))
                    || segments(&w, &pinyin())
                    || segments(&w, &jyutping())
            }
            Script::Hangul => segments(&w, &korean()),
            _ => false,
        };
        ok += usize::from(read);
    }
    (ok, all)
}

/// Lowercase ASCII, accents and tone marks taken off (ā → a, ü → v), apostrophes dropped.
fn fold(word: &str) -> String {
    word.chars()
        .filter(|c| *c != '\'' && *c != '’')
        .flat_map(char::to_lowercase)
        .map(|c| match c {
            'ā' | 'á' | 'ǎ' | 'à' | 'â' => 'a',
            'ē' | 'é' | 'ě' | 'è' | 'ê' => 'e',
            'ī' | 'í' | 'ǐ' | 'ì' | 'î' => 'i',
            'ō' | 'ó' | 'ǒ' | 'ò' | 'ô' => 'o',
            'ū' | 'ú' | 'ǔ' | 'ù' | 'û' => 'u',
            'ü' | 'ǖ' | 'ǘ' | 'ǚ' | 'ǜ' => 'v',
            c => c,
        })
        .collect()
}

/// `word` splits wholly into `syllables` (a doubled consonant, as romaji's small tsu, counts:
/// `kitte`, `matcha`).
fn segments(word: &str, syllables: &[String]) -> bool {
    let b = word.as_bytes();
    if !word.is_ascii() || b.is_empty() {
        return false;
    }
    let mut reach = vec![false; b.len() + 1];
    reach[0] = true;
    for i in 0..b.len() {
        if !reach[i] {
            continue;
        }
        for s in syllables {
            if b[i..].starts_with(s.as_bytes()) {
                reach[i + s.len()] = true;
            }
        }
        // A doubled consonant: kk, tt, ss, pp, tch.
        let doubled = b.get(i + 1).is_some_and(|&n| n == b[i] || (b[i] == b't' && n == b'c'));
        if doubled && !b"aeioun".contains(&b[i]) {
            reach[i + 1] = true;
        }
    }
    reach[b.len()]
}

fn product(onsets: &[&str], nuclei: &[&str], codas: &[&str]) -> Vec<String> {
    let mut out = Vec::new();
    for o in onsets {
        for n in nuclei {
            for c in codas {
                out.push(format!("{o}{n}{c}"));
            }
        }
    }
    out
}

/// Hepburn (and Kunrei) romaji: the kana's syllables, the moraic n.
fn romaji() -> Vec<String> {
    let onsets = [
        "", "k", "g", "s", "z", "t", "d", "n", "h", "b", "p", "m", "y", "r", "w", "f", "j", "v", "ky", "gy", "ny",
        "hy", "by", "py", "my", "ry", "sh", "ch", "ts", "jy", "zy", "dy", "ty", "sy", "kw", "gw",
    ];
    let mut s = product(&onsets, &["a", "i", "u", "e", "o"], &[""]);
    s.extend(["n", "m"].map(String::from));
    s
}

/// Hanyu pinyin's syllables (initials × finals, loosely: no table of the pairs that occur).
fn pinyin() -> Vec<String> {
    let initials = [
        "", "b", "p", "m", "f", "d", "t", "n", "l", "g", "k", "h", "j", "q", "x", "zh", "ch", "sh", "r", "z", "c", "s",
        "y", "w",
    ];
    let finals = [
        "a", "o", "e", "i", "u", "v", "ai", "ei", "ao", "ou", "an", "en", "ang", "eng", "ong", "er", "ia", "ie", "iao",
        "iu", "ian", "in", "iang", "ing", "iong", "ua", "uo", "uai", "ui", "uan", "un", "uang", "ueng", "ve", "van",
        "vn", "ue",
    ];
    product(&initials, &finals, &[""])
}

/// Jyutping's syllables (Cantonese), without their tone numbers.
fn jyutping() -> Vec<String> {
    let initials =
        ["", "b", "p", "m", "f", "d", "t", "n", "l", "g", "k", "ng", "h", "gw", "kw", "w", "z", "c", "s", "j"];
    let finals = [
        "aa", "aai", "aau", "aam", "aan", "aang", "aap", "aat", "aak", "a", "ai", "au", "am", "an", "ang", "ap", "at",
        "ak", "e", "ei", "eu", "em", "eng", "ep", "ek", "i", "iu", "im", "in", "ing", "ip", "it", "ik", "o", "oi",
        "ou", "on", "ong", "ot", "ok", "oe", "oeng", "oek", "eoi", "eon", "eot", "u", "ui", "un", "ung", "ut", "uk",
        "yu", "yun", "yut",
    ];
    let mut s = product(&initials, &finals, &[""]);
    s.extend(["m", "ng"].map(String::from));
    s
}

/// The Revised Romanization of Korean (with its common spellings of the finals).
fn korean() -> Vec<String> {
    let initials =
        ["", "g", "kk", "n", "d", "tt", "r", "l", "m", "b", "pp", "s", "ss", "j", "jj", "ch", "k", "t", "p", "h"];
    let vowels = [
        "a", "ae", "ya", "yae", "eo", "e", "yeo", "ye", "o", "wa", "wae", "oe", "yo", "u", "wo", "we", "wi", "yu",
        "eu", "ui", "i",
    ];
    product(&initials, &vowels, &["", "k", "n", "t", "l", "m", "p", "ng", "g", "r"])
}

/// The layer the extra lines at place `k` of the groups (1: the first under the line) read as:
/// a romanisation when most of their words romanise the song's script, else a translation.
fn layer_of(groups: &[Group], main: Script, k: usize) -> &'static str {
    let (mut ok, mut all) = (0, 0);
    for line in groups.iter().filter_map(|g| g.lines.get(k)) {
        if main != Script::Latin && script(line) == Some(Script::Latin) {
            let (o, a) = romanised(line, main);
            ok += o;
            all += a;
        } else if script(line).is_some() {
            // A word of another script: no romanisation.
            all += line.split_whitespace().count().max(1);
        }
    }
    if all > 0 && ok * 5 >= all * 4 {
        "roma"
    } else {
        "trans"
    }
}

/// Lays the extra lines of every group under its line as the layers they read as: the first
/// line is the song's (it carries the word stamps). A layer a group already has (a plugin's),
/// and a second extra place reading as the same layer, stay lines.
pub(super) fn sort_layers(groups: &mut [Group]) {
    let places = groups.iter().map(|g| g.lines.len()).max().unwrap_or(0);
    if places < 2 {
        return;
    }
    let mut mains: Vec<Script> = groups.iter().filter_map(|g| g.lines.first().and_then(|l| script(l))).collect();
    mains.sort_by_key(|s| *s as u8);
    let Some(main) = most(&mains) else { return };
    let mut taken = Vec::new();
    let ids: Vec<Option<&'static str>> = (1..places)
        .map(|k| {
            let id = layer_of(groups, main, k);
            (!taken.contains(&id)).then(|| {
                taken.push(id);
                id
            })
        })
        .collect();
    for g in groups.iter_mut() {
        for k in (1..g.lines.len()).rev() {
            let Some(id) = ids[k - 1] else { continue };
            if g.layers.iter().any(|(l, _)| l == id) {
                continue;
            }
            let line = g.lines.remove(k);
            if !line.trim().is_empty() {
                g.layers.push((id.to_owned(), line));
            }
        }
        // In `LAYER_ORDER`: the reading first.
        g.layers.sort_by_key(|(l, _)| super::LAYER_ORDER.iter().position(|o| o == l));
    }
}

/// The most frequent of a sorted list.
fn most(sorted: &[Script]) -> Option<Script> {
    sorted.chunk_by(|a, b| a == b).max_by_key(|run| run.len()).map(|run| run[0])
}

#[cfg(test)]
mod tests {
    use super::super::Lyrics;

    /// A duet's voices: tagged in the file, or given by a `.duet.lrc` of the same stamps.
    #[test]
    fn duet_voices_from_the_file_or_beside_it() {
        let own = Lyrics::parse("[00:01.00][v1]你好\n[00:02.00][v2]再见\n[00:03.00]我们\n").unwrap();
        assert_eq!(own.groups.iter().map(|g| g.voice).collect::<Vec<_>>(), [1, 2, 0]);
        assert_eq!(own.groups[0].lines, ["你好"], "the tag is not the line's");
        assert!(own.duet());
        let plain = Lyrics::parse("[00:01.00]你好\n[00:01.00]ni hao\n[00:02.00]再见\n").unwrap();
        assert!(!plain.duet());
        let duet = Lyrics::parse("[00:01.00][v1]你好\n[00:02.03][v2]再见\n").unwrap();
        let both = plain.with_voices(Some(&duet));
        assert_eq!(both.groups.iter().map(|g| g.voice).collect::<Vec<_>>(), [1, 2]);
        assert!(both.duet() && both.groups[0].layers.len() == 1, "its layers kept");
        // Lyrics timed 0.18 s apart (a container's): by their words; `v1000` read too.
        let early = Lyrics::parse("[00:16.16]想要一個人\n[00:18.00]放聲\n[00:20.00]聽 旋律\n").unwrap();
        let duet = Lyrics::parse("[00:16.34][v1]想要一個人\n[00:18.18][v2]放聲\n[00:20.18][v1000]聽旋律\n").unwrap();
        let v: Vec<u16> = early.with_voices(Some(&duet)).groups.iter().map(|g| g.voice).collect();
        assert_eq!(v, [1, 2, 1000]);
    }

    fn layers(text: &str) -> Vec<Vec<(String, String)>> {
        Lyrics::parse(text).unwrap().groups.into_iter().map(|g| g.layers).collect()
    }

    /// Japanese with romaji under it: the reading layer; the line's word stamps stay its own.
    #[test]
    fn romaji_under_japanese_reads_as_the_romanisation() {
        let text = "[00:01.00]<00:01.10>夜に<00:01.60>駆ける\n[00:01.00]yoru ni kakeru\n\
                    [00:03.00]<00:03.10>きっと<00:03.50>大丈夫\n[00:03.00]kitto daijoubu\n";
        let l = Lyrics::parse(text).unwrap();
        assert_eq!(l.groups[0].lines, ["夜に駆ける"]);
        assert_eq!(l.groups[0].layers, [("roma".to_owned(), "yoru ni kakeru".to_owned())]);
        assert!(l.word_timed());
    }

    /// Traditional Chinese, then simplified, then pinyin (syllables run together or not): the
    /// simplified line is the translation, the pinyin the reading.
    #[test]
    fn simplified_and_pinyin_under_traditional() {
        let text = "[00:07.16]<00:07.98>曾許<00:08.57>下心願\n[00:07.16]曾许下心愿\n[00:07.16]ceng xuxiaxinyuan\n\
                    [00:14.26]褪色的鞦韆 Oh\n[00:14.26]褪色的秋千 Oh\n[00:14.26]tui se de qiu qian Oh\n";
        let got = layers(text);
        assert_eq!(got[0], [("roma".into(), "ceng xuxiaxinyuan".into()), ("trans".into(), "曾许下心愿".into())]);
        assert_eq!(got[1][0].0, "roma", "one word that is no pinyin does not turn the place: {:?}", got[1]);
    }

    /// Cantonese with its Jyutping, without tone numbers: the reading.
    #[test]
    fn toneless_jyutping_under_cantonese() {
        let text =
            "[00:16.34]想要一個人聽海風\n[00:16.34]想要一个人听海风\n[00:16.34]soeng jiu jat go jan ting hoi fung\n\
                    [00:20.28]帶著那份悸動\n[00:20.28]带着那份悸动\n[00:20.28]daai zoek naa fan gwai dung\n";
        let got = layers(text);
        assert_eq!(
            got[0],
            [("roma".into(), "soeng jiu jat go jan ting hoi fung".into()), ("trans".into(), "想要一个人听海风".into())]
        );
    }

    /// An English song with Chinese under it: a translation. English under Japanese, too.
    #[test]
    fn another_language_under_the_line_is_the_translation() {
        assert_eq!(layers("[00:01.00]Once upon a time\n[00:01.00]很久很久以前\n")[0][0].0, "trans");
        let text =
            "[00:01.00]夜に駆ける\n[00:01.00]Racing into the night\n[00:02.00]きっと\n[00:02.00]Surely it will\n";
        assert_eq!(layers(text)[0][0].0, "trans");
    }

    /// One line a stamp: nothing is a layer.
    #[test]
    fn single_lines_stay_lines() {
        let l = Lyrics::parse("[00:01.00]a\n[00:02.00]b\n").unwrap();
        assert!(l.groups.iter().all(|g| g.layers.is_empty() && g.lines.len() == 1));
    }
}
