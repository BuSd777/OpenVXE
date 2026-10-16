//! HID discovery and I/O.

use crate::protocol::{Packet, VXE_PID, VXE_VID};
use hidapi::{HidApi, HidDevice, HidError};

/// First vendor-defined HID usage page.
const VENDOR_USAGE_PAGE: u16 = 0xff00;

#[derive(Debug, thiserror::Error)]
pub enum DeviceError {
    #[error("device not found: is the dongle plugged in?")]
    NotFound,
    #[error("device found but it cannot be opened: install the udev rules (see README) and replug the dongle")]
    NoAccess,
    #[error(transparent)]
    Hid(#[from] HidError),
}

/// Open the mouse interfaces in non-blocking mode, once per device path.
///
/// hidapi lists one entry per top-level collection, so the same path (e.g. one hidraw
/// node) can appear several times. Only paths exposing a vendor usage page are opened;
/// if no entry reports usage pages, every path is opened.
///
/// Fails with [`DeviceError::NotFound`] or [`DeviceError::NoAccess`] if nothing could be
/// opened. The device list is the one of the last [`HidApi::refresh_devices`].
pub fn open_devices(api: &HidApi) -> Result<Vec<HidDevice>, DeviceError> {
    let entries: Vec<_> = api
        .device_list()
        .filter(|info| info.vendor_id() == VXE_VID && info.product_id() == VXE_PID)
        .collect();
    if entries.is_empty() {
        return Err(DeviceError::NotFound);
    }
    let has_vendor_page = entries.iter().any(|info| info.usage_page() >= VENDOR_USAGE_PAGE);

    let mut paths = Vec::new();
    for info in entries {
        if (!has_vendor_page || info.usage_page() >= VENDOR_USAGE_PAGE) && !paths.contains(&info.path()) {
            paths.push(info.path());
        }
    }

    let mut devices = Vec::new();
    for path in paths {
        if let Ok(dev) = api.open_path(path) {
            // A blocking device would freeze the UI loop, so skip it.
            if dev.set_blocking_mode(false).is_ok() {
                devices.push(dev);
            }
        }
    }
    if devices.is_empty() {
        return Err(DeviceError::NoAccess);
    }
    Ok(devices)
}

/// Send a packet as an output report (what the official WebHID software does) to every
/// interface. Succeeds if at least one interface accepted it.
pub fn send(devices: &[HidDevice], packet: &Packet) -> Result<(), DeviceError> {
    let mut result = Err(DeviceError::NotFound);
    for dev in devices {
        match dev.write(packet) {
            Ok(_) => result = Ok(()),
            Err(err) if result.is_err() => result = Err(err.into()),
            Err(_) => {}
        }
    }
    result
}

/// Drain all pending input reports from every interface.
///
/// A read error means the device is gone (e.g. the dongle was unplugged).
pub fn read_all(devices: &[HidDevice]) -> Result<Vec<Vec<u8>>, DeviceError> {
    let mut reports = Vec::new();
    for dev in devices {
        let mut buf = [0u8; 64];
        loop {
            let sz = dev.read(&mut buf)?;
            if sz == 0 {
                break;
            }
            reports.push(buf[..sz].to_vec());
        }
    }
    Ok(reports)
}

/// One line per HID interface of the mouse: interface number, usage page/usage and path.
pub fn describe_interfaces(api: &HidApi) -> Vec<String> {
    api.device_list()
        .filter(|info| info.vendor_id() == VXE_VID && info.product_id() == VXE_PID)
        .map(|info| {
            format!(
                "interface {} usage_page=0x{:04X} usage=0x{:04X} path={}",
                info.interface_number(),
                info.usage_page(),
                info.usage(),
                info.path().to_string_lossy()
            )
        })
        .collect()
}

/// Read the feature report `report_id` from every interface that answers.
pub fn get_feature_reports(devices: &[HidDevice], report_id: u8) -> Vec<Vec<u8>> {
    let mut reports = Vec::new();
    for dev in devices {
        let mut buf = [0u8; 64];
        buf[0] = report_id;
        if let Ok(sz) = dev.get_feature_report(&mut buf) {
            if sz > 0 {
                reports.push(buf[..sz].to_vec());
            }
        }
    }
    reports
}
