//! The NetEase Cloud Music web API over `weapi` (endpoints as NeteaseCloudMusicApi 4.32.0's
//! modules call them; `util/request.js` for headers and cookies).

use crate::quality::{quality_label, DEFAULT_LEVEL, LEVELS};
use kit::http::Http;
use kit::json::{self, Value};
use kit::responses::{Quality, UserList};
use kit::util;
use std::collections::BTreeMap;

const UA: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36 Edg/124.0.0.0";

pub struct Client {
    http: Http,
    /// The anonymous MUSIC_A cookie (fetched lazily; login-less calls need it).
    anonymous: Option<String>,
    anonymous_attempted: bool,
    device_id: String,
}

/// A POST's answer: the body JSON and the cookies it set (the login check's MUSIC_U…).
pub struct Answer {
    pub body: Value,
    pub cookies: Vec<(String, String)>,
}

pub struct QrStatus {
    pub code: u64,
    pub cookies: Vec<(String, String)>,
    pub message: Option<String>,
}

impl Client {
    pub fn new() -> Self {
        Client {
            http: Http::new(std::time::Duration::from_secs(10)),
            anonymous: None,
            anonymous_attempted: false,
            device_id: (0..52).map(|_| b"0123456789ABCDEF"[rand_index()]).map(char::from).collect(),
        }
    }

    pub fn begin_call(&mut self, call: &str) {
        self.http.begin_call(call);
        // A transient anonymous-registration failure may retry on the next host call,
        // never on every fallback/batch inside this one.
        self.anonymous_attempted = self.anonymous.is_some();
    }

    /// The token is the cookie string the sign-in returned ("MUSIC_U=…; __csrf=…").
    fn token_cookies(token: Option<&str>) -> Vec<(String, String)> {
        let mut out: Vec<(String, String)> = token
            .unwrap_or("")
            .split(';')
            .filter_map(|p| p.trim().split_once('='))
            .map(|(k, v)| (k.trim().to_string(), v.trim().to_string()))
            .filter(|(k, _)| !k.is_empty())
            .collect();
        out.push(("__remember_me".into(), "true".into()));
        out.push(("_ntes_nuid".into(), util::rand_hex(16)));
        out.push(("os".into(), "pc".into()));
        out.push(("appver".into(), "3.1.17.204416".into()));
        out.push(("osver".into(), "Microsoft-Windows-10-Professional-build-19045-64bit".into()));
        out.push(("channel".into(), "netease".into()));
        out
    }

    /// POST /weapi/<path> with `data` (csrf_token added); login paths omit NMTID.
    fn weapi(&self, path: &str, mut data: Value, cookies: Vec<(String, String)>) -> Result<Answer, String> {
        let csrf = cookies.iter().find(|(k, _)| k == "__csrf").map(|(_, v)| v.clone()).unwrap_or_default();
        if let Value::Obj(m) = &mut data {
            m.insert("csrf_token".into(), Value::str(csrf.clone()));
        }
        let mut cookies = cookies;
        if !cookies.iter().any(|(k, _)| k == "MUSIC_U") {
            if let Some(a) = &self.anonymous {
                cookies.push(("MUSIC_A".into(), a.clone()));
            }
        }
        if !path.contains("login") {
            cookies.push(("NMTID".into(), util::rand_hex(16)));
        }
        let (params, enc_sec_key) = crate::crypto::weapi(&json::write(&data));
        let url = format!("https://music.163.com/weapi/{path}?csrf_token={}", util::urlencode(&csrf));
        let res = self.http.post_form(
            &url,
            &util::form(&[("params", &params), ("encSecKey", &enc_sec_key)]),
            &[("Referer", "https://music.163.com"), ("User-Agent", UA)],
            &cookies.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect::<Vec<_>>(),
            false,
        )?;
        Ok(Answer { body: res.json()?, cookies: res.cookies })
    }

