//! System tray icon (StatusNotifierItem, shown by KDE Plasma) with the battery level.
//!
//! A background thread asks the mouse for `GetBatteryLevel` every [`QUERY_INTERVAL`] and
//! pushes the result to the tray. The icon is drawn in code: the percentage in white on
//! a square colored by level (blue while charging, grey while asleep or disconnected).
//!
//! It can run next to the TUI: hidraw delivers every reply to every reader, and only the
//! reply matching the request in flight is used.

use hidapi::{HidApi, HidDevice};
use ksni::blocking::TrayMethods;
use ksni::menu::StandardItem;
use ksni::{Category, Icon, MenuItem, ToolTip, Tray};
use openvxe::device;
use openvxe::protocol::{self, Packet, Report};
use std::process::Command;
use std::thread;
use std::time::{Duration, Instant};

/// Wait between battery queries while the mouse answers.
const QUERY_INTERVAL: Duration = Duration::from_secs(30);
/// Wait between battery queries while the mouse is asleep.
const ASLEEP_INTERVAL: Duration = Duration::from_secs(5);
const REPLY_TIMEOUT: Duration = Duration::from_millis(300);
/// Wait between scans for the dongle while it is not connected.
const RESCAN_INTERVAL: Duration = Duration::from_secs(2);
/// Unanswered queries after which the mouse is considered asleep.
const MISSES_BEFORE_ASLEEP: u8 = 2;
const POLL_INTERVAL: Duration = Duration::from_millis(50);
/// Below this level the icon turns red and a notification is sent.
const LOW_PERCENT: u8 = 20;
/// The low battery notification is sent again only after the level gets back here.
const REARM_PERCENT: u8 = 25;
/// Above this level the icon is green, otherwise orange.
const GOOD_PERCENT: u8 = 50;

#[derive(Clone, Copy, Debug, PartialEq)]
enum Status {
    Disconnected,
    /// The dongle is there but the mouse does not answer; last known level and charging.
    Asleep(Option<(u8, bool)>),
    Online(u8, bool),
}

struct BatteryTray {
    status: Status,
}

impl Tray for BatteryTray {
    const MENU_ON_ACTIVATE: bool = true;

    fn id(&self) -> String {
        "openvxe-tray".into()
    }

    fn title(&self) -> String {
        "OpenVXE".into()
    }

    fn category(&self) -> Category {
        Category::Hardware
    }

    fn status(&self) -> ksni::Status {
        match self.status {
            // Plasma moves passive items to the hidden part of the tray.
            Status::Disconnected => ksni::Status::Passive,
            _ => ksni::Status::Active,
        }
    }

    fn icon_pixmap(&self) -> Vec<Icon> {
        let small = draw_icon(self.status);
        let large = upscale(&small, 2);
        vec![small, large]
    }

    fn tool_tip(&self) -> ToolTip {
        ToolTip {
            title: "VXE R1 SE+".into(),
            description: describe(self.status),
            ..Default::default()
        }
    }

    fn menu(&self) -> Vec<MenuItem<Self>> {
        vec![
            StandardItem {
                label: describe(self.status),
                enabled: false,
                ..Default::default()
            }
            .into(),
            MenuItem::Separator,
            StandardItem {
                label: "Open OpenVXE".into(),
                icon_name: "input-mouse".into(),
                activate: Box::new(|_| open_tui()),
                ..Default::default()
            }
            .into(),
            StandardItem {
                label: "Quit".into(),
                icon_name: "application-exit".into(),
                activate: Box::new(|_| std::process::exit(0)),
                ..Default::default()
            }
            .into(),
        ]
    }
}

fn describe(status: Status) -> String {
    match status {
        Status::Disconnected => "Dongle not connected".into(),
        Status::Asleep(None) => "Mouse asleep".into(),
        Status::Asleep(Some((percent, _))) => format!("Mouse asleep (last reading {}%)", percent),
        Status::Online(percent, true) => format!("Battery {}%, charging", percent),
        Status::Online(percent, false) => format!("Battery {}%", percent),
    }
}

