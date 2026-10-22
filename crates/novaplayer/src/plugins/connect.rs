//! Connecting a source, as `docs/plugins.md` §connect says: `connect.begin` names a session and
//! what to show (a QR code, a page to open, a code to type; a `form` flow shows nothing: the
//! user's values went with the call); `connect.poll` is asked at the plugin's own cadence until
//! it answers done; the credential goes to the vault (`accounts.rs`). Each call is a task the
//! connection holds, never a frame's work; between polls the connection's clock waits.

use crate::app::{Job, Landed, Player, ViewAsk};
use crate::plugins::pcm::host;
use novaplayer_plugin_protocol::json::Value;
use novaui::Ui;
use std::time::Duration;

/// What the sign-in panel shows the user.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Show {
    /// Text to draw as a QR code, for the phone to scan.
    Qr(String),
    /// A page the user finishes signing in on (opened in the browser).
    Url(String),
    /// A code the user types in at the service.
    Code(String),
}

pub struct Connection {
    pub plugin: String,
    /// What the plugin asked to show (None until `connect.begin` answered).
    pub show: Option<Show>,
    /// The session the plugin named, once `connect.begin` answered.
    session: Option<String>,
    /// A `form` flow: its first poll is asked at once (the values went with `connect.begin`).
    form: bool,
    /// The plugin's poll cadence, and when the connection gives up (host time).
    every: f64,
    until: f64,
    /// The call on its way (`connect.begin` or a poll): dropped with the connection.
    call: Option<novaui::Task>,
    /// The next poll's time (`Msg::Connect`).
    clock: novaui::Clock,
}

impl Connection {
    /// Asks nothing more of the plugin, what it shows kept (a screenshot of a connection
    /// waiting for the user).
    #[cfg(all(feature = "test-support", windows))] // the screenshot suites (Windows only)
    pub fn hold(&mut self) {
        self.clock.cancel();
        self.call = None;
    }
}

/// What `connect.begin` answered (`Job::ConnectBegun`).
pub struct Begun {
    session: String,
    show: Option<Show>,
    every: Duration,
    limit: Duration,
}

/// What a `connect.poll` answered (`Job::ConnectPolled`).
pub enum Polled {
    /// Not yet; what to show now (a renewed QR code, a code that changed).
    Waiting(Option<Show>),
    /// Signed in, as this account (its credential is in the vault).
    Done(String),
}

/// The poll cadence when the plugin names none, and the range it may name.
const POLL_DEFAULT: Duration = Duration::from_millis(2000);
const POLL_RANGE: (u64, u64) = (500, 10_000);
/// How long a connection may wait for the user when the plugin names none, and the longest it may ask.
const EXPIRES_DEFAULT: Duration = Duration::from_secs(300);
const EXPIRES_MAX: u64 = 900;

/// What a `connect.begin` / `connect.poll` answer asks to show: `show: {qr | url | code}`.
fn show_of(r: &crate::plugins::host::Reply) -> Option<Show> {
    let s = r.value.get("show")?;
    let text = |k: &str| s.get(k).and_then(Value::as_str).filter(|t| !t.is_empty()).map(str::to_owned);
    if let Some(q) = text("qr") {
        return Some(Show::Qr(q));
    }
    if let Some(u) = text("url").filter(|u| crate::files::is_web_address(u)) {
        return Some(Show::Url(u));
    }
    text("code").map(Show::Code)
}

/// Tells the plugin its session is called off (`connect.cancel`).
fn cancel(plugin: &str, session: &str) {
    let params = [("session", Value::Str(session.to_owned()))];
    let _ = host().ask(plugin, "connect.cancel", &params);
}

/// Starts a connection. An older one is called off first, so two never poll one plugin at
/// once. The flow is the first the plugin offers; `fields` are the values the user typed for a
/// `form` flow (id, value), sent once with `connect.begin` and kept nowhere.
pub fn start(p: &mut Player, ui: &mut Ui, plugin: &str, fields: Vec<(String, String)>) {
    let flow = p
        .plugin
        .registry
        .manifest(plugin)
        .and_then(|m| m.provides.connect.as_ref())
        .and_then(|c| c.flows.first().cloned());
    let Some(flow) = flow else { return };
    p.plugin.call_off_connection(ui);
    let form = flow == "form";
    let id = plugin.to_owned();
    let call = ui.spawn_waiting(
        move |_| {
            let mut params = vec![("flow", Value::Str(flow))];
            if form {
                params.push(("fields", Value::Obj(fields.into_iter().map(|(k, v)| (k, Value::Str(v))).collect())));
            }
            let begun = host().ask(&id, "connect.begin", &params)?;
            let Some(session) = begun.str("session").filter(|s| !s.is_empty()).map(str::to_owned) else {
                return Err(novaui::i18n::t("err.connect-no-session", &[]));
            };
            // The plugin holds the session from here: every way out but its own verdict says so.
            let show = show_of(&begun);
            if show.is_none() && !form {
                cancel(&id, &session);
                return Err(novaui::i18n::t("err.connect-nothing-shown", &[]));
            }
            let every = begun
                .num("poll_ms")
                .map_or(POLL_DEFAULT, |ms| Duration::from_millis((ms as u64).clamp(POLL_RANGE.0, POLL_RANGE.1)));
            let limit =
                begun.num("expires_s").map_or(EXPIRES_DEFAULT, |s| Duration::from_secs((s as u64).min(EXPIRES_MAX)));
            Ok(Begun { session, show, every, limit })
        },
        |r| Landed(Job::ConnectBegun(r)),
    );
    p.plugin.connection = Some(Connection {
        plugin: plugin.to_owned(),
        show: None,
        session: None,
        form,
        every: 0.0,
        until: f64::INFINITY,
        call: Some(call),
        clock: ui.clock(crate::app::Msg::Connect),
    });
}

