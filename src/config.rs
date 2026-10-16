//! Mouse configuration: an image of the configuration memory (EEPROM), the settings
//! decoded from it and the writes needed to change them.
//!
//! Changes are applied read-modify-write on whole blocks, the same blocks the official
//! ATK HUB driver writes (see docs/PROTOCOL.md).

use crate::protocol::{self, complement, Packet};
use std::fmt;

/// Range read at startup: everything the TUI decodes.
pub const IMAGE_START: u16 = 0x0000;
pub const IMAGE_END: u16 = 0x00c0;

const REPORT_RATE: u16 = 0x00;
const STAGE_COUNT: u16 = 0x02;
const ACTIVE_STAGE: u16 = 0x04;
const LOD: u16 = 0x0a;
const DPI_STAGES: u16 = 0x0c;
/// Debounce, motion sync, sleep, angle snapping, ripple control: written together.
const PERFORMANCE_BLOCK: u16 = 0xa9;
const PERFORMANCE_LEN: u8 = 10;
const DEBOUNCE: u16 = 0xa9;
const MOTION_SYNC: u16 = 0xab;
const SLEEP: u16 = 0xad;
const ANGLE_SNAP: u16 = 0xaf;
const RIPPLE: u16 = 0xb1;

pub const MAX_STAGES: u8 = 8;
pub const MIN_DPI: u16 = 50;
/// Highest DPI of the PAW3395SE code table; higher values need an extra flag (not supported).
pub const MAX_DPI: u16 = 10_000;
pub const DPI_STEP: u16 = 50;
const REPORT_RATES: [(u16, u8); 4] = [(125, 8), (250, 4), (500, 2), (1000, 1)];
/// Sleep options offered by the official software for the R1, in seconds.
const SLEEP_OPTIONS: [u16; 8] = [30, 60, 120, 180, 300, 1200, 1500, 1800];
const MAX_DEBOUNCE: u8 = 20;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Setting {
    /// DPI of stage `index` (0-based); `None` if the stored code is not understood.
    DpiStage {
        index: u8,
        dpi: Option<u16>,
    },
    StageCount(u8),
    /// 0-based.
    ActiveStage(u8),
    PollingRate(u16),
    Lod(u8),
    Debounce(u8),
    MotionSync(bool),
    /// Seconds.
    Sleep(u16),
    AngleSnap(bool),
    Ripple(bool),
}

fn on_off(v: bool) -> &'static str {
    if v {
        "Enabled"
    } else {
        "Disabled"
    }
}

impl fmt::Display for Setting {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Setting::DpiStage { index, dpi: Some(dpi) } => write!(f, "DPI Stage {}: {} DPI", index + 1, dpi),
            Setting::DpiStage { index, dpi: None } => write!(f, "DPI Stage {}: unknown encoding", index + 1),
            Setting::StageCount(v) => write!(f, "DPI Stages: {}", v),
            Setting::ActiveStage(v) => write!(f, "Active DPI Stage: {}", v + 1),
            Setting::PollingRate(v) => write!(f, "Polling Rate: {} Hz", v),
            Setting::Lod(v) => write!(f, "Lift-off Distance: {} mm", v),
            Setting::Debounce(v) => write!(f, "Debounce Time: {} ms", v),
            Setting::MotionSync(v) => write!(f, "Motion Sync: {}", on_off(*v)),
            Setting::Sleep(v) if *v >= 60 && v % 60 == 0 => write!(f, "Sleep After: {} min", v / 60),
            Setting::Sleep(v) => write!(f, "Sleep After: {} s", v),
            Setting::AngleSnap(v) => write!(f, "Angle Snapping: {}", on_off(*v)),
            Setting::Ripple(v) => write!(f, "Ripple Control: {}", on_off(*v)),
        }
    }
}

/// Step through a list of options: next/previous option, or the nearest one for a value
/// that is not in the list.
fn step_option(options: &[u16], value: u16, dir: i32) -> u16 {
    if dir > 0 {
        options
            .iter()
            .copied()
            .find(|&o| o > value)
            .unwrap_or(options[options.len() - 1])
    } else {
        options.iter().rev().copied().find(|&o| o < value).unwrap_or(options[0])
    }
}