    /// Calls with the account's cookies (or anonymous ones).
    pub(super) fn call(&mut self, path: &str, data: Value, token: Option<&str>) -> Result<Answer, String> {
        let cookies = Self::token_cookies(token);
        if token.is_none() && !self.anonymous_attempted {
            self.anonymous_attempted = true;
            // No account: register an anonymous device first (its MUSIC_A unlocks guest calls).
            let username = crate::crypto::anonymous_username(&self.device_id);
            let mut m = BTreeMap::new();
            m.insert("username".into(), Value::str(username));
            if let Ok(a) = self.weapi("register/anonimous", Value::Obj(m), Vec::new()) {
                self.anonymous = a.cookies.iter().find(|(k, _)| k == "MUSIC_A").map(|(_, v)| v.clone());
            }
        }
        self.weapi(path, data, cookies)
    }

    // ---- calls the protocol needs ------------------------------------------------------

    /// `signin` (qr): a unikey; the QR encodes the login page URL carrying it.
    pub fn qr_unikey(&mut self) -> Result<String, String> {
        let a = self.call("login/qrcode/unikey", json::obj(vec![("type", Value::Num(3.0))]), None)?;
        a.body
            .get("unikey")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .ok_or_else(|| code_err(&a.body, kit::pipe::tr("没有拿到 unikey", "No unikey returned")))
    }

    /// `signin_poll`: the QR's state; on 803 the cookies hold the account.
    pub fn qr_check(&mut self, key: &str) -> Result<QrStatus, String> {
        let a = self.call(
            "login/qrcode/client/login",
            json::obj(vec![("key", Value::str(key)), ("type", Value::Num(3.0))]),
            None,
        )?;
        let code = a.body.get("code").and_then(Value::as_u64).unwrap_or(0);
        let msg = a.body.get("message").and_then(Value::as_str).map(str::to_owned);
        Ok(QrStatus { code, cookies: a.cookies, message: msg })
    }

    /// The account's (uid, nickname) for the token.
    pub fn account(&mut self, token: &str) -> Result<(u64, String), String> {
        let a = self.call("nuser/account/get", Value::Obj(BTreeMap::new()), Some(token))?;
        let uid = a
            .body
            .at(&["account", "id"])
            .and_then(Value::as_u64)
            .ok_or_else(|| code_err(&a.body, kit::pipe::tr("登录已失效", "The sign-in has expired")))?;
        let name = a
            .body
            .at(&["profile", "nickname"])
            .and_then(Value::as_str)
            .unwrap_or(kit::pipe::tr("网易云用户", "NetEase user"))
            .to_owned();
        Ok((uid, name))
    }

    /// The account's own playlists (user/playlist; the first is 我喜欢的音乐).
    pub fn user_playlists(&mut self, token: &str) -> Result<Vec<UserList>, String> {
        let (uid, _) = self.account(token)?;
        let a = self.call(
            "user/playlist",
            json::obj(vec![
                ("uid", Value::Num(uid as f64)),
                ("limit", Value::Num(1000.0)),
                ("offset", Value::Num(0.0)),
                ("includeVideo", Value::Bool(true)),
            ]),
            Some(token),
        )?;
        let Some(list) = a.body.get("playlist").and_then(Value::as_array) else {
            return Err(code_err(&a.body, kit::pipe::tr("没有返回歌单", "No playlist returned")));
        };
        Ok(list
            .iter()
            .filter_map(|p| {
                let id = p.get("id").and_then(Value::as_u64)?;
                let name =
                    p.get("name").and_then(Value::as_str).unwrap_or(kit::pipe::tr("歌单", "Playlist")).to_owned();
                let count = p.get("trackCount").and_then(Value::as_u64);
                let group = if p.get("subscribed").and_then(Value::as_bool).unwrap_or(false) {
                    kit::pipe::tr("我收藏的", "Saved")
                } else {
                    kit::pipe::tr("我创建的", "Created")
                };
                Some(UserList { url: format!("ncm:playlist:{id}"), name, count, group })
            })
            .collect())
    }

