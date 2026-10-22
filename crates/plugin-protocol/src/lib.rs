//! Host/service-independent JSON-lines protocol primitives.
pub mod json;

/// The protocol version (`docs/plugins.md`): what a plugin's `hello` answers and its manifest's
/// `api` asks for. A plugin asking for another is listed, not run.
pub const API: u32 = 3;

/// Maximum UTF-8 JSON line size, excluding newline.
pub const MAX_LINE_BYTES: usize = 16 * 1024 * 1024;

use std::time::Duration;

/// How long the host waits for the answer to `call` before it gives the call up and stops the
/// plugin: the one table the host and the plugins read (plugins.md).
pub fn timeout(call: &str) -> Duration {
    Duration::from_secs(match call {
        "playlist" => 180,
        "thumbs" => 60,
        "open" => 10,
        "read" | "play_event" | "connect.cancel" => 5,
        _ => 20,
    })
}

/// How long a plugin gives itself for `call`, its fetches and fallbacks included: the host's
/// `timeout` less time to write the answer (a tenth of it, five seconds at most).
pub fn budget(call: &str) -> Duration {
    let timeout = timeout(call);
    timeout - (timeout / 10).min(Duration::from_secs(5))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_plugin_answers_inside_the_hosts_wait() {
        for call in ["playlist", "thumbs", "open", "read", "play_event", "connect.cancel", "resolve", "metadata"] {
            assert!(budget(call) < timeout(call) && !budget(call).is_zero(), "{call}");
        }
        assert_eq!((budget("playlist"), budget("resolve")), (Duration::from_secs(175), Duration::from_secs(18)));
    }
}
