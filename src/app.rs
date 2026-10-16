//! Application state and the request queue that talks to the mouse.
//!
//! One request is in flight at a time. A request is done when the matching reply arrives
//! (writes are acknowledged by their echo). Without a reply the mouse is considered asleep
//! and the request is retried until it wakes up, so changes are never silently lost.

use hidapi::{HidApi, HidDevice};
use openvxe::config::{self, Config, Setting};
use openvxe::device::{self, DeviceError};
use openvxe::protocol::{self, Packet, Report};
use ratatui::widgets::ListState;
use std::collections::VecDeque;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::time::{Duration, Instant};

const MAX_LOG_LINES: usize = 6;
const BATTERY_INTERVAL: Duration = Duration::from_secs(10);
const REPLY_TIMEOUT: Duration = Duration::from_millis(300);
/// Wait between scans for the dongle while it is not connected.
const RESCAN_INTERVAL: Duration = Duration::from_secs(2);
/// Unanswered attempts after which the mouse is considered asleep.
const ATTEMPTS_BEFORE_ASLEEP: u8 = 2;
/// Wait between attempts while the mouse is asleep.
const RETRY_INTERVAL: Duration = Duration::from_secs(1);
/// Rejected replies (non-zero status) before giving up on a request.
const MAX_REJECTIONS: u8 = 3;
/// CID/MID of the R1 SE+ with PAW3395SE sensor, the only DPI encoding supported.
const PAW3395_CID_MID: (u8, u8) = (2, 31);
/// Environment variable naming a file where every TX/RX packet is appended.
const SNIFF_ENV: &str = "OPENVXE_SNIFF";

#[derive(Clone, Debug, PartialEq)]
enum Purpose {
    Identity,
    Version,
    ReadImage { addr: u16 },
    Battery,
    Write { block: (u16, u8), label: String },
    Resync { block: (u16, u8) },
}

struct Request {
    packet: Packet,
    purpose: Purpose,
    attempts: u8,
    rejections: u8,
}

impl Request {
    fn new(packet: Packet, purpose: Purpose) -> Self {
        Request {
            packet,
            purpose,
            attempts: 0,
            rejections: 0,
        }
    }
}

pub struct App {
    api: HidApi,
    pub devices: Vec<HidDevice>,
    last_scan: Option<Instant>,
    /// Last connection error, logged once instead of at every scan.
    last_scan_error: Option<String>,
    /// Battery percentage and charging state, `None` until the first reply.
    pub battery: Option<(u8, bool)>,
    last_battery_req: Option<Instant>,
    pub state: ListState,
    pub items: Vec<Setting>,
    pub logs: Vec<String>,
    sniff: Option<File>,
    started: Instant,

    queue: VecDeque<Request>,
    in_flight: Option<(Request, Instant)>,
    retry_after: Option<Instant>,
    /// The mouse stopped answering (asleep or out of range).
    pub asleep: bool,
    /// EEPROM bytes collected while the configuration is being read.
    image: Vec<u8>,
    pub config: Option<Config>,
    pub cid_mid: Option<(u8, u8)>,
    pub firmware: Option<String>,
    quit_warned: bool,
}

impl App {
    pub fn new(api: HidApi) -> Self {
        let mut app = App {
            api,
            devices: Vec::new(),
            last_scan: None,
            last_scan_error: None,
            battery: None,
            last_battery_req: None,
            state: ListState::default(),
            items: Vec::new(),
            logs: vec!["System: Initializing OpenVXE...".to_string()],
            sniff: None,
            started: Instant::now(),
            queue: VecDeque::new(),
            in_flight: None,
            retry_after: None,
            asleep: false,
            image: Vec::new(),
            config: None,
            cid_mid: None,
            firmware: None,
            quit_warned: false,
        };

        if let Ok(path) = std::env::var(SNIFF_ENV) {
            match OpenOptions::new().create(true).append(true).open(&path) {
                Ok(file) => {
                    app.sniff = Some(file);
                    app.log(format!("System: Sniffing packets to {}", path));
                }
                Err(err) => app.log(format!("Error: cannot open sniff file {}: {}", path, err)),
            }
        }

        app.connect();
        app.state.select(Some(0));
        app
    }

