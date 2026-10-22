//! QQ's web QR sign-in (L-1124/QQMusicApi `modules/login.py`, the QRLoginType.QQ path):
//! ptqrshow → the QR image and the `qrsig` cookie; ptqrlogin polls with
//! ptqrtoken = hash33(qrsig); on success check_sig sets p_skey, graph.qq.com's authorize
//! trades it for a code, and musicu's QQConnectLogin turns the code into the music
//! credential.

use crate::api::{Client, Credential};
use crate::crypto::hash33;
use kit::json::{self, Value};
use kit::util;

pub struct QrSession {
    qrsig: String,
}

pub enum Poll {
    Waiting,
    Expired,
    Failed(String),
    Done(Box<Credential>),
}

/// `signin` (qr): fetch the QR image, decode it to the text the host renders.
pub fn create(client: &Client) -> Result<(QrSession, String), String> {
    let url = format!(
        "https://ssl.ptlogin2.qq.com/ptqrshow?appid=716027609&e=2&l=M&s=3&d=72&v=4&t=0.{}&daid=383&pt_3rd_aid=100497308",
        util::now_ms() % 1_000_000
    );
    let res = client.http.get(&url, &[("Referer", "https://xui.ptlogin2.qq.com/")], &[], false)?;
    let qrsig = res.cookie("qrsig").ok_or("no qrsig returned")?;
    let text = decode_qr(&res.body)?;
    Ok((QrSession { qrsig }, text))
}

/// The QR image (PNG/GIF) decoded to the text it encodes (rqrr).
fn decode_qr(img: &[u8]) -> Result<String, String> {
    let mut reader = image::ImageReader::new(std::io::Cursor::new(img))
        .with_guessed_format()
        .map_err(|e| format!("QR image cannot be decoded: {e}"))?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(2048);
    limits.max_image_height = Some(2048);
    limits.max_alloc = Some(32 * 1024 * 1024);
    reader.limits(limits);
    let gray = reader.decode().map_err(|e| format!("QR image cannot be decoded: {e}"))?.to_luma8();
    let mut prepared = rqrr::PreparedImage::prepare(gray);
    let grid = prepared.detect_grids().into_iter().next().ok_or("QR code not found in the image")?;
    let (_, text) = grid.decode().map_err(|e| format!("QR code cannot be read: {e}"))?;
    Ok(text)
}

/// `signin_poll`: the QR's state; on success, authorize into the music credential.
pub fn check(client: &Client, s: &QrSession) -> Result<Poll, String> {
    let url = format!(
        "https://ssl.ptlogin2.qq.com/ptqrlogin?u1=https%3A%2F%2Fgraph.qq.com%2Foauth2.0%2Flogin_jump&ptqrtoken={}&ptredirect=0&h=1&t=1&g=1&from_ui=1&ptlang=2052&action=0-0-{}&js_ver=20102616&js_type=1&login_sig=&pt_uistyle=40&aid=716027609&daid=383&pt_3rd_aid=100497308&has_onekey=1",
        hash33(&s.qrsig, 0),
        util::now_ms()
    );
    let res =
        client.http.get(&url, &[("Referer", "https://xui.ptlogin2.qq.com/")], &[("qrsig", s.qrsig.as_str())], false)?;
    let text = res.text()?;
    let args = ptui_args(&text).ok_or("sign-in state cannot be parsed")?;
    let code: u64 = args.first().and_then(|a| a.parse().ok()).ok_or("sign-in state code cannot be parsed")?;
    match code {
        // QRCodeLoginEvents: DONE (0, 405), TIMEOUT (65, 402), REFUSE (68, 403), else waiting.
        0 | 405 => {
            let url = args.get(2).filter(|u| u.starts_with("http")).ok_or("signed in but no redirect returned")?;
            authorize(client, url).map(Box::new).map(Poll::Done)
        }
        65 | 402 => Ok(Poll::Expired),
        68 | 403 => {
            Ok(Poll::Failed(kit::pipe::tr("已在手机上取消登录", "The sign-in was cancelled on the phone").into()))
        }
        _ => Ok(Poll::Waiting),
    }
}

