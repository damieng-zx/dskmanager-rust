//! Argument and value parsing for the `dsk` console.
//!
//! Command implementations live in the sibling modules; this holds the helpers
//! they share for turning user input into typed values.

use dskmanager::*;


/// Split a command line into arguments, respecting double quotes.
pub fn parse_command_line(input: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;

    for ch in input.chars() {
        match ch {
            '"' => in_quotes = !in_quotes,
            ' ' | '\t' if !in_quotes => {
                if !current.is_empty() {
                    parts.push(std::mem::take(&mut current));
                }
            }
            _ => current.push(ch),
        }
    }

    if !current.is_empty() {
        parts.push(current);
    }

    parts
}

/// Parse a decimal byte or a `0xNN` hexadecimal byte.
pub fn parse_hex_or_dec(text: &str) -> Option<u8> {
    if let Some(hex) = text.strip_prefix("0x").or_else(|| text.strip_prefix("0X")) {
        u8::from_str_radix(hex, 16).ok()
    } else {
        text.parse().ok()
    }
}

/// Parse the `<side> <track> <sector>` argument triple.
pub fn parse_sector_address(parts: &[String]) -> std::result::Result<(u8, u8, u8), String> {
    let parse = |index: usize| {
        parts
            .get(index)
            .and_then(|value| parse_hex_or_dec(value))
            .ok_or_else(|| {
                format!(
                    "Invalid side/track/sector number: {}",
                    parts.get(index).map(String::as_str).unwrap_or("")
                )
            })
    };
    Ok((parse(1)?, parse(2)?, parse(3)?))
}

/// Print a hex dump of at most `max_bytes` bytes.
pub fn print_hex_dump(data: &[u8], max_bytes: usize) {
    let len = data.len().min(max_bytes);

    for (i, chunk) in data[..len].chunks(16).enumerate() {
        print!("{:04X}: ", i * 16);

        for (j, byte) in chunk.iter().enumerate() {
            print!("{:02X} ", byte);
            if j == 7 {
                print!(" ");
            }
        }

        for j in chunk.len()..16 {
            print!("   ");
            if j == 7 {
                print!(" ");
            }
        }

        print!(" |");

        for byte in chunk {
            let c = if (32..127).contains(byte) {
                *byte as char
            } else {
                '.'
            };
            print!("{}", c);
        }

        println!("|");
    }

    if data.len() > max_bytes {
        println!("... ({} more bytes)", data.len() - max_bytes);
    }
}

/// Pick the filesystem to format: an explicit request wins, otherwise reuse the
/// session mode.
pub fn resolve_format_mode(
    current: FileSystemType,
    requested: Option<FileSystemType>,
) -> FileSystemType {
    requested.unwrap_or(current)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fs_format_argument_selects_the_mode_for_later_commands() {
        // `fs-format mgt` must both format and switch the session mode, so the
        // next listing reads the fresh disk with the new driver.
        assert_eq!(
            resolve_format_mode(FileSystemType::Auto, Some(FileSystemType::Mgt)),
            FileSystemType::Mgt
        );
        assert_eq!(
            resolve_format_mode(FileSystemType::Cpm, None),
            FileSystemType::Cpm
        );
    }

    #[test]
    fn parse_command_line_splits_on_whitespace_and_respects_quotes() {
        assert_eq!(parse_command_line("open game.dsk"), vec!["open", "game.dsk"]);
        assert_eq!(
            parse_command_line("open \"my disk.dsk\""),
            vec!["open", "my disk.dsk"]
        );
        assert_eq!(
            parse_command_line("  save\tout.dsk  "),
            vec!["save", "out.dsk"]
        );
        assert!(parse_command_line("   ").is_empty());
        // An unterminated quote still yields the accumulated text.
        assert_eq!(parse_command_line("open \"half"), vec!["open", "half"]);
    }

    #[test]
    fn parse_hex_or_dec_accepts_both_forms() {
        assert_eq!(parse_hex_or_dec("0xC1"), Some(0xC1));
        assert_eq!(parse_hex_or_dec("193"), Some(193));
        assert_eq!(parse_hex_or_dec("zz"), None);
    }

    #[test]
    fn parse_sector_address_reads_the_triple() {
        let parts: Vec<String> = ["read", "0", "0x1", "0xC1"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert_eq!(parse_sector_address(&parts), Ok((0, 1, 0xC1)));

        let bad: Vec<String> = ["read", "x", "0", "1"].iter().map(|s| s.to_string()).collect();
        assert!(parse_sector_address(&bad).is_err());
    }

}
