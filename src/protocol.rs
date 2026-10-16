//! VXE R1 SE+ HID protocol: packet encoding and report parsing.
//!
//! Pure functions with no HID dependency, so they can be unit tested. See docs/PROTOCOL.md.

pub const VXE_VID: u16 = 0x373b;
pub const VXE_PID: u16 = 0x1085;

pub const REPORT_ID: u8 = 0x08;
pub const CMD_WRITE: u8 = 0x07;

/// Every value byte is followed by its complement `0x55 - value`.
const VALUE_KEY: u8 = 0x55;
/// The last payload byte makes the payload sum equal to `0x4D`.
const CHECKSUM_KEY: u8 = 0x4D;

/// A complete output report: report ID followed by the 16-byte payload.
pub type Packet = [u8; 17];

pub fn complement(value: u8) -> u8 {
    VALUE_KEY.wrapping_sub(value)
}

fn checksum(payload: &[u8; 16]) -> u8 {
    let sum = payload[0..15].iter().fold(0u8, |acc, &x| acc.wrapping_add(x));
    CHECKSUM_KEY.wrapping_sub(sum)
}

fn finish(mut payload: [u8; 16]) -> Packet {
    payload[15] = checksum(&payload);
    let mut buf = [0u8; 17];
    buf[0] = REPORT_ID;
    buf[1..17].copy_from_slice(&payload);
    buf
}

/// GetBatteryLevel query (`0x04`), verified on the `373b:1085` dongle.
pub fn battery_request() -> Packet {
    finish([CMD_BATTERY_LEVEL, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0])
}

/// DPI codes of the PAW3395SE sensor (CID/MID 2,31): 50-10,000 DPI in 50 DPI steps are
/// mapped to codes 1-235, skipping these codes. Verified on hardware for 400, 800, 1600
/// and 3200 DPI; values above 10,000 use an extra flag in the `ex` byte (not supported yet).
const PAW3395_SKIPPED_CODES: [u8; 35] = [
    7, 13, 20, 26, 33, 40, 46, 53, 60, 66, 73, 80, 86, 93, 100, 106, 113, 120, 126, 133, 140, 146, 153, 160, 166, 173,
    180, 186, 193, 200, 206, 213, 220, 226, 233,
];

/// PAW3395SE code for `dpi` (a multiple of 50 between 50 and 10,000).
pub fn paw3395_code(dpi: u16) -> Option<u8> {
    if !(50..=10_000).contains(&dpi) || !dpi.is_multiple_of(50) {
        return None;
    }
    (1..=235u8)
        .filter(|c| !PAW3395_SKIPPED_CODES.contains(c))
        .nth((dpi / 50 - 1) as usize)
}

/// DPI value of a PAW3395SE code, if the code is valid.
pub fn paw3395_dpi(code: u8) -> Option<u16> {
    if code == 0 || code > 235 || PAW3395_SKIPPED_CODES.contains(&code) {
        return None;
    }
    let skipped_below = PAW3395_SKIPPED_CODES.iter().filter(|&&c| c < code).count() as u16;
    Some((code as u16 - skipped_below) * 50)
}

#[derive(Debug, PartialEq, Eq)]
pub enum Report {
    /// Standard mouse/keyboard input or too short to be a vendor response.
    Ignored,
    /// Battery level in percent and charging state.
    Battery { percent: u8, charging: bool },
    /// Any other vendor response, kept for the hex sniffer.
    Unknown,
}

pub fn parse_report(buf: &[u8]) -> Report {
    if matches!(buf.first(), None | Some(0x01 | 0x02)) {
        return Report::Ignored;
    }
    match payload_of(buf) {
        Some(p) if p[0] == CMD_BATTERY_LEVEL && p[1] == 0 => Report::Battery {
            percent: p[5].min(100),
            charging: p[6] != 0,
        },
        Some(_) => Report::Unknown,
        None => Report::Ignored,
    }
}