/// `ptuiCB('66','0','','0','…')` → its single-quoted arguments.
fn ptui_args(text: &str) -> Option<Vec<String>> {
    let bytes = text.as_bytes();
    let mut i = text.find("ptuiCB(")? + 7;
    let mut args = Vec::new();
    while i < bytes.len() && bytes[i] != b')' {
        if bytes[i] == b'\'' {
            i += 1;
            let mut s = Vec::new();
            while i < bytes.len() && bytes[i] != b'\'' {
                if bytes[i] == b'\\' && i + 1 < bytes.len() {
                    i += 1;
                }
                s.push(bytes[i]);
                i += 1;
            }
            if bytes.get(i) != Some(&b'\'') {
                return None;
            }
            i += 1;
            args.push(String::from_utf8(s).ok()?);
        } else {
            i += 1;
        }
    }
    (bytes.get(i) == Some(&b')')).then_some(args)
}

/// check_sig → p_skey; authorize → code; QQLogin → the credential (login.py::_authorize_qq_qr).
fn authorize(client: &Client, jump: &str) -> Result<Credential, String> {
    let uin = util::urlencode(util::query_param(jump, "uin").ok_or("the redirect has no uin")?);
    let sigx = util::urlencode(util::query_param(jump, "ptsigx").ok_or("the redirect has no ptsigx")?);
    let url = format!(
        "https://ssl.ptlogin2.graph.qq.com/check_sig?uin={uin}&pttype=1&service=ptqrlogin&nodirect=0&ptsigx={sigx}&s_url=https%3A%2F%2Fgraph.qq.com%2Foauth2.0%2Flogin_jump&ptlang=2052&ptredirect=100&aid=716027609&daid=383&j_later=0&low_login_hour=0&regmaster=0&pt_login_type=3&pt_aid=0&pt_aaid=16&pt_light=0&pt_3rd_aid=100497308"
    );
    let res = client.http.get(&url, &[("Referer", "https://xui.ptlogin2.qq.com/")], &[], false)?;
    let p_skey = res.cookie("p_skey").filter(|v| !v.is_empty()).ok_or("no p_skey returned")?;
    let cookies: Vec<(&str, &str)> = res.cookies.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
    let res = client.http.post_form(
        "https://graph.qq.com/oauth2.0/authorize",
        &util::form(&[
            ("response_type", "code"),
            ("client_id", "100497308"),
            ("redirect_uri", "https://y.qq.com/portal/wx_redirect.html?login_type=1&surl=https://y.qq.com/"),
            ("scope", "get_user_info,get_app_friends"),
            ("state", "state"),
            ("switch", ""),
            ("from_ptlogin", "1"),
            ("src", "1"),
            ("update_auth", "1"),
            ("openapi", "1010_1030"),
            ("g_tk", &hash33(&p_skey, 5381).to_string()),
            ("auth_time", &util::now_ms().to_string()),
            (
                "ui",
                &format!(
                    "{:08x}-{:04x}-{:04x}-{:04x}-{:012x}",
                    util::rand_bytes(4).iter().fold(0u32, |n, b| (n << 8) | u32::from(*b)),
                    util::now_ms() % 0xffff,
                    util::now_ms() % 0xffff,
                    util::now_ms() % 0xffff,
                    util::now_ms() % 0xffffffffffff
                ),
            ),
        ]),
        &[],
        &cookies,
        false,
    )?;
    let location = res.header("Location").ok_or("the authorization returned no redirect")?;
    let code = util::query_param(&location, "code").ok_or("the authorization returned no code")?;
    let data = client.musicu(
        "QQConnectLogin.LoginServer",
        "QQLogin",
        json::obj(vec![("code", Value::str(code))]),
        None,
        &[("tmeLoginType", "2")],
    )?;
    let cred = Credential::from_json(&data);
    if !cred.usable() {
        return Err(kit::pipe::tr("登录没有返回音乐凭据", "The sign-in returned no music credentials").into());
    }
    Ok(cred)
}