/// Run the TUI in a terminal: Konsole, else `$TERMINAL`, else xterm.
fn open_tui() {
    let mut terminals = vec!["konsole".to_string()];
    terminals.extend(std::env::var("TERMINAL").ok());
    terminals.push("xterm".into());
    for terminal in terminals {
        if let Ok(mut child) = Command::new(&terminal).args(["-e", "openvxe"]).spawn() {
            // Reap the terminal when it closes, so it does not linger as a zombie.
            thread::spawn(move || child.wait());
            return;
        }
    }
    eprintln!("openvxe-tray: no terminal found to run openvxe");
}

/// Sends one notification each time the level drops below [`LOW_PERCENT`].
struct LowBattery {
    armed: bool,
}

impl LowBattery {
    /// Returns true when a notification should be sent.
    fn update(&mut self, percent: u8, charging: bool) -> bool {
        if charging || percent >= REARM_PERCENT {
            self.armed = true;
        } else if percent < LOW_PERCENT && self.armed {
            self.armed = false;
            return true;
        }
        false
    }
}

fn notify_low(percent: u8) {
    let res = notify_rust::Notification::new()
        .appname("OpenVXE")
        .summary("Mouse battery low")
        .body(&format!("The VXE R1 SE+ is at {}%. Charge it soon.", percent))
        .icon("battery-caution")
        .show();
    if let Err(err) = res {
        eprintln!("openvxe-tray: cannot send notification: {}", err);
    }
}

/// Query the battery forever, calling `publish` every time the status changes.
fn poll_battery(mut publish: impl FnMut(Status)) {
    let mut api = match HidApi::new() {
        Ok(api) => api,
        Err(err) => {
            eprintln!("openvxe-tray: cannot initialize HID: {}", err);
            std::process::exit(1);
        }
    };
    let request: Packet = protocol::battery_request();
    let mut devices: Vec<HidDevice> = Vec::new();
    let mut status = Status::Disconnected;
    let mut last_scan: Option<Instant> = None;
    let mut in_flight: Option<Instant> = None;
    let mut next_query = Instant::now();
    let mut misses = 0u8;
    let mut low = LowBattery { armed: true };

    let mut set = |new: Status, status: &mut Status| {
        if new != *status {
            *status = new;
            if let Status::Online(percent, charging) = new {
                if low.update(percent, charging) {
                    notify_low(percent);
                }
            }
            publish(new);
        }
    };

    loop {
        thread::sleep(POLL_INTERVAL);

        if devices.is_empty() {
            if last_scan.is_none_or(|t| t.elapsed() >= RESCAN_INTERVAL) {
                last_scan = Some(Instant::now());
                if api.refresh_devices().is_ok() {
                    if let Ok(opened) = device::open_devices(&api) {
                        devices = opened;
                        in_flight = None;
                        misses = 0;
                        next_query = Instant::now();
                    }
                }
            }
            continue;
        }

        let reports = match device::read_all(&devices) {
            Ok(reports) => reports,
            Err(_) => {
                devices.clear();
                set(Status::Disconnected, &mut status);
                continue;
            }
        };
        let last_known = match status {
            Status::Online(percent, charging) => Some((percent, charging)),
            Status::Asleep(last) => last,
            Status::Disconnected => None,
        };
        let mut missed = false;
        for report in reports {
            if in_flight.is_none() || !protocol::is_reply_to(&request, &report) {
                continue;
            }
            in_flight = None;
            match protocol::parse_report(&report) {
                Report::Battery { percent, charging } => {
                    misses = 0;
                    set(Status::Online(percent, charging), &mut status);
                }
                // Rejected: the mouse is not linked to the dongle (off or asleep).
                _ => missed = true,
            }
        }
        if in_flight.is_some_and(|sent| sent.elapsed() >= REPLY_TIMEOUT) {
            in_flight = None;
            missed = true;
        }
        if missed {
            misses = misses.saturating_add(1);
            if misses >= MISSES_BEFORE_ASLEEP {
                set(Status::Asleep(last_known), &mut status);
                next_query = Instant::now() + ASLEEP_INTERVAL;
            } else {
                next_query = Instant::now();
            }
        }

        if in_flight.is_none() && Instant::now() >= next_query {
            if device::send(&devices, &request).is_err() {
                devices.clear();
                set(Status::Disconnected, &mut status);
                continue;
            }
            in_flight = Some(Instant::now());
            next_query = Instant::now() + QUERY_INTERVAL;
        }
    }
}

