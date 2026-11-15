//! The analysis-oriented commands: disassembly, string search and chartable sets.

use dez80::Instruction;
use dskmanager::*;

/// Disassemble a byte range as Z80, showing undecodable bytes as `DB`.
pub fn disassemble_z80(data: &[u8]) {
    let mut address: u16 = 0;

    while (address as usize) < data.len() {
        // Always work from the remaining bytes by address: the decoder may
        // consume a prefix byte and then fail, which would otherwise leave us
        // with an empty slice and nowhere to read the problem byte from.
        let mut remaining = &data[address as usize..];
        let before = remaining.len();

        match Instruction::decode_one(&mut remaining) {
            Ok(instruction) if remaining.len() < before => {
                let consumed = before - remaining.len();
                let bytes: Vec<String> = data[address as usize..address as usize + consumed]
                    .iter()
                    .map(|b| format!("{:02X}", b))
                    .collect();
                println!("{:04X}  {:<12} {}", address, bytes.join(" "), instruction);
                address += consumed as u16;
            }
            _ => {
                let byte = data[address as usize];
                println!("{:04X}  {:02X}           DB {:02X}h", address, byte, byte);
                address += 1;
            }
        }
    }
}

/// A string found on the disk, with its location.
pub struct StringHit {
    /// Side the string was found on.
    pub side: usize,
    /// Track number.
    pub track: u8,
    /// Sector ID.
    pub sector: u8,
    /// Byte offset within the sector.
    pub offset: usize,
    /// The matched text.
    pub text: String,
}

/// Conservative English-like character set: letters, digits and common
/// punctuation, so random byte runs do not read as words.
pub fn default_ascii_chars() -> Vec<u8> {
    let mut chars = Vec::new();
    chars.extend(b'A'..=b'Z');
    chars.extend(b'a'..=b'z');
    chars.extend(b'0'..=b'9');
    chars.extend(b" !\"'()*+,-.:;=?".iter());
    chars
}

/// Parse a charset such as `A-Za-z0-9` or `32-126`.
///
/// Ranges may be written as characters (`A-Z`) or as decimal byte values
/// (`32-126`), since the help text advertises both forms.
pub fn parse_charset(spec: &str) -> Vec<u8> {
    let mut chars = Vec::new();

    for token in spec.split(',') {
        let token = token.trim();
        if token.is_empty() {
            continue;
        }

        // A numeric range: "32-126".
        if let Some((start, end)) = token.split_once('-') {
            match (parse_byte(start), parse_byte(end)) {
                (Some(start), Some(end)) if start <= end => {
                    chars.extend(start..=end);
                    continue;
                }
                _ => {}
            }
            // A character range: "A-Z".
            let mut iter = token.chars();
            if let (Some(start), Some('-'), Some(end)) = (iter.next(), iter.next(), iter.next()) {
                let (start, end) = (start as u8, end as u8);
                if start <= end {
                    chars.extend(start..=end);
                    continue;
                }
            }
            continue;
        }

        // A single character or a decimal byte.
        if let Some(byte) = parse_byte(token).or_else(|| {
            let mut chars = token.chars();
            let first = chars.next();
            if chars.next().is_none() {
                first.map(|c| c as u8)
            } else {
                None
            }
        }) {
            chars.push(byte);
        }
    }

    if chars.is_empty() {
        default_ascii_chars()
    } else {
        chars
    }
}

fn parse_byte(text: &str) -> Option<u8> {
    text.parse::<u8>().ok()
}

/// Lower bound on the sector ID, used to pick a default sector to disassemble.
pub fn find_lowest_sector_id(image: &DiskImage, side: u8, track: u8) -> Option<u8> {
    image
        .disks()
        .get(side as usize)?
        .get_track(track)?
        .sectors()
        .iter()
        .map(|sector| sector.id.sector)
        .min()
}

/// Search every sector, in logical order, for runs of `charset` characters.
pub fn find_strings_in_disk(
    image: &DiskImage,
    min_length: usize,
    min_unique: usize,
    charset: &[u8],
) -> Vec<StringHit> {
    let mut hits = Vec::new();

    match image.spec().side_mode {
        SideMode::SingleSide => {
            if let Some(disk) = image.get_disk(0) {
                for track in 0..disk.track_count() {
                    find_strings_in_track(disk, 0, track as u8, min_length, min_unique, charset, &mut hits);
                }
            }
        }
        SideMode::Alternate => {
            let max_tracks = image
                .disks()
                .iter()
                .map(|disk| disk.track_count())
                .max()
                .unwrap_or(0);
            for track in 0..max_tracks {
                for (side, disk) in image.disks().iter().enumerate() {
                    find_strings_in_track(disk, side, track as u8, min_length, min_unique, charset, &mut hits);
                }
            }
        }
        SideMode::Successive => {
            for (side, disk) in image.disks().iter().enumerate() {
                for track in 0..disk.track_count() {
                    find_strings_in_track(disk, side, track as u8, min_length, min_unique, charset, &mut hits);
                }
            }
        }
    }

    hits
}