    /// Look for the dongle and, once it is open, start reading the configuration.
    fn connect(&mut self) {
        self.last_scan = Some(Instant::now());
        let result = self
            .api
            .refresh_devices()
            .map_err(DeviceError::from)
            .and_then(|()| device::open_devices(&self.api));
        match result {
            Ok(devices) => {
                self.devices = devices;
                self.last_scan_error = None;
                self.log(format!(
                    "System: Connected to VXE R1 SE+ ({} interface(s)), reading configuration...",
                    self.devices.len()
                ));
                self.enqueue_startup_reads();
            }
            Err(err) => {
                let msg = err.to_string();
                if self.last_scan_error.as_ref() != Some(&msg) {
                    self.log(format!("Error: {}", msg));
                    self.last_scan_error = Some(msg);
                }
            }
        }
    }

    /// Forget the device and everything read from it, then wait for it to come back.
    fn disconnect(&mut self, err: DeviceError) {
        let lost = self.pending_writes();
        self.devices.clear();
        self.queue.clear();
        self.in_flight = None;
        self.retry_after = None;
        self.asleep = false;
        self.image.clear();
        self.config = None;
        self.items.clear();
        self.cid_mid = None;
        self.firmware = None;
        self.battery = None;
        self.last_battery_req = None;
        self.quit_warned = false;
        self.last_scan = Some(Instant::now());
        self.log(format!("Error: Device disconnected ({}).", err));
        if lost > 0 {
            self.log(format!("Warning: {} change(s) not applied.", lost));
        }
    }

    fn enqueue_startup_reads(&mut self) {
        let identity = protocol::build_packet(&[protocol::CMD_GET_CID_MID]).expect("one byte payload");
        let version = protocol::build_packet(&[protocol::CMD_GET_VERSION]).expect("one byte payload");
        self.queue.push_back(Request::new(identity, Purpose::Identity));
        self.queue.push_back(Request::new(version, Purpose::Version));
        let mut addr = config::IMAGE_START;
        while addr < config::IMAGE_END {
            let len = (config::IMAGE_END - addr).min(protocol::EEPROM_CHUNK as u16) as u8;
            self.queue.push_back(Request::new(
                protocol::eeprom_read(addr, len),
                Purpose::ReadImage { addr },
            ));
            addr += len as u16;
        }
    }

    pub fn log(&mut self, msg: String) {
        self.logs.push(msg);
        if self.logs.len() > MAX_LOG_LINES {
            self.logs.remove(0);
        }
    }

    /// Append a packet to the sniff file, in a format `vxe-decode` understands.
    fn sniff(&mut self, direction: &str, bytes: &[u8]) {
        if let Some(file) = &mut self.sniff {
            let elapsed = self.started.elapsed().as_secs_f64();
            let _ = writeln!(file, "{:.3} {} {}", elapsed, direction, protocol::to_hex(bytes));
        }
    }

    fn send(&mut self, packet: &Packet) -> Result<(), DeviceError> {
        self.sniff("TX", packet);
        device::send(&self.devices, packet)
    }

    pub fn dpi_editable(&self) -> bool {
        self.cid_mid == Some(PAW3395_CID_MID)
    }

    /// Writes not yet acknowledged by the mouse.
    pub fn pending_writes(&self) -> usize {
        let is_write = |r: &Request| matches!(r.purpose, Purpose::Write { .. });
        self.queue.iter().filter(|r| is_write(r)).count() + self.in_flight.iter().filter(|(r, _)| is_write(r)).count()
    }

    /// DPI and index of the active stage, if known.
    pub fn active_dpi(&self) -> Option<(u16, u8)> {
        let config = self.config.as_ref()?;
        let stage = config.active_stage();
        Some((config.stage_dpi(stage)?, stage))
    }