pub fn to_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{:02X}", b)).collect::<Vec<_>>().join(" ")
}

/// Parse hex bytes such as `07 00 0c`, `07:00:0c`, `0x07,0x00` or `07000c`.
pub fn parse_hex(s: &str) -> Option<Vec<u8>> {
    let digits: String = s
        .split([' ', '\t', ':', ',', '-'])
        .map(|t| t.trim_start_matches("0x").trim_start_matches("0X"))
        .collect();
    if digits.is_empty() || !digits.len().is_multiple_of(2) || !digits.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    (0..digits.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&digits[i..i + 2], 16).ok())
        .collect()
}

/// Build a packet from up to 15 payload bytes, padding with zeros and appending the checksum.
pub fn build_packet(payload: &[u8]) -> Option<Packet> {
    if payload.len() > 15 {
        return None;
    }
    let mut p = [0u8; 16];
    p[..payload.len()].copy_from_slice(payload);
    Some(finish(p))
}

/// Extract the 16-byte payload from a report, with or without the leading report ID.
fn payload_of(bytes: &[u8]) -> Option<[u8; 16]> {
    let start = if bytes.len() >= 17 && bytes[0] == REPORT_ID {
        1
    } else {
        0
    };
    bytes.get(start..start + 16)?.try_into().ok()
}

/// Command names of the Compx/ATK vendor protocol, as used by the official ATK HUB web driver.
/// See docs/PROTOCOL.md.
fn command_name(cmd: u8) -> Option<&'static str> {
    Some(match cmd {
        0x01 => "DownLoadData",
        0x02 => "DownLoadDriverStatus",
        0x03 => "GetWirelessMouseOnline",
        0x04 => "GetBatteryLevel",
        0x05 => "SetWirelessDonglePair",
        0x06 => "GetWirelessDonglePairResult",
        0x07 => "SetEEPROM",
        0x08 => "GetEEPROM",
        0x09 => "RestoreFactory",
        0x0a => "ReportMouseStatus",
        0x0d => "EnterUSBUpgradeMode",
        0x0e => "GetCurrentConfig",
        0x0f => "SetCurrentConfig",
        0x10 => "GetMouseCIDMID",
        0x12 => "GetMouseVersion",
        0x13 => "DongleExitPair",
        0x14 => "Set4KRGBMode",
        0x15 => "Get4KRGBMode",
        0x16 => "SetFarDistanceMode",
        0x17 => "GetFarDistanceMode",
        0x18 => "SetDongleLightMode",
        0x19 => "GetDongleLightMode",
        0x1d => "GetDongleVersion",
        _ => return None,
    })
}

pub const CMD_BATTERY_LEVEL: u8 = 0x04;
pub const CMD_GET_EEPROM: u8 = 0x08;
pub const CMD_GET_CID_MID: u8 = 0x10;
pub const CMD_GET_VERSION: u8 = 0x12;

/// Maximum data bytes in one EEPROM read/write.
pub const EEPROM_CHUNK: u8 = 10;

/// Read `len` bytes (max [`EEPROM_CHUNK`]) of the configuration memory at `addr`.
pub fn eeprom_read(addr: u16, len: u8) -> Packet {
    let [hi, lo] = addr.to_be_bytes();
    finish([
        CMD_GET_EEPROM,
        0,
        hi,
        lo,
        len.min(EEPROM_CHUNK),
        0,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
    ])
}

/// Write `data` (max [`EEPROM_CHUNK`] bytes) to the configuration memory at `addr`.
pub fn set_eeprom(addr: u16, data: &[u8]) -> Option<Packet> {
    if data.is_empty() || data.len() > EEPROM_CHUNK as usize {
        return None;
    }
    let [hi, lo] = addr.to_be_bytes();
    let mut p = [0u8; 16];
    p[..5].copy_from_slice(&[CMD_WRITE, 0, hi, lo, data.len() as u8]);
    p[5..5 + data.len()].copy_from_slice(data);
    Some(finish(p))
}