impl Setting {
    /// Step the setting by one unit in direction `dir` (negative = decrease).
    pub fn adjust(&mut self, dir: i32, stage_count: u8) {
        match self {
            Setting::DpiStage { dpi: Some(v), .. } => {
                *v = (*v as i32 + dir * DPI_STEP as i32).clamp(MIN_DPI as i32, MAX_DPI as i32) as u16
            }
            Setting::DpiStage { dpi: None, .. } => {}
            Setting::StageCount(v) => *v = (*v as i32 + dir).clamp(1, MAX_STAGES as i32) as u8,
            Setting::ActiveStage(v) => *v = (*v as i32 + dir).clamp(0, stage_count.max(1) as i32 - 1) as u8,
            Setting::PollingRate(v) => {
                let rates: Vec<u16> = REPORT_RATES.iter().map(|(hz, _)| *hz).collect();
                *v = step_option(&rates, *v, dir)
            }
            Setting::Lod(v) => *v = if dir > 0 { 2 } else { 1 },
            Setting::Debounce(v) => *v = (*v as i32 + dir).clamp(1, MAX_DEBOUNCE as i32) as u8,
            Setting::Sleep(v) => *v = step_option(&SLEEP_OPTIONS, *v, dir),
            Setting::MotionSync(v) | Setting::AngleSnap(v) | Setting::Ripple(v) => *v = !*v,
        }
    }
}

/// Image of the configuration memory from [`IMAGE_START`] to [`IMAGE_END`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Config {
    bytes: Vec<u8>,
}

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum ConfigError {
    #[error("expected {expected} bytes, got {got}")]
    WrongSize { expected: usize, got: usize },
}

impl Config {
    pub fn from_bytes(bytes: Vec<u8>) -> Result<Self, ConfigError> {
        let expected = (IMAGE_END - IMAGE_START) as usize;
        if bytes.len() != expected {
            return Err(ConfigError::WrongSize {
                expected,
                got: bytes.len(),
            });
        }
        Ok(Config { bytes })
    }

    fn byte(&self, addr: u16) -> u8 {
        self.bytes[(addr - IMAGE_START) as usize]
    }

    fn set_byte(&mut self, addr: u16, v: u8) {
        self.bytes[(addr - IMAGE_START) as usize] = v;
    }

    /// Value of a value/complement pair, if the complement is valid.
    fn pair(&self, addr: u16) -> Option<u8> {
        let v = self.byte(addr);
        (self.byte(addr + 1) == complement(v)).then_some(v)
    }

    fn set_pair(&mut self, addr: u16, v: u8) {
        self.set_byte(addr, v);
        self.set_byte(addr + 1, complement(v));
    }

    pub fn stage_count(&self) -> u8 {
        self.pair(STAGE_COUNT).unwrap_or(1).clamp(1, MAX_STAGES)
    }

    /// Active stage as stored, without clamping to the stage count.
    pub fn stored_active_stage(&self) -> Option<u8> {
        self.pair(ACTIVE_STAGE)
    }

    pub fn active_stage(&self) -> u8 {
        self.pair(ACTIVE_STAGE).unwrap_or(0).min(self.stage_count() - 1)
    }

    /// DPI of a stage, if it uses the PAW3395SE code table with equal X and Y.
    pub fn stage_dpi(&self, index: u8) -> Option<u16> {
        let a = DPI_STAGES + index as u16 * 4;
        let [x, y, ex, check] = [self.byte(a), self.byte(a + 1), self.byte(a + 2), self.byte(a + 3)];
        let valid = check == complement(x.wrapping_add(y).wrapping_add(ex));
        if valid && x == y && ex == 0 {
            protocol::paw3395_dpi(x)
        } else {
            None
        }
    }

