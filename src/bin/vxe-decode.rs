//! Decode captured VXE packets.
//!
//! Reads text lines (a file or stdin) such as `tshark -T fields` output or the
//! OpenVXE sniffer log, finds the hex data on each line and prints the decoded packet.

use openvxe::protocol;
use std::io::{self, BufRead, BufReader};

const USAGE: &str = "Usage: vxe-decode [FILE]

Decode VXE packets from text lines (stdin if FILE is omitted). Each line may contain
any prefix (timestamp, direction...) followed by the packet as hex, e.g.:

  0.123  host  1.5.0  0807000000...
  TX 08 07 00 00 0c 08 04 04 00 4d 10 10 00 35 00 00 88

See docs/CAPTURE.md for how to produce such lines with tshark.";

/// Split a line into (prefix, packet bytes), or None if it has no packet data.
fn extract_packet(line: &str) -> Option<(String, Vec<u8>)> {
    let tokens: Vec<&str> = line.split_whitespace().collect();
    let is_byte = |t: &str| t.len() == 2 && t.chars().all(|c| c.is_ascii_hexdigit());

    // A run of at least 8 single-byte tokens: "08 07 00 ...".
    let mut i = 0;
    while i < tokens.len() {
        let start = i;
        while i < tokens.len() && is_byte(tokens[i]) {
            i += 1;
        }
        if i - start >= 8 {
            let bytes = protocol::parse_hex(&tokens[start..i].join(" "))?;
            return Some((tokens[..start].join(" "), bytes));
        }
        i += 1;
    }

    // Otherwise the last token that is a packet-sized hex string: "0807..." or "08:07:...".
    tokens.iter().enumerate().rev().find_map(|(idx, t)| {
        let bytes = protocol::parse_hex(t)?;
        (bytes.len() >= 16).then(|| (tokens[..idx].join(" "), bytes))
    })
}

fn main() -> io::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let reader: Box<dyn BufRead> = match args.as_slice() {
        [] => Box::new(io::stdin().lock()),
        [flag] if flag == "-h" || flag == "--help" => {
            println!("{}", USAGE);
            return Ok(());
        }
        [path] => Box::new(BufReader::new(std::fs::File::open(path)?)),
        _ => {
            eprintln!("{}", USAGE);
            std::process::exit(2);
        }
    };

    for line in reader.lines() {
        let line = line?;
        if let Some((prefix, bytes)) = extract_packet(&line) {
            println!("{} {}", prefix, protocol::to_hex(&bytes));
            println!("    -> {}", protocol::describe(&bytes));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_spaced_bytes_with_prefix() {
        let (prefix, bytes) = extract_packet("1.25 TX 08 07 00 00 0c 08 04 04 00 4d 10").unwrap();
        assert_eq!(prefix, "1.25 TX");
        assert_eq!(bytes.len(), 11);
    }

    #[test]
    fn extracts_tshark_hex_field() {
        let line = "0.500\thost\t1.5.0\t\t08:17:00:00:00:00:00:00:00:00:00:00:00:00:00:00:36";
        let (prefix, bytes) = extract_packet(line).unwrap();
        assert_eq!(prefix, "0.500 host 1.5.0");
        assert_eq!(bytes[1], 0x17);
    }

    #[test]
    fn ignores_lines_without_packets() {
        assert_eq!(extract_packet("0.1 host 1.5.0"), None);
        assert_eq!(extract_packet("12 34"), None);
    }
}
