# VXE R1 SE+ HID protocol

The VXE R1 SE+ uses the Compx / ATK vendor protocol shared by many VXE, ATK, VGN and
MCHOSE mice. This document collects what is known, **where it comes from and how sure we
are**. Please keep the confidence column honest when you update it.

Confidence levels:

- ✅ **vendor**: read in the official ATK HUB web driver bundle (WebHID JavaScript)
- ✅ **hw**: verified on real hardware by a public project (source given)
- 🟡 **consistent**: inferred, and consistent with the packets OpenVXE already sends
- ❓ **unverified**: single source or guess — needs a capture / `vxe-probe` check
- 🟢 **OpenVXE hw**: verified by OpenVXE on a VXE R1 SE+ (dongle `373b:1085`, firmware
  3.15, CID/MID 2,31) with `vxe-probe --info` / `--dump` (2026-10-15)

## Device identification

| VID:PID | Device | Confidence |
| --- | --- | --- |
| `373b:1085` | "Wireless mouse 1k dongle" — generic receiver, vendor usage page `0xFF04`, usage 2 | ✅ vendor |
| `3554:f58f` | R1 SE / SE+ wired | ❓ (OpenMouse) |
| `3554:f58e`, `3554:fb12` | VXE Mouse 1K Dongle | ✅ vendor (`fb12`), ❓ (`f58e`) |
| `373b:12dc`, `373b:1319` | R1 SE+ CORE26K (NearLink) | ❓ (OpenMouse, from HUB) |

Since `1085` is a generic receiver, the actual mouse model must be identified at runtime with
`GetMouseCIDMID`. Our R1 SE+ reports **CID/MID 2,31** (PAW3395SE) 🟢; other sources report
2,32 and 2,116.

On `373b:1085` hidapi lists one entry per top-level collection of the same hidraw node
(interface 1: usage pages `0x0001`, `0xFF03`, `0x000C`, `0xFF02`, `0xFF04`), plus the keyboard
interface 0. OpenVXE opens each path once, only if it exposes a vendor usage page. 🟢
Run `vxe-probe --list` to see the interfaces and usage pages of your device.

Requests can be sent either as an output report or as a feature report: the mouse answers
both with the same input report 🟢. OpenVXE sends output reports only, like the official
WebHID software.

## Frame

Output report ID `0x08`, 16-byte payload:

| Byte | Meaning |
| --- | --- |
| `[0]` | command |
| `[1]` | status in replies (0 = ok, non-zero = rejected / unsupported) |
| `[2..3]` | EEPROM address, 16-bit big-endian |
| `[4]` | data length (max 10) |
| `[5..14]` | data |
| `[15]` | checksum: `0x55 - (0x08 + sum([0..14]))`, i.e. the payload sums to `0x4D` |

Replies arrive as input reports with the same report ID and echo command and address.
A `SetEEPROM` is acknowledged by echoing the whole request (status 0) after ~80 ms. 🟢

**Sleep:** while the mouse sleeps (sleep timer at `0xAD`) the dongle does not answer or
forward requests: queries get no reply and writes are silently lost. Requests work again
a few seconds after the mouse wakes up. Software must treat "no reply" as "mouse asleep"
and retry, never assume a write succeeded without the echo. 🟢
Confidence: ✅ vendor; checksum verified against a published raw frame (unit test
`eeprom_read_matches_published_frame`).

## Commands

| Code | Name | Notes |
| --- | --- | --- |
| `0x01` | DownLoadData | |
| `0x02` | DownLoadDriverStatus | |
| `0x03` | GetWirelessMouseOnline | |
| `0x04` | **GetBatteryLevel** | see below 🟢 |
| `0x05` / `0x06` | SetWirelessDonglePair / GetWirelessDonglePairResult | |
| `0x07` | **SetEEPROM** | write configuration memory |
| `0x08` | **GetEEPROM** | read configuration memory (some firmware replies with `0x07`) |
| `0x09` | RestoreFactory | ⚠️ |
| `0x0a` | ReportMouseStatus | |
| `0x0d` | EnterUSBUpgradeMode | ⚠️ puts the mouse in bootloader mode |
| `0x0e` / `0x0f` | GetCurrentConfig / SetCurrentConfig | profile bank 0–3 |
| `0x10` | **GetMouseCIDMID** | CID at data `[0]`, MID at data `[1]` 🟢 |
| `0x12` | **GetMouseVersion** | BCD, e.g. `03 15` = 3.15 🟢 |
| `0x13` | DongleExitPair | |
| `0x14` / `0x15` | Set / Get4KRGBMode | |
| `0x16` / `0x17` | Set / **GetFarDistanceMode** | long range mode flag; our mouse replies len 10, all zero 🟢 |
| `0x18` / `0x19` | Set / GetDongleLightMode | |
| `0x1d` | GetDongleVersion | |

Confidence: ✅ vendor (enum read in the HUB bundle).

> OpenVXE used `0x17` as battery query until this was verified: its reply (length 10,
> all zero) was read as `10 * 2 = 20 %`. The TUI now uses `0x04`. 🟢