fn find_strings_in_track(
    disk: &Disk,
    side: usize,
    track_num: u8,
    min_length: usize,
    min_unique: usize,
    charset: &[u8],
    hits: &mut Vec<StringHit>,
) {
    let Some(track) = disk.get_track(track_num) else {
        return;
    };

    let mut sector_ids: Vec<u8> = track.sectors().iter().map(|s| s.id.sector).collect();
    sector_ids.sort();

    for sector_id in sector_ids {
        let Some(sector) = track.get_sector(sector_id) else {
            continue;
        };

        let mut current = String::new();
        let mut start_offset = 0;
        let record = |text: &mut String, start: usize, hits: &mut Vec<StringHit>| {
            if text.chars().count() >= min_length
                && unique_char_count(text) >= min_unique
            {
                hits.push(StringHit {
                    side,
                    track: track_num,
                    sector: sector_id,
                    offset: start,
                    text: std::mem::take(text),
                });
            } else {
                text.clear();
            }
        };

        for (i, &byte) in sector.data().iter().enumerate() {
            if charset.contains(&byte) {
                if current.is_empty() {
                    start_offset = i;
                }
                current.push(byte as char);
            } else {
                record(&mut current, start_offset, hits);
            }
        }
        // The final run is not terminated by a non-charset byte.
        record(&mut current, start_offset, hits);
    }
}

fn unique_char_count(text: &str) -> usize {
    let mut seen = std::collections::HashSet::new();
    text.chars().for_each(|c| {
        seen.insert(c);
    });
    seen.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_charset_expands_char_and_numeric_ranges() {
        assert_eq!(parse_charset("A-C"), vec![b'A', b'B', b'C']);
        // The help text advertises numeric ranges too.
        assert_eq!(parse_charset("32-34"), vec![32, 33, 34]);
        assert_eq!(parse_charset("A,B"), vec![b'A', b'B']);
        // A reversed range contributes nothing, so the default set applies.
        assert_eq!(parse_charset("C-A"), default_ascii_chars());
        // Empty input falls back rather than returning nothing.
        assert_eq!(parse_charset(""), default_ascii_chars());
    }

    #[test]
    fn default_charset_excludes_control_bytes() {
        let chars = default_ascii_chars();
        assert!(chars.contains(&b'A'));
        assert!(!chars.contains(&0x00));
        assert!(!chars.contains(&0xE5));
    }

    #[test]
    fn finds_strings_and_reports_their_location() {
        let mut image = DiskImage::builder()
            .num_tracks(1)
            .sectors_per_track(1)
            .build()
            .unwrap();
        image
            .write_sector(0, 0, 0xC1, &{
                let mut data = vec![0u8; 512];
                data[8..20].copy_from_slice(b"Hello world!");
                data
            })
            .unwrap();

        let hits = find_strings_in_disk(&image, 8, 3, &default_ascii_chars());
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].text, "Hello world!");
        assert_eq!((hits[0].side, hits[0].track, hits[0].sector), (0, 0, 0xC1));
        assert_eq!(hits[0].offset, 8);
    }

    #[test]
    fn short_or_low_variety_runs_are_not_reported() {
        let mut image = DiskImage::builder()
            .num_tracks(1)
            .sectors_per_track(1)
            .build()
            .unwrap();
        image
            .write_sector(0, 0, 0xC1, &{
                let mut data = vec![0u8; 512];
                // Long, but only one distinct character.
                data[0..40].fill(b'A');
                // Varied, but too short.
                data[100..103].copy_from_slice(b"abc");
                data
            })
            .unwrap();

        assert!(find_strings_in_disk(&image, 8, 3, &default_ascii_chars()).is_empty());
    }

    #[test]
    fn lowest_sector_id_handles_missing_tracks() {
        let image = DiskImage::builder().num_tracks(1).sectors_per_track(2).build().unwrap();
        assert_eq!(find_lowest_sector_id(&image, 0, 0), Some(0xC1));
        assert_eq!(find_lowest_sector_id(&image, 0, 99), None);
        assert_eq!(find_lowest_sector_id(&image, 9, 0), None);
    }

    #[test]
    fn disassembly_covers_valid_and_invalid_opcodes() {
        // NOP NOP then 0xDD alone (truncated prefix).
        disassemble_z80(&[0x00, 0x00, 0xDD]);
    }
}