fn main() {
    let tray = BatteryTray {
        status: Status::Disconnected,
    };
    // Autostart can run before plasmashell registers the tray host: wait for it.
    let handle = match tray.assume_sni_available(true).spawn() {
        Ok(handle) => handle,
        Err(err) => {
            eprintln!("openvxe-tray: cannot create the tray icon: {}", err);
            std::process::exit(1);
        }
    };
    poll_battery(move |status| {
        handle.update(|tray| tray.status = status);
    });
}

// --- Icon drawing ---

const ICON_SIZE: usize = 32;
const CORNER_RADIUS: usize = 6;
const WHITE: [u8; 3] = [0xff, 0xff, 0xff];
const RED: [u8; 3] = [0xda, 0x44, 0x53];
const ORANGE: [u8; 3] = [0xf6, 0x74, 0x00];
const GREEN: [u8; 3] = [0x27, 0xae, 0x60];
const BLUE: [u8; 3] = [0x3d, 0xae, 0xe9];
const GREY: [u8; 3] = [0x7f, 0x8c, 0x8d];

/// 3x5 glyphs, one row per entry, most significant of the 3 bits on the left.
fn glyph(c: char) -> [u8; 5] {
    match c {
        '0' => [0b111, 0b101, 0b101, 0b101, 0b111],
        '1' => [0b010, 0b110, 0b010, 0b010, 0b111],
        '2' => [0b111, 0b001, 0b111, 0b100, 0b111],
        '3' => [0b111, 0b001, 0b111, 0b001, 0b111],
        '4' => [0b101, 0b101, 0b111, 0b001, 0b001],
        '5' => [0b111, 0b100, 0b111, 0b001, 0b111],
        '6' => [0b111, 0b100, 0b111, 0b101, 0b111],
        '7' => [0b111, 0b001, 0b001, 0b001, 0b001],
        '8' => [0b111, 0b101, 0b111, 0b101, 0b111],
        '9' => [0b111, 0b101, 0b111, 0b001, 0b111],
        _ => [0b000, 0b000, 0b111, 0b000, 0b000],
    }
}

fn background(status: Status) -> [u8; 3] {
    match status {
        Status::Online(_, true) => BLUE,
        Status::Online(percent, false) if percent < LOW_PERCENT => RED,
        Status::Online(percent, false) if percent <= GOOD_PERCENT => ORANGE,
        Status::Online(..) => GREEN,
        Status::Asleep(_) | Status::Disconnected => GREY,
    }
}

fn label(status: Status) -> String {
    match status {
        Status::Online(percent, _) | Status::Asleep(Some((percent, _))) => percent.to_string(),
        Status::Asleep(None) | Status::Disconnected => "--".into(),
    }
}

/// Draw the 32x32 icon: the level in white on a rounded square.
fn draw_icon(status: Status) -> Icon {
    let n = ICON_SIZE;
    let mut pixels = vec![[0u8; 4]; n * n];
    let [r, g, b] = background(status);
    for y in 0..n {
        for x in 0..n {
            if inside_rounded_square(x, y) {
                pixels[y * n + x] = [0xff, r, g, b];
            }
        }
    }

    // Three digits ("100") only fit at scale 2; one or two digits are drawn at scale 3.
    let text = label(status);
    let count = text.chars().count();
    let scale = if count > 2 { 2 } else { 3 };
    let width = (count * 4 - 1) * scale;
    let height = 5 * scale;
    let (left, top) = ((n - width) / 2, (n - height) / 2);
    for (i, c) in text.chars().enumerate() {
        for (row, bits) in glyph(c).iter().enumerate() {
            for col in 0..3 {
                if bits & (0b100 >> col) == 0 {
                    continue;
                }
                let x0 = left + (i * 4 + col) * scale;
                let y0 = top + row * scale;
                for y in y0..y0 + scale {
                    for x in x0..x0 + scale {
                        let [r, g, b] = WHITE;
                        pixels[y * n + x] = [0xff, r, g, b];
                    }
                }
            }
        }
    }

    Icon {
        width: n as i32,
        height: n as i32,
        data: pixels.concat(),
    }
}

fn inside_rounded_square(x: usize, y: usize) -> bool {
    let (r, last) = (CORNER_RADIUS, ICON_SIZE - 1);
    // Distance from the nearest corner center, in each axis (0 outside the corner areas).
    let dx = r.saturating_sub(x).max((x + r).saturating_sub(last));
    let dy = r.saturating_sub(y).max((y + r).saturating_sub(last));
    dx * dx + dy * dy <= r * r
}

