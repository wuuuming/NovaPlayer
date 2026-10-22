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
            Some(Command::OutDop),
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
