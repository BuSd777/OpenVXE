//! Send raw packets to the mouse and print its responses, to test protocol hypotheses.

use anyhow::{anyhow, bail, Context};
use hidapi::HidApi;
use openvxe::{device, protocol};
use std::time::{Duration, Instant};

const USAGE: &str = "Usage:
  vxe-probe --list                       List the HID interfaces of the mouse
  vxe-probe --info                       Query battery, CID/MID, firmware version (read-only)
  vxe-probe --dump [--from HEX --to HEX] Read the configuration memory (read-only,
                                         default 0x0000-0x00C0)
  vxe-probe [OPTIONS] HEX...             Send a packet and print the responses
  vxe-probe --listen MS                  Only print incoming reports for MS milliseconds

HEX is the payload without report ID and checksum (up to 15 bytes, zero padded),
e.g. `vxe-probe 04` sends the battery level query. The 0x08 report ID and the
checksum are added automatically.

Options:
  --raw        Send HEX as-is (17 bytes, report ID and checksum included)
  --listen MS  Read responses for MS milliseconds after sending (default 500)
  --feature    Also read feature report 0x08 after sending
  --yes        Required for any command that is not a known read: writes (0x07),
               factory reset (0x09), firmware upgrade mode (0x0D), pairing...";

/// Commands known to only read state; everything else needs --yes.
const READ_ONLY_COMMANDS: [u8; 12] = [0x02, 0x03, 0x04, 0x06, 0x08, 0x0e, 0x10, 0x12, 0x15, 0x17, 0x19, 0x1d];

/// How long to wait for the answer to one request.
const REPLY_TIMEOUT: Duration = Duration::from_millis(300);

struct Options {
    list: bool,
    info: bool,
    dump: Option<(u16, u16)>,
    raw: bool,
    feature: bool,
    yes: bool,
    listen_ms: u64,
    hex: Vec<String>,
}

/// Parse the command line; `None` means the usage should be printed.
fn parse_args() -> anyhow::Result<Option<Options>> {
    let mut opts = Options {
        list: false,
        info: false,
        dump: None,
        raw: false,
        feature: false,
        yes: false,
        listen_ms: 500,
        hex: Vec::new(),
    };
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-h" | "--help" => return Ok(None),
            "--list" => opts.list = true,
            "--info" => opts.info = true,
            "--dump" => opts.dump = Some(opts.dump.unwrap_or((0x0000, 0x00c0))),
            "--from" | "--to" => {
                let value = args.next().with_context(|| format!("{} needs a value", arg))?;
                let addr = u16::from_str_radix(value.trim_start_matches("0x"), 16)
                    .with_context(|| format!("invalid address: {}", value))?;
                let (from, to) = opts.dump.unwrap_or((0x0000, 0x00c0));
                opts.dump = Some(if arg == "--from" { (addr, to) } else { (from, addr) });
            }
            "--raw" => opts.raw = true,
            "--feature" => opts.feature = true,
            "--yes" => opts.yes = true,
            "--listen" => {
                let ms = args.next().context("--listen needs a value")?;
                opts.listen_ms = ms.parse().with_context(|| format!("invalid --listen value: {}", ms))?;
            }
            _ => opts.hex.push(arg),
        }
    }
    let nothing_to_do = !opts.list
        && !opts.info
        && opts.dump.is_none()
        && opts.hex.is_empty()
        && !std::env::args().any(|a| a == "--listen");
    Ok((!nothing_to_do).then_some(opts))
}

fn build(opts: &Options) -> anyhow::Result<Option<protocol::Packet>> {
    if opts.hex.is_empty() {
        return Ok(None);
    }
    let bytes = protocol::parse_hex(&opts.hex.join(" ")).context("invalid hex payload")?;
    let packet = if opts.raw {
        bytes
            .as_slice()
            .try_into()
            .map_err(|_| anyhow!("--raw needs exactly 17 bytes, got {}", bytes.len()))?
    } else {
        protocol::build_packet(&bytes).with_context(|| format!("payload too long: {} bytes (max 15)", bytes.len()))?
    };
    if packet[0] != protocol::REPORT_ID {
        bail!("report ID must be 0x{:02X}", protocol::REPORT_ID);
    }
    if !READ_ONLY_COMMANDS.contains(&packet[1]) && !opts.yes {
        bail!(
            "command 0x{:02X} is not a known read and may change the mouse state; add --yes to send it",
            packet[1]
        );
    }
    Ok(Some(packet))
}