    /// Settings shown in the TUI, in display order. Fields with an invalid complement are skipped.
    pub fn settings(&self) -> Vec<Setting> {
        let mut items: Vec<Setting> = (0..self.stage_count())
            .map(|index| Setting::DpiStage {
                index,
                dpi: self.stage_dpi(index),
            })
            .collect();
        items.push(Setting::StageCount(self.stage_count()));
        items.push(Setting::ActiveStage(self.active_stage()));

        let rate = self
            .pair(REPORT_RATE)
            .and_then(|code| REPORT_RATES.iter().find(|(_, c)| *c == code).map(|(hz, _)| *hz));
        let optional = [
            rate.map(Setting::PollingRate),
            self.pair(LOD).map(Setting::Lod),
            self.pair(DEBOUNCE).map(Setting::Debounce),
            self.pair(MOTION_SYNC).map(|v| Setting::MotionSync(v != 0)),
            self.pair(SLEEP).map(|v| Setting::Sleep(v as u16 * 10)),
            self.pair(ANGLE_SNAP).map(|v| Setting::AngleSnap(v != 0)),
            self.pair(RIPPLE).map(|v| Setting::Ripple(v != 0)),
        ];
        items.extend(optional.into_iter().flatten());
        items
    }

    /// Store `setting` in the image and return the block `(address, length)` to write.
    /// Returns `None` if the setting cannot be encoded.
    pub fn apply(&mut self, setting: &Setting) -> Option<(u16, u8)> {
        match *setting {
            Setting::DpiStage { index, dpi } => {
                let code = protocol::paw3395_code(dpi?)?;
                let a = DPI_STAGES + index as u16 * 4;
                for (i, b) in [code, code, 0, complement(code.wrapping_add(code))]
                    .into_iter()
                    .enumerate()
                {
                    self.set_byte(a + i as u16, b);
                }
                // The official driver writes stages in pairs.
                Some((DPI_STAGES + (index as u16 / 2) * 8, 8))
            }
            Setting::StageCount(v) => {
                self.set_pair(STAGE_COUNT, v.clamp(1, MAX_STAGES));
                Some((STAGE_COUNT, 2))
            }
            Setting::ActiveStage(v) => {
                self.set_pair(ACTIVE_STAGE, v);
                Some((ACTIVE_STAGE, 2))
            }
            Setting::PollingRate(hz) => {
                let code = REPORT_RATES.iter().find(|(r, _)| *r == hz)?.1;
                self.set_pair(REPORT_RATE, code);
                Some((REPORT_RATE, 2))
            }
            Setting::Lod(v) => {
                self.set_pair(LOD, v);
                Some((LOD, 2))
            }
            Setting::Debounce(v) => self.apply_performance(DEBOUNCE, v),
            Setting::MotionSync(v) => self.apply_performance(MOTION_SYNC, v.into()),
            Setting::Sleep(seconds) => self.apply_performance(SLEEP, u8::try_from(seconds / 10).ok()?),
            Setting::AngleSnap(v) => self.apply_performance(ANGLE_SNAP, v.into()),
            Setting::Ripple(v) => self.apply_performance(RIPPLE, v.into()),
        }
    }

    fn apply_performance(&mut self, addr: u16, v: u8) -> Option<(u16, u8)> {
        // The whole block is written: refuse if another field of it is corrupted, so we
        // never write back garbage.
        let others_ok = (0..PERFORMANCE_LEN as u16 / 2)
            .map(|i| PERFORMANCE_BLOCK + i * 2)
            .all(|a| a == addr || self.pair(a).is_some());
        if !others_ok {
            return None;
        }
        self.set_pair(addr, v);
        Some((PERFORMANCE_BLOCK, PERFORMANCE_LEN))
    }

    /// SetEEPROM packet writing the current image content of a block.
    pub fn write_packet(&self, (addr, len): (u16, u8)) -> Packet {
        let start = (addr - IMAGE_START) as usize;
        protocol::set_eeprom(addr, &self.bytes[start..start + len as usize]).expect("block fits one packet")
    }

