//! Build-time configuration, baked in from environment variables so a
//! development build needs no provisioning step (design brief §14).
//!
//! ```text
//! WIFI_SSID=home WIFI_PASSWORD=secret SCREEN_URL=http://192.168.1.20:8080/screen \
//!     cargo run --release
//! ```
//!
//! Unset variables fall back to obvious placeholders so CI can compile the
//! crate without secrets; a device flashed with the placeholders logs a
//! warning and fails its fetch cycle harmlessly.

/// WiFi network name (`WIFI_SSID`).
pub const WIFI_SSID: &str = match option_env!("WIFI_SSID") {
    Some(v) => v,
    None => "CHANGE-ME",
};

/// WiFi passphrase (`WIFI_PASSWORD`); WPA2-PSK.
pub const WIFI_PASSWORD: &str = match option_env!("WIFI_PASSWORD") {
    Some(v) => v,
    None => "CHANGE-ME",
};

/// The `GET /screen` URL of the LAN server (`SCREEN_URL`). Plain HTTP only.
pub const SCREEN_URL: &str = match option_env!("SCREEN_URL") {
    Some(v) => v,
    None => "http://192.168.1.2:8080/screen",
};

/// Seconds between timer wake-ups when everything is healthy
/// (`POLL_INTERVAL_SECS`, default 15 minutes).
pub const POLL_INTERVAL_SECS: u64 = match option_env!("POLL_INTERVAL_SECS") {
    Some(v) => parse_u64(v),
    None => 15 * 60,
};

/// Seconds between wake-ups while fetches are failing.
pub const RETRY_INTERVAL_SECS: u64 = 120;

/// After this many consecutive failed cycles the panel shows an error
/// screen once (until then the last good image stays up).
pub const FAILURES_BEFORE_ERROR_SCREEN: u32 = 3;

/// Whether the build carries the placeholder credentials.
pub const fn is_placeholder_config() -> bool {
    const_eq(WIFI_SSID, "CHANGE-ME")
}

const fn const_eq(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    let mut i = 0;
    while i < a.len() {
        if a[i] != b[i] {
            return false;
        }
        i += 1;
    }
    true
}

/// Decimal parse usable in a `const` (no `FromStr` there). Panics at
/// compile time on a malformed value, which is the behaviour we want.
const fn parse_u64(s: &str) -> u64 {
    let bytes = s.as_bytes();
    assert!(
        !bytes.is_empty(),
        "POLL_INTERVAL_SECS must be a decimal number"
    );
    let mut n: u64 = 0;
    let mut i = 0;
    while i < bytes.len() {
        let d = bytes[i];
        assert!(
            d.is_ascii_digit(),
            "POLL_INTERVAL_SECS must be a decimal number"
        );
        n = n * 10 + (d - b'0') as u64;
        i += 1;
    }
    assert!(n >= 30, "POLL_INTERVAL_SECS must be at least 30");
    n
}