### Battery (`0x04`)

Request `04 00 … 00`. Reply data (payload offsets): `[5]` percent 0–100 (no ×2),
`[6]` charging (0 = no, 1 = yes), `[7..8]` voltage in mV, big-endian.
Example reply payload `04 00 00 00 02 5F 00 10 00 …` = 95 %, not charging, 4096 mV.
Confidence: 🟢 (reply `04 00 00 00 02 64 00 10 79` = 100 %, not charging, 4217 mV),
❓ charging flag value while charging.

## EEPROM map

Most single-byte settings are stored as **value / complement pairs** (`v`, `0x55 - v`);
records of 4 bytes end with `0x55 - (b0 + b1 + b2)`.

| Address | Field | Encoding | Confidence |
| --- | --- | --- | --- |
| `0x00` | report rate | pair: 1 = 1000, 2 = 500, 4 = 250, 8 = 125 Hz | ✅ vendor, 🟢 (1000 Hz read) |
| `0x02` | DPI stage count | pair | ✅ vendor, 🟢 (4 read) |
| `0x04` | active DPI stage (0-based) | pair | ✅ vendor, 🟢 |
| `0x06`, `0x08` | unknown | pairs, both 0 | 🟢 read, meaning ❓ |
| `0x0a` | lift-off distance | pair: 1 = 1 mm, 2 = 2 mm | ✅ vendor, 🟢 (1 read) |
| `0x0c`–`0x2b` | DPI stages 1–8 | records `[x, y, ex, check]` | ✅ vendor, 🟢 (400/800/1600/3200 defaults) |
| `0x2c`–`0x4b` | DPI stage colours | records `[r, g, b, check]` | 🟢 (red, green, blue, magenta...) |
| `0x4c`–`0x53` | DPI LED: effect, brightness, speed, enable | pairs (`02, 80, 03, 00` read) | ✅ vendor names, 🟢 read |
| `0x54` | logo LED colour | record `[r, g, b, check]` (`FF 00 FF` read) | ✅ vendor, 🟢 |
| `0x58`–`0x5f` | logo LED: effect, brightness, breathing speed, energy saving | pairs (`00, 80, 03, 00` read) | ✅ vendor names, 🟢 read |
| `0x60`–`0x9f` | button / key slots | records `[class, v1, v2, check]`; `0x60` left, `0x64` right, `0x68` middle, `0x6c` back, `0x70` forward, `0x74` bottom | ✅ vendor, 🟢 (all records valid, forward = mouse `0x10`) |
| `0xa0` | decoration light | | ✅ vendor name, meaning ❓ |
| `0xa9` | debounce (ms) | pair | ✅ vendor, 🟢 (8 read) |
| `0xab` | motion sync | pair | ✅ vendor, 🟢 (on) |
| `0xad` | sleep time | pair, **units of 10 s** | ✅ vendor (`raw * 10`), 🟢 (6 = 60 s read) |
| `0xaf` | angle snapping (vendor: linear correction) | pair, 0/1 | ✅ vendor, 🟢 (0 read) |
| `0xb1` | ripple control | pair, 0/1 | ✅ vendor, 🟢 (0 read) |
| `0xb3` | move-close lights | pair | ✅ vendor name, 🟢 (0 read) |
| `0xb5`, `0xb7`, `0xb9` | sensor enable, sensor time, sensor mode | pairs (`00, 06, 00` read) | ✅ vendor names, meaning ❓ |
| `0xbb` | RF TX time | pair (`0A` read) | ✅ vendor name, meaning ❓ |
| `0xbd` | angle | pair (`00` read) | ✅ vendor name, meaning ❓ |
| `0xe3` | rolling (wheel) delay | | ✅ vendor name, not read yet |
| `0x100`– | shortcuts, 32-byte slots | | ❓ |
| `0x300`– | macros, 384-byte slots | | ❓ |

Button classes: 0 disabled, 1 mouse (v1: 1 L, 2 R, 4 M, 8 back, 0x10 forward), 2 DPI
(1 cycle, 2 up, 3 down), 3 h-scroll, 4 fire, 5 shortcut, 6 macro, 7 report rate, 8 light,
9 profile, 0x0a DPI lock, 0x0b wheel. Confidence: ❓ (libatk-rs, MouseMod).

### DPI encoding

Each stage is `[x, y, ex, 0x55 - (x + y + ex)]`. `ex` holds extra bits: low nibble for X,
high nibble for Y. The meaning of `x`/`y` **depends on the sensor**:

- **PAW3395SE** variant (our mouse, CID/MID 2,31): `x`/`y` are an index into a code table
  (not `dpi / 50`): 50–10,000 DPI in 50 DPI steps map to codes 1–235, skipping codes 7, 13,
  20, 26, 33, 40, 46, 53, 60, 66, 73, 80, 86, 93, 100, 106, 113, 120, 126, 133, 140, 146,
  153, 160, 166, 173, 180, 186, 193, 200, 206, 213, 220, 226, 233. 🟢 for 400 → `0x09`,
  800 → `0x12`, 1600 → `0x25`, 3200 → `0x4b` (default stages). Above 10,000 DPI half the
  value is written and an `ex` flag is set ❓ (OpenMouse).
