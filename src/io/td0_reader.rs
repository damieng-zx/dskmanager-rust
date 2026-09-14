/// TeleDisk (TD0) file reader.
///
/// TD0 stores floppy tracks and sector metadata rather than a flat sector
/// dump.  This reader converts those records into the same track/sector model
/// used by the DSK readers.
use crate::error::{DskError, Result};
use crate::fdc::{FdcStatus1, FdcStatus2};
use crate::format::{DiskImageFormat, FormatSpec, SideMode};
use crate::image::{Disk, DiskImage, Sector, SectorId, Track};
use std::fs::File;
use std::io::Read;
use std::path::Path;

const HEADER_SIZE: usize = 12;
const MAX_FILE_SIZE: usize = 4 * 1024 * 1024;
const LAST_TRACK: u8 = 0xFF;
const FLAG_CRC_ERROR: u8 = 0x02;
const FLAG_DELETED: u8 = 0x04;
const FLAG_UNALLOCATED: u8 = 0x10;
const FLAG_NO_DATA: u8 = 0x20;

/// Check if a path has a TeleDisk extension.
pub fn is_td0_file<P: AsRef<Path>>(path: P) -> bool {
    path.as_ref()
        .extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| extension.eq_ignore_ascii_case("td0"))
        .unwrap_or(false)
}

