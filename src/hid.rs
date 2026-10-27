//! Raw HID link to the keyboard, on QMK's raw HID interface (usage page
//! 0xFF60, usage 0x61). Protocol, 32-byte reports with byte 0 = 0xAA:
//!   board -> host  [0xAA, 0x01, layer_state as u32 little endian]  on every layer change
//!   host -> board  [0xAA, 0x01]                                    request the current state

use crate::Cmd;
use hidapi::{HidApi, HidDevice};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use winit::event_loop::EventLoopProxy;

const USAGE_PAGE: u16 = 0xFF60;
const USAGE: u16 = 0x61;
const MAGIC: u8 = 0xAA;
const LAYER_STATE: u8 = 0x01;
const SCAN_INTERVAL: Duration = Duration::from_secs(2);

/// Which board to look for: (VID, PID) from the layout, or None for "first QMK
/// raw HID device". Shared so a layout reload can retarget the running thread.
pub type Target = Arc<Mutex<Option<(u16, u16)>>>;

/// Watches for the board and forwards its layer state to the event loop.
/// Reconnects on unplug; a disconnect resets the overlay to Base.
pub fn spawn(proxy: EventLoopProxy<Cmd>, target: Target) {
    std::thread::spawn(move || {
        let mut api = match HidApi::new() {
            Ok(api) => api,
            Err(e) => return eprintln!("hid: {e}"),
        };
        let mut waiting_logged = false;
        loop {
            let usb = *target.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(dev) = open(&mut api, usb) {
                waiting_logged = false;
                println!("keyboard connected");
                // Only ask a device we positively identified; with no VID/PID the
                // match is "any QMK raw HID board", which may not speak this protocol.
                if usb.is_some() {
                    request_state(&dev);
                }
                if !listen(&dev, &proxy) {
                    return; // event loop is gone
                }
                println!("keyboard disconnected");
                // Whatever layer was held when it went away is no longer held.
                if proxy.send_event(Cmd::LayerState(0)).is_err() {
                    return;
                }
            } else if !waiting_logged {
                waiting_logged = true;
                println!("waiting for keyboard (raw HID usage page 0xFF60)...");
            }
            std::thread::sleep(SCAN_INTERVAL);
        }
    });
}

fn open(api: &mut HidApi, usb: Option<(u16, u16)>) -> Option<HidDevice> {
    api.refresh_devices().ok()?;
    let info = api.device_list().find(|d| {
        d.usage_page() == USAGE_PAGE
            && d.usage() == USAGE
            && usb.is_none_or(|(vid, pid)| d.vendor_id() == vid && d.product_id() == pid)
    })?;
    info.open_device(api).map_err(|e| eprintln!("hid open: {e}")).ok()
}

fn request_state(dev: &HidDevice) {
    // Leading 0 is the report ID hidapi expects on write; then the 32-byte report.
    let mut request = [0u8; 33];
    request[1] = MAGIC;
    request[2] = LAYER_STATE;
    if let Err(e) = dev.write(&request) {
        eprintln!("hid write: {e}");
    }
}

/// Reads until the device goes away. Returns false if the event loop has exited.
/// Repeated states are dropped and changes are forwarded at most once per
/// MIN_INTERVAL (latest wins), so a misbehaving device can't flood the UI.
fn listen(dev: &HidDevice, proxy: &EventLoopProxy<Cmd>) -> bool {
    let mut buf = [0u8; 32];
    let mut throttle = Throttle::default();
    loop {
        let timeout = throttle.wait(Instant::now()).map_or(1000, |d| d.as_millis().max(1) as i32);
        match dev.read_timeout(&mut buf, timeout) {
            Ok(n) => {
                if let Some(state) = parse_report(&buf[..n.min(buf.len())]) {
                    throttle.offer(state);
                }
            }
            Err(_) => return true,
        }
        if let Some(state) = throttle.take(Instant::now()) {
            if proxy.send_event(Cmd::LayerState(state)).is_err() {
                return false;
            }
        }
    }
}

/// Forwarding interval cap: ~1 frame. Real layer changes are far slower.
const MIN_INTERVAL: Duration = Duration::from_millis(8);

/// Dedupes and rate-limits layer states coming off the device.
#[derive(Default)]
struct Throttle {
    sent: Option<u32>,
    pending: Option<u32>,
    last_send: Option<Instant>,
}

impl Throttle {
    fn offer(&mut self, state: u32) {
        self.pending = (Some(state) != self.sent).then_some(state);
    }

    /// The state to forward now, if any.
    fn take(&mut self, now: Instant) -> Option<u32> {
        if self.wait(now).is_some_and(|d| !d.is_zero()) {
            return None;
        }
        let state = self.pending.take()?;
        self.sent = Some(state);
        self.last_send = Some(now);
        Some(state)
    }

    /// How long until a pending state may be sent (None = nothing pending).
    fn wait(&self, now: Instant) -> Option<Duration> {
        self.pending?;
        let ready = self.last_send.map_or(now, |t| t + MIN_INTERVAL);
        Some(ready.saturating_duration_since(now))
    }
}

/// The layer_state carried by one of our reports; anything else is ignored.
fn parse_report(report: &[u8]) -> Option<u32> {
    match report {
        [MAGIC, LAYER_STATE, a, b, c, d, ..] => Some(u32::from_le_bytes([*a, *b, *c, *d])),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn throttle_dedupes_and_rate_limits() {
        let t0 = Instant::now();
        let mut t = Throttle::default();
        t.offer(0b10);
        assert_eq!(t.take(t0), Some(0b10), "first change goes straight through");
        t.offer(0b10);
        assert_eq!(t.take(t0 + MIN_INTERVAL * 5), None, "repeat dropped");
        assert_eq!(t.wait(t0), None, "nothing pending");
        t.offer(0b100);
        t.offer(0b1000); // flood inside one interval: latest wins
        assert_eq!(t.take(t0 + Duration::from_millis(1)), None, "too soon");
        assert!(t.wait(t0 + Duration::from_millis(1)).is_some_and(|d| d > Duration::ZERO));
        assert_eq!(t.take(t0 + MIN_INTERVAL), Some(0b1000));
        t.offer(0b1000);
        t.offer(0b10);
        t.offer(0b1000); // flipped back before sending: nothing to send
        assert_eq!(t.take(t0 + MIN_INTERVAL * 3), None);
    }

    #[test]
    fn reports() {
        assert_eq!(parse_report(&[0xAA, 0x01, 0b100, 0, 0, 0]), Some(0b100));
        assert_eq!(parse_report(&[0xAA, 0x01, 0x78, 0x56, 0x34, 0x12, 9, 9]), Some(0x1234_5678));
        assert_eq!(parse_report(&[0xAA, 0x01, 1, 0, 0]), None, "too short");
        assert_eq!(parse_report(&[0xAB, 0x01, 1, 0, 0, 0]), None, "wrong magic");
        assert_eq!(parse_report(&[0xAA, 0x02, 1, 0, 0, 0]), None, "unknown type");
        assert_eq!(parse_report(&[]), None);
    }
}