    /// A playlist's songs (v6/playlist/detail): (id, title, artist, album, seconds).
    pub fn playlist(&mut self, id: u64, token: Option<&str>) -> Result<(String, Vec<Song>), String> {
        let a = self.call(
            "v6/playlist/detail",
            json::obj(vec![("id", Value::Num(id as f64)), ("n", Value::Num(100000.0)), ("s", Value::Num(8.0))]),
            token,
        )?;
        let pl = a
            .body
            .get("playlist")
            .ok_or_else(|| code_err(&a.body, kit::pipe::tr("没有返回歌单", "No playlist returned")))?;
        let name = pl
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or(kit::pipe::tr("网易云歌单", "NetEase playlist"))
            .to_owned();
        // `tracks` stops at 1000 songs; `trackIds` lists them all. The rest are read from
        // song/detail in batches (the list's words only: the cover and lyrics come at play).
        let mut known: std::collections::HashMap<u64, Song> = std::collections::HashMap::new();
        for t in pl.get("tracks").and_then(Value::as_array).into_iter().flatten() {
            if let Some(s) = song_of(t) {
                known.insert(s.id, s);
            }
        }
        let ids: Vec<u64> = pl
            .get("trackIds")
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(|t| t.get("id").and_then(Value::as_u64)).collect())
            .unwrap_or_else(|| known.keys().copied().collect());
        if ids.len() > 100_000 {
            return Err(kit::pipe::tr("歌单超过 100000 首上限", "The playlist has more than 100000 songs").into());
        }
        let missing: Vec<u64> = ids.iter().copied().filter(|id| !known.contains_key(id)).collect();
        for batch in missing.chunks(500) {
            let c = batch.iter().map(|id| format!("{{\"id\":{id}}}")).collect::<Vec<_>>().join(",");
            // Bounded retries share the playlist deadline. A network failure must not be
            // presented as songs withheld by the service.
            let mut failure = Some(kit::pipe::tr("没有返回歌曲详情", "No song details returned").to_string());
            for attempt in 0..3 {
                self.http.remaining()?;
                if attempt > 0 {
                    std::thread::sleep(std::time::Duration::from_millis(600 * attempt));
                }
                let a = match self.call("v3/song/detail", json::obj(vec![("c", Value::str(format!("[{c}]")))]), token) {
                    Ok(a) => a,
                    Err(e) => {
                        failure = Some(e);
                        continue;
                    }
                };
                let Some(list) = a.body.get("songs").and_then(Value::as_array) else { continue };
                for t in list {
                    if let Some(s) = song_of(t) {
                        known.insert(s.id, s);
                    }
                }
                failure = None;
                break;
            }
            if let Some(e) = failure {
                return Err(e);
            }
        }
        // The list's own order (trackIds). A song song/detail never named stays as a bare id:
        // the host lists it as unavailable.
        let songs = ids
            .iter()
            .map(|id| {
                known.remove(id).unwrap_or(Song {
                    id: *id,
                    title: String::new(),
                    artist: String::new(),
                    album: String::new(),
                    duration: None,
                })
            })
            .collect();
        Ok((name, songs))
    }

    /// The playable address at the highest level the account may use, up to `cap`
    /// (song_url_v1.js). The service itself steps down to what the account may play and says
    /// which level it gave; an empty answer steps down one level. → (url, level).
    pub fn stream_url(&mut self, id: u64, cap: &str, token: Option<&str>) -> Result<(String, String), String> {
        let from = LEVELS.iter().position(|l| l.0 == cap).unwrap_or(DEFAULT_LEVEL);
        for (level, ..) in &LEVELS[from..] {
            let data = json::obj(vec![
                ("ids", Value::str(format!("[{id}]"))),
                ("level", Value::str(*level)),
                ("encodeType", Value::str("flac")),
            ]);
            let a = self.call("song/enhance/player/url/v1", data, token)?;
            let d = a.body.at(&["data"]).and_then(Value::as_array).and_then(|d| d.first()).cloned();
            let url = d.as_ref().and_then(|d| d.get("url")).and_then(Value::as_str).filter(|u| !u.is_empty());
            // A song the account may only try gets a clip of it (freeTrialInfo: where the clip
            // starts and ends), at every level: not the song, so it does not play at all.
            if url.is_some() && d.as_ref().is_some_and(is_trial) {
                return Err(kit::pipe::tr(
                    "该歌曲需要会员才能完整播放，仅提供试听片段",
                    "This song needs a membership to play in full; only a preview clip is offered",
                )
                .into());
            }
            if let Some(u) = url {
                let got = d.as_ref().and_then(|d| d.get("level")).and_then(Value::as_str).unwrap_or(level).to_owned();
                return Ok((https(u), got));
            }
        }
        Err(kit::pipe::tr(
            "该歌曲暂时无法播放（可能需要会员或已下架）",
            "This song can't play now (it may need a membership or was taken down)",
        )
        .into())
    }

    /// The levels this song exists in (song/music/detail/get), highest first:
    /// (level, name, "96 kHz / 24-bit FLAC").
    pub fn qualities(&mut self, id: u64, token: Option<&str>) -> Result<Vec<Quality>, String> {
        let a = self.call("song/music/detail/get", json::obj(vec![("songId", Value::str(id.to_string()))]), token)?;
        let d = a
            .body
            .get("data")
            .ok_or_else(|| code_err(&a.body, kit::pipe::tr("没有返回音质", "No qualities returned")))?;
        let mut out = Vec::new();
        for (level, zh, en, key) in LEVELS {
            let Some(q) = d.get(key).filter(|q| !matches!(q, Value::Null)) else { continue };
            let sr = q.get("sr").and_then(Value::as_f64).unwrap_or(0.0) as u32;
            let br = q.get("br").and_then(Value::as_f64).unwrap_or(0.0) as u32;
            out.push(Quality {
                id: level.to_string(),
                name: kit::pipe::tr(zh, en).to_string(),
                label: quality_label(level, sr, br),
            });
        }
        Ok(out)
    }

    pub fn logout(&mut self, token: &str) {
        let _ = self.call("logout", Value::Obj(BTreeMap::new()), Some(token));
    }
}

