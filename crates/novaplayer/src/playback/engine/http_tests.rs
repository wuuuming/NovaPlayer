//! Real HTTP -> ffmpeg -> PulseAudio regression. No service account or user files.
//! Run alone with --ignored --test-threads=1 and an isolated PulseAudio null sink.

// Test code: a failed step panics.
#![allow(clippy::unwrap_used, clippy::expect_used)]
use crate::playback::engine::{Cmd, Engine, Snapshot};
use crate::player_core::{
    audio_output_available, use_null_audio_output, AudioPlayer, TrackHint, UrlAudioSourceFactory,
};
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::sync::Mutex;
use std::time::{Duration, Instant};

struct Server {
    addr: std::net::SocketAddr,
    requests: Arc<Mutex<Vec<String>>>,
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Server {
    fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let (seen, done) = (requests.clone(), stop.clone());
        let thread = std::thread::spawn(move || {
            let wav = tone();
            for stream in listener.incoming() {
                if done.load(Ordering::Relaxed) {
                    break;
                }
                let mut stream = stream.unwrap();
                stream.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
                stream.set_write_timeout(Some(Duration::from_secs(2))).unwrap();
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut first = String::new();
                if reader.read_line(&mut first).is_err() {
                    continue;
                }
                let mut range = None;
                loop {
                    let mut line = String::new();
                    if reader.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" {
                        break;
                    }
                    if let Some(v) = line.to_ascii_lowercase().strip_prefix("range: bytes=") {
                        range = v.split('-').next().and_then(|n| n.parse::<usize>().ok());
                    }
                }
                seen.lock().unwrap().push(first.clone());
                if !first.contains("/tone.wav?token=a%2Fb+cd&v=1 ") {
                    let _ =
                        stream.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
                    continue;
                }
                let start = range.unwrap_or(0).min(wav.len());
                let mut header = if range.is_some() {
                    format!(
                        "HTTP/1.1 206 Partial Content\r\nContent-Range: bytes {start}-{}/{}\r\n",
                        wav.len() - 1,
                        wav.len()
                    )
                } else {
                    "HTTP/1.1 200 OK\r\n".into()
                };
                header.push_str(&format!("Content-Type: audio/wav\r\nAccept-Ranges: bytes\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", wav.len() - start));
                let _ = stream.write_all(header.as_bytes());
                let _ = stream.write_all(&wav[start..]);
            }
        });
        Self { addr, requests, stop, thread: Some(thread) }
    }

    fn url(&self, path: &str) -> String {
        format!("http://{}{path}", self.addr)
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        let _ = TcpStream::connect(self.addr);
        let _ = self.thread.take().unwrap().join();
    }
}

/// Three seconds of stereo PCM in memory. No decoder/plugin or fixture file generates it.
fn tone() -> Vec<u8> {
    let size = 48_000u32 * 3 * 4;
    let mut wav = Vec::new();
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&(36 + size).to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    wav.extend_from_slice(&16u32.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes());
    wav.extend_from_slice(&2u16.to_le_bytes());
    wav.extend_from_slice(&48_000u32.to_le_bytes());
    wav.extend_from_slice(&192_000u32.to_le_bytes());
    wav.extend_from_slice(&4u16.to_le_bytes());
    wav.extend_from_slice(&16u16.to_le_bytes());
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&size.to_le_bytes());
    for i in 0..48_000 * 3 {
        let sample = ((i as f32 * 440.0 * std::f32::consts::TAU / 48_000.0).sin() * 1000.0) as i16;
        wav.extend_from_slice(&sample.to_le_bytes());
        wav.extend_from_slice(&sample.to_le_bytes());
    }
    wav
}

fn wait(engine: &Engine, condition: impl Fn(&Snapshot) -> bool) -> Snapshot {
    let until = Instant::now() + Duration::from_secs(8);
    loop {
        let s = engine.snapshot();
        if condition(&s) {
            return s;
        }
        assert!(Instant::now() < until, "timed out: {s:?}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
#[ignore = "needs ffmpeg/ffprobe and PulseAudio; run alone on a disposable null sink"]
fn http_playback_pause_seek_and_failure() {
    use_null_audio_output(false);
    assert!(audio_output_available(), "a real PulseAudio server is required, not clock-only playback");
    let server = Server::start();
    let url = server.url("/tone.wav?token=a%2Fb+cd&v=1");
    // Unknown duration takes the ffprobe path too. No path normalization may alter the query.
    let factory = Arc::new(UrlAudioSourceFactory::new(&url, Default::default()));
    // player-core keeps `silent`/`duration` private; sound is required above, and the engine
    // run below checks the 3 s end.
    let player = AudioPlayer::open(factory, TrackHint, Arc::new(|| {}), None).unwrap();
    drop(player);

    let engine = Engine::new(Arc::new(|| {}), Default::default(), crate::player_core::clap::Effects::new());
    engine.send(Cmd::Volume(0.05));
    engine.send(Cmd::Open {
        version: 1,
        path: PathBuf::from(&url),
        span: None,
        at: Duration::ZERO,
        play: true,
        duration: Some(Duration::from_secs(3)),
        rate: Some(48_000),
    });
    // Snapshot.position comes from samples written minus PulseAudio latency, not UI's clock.
    let s = wait(&engine, |s| s.error.is_some() || s.position > Duration::from_millis(400));
    assert!(s.error.is_none() && s.playing && !s.ended, "{s:?}");
    engine.send(Cmd::Pause);
    let held = wait(&engine, |s| !s.playing);
    std::thread::sleep(Duration::from_millis(150));
    assert_eq!(engine.snapshot().position, held.position);
    engine.send(Cmd::Seek(Duration::from_millis(2000)));
    let sought = wait(&engine, |s| s.seeks == 1);
    assert_eq!(sought.position, Duration::from_secs(2));
    engine.send(Cmd::Play);
    let end = wait(&engine, |s| s.ended || s.error.is_some());
    assert!(end.error.is_none() && end.ended && !end.playing, "{end:?}");
    assert_eq!(end.position, Duration::from_secs(3));
    assert!(server.requests.lock().unwrap().len() >= 3, "probe, play, seek must reach HTTP");

    engine.send(Cmd::Open {
        version: 2,
        path: PathBuf::from(server.url("/expired.wav")),
        span: None,
        at: Duration::ZERO,
        play: true,
        duration: Some(Duration::from_secs(3)),
        rate: Some(48_000),
    });
    let failed = wait(&engine, |s| s.version == 2 && (s.error.is_some() || s.ended));
    assert!(failed.error.is_some() && !failed.ended && !failed.playing, "HTTP 404 is failure, not EOF: {failed:?}");
    assert!(!failed.error.unwrap().contains("file not found"));
}