/// Whether `report` answers `request`: same command and address. Some firmware answers
/// GetEEPROM with the SetEEPROM command code; a SetEEPROM is acknowledged by its echo.
pub fn is_reply_to(request: &Packet, report: &[u8]) -> bool {
    let Some(p) = payload_of(report) else { return false };
    let same_cmd = p[0] == request[1] || (request[1] == CMD_GET_EEPROM && p[0] == CMD_WRITE);
    same_cmd && p[2..4] == request[3..5]
}

/// Status byte of a reply (0 = ok).
pub fn reply_status(report: &[u8]) -> Option<u8> {
    payload_of(report).map(|p| p[1])
}

/// Data bytes of a reply, as long as its length field says.
pub fn reply_data(report: &[u8]) -> Option<Vec<u8>> {
    let p = payload_of(report)?;
    let len = (p[4] as usize).min(10);
    Some(p[5..5 + len].to_vec())
}

const BUTTON_NAMES: [&str; 6] = ["left", "right", "middle", "back", "forward", "bottom"];

fn button_class(class: u8) -> &'static str {
    match class {
        0x00 => "disabled",
        0x01 => "mouse",
        0x02 => "dpi",
        0x03 => "h-scroll",
        0x04 => "fire",
        0x05 => "shortcut",
        0x06 => "macro",
        0x07 => "report-rate",
        0x08 => "light",
        0x09 => "profile",
        0x0a => "dpi-lock",
        0x0b => "wheel",
        _ => "unknown",
    }
}

/// Decode the value of a value/complement pair field of the EEPROM map.
fn pair_field(addr: u16, v: u8) -> Option<String> {
    Some(match addr {
        0x00 => format!(
            "report rate {}",
            match v {
                1 => "1000 Hz",
                2 => "500 Hz",
                4 => "250 Hz",
                8 => "125 Hz",
                _ => "unknown",
            }
        ),
        0x02 => format!("DPI stage count {}", v),
        0x04 => format!("active DPI stage {}", v),
        0x0a => format!("LOD {}", v),
        0x4c => format!("DPI LED effect {}", v),
        0x4e => format!("DPI LED brightness {}", v),
        0x50 => format!("DPI LED speed {}", v),
        0x52 => format!("DPI LED enabled {}", v),
        0x58 => format!("logo LED effect {}", v),
        0x5a => format!("logo LED brightness {}", v),
        0x5c => format!("logo LED breathing speed {}", v),
        0x5e => format!("logo LED energy saving {}", v),
        0xa9 => format!("debounce {} ms", v),
        0xab => format!("motion sync {}", if v != 0 { "on" } else { "off" }),
        0xad => format!("sleep {} s", v as u32 * 10),
        0xaf => format!("angle snapping (linear correction) {}", v),
        0xb1 => format!("ripple control {}", v),
        0xb3 => format!("LEDs off while moving {}", v),
        0xb5 => format!("sensor sleep enabled {}", v),
        0xb7 => format!("sensor sleep time {} s", v as u32 * 10),
        0xb9 => format!("sensor mode {}", v),
        0xbb => format!("RF TX time {}", v),
        0xbd => format!("angle tuning {}", v),
        _ => return None,
    })
}

/// The pre-research OpenVXE interpretation of writes to 0x70: `[register, value, 0, 0x55 - value]`.
fn legacy_sensor_register(record: &[u8]) -> Option<String> {
    let [reg, v, _, check] = *record else { return None };
    if check != complement(v) {
        return None;
    }
    Some(match reg {
        0x01 => format!("angle snapping {}", if v != 0 { "on" } else { "off" }),
        0x02 => format!("debounce {} ms", v),
        0x03 => format!("LOD {}", v),
        0x0b => format!("polling rate code {}", v),
        _ => format!("register 0x{:02X} = 0x{:02X}", reg, v),
    })
}

