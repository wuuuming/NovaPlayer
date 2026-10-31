//! Where the sound goes (b7b33f5's `output`): the audio system, then its device, chosen from the
//! volume button's secondary press (6.6, 15.x), kept between runs in
//! `%LOCALAPPDATA%\NovaPlayer\output.txt` (`system wasapi|asio`, `wasapi <endpoint id>`,
//! `asio <driver name>`, `dop 0|1`).

use crate::player_core::{AudioOutputDeviceInfo, AudioOutputHost, AudioOutputRoute, AudioOutputSystem};
use std::path::PathBuf;

/// The system, and in each system the device last chosen in it: going back to a system goes back
/// to its device.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OutputChoice {
    pub system: AudioOutputHost,
    /// The WASAPI endpoint by id; None for Windows' default output.
    pub wasapi: Option<String>,
    /// The ASIO driver by its registered name.
    pub asio: Option<String>,
    /// DSD songs may go to the ASIO driver as DoP.
    pub dop: bool,
}

impl Default for OutputChoice {
    fn default() -> Self {
        OutputChoice { system: AudioOutputHost::Wasapi, wasapi: None, asio: None, dop: false }
    }
}

impl OutputChoice {
    pub fn route(&self) -> AudioOutputRoute {
        match (self.system, &self.wasapi, &self.asio) {
            (AudioOutputHost::Wasapi, Some(id), _) => AudioOutputRoute::Wasapi(id.clone()),
            (AudioOutputHost::Asio, _, Some(d)) => AudioOutputRoute::Asio(d.clone()),
            _ => AudioOutputRoute::WasapiDefault,
        }
    }

    pub fn parse(text: &str) -> Self {
        let mut c = Self::default();
        for line in text.lines() {
            let (k, v) = line.split_once(' ').unwrap_or((line, ""));
            let v = v.trim();
            match k.trim() {
                "system" => c.system = if v == "asio" { AudioOutputHost::Asio } else { AudioOutputHost::Wasapi },
                "wasapi" if !v.is_empty() => c.wasapi = Some(v.to_owned()),
                "asio" if !v.is_empty() => c.asio = Some(v.to_owned()),
                "dop" => c.dop = v == "1",
                _ => {}
            }
        }
        c
    }

    pub fn text(&self) -> String {
        let mut t = format!("system {}\n", if self.system == AudioOutputHost::Asio { "asio" } else { "wasapi" });
        if let Some(id) = &self.wasapi {
            t.push_str(&format!("wasapi {id}\n"));
        }
        if let Some(d) = &self.asio {
            t.push_str(&format!("asio {d}\n"));
        }
        t.push_str(if self.dop { "dop 1\n" } else { "dop 0\n" });
        t
    }
}

pub struct Output {
    pub choice: OutputChoice,
    path: Option<PathBuf>,
    /// The systems as last listed (the flyout shows these while it lists again).
    pub systems: Vec<AudioOutputSystem>,
    /// They were listed at least once (`Player::list_outputs`).
    pub listed: bool,
    /// The flyout or a settings dropdown (`views::menus::output::DROPS`) was asked for before
    /// the first list landed: it opens when it does.
    pub asked: Option<Asked>,
    /// The route the outputs were sent to: the core's answer, set where the choice changes (a
    /// kept ASIO choice in a build without ASIO goes to WASAPI's default).
    pub route: AudioOutputRoute,
}

impl Output {
    /// Reads the kept choice and sends every output there, before the first song opens. A kept
    /// ASIO choice in a build without ASIO plays through WASAPI; the file keeps it.
    pub fn install() -> (Self, Option<String>) {
        let path = crate::data_dir().map(|d| d.join("output.txt"));
        let choice = path
            .as_deref()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .map(|t| OutputChoice::parse(&t))
            .unwrap_or_default();
        let mut note = None;
        let mut route = apply(&choice);
        if route != choice.route() {
            let mut c = choice.clone();
            c.system = AudioOutputHost::Wasapi;
            route = apply(&c);
            note = Some(novaui::i18n::t("output.no-asio", &[("host", &system_name(AudioOutputHost::Wasapi))]));
        }
        (Output { choice, path, systems: Vec::new(), listed: false, asked: None, route }, note)
    }

    /// The outputs go to an ASIO driver (the volume is the driver's then).
    pub fn asio(&self) -> bool {
        matches!(self.route, AudioOutputRoute::Asio(_))
    }

    pub fn has_asio(&self) -> bool {
        cfg!(feature = "asio") && self.systems.iter().any(|s| s.host == AudioOutputHost::Asio && !s.devices.is_empty())
    }

    fn devices(&self, host: AudioOutputHost) -> &[AudioOutputDeviceInfo] {
        self.systems.iter().find(|s| s.host == host).map_or(&[][..], |s| &s.devices[..])
    }

