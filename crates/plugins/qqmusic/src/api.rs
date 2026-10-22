//! The QQ Music web API over `musicu.fcg`, after L-1124/QQMusicApi: the common params
//! (`core/versioning.py`, web profile), the modules the plugin's calls need
//! (`modules/user.py`, `modules/songlist.py`, `modules/song.py`, `modules/lyric.py`).

use kit::http::Http;
use kit::json::{self, Value};
use kit::responses::UserList;
use kit::util;

pub const MUSICU: &str = "https://u.y.qq.com/cgi-bin/musicu.fcg";
const UA: &str =
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36";

pub use crate::credential::Credential;

/// A playlist's song, for the host's list.
pub struct Song {
    pub mid: String,
    pub media_mid: String,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub duration: Option<f64>,
    /// Down/off-tenant songs are listed but given no stream (the host marks them unavailable).
    pub playable: bool,
}

pub struct Client {
    pub http: Http,
    sip: Option<String>,
}

impl Client {
    pub fn new() -> Self {
        Client { http: Http::new(std::time::Duration::from_secs(10)), sip: None }
    }

    /// POST musicu.fcg: {"comm": …, "req_0": {module, method, param}} → req_0.data.
    /// `extra_comm` merges into comm (e.g. tmeLoginType at login).
    pub fn musicu(
        &self,
        module: &str,
        method: &str,
        param: Value,
        cred: Option<&Credential>,
        extra_comm: &[(&str, &str)],
    ) -> Result<Value, String> {
        // The web profile (versioning.py): every comm value is a string.
        let mut comm: Vec<(&str, String)> = vec![
            ("ct", "24".into()),
            ("cv", "4747474".into()),
            ("platform", "yqq.json".into()),
            ("chid", "0".into()),
            ("format", "json".into()),
            ("inCharset", "utf-8".into()),
            ("outCharset", "utf-8".into()),
            ("notice", "0".into()),
            ("need_new_code", "1".into()),
        ];
        if let Some(c) = cred.filter(|c| c.musicid > 0) {
            let g_tk = crate::crypto::hash33(&c.musickey, 5381).to_string();
            comm.push(("uin", c.musicid.to_string()));
            comm.push(("g_tk", g_tk.clone()));
            comm.push(("g_tk_new_20200303", g_tk));
        }
        for &(k, v) in extra_comm {
            comm.retain(|(ck, _)| *ck != k);
            comm.push((k, v.to_string()));
        }
        let body = json::write(&json::obj(vec![
            ("comm", Value::Obj(comm.into_iter().map(|(k, v)| (k.to_string(), Value::str(v))).collect())),
            (
                "req_0",
                json::obj(vec![("module", Value::str(module)), ("method", Value::str(method)), ("param", param)]),
            ),
        ]));
        let cookies = cred.map(Credential::cookies).unwrap_or_default();
        let cookie_refs: Vec<_> = cookies.iter().map(|(k, v)| (*k, v.as_str())).collect();
        // Authenticated cookies must not follow a server redirect to another host.
        let res = self.http.post_json(MUSICU, &body, &[("User-Agent", UA)], &cookie_refs)?;
        let v = res.json()?;
        let req = v.get("req_0").ok_or("no req_0 in the answer")?;
        let code = req.get("code").and_then(Value::as_num_loose).unwrap_or(-1.0) as i64;
        if code != 0 {
            return Err(format!("{} ({code})", kit::pipe::tr("接口返回错误", "The service returned an error")));
        }
        Ok(req.get("data").cloned().unwrap_or(Value::Null))
    }

    /// music.UserInfo.userInfoServer/GetLoginUserInfo → the account's nick (login.rs).
    pub fn nickname(&self, cred: &Credential) -> Option<String> {
        let data = self
            .musicu(
                "music.UserInfo.userInfoServer",
                "GetLoginUserInfo",
                Value::Obj(Default::default()),
                Some(cred),
                &[],
            )
            .ok()?;
        data.get("nick").and_then(Value::as_str).filter(|n| !n.is_empty()).map(str::to_owned)
    }

    /// Refresh an expired musickey (modules/login.py::refresh_credential).
    pub fn refresh(&self, cred: &Credential) -> Option<Credential> {
        let lt = cred.login_type.to_string();
        let data = self
            .musicu("music.login.LoginServer", "Login", cred.refresh_params(), Some(cred), &[("tmeLoginType", &lt)])
            .ok()?;
        let new = Credential::from_json(&data);
        (!new.musickey.is_empty()).then_some(new)
    }