/// Scale an icon by an integer factor, repeating each pixel.
fn upscale(icon: &Icon, factor: usize) -> Icon {
    let (w, h) = (icon.width as usize, icon.height as usize);
    let mut data = Vec::with_capacity(icon.data.len() * factor * factor);
    for y in 0..h * factor {
        for x in 0..w * factor {
            let i = ((y / factor) * w + x / factor) * 4;
            data.extend_from_slice(&icon.data[i..i + 4]);
        }
    }
    Icon {
        width: (w * factor) as i32,
        height: (h * factor) as i32,
        data,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pixel(icon: &Icon, x: usize, y: usize) -> [u8; 4] {
        let i = (y * icon.width as usize + x) * 4;
        icon.data[i..i + 4].try_into().unwrap()
    }

    #[test]
    fn icon_size_and_format() {
        for status in [
            Status::Disconnected,
            Status::Asleep(None),
            Status::Asleep(Some((7, false))),
            Status::Online(0, false),
            Status::Online(100, true),
        ] {
            let icon = draw_icon(status);
            assert_eq!((icon.width, icon.height), (32, 32));
            assert_eq!(icon.data.len(), 32 * 32 * 4);
            let large = upscale(&icon, 2);
            assert_eq!((large.width, large.height), (64, 64));
            assert_eq!(large.data.len(), 64 * 64 * 4);
        }
    }

    #[test]
    fn corners_are_transparent_center_is_opaque() {
        let icon = draw_icon(Status::Online(80, false));
        assert_eq!(pixel(&icon, 0, 0)[0], 0);
        assert_eq!(pixel(&icon, 31, 31)[0], 0);
        assert_eq!(pixel(&icon, 0, 16), [0xff, GREEN[0], GREEN[1], GREEN[2]]);
    }

    #[test]
    fn digits_are_drawn_in_white() {
        // "100" at scale 2: 22 px wide starting at x = 5, 10 px tall starting at y = 11.
        let icon = draw_icon(Status::Online(100, false));
        assert_eq!(pixel(&icon, 7, 11), [0xff, 0xff, 0xff, 0xff]); // top of the "1"
        assert_eq!(pixel(&icon, 4, 11), [0xff, GREEN[0], GREEN[1], GREEN[2]]);
        // "5" at scale 3: 9 px wide starting at x = 11.
        let icon = draw_icon(Status::Online(5, false));
        assert_eq!(pixel(&icon, 11, 8), [0xff, 0xff, 0xff, 0xff]);
        assert_eq!(pixel(&icon, 11, 8 + 3 * 3), [0xff, RED[0], RED[1], RED[2]]);
    }

    #[test]
    fn background_by_level() {
        assert_eq!(background(Status::Online(19, false)), RED);
        assert_eq!(background(Status::Online(20, false)), ORANGE);
        assert_eq!(background(Status::Online(50, false)), ORANGE);
        assert_eq!(background(Status::Online(51, false)), GREEN);
        assert_eq!(background(Status::Online(10, true)), BLUE);
        assert_eq!(background(Status::Asleep(Some((90, false)))), GREY);
        assert_eq!(background(Status::Disconnected), GREY);
    }

    #[test]
    fn low_battery_notifies_once_per_discharge() {
        let mut low = LowBattery { armed: true };
        assert!(!low.update(30, false));
        assert!(low.update(19, false));
        assert!(!low.update(15, false));
        assert!(!low.update(21, false)); // hysteresis: not re-armed yet
        assert!(!low.update(19, false));
        assert!(!low.update(25, false)); // re-armed
        assert!(low.update(18, false));
        assert!(!low.update(18, true)); // charging re-arms
        assert!(low.update(17, false));
    }

    #[test]
    fn tooltip_text() {
        assert_eq!(describe(Status::Online(87, true)), "Battery 87%, charging");
        assert_eq!(describe(Status::Online(87, false)), "Battery 87%");
        assert_eq!(
            describe(Status::Asleep(Some((87, false)))),
            "Mouse asleep (last reading 87%)"
        );
        assert_eq!(describe(Status::Disconnected), "Dongle not connected");
    }
}
