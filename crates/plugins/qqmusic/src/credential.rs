//! QQ credential serialization and process-local refresh cache.
use kit::json::{self, Value};
use kit::util;

/// The account credential (models/request.py::Credential), serialised as the credential the host keeps for the plugin.
#[derive(Clone, Default)]
pub struct Credential {
    pub musicid: u64,
    pub musickey: String,
    pub str_musicid: String,
    pub openid: String,
    pub access_token: String,
    pub refresh_token: String,
    pub refresh_key: String,
    pub unionid: String,
    pub login_type: u64,
    pub encrypt_uin: String,
    pub musickey_create_time: u64,
    pub key_expires_in: u64,
    pub expired_at: u64,
}

impl Credential {
    /// It can sign calls: a music key and a music id.
    pub fn usable(&self) -> bool {
        !self.musickey.is_empty() && self.musicid != 0
    }

    pub fn from_json(v: &Value) -> Credential {
        let s = |keys: &[&str]| keys.iter().find_map(|k| v.get(k).and_then(Value::as_str_loose)).unwrap_or_default();
        let n = |keys: &[&str]| keys.iter().find_map(|k| v.get(k).and_then(Value::as_num_loose)).unwrap_or(0.0) as u64;
        Credential {
            musicid: n(&["musicid"]),
            musickey: s(&["musickey"]),
            str_musicid: s(&["str_musicid", "strMusicid"]),
            openid: s(&["openid"]),
            access_token: s(&["access_token", "accessToken"]),
            refresh_token: s(&["refresh_token", "refreshToken"]),
            refresh_key: s(&["refresh_key", "refreshKey"]),
            unionid: s(&["unionid"]),
            login_type: v
                .get("loginType")
                .or_else(|| v.get("login_type"))
                .and_then(Value::as_num_loose)
                .map(|n| n as u64)
                .unwrap_or_else(|| if s(&["musickey"]).starts_with("W_X") { 1 } else { 2 }),
            encrypt_uin: s(&["encryptUin", "encrypt_uin"]),
            musickey_create_time: n(&["musickeyCreateTime"]),
            key_expires_in: n(&["keyExpiresIn"]),
            expired_at: n(&["expired_at"]),
        }
    }

    pub fn to_json(&self) -> Value {
        json::obj(vec![
            ("musicid", Value::Num(self.musicid as f64)),
            ("musickey", Value::str(self.musickey.clone())),
            ("strMusicid", Value::str(self.str_musicid.clone())),
            ("openid", Value::str(self.openid.clone())),
            ("accessToken", Value::str(self.access_token.clone())),
            ("refreshToken", Value::str(self.refresh_token.clone())),
            ("refreshKey", Value::str(self.refresh_key.clone())),
            ("unionid", Value::str(self.unionid.clone())),
            ("loginType", Value::Num(self.login_type as f64)),
            ("encryptUin", Value::str(self.encrypt_uin.clone())),
            ("musickeyCreateTime", Value::Num(self.musickey_create_time as f64)),
            ("keyExpiresIn", Value::Num(self.key_expires_in as f64)),
            ("expired_at", Value::Num(self.expired_at as f64)),
        ])
    }

    /// Refresh payload from QQMusicApi modules/login.py. OAuth expired_at differs from
    /// the musickey lifetime keyExpiresIn; do not interchange them.
    pub fn refresh_params(&self) -> Value {
        let mut fields = vec![
            ("openid", Value::str(self.openid.clone())),
            ("refresh_token", Value::str(self.refresh_token.clone())),
            ("musickey", Value::str(self.musickey.clone())),
            ("refresh_key", Value::str(self.refresh_key.clone())),
            ("loginMode", Value::Num(2.0)),
        ];
        if self.login_type != 1 {
            fields.extend([
                ("access_token", Value::str(self.access_token.clone())),
                ("expired_in", Value::Num(self.expired_at as f64)),
                ("musicid", Value::Num(self.musicid as f64)),
            ]);
        }
        if self.login_type != 2 {
            fields.extend([("str_musicid", Value::str(self.uin_str())), ("unionid", Value::str(self.unionid.clone()))]);
        }
        json::obj(fields)
    }

    pub fn expired(&self) -> bool {
        self.musickey_create_time > 0
            && self.key_expires_in > 0
            && util::now_s() >= self.musickey_create_time.saturating_add(self.key_expires_in)
    }

    /// Web transport cookie names from QQMusicApi core/executor.py. Never send refresh
    /// tokens to ordinary endpoints: only these four cookies belong to the web profile.
    pub fn cookies(&self) -> Vec<(&'static str, String)> {
        let mut out = Vec::new();
        if self.musicid > 0 {
            let uin = self.uin_str();
            out.push(("uin", uin.clone()));
            out.push(("qqmusic_uin", uin));
        }
        if !self.musickey.is_empty() {
            out.push(("qm_keyst", self.musickey.clone()));
            out.push(("qqmusic_key", self.musickey.clone()));
        }
        out
    }

    /// uin for UrlGetVkey (str_musicid when present).
    pub fn uin_str(&self) -> String {
        if !self.str_musicid.is_empty() {
            self.str_musicid.clone()
        } else {
            self.musicid.to_string()
        }
    }
}
