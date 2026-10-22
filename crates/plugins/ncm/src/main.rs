//! The NetEase Cloud Music plugin (docs/plugins.md): QR sign-in, the account's own playlists,
//! playlist import from links, quality-selectable stream addresses, online metadata.
//!
//! Flow and endpoints follow NeteaseCloudMusicApi (npm 4.32.0); the weapi encryption follows
//! its util/crypto.js (`crypto.rs`).

mod api;
mod crypto;
mod metadata;
mod quality;

use kit::json::Value;
use kit::pipe::{self};

/// A playlist id from a `playlist` url: `ncmlist:123`, bare digits, or a music.163.com link
/// carrying `id=`.
fn playlist_id(url: &str) -> Option<u64> {
    let url = url.trim();
    for p in ["ncmlist:", "ncm:playlist:"] {
        if let Some(rest) = url.strip_prefix(p) {
            return rest.parse().ok();
        }
    }
    if url.bytes().all(|b| b.is_ascii_digit()) {
        return url.parse().ok();
    }
    if kit::util::has_web_host(url, &["music.163.com"]) {
        return kit::util::query_param(url, "id")?.parse().ok();
    }
    None
}

/// A song's URI (`ncm:song:123`), `song_id`'s inverse.
fn song_uri(id: impl std::fmt::Display) -> String {
    format!("ncm:song:{id}")
}

/// A song id from its URI (`ncm:song:123`).
fn song_id(reference: &str) -> Option<u64> {
    let r = reference.trim();
    r.strip_prefix("ncm:song:").unwrap_or(r).parse().ok()
}

fn main() {
    let mut client = api::Client::new();
    let mut unikey: Option<String> = None;
    pipe::run("NetEase Cloud Music", |req| {
        client.begin_call(pipe::str_param(req, "call").unwrap_or(""));
        let credential = pipe::str_param(req, "credential").map(str::to_owned);
        match pipe::str_param(req, "call").unwrap_or("") {
            "connect.begin" => {
                let key = client.qr_unikey()?;
                let qr = format!("https://music.163.com/login?codekey={key}");
                unikey = Some(key.clone());
                pipe::ok(vec![
                    ("session".into(), Value::str(key)),
                    ("show".into(), kit::json::obj(vec![("qr", Value::str(qr))])),
                    ("poll_ms".into(), Value::Num(2000.0)),
                    ("expires_s".into(), Value::Num(300.0)),
                ])
            }
            "connect.poll" => {
                let Some(key) = unikey.clone().filter(|k| pipe::str_param(req, "session") == Some(k.as_str())) else {
                    return pipe::fail(kit::pipe::tr("没有进行中的登录", "No sign-in in progress"));
                };
                let status = client.qr_check(&key)?;
                match status.code {
                    800 => {
                        unikey = None;
                        pipe::ok(vec![("state".into(), Value::str("expired"))])
                    }
                    801 | 802 => pipe::ok(vec![("state".into(), Value::str("waiting"))]),
                    803 => {
                        unikey = None;
                        // The account's cookies are the credential (the host keeps them in its
                        // vault and hands them back to this plugin only).
                        let pairs: Vec<(&str, &str)> =
                            status.cookies.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
                        let credential = kit::http::cookie_header(&pairs);
                        if !status.cookies.iter().any(|(k, v)| k == "MUSIC_U" && !v.is_empty()) {
                            return pipe::fail(kit::pipe::tr(
                                "登录没有返回凭据",
                                "The sign-in returned no credentials",
                            ));
                        }
                        let name = client
                            .account(&credential)
                            .map(|(_, n)| n)
                            .unwrap_or_else(|_| kit::pipe::tr("网易云用户", "NetEase user").into());
                        pipe::ok(vec![
                            ("state".into(), Value::str("done")),
                            ("credential".into(), Value::str(credential)),
                            ("display_name".into(), Value::str(name)),
                        ])
                    }
                    _ => pipe::ok(vec![
                        ("state".into(), Value::str("failed")),
                        (
                            "error".into(),
                            Value::str(status.message.unwrap_or_else(|| {
                                format!(
                                    "{} ({})",
                                    kit::pipe::tr("登录状态异常", "Unexpected sign-in state"),
                                    status.code
                                )
                            })),
                        ),
                    ]),
                }
            }
            "connect.cancel" => {
                if pipe::str_param(req, "session") == unikey.as_deref() {
                    unikey = None;
                }
                pipe::ok(vec![])
            }
            "connect.end" => {
                unikey = None;
                if let Some(c) = credential {
                    client.logout(&c);
                }
                pipe::ok(vec![])
            }
            "lists" => {
                let Some(t) = credential else { return pipe::fail(kit::pipe::tr("未登录", "Not signed in")) };
                let lists = client.user_playlists(&t)?;
                pipe::ok(vec![(
                    "lists".into(),
                    Value::Arr(lists.into_iter().map(kit::responses::UserList::into_json).collect()),
                )])
            }
            "playlist" => {
                let url = pipe::str_param(req, "url").unwrap_or("");
                let Some(id) = playlist_id(url) else {
                    return pipe::fail(kit::pipe::tr("无法识别的网易云歌单链接", "Not a NetEase playlist link"));
                };
                let (name, songs) = client.playlist(id, credential.as_deref())?;
                // A song song/detail never named has nothing to show or play: left out, counted.
                let unavailable = songs.iter().filter(|g| g.title.is_empty()).count();
                let uris: Vec<String> = songs.iter().map(|g| song_uri(g.id)).collect();
                let entries =
                    songs.iter().zip(&uris).filter(|(g, _)| !g.title.is_empty()).map(|(g, u)| kit::m3u8::Entry {
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
                let Some(id) = song_id(uri) else {
                    return pipe::fail(kit::pipe::tr("无法识别的歌曲", "Unknown song"));
                };
                let cap = pipe::str_param(req, "option").unwrap_or("");
                let (url, level) = client.stream_url(id, cap, credential.as_deref())?;
                let name = quality::level_name(&level);
                let mut out = vec![("url".into(), Value::str(url)), ("option".into(), Value::str(level.clone()))];
                if !name.is_empty() {
                    out.push(("option_name".into(), Value::str(name)));
                }
                pipe::ok(out)
            }
            "options" => {
                let uri = pipe::str_param(req, "uri").unwrap_or("");
                let Some(id) = song_id(uri) else {
                    return pipe::fail(kit::pipe::tr("无法识别的歌曲", "Unknown song"));
                };
                let list = client.qualities(id, credential.as_deref())?;
                pipe::ok(vec![(
                    "values".into(),
                    Value::Arr(list.into_iter().map(kit::responses::Quality::into_json).collect()),
                )])
            }
            "thumbs" => {
                let uris: Vec<&str> =
                    req.get("uris").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str).collect();
                let ids: Vec<u64> = uris.iter().filter_map(|u| song_id(u)).collect();
                let got = client.thumbs(&ids, credential.as_deref());
                pipe::ok(vec![(
                    "thumbs".into(),
                    Value::Obj(got.into_iter().map(|(id, u)| (song_uri(id), Value::str(u))).collect()),
                )])
            }
            "metadata" => {
                let Some(id) = song_id(pipe::str_param(req, "uri").unwrap_or("")) else {
                    return pipe::ok(vec![]);
                };
                pipe::ok(client.metadata(id, credential.as_deref()))
            }
            "play_event" => pipe::ok(vec![]),
            other => pipe::fail(format!("unknown call: {other}")),
        }
    });
}
