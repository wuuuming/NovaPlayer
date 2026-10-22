//! The QQ Music plugin (docs/plugins.md): QR sign-in, the account's own playlists, playlist
//! import from links, highest-quality stream addresses, online metadata.
//!
//! Flow and endpoints follow L-1124/QQMusicApi (modules/login.py, user.py, songlist.py,
//! song.py, lyric.py); the flow reference product is folia-major.

mod api;
mod credential;
mod crypto;
mod login;
mod metadata;
mod session;

use api::Client;
use kit::json::{self, Value};
use kit::pipe::{self};

/// (disstid, dirid) from a `playlist` url: `qqmusic:fav`, `qqmusic:list:<diss>:<dirid>`,
/// `qqmusic:playlist:<diss>`, or a y.qq.com / i.y.qq.com link.
fn playlist_ref(url: &str) -> Option<(u64, u64)> {
    let url = url.trim();
    if url == "qqmusic:fav" {
        return Some((0, 201));
    }
    if let Some(rest) = url.strip_prefix("qqmusic:") {
        if let Some(list) = rest.strip_prefix("list:") {
            let (a, b) = list.split_once(':')?;
            return Some((a.parse().ok()?, b.parse().ok()?));
        }
        let rest = rest.strip_prefix("playlist:").unwrap_or(rest);
        return Some((rest.parse().ok()?, 0));
    }
    if kit::util::has_web_host(url, &["y.qq.com", "i.y.qq.com"]) {
        // https://y.qq.com/n/ryqq/playlist/<id>, …taoge.html?id=<id>, ?disstid=<id>
        if let Some(seg) = url.split("/playlist/").nth(1) {
            let digits: String = seg.chars().take_while(|c| c.is_ascii_digit()).collect();
            if !digits.is_empty() {
                return Some((digits.parse().ok()?, 0));
            }
        }
        for key in ["disstid", "id"] {
            if let Some(value) = kit::util::query_param(url, key) {
                return Some((value.parse().ok()?, 0));
            }
        }
    }
    None
}

/// (mid, media_mid) from a song's URI `qq:<mid>:<media_mid>`.
fn stream_ref(r: &str) -> Option<(String, String)> {
    let rest = r.strip_prefix("qq:")?;
    let (mid, media) = rest.split_once(':').unwrap_or((rest, ""));
    let valid = |s: &str| s.len() <= 128 && s.bytes().all(|b| b.is_ascii_alphanumeric());
    (!mid.is_empty() && valid(mid) && valid(media)).then(|| (mid.to_string(), media.to_string()))
}