- **CORE26K** variant (up to 26,000 DPI): count = `dpi / 50 - 1`, low byte in `x`/`y`,
  bits 8–9 in `ex` bits 2–3 (Y: 6–7), i.e. `ex = high * 0x44` for X = Y. ❓ (OpenMouse,
  Linux-ATK).

The legacy OpenVXE packet wrote `dpi / 50` with `ex = 0`, which matches neither: on our
PAW3395SE, asking 1600 DPI wrote code `0x20` = **1400 DPI**, and stage 1 was forced to code
`0x04` = **200 DPI**. The TUI now uses the PAW3395SE table and enables DPI editing only when
`GetMouseCIDMID` answers 2,31.

## How the TUI reads and writes (phase 4)

1. At startup: `GetMouseCIDMID`, `GetMouseVersion`, then `GetEEPROM` over `0x00–0xBF` in
   10-byte chunks. The settings shown are decoded from this image; nothing is hardcoded.
2. One request is in flight at a time. A reply matches when it has the same command (or
   `0x07` for `GetEEPROM`) and the same address. No reply within 300 ms twice in a row means
   the mouse is asleep: the request goes back to the head of the queue and is retried every
   second.
3. A change rewrites the smallest block that contains the field, built from the current
   image: a pair (`0x00`, `0x02`, `0x04`, `0x0a`), two DPI stages as one 8-byte block
   (`0x0c`, `0x14`, `0x1c`, `0x24`), or the 10-byte performance block at `0xa9` (debounce,
   motion sync, sleep, angle snapping, ripple). The performance block is not written if any
   of its pairs has an invalid complement.
4. A write is done when the mouse echoes it (~80 ms). Writes to the same block that are
   still queued are merged. If the mouse rejects a write, the block is read back so the UI
   matches the EEPROM again.
5. A read or write error means the dongle is gone: the device is closed, everything read
   from it is forgotten, and the dongle is looked for again every 2 seconds. On reconnect the
   startup reads run again. 🟢 Tested by unplugging and replugging the dongle.

🟢 Tested from the TUI: polling 1000 → 500 → 1000 Hz, both writes echoed, the final
EEPROM identical to the backup (all 192 bytes).

## How the legacy OpenVXE packets map onto this

Re-reading the packets the TUI sent before phase 4 with the map above (`vxe-decode` does this
automatically):

- **DPI** `07 00 00 0c 08 | 04 04 00 4d | X Y 00 C` writes **two stages**: stage 1 fixed
  to code 4 and stage 2 = the chosen value. 🟡 consistent (both records have valid checks).
- **Motion sync** `07 00 00 a9 0a | 08 4d 01 54 05 50 00 00 00 00` writes 10 bytes from
  `0xa9`: **debounce 8 ms, motion sync, sleep 50 s**, then straight-line correction and
  ripple control with *invalid* complements (`00 00`). Toggling motion sync therefore also
  resets debounce and sleep. 🟡 consistent.
- **Polling rate, LOD, debounce, angle snapping** are written as 4 bytes at `0x70`
  (`[register, value, 0, 0x55 - value]`). In the vendor map `0x70` is the **forward
  button** record, and these writes don't have a valid record checksum. No source other
  than OpenVXE describes this scheme. **Tested** (polling 500 Hz packet): the mouse echoes
  it, but the EEPROM does not change — the forward button stays `01 10 00 44` and the report
  rate at `0x00` stays 1000 Hz. The firmware most likely drops records with a bad checksum.
  These writes are harmless but are not persisted; any non-persistent live effect is
  unverified. 🟢
- **A correct write works:** `SetEEPROM 0x0000 len=2 [02 53]` changed the report rate to
  500 Hz (verified by reading it back), and `[01 54]` restored 1000 Hz. 🟢

## Sources

- ATK HUB official web driver, bundle v3.2.27 — <https://hub.atk.pro/>
- OpenMouse-Project/mouse-protocol — <https://github.com/OpenMouse-Project/mouse-protocol>
  (`src/atk/index.ts`, `docs/atk-testing.md`)
- cyberphantom52/libatk-rs — <https://github.com/cyberphantom52/libatk-rs> (`src/types.rs`);
  VoideUI/Linux-ATK — <https://github.com/VoideUI/Linux-ATK>
- LCournollet/MouseMod — <https://github.com/LCournollet/MouseMod/blob/main/PROTOCOL.md>
- ivancacic/vxe-battery-tray — <https://github.com/ivancacic/vxe-battery-tray/blob/main/PROTOCOL.md>
- SpookyyQ ATK Fierce X notes — <https://github.com/SpookyyQ/IPI-Stay-Fly-Driver/blob/main/docs/atk-fierce-x-protocol-notes.md>
- TnZzZHlp/mouse (ATK X1 Pro) — <https://github.com/TnZzZHlp/mouse>
