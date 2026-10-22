//! What is shown of one plugin, wherever it is shown: the app menu's panel (a quick way in) and
//! its settings page (`设置 › 插件 › <name>`). Two entries, one description: each draws this in
//! its own container (flyout rows, cards), neither works out a fact or a string of its own.
use crate::plugins::connect::Show;
use crate::plugins::registry::{FieldKind, Plugin, State};
use crate::plugins::Plugins;
use novaui::Ui;

pub struct PluginView {
    pub id: String,
    pub name: String,
    /// Empty: the manifest names none.
    pub version: String,
    /// Its state in words (已启用 / 未启用 / …).
    pub note: String,
    /// It cannot run: why.
    pub broken: Option<String>,
    pub on: bool,
    /// Its switch was pressed once: what it asks for is showing, a second press enables.
    pub pending: bool,
    /// While pending: what it asked for, before it runs with the user's rights.
    pub asks: Vec<String>,
    /// The connection, when it is on and connects.
    pub conn: Option<Conn>,
    /// Connected, and offers its account's playlists.
    pub lists: bool,
    /// Why a control it declared could not be read (its other abilities work).
    pub fault: Option<String>,
}

pub enum Conn {
    /// Connected as this account.
    Who(String),
    /// Connecting: what to do now, and the QR code to scan, when it gave one.
    Busy { line: String, qr: Option<String> },
    /// Not connected: the form to fill (label, kind), then 连接.
    Form(Vec<(String, FieldKind)>),
}

impl PluginView {
    /// `pending`: the press waiting for its second (`Player::pending`).
    pub fn of(ui: &Ui, plugins: &Plugins, pl: &Plugin, pending: Option<&crate::app::command::Confirm>) -> Self {
        let id = pl.id();
        let on = pl.state == State::On;
        let pending = !on && matches!(pending, Some(crate::app::command::Confirm::EnablePlugin(p)) if *p == id);
        let note = ui.t(
            match pl.state {
                State::On => "plugin.on",
                State::Off => "plugin.off",
                State::Changed => "plugin.changed",
                State::Checking => "plugin.checking",
                State::Broken(_) => "plugin.broken",
            },
            &[],
        );
        let broken = match &pl.state {
            State::Broken(why) => Some(why.clone()),
            _ => None,
        };
        let mut asks = Vec::new();
        if pending {
            let summary = pl.summary();
            if !summary.is_empty() {
                asks.push(summary);
            }
            if let Some(m) = pl.manifest.as_ref().filter(|m| !m.network.is_empty()) {
                asks.push(ui.t("plugin.net", &[("hosts", &m.network.join(", "))]));
            }
            asks.push(ui.t("plugin.trust", &[]));
        }
        let who = plugins.accounts.name(&id);
        let manifest = pl.manifest.as_ref();
        let connect = manifest.and_then(|m| m.provides.connect.as_ref()).filter(|_| on && broken.is_none());
        let conn = connect.map(|c| match (&who, plugins.connection.as_ref().filter(|s| s.plugin == id)) {
            (Some(a), _) => Conn::Who(a.clone()),
            (None, Some(s)) => {
                let (line, qr) = match &s.show {
                    Some(Show::Qr(q)) => (ui.t("plugin.conn.qr", &[]), Some(q.clone())),
                    Some(Show::Url(_)) => (ui.t("plugin.conn.url", &[]), None),
                    Some(Show::Code(code)) => (ui.t("plugin.conn.code", &[("code", code)]), None),
                    None => (ui.t("plugin.conn.wait", &[]), None),
                };
                Conn::Busy { line, qr }
            }
            (None, None) => Conn::Form(
                c.fields
                    .iter()
                    .map(|f| (manifest.map_or_else(|| f.label.clone(), |m| m.text(&f.label)), f.kind))
                    .collect(),
            ),
        });
        let lists = on && who.is_some() && pl.manifest.as_ref().is_some_and(|m| m.provides.lists);
        let fault = match pl.manifest.as_ref().map(|m| &m.controls) {
            Some(Err(why)) => Some(why.clone()),
            _ => None,
        };
        let version = pl.manifest.as_ref().map(|m| m.version.clone()).unwrap_or_default();
        PluginView { id, name: pl.name(), version, note, broken, on, pending, asks, conn, lists, fault }
    }
}