fn print_report(tag: &str, report: &[u8]) {
    println!("{} {}", tag, protocol::to_hex(report));
    println!("    -> {}", protocol::describe(report));
}

/// Send one request and return the matching reply (same command and address).
fn request(devices: &[hidapi::HidDevice], packet: &protocol::Packet) -> anyhow::Result<Option<Vec<u8>>> {
    device::send(devices, packet)?;
    let start = Instant::now();
    while start.elapsed() < REPLY_TIMEOUT {
        if let Some(report) = device::read_all(devices)?
            .into_iter()
            .find(|r| protocol::is_reply_to(packet, r))
        {
            return Ok(Some(report));
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    Ok(None)
}

fn print_reply(tag: &str, reply: Option<Vec<u8>>) {
    match reply {
        Some(report) => print_report(tag, &report),
        None => println!("{} no reply", tag),
    }
}

fn info(devices: &[hidapi::HidDevice]) -> anyhow::Result<()> {
    for cmd in [
        protocol::CMD_BATTERY_LEVEL,
        protocol::CMD_GET_CID_MID,
        protocol::CMD_GET_VERSION,
        0x17, // GetFarDistanceMode (used as battery query by OpenVXE before 0.2)
    ] {
        let packet = protocol::build_packet(&[cmd]).expect("one byte payload");
        print_reply(&format!("[0x{:02X}]", cmd), request(devices, &packet)?);
    }
    Ok(())
}

/// Read the EEPROM range in chunks, print a hex dump and decode each contiguous run of
/// successfully read bytes as a whole, so records are not split at chunk boundaries.
fn dump(devices: &[hidapi::HidDevice], from: u16, to: u16) -> anyhow::Result<()> {
    let mut missing = 0;
    let mut segments: Vec<(u16, Vec<u8>)> = Vec::new();
    let mut addr = from;
    while addr < to {
        let len = (to - addr).min(protocol::EEPROM_CHUNK as u16) as u8;
        match request(devices, &protocol::eeprom_read(addr, len))? {
            Some(report) if protocol::reply_status(&report) == Some(0) => {
                let data = protocol::reply_data(&report).unwrap_or_default();
                match segments.last_mut() {
                    Some((start, bytes)) if *start as usize + bytes.len() == addr as usize => {
                        bytes.extend_from_slice(&data)
                    }
                    _ => segments.push((addr, data)),
                }
            }
            reply => {
                missing += 1;
                print_reply(&format!("[0x{:04X}]", addr), reply)
            }
        }
        addr += len as u16;
    }

    for (start, bytes) in &segments {
        println!("Hex dump:");
        for (i, line) in bytes.chunks(16).enumerate() {
            println!("  0x{:04X}  {}", *start as usize + i * 16, protocol::to_hex(line));
        }
        println!("Decoded:");
        for field in protocol::eeprom_fields(*start, bytes) {
            println!("  {}", field);
        }
    }
    if missing > 0 {
        bail!(
            "incomplete dump: {} chunk(s) not read (mouse asleep? move it and retry)",
            missing
        );
    }
    Ok(())
}

fn run(opts: Options) -> anyhow::Result<()> {
    let api = HidApi::new().context("cannot initialize HID")?;
    if opts.list {
        for line in device::describe_interfaces(&api) {
            println!("{}", line);
        }
        return Ok(());
    }

    let packet = build(&opts)?;
    let devices = device::open_devices(&api)?;

    if opts.info {
        return info(&devices);
    }
    if let Some((from, to)) = opts.dump {
        return dump(&devices, from, to);
    }

    if let Some(packet) = packet {
        print_report("TX", &packet);
        device::send(&devices, &packet)?;
    }

    let start = Instant::now();
    let listen = Duration::from_millis(opts.listen_ms);
    while start.elapsed() < listen {
        for report in device::read_all(&devices)? {
            // Skip standard mouse input reports.
            if report.first().is_some_and(|&id| id == 0x01 || id == 0x02) {
                continue;
            }
            print_report(&format!("RX +{}ms", start.elapsed().as_millis()), &report);
        }
        std::thread::sleep(Duration::from_millis(5));
    }

    if opts.feature {
        for report in device::get_feature_reports(&devices, protocol::REPORT_ID) {
            print_report("FEATURE", &report);
        }
    }
    Ok(())
}

fn main() -> anyhow::Result<()> {
    match parse_args()? {
        Some(opts) => run(opts),
        None => {
            println!("{}", USAGE);
            Ok(())
        }
    }
}