    /// Update the image with data read back from the mouse.
    pub fn update(&mut self, addr: u16, data: &[u8]) {
        for (i, &b) in data.iter().enumerate() {
            let a = addr + i as u16;
            if (IMAGE_START..IMAGE_END).contains(&a) {
                self.set_byte(a, b);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// EEPROM 0x0000-0x00BF read from a VXE R1 SE+ (firmware 3.15, CID/MID 2,31).
    const REAL_IMAGE: [u8; 0xc0] = [
        0x01, 0x54, 0x04, 0x51, 0x01, 0x54, 0x00, 0x55, 0x00, 0x55, 0x01, 0x54, 0x09, 0x09, 0x00, 0x43, //
        0x12, 0x12, 0x00, 0x31, 0x25, 0x25, 0x00, 0x0B, 0x4B, 0x4B, 0x00, 0xBF, 0x4B, 0x4B, 0x00, 0xBF, //
        0x4B, 0x4B, 0x00, 0xBF, 0x4B, 0x4B, 0x00, 0xBF, 0x4B, 0x4B, 0x00, 0xBF, 0xFF, 0x00, 0x00, 0x56, //
        0x00, 0xFF, 0x00, 0x56, 0x00, 0x00, 0xFF, 0x56, 0xFF, 0x00, 0xFF, 0x57, 0xFF, 0x00, 0xFF, 0x57, //
        0xFF, 0x00, 0xFF, 0x57, 0xFF, 0x00, 0xFF, 0x57, 0xFF, 0x00, 0xFF, 0x57, 0x02, 0x53, 0x80, 0xD5, //
        0x03, 0x52, 0x00, 0x55, 0xFF, 0x00, 0xFF, 0x57, 0x00, 0x55, 0x80, 0xD5, 0x03, 0x52, 0x00, 0x55, //
        0x01, 0x01, 0x00, 0x53, 0x01, 0x02, 0x00, 0x52, 0x01, 0x04, 0x00, 0x50, 0x01, 0x08, 0x00, 0x4C, //
        0x01, 0x10, 0x00, 0x44, 0x02, 0x01, 0x00, 0x52, 0x00, 0x00, 0x00, 0x55, 0x00, 0x00, 0x00, 0x55, //
        0x07, 0x00, 0x00, 0x4E, 0x02, 0x02, 0x00, 0x51, 0x02, 0x03, 0x00, 0x50, 0x04, 0x0A, 0x03, 0x44, //
        0x00, 0x00, 0x00, 0x55, 0x00, 0x00, 0x00, 0x55, 0x00, 0x00, 0x00, 0x55, 0x00, 0x00, 0x00, 0x55, //
        0x01, 0xFF, 0x00, 0xFF, 0x07, 0x09, 0x46, 0x00, 0x55, 0x08, 0x4D, 0x01, 0x54, 0x06, 0x4F, 0x00, //
        0x55, 0x00, 0x55, 0x00, 0x55, 0x00, 0x55, 0x06, 0x4F, 0x00, 0x55, 0x0A, 0x4B, 0x00, 0x55, 0x00, //
    ];

    fn real() -> Config {
        Config::from_bytes(REAL_IMAGE.to_vec()).unwrap()
    }

    #[test]
    fn settings_from_real_image() {
        let settings: Vec<String> = real().settings().iter().map(|s| s.to_string()).collect();
        assert_eq!(
            settings,
            [
                "DPI Stage 1: 400 DPI",
                "DPI Stage 2: 800 DPI",
                "DPI Stage 3: 1600 DPI",
                "DPI Stage 4: 3200 DPI",
                "DPI Stages: 4",
                "Active DPI Stage: 2",
                "Polling Rate: 1000 Hz",
                "Lift-off Distance: 1 mm",
                "Debounce Time: 8 ms",
                "Motion Sync: Enabled",
                "Sleep After: 1 min",
                "Angle Snapping: Disabled",
                "Ripple Control: Disabled",
            ]
        );
    }

    #[test]
    fn rejects_wrong_image_size() {
        assert!(Config::from_bytes(vec![0; 10]).is_err());
    }

    #[test]
    fn polling_rate_write_matches_hardware_test() {
        let mut config = real();
        let block = config.apply(&Setting::PollingRate(500)).unwrap();
        assert_eq!(block, (0x00, 2));
        // The exact packet verified on the mouse (see PROTOCOL.md).
        assert_eq!(
            config.write_packet(block),
            protocol::set_eeprom(0x00, &[0x02, 0x53]).unwrap()
        );
        assert!(config.settings().contains(&Setting::PollingRate(500)));
    }

    #[test]
    fn dpi_stage_write_uses_pairs_and_paw3395_codes() {
        let mut config = real();
        // Stage 2 (index 1) to 1600 DPI: code 0x25; the pair with stage 1 is written.
        let block = config
            .apply(&Setting::DpiStage {
                index: 1,
                dpi: Some(1600),
            })
            .unwrap();
        assert_eq!(block, (0x0c, 8));
        assert_eq!(
            config.write_packet(block),
            protocol::set_eeprom(0x0c, &[0x09, 0x09, 0x00, 0x43, 0x25, 0x25, 0x00, 0x0b]).unwrap()
        );
        assert_eq!(config.stage_dpi(1), Some(1600));
        // Stage 4 (index 3) belongs to the second pair.
        assert_eq!(
            config.apply(&Setting::DpiStage {
                index: 3,
                dpi: Some(800)
            }),
            Some((0x14, 8))
        );
        // Values outside the code table are refused.
        assert_eq!(
            config.apply(&Setting::DpiStage {
                index: 0,
                dpi: Some(12_000)
            }),
            None
        );
        assert_eq!(config.apply(&Setting::DpiStage { index: 0, dpi: None }), None);
    }

    #[test]
    fn performance_block_is_written_whole_with_current_values() {
        let mut config = real();
        let block = config.apply(&Setting::MotionSync(false)).unwrap();
        assert_eq!(block, (0xa9, 10));
        // Debounce 8, motion sync off, sleep 60 s, angle snap off, ripple off: all complements valid.
        assert_eq!(
            config.write_packet(block),
            protocol::set_eeprom(0xa9, &[0x08, 0x4d, 0x00, 0x55, 0x06, 0x4f, 0x00, 0x55, 0x00, 0x55]).unwrap()
        );
        config.apply(&Setting::Sleep(300)).unwrap();
        assert!(config.settings().contains(&Setting::Sleep(300)));
    }

    #[test]
    fn performance_block_refused_if_another_field_is_corrupted() {
        let mut bytes = REAL_IMAGE.to_vec();
        bytes[0xb2] = 0x00; // ripple control complement
        let mut config = Config::from_bytes(bytes).unwrap();
        assert_eq!(config.apply(&Setting::Debounce(4)), None);
        assert!(!config.settings().iter().any(|s| matches!(s, Setting::Ripple(_))));
    }

    #[test]
    fn adjust_ranges_and_options() {
        let mut dpi = Setting::DpiStage {
            index: 0,
            dpi: Some(MAX_DPI),
        };
        dpi.adjust(1, 4);
        assert_eq!(
            dpi,
            Setting::DpiStage {
                index: 0,
                dpi: Some(MAX_DPI)
            }
        );

        let mut rate = Setting::PollingRate(1000);
        rate.adjust(1, 4);
        assert_eq!(rate, Setting::PollingRate(1000));
        for expected in [500, 250, 125, 125] {
            rate.adjust(-1, 4);
            assert_eq!(rate, Setting::PollingRate(expected));
        }

        let mut active = Setting::ActiveStage(3);
        active.adjust(1, 4);
        assert_eq!(active, Setting::ActiveStage(3));

        let mut sleep = Setting::Sleep(60);
        sleep.adjust(1, 4);
        assert_eq!(sleep, Setting::Sleep(120));
        let mut odd = Setting::Sleep(90);
        odd.adjust(-1, 4);
        assert_eq!(odd, Setting::Sleep(60));

        let mut count = Setting::StageCount(8);
        count.adjust(1, 8);
        assert_eq!(count, Setting::StageCount(8));
    }

    #[test]
    fn update_from_read_back() {
        let mut config = real();
        config.update(0x00, &[0x02, 0x53]);
        assert!(config.settings().contains(&Setting::PollingRate(500)));
        config.update(0xbe, &[0x00, 0x00, 0x00, 0x00]); // partly outside the image: ignored there
    }
}