/// Read a TeleDisk image and expose it as a sectorized disk image.
pub fn read_td0<P: AsRef<Path>>(path: P) -> Result<DiskImage> {
    let filename = path
        .as_ref()
        .file_name()
        .and_then(|name| name.to_str())
        .map(str::to_string);

    let mut file = File::open(&path)?;
    let file_size = file.metadata()?.len() as usize;
    if file_size < HEADER_SIZE || file_size > MAX_FILE_SIZE {
        return Err(DskError::invalid_format(format!(
            "TD0 file has unexpected size {} bytes",
            file_size
        )));
    }

    let mut bytes = Vec::with_capacity(file_size);
    file.read_to_end(&mut bytes)?;
    let header = &bytes[..HEADER_SIZE];
    if !((header[0] == b'T' || header[0] == b't') && (header[1] == b'D' || header[1] == b'd')) {
        return Err(DskError::invalid_format("Invalid TD0 signature"));
    }

    let mut warnings = Vec::new();
    let stored_crc = u16::from_le_bytes([header[10], header[11]]);
    if teledisk_crc(&header[..10]) != stored_crc {
        warnings.push("TD0 header CRC is invalid.".to_string());
    }

    let image_data = if header[0] == b't' && (header[4] / 10) % 10 == 2 {
        decode_lzhuf_stream(&bytes[HEADER_SIZE..])?
    } else if header[0] == b't' {
        decode_lzw_stream(&bytes[HEADER_SIZE..])?
    } else {
        bytes[HEADER_SIZE..].to_vec()
    };

    let mut cursor = 0usize;
    if header[7] & 0x80 != 0 {
        cursor = parse_comment(&image_data, &mut cursor, &mut warnings)?;
    }

    let declared_sides = header[9].clamp(1, 2);
    let mut tracks: Vec<(u8, u8, Track)> = Vec::new();
    let mut max_track = 0u8;

    loop {
        let track_header = take(&image_data, &mut cursor, 4, "track header")?;
        let sector_count = track_header[0];
        if sector_count == LAST_TRACK {
            break;
        }

        let track_number = track_header[1];
        let side = track_header[2] & 1;
        if teledisk_crc(&track_header[..3]) as u8 != track_header[3] {
            warnings.push(format!(
                "TD0 track {} side {} header CRC is invalid.",
                track_number, side
            ));
        }

        let mut track = Track::new(track_number, side);
        track.gap3_length = 0x2A;
        track.filler_byte = 0xE5;
        track.recording_mode = if header[5] & 0x80 != 0 {
            crate::image::RecordingMode::FM
        } else {
            crate::image::RecordingMode::MFM
        };
        track.data_rate = crate::image::DataRate::from((header[5] & 3) + 1);

        for _ in 0..sector_count {
            let sector_header = take(&image_data, &mut cursor, 6, "sector header")?;
            let logical_track = sector_header[0];
            let logical_side = sector_header[1] & 1;
            let sector_number = sector_header[2];
            let size_code = sector_header[3];
            if size_code > 7 {
                return Err(DskError::parse(
                    cursor.saturating_sub(6),
                    "Invalid TD0 sector size code",
                ));
            }
            let sector_size = 128usize.checked_shl(size_code as u32).ok_or_else(|| {
                DskError::parse(cursor.saturating_sub(6), "Invalid TD0 sector size")
            })?;
            let flags = sector_header[4];
            let mut status1 = if flags & FLAG_CRC_ERROR != 0 {
                FdcStatus1::new(FdcStatus1::DE)
            } else {
                FdcStatus1::new(0)
            };
            let status2 = if flags & FLAG_DELETED != 0 {
                FdcStatus2::new(FdcStatus2::CM)
            } else {
                FdcStatus2::new(0)
            };

            let data = if flags & (FLAG_UNALLOCATED | FLAG_NO_DATA) != 0 {
                let filler = if flags & FLAG_UNALLOCATED != 0 {
                    0xF6
                } else {
                    0x00
                };
                status1 = FdcStatus1::new(status1.0 | FdcStatus1::ND);
                vec![filler; sector_size]
            } else {
                decode_sector_data(&image_data, &mut cursor, sector_size, &mut warnings)?
            };

            let sector_crc = sector_header[5];
            let mut sector_crc_input = sector_header[..5].to_vec();
            if flags & (FLAG_UNALLOCATED | FLAG_NO_DATA) == 0 {
                sector_crc_input.extend_from_slice(&data);
            }
            if teledisk_crc(&sector_crc_input) as u8 != sector_crc {
                warnings.push(format!(
                    "TD0 track {} side {} sector {} header CRC is invalid.",
                    track_number, side, sector_number
                ));
            }
            let id = SectorId::new(logical_track, logical_side, sector_number, size_code);
            track.add_sector(Sector::with_status(id, status1, status2, data));
        }

        max_track = max_track.max(track_number);
        tracks.push((track_number, side, track));
    }

    if tracks.is_empty() {
        return Err(DskError::invalid_format("TD0 image contains no tracks"));
    }

    let side_count = tracks
        .iter()
        .map(|(_, side, _)| *side as usize + 1)
        .max()
        .unwrap_or(declared_sides as usize)
        .max(declared_sides as usize)
        .min(2) as u8;
    let track_count = max_track.saturating_add(1);
    let mut disks: Vec<Disk> = (0..side_count)
        .map(|side| {
            let mut disk = Disk::with_capacity(side, track_count as usize);
            disk.ensure_track_count(track_count as usize);
            disk
        })
        .collect();

    for (track_number, side, track) in tracks {
        if let Some(disk) = disks.get_mut(side as usize) {
            if let Some(destination) = disk.get_track_mut(track_number) {
                *destination = track;
            }
        }
    }

    let (sectors_per_track, sector_size, first_sector_id, gap3_length, filler_byte) = disks
        .iter()
        .flat_map(|disk| disk.tracks())
        .find(|track| !track.is_empty())
        .map(|track| {
            let first = &track.sectors()[0];
            (
                track.sector_count() as u8,
                first.advertised_size() as u16,
                first.id.sector,
                track.gap3_length,
                track.filler_byte,
            )
        })
        .unwrap_or((0, 512, 1, 0x2A, 0xE5));

    let spec = FormatSpec {
        num_sides: side_count,
        num_tracks: track_count,
        sectors_per_track,
        sector_size,
        first_sector_id,
        gap3_length,
        filler_byte,
        interleave: 1,
        side_mode: if side_count == 1 {
            SideMode::SingleSide
        } else {
            SideMode::Successive
        },
    };

    Ok(DiskImage {
        format: DiskImageFormat::RawTd0,
        spec,
        disks,
        changed: false,
        filename,
        warnings,
    })
}

fn parse_comment(data: &[u8], cursor: &mut usize, warnings: &mut Vec<String>) -> Result<usize> {
    let comment = take(data, cursor, 10, "comment header")?;
    let stored_crc = u16::from_le_bytes([comment[0], comment[1]]);
    let length = u16::from_le_bytes([comment[2], comment[3]]) as usize;
    let comment_data = take(data, cursor, length, "comment data")?;
    let mut crc_input = comment[2..10].to_vec();
    crc_input.extend_from_slice(comment_data);
    if teledisk_crc(&crc_input) != stored_crc {
        warnings.push("TD0 comment CRC is invalid.".to_string());
    }
    Ok(*cursor)
}

