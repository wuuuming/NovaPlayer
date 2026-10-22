//! Sound from a decode plugin, as the playback cores take sound.
//!
//! A plugin answers `open` (rate, channels, length) and `read` (PCM s16le from any frame). That is
//! a seekable PCM stream, player-core's `PcmSource`; its `PcmAudioSourceFactory` plays it on both
//! platforms (Windows reads it as a WAV file, Linux feeds it to ffmpeg).

use crate::plugins::host::Host;
use crate::sync::Locked;
use novaplayer_plugin_protocol::json::Value;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

/// The process's plugin host and which plugin decodes which extension (kept in step with the
/// enabled plugins by `sync`).
pub fn host() -> &'static Host {
    static H: OnceLock<Host> = OnceLock::new();
    H.get_or_init(Host::default)
}

static DECODERS: Mutex<Vec<(String, String)>> = Mutex::new(Vec::new());
/// URI scheme → the plugin that resolves it.
static SCHEMES: Mutex<Vec<(String, String)>> = Mutex::new(Vec::new());

/// Registers the enabled plugins with the host and records their decode extensions and URI
/// schemes.
pub fn sync(reg: &crate::plugins::registry::Registry) {
    let h = host();
    let mut dec = Vec::new();
    let mut sch = Vec::new();
    for p in &reg.list {
        let id = p.id();
        match (&p.manifest, &p.state) {
            (Some(m), crate::plugins::registry::State::On) => {
                if !h.registered(&id) {
                    h.register(&id, p.dir.clone(), p.dir.join(&m.exec), m.concurrent);
                }
                for e in &m.provides.decode {
                    dec.push((e.clone(), id.clone()));
                }
                for s in &m.provides.schemes {
                    sch.push((s.clone(), id.clone()));
                }
            }
            _ => h.unregister(&id),
        }
    }
    *DECODERS.locked() = dec;
    *SCHEMES.locked() = sch;
}

/// Stops every running plugin and registers the enabled ones afresh (重新加载插件): a plugin
/// whose program was replaced runs the new one on its next call.
pub fn reload(reg: &crate::plugins::registry::Registry) {
    let h = host();
    for p in &reg.list {
        h.unregister(&p.id());
    }
    sync(reg);
}

/// The plugin that resolves URIs of `scheme`, if an enabled one declares it.
pub fn resolver_for(scheme: &str) -> Option<String> {
    let scheme = scheme.to_ascii_lowercase();
    #[cfg(any(test, feature = "test-support"))]
    if let Some(id) = TEST_SCHEMES.locked().iter().find(|(s, _)| *s == scheme).map(|(_, id)| id.clone()) {
        return Some(id);
    }
    SCHEMES.locked().iter().find(|(s, _)| *s == scheme).map(|(_, id)| id.clone())
}

/// Schemes the tests register by hand: kept apart from `sync`, which other tests run.
#[cfg(any(test, feature = "test-support"))]
static TEST_SCHEMES: Mutex<Vec<(String, String)>> = Mutex::new(Vec::new());

#[cfg(any(test, feature = "test-support"))]
pub fn add_scheme_for_test(scheme: &str, id: &str) {
    let mut s = TEST_SCHEMES.locked();
    if !s.iter().any(|(a, _)| a == scheme) {
        s.push((scheme.to_owned(), id.to_owned()));
    }
}

/// The extensions the enabled plugins decode.
pub fn decoded_extensions() -> Vec<String> {
    DECODERS.locked().iter().map(|(e, _)| e.clone()).collect()
}

/// The plugin that decodes `path`, if an enabled one claims its extension.
pub fn decoder_for(path: &Path) -> Option<String> {
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    DECODERS.locked().iter().find(|(e, _)| *e == ext).map(|(_, id)| id.clone())
}

#[derive(Clone, Debug)]
pub struct PcmInfo {
    pub rate: u32,
    pub channels: u16,
    /// Total frames, when the plugin knows.
    pub frames: Option<u64>,
    /// What the plugin says of the song; its length is the frames'.
    pub tags: crate::library::Tags,
}

/// A file a plugin decodes.
#[derive(Clone)]
pub struct PcmSource {
    pub plugin: String,
    pub path: PathBuf,
    pub info: PcmInfo,
}

impl PcmSource {
    pub fn open(plugin: &str, path: &Path) -> Result<PcmSource, String> {
        let r = host().ask(plugin, "open", &[("path", Value::Str(path.to_string_lossy().into()))])?;
        let rate = r
            .num("rate")
            .filter(|r| *r >= 1000.0 && *r <= 768_000.0)
            .ok_or_else(|| novaui::i18n::t("err.pcm-no-rate", &[]))? as u32;
        let channels = r
            .num("channels")
            .filter(|c| (1.0..=8.0).contains(c))
            .ok_or_else(|| novaui::i18n::t("err.pcm-no-channels", &[]))? as u16;
        let frames = r.num("frames").filter(|f| *f > 0.0).map(|f| f as u64);
        let tags = crate::library::Tags {
            duration: frames.map(|f| Duration::from_secs_f64(f as f64 / f64::from(rate))),
            ..r.tags()
        };
        let info = PcmInfo { rate, channels, frames, tags };
        Ok(PcmSource { plugin: plugin.to_owned(), path: path.to_owned(), info })
    }