/// A playlist's song, for the host's list.
pub struct Song {
    pub id: u64,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub duration: Option<f64>,
}

/// A song of playlist/detail's `tracks` or song/detail's `songs` (the same shape).
fn song_of(t: &Value) -> Option<Song> {
    let id = t.get("id").and_then(Value::as_u64)?;
    let artist = t
        .get("ar")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(|x| x.get("name").and_then(Value::as_str)).collect::<Vec<_>>().join("/"))
        .unwrap_or_default();
    Some(Song {
        id,
        title: t.get("name").and_then(Value::as_str).unwrap_or("").to_owned(),
        artist,
        album: t.at(&["al", "name"]).and_then(Value::as_str).unwrap_or("").to_owned(),
        duration: t.get("dt").and_then(Value::as_f64).map(|ms| ms / 1000.0),
    })
}

/// A song_url_v1 entry that is only a preview clip (`freeTrialInfo` not null).
fn is_trial(d: &Value) -> bool {
    d.get("freeTrialInfo").is_some_and(|t| !matches!(t, Value::Null))
}

fn code_err(body: &Value, fallback: &str) -> String {
    let code = body.get("code").and_then(Value::as_f64).map(|c| c as i64);
    let msg = body.get("message").or_else(|| body.get("msg")).and_then(Value::as_str);
    match (code, msg) {
        (_, Some(m)) if !m.is_empty() => m.to_owned(),
        (Some(c), _) => format!("{fallback}{}{c}{}", kit::pipe::tr("（", " ("), kit::pipe::tr("）", ")")),
        _ => fallback.to_owned(),
    }
}

/// A random index 0..16 (for the device id's uppercase hex).
fn rand_index() -> usize {
    util::rand_bytes(1)[0] as usize % 16
}

/// NetEase's addresses go over https.
pub fn https(u: &str) -> String {
    u.replace("http://", "https://")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_preview_clip_is_told_from_the_song() {
        let entry =
            |trial: &str| json::parse(&format!(r#"{{"url":"http://x/a.mp3","freeTrialInfo":{trial}}}"#)).unwrap();
        assert!(is_trial(&entry(r#"{"start":30,"end":60}"#)));
        assert!(!is_trial(&entry("null")));
        assert!(!is_trial(&json::parse(r#"{"url":"http://x/a.mp3"}"#).unwrap()));
    }
}