fn decode_sector_data(
    data: &[u8],
    cursor: &mut usize,
    sector_size: usize,
    warnings: &mut Vec<String>,
) -> Result<Vec<u8>> {
    let data_header = take(data, cursor, 3, "sector data header")?;
    let block_size = u16::from_le_bytes([data_header[0], data_header[1]]) as usize;
    if block_size < 1 {
        return Err(DskError::parse(
            cursor.saturating_sub(3),
            "Invalid TD0 data block size",
        ));
    }

    let encoding = data_header[2];
    let mut decoded = match encoding {
        0 => {
            let payload = take(data, cursor, block_size - 1, "sector data")?;
            payload.to_vec()
        }
        1 => {
            let payload = take(data, cursor, 4, "sector repeat data")?;
            if payload.len() < 4 {
                return Err(DskError::parse(
                    cursor.saturating_sub(4),
                    "Short TD0 repeat block",
                ));
            }
            let count = u16::from_le_bytes([payload[0], payload[1]]) as usize;
            let pattern = [payload[2], payload[3]];
            pattern.repeat(count)
        }
        2 => decode_rle(data, cursor, sector_size)?,
        _ => {
            return Err(DskError::parse(
                cursor.saturating_sub(3),
                "Unsupported TD0 sector encoding",
            ))
        }
    };

    if decoded.len() != sector_size {
        warnings.push(format!(
            "TD0 sector data has {} bytes; expected {}.",
            decoded.len(),
            sector_size
        ));
        decoded.resize(sector_size, 0);
        decoded.truncate(sector_size);
    }
    Ok(decoded)
}

fn decode_rle(payload: &[u8], cursor: &mut usize, expected_size: usize) -> Result<Vec<u8>> {
    let mut decoded = Vec::with_capacity(expected_size);
    while decoded.len() < expected_size {
        let length = *payload
            .get(*cursor)
            .ok_or_else(|| DskError::parse(*cursor, "Short TD0 RLE block"))?;
        *cursor += 1;
        let count = *payload
            .get(*cursor)
            .ok_or_else(|| DskError::parse(*cursor, "Short TD0 RLE block"))?
            as usize;
        *cursor += 1;
        if count == 0 {
            return Err(DskError::parse(*cursor, "Empty TD0 RLE block"));
        }
        if length == 0 {
            let literal = payload
                .get(*cursor..*cursor + count)
                .ok_or_else(|| DskError::parse(*cursor, "Short TD0 literal block"))?;
            decoded.extend_from_slice(literal);
            *cursor += count;
        } else {
            let pattern_size = (length as usize) * 2;
            let pattern = payload
                .get(*cursor..*cursor + pattern_size)
                .ok_or_else(|| DskError::parse(*cursor, "Short TD0 repeat block"))?;
            *cursor += pattern_size;
            for _ in 0..count {
                decoded.extend_from_slice(pattern);
            }
        }
        if decoded.len() > expected_size {
            decoded.truncate(expected_size);
        }
    }
    Ok(decoded)
}

fn decode_lzw_stream(data: &[u8]) -> Result<Vec<u8>> {
    let mut output = Vec::new();
    let mut cursor = 0usize;
    while cursor < data.len() {
        if data.len() - cursor < 2 {
            return Err(DskError::parse(cursor, "Short TD0 LZW block header"));
        }
        let bit_length = u16::from_le_bytes([data[cursor], data[cursor + 1]]) as usize;
        cursor += 2;
        if bit_length == 0 {
            break;
        }
        let byte_length = (bit_length + 1) / 2;
        let block = data
            .get(cursor..cursor + byte_length)
            .ok_or_else(|| DskError::parse(cursor, "Short TD0 LZW block"))?;
        output.extend(decode_lzw_block(block, bit_length)?);
        cursor += byte_length;
    }
    Ok(output)
}

