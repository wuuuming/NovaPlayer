//! NetEase metadata, lyrics and sized album covers.
use crate::api::Client;
use kit::json::{self, Value};

impl Client {
    /// Small covers for many songs (v3/song/detail, up to 500 a request): id → 120 px cover URL.
    pub fn thumbs(&mut self, ids: &[u64], token: Option<&str>) -> Vec<(u64, String)> {
        let mut out = Vec::new();
        for batch in ids.chunks(500) {
            let c = batch.iter().map(|id| format!("{{\"id\":{id}}}")).collect::<Vec<_>>().join(",");
            let Ok(a) = self.call("v3/song/detail", json::obj(vec![("c", Value::str(format!("[{c}]")))]), token) else {
                continue;
            };
            for s in a.body.get("songs").and_then(Value::as_array).into_iter().flatten() {
                let (Some(id), Some(u)) =
                    (s.get("id").and_then(Value::as_u64), s.at(&["al", "picUrl"]).and_then(Value::as_str))
                else {
                    continue;
                };
                if !u.is_empty() {
                    out.push((id, format!("{}?param=120y120", crate::api::https(u).split('?').next().unwrap_or(u))));
                }
            }
        }
        out
    }

    /// Online metadata: the song's words, its cover, its lyrics (v3/song/detail, song/lyric/v1).
    pub fn metadata(&mut self, id: u64, token: Option<&str>) -> Vec<(String, Value)> {
        let mut out: Vec<(String, Value)> = Vec::new();
        let detail =
            json::obj(vec![("c", Value::str(format!("[{{\"id\":{id}}}]"))), ("ids", Value::str(format!("[{id}]")))]);
        if let Ok(a) = self.call("v3/song/detail", detail, token) {
            if let Some(s) = a.body.get("songs").and_then(Value::as_array).and_then(|s| s.first()) {
                let artist = s
                    .get("ar")
                    .and_then(Value::as_array)
                    .map(|a| {
                        a.iter().filter_map(|x| x.get("name").and_then(Value::as_str)).collect::<Vec<_>>().join("/")
                    })
                    .unwrap_or_default();
                for (k, v) in [
                    ("title", s.get("name").and_then(Value::as_str).map(str::to_owned)),
                    ("artist", Some(artist).filter(|a| !a.is_empty())),
                    ("album", s.at(&["al", "name"]).and_then(Value::as_str).map(str::to_owned)),
                    ("cover", s.at(&["al", "picUrl"]).and_then(Value::as_str).map(cover_url)),
                    (
                        "date",
                        s.get("publishTime").and_then(Value::as_f64).map(|ms| {
                            let days = (ms / 1000.0 / 86400.0) as i64;
                            util_date(days)
                        }),
                    ),
                ] {
                    if let Some(v) = v {
                        out.push((k.into(), Value::str(v)));
                    }
                }
            }
        }
        // lyric.js (the classic endpoint): plain LRC text, not lyric_new's JSON lines.
        let lyric = json::obj(vec![
            ("id", Value::Num(id as f64)),
            ("tv", Value::Num(-1.0)),
            ("lv", Value::Num(-1.0)),
            ("rv", Value::Num(-1.0)),
            ("kv", Value::Num(-1.0)),
            ("_nmclfl", Value::Num(1.0)),
        ]);
        if let Ok(a) = self.call("song/lyric", lyric, token) {
            let empty = a.body.get("nolyric").and_then(Value::as_bool).unwrap_or(false)
                || a.body.get("uncollected").and_then(Value::as_bool).unwrap_or(false);
            if !empty {
                if let Some(l) = a.body.at(&["lrc", "lyric"]).and_then(Value::as_str).filter(|l| !l.trim().is_empty()) {
                    out.push(("lyrics".into(), Value::str(l)));
                    // The translation (tv = -1) and the romanisation (rv = -1): LRCs of the same
                    // stamps.
                    let layer =
                        |k: &str| a.body.at(&[k, "lyric"]).and_then(Value::as_str).filter(|t| kit::layers::is_lrc(t));
                    let (trans, roma) = (layer("tlyric"), layer("romalrc"));
                    if trans.is_some() || roma.is_some() {
                        out.push(("layers".into(), kit::layers::layers(trans, roma)));
                    }
                }
            }
        }
        out
    }
}

/// The cover at the size a player draws. NetEase's image host serves the album master (often a
/// 3000 × 3000 PNG of 12 MB) unless asked: `param=1024y1024` gives a 1024 px one.
fn cover_url(u: &str) -> String {
    let u = crate::api::https(u);
    if u.contains("param=") {
        return u;
    }
    let sep = if u.contains('?') { '&' } else { '?' };
    format!("{u}{sep}param=1024y1024")
}

/// Days since 1970-01-01 → "YYYY-MM-DD".
fn util_date(days: i64) -> String {
    // Howard Hinnant's civil-from-days.
    let z = days + 719468;
    let era = z.div_euclid(146097);
    let doe = z.rem_euclid(146097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{y:04}-{m:02}-{d:02}")
}

#[cfg(test)]
mod cover_tests {
    #[test]
    fn covers_are_asked_at_1024() {
        assert_eq!(
            super::cover_url("http://p4.music.126.net/a==/1.jpg"),
            "https://p4.music.126.net/a==/1.jpg?param=1024y1024"
        );
        assert_eq!(
            super::cover_url("https://p4.music.126.net/a==/1.jpg?param=300y300"),
            "https://p4.music.126.net/a==/1.jpg?param=300y300"
        );
    }
}
