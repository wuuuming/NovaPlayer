//! A plugin's panel: the second flyout, right of the app menu.
//! Name, version and state; what the plugin asked for and the switch that enables it; the
//! connection (sign in / out, the QR code, link, code or form while connecting); the account's
//! playlists to import; a control that could not be read. The player draws all of it; the
//! plugin only declared what it can do.
use crate::app::{Models, Player, Views};
use crate::plugins::registry::{FieldKind, Plugin};
use crate::views::menus::*;
use novaui::dsl::Val;
use novaui::*;

/// The values the form's fields hold, as `(field id, value)` pairs (what `connect.begin` gets).
pub(crate) fn connect_fields(p: &Player, plugin: &str) -> Vec<(String, String)> {
    let Some(c) = p.plugin.registry.manifest(plugin).and_then(|m| m.provides.connect.as_ref()) else {
        return Vec::new();
    };
    let form = &p.plugin.form;
    let values = if form.plugin == plugin { &form.values[..] } else { &[] };
    c.fields.iter().enumerate().map(|(k, f)| (f.id.clone(), values.get(k).cloned().unwrap_or_default())).collect()
}

/// A line of text in the flyout (no action).
fn line(v: &mut Views, ui: &mut Ui, menu: Id, text: &str, name: &str) {
    let _ = v.doc.instantiate(ui, menu, None, "MenuLine", &[Val::Str(text.into())], name);
}

/// A section heading.
fn heading(v: &mut Views, ui: &mut Ui, menu: Id, text: &str, name: &str) {
    let _ = v.doc.instantiate(ui, menu, None, "MenuGroup", &[Val::Str(text.into())], name);
}

/// A sign-in's QR code of `text` under `parent` (the sheet's `QrCode`, placed `align`), named
/// `name` (the plugin flyout's and the settings page's connections).
pub(crate) fn qr_code(doc: &mut novaui::dsl::Doc, ui: &mut Ui, parent: Id, text: &str, align: &str, name: &str) {
    if doc.instantiate(ui, parent, None, "QrCode", &[Val::Ident(align.into())], name).is_ok() {
        let image = novaui::widgets::qr::image(text);
        ui.tree.set_content(
            doc.id(&format!("{name}.image")),
            Content::Image(ImageContent { image, opacity: 1.0, fit_square: true, hdr: false }),
        );
    }
}

/// One form field: a single-line text box with the field's label as its hint, holding `kept`;
/// its events name `form:<k>` (the plugin flyout's and the settings page's connection forms).
/// Returns the box.
#[allow(clippy::too_many_arguments)] // Flat field description; a struct would only rename the arguments.
pub(crate) fn field(
    doc: &mut novaui::dsl::Doc,
    ui: &mut Ui,
    parent: Id,
    name: &str,
    k: usize,
    label: &str,
    kind: FieldKind,
    kept: &str,
) -> Option<Id> {
    let args = [Val::Str(label.into()), Val::Str(Command::Form(k).to_string())];
    doc.instantiate(ui, parent, None, "TextBox", &args, name).ok()?;
    let edit = doc.id(&format!("{name}.edit"));
    ui.set_edit_secret(edit, kind == FieldKind::Password);
    ui.set_edit_text(edit, kept);
    Some(edit)
}

/// Builds the flyout of `pl` (the open one), beside its `row` in the app flyout.
pub(crate) fn open_panel(v: &mut Views, ui: &mut Ui, m: &Models, pl: &Plugin, row: Id) {
    use crate::views::plugin_view::{Conn, PluginView};
    let host = v.id("plugin-flyout");
    let fresh = !ui.tree.is_up(host);
    let panel = v.menus.side.reset(ui, host);
    v.doc.prune(ui);
    let pv = PluginView::of(ui, m.plugin, pl, m.pending);
    let id = pv.id.clone();

    // Name, version, state.
    let title = if pv.version.is_empty() { pv.name.clone() } else { format!("{}  {}", pv.name, pv.version) };
    heading(v, ui, panel, &title, "pp-title");
    let mut first_field = None;
    // One that cannot run: why, and the way out.
    if let Some(why) = &pv.broken {
        line(v, ui, panel, why, "pp-why");
    } else {
        for (k, ask) in pv.asks.iter().enumerate() {
            line(v, ui, panel, ask, &format!("pp-ask{k}"));
        }
        let label = ui.t(if pv.pending { "plugin.enable.confirm" } else { "plugin.enable" }, &[]);
        check_item(v, ui, panel, pv.on, &label, &pv.note, Some(Command::PluginSwitch(id.clone())), true, "pp-on");

        // The connection.
        if let Some(conn) = &pv.conn {
            let _ = v.doc.instantiate(ui, panel, None, "MenuRule", &[], "pp-r1");
            heading(v, ui, panel, &ui.t("plugin.conn", &[]), "pp-conn");
            match conn {
                Conn::Who(a) => {
                    line(v, ui, panel, &ui.t("plugin.conn.who", &[("who", a)]), "pp-who");
                    let _ = item(
                        v,
                        ui,
                        panel,
                        Icon::Person,
                        &ui.t("plugin.conn.out", &[]),
                        "",
                        Some(Command::Disconnect(id.clone())),
                        true,
                        "pp-out",
                    );
                }
                Conn::Busy { line: words, qr } => {
                    if let Some(q) = qr {
                        qr_code(v.doc, ui, panel, q, "center", "pp-qr");
                    }
                    line(v, ui, panel, words, "pp-busy");
                    let _ = item(
                        v,
                        ui,
                        panel,
                        Icon::Dismiss,
                        &ui.t("plugin.conn.cancel", &[]),
                        "",
                        Some(Command::ConnectCancel(id.clone())),
                        true,
                        "pp-cancel",
                    );
                }
                Conn::Form(fields) => {
                    for (k, (label, kind)) in fields.iter().enumerate() {
                        let kept = m.plugin.form.kept(&id, k);
                        let e = field(v.doc, ui, panel, &format!("pp-f{k}"), k, label, *kind, kept);
                        first_field = first_field.or(e);
                    }
                    let _ = item(
                        v,
                        ui,
                        panel,
                        Icon::Person,
                        &ui.t("plugin.conn.go", &[]),
                        "",
                        Some(Command::Connect(id.clone())),
                        true,
                        "pp-go",
                    );
                }
            }
        }
        if pv.lists {
            let _ = item(
                v,
                ui,
                panel,
                Icon::ArrowImport,
                &ui.t("plugin.lists.open", &[]),
                "",
                Some(Command::PickLists(id.clone())),
                true,
                "pp-lists",
            );
        }
        if let Some(fault) = &pv.fault {
            let _ = v.doc.instantiate(ui, panel, None, "MenuRule", &[], "pp-r2");
            line(v, ui, panel, &ui.t("plugin.fault.line", &[("why", fault)]), "pp-fault");
        }
    }
    let _ = v.doc.instantiate(ui, panel, None, "MenuRule", &[], "pp-r3");
    let _ = item(
        v,
        ui,
        panel,
        Icon::Delete,
        &ui.t("plugin.delete", &[]),
        "",
        Some(Command::PluginDelete(id.clone())),
        true,
        "pp-del",
    );
    // Right of the app flyout, its top level with the row; left of it when there is no room.
    show_side(v, ui, row);
    // Typing starts in the first field.
    if let Some(e) = first_field.filter(|_| fresh) {
        ui.set_focus(Some(e));
    }
}
