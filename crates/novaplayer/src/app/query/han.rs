//! The readings of CJK ideographs, from the Unihan database (`tools/readings.py` writes
//! `readings.z`): Mandarin pinyin, Cantonese jyutping and the Japanese readings, without tones.
//! Read once, the first time a search needs them (`warm` reads them ahead, off the UI thread).

use std::sync::OnceLock;

static DATA: &[u8] = include_bytes!("readings.z");

/// The planes the table covers (`tools/readings.py` `HAN`), laid end to end by `slot`.
const RANGES: [(u32, u32); 3] = [(0x3400, 0x4DBF), (0x4E00, 0x9FFF), (0xF900, 0xFAFF)];

/// Where `c` sits in the ranges laid end to end; `None` outside them.
fn slot(c: char) -> Option<usize> {
    let c = c as u32;
    let mut base = 0;
    for (lo, hi) in RANGES {
        if (lo..=hi).contains(&c) {
            return Some((base + c - lo) as usize);
        }
        base += hi - lo + 1;
    }
    None
}

/// Every character's readings, found by its `slot` (`entry`: the entry's index + 1, 0 for none).
/// Entry `k`'s readings are `ids[first[k]..first[k + 1]]`, each an index into `ends`, the end of
/// that reading in `pool`; `starts[k]` is what they start with (`fold::start`) and `letters[k]`
/// the letters of the first (`fold::letters`), so a search learns them without spelling them out.
pub(super) struct Table {
    entry: Vec<u32>,
    first: Vec<u32>,
    ids: Vec<u32>,
    starts: Vec<u32>,
    letters: Vec<u64>,
    pool: String,
    ends: Vec<u32>,
}

static TABLE: OnceLock<Table> = OnceLock::new();

/// The table, read on first use.
pub(super) fn table() -> &'static Table {
    TABLE.get_or_init(load)
}

/// Reads the table on a thread of its own, so a search about to be typed on the UI thread (a
/// dropdown's box taking the keyboard) does not wait for it. Once; nothing after it is read.
pub fn warm() {
    static ASKED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    if TABLE.get().is_some() || ASKED.swap(true, std::sync::atomic::Ordering::Relaxed) {
        return;
    }
    let spawned = std::thread::Builder::new().name("novaplayer-readings".into()).spawn(|| {
        table();
    });
    if let Err(e) = spawned {
        eprintln!("novaplayer: search readings not read ahead: {e}");
    }
}

fn load() -> Table {
    let size = RANGES.iter().map(|(lo, hi)| (hi - lo + 1) as usize).sum();
    let mut t = Table {
        entry: vec![0; size],
        first: vec![0],
        ids: Vec::new(),
        starts: Vec::new(),
        letters: Vec::new(),
        pool: String::new(),
        ends: Vec::new(),
    };
    let text = match miniz_oxide::inflate::decompress_to_vec_zlib(DATA) {
        Ok(bytes) => String::from_utf8(bytes).unwrap_or_default(),
        Err(e) => {
            eprintln!("novaplayer: search readings unreadable: {e:?}");
            String::new()
        }
    };
    let mut known: std::collections::HashMap<String, u32> = std::collections::HashMap::new();
    let mut own: Vec<u32> = Vec::new();
    for line in text.lines() {
        let mut chars = line.chars();
        let Some(at) = chars.next().and_then(slot) else { continue };
        own.clear();
        for r in chars.as_str().split(' ').filter(|r| !r.is_empty()) {
            let mut add = |s: String| {
                let id = *known.entry(s).or_insert_with_key(|s| {
                    t.pool.push_str(s);
                    t.ends.push(t.pool.len() as u32);
                    t.ends.len() as u32 - 1
                });
                if !own.contains(&id) {
                    own.push(id);
                }
            };
            if r.starts_with(super::fold::is_kana) {
                let kana: Vec<char> = r.chars().collect();
                for kunrei in [false, true] {
                    let spelt = super::kana::romaji(&kana, kunrei);
                    let short = spelt.replace("ou", "o").replace("uu", "u").replace("oo", "o");
                    add(spelt);
                    add(short);
                }
            } else {
                add(r.to_owned());
            }
        }
        t.entry[at] = t.first.len() as u32;
        let starts = own.iter().fold(0, |m, &id| m | t.reading(id).chars().next().map_or(0, super::fold::start));
        let letters = own.first().map_or(0, |&id| super::fold::letters(t.reading(id).chars()));
        t.starts.push(starts);
        t.letters.push(letters);
        t.ids.extend_from_slice(&own);
        t.first.push(t.ids.len() as u32);
    }
    t
}

impl Table {
    /// The entry of `c`, if it has readings.
    fn of(&self, c: char) -> Option<usize> {
        slot(c).and_then(|s| self.entry[s].checked_sub(1)).map(|k| k as usize)
    }

    fn reading(&self, id: u32) -> &str {
        let start = id.checked_sub(1).map_or(0, |p| self.ends[p as usize]) as usize;
        &self.pool[start..self.ends[id as usize] as usize]
    }

    fn ids(&self, k: usize) -> &[u32] {
        &self.ids[self.first[k] as usize..self.first[k + 1] as usize]
    }

    /// Calls `f` with each reading of `c` until it returns true (then true).
    pub fn readings(&self, c: char, f: &mut dyn FnMut(&str) -> bool) -> bool {
        self.of(c).is_some_and(|k| self.ids(k).iter().any(|&id| f(self.reading(id))))
    }

    /// Its first reading (Mandarin where it has one).
    pub fn first(&self, c: char) -> Option<&str> {
        self.of(c).and_then(|k| self.ids(k).first()).map(|&id| self.reading(id))
    }

    /// What its readings start with, and the letters of the first (none: no readings).
    pub fn masks(&self, c: char) -> (u32, u64) {
        self.of(c).map_or((0, 0), |k| (self.starts[k], self.letters[k]))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn all(c: char) -> Vec<String> {
        let mut v = Vec::new();
        table().readings(c, &mut |s| {
            v.push(s.to_owned());
            false
        });
        v
    }

    #[test]
    fn an_ideograph_has_its_chinese_and_japanese_readings() {
        let zhong = all('重');
        for r in ["zhong", "chong", "cung", "juu", "ju", "chou", "cho", "omoi", "kasaneru"] {
            assert!(zhong.contains(&r.to_owned()), "{r} in {zhong:?}");
        }
        let lv = all('绿');
        assert!(lv.contains(&"lv".to_owned()) && lv.contains(&"lu".to_owned()), "{lv:?}");
        let east = all('東');
        assert!(
            east.contains(&"tou".to_owned()) && east.contains(&"to".to_owned()) && east.contains(&"higashi".to_owned())
        );
        assert!(all('a').is_empty());
    }
}