    /// Changes the choice and sends the outputs there; false: it was so already. Keeping it is
    /// the caller's (`Player::choose_output`).
    fn choose(&mut self, f: impl FnOnce(&mut OutputChoice)) -> bool {
        let mut c = self.choice.clone();
        f(&mut c);
        if c == self.choice {
            return false;
        }
        self.route = apply(&c);
        self.choice = c;
        true
    }

    /// The file the choice is kept in.
    pub fn file(&self) -> Option<&std::path::Path> {
        self.path.as_deref()
    }

    pub fn choose_route(&mut self, route: AudioOutputRoute) -> bool {
        self.choose(|c| match route {
            AudioOutputRoute::WasapiDefault => {
                c.system = AudioOutputHost::Wasapi;
                c.wasapi = None;
            }
            AudioOutputRoute::Wasapi(id) => {
                c.system = AudioOutputHost::Wasapi;
                c.wasapi = Some(id);
            }
            AudioOutputRoute::Asio(d) => {
                c.system = AudioOutputHost::Asio;
                c.asio = Some(d);
            }
        })
    }

    /// The system's row: back to that system with the device it last had in it, if still
    /// registered; else the first sound card's own driver; else the first.
    pub fn choose_system(&mut self, host: AudioOutputHost) -> bool {
        if host == self.choice.system {
            return false;
        }
        let drivers = self.devices(AudioOutputHost::Asio).to_vec();
        let driver = self
            .choice
            .asio
            .clone()
            .filter(|n| drivers.is_empty() || drivers.iter().any(|d| &d.id == n))
            .or_else(|| drivers.iter().find(|d| !d.generic).or(drivers.first()).map(|d| d.id.clone()));
        self.choose(|c| {
            c.system = host;
            if host == AudioOutputHost::Asio {
                c.asio = driver;
            }
        })
    }

    pub fn set_dop(&mut self, on: bool) -> bool {
        self.choose(|c| c.dop = on)
    }

    /// The device menu's rows: (device, route, checked). WASAPI: the system default first,
    /// then each endpoint; ASIO: each driver. The words are the menu's.
    pub fn device_rows(&self) -> Vec<(Device, AudioOutputRoute, bool)> {
        let route = self.choice.route();
        let mut rows = Vec::new();
        match self.choice.system {
            AudioOutputHost::Wasapi => {
                let ds = self.devices(AudioOutputHost::Wasapi);
                let def = ds.iter().find(|d| d.is_default).map(|d| d.name.clone());
                rows.push((
                    Device::Default(def),
                    AudioOutputRoute::WasapiDefault,
                    route == AudioOutputRoute::WasapiDefault,
                ));
                for d in ds {
                    let r = AudioOutputRoute::Wasapi(d.id.clone());
                    rows.push((Device::One(d.clone()), r.clone(), r == route));
                }
            }
            AudioOutputHost::Asio => {
                for d in self.devices(AudioOutputHost::Asio) {
                    let r = AudioOutputRoute::Asio(d.id.clone());
                    rows.push((Device::One(d.clone()), r.clone(), r == route));
                }
            }
        }
        rows
    }
}

/// A row of the device menu.
/// What waits for the outputs to be listed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Asked {
    Flyout,
    Drop(&'static str),
}

pub enum Device {
    /// The system's default output; the device it is now, when known.
    Default(Option<String>),
    One(AudioOutputDeviceInfo),
}

/// Sends the outputs to `c`; answers the route the core took.
fn apply(c: &OutputChoice) -> AudioOutputRoute {
    crate::player_core::set_dop_output(c.dop);
    crate::player_core::set_output_route(c.route())
}

pub fn system_name(h: AudioOutputHost) -> String {
    match h {
        // Linux's shim keeps Windows' host names; its one system is the PulseAudio server.
        AudioOutputHost::Wasapi if cfg!(windows) => novaui::i18n::t("output.wasapi", &[]),
        AudioOutputHost::Wasapi => "PulseAudio".into(),
        AudioOutputHost::Asio => "ASIO".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn choice_round_trips_and_routes() {
        let c = OutputChoice {
            system: AudioOutputHost::Asio,
            wasapi: Some("{a}".into()),
            asio: Some("Card".into()),
            dop: true,
        };
        assert_eq!(OutputChoice::parse(&c.text()), c);
        assert_eq!(c.route(), AudioOutputRoute::Asio("Card".into()));
        let c = OutputChoice { asio: None, ..c };
        assert_eq!(c.route(), AudioOutputRoute::WasapiDefault);
        assert_eq!(OutputChoice::parse("junk\nsystem wasapi"), OutputChoice::default());
    }
}