fn decode_lzw_block(data: &[u8], nibble_length: usize) -> Result<Vec<u8>> {
    let mut nibble_pos = 0usize;
    let mut next_code = 256u16;
    let mut dictionary: Vec<(u16, u8)> = Vec::with_capacity(3840);
    let first = next_lzw_code(data, &mut nibble_pos, nibble_length)?;
    if first > 255 {
        return Err(DskError::parse(0, "Invalid first TD0 LZW code"));
    }

    let mut output = vec![first as u8];
    let mut previous = first;
    while nibble_pos + 3 <= nibble_length {
        let code = next_lzw_code(data, &mut nibble_pos, nibble_length)?;
        let sequence = if code < next_code {
            expand_lzw_code(code, &dictionary)?
        } else if code == next_code {
            let mut sequence = expand_lzw_code(previous, &dictionary)?;
            let first_byte = *sequence
                .first()
                .ok_or_else(|| DskError::parse(0, "Invalid TD0 LZW dictionary entry"))?;
            sequence.push(first_byte);
            sequence
        } else {
            return Err(DskError::parse(0, "Invalid TD0 LZW code"));
        };

        let first_byte = sequence[0];
        output.extend_from_slice(&sequence);
        if next_code < 4096 {
            dictionary.push((previous, first_byte));
            next_code += 1;
        }
        previous = code;
    }
    Ok(output)
}

fn decode_lzhuf_stream(data: &[u8]) -> Result<Vec<u8>> {
    let mut decoder = LzhufDecoder::new(data);
    let mut output = Vec::new();
    while output.len() < MAX_FILE_SIZE {
        let code = match decoder.decode_char() {
            Some(code) => code,
            None => break,
        };
        if code < 256 {
            let byte = code as u8;
            output.push(byte);
            decoder.text_buf[decoder.r] = byte;
            decoder.r = (decoder.r + 1) & 0xFFF;
        } else {
            let position = match decoder.decode_position() {
                Some(position) => position,
                None => break,
            };
            let start = (decoder.r + 4096 - position - 1) & 0xFFF;
            let length = code - 255 + 2;
            for index in 0..length {
                if output.len() >= MAX_FILE_SIZE {
                    break;
                }
                let byte = decoder.text_buf[(start + index as usize) & 0xFFF];
                output.push(byte);
                decoder.text_buf[decoder.r] = byte;
                decoder.r = (decoder.r + 1) & 0xFFF;
            }
        }
    }
    if output.is_empty() {
        return Err(DskError::invalid_format("TD0 LZHUF stream is empty"));
    }
    Ok(output)
}

struct LzhufDecoder<'a> {
    data: &'a [u8],
    byte_pos: usize,
    bit_buffer: u16,
    bit_length: u8,
    text_buf: [u8; 4096],
    freq: [u16; 628],
    parent: [usize; 941],
    son: [usize; 627],
    r: usize,
}

impl<'a> LzhufDecoder<'a> {
    fn new(data: &'a [u8]) -> Self {
        let mut decoder = Self {
            data,
            byte_pos: 0,
            bit_buffer: 0,
            bit_length: 0,
            text_buf: [b' '; 4096],
            freq: [0; 628],
            parent: [0; 941],
            son: [0; 627],
            r: 4096 - 60,
        };
        decoder.start_huffman();
        decoder
    }

    fn read_bit(&mut self) -> Option<u8> {
        while self.bit_length <= 8 && self.byte_pos < self.data.len() {
            self.bit_buffer |= (self.data[self.byte_pos] as u16) << (8 - self.bit_length);
            self.byte_pos += 1;
            self.bit_length += 8;
        }
        if self.bit_length == 0 {
            return None;
        }
        let bit = (self.bit_buffer >> 15) as u8;
        self.bit_buffer <<= 1;
        self.bit_length -= 1;
        Some(bit)
    }

    fn read_byte(&mut self) -> Option<u8> {
        let mut value = 0u8;
        for _ in 0..8 {
            value = (value << 1) | self.read_bit()?;
        }
        Some(value)
    }

    fn start_huffman(&mut self) {
        for i in 0..314 {
            self.freq[i] = 1;
            self.son[i] = i + 627;
            self.parent[i + 627] = i;
        }
        let mut i = 0;
        let mut j = 314;
        while j <= 626 {
            self.freq[j] = self.freq[i] + self.freq[i + 1];
            self.son[j] = i;
            self.parent[i] = j;
            self.parent[i + 1] = j;
            i += 2;
            j += 1;
        }
        self.freq[627] = 0xFFFF;
        self.parent[626] = 0;
    }