fn record_ok(r: &[u8]) -> bool {
    r[3] == VALUE_KEY.wrapping_sub(r[0].wrapping_add(r[1]).wrapping_add(r[2]))
}

/// Decode EEPROM data starting at `addr`, one entry per field.
///
/// Known fields are named; unknown ones are shown as value/complement pairs or 4-byte
/// records when their checks match, otherwise as raw bytes.
pub fn eeprom_fields(addr: u16, data: &[u8]) -> Vec<String> {
    let mut fields = Vec::new();
    let mut raw: Option<(u16, Vec<u8>)> = None;
    let mut a = addr;
    let mut rest = data;
    while !rest.is_empty() {
        let record = rest.get(..4);
        let entry = match (a, record) {
            (0x0c..=0x2b, Some(r)) if (a - 0x0c).is_multiple_of(4) => {
                let dpi = match (r[2], paw3395_dpi(r[0])) {
                    (0, Some(dpi)) if r[0] == r[1] => format!("{} DPI (PAW3395SE) ", dpi),
                    _ => String::new(),
                };
                let text = format!(
                    "DPI stage {}: {}x=0x{:02X} y=0x{:02X} ex=0x{:02X}{}",
                    (a - 0x0c) / 4 + 1,
                    dpi,
                    r[0],
                    r[1],
                    r[2],
                    if record_ok(r) {
                        ""
                    } else {
                        " [record checksum mismatch]"
                    }
                );
                Some((text, 4))
            }
            (0x54, Some(r)) if record_ok(r) => Some((format!("logo colour #{:02X}{:02X}{:02X}", r[0], r[1], r[2]), 4)),
            (0x2c..=0x4b, Some(r)) if (a - 0x2c).is_multiple_of(4) => {
                let text = format!(
                    "DPI stage {} colour #{:02X}{:02X}{:02X}{}",
                    (a - 0x2c) / 4 + 1,
                    r[0],
                    r[1],
                    r[2],
                    if record_ok(r) {
                        ""
                    } else {
                        " [record checksum mismatch]"
                    }
                );
                Some((text, 4))
            }
            (0x60..=0x9f, Some(r)) if (a - 0x60).is_multiple_of(4) => {
                let slot = ((a - 0x60) / 4) as usize;
                let name = BUTTON_NAMES
                    .get(slot)
                    .map_or(format!("key slot {}", slot), |n| format!("button {}", n));
                let mut text = format!("{}: {} v1=0x{:02X} v2=0x{:02X}", name, button_class(r[0]), r[1], r[2]);
                if !record_ok(r) {
                    text.push_str(" [record checksum mismatch");
                    if a == 0x70 {
                        if let Some(legacy) = legacy_sensor_register(r) {
                            text.push_str(&format!("; matches OpenVXE legacy write: {}", legacy));
                        }
                    }
                    text.push(']');
                }
                Some((text, 4))
            }
            _ => {
                let pair_ok = rest.get(1) == Some(&complement(rest[0]));
                match (pair_field(a, rest[0]), rest.get(1)) {
                    (Some(field), Some(_)) => Some((
                        format!("{}{}", field, if pair_ok { "" } else { " [bad complement]" }),
                        2,
                    )),
                    _ if pair_ok => Some((format!("0x{:04X} = 0x{:02X}", a, rest[0]), 2)),
                    _ => match record {
                        Some(r) if record_ok(r) => Some((format!("0x{:04X}: record [{}]", a, to_hex(r)), 4)),
                        _ => None,
                    },
                }
            }
        };

        let used = match entry {
            Some((text, used)) => {
                if let Some((start, bytes)) = raw.take() {
                    fields.push(format!("0x{:04X}: [{}]", start, to_hex(&bytes)));
                }
                fields.push(text);
                used
            }
            None => {
                raw.get_or_insert((a, Vec::new())).1.push(rest[0]);
                1
            }
        };
        a = a.wrapping_add(used as u16);
        rest = &rest[used.min(rest.len())..];
    }
    if let Some((start, bytes)) = raw {
        fields.push(format!("0x{:04X}: [{}]", start, to_hex(&bytes)));
    }
    fields
}