    /// The account's playlists: 我喜欢, the created ones (GetPlaylistByUin), then the
    /// favourited ones (CgiGetPlaylistFavInfo).
    pub fn user_lists(&self, cred: &Credential) -> Result<Vec<UserList>, String> {
        let mut out = vec![UserList {
            url: "qqmusic:fav".into(),
            name: kit::pipe::tr("我喜欢", "Liked").into(),
            count: None,
            group: kit::pipe::tr("我创建的", "Created"),
        }];
        let created = self.musicu(
            "music.musicasset.PlaylistBaseRead",
            "GetPlaylistByUin",
            json::obj(vec![("uin", Value::str(cred.musicid.to_string()))]),
            Some(cred),
            &[],
        )?;
        for p in created.get("v_playlist").and_then(Value::as_array).into_iter().flatten() {
            let diss = p.get("dissid").or_else(|| p.get("tid")).and_then(Value::as_num_loose).unwrap_or(0.0) as u64;
            let dirid = p.get("dirId").or_else(|| p.get("dirid")).and_then(Value::as_num_loose).unwrap_or(0.0) as u64;
            let name = p
                .get("title")
                .or_else(|| p.get("dissname"))
                .or_else(|| p.get("dirName"))
                .and_then(Value::as_str)
                .unwrap_or(kit::pipe::tr("歌单", "Playlist"))
                .to_owned();
            let count = p.get("song_cnt").or_else(|| p.get("songnum")).and_then(Value::as_num_loose).map(|n| n as u64);
            if dirid == 201 {
                continue; // 我喜欢 already heads the list
            }
            out.push(UserList {
                url: format!("qqmusic:list:{diss}:{dirid}"),
                name,
                count,
                group: kit::pipe::tr("我创建的", "Created"),
            });
        }
        if !cred.encrypt_uin.is_empty() {
            if let Ok(fav) = self.musicu(
                "music.musicasset.PlaylistFavRead",
                "CgiGetPlaylistFavInfo",
                json::obj(vec![
                    ("uin", Value::str(cred.encrypt_uin.clone())),
                    ("offset", Value::Num(0.0)),
                    ("size", Value::Num(100.0)),
                ]),
                Some(cred),
                &[],
            ) {
                for p in fav.get("v_list").and_then(Value::as_array).into_iter().flatten() {
                    let Some(diss) =
                        p.get("dissid").or_else(|| p.get("tid")).and_then(Value::as_num_loose).map(|n| n as u64)
                    else {
                        continue;
                    };
                    let name = p
                        .get("title")
                        .or_else(|| p.get("dissname"))
                        .and_then(Value::as_str)
                        .unwrap_or(kit::pipe::tr("歌单", "Playlist"))
                        .to_owned();
                    let count =
                        p.get("song_cnt").or_else(|| p.get("songnum")).and_then(Value::as_num_loose).map(|n| n as u64);
                    out.push(UserList {
                        url: format!("qqmusic:list:{diss}:0"),
                        name,
                        count,
                        group: kit::pipe::tr("我收藏的", "Saved"),
                    });
                }
            }
        }
        Ok(out)
    }

    /// A playlist's songs (music.srfDissInfo.DissInfo/CgiGetDiss, paged).
    pub fn playlist(&self, disstid: u64, dirid: u64, cred: Option<&Credential>) -> Result<(String, Vec<Song>), String> {
        let euin = cred.map(|c| c.encrypt_uin.clone()).unwrap_or_default();
        let mut name = String::new();
        let mut songs = Vec::new();
        let mut begin = 0u64;
        loop {
            self.http.remaining()?;
            if begin >= 100_000 {
                return Err(kit::pipe::tr("歌单超过 100000 首上限", "The playlist has more than 100000 songs").into());
            }
            let mut param = vec![
                ("disstid", Value::Num(disstid as f64)),
                ("dirid", Value::Num(dirid as f64)),
                ("tag", Value::Bool(true)),
                ("song_begin", Value::Num(begin as f64)),
                ("song_num", Value::Num(200.0)),
                ("userinfo", Value::Bool(true)),
                ("orderlist", Value::Bool(true)),
                ("onlysonglist", Value::Bool(false)),
            ];
            if !euin.is_empty() {
                param.push(("enc_host_uin", Value::str(euin.clone())));
            }
            let data = self.musicu("music.srfDissInfo.DissInfo", "CgiGetDiss", json::obj(param), cred, &[])?;
            if name.is_empty() {
                name = data
                    .at(&["dirinfo", "title"])
                    .or_else(|| data.at(&["dirinfo", "dissname"]))
                    .and_then(Value::as_str)
                    .unwrap_or(kit::pipe::tr("QQ音乐歌单", "QQ Music playlist"))
                    .to_owned();
            }
            let batch: Vec<&Value> =
                data.get("songlist").and_then(Value::as_array).map(|a| a.iter().collect()).unwrap_or_default();
            let total = data.get("total_song_num").and_then(Value::as_num_loose).unwrap_or(0.0) as u64;
            for t in &batch {
                let mid = t.get("mid").and_then(Value::as_str).unwrap_or_default().to_owned();
                let artist = t
                    .get("singer")
                    .and_then(Value::as_array)
                    .map(|a| {
                        a.iter().filter_map(|x| x.get("name").and_then(Value::as_str)).collect::<Vec<_>>().join("/")
                    })
                    .unwrap_or_default();
                let status = t.get("status").and_then(Value::as_num_loose).unwrap_or(0.0) as u64;
                songs.push(Song {
                    media_mid: t.at(&["file", "media_mid"]).and_then(Value::as_str).unwrap_or_default().to_owned(),
                    playable: !mid.is_empty() && status == 0,
                    mid,
                    title: t.get("title").or_else(|| t.get("name")).and_then(Value::as_str).unwrap_or("").to_owned(),
                    artist,
                    album: t.at(&["album", "name"]).and_then(Value::as_str).unwrap_or("").to_owned(),
                    duration: t.get("interval").and_then(Value::as_num_loose),
                });
            }
            begin += batch.len() as u64;
            if batch.is_empty() || begin >= total || total == 0 {
                break;
            }
        }
        Ok((name, songs))
    }