    fn reconstruct(&mut self) {
        let mut j = 0;
        for i in 0..627 {
            if self.son[i] >= 627 {
                self.freq[j] = (self.freq[i] + 1) / 2;
                self.son[j] = self.son[i];
                j += 1;
            }
        }

        let mut i = 0;
        let mut j = 314;
        while j < 627 {
            let k_pair = i + 1;
            let frequency = self.freq[i] + self.freq[k_pair];
            self.freq[j] = frequency;
            let mut k = j - 1;
            while frequency < self.freq[k] && k > 0 {
                k -= 1;
            }
            k += 1;
            for index in (k..j).rev() {
                self.freq[index + 1] = self.freq[index];
                self.son[index + 1] = self.son[index];
            }
            self.freq[k] = frequency;
            self.son[k] = i;
            i += 2;
            j += 1;
        }

        for i in 0..627 {
            let child = self.son[i];
            if child >= 627 {
                self.parent[child] = i;
            } else {
                self.parent[child] = i;
                self.parent[child + 1] = i;
            }
        }
    }

    fn update(&mut self, code: usize) {
        if self.freq[626] == 0x8000 {
            self.reconstruct();
        }
        let mut node = self.parent[code + 627];
        loop {
            self.freq[node] += 1;
            let mut next = node + 1;
            if self.freq[node] > self.freq[next] {
                while self.freq[node] > self.freq[next] {
                    next += 1;
                }
                next -= 1;
                self.freq.swap(node, next);
                let child = self.son[node];
                self.son[node] = self.son[next];
                self.son[next] = child;
                self.parent[self.son[node]] = node;
                if self.son[node] < 627 {
                    self.parent[self.son[node] + 1] = node;
                }
                self.parent[self.son[next]] = next;
                if self.son[next] < 627 {
                    self.parent[self.son[next] + 1] = next;
                }
                node = next;
            }
            node = self.parent[node];
            if node == 0 {
                break;
            }
        }
    }

    fn decode_char(&mut self) -> Option<usize> {
        let mut node = self.son[626];
        while node < 627 {
            node += self.read_bit()? as usize;
            node = self.son[node];
        }
        let code = node - 627;
        self.update(code);
        Some(code)
    }

    fn decode_position(&mut self) -> Option<usize> {
        const POSITION_LENGTH: [u8; 64] = [
            3, 4, 4, 4, 5, 5, 5, 5, 5, 5, 5, 5, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 7, 7, 7, 7, 7,
            7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8,
            8, 8, 8, 8, 8, 8,
        ];
        const POSITION_CODE: [u8; 64] = [
            0x00, 0x20, 0x30, 0x40, 0x50, 0x58, 0x60, 0x68, 0x70, 0x78, 0x80, 0x88, 0x90, 0x94,
            0x98, 0x9C, 0xA0, 0xA4, 0xA8, 0xAC, 0xB0, 0xB4, 0xB8, 0xBC, 0xC0, 0xC2, 0xC4, 0xC6,
            0xC8, 0xCA, 0xCC, 0xCE, 0xD0, 0xD2, 0xD4, 0xD6, 0xD8, 0xDA, 0xDC, 0xDE, 0xE0, 0xE2,
            0xE4, 0xE6, 0xE8, 0xEA, 0xEC, 0xEE, 0xF0, 0xF1, 0xF2, 0xF3, 0xF4, 0xF5, 0xF6, 0xF7,
            0xF8, 0xF9, 0xFA, 0xFB, 0xFC, 0xFD, 0xFE, 0xFF,
        ];
        let first = self.read_byte()?;
        let code = POSITION_CODE
            .iter()
            .rposition(|&prefix| prefix <= first)
            .unwrap_or(0);
        let mut lower = first as usize;
        for _ in 2..POSITION_LENGTH[code] {
            lower = (lower << 1) | self.read_bit()? as usize;
        }
        Some((code << 6) | (lower & 0x3F))
    }
}

fn next_lzw_code(data: &[u8], nibble_pos: &mut usize, nibble_length: usize) -> Result<u16> {
    if *nibble_pos + 3 > nibble_length {
        return Err(DskError::parse(*nibble_pos, "Short TD0 LZW code"));
    }
    let mut code = 0u16;
    for shift in 0..3 {
        let position = *nibble_pos + shift;
        let byte = *data
            .get(position / 2)
            .ok_or_else(|| DskError::parse(position, "Short TD0 LZW data"))?;
        let nibble = if position & 1 == 0 {
            byte & 0x0F
        } else {
            byte >> 4
        };
        code |= (nibble as u16) << (shift * 4);
    }
    *nibble_pos += 3;
    Ok(code)
}