fn describe_eeprom(addr: u16, data: &[u8]) -> String {
    eeprom_fields(addr, data).join("; ")
}

/// Human-readable description of a vendor packet, for the sniffer, the probe and the capture decoder.
///
/// Payload layout: `[cmd, status, addr_hi, addr_lo, len, data..., checksum]`. Known commands
/// and EEPROM fields are decoded; anything else is shown raw to help reverse engineering.
pub fn describe(bytes: &[u8]) -> String {
    let Some(p) = payload_of(bytes) else {
        return format!("short report ({} bytes)", bytes.len());
    };
    let bad_checksum = if checksum(&p) == p[15] { "" } else { " [bad checksum]" };
    let status = if p[1] != 0 {
        format!(" status=0x{:02X} (rejected?)", p[1])
    } else {
        String::new()
    };
    let addr = u16::from_be_bytes([p[2], p[3]]);
    let len = p[4] as usize;
    let data = &p[5..(5 + len).min(15)];
    let is_query = p[1..15].iter().all(|&b| b == 0);
    let name = command_name(p[0]).map_or(format!("CMD 0x{:02X}", p[0]), str::to_string);

    let body = match p[0] {
        _ if is_query => format!("{} query", name),
        CMD_BATTERY_LEVEL => format!(
            "{}: {}%, {}, {} mV",
            name,
            p[5],
            if p[6] != 0 { "charging" } else { "not charging" },
            u16::from_be_bytes([p[7], p[8]])
        ),
        CMD_GET_CID_MID => format!("{}: CID={} MID={}", name, p[5], p[6]),
        CMD_GET_VERSION => format!("{}: firmware {:x}.{:02x}", name, p[5], p[6]),
        CMD_WRITE | CMD_GET_EEPROM if data.iter().all(|&b| b == 0) => {
            format!("{} 0x{:04X} len={}", name, addr, len)
        }
        CMD_WRITE | CMD_GET_EEPROM => format!(
            "{} 0x{:04X} len={} data=[{}] -> {}",
            name,
            addr,
            len,
            to_hex(data),
            describe_eeprom(addr, data)
        ),
        _ => format!("{} addr=0x{:04X} len={} data=[{}]", name, addr, len, to_hex(data)),
    };
    format!("{}{}{}", body, status, bad_checksum)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn payload_sum(p: &Packet) -> u8 {
        p[1..].iter().fold(0u8, |acc, &x| acc.wrapping_add(x))
    }

    /// Packets sent by OpenVXE before the protocol was understood, kept to test the decoder.
    fn legacy(payload: &[u8]) -> Packet {
        build_packet(payload).unwrap()
    }
    fn legacy_dpi_1600() -> Packet {
        legacy(&[0x07, 0, 0, 0x0c, 0x08, 0x04, 0x04, 0x00, 0x4d, 0x20, 0x20, 0x00, 0x15])
    }
    fn legacy_polling_500() -> Packet {
        legacy(&[0x07, 0, 0, 0x70, 0x04, 0x0b, 0x02, 0x00, 0x53])
    }
    fn legacy_motion_sync_on() -> Packet {
        legacy(&[0x07, 0, 0, 0xa9, 0x0a, 0x08, 0x4d, 0x01, 0x54, 0x05, 0x50])
    }

    #[test]
    fn checksum_makes_payload_sum_0x4d() {
        for p in [
            battery_request(),
            eeprom_read(0x0c, 8),
            set_eeprom(0x00, &[0x02, 0x53]).unwrap(),
        ] {
            assert_eq!(payload_sum(&p), CHECKSUM_KEY);
        }
    }

    #[test]
    fn battery_request_bytes() {
        let mut expected = [0u8; 17];
        expected[0] = 0x08;
        expected[1] = 0x04;
        expected[16] = 0x49;
        assert_eq!(battery_request(), expected);
    }

    #[test]
    fn set_eeprom_matches_hardware_test() {
        // Report rate 500 Hz, applied and echoed by the mouse (see PROTOCOL.md).
        assert_eq!(
            set_eeprom(0x0000, &[0x02, 0x53]).unwrap(),
            [0x08, 0x07, 0x00, 0x00, 0x00, 0x02, 0x02, 0x53, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xef]
        );
        assert_eq!(set_eeprom(0, &[]), None);
        assert_eq!(set_eeprom(0, &[0; 11]), None);
    }

    #[test]
    fn replies_are_matched_by_command_and_address() {
        let write = set_eeprom(0x0000, &[0x02, 0x53]).unwrap();
        assert!(is_reply_to(&write, &write));
        assert!(!is_reply_to(&write, &set_eeprom(0x000a, &[0x02, 0x53]).unwrap()));
        assert!(!is_reply_to(&write, &REAL_BATTERY_REPLY));
        assert!(is_reply_to(&battery_request(), &REAL_BATTERY_REPLY));

        let read = eeprom_read(0x0000, 10);
        assert!(is_reply_to(&read, &REAL_EEPROM_0000));
        assert_eq!(reply_status(&REAL_EEPROM_0000), Some(0));
        assert_eq!(
            reply_data(&REAL_EEPROM_0000).unwrap(),
            [0x01, 0x54, 0x04, 0x51, 0x01, 0x54, 0x00, 0x55, 0x00, 0x55]
        );
        // Some firmware answers GetEEPROM with the SetEEPROM code.
        let mut as_write = REAL_EEPROM_0000;
        as_write[1] = CMD_WRITE;
        assert!(is_reply_to(&read, &as_write));
    }

    #[test]
    fn parse_hex_formats() {
        let expected = Some(vec![0x07, 0x00, 0x0c]);
        assert_eq!(parse_hex("07 00 0c"), expected);
        assert_eq!(parse_hex("07:00:0C"), expected);
        assert_eq!(parse_hex("0x07,0x00,0x0c"), expected);
        assert_eq!(parse_hex("07000c"), expected);
        assert_eq!(parse_hex("070"), None);
        assert_eq!(parse_hex("zz"), None);
        assert_eq!(parse_hex(""), None);
    }

    #[test]
    fn build_packet_pads_and_checksums() {
        assert_eq!(build_packet(&[0x04]), Some(battery_request()));
        assert_eq!(build_packet(&[0; 16]), None);
    }

    #[test]
    fn eeprom_read_matches_published_frame() {
        // GetEEPROM of 8 bytes at 0x60, from the ATK Fierce X protocol notes (see PROTOCOL.md).
        let expected = [0x08, 0x00, 0x00, 0x60, 0x08, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0xdd];
        assert_eq!(eeprom_read(0x60, 8)[1..], expected);
        assert_eq!(eeprom_read(0x60, 50)[5], EEPROM_CHUNK);
    }

    #[test]
    fn describe_dpi_write_as_two_stages() {
        // The legacy dpi/50 encoding is wrong for the PAW3395SE: asking 1600 DPI writes code
        // 0x20, which the sensor reads as 1400 DPI, and stage 1 is forced to 200 DPI.
        let dpi = describe(&legacy_dpi_1600());
        assert!(
            dpi.ends_with(
                "DPI stage 1: 200 DPI (PAW3395SE) x=0x04 y=0x04 ex=0x00; \
                 DPI stage 2: 1400 DPI (PAW3395SE) x=0x20 y=0x20 ex=0x00"
            ),
            "{dpi}"
        );
    }

    #[test]
    fn describe_legacy_0x70_writes_as_forward_button() {
        let rate = describe(&legacy_polling_500());
        assert!(rate.contains("button forward: wheel"), "{rate}");
        assert!(
            rate.contains("matches OpenVXE legacy write: polling rate code 2"),
            "{rate}"
        );
        // Without the report ID, as some capture tools export it.
        let lod = legacy(&[0x07, 0, 0, 0x70, 0x04, 0x03, 0x02, 0x00, 0x53]);
        assert!(describe(&lod[1..]).contains("legacy write: LOD 2"));
    }

    #[test]
    fn describe_motion_sync_write_covers_debounce_and_sleep() {
        let sync = describe(&legacy_motion_sync_on());
        assert!(
            sync.contains(
                "debounce 8 ms; motion sync on; sleep 50 s; angle snapping (linear correction) 0 [bad complement]"
            ),
            "{sync}"
        );
    }

    #[test]
    fn describe_queries_and_responses() {
        assert_eq!(describe(&battery_request()), "GetBatteryLevel query");
        assert_eq!(describe(&build_packet(&[0x17]).unwrap()), "GetFarDistanceMode query");
        assert_eq!(describe(&eeprom_read(0x60, 8)), "GetEEPROM 0x0060 len=8");

        let battery = build_packet(&[0x04, 0x00, 0x00, 0x00, 0x02, 0x5f, 0x00, 0x10, 0x00]).unwrap();
        assert_eq!(describe(&battery), "GetBatteryLevel: 95%, not charging, 4096 mV");
        let version = build_packet(&[0x12, 0x00, 0x00, 0x00, 0x02, 0x03, 0x15]).unwrap();
        assert_eq!(describe(&version), "GetMouseVersion: firmware 3.15");

        let read = build_packet(&[0x08, 0x00, 0x00, 0x00, 0x06, 0x01, 0x54, 0x04, 0x51, 0x00, 0x55]).unwrap();
        assert_eq!(
            describe(&read),
            "GetEEPROM 0x0000 len=6 data=[01 54 04 51 00 55] -> report rate 1000 Hz; DPI stage count 4; active DPI stage 0"
        );
    }

    #[test]
    fn describe_unknown_and_invalid_packets() {
        let unknown = describe(&build_packet(&[0x30, 0x01, 0x00, 0x0c, 0x02, 0x10, 0x45]).unwrap());
        assert_eq!(
            unknown,
            "CMD 0x30 addr=0x000C len=2 data=[10 45] status=0x01 (rejected?)"
        );

        let mut corrupted = legacy_dpi_1600();
        corrupted[16] ^= 0xff;
        assert!(describe(&corrupted).ends_with("[bad checksum]"));
        assert_eq!(describe(&[0x08, 0x07]), "short report (2 bytes)");
    }

    /// Replies captured from a VXE R1 SE+ (dongle 373b:1085, firmware 3.15, CID/MID 2,31).
    const REAL_BATTERY_REPLY: [u8; 17] = [
        0x08, 0x04, 0x00, 0x00, 0x00, 0x02, 0x64, 0x00, 0x10, 0x79, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x5a,
    ];
    const REAL_FAR_DISTANCE_REPLY: [u8; 17] = [
        0x08, 0x17, 0x00, 0x00, 0x00, 0x0a, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x2c,
    ];
    const REAL_EEPROM_0000: [u8; 17] = [
        0x08, 0x08, 0x00, 0x00, 0x00, 0x0a, 0x01, 0x54, 0x04, 0x51, 0x01, 0x54, 0x00, 0x55, 0x00, 0x55, 0x92,
    ];
    /// EEPROM 0x000C-0x002B: the four default stages 400/800/1600/3200 DPI, then copies of 3200.
    const REAL_DPI_STAGES: [u8; 32] = [
        0x09, 0x09, 0x00, 0x43, 0x12, 0x12, 0x00, 0x31, 0x25, 0x25, 0x00, 0x0b, 0x4b, 0x4b, 0x00, 0xbf, 0x4b, 0x4b,
        0x00, 0xbf, 0x4b, 0x4b, 0x00, 0xbf, 0x4b, 0x4b, 0x00, 0xbf, 0x4b, 0x4b, 0x00, 0xbf,
    ];

    #[test]
    fn parse_real_battery_reply() {
        assert_eq!(
            parse_report(&REAL_BATTERY_REPLY),
            Report::Battery {
                percent: 100,
                charging: false
            }
        );
        assert_eq!(
            describe(&REAL_BATTERY_REPLY),
            "GetBatteryLevel: 100%, not charging, 4217 mV"
        );
        // The old 0x17 query is GetFarDistanceMode: its reply must not be read as battery.
        assert_eq!(parse_report(&REAL_FAR_DISTANCE_REPLY), Report::Unknown);
        assert_eq!(battery_request()[1], CMD_BATTERY_LEVEL);
    }

    #[test]
    fn decode_real_eeprom() {
        assert!(!describe(&REAL_EEPROM_0000).contains("bad checksum"));
        assert_eq!(
            describe(&REAL_EEPROM_0000),
            "GetEEPROM 0x0000 len=10 data=[01 54 04 51 01 54 00 55 00 55] -> \
             report rate 1000 Hz; DPI stage count 4; active DPI stage 1; 0x0006 = 0x00; 0x0008 = 0x00"
        );
        let stages = eeprom_fields(0x0c, &REAL_DPI_STAGES);
        assert_eq!(stages.len(), 8);
        assert_eq!(stages[0], "DPI stage 1: 400 DPI (PAW3395SE) x=0x09 y=0x09 ex=0x00");
        assert!(stages[1].starts_with("DPI stage 2: 800 DPI"));
        assert!(stages[2].starts_with("DPI stage 3: 1600 DPI"));
        assert!(stages[3].starts_with("DPI stage 4: 3200 DPI"));
    }

    #[test]
    fn paw3395_codes() {
        for (dpi, code) in [
            (50, 1),
            (300, 6),
            (350, 8),
            (400, 0x09),
            (800, 0x12),
            (1600, 0x25),
            (3200, 0x4b),
        ] {
            assert_eq!(paw3395_code(dpi), Some(code), "{dpi}");
            assert_eq!(paw3395_dpi(code), Some(dpi), "{code}");
        }
        assert_eq!(paw3395_code(10_000), Some(235));
        assert_eq!(paw3395_code(10_050), None);
        assert_eq!(paw3395_code(825), None);
        assert_eq!(paw3395_dpi(7), None);
        for code in 1..=235 {
            if let Some(dpi) = paw3395_dpi(code) {
                assert_eq!(paw3395_code(dpi), Some(code));
            }
        }
    }

    #[test]
    fn eeprom_fields_group_unknown_bytes() {
        assert_eq!(
            eeprom_fields(0x48, &[0xff, 0x00, 0xff, 0x57]),
            ["DPI stage 8 colour #FF00FF"]
        );
        assert_eq!(eeprom_fields(0x54, &[0xff, 0x00, 0xff, 0x57]), ["logo colour #FF00FF"]);
        // Unknown area: a pair, a valid record, then bytes that are neither.
        assert_eq!(
            eeprom_fields(0xc0, &[0x02, 0x53, 0xff, 0x00, 0xff, 0x57, 0x01, 0xff]),
            ["0x00C0 = 0x02", "0x00C2: record [FF 00 FF 57]", "0x00C6: [01 FF]"]
        );
    }

    #[test]
    fn parse_ignores_input_and_short_reports() {
        assert_eq!(parse_report(&[]), Report::Ignored);
        assert_eq!(parse_report(&[0x01; 16]), Report::Ignored);
        assert_eq!(parse_report(&[0x02; 16]), Report::Ignored);
        assert_eq!(parse_report(&[0x04, 0, 0, 0, 50]), Report::Ignored);
        assert_eq!(parse_report(&[0x09; 16]), Report::Unknown);
    }
}