    pub fn frame_bytes(&self) -> u64 {
        u64::from(self.info.channels) * 2
    }

    /// PCM from frame `at`, up to `frames` frames (empty at the end).
    pub fn read(&self, at: u64, frames: u64) -> Result<Vec<u8>, String> {
        let r = host().ask(
            &self.plugin,
            "read",
            &[
                ("path", Value::Str(self.path.to_string_lossy().into())),
                ("at", Value::Num(at as f64)),
                ("frames", Value::Num(frames as f64)),
            ],
        )?;
        let mut b = r.bytes;
        b.truncate((b.len() as u64 / self.frame_bytes() * self.frame_bytes()) as usize);
        Ok(b)
    }
}

impl crate::player_core::PcmSource for PcmSource {
    fn sample_rate(&self) -> u32 {
        self.info.rate
    }
    fn channels(&self) -> u16 {
        self.info.channels
    }
    fn duration(&self) -> Option<Duration> {
        self.info.tags.duration
    }
    fn frames(&self) -> Option<u64> {
        self.info.frames
    }
    fn read_cancellable(&self, at: u64, frames: u64, cancelled: &AtomicBool) -> Result<Vec<u8>, String> {
        // One plugin call cannot be interrupted; a cancelled reader stops before the next.
        if cancelled.load(Ordering::Acquire) {
            return Err("cancelled".into());
        }
        self.read(at, frames)
    }
    fn label(&self) -> String {
        format!("plugin:{}", self.plugin)
    }
}

/// The factory a plugin's file plays through.
pub fn factory(src: PcmSource) -> std::sync::Arc<dyn crate::player_core::AudioSourceFactory> {
    std::sync::Arc::new(crate::player_core::PcmAudioSourceFactory::new(std::sync::Arc::new(src)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn demo_source() -> (PcmSource, PathBuf) {
        let p = crate::plugins::host::demo_plugin();
        if !host().registered("demo") {
            host().register("demo", p.parent().unwrap().to_path_buf(), p, true);
        }
        let tone = std::env::temp_dir().join(format!("np-src-{}.tone", std::process::id()));
        std::fs::write(&tone, "1000 2").unwrap();
        (PcmSource::open("demo", &tone).unwrap(), tone)
    }

    #[test]
    fn the_plugins_pcm_reads_as_a_wav_file() {
        let (src, tone) = demo_source();
        assert_eq!((src.info.rate, src.info.channels, src.info.frames), (48000, 2, Some(96000)));
        let mut wav = factory(src.clone()).open().unwrap();
        assert_eq!(wav.size(), Some(44 + 96000 * 4));
        let mut head = [0u8; 44];
        let mut got = 0;
        while got < 44 {
            got += wav.read(got as u64, &mut head[got..]).unwrap();
        }
        assert_eq!((&head[..4], &head[36..40]), (&b"RIFF"[..], &b"data"[..]));
        // A seek is a read elsewhere: frame 50 000 matches what `read` says for it.
        let mut b = [0u8; 8];
        assert_eq!(wav.read(44 + 50_000 * 4, &mut b).unwrap(), 8);
        assert_eq!(&b[..], &src.read(50_000, 2).unwrap()[..]);
        // Past the end: nothing.
        assert_eq!(wav.read(44 + 96_000 * 4, &mut b).unwrap(), 0);
        let _ = std::fs::remove_file(tone);
    }
}

#[cfg(test)]
mod play_tests {
    use super::*;
    use crate::player_core::AudioPlayer;
    use std::sync::Arc;
    use std::time::Instant;

    /// A `.tone` file played end to end by the platform's core on the null output: it opens with
    /// the plugin's length, a seek lands, playing advances, and it reaches the end.
    #[test]
    fn a_plugin_file_plays_through_the_core() {
        crate::player_core::use_null_audio_output(true);
        let p = crate::plugins::host::demo_plugin();
        if !host().registered("demo") {
            host().register("demo", p.parent().unwrap().to_path_buf(), p, true);
        }
        let tone = std::env::temp_dir().join(format!("np-play-{}.tone", std::process::id()));
        std::fs::write(&tone, "660 3").unwrap();
        let src = PcmSource::open("demo", &tone).unwrap();
        let d = src.info.tags.duration;
        assert_eq!(d, Some(Duration::from_secs(3)));
        let f = factory(src);
        let options =
            crate::player_core::AudioOpenOptions { duration: d, source_rate: Some(48000), ..Default::default() };
        let mut pl = AudioPlayer::open_with(f, Arc::new(|| {}), options).unwrap();
        pl.seek(Duration::from_millis(1500)).unwrap();
        let _ = pl.play();
        let t = Instant::now();
        let mut ended = false;
        while t.elapsed() < Duration::from_secs(8) {
            let _ = pl.service(Instant::now());
            if pl.ended() {
                ended = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        let took = t.elapsed();
        let _ = pl.dispose();
        let _ = std::fs::remove_file(&tone);
        assert!(ended, "a plugin-decoded song did not reach its end");
        // From 1.5 s of 3 s: about 1.5 s of playing, not the whole file and not instantly.
        assert!(took >= Duration::from_millis(1200) && took < Duration::from_millis(4000), "{took:?}");
    }
}