    /// Queue periodic requests, process replies, handle timeouts and send the next request.
    pub fn poll_device(&mut self) {
        if self.devices.is_empty() {
            if self.last_scan.is_none_or(|t| t.elapsed() >= RESCAN_INTERVAL) {
                self.connect();
            }
            return;
        }

        let battery_queued = self
            .queue
            .iter()
            .chain(self.in_flight.iter().map(|(r, _)| r))
            .any(|r| r.purpose == Purpose::Battery);
        if !battery_queued && self.last_battery_req.is_none_or(|t| t.elapsed() >= BATTERY_INTERVAL) {
            self.queue
                .push_back(Request::new(protocol::battery_request(), Purpose::Battery));
            self.last_battery_req = Some(Instant::now());
        }

        let reports = match device::read_all(&self.devices) {
            Ok(reports) => reports,
            Err(err) => return self.disconnect(err),
        };
        for report in reports {
            if matches!(report.first(), Some(0x01 | 0x02)) {
                continue;
            }
            self.sniff("RX", &report);
            let answers_in_flight = self
                .in_flight
                .as_ref()
                .is_some_and(|(r, _)| protocol::is_reply_to(&r.packet, &report));
            if answers_in_flight {
                let (request, _) = self.in_flight.take().expect("checked above");
                self.handle_reply(request, &report);
            } else if protocol::parse_report(&report) == Report::Unknown {
                let hex = protocol::to_hex(&report[..report.len().min(8)]);
                self.log(format!("RX: [{}]", hex));
            }
        }

        if self
            .in_flight
            .as_ref()
            .is_some_and(|(_, sent)| sent.elapsed() >= REPLY_TIMEOUT)
        {
            let (mut request, _) = self.in_flight.take().expect("checked above");
            request.attempts = request.attempts.saturating_add(1);
            if request.attempts >= ATTEMPTS_BEFORE_ASLEEP {
                if !self.asleep {
                    self.asleep = true;
                    self.log("Warning: Mouse not responding (asleep?). Move it to wake it up.".to_string());
                }
                self.retry_after = Some(Instant::now() + RETRY_INTERVAL);
            }
            self.queue.push_front(request);
        }

        let ready = self.retry_after.is_none_or(|t| Instant::now() >= t);
        if self.in_flight.is_none() && ready {
            if let Some(request) = self.queue.pop_front() {
                if let Err(err) = self.send(&request.packet) {
                    self.queue.push_front(request);
                    return self.disconnect(err);
                }
                self.in_flight = Some((request, Instant::now()));
            }
        }
    }

    fn handle_reply(&mut self, mut request: Request, report: &[u8]) {
        if self.asleep {
            self.asleep = false;
            self.log("System: Mouse is responding again.".to_string());
        }
        self.retry_after = None;

        if protocol::reply_status(report) != Some(0) {
            request.rejections = request.rejections.saturating_add(1);
            let essential = matches!(request.purpose, Purpose::ReadImage { .. } | Purpose::Resync { .. });
            if request.rejections < MAX_REJECTIONS || essential {
                // Reads are also rejected while the mouse is not linked to the dongle: retry.
                self.retry_after = Some(Instant::now() + RETRY_INTERVAL);
                self.queue.push_front(request);
            } else if let Purpose::Write { block, label } = request.purpose {
                self.log(format!("Error: Mouse rejected {}; re-reading it.", label));
                let packet = protocol::eeprom_read(block.0, block.1);
                self.queue.push_front(Request::new(packet, Purpose::Resync { block }));
            }
            return;
        }

        let data = protocol::reply_data(report).unwrap_or_default();
        match request.purpose {
            Purpose::Identity => {
                if let [cid, mid, ..] = data[..] {
                    self.cid_mid = Some((cid, mid));
                    if !self.dpi_editable() {
                        self.log(format!(
                            "Warning: Unknown sensor (CID/MID {},{}): DPI stages are read-only.",
                            cid, mid
                        ));
                    }
                }
            }
            Purpose::Version => {
                if let [major, minor, ..] = data[..] {
                    self.firmware = Some(format!("{:x}.{:02x}", major, minor));
                }
            }
            Purpose::ReadImage { addr } => {
                // Chunks arrive in order; drop anything that does not continue the image.
                if config::IMAGE_START as usize + self.image.len() == addr as usize {
                    self.image.extend_from_slice(&data);
                }
                if config::IMAGE_START as usize + self.image.len() >= config::IMAGE_END as usize {
                    let bytes = std::mem::take(&mut self.image);
                    match Config::from_bytes(bytes) {
                        Ok(config) => {
                            self.config = Some(config);
                            self.refresh_items();
                            self.log("System: Configuration loaded from the mouse.".to_string());
                        }
                        Err(err) => self.log(format!("Error: Invalid configuration: {}", err)),
                    }
                }
            }
            Purpose::Battery => {
                if let Report::Battery { percent, charging } = protocol::parse_report(report) {
                    if self.battery != Some((percent, charging)) {
                        self.battery = Some((percent, charging));
                        let state = if charging { " (charging)" } else { "" };
                        self.log(format!("System: Battery {}%{}", percent, state));
                    }
                }
            }
            Purpose::Write { label, .. } => self.log(format!("TX: Applied {}", label)),
            Purpose::Resync { block } => {
                if let Some(config) = &mut self.config {
                    config.update(block.0, &data);
                }
                self.refresh_items();
            }
        }
    }

