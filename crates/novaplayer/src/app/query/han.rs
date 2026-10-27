//! The readings of CJK ideographs, from the Unihan database (`tools/readings.py` writes
//! `readings.z`): Mandarin pinyin, Cantonese jyutping and the Japanese readings, without tones.
//! Read once, the first time a search needs them (`warm` reads them ahead, off the UI thread).

use std::sync::OnceLock;

static DATA: &[u8] = include_bytes!("readings.z");

/// Every character's readings: `keys` sorted; character `k`'s readings are `ids[first[k]..
/// first[k + 1]]`, each an index into `ends`, the end of that reading in `pool`.
pub(super) struct Table {
    keys: Vec<char>,
    first: Vec<u32>,
    ids: Vec<u32>,
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
    let mut t = Table { keys: Vec::new(), first: vec![0], ids: Vec::new(), pool: String::new(), ends: Vec::new() };
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
        let Some(key) = chars.next() else { continue };
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
        t.keys.push(key);
        t.ids.extend_from_slice(&own);
        t.first.push(t.ids.len() as u32);
    }
    t
}

impl Table {
    /// Calls `f` with each reading of `c` until it returns true (then true).
    pub fn readings(&self, c: char, f: &mut dyn FnMut(&str) -> bool) -> bool {
        let Ok(k) = self.keys.binary_search(&c) else { return false };
        let ids = &self.ids[self.first[k] as usize..self.first[k + 1] as usize];
        ids.iter().any(|&id| {
            let start = id.checked_sub(1).map_or(0, |p| self.ends[p as usize]) as usize;
            f(&self.pool[start..self.ends[id as usize] as usize])
        })
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
