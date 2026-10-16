# Capturing the official software's USB traffic

Most of the missing features (reading the configuration, DPI above 12,750, DPI stages,
button remapping, sleep timer...) need protocol knowledge that can only come from watching
what the official configuration software sends to the mouse. This guide explains how to
record that traffic and decode it with the tools in this repository.

Much of the protocol is already documented in [PROTOCOL.md](PROTOCOL.md) from public
sources; captures are mainly needed to **confirm** the entries marked ❓ there.

## Start here: read-only checks

These commands only read from the mouse and are safe to run:

```bash
./target/release/vxe-probe --list              # interfaces and usage pages
./target/release/vxe-probe --info              # battery (0x04), CID/MID, firmware, 0x17
./target/release/vxe-probe --dump              # configuration memory 0x0000-0x00C0
```

`--dump` is decoded with the EEPROM map: compare it with what the official software shows.
To check whether the TUI's polling/LOD/debounce/angle-snap writes touch the forward button
(see PROTOCOL.md), run `--dump --from 60 --to 78`, change one of those settings in the TUI,
and dump again.

> ⚠️ Before experimenting, write down (or screenshot) your current configuration in the
> official software so you can restore it. Write commands change the mouse memory.

## Tools in this repository

| Tool | Purpose |
| --- | --- |
| `vxe-decode` | Decodes packets from text lines (tshark output, sniff logs) |
| `vxe-probe` | Sends a single packet to the mouse and prints the responses |
| `OPENVXE_SNIFF=file` | Makes the TUI append every TX/RX packet to `file` |

Build them with `cargo build --release`; binaries end up in `target/release/`.

## Option A — Linux host, official software in a Windows VM (recommended)

1. Load the USB monitor and find the dongle's bus and device number:
   ```bash
   sudo modprobe usbmon
   lsusb | grep 373b
   # Bus 003 Device 005: ID 373b:1085 ...
   ```
2. Pass the dongle through to a Windows VM (virt-manager: *Add Hardware → USB Host
   Device*; VirtualBox: *Devices → USB*) and install the official VXE software there.
   The device number may change after passthrough: run `lsusb` again.
3. Capture on the host bus (here bus 3):
   ```bash
   sudo tshark -i usbmon3 -w vxe.pcapng
   ```
   or open `usbmon3` in Wireshark with the display filter `usb.device_address == 5`.
4. Perform **one action at a time** in the software and write down the time of each
   (see the checklist below). Keep the mouse still: movement reports flood the capture.

## Option B — native Windows

Install Wireshark with **USBPcap**, capture on the USBPcap root hub the dongle is
attached to, then copy the `.pcapng` to Linux (or run the same tshark command on Windows).

## Option C — WebHID web driver in the browser

If a web version of the configuration software is available, it runs in Chrome/Chromium
directly on Linux and can be captured with usbmon as in option A, without a VM.

Even simpler, WebHID calls can be logged from the DevTools console *before* connecting
the device in the page. Paste this snippet in the console:

```js
(() => {
  const hex = (id, data) => [id, ...new Uint8Array(data.buffer ?? data)]
    .map(b => b.toString(16).padStart(2, '0')).join(' ');
  const t0 = performance.now();
  const ts = () => ((performance.now() - t0) / 1000).toFixed(3);
  const p = HIDDevice.prototype;
  for (const name of ['sendReport', 'sendFeatureReport']) {
    const orig = p[name];
    p[name] = function (id, data) {
      console.log(`${ts()} TX ${hex(id, data)}`);
      return orig.call(this, id, data);
    };
  }
  const origRecv = p.receiveFeatureReport;
  p.receiveFeatureReport = async function (id) {
    const view = await origRecv.call(this, id);
    console.log(`${ts()} FEATURE ${hex(id, new Uint8Array(view.buffer).slice(1))}`);
    return view;
  };
  const origOpen = p.open;
  p.open = async function () {
    this.addEventListener('inputreport', e => console.log(`${ts()} RX ${hex(e.reportId, e.data)}`));
    return origOpen.call(this);
  };
})();
```

Then *right click → Save as…* on the console output and feed the file to `vxe-decode`.
Note that the received feature report buffer may already start with the report ID
depending on the browser: check the first bytes and adjust the `slice(1)` if needed.

## Decoding a capture

Export the HID data with tshark and pipe it into the decoder (replace `5` with the
device number from `lsusb`):

```bash
tshark -r vxe.pcapng \
  -Y 'usb.device_address == 5' \
  -T fields -e frame.time_relative -e usb.src -e usb.dst \
  -e usb.data_fragment -e usbhid.data -e usb.capdata \
  | ./target/release/vxe-decode > decoded.txt
```

- `usb.data_fragment` holds SET_REPORT / GET_REPORT data sent over control transfers,
  `usbhid.data` / `usb.capdata` hold interrupt transfers (field names differ between
  Wireshark versions, so all three are exported).
- `usb.src` / `usb.dst` tell the direction: `host → 3.5.0` is TX, `3.5.1 → host` is RX.
- Short reports (mouse movement, buttons) are skipped automatically.

Every packet is printed with its decoding: known commands are named, unknown ones are
split into command / address / length / data, value–complement pairs (`v`, `0x55 - v`)
are highlighted and bad checksums are flagged. Unknown commands and addresses are the
interesting part.

## What to capture

Each item is a separate short capture (or clearly separated by notes with timestamps):

| # | Action in the official software | What it reveals |
| --- | --- | --- |
| 1 | Start the software with the mouse connected | **Config read command** and memory layout (roadmap 2.1) |
| 2 | Set DPI 12,750 → 12,800 → 20,000 → 26,000 | **DPI high byte** (roadmap 1.1) |
| 3 | Set different X and Y DPI, if available | Confirms bytes `p[9]`/`p[10]` |
| 4 | Change the number of DPI stages, select another stage, change a stage colour | DPI stages (roadmap 2.2) |
| 5 | Press the physical DPI button on the mouse | Current-stage notification from the mouse |
| 6 | Change the sleep timer only | Sleep field of the `0xA9` packet (roadmap 1.6) |
| 7 | Remap one button, then reset it | Button mapping (roadmap 2.3) |
| 8 | Record and assign a short macro | Macro storage |
| 9 | Open the "about"/firmware page | Firmware version query |
| 10 | Plug the charging cable while the software is open | Charging flag in the battery response |
| 11 | Connect the mouse with the cable (wired mode) | Wired PID / interfaces (`vxe-probe --list`) |
| 12 | "Restore defaults" | Full list of writable addresses and default values |

## Testing a hypothesis with `vxe-probe`

```bash
./target/release/vxe-probe 04                  # battery level query, prints the reply
./target/release/vxe-probe 08 00 00 0c 08      # GetEEPROM: 8 bytes at 0x000C (DPI stages 1-2)
./target/release/vxe-probe --feature 04        # also read feature report 0x08
./target/release/vxe-probe --listen 10000      # watch what the mouse sends (press DPI button)
./target/release/vxe-probe --yes 07 00 00 0c 08 04 04 00 4d 10 10 00 35   # write DPI 800
```

The report ID `0x08` and the checksum are added automatically. Any command that is not a
known read (writes `0x07`, factory reset `0x09`, firmware upgrade mode `0x0D`, pairing...)
requires `--yes`. Prefer replaying packets seen in a capture over guessing new ones.

Please share interesting findings (decoded captures, not raw personal data) in an issue
or directly as an update to [PROTOCOL.md](PROTOCOL.md).