    fn refresh_items(&mut self) {
        self.items = self.config.as_ref().map(Config::settings).unwrap_or_default();
        let selected = self
            .state
            .selected()
            .unwrap_or(0)
            .min(self.items.len().saturating_sub(1));
        self.state.select(Some(selected));
    }

    /// Queue a write of `block`, merging it with a queued write of the same block.
    fn enqueue_write(&mut self, block: (u16, u8), label: String) {
        let Some(config) = &self.config else { return };
        let packet = config.write_packet(block);
        let queued = self
            .queue
            .iter_mut()
            .find(|r| matches!(&r.purpose, Purpose::Write { block: b, .. } if *b == block));
        match queued {
            Some(request) => {
                request.packet = packet;
                request.purpose = Purpose::Write { block, label };
            }
            None => self
                .queue
                .push_back(Request::new(packet, Purpose::Write { block, label })),
        }
    }

    pub fn select_next(&mut self) {
        if self.items.is_empty() {
            return;
        }
        let i = match self.state.selected() {
            Some(i) if i + 1 < self.items.len() => i + 1,
            _ => 0,
        };
        self.state.select(Some(i));
    }

    pub fn select_prev(&mut self) {
        if self.items.is_empty() {
            return;
        }
        let i = match self.state.selected() {
            Some(0) | None => self.items.len() - 1,
            Some(i) => i - 1,
        };
        self.state.select(Some(i));
    }

    /// Change the selected setting, store it in the configuration image and queue the write.
    pub fn modify_setting(&mut self, dir: i32) {
        let Some(idx) = self.state.selected() else { return };
        let Some(before) = self.items.get(idx).cloned() else {
            return;
        };
        if matches!(before, Setting::DpiStage { .. }) && !self.dpi_editable() {
            self.log("Warning: DPI editing is only supported for the PAW3395SE sensor.".to_string());
            return;
        }
        let Some(config) = &mut self.config else { return };

        let mut setting = before.clone();
        setting.adjust(dir, config.stage_count());
        if setting == before {
            return;
        }
        let Some(block) = config.apply(&setting) else {
            self.log(format!("Error: Cannot encode {}", setting));
            return;
        };
        let mut writes = vec![(block, setting.to_string())];

        // Keep the active stage within the stage count.
        if let Setting::StageCount(count) = setting {
            if config.stored_active_stage().is_some_and(|active| active >= count) {
                let active = Setting::ActiveStage(count - 1);
                if let Some(block) = config.apply(&active) {
                    writes.push((block, active.to_string()));
                }
            }
        }

        for (block, label) in writes {
            self.enqueue_write(block, label);
        }
        self.refresh_items();
    }

    /// Whether the app may quit now. With unapplied changes the first request only warns.
    pub fn request_quit(&mut self) -> bool {
        let pending = self.pending_writes();
        if pending == 0 || self.quit_warned {
            return true;
        }
        self.quit_warned = true;
        self.log(format!(
            "Warning: {} change(s) not applied yet. Press q again to quit anyway.",
            pending
        ));
        false
    }
}
