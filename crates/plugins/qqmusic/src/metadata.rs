//! QQ metadata, lyrics and small album covers.
use crate::api::{Client, Credential};
use kit::json::{self, Value};

impl Client {
    /// Online metadata: words, cover, lyrics (get_song_detail_yqq + GetPlayLyricInfo; the
    /// lyric is 3DES-encrypted hex, crypto.rs).
    /// Small covers for many songs (music.trackInfo.UniformRuleCtrl/CgiGetTrackInfo, 100 a
    /// request): mid → 150 px album cover URL.
    pub fn thumbs(&self, mids: &[String], cred: Option<&Credential>) -> Vec<(String, String)> {
        let mut out = Vec::new();
        for batch in mids.chunks(100) {
            let n = batch.len();
            let param = json::obj(vec![
                ("ctx", Value::Num(0.0)),
                ("client", Value::Num(1.0)),
                ("types", Value::Arr(vec![Value::Num(0.0); n])),
                ("modify_stamp", Value::Arr(vec![Value::Num(0.0); n])),
                ("mids", Value::Arr(batch.iter().map(|m| Value::str(m.clone())).collect())),
            ]);
            let Ok(data) = self.musicu("music.trackInfo.UniformRuleCtrl", "CgiGetTrackInfo", param, cred, &[]) else {
                continue;
            };
            for t in data.get("tracks").and_then(Value::as_array).into_iter().flatten() {
                let mid = t.get("mid").and_then(Value::as_str).unwrap_or_default();
                let album = t.at(&["album", "mid"]).and_then(Value::as_str).unwrap_or_default();
                if !mid.is_empty() && !album.is_empty() {
                    out.push((
                        mid.to_owned(),
                        format!("https://y.gtimg.cn/music/photo_new/T002R150x150M000{album}.jpg"),
                    ));
                }
            }
        }
        out
    }

    pub fn metadata(&self, mid: &str, cred: Option<&Credential>) -> Vec<(String, Value)> {
        let mut out: Vec<(String, Value)> = Vec::new();
        if let Ok(data) = self.musicu(
            "music.pf_song_detail_svr",
            "get_song_detail_yqq",
            json::obj(vec![("song_mid", Value::str(mid))]),
            cred,
            &[],
        ) {
            let t = data.get("track_info").cloned().unwrap_or(Value::Null);
            let artist = t
                .get("singer")
                .and_then(Value::as_array)
                .map(|a| a.iter().filter_map(|x| x.get("name").and_then(Value::as_str)).collect::<Vec<_>>().join("/"))
                .unwrap_or_default();
            let cover = t
                .at(&["album", "mid"])
                .or_else(|| t.at(&["album", "pmid"]))
                .and_then(Value::as_str)
                .filter(|m| !m.is_empty())
                .map(|m| format!("https://y.gtimg.cn/music/photo_new/T002R800x800M000{m}.jpg"));
            for (k, v) in [
                ("title", t.get("title").or_else(|| t.get("name")).and_then(Value::as_str).map(str::to_owned)),
                ("artist", Some(artist).filter(|a| !a.is_empty())),
                ("album", t.at(&["album", "name"]).and_then(Value::as_str).map(str::to_owned)),
                ("date", t.get("time_public").and_then(Value::as_str).map(str::to_owned)),
                ("cover", cover),
            ] {
                if let Some(v) = v {
                    out.push((k.into(), Value::str(v)));
                }
            }
        }
        // The legacy fcg gives plain LRC even anonymously (nobase64=1); the musicu endpoint
        // (3DES-encrypted, crypto.rs) is the fallback when the legacy one has nothing.
        let legacy = self.http.get(
            &format!("https://c.y.qq.com/lyric/fcgi-bin/fcg_query_lyric_new.fcg?songmid={mid}&format=json&nobase64=1&g_tk=5381&loginUin=0&hostUin=0&inCharset=utf8&outCharset=utf-8&notice=0&platform=yqq.json&needNewCode=0"),
            &[("Referer", "https://y.qq.com/")],
            &[],
            true,
        );
        let legacy = legacy.ok().and_then(|r| r.json().ok());
        let mut lyric = legacy
            .as_ref()
            .and_then(|v| v.get("lyric").and_then(Value::as_str).map(str::to_owned))
            .filter(|l| !l.trim().is_empty());
        // The translation, an LRC of the same stamps (songs in another language).
        let mut trans = legacy
            .as_ref()
            .and_then(|v| v.get("trans").and_then(Value::as_str).map(str::to_owned))
            .filter(|t| kit::layers::is_lrc(t));
        if lyric.is_none() {
            if let Ok(data) = self.musicu(
                "music.musichallSong.PlayLyricInfo",
                "GetPlayLyricInfo",
                json::obj(vec![
                    ("songMid", Value::str(mid)),
                    ("type", Value::Num(1.0)),
                    ("crypt", Value::Num(1.0)),
                    ("lrc_t", Value::Num(0.0)),
                    ("qrc", Value::Num(0.0)),
                    ("qrc_t", Value::Num(0.0)),
                    ("roma", Value::Num(0.0)),
                    ("roma_t", Value::Num(0.0)),
                    ("trans", Value::Num(1.0)),
                    ("trans_t", Value::Num(0.0)),
                    ("needSingingAnnotations", Value::Num(0.0)),
                ]),
                cred,
                &[],
            ) {
                lyric = data
                    .get("lyric")
                    .and_then(Value::as_str)
                    .filter(|l| !l.is_empty())
                    .and_then(crate::crypto::qrc_decrypt);
                if trans.is_none() {
                    trans = data
                        .get("trans")
                        .and_then(Value::as_str)
                        .filter(|t| !t.is_empty())
                        .and_then(crate::crypto::qrc_decrypt)
                        .filter(|t| kit::layers::is_lrc(t));
                }
            }
        }
        if let Some(l) = lyric {
            out.push(("lyrics".into(), Value::str(l)));
            if let Some(t) = trans {
                out.push(("layers".into(), kit::layers::translation(&t)));
            }
        }
        out
    }
}
