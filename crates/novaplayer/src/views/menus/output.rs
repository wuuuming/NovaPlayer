//! The output flyout.
use crate::app::{Models, Views};
use crate::playback::output::Device;
use crate::player_core::AudioOutputDeviceInfo;
use crate::views::menus::*;
use novaui::*;

/// The output flyout (6.6): one flat panel above the volume button, as Windows 11's quick
/// settings sound picker: the volume (when the control bar has no room for its own), then the
/// drivers when there is a choice of them, then the devices of the chosen driver, each a menu
/// row with its check; DSD over DoP last under ASIO. No dropdown inside a flyout: one press picks.
/// As wide as its widest row (`Kind::width`).
pub fn open_output_menu(v: &mut Views, ui: &mut Ui, m: &Models) {
    let menu = begin(v, ui, Kind::Output);
    let rows = m.playback.output.device_rows();
    let asio = m.playback.output.has_asio();
    let on_asio = m.playback.output.choice.system == crate::player_core::AudioOutputHost::Asio;
    let rule = |v: &mut Views, ui: &mut Ui, n: &str| {
        let _ = v.doc.instantiate(ui, menu, None, "MenuRule", &[], n);
    };
    // Drivers: only when the build offers a choice.
    if asio {
        let wasapi = crate::playback::output::system_name(crate::player_core::AudioOutputHost::Wasapi);
        check_item(
            v,
            ui,
            menu,
            !on_asio,
            &wasapi,
            "",
            Some(Command::OutSys(crate::player_core::AudioOutputHost::Wasapi)),
            true,
            "om-s0",
        );
        check_item(
            v,
            ui,
            menu,
            on_asio,
            "ASIO",
            "",
            Some(Command::OutSys(crate::player_core::AudioOutputHost::Asio)),
            true,
            "om-s1",
        );
        rule(v, ui, "om-r1");
    }
    // Devices of the chosen driver.
    if rows.is_empty() {
        check_item(v, ui, menu, false, &ui.t("output.loading", &[]), "", None, false, "om-ld");
    }
    for (k, (device, route, on)) in rows.into_iter().enumerate() {
        let (label, detail) = match device {
            Device::Default(None) => (ui.t("output.default", &[]), String::new()),
            Device::Default(Some(n)) => (ui.t("output.default-of", &[("device", &n)]), String::new()),
            Device::One(d) => (d.name.clone(), device_detail(ui, &d)),
        };
        check_item(v, ui, menu, on, &label, &detail, Some(Command::Route(route)), true, &format!("om-d{k}"));
    }
    if on_asio {
        rule(v, ui, "om-r2");
        check_item(
            v,
            ui,
            menu,
            m.playback.output.choice.dop,
            &ui.t("output.dop", &[]),
            "",
            Some(Command::OutDop(!m.playback.output.choice.dop)),
            true,
            "om-dop",
        );
    }
    // Above the volume button; a long device list scrolls inside its own height, between the
    // title bar and the button: it never covers the window.
    let vol = v.id("vol");
    show(v, ui, Place::on(vol, Side::Above, Along::Start, 4.0).shift(-4.0).fixed(), Some(vol));
}

fn khz(rate: u32) -> String {
    if rate.is_multiple_of(1000) {
        format!("{}", rate / 1000)
    } else {
        format!("{:.1}", rate as f64 / 1000.0)
    }
}

/// A device's detail at its row's right: channels and rates, DSD, 通用 for a wrapping driver.
fn device_detail(ui: &Ui, d: &AudioOutputDeviceInfo) -> String {
    let mut said = Vec::new();
    if d.generic {
        said.push(ui.t("output.generic", &[]));
    }
    if let Some(c) = &d.caps {
        said.push(ui.t("output.channels", &[("n", &c.channels.to_string())]));
        match (c.rates.iter().min(), c.rates.iter().max()) {
            (Some(a), Some(b)) if a == b => said.push(format!("{} kHz", khz(*a))),
            (Some(a), Some(b)) => said.push(format!("{}–{} kHz", khz(*a), khz(*b))),
            _ => {}
        }
        if c.dsd == Some(true) {
            said.push("DSD".to_owned());
        }
    }
    said.join(" · ")
}