/// Ends a connection the user or the app left (another one, a cancel, a sign-out, the plugin
/// removed): the plugin is told its session is off, as an errand.
pub fn call_off(errands: &mut crate::app::Tasks, ui: &mut Ui, c: Connection) {
    if let Some(session) = c.session.clone() {
        let plugin = c.plugin.clone();
        let task = ui.spawn_waiting(move |_| cancel(&plugin, &session), |()| ());
        errands.hold(Some(task));
    }
}

/// What it shows changed: a page to open is opened; the views follow.
fn shown(p: &mut Player, show: Show) {
    if let Show::Url(u) = &show {
        novaui::platform::open(u);
    }
    if let Some(c) = p.plugin.connection.as_mut() {
        c.show = Some(show);
    }
    p.view_asks.push(ViewAsk::RefreshPlugins);
}

/// `connect.begin` answered.
pub fn begun(p: &mut Player, ui: &mut Ui, r: Result<Begun, String>) {
    let b = match r {
        Ok(b) => b,
        Err(e) => return finished(p, ui, Err(e)),
    };
    let now = ui.now();
    let Some(c) = p.plugin.connection.as_mut() else { return };
    c.session = Some(b.session);
    c.every = b.every.as_secs_f64();
    c.until = now + b.limit.as_secs_f64();
    // A form's values have been sent: its first poll is asked at once.
    c.clock.at(if c.form { now } else { now + c.every });
    if let Some(s) = b.show {
        shown(p, s);
    }
}

/// The connection's clock: its next poll is due.
pub fn due(p: &mut Player, ui: &mut Ui) {
    let now = ui.now();
    let Some(c) = p.plugin.connection.as_mut() else { return };
    let Some(session) = c.session.clone() else { return };
    if c.call.as_ref().is_some_and(novaui::Task::pending) {
        return;
    }
    if now > c.until {
        let Some(plugin) = p.plugin.call_off_connection(ui) else { return };
        return finished_as(p, ui, &plugin, Err(novaui::i18n::t("err.connect-timeout", &[])));
    }
    let (plugin, acc) = (c.plugin.clone(), p.plugin.accounts.clone());
    c.call = Some(ui.spawn_waiting(
        move |_| {
            let r = host().ask(&plugin, "connect.poll", &[("session", Value::Str(session))])?;
            match r.str("state") {
                Some("done") => {
                    let credential = r.str("credential").unwrap_or_default().to_owned();
                    let who = r
                        .str("display_name")
                        .filter(|n| !n.is_empty())
                        .map_or_else(|| novaui::i18n::t("plugin.conn.anon", &[]), str::to_owned);
                    if credential.is_empty() {
                        return Err(novaui::i18n::t("err.connect-no-credentials", &[]));
                    }
                    match acc.save(&plugin, &credential, &who) {
                        Ok(()) => Ok(Polled::Done(who)),
                        Err(e) => {
                            // Not kept here, so the plugin must not keep the account either.
                            end_now(&plugin, &credential);
                            Err(e)
                        }
                    }
                }
                Some("expired") => Err(novaui::i18n::t("err.connect-expired", &[])),
                Some("failed") => {
                    Err(r.str("error").map_or_else(|| novaui::i18n::t("err.connect-failed", &[]), str::to_owned))
                }
                _ => Ok(Polled::Waiting(show_of(&r))),
            }
        },
        |r| Landed(Job::ConnectPolled(r)),
    ));
}

/// A `connect.poll` answered.
pub fn polled(p: &mut Player, ui: &mut Ui, r: Result<Polled, String>) {
    match r {
        Ok(Polled::Waiting(show)) => {
            let now = ui.now();
            let Some(c) = p.plugin.connection.as_mut() else { return };
            c.clock.at(now + c.every);
            // A QR code that was renewed, a code that changed.
            if let Some(s) = show.filter(|s| Some(s) != c.show.as_ref()) {
                shown(p, s);
            }
        }
        Ok(Polled::Done(who)) => finished(p, ui, Ok(who)),
        Err(e) => finished(p, ui, Err(e)),
    }
}

/// The plugin's verdict: the connection is over, its session the plugin's to end.
fn finished(p: &mut Player, ui: &mut Ui, result: Result<String, String>) {
    if let Some(c) = p.plugin.connection.take() {
        finished_as(p, ui, &c.plugin, result);
    }
}

/// The connection ended with `result`: the account's name, or why not.
fn finished_as(p: &mut Player, ui: &mut Ui, plugin: &str, result: Result<String, String>) {
    match result {
        Ok(who) => {
            p.toast(ui, ui.t("toast.connected", &[("who", &who)]));
            // The typed values (a password among them) are not kept.
            p.plugin.form.clear();
            // A source with a library offers its playlists for import (docs/plugins.md §lists).
            let library = p.plugin.registry.manifest(plugin).is_some_and(|m| m.provides.lists);
            if library {
                p.pick_lists(ui, plugin);
            }
        }
        Err(e) => p.toast_error(ui, ui.t("toast.connect-failed", &[("e", &e)])),
    }
    p.view_asks.push(ViewAsk::RefreshPlugins);
}

/// Tells the plugin the account is disconnected (`connect.end`), on the caller's thread (a
/// sign-out's task). Best effort: the credential is already gone from the vault, whatever the
/// plugin answers.
pub fn end_now(plugin: &str, credential: &str) {
    let _ = host().ask(plugin, "connect.end", &[("credential", Value::Str(credential.to_owned()))]);
}