fn expand_lzw_code(code: u16, dictionary: &[(u16, u8)]) -> Result<Vec<u8>> {
    let mut code = code;
    let mut reversed = Vec::new();
    while code >= 256 {
        let entry = dictionary
            .get((code - 256) as usize)
            .ok_or_else(|| DskError::parse(0, "Invalid TD0 LZW dictionary reference"))?;
        reversed.push(entry.1);
        code = entry.0;
    }
    reversed.push(code as u8);
    reversed.reverse();
    Ok(reversed)
}

fn take<'a>(data: &'a [u8], cursor: &mut usize, length: usize, what: &str) -> Result<&'a [u8]> {
    let end = cursor
        .checked_add(length)
        .ok_or_else(|| DskError::parse(*cursor, format!("Invalid TD0 {} length", what)))?;
    let result = data
        .get(*cursor..end)
        .ok_or_else(|| DskError::parse(*cursor, format!("Short TD0 {}", what)))?;
    *cursor = end;
    Ok(result)
}

fn teledisk_crc(data: &[u8]) -> u16 {
    let mut crc = 0u16;
    for &byte in data {
        crc ^= (byte as u16) << 8;
        for _ in 0..8 {
            crc = if crc & 0x8000 != 0 {
                (crc << 1) ^ 0xA097
            } else {
                crc << 1
            };
        }
    }
    crc
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_td0_extension() {
        assert!(is_td0_file("CPM3.TD0"));
        assert!(!is_td0_file("disk.dsk"));
    }

    #[test]
    fn test_lzw_block() {
        // Codes for "ABABA": A, B, AB, ABA in 12-bit nibble order.
        let codes = [0x41u16, 0x42, 0x100, 0x41];
        let mut encoded = Vec::new();
        let mut nibbles = Vec::new();
        for code in codes {
            nibbles.push((code & 0x0F) as u8);
            nibbles.push(((code >> 4) & 0x0F) as u8);
            nibbles.push(((code >> 8) & 0x0F) as u8);
        }
        for pair in nibbles.chunks(2) {
            encoded.push(pair[0] | (pair.get(1).copied().unwrap_or(0) << 4));
        }
        assert_eq!(decode_lzw_block(&encoded, nibbles.len()).unwrap(), b"ABABA");
    }

    #[test]
    fn test_rle_block() {
        let payload = [1, 2, b'A', b'B'];
        let mut cursor = 0;
        assert_eq!(decode_rle(&payload, &mut cursor, 4).unwrap(), b"ABAB");
    }

    #[test]
    fn test_uncompressed_td0_image() {
        let mut image = vec![b'T', b'D', 0, 0, 15, 0, 1, 0, 0, 1, 0, 0];
        let header_crc = teledisk_crc(&image[..10]).to_le_bytes();
        image[10..12].copy_from_slice(&header_crc);

        let track_header = [1, 0, 0, teledisk_crc(&[1, 0, 0]) as u8];
        image.extend_from_slice(&track_header);

        let mut sector_header = [0, 0, 1, 2, 0, 0];
        let data = vec![0xE5; 512];
        let mut sector_crc_data = sector_header[..5].to_vec();
        sector_crc_data.extend_from_slice(&data);
        sector_header[5] = teledisk_crc(&sector_crc_data) as u8;
        image.extend_from_slice(&sector_header);
        image.extend_from_slice(&[0x01, 0x02, 0]);
        image.extend_from_slice(&data);
        image.extend_from_slice(&[LAST_TRACK, 0, 0, 0]);

        let path = std::env::temp_dir().join(format!("dskmgr_td0_{}.td0", std::process::id()));
        std::fs::write(&path, image).unwrap();
        let parsed = DiskImage::open(&path).unwrap();
        std::fs::remove_file(path).ok();

        assert_eq!(parsed.format(), DiskImageFormat::RawTd0);
        assert_eq!(parsed.spec().num_sides, 1);
        assert_eq!(parsed.spec().num_tracks, 1);
        assert_eq!(parsed.read_sector(0, 0, 1).unwrap(), data.as_slice());
        assert!(parsed.warnings().is_empty(), "{:?}", parsed.warnings());
    }
}