/// The output flyout is up (it shows the outputs as they are now when they change).
pub(crate) fn is_up(v: &Views) -> bool {
    super::is_open(v, |k| *k == super::Kind::Output)
}

/// The settings page's output dropdowns (`设置 › 播放`): the interface, the device, DoP. They
/// choose what the flyout chooses; their lists are made here from the outputs as listed.
pub const DROPS: [&str; 3] = ["output-api", "output-device", "output-dop"];

/// A dropdown's rows (what a pick does, its label) and the current one.
pub type DropRows = (Vec<(Command, String)>, Option<usize>);

/// The rows of output dropdown `key` (`DROPS`): what a pick does and its label, and which is
/// the current choice. `None`: not an output dropdown.
pub fn drop_rows(ui: &Ui, m: &Models, key: &str) -> Option<DropRows> {
    use crate::player_core::AudioOutputHost as H;
    let o = &m.playback.output;
    let on_asio = o.choice.system == H::Asio;
    Some(match key {
        "output-api" => {
            let mut rows = vec![(Command::OutSys(H::Wasapi), crate::playback::output::system_name(H::Wasapi))];
            if o.has_asio() || on_asio {
                rows.push((Command::OutSys(H::Asio), "ASIO".to_owned()));
            }
            (rows, Some(usize::from(on_asio)))
        }
        "output-device" => {
            let mut current = None;
            let rows = o
                .device_rows()
                .into_iter()
                .enumerate()
                .map(|(k, (device, route, on))| {
                    if on {
                        current = Some(k);
                    }
                    (Command::Route(route), device_label(ui, &device))
                })
                .collect();
            (rows, current)
        }
        "output-dop" => {
            let rows = [false, true]
                .map(|on| (Command::OutDop(on), ui.t(if on { "output-dop.on" } else { "output-dop.off" }, &[])));
            (rows.to_vec(), Some(usize::from(o.choice.dop)))
        }
        _ => return None,
    })
}

/// A device row's name: the system's default (naming the device it is now), or the device's.
fn device_label(ui: &Ui, device: &Device) -> String {
    match device {
        Device::Default(None) => ui.t("output.default", &[]),
        Device::Default(Some(n)) => ui.t("output.default-of", &[("device", n)]),
        Device::One(d) => d.name.clone(),
    }
}

/// The output dropdowns' boxes name the choice as it is; the interface's card shows when there
/// is a choice of them, DoP's under ASIO. Present phase, while the settings page is up.
pub(crate) fn present_drops(v: &mut Views, ui: &mut Ui, m: &Models) {
    use crate::player_core::AudioOutputHost as H;
    let o = &m.playback.output;
    let on_asio = o.choice.system == H::Asio;
    v.doc.set(ui, "output-api-on", o.has_asio() || on_asio);
    v.doc.set(ui, "output-dop-on", on_asio);
    for key in DROPS {
        let Some((rows, current)) = drop_rows(ui, m, key) else { continue };
        let label = match current.and_then(|k| rows.get(k)) {
            Some((_, l)) => l.clone(),
            // The choice kept, not listed (yet): the device by its id's route, or "reading".
            None if key == "output-device" && !o.listed => ui.t("output.loading", &[]),
            None => device_label(ui, &Device::Default(None)),
        };
        let id = v.id(&format!("{key}-drop.value"));
        if ui.tree.text_of(id) != Some(label.as_str()) {
            ui.tree.set_text(id, &label);
        }
    }
}

/// The output dropdown whose list is up, if one is.
pub(crate) fn drop_up(v: &Views) -> Option<&'static str> {
    match v.menus.open.as_ref() {
        Some(super::Kind::Drop(l)) => DROPS.into_iter().find(|k| *k == l.key),
        _ => None,
    }
}