    /// The playable address at the highest quality the account may use (modules/song.py:
    /// UrlGetVkey, one call carrying every candidate file name, the first non-empty purl
    /// wins). Encrypted files (.mflac/.mgg/…) are never asked for — the player cannot read
    /// them and the protocol forbids decrypting them.
    pub fn stream_url(&mut self, mid: &str, media_mid: &str, cred: Option<&Credential>) -> Result<String, String> {
        // Highest first: 臻品母带, 臻品音质 2.0, SQ FLAC, then OGG/MP3/AAC descents.
        const QUALITY: [(&str, &str); 9] = [
            ("AI00", ".flac"),
            ("Q000", ".flac"),
            ("F000", ".flac"),
            ("O801", ".ogg"),
            ("O800", ".ogg"),
            ("M800", ".mp3"),
            ("O600", ".ogg"),
            ("M500", ".mp3"),
            ("C400", ".m4a"),
        ];
        let filename: Vec<Value> = QUALITY
            .iter()
            .map(|(s, e)| {
                Value::str(if media_mid.is_empty() {
                    format!("{s}{mid}{mid}{e}")
                } else {
                    format!("{s}{media_mid}{e}")
                })
            })
            .collect();
        let n = QUALITY.len();
        let param = json::obj(vec![
            ("uin", Value::str(cred.map(|c| c.uin_str()).unwrap_or_else(|| "0".into()))),
            ("filename", Value::Arr(filename)),
            ("guid", Value::str(util::rand_hex(16))),
            ("songmid", Value::Arr(vec![Value::str(mid); n])),
            ("songtype", Value::Arr(vec![Value::Num(0.0); n])),
            ("ctx", Value::Num(0.0)),
        ]);
        let data = self.musicu("music.vkey.GetVkey", "UrlGetVkey", param, cred, &[])?;
        let infos = data.get("midurlinfo").and_then(Value::as_array);
        let purl =
            infos.and_then(|a| a.iter().find_map(|i| i.get("purl").and_then(Value::as_str).filter(|p| !p.is_empty())));
        let Some(purl) = purl else {
            return Err(kit::pipe::tr(
                "该歌曲暂时无法播放（可能需要会员或已下架）",
                "This song can't play now (it may need a membership or was taken down)",
            )
            .into());
        };
        Ok(format!("{}{}", self.sip(cred)?, purl))
    }

    /// The stream CDN root (music.audioCdnDispatch.cdnDispatch), cached for the process.
    fn sip(&mut self, cred: Option<&Credential>) -> Result<String, String> {
        if let Some(s) = &self.sip {
            return Ok(s.clone());
        }
        let data = self.musicu(
            "music.audioCdnDispatch.cdnDispatch",
            "GetCdnDispatch",
            json::obj(vec![
                ("guid", Value::str(util::rand_hex(16))),
                ("uid", Value::str("0")),
                ("use_new_domain", Value::Num(1.0)),
                ("use_ipv6", Value::Num(1.0)),
            ]),
            cred,
            &[],
        )?;
        let sip = data
            .get("sip")
            .and_then(Value::as_array)
            .and_then(|a| a.first())
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .unwrap_or("https://isure.stream.qqmusic.qq.com/")
            .to_owned();
        self.sip = Some(sip.clone());
        Ok(sip)
    }

    pub fn logout(&self, cred: &Credential) {
        let _ = self.musicu("music.login.LoginServer", "Logout", Value::Obj(Default::default()), Some(cred), &[]);
    }
}