fn main() {
    let mut client = Client::new();
    let mut session: Option<(String, login::QrSession)> = None;
    let mut credentials = session::Session::default();
    pipe::run("QQ Music", |req| {
        client.http.begin_call(pipe::str_param(req, "call").unwrap_or(""));
        let credential = pipe::str_param(req, "credential");
        match pipe::str_param(req, "call").unwrap_or("") {
            "connect.begin" => {
                let (sess, text) = login::create(&client)?;
                let id = format!("qq-{}", kit::util::rand_hex(8));
                session = Some((id.clone(), sess));
                pipe::ok(vec![
                    ("session".into(), Value::str(id)),
                    ("show".into(), json::obj(vec![("qr", Value::str(text))])),
                    ("poll_ms".into(), Value::Num(2000.0)),
                    ("expires_s".into(), Value::Num(300.0)),
                ])
            }
            "connect.poll" => {
                let Some((_, sess)) =
                    session.as_ref().filter(|(id, _)| pipe::str_param(req, "session") == Some(id.as_str()))
                else {
                    return pipe::fail(kit::pipe::tr("没有进行中的登录", "No sign-in in progress"));
                };
                match login::check(&client, sess)? {
                    login::Poll::Waiting => pipe::ok(vec![("state".into(), Value::str("waiting"))]),
                    login::Poll::Expired => {
                        session = None;
                        pipe::ok(vec![("state".into(), Value::str("expired"))])
                    }
                    login::Poll::Failed(e) => {
                        session = None;
                        pipe::ok(vec![("state".into(), Value::str("failed")), ("error".into(), Value::str(e))])
                    }
                    login::Poll::Done(c) => {
                        session = None;
                        credentials.clear();
                        let name = client.nickname(&c).unwrap_or_else(|| c.uin_str());
                        let credential = json::write(&c.to_json());
                        pipe::ok(vec![
                            ("state".into(), Value::str("done")),
                            ("credential".into(), Value::str(credential)),
                            ("display_name".into(), Value::str(name)),
                        ])
                    }
                }
            }
            "connect.cancel" => {
                if session.as_ref().is_some_and(|(id, _)| pipe::str_param(req, "session") == Some(id.as_str())) {
                    session = None;
                }
                pipe::ok(vec![])
            }
            "connect.end" => {
                if let Ok(Some(c)) = credentials.credential(&client, credential) {
                    client.logout(&c);
                }
                credentials.clear();
                session = None;
                pipe::ok(vec![])
            }
            "lists" => {
                let c = credentials.credential(&client, credential)?.ok_or(kit::pipe::tr("未登录", "Not signed in"))?;
                let lists = client.user_lists(&c)?;
                pipe::ok(vec![(
                    "lists".into(),
                    Value::Arr(lists.into_iter().map(kit::responses::UserList::into_json).collect()),
                )])
            }
            "playlist" => {
                let url = pipe::str_param(req, "url").unwrap_or("");
                let Some((diss, dirid)) = playlist_ref(url) else {
                    return pipe::fail(kit::pipe::tr("无法识别的 QQ 音乐歌单链接", "Not a QQ Music playlist link"));
                };
                let c = credentials.credential(&client, credential)?;
                let (name, songs) = client.playlist(diss, dirid, c.as_ref())?;
                // Withheld songs (no rights, taken down) have nothing to play: left out, counted.
                let unavailable = songs.iter().filter(|g| !g.playable).count();
                let uris: Vec<String> = songs.iter().map(|g| format!("qq:{}:{}", g.mid, g.media_mid)).collect();
                let entries = songs.iter().zip(&uris).filter(|(g, _)| g.playable).map(|(g, u)| kit::m3u8::Entry {
                    uri: u,
                    title: &g.title,
                    artist: &g.artist,
                    album: &g.album,
                    duration: g.duration,
                });
                pipe::ok(vec![
                    ("m3u8".into(), Value::str(kit::m3u8::write(&name, entries))),
                    ("unavailable".into(), Value::Num(unavailable as f64)),
                ])
            }
            "resolve" => {
                let uri = pipe::str_param(req, "uri").unwrap_or("");
                let Some((mid, media)) = stream_ref(uri) else {
                    return pipe::fail(kit::pipe::tr("无法识别的歌曲", "Unknown song"));
                };
                let c = credentials.credential(&client, credential)?;
                let url = client.stream_url(&mid, &media, c.as_ref())?;
                pipe::ok(vec![("url".into(), Value::str(url))])
            }
            "thumbs" => {
                let uris: Vec<String> = req
                    .get("uris")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect();
                let by_mid: Vec<(String, String)> =
                    uris.iter().filter_map(|u| stream_ref(u).map(|(m, _)| (m, u.clone()))).collect();
                let mids: Vec<String> = by_mid.iter().map(|(m, _)| m.clone()).collect();
                let c = credentials.credential(&client, credential)?;
                let got: std::collections::HashMap<String, String> =
                    client.thumbs(&mids, c.as_ref()).into_iter().collect();
                pipe::ok(vec![(
                    "thumbs".into(),
                    Value::Obj(
                        by_mid
                            .into_iter()
                            .filter_map(|(m, u)| got.get(&m).map(|t| (u, Value::str(t.clone()))))
                            .collect(),
                    ),
                )])
            }
            "metadata" => {
                let Some((mid, _)) = stream_ref(pipe::str_param(req, "uri").unwrap_or("")) else {
                    return pipe::ok(vec![]);
                };
                let c = credentials.credential(&client, credential)?;
                pipe::ok(client.metadata(&mid, c.as_ref()))
            }
            "play_event" => pipe::ok(vec![]),
            other => pipe::fail(format!("unknown call: {other}")),
        }
    });
}
