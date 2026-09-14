/// DSK file reader

use crate::error::{DskError, Result};
use crate::fdc::{FdcStatus1, FdcStatus2};
use crate::format::constants::*;
use crate::format::{detect_format, DiskImageFormat, FormatSpec};
use crate::image::{DataRate, Disk, DiskImage, RecordingMode, Sector, SectorId, Track};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

/// Read a DSK file from disk
pub fn read_dsk<P: AsRef<Path>>(path: P) -> Result<DiskImage> {
    let filename = path.as_ref()
        .file_name()
        .and_then(|n| n.to_str())
        .map(|s| s.to_string());

    let mut file = File::open(&path)?;

    // Read disk info block (256 bytes)
    let mut disk_info = vec![0u8; DISK_INFO_BLOCK_SIZE];
    file.read_exact(&mut disk_info)?;

    // Non-fatal issues collected while reading; attached to the image below.
    let mut warnings = Vec::new();

    // Detect format, tolerating a signature whose case is wrong (some writers
    // emit "EXTENDED CPC DSK FILE" in upper case).
    let format = match detect_format(&disk_info) {
        Some(f) => f,
        None => match detect_format_ignore_case(&disk_info) {
            Some(f) => {
                warnings.push("File signature has incorrect case.".to_string());
                f
            }
            None => return Err(DskError::invalid_format("Unknown DSK format")),
        },
    };

    // The creator/tool signature should be present.
    if !has_creator_signature(&disk_info, format) {
        warnings.push("Missing creator signature.".to_string());
    }

    let mut image = match format {
        DiskImageFormat::StandardDSK => read_standard_dsk(file, &disk_info, filename, &mut warnings)?,
        DiskImageFormat::ExtendedDSK => read_extended_dsk(file, &disk_info, filename, &mut warnings)?,
        DiskImageFormat::RawMgt => {
            return Err(DskError::invalid_format("RawMgt format should use read_mgt"))
        }
        DiskImageFormat::RawTrd => {
            return Err(DskError::invalid_format("RawTrd format should use read_trd"))
        }
        DiskImageFormat::RawTd0 => {
            return Err(DskError::invalid_format("RawTd0 format should use read_td0"))
        }
    };

    image.warnings = warnings;
    Ok(image)
}

/// Whether the disk-info block identifies its creator.
///
/// The dedicated creator field is at offset 0x22 (14 bytes), but many writers
/// (e.g. CPCEMU) instead stamp their name/date into the 34-byte descriptor
/// line itself - e.g. "MV - CPCEMU / 12 May 97" in place of the canonical
/// "MV - CPCEMU Disk-File...". A descriptor that deviates from the canonical
/// signature is therefore treated as carrying creator info.
fn has_creator_signature(disk_info: &[u8], format: DiskImageFormat) -> bool {
    let field = &disk_info[DISK_INFO_CREATOR_OFFSET..DISK_INFO_CREATOR_OFFSET + 14];
    if field.iter().any(|&b| b != 0 && b != b' ') {
        return true;
    }
    let canonical: &[u8] = match format {
        DiskImageFormat::ExtendedDSK => EXTENDED_DSK_SIGNATURE,
        DiskImageFormat::StandardDSK => STANDARD_DSK_SIGNATURE,
        DiskImageFormat::RawMgt => return true,
        DiskImageFormat::RawTrd => return true,
        DiskImageFormat::RawTd0 => return true,
    };
    // Compare the 34-byte descriptor; a difference means embedded creator text.
    disk_info[..DISK_INFO_CREATOR_OFFSET] != canonical[..DISK_INFO_CREATOR_OFFSET]
}

/// Detect the DSK format ignoring ASCII case in the signature.
fn detect_format_ignore_case(magic: &[u8]) -> Option<DiskImageFormat> {
    let starts_with_ci = |prefix: &[u8]| {
        magic.len() >= prefix.len()
            && magic[..prefix.len()].eq_ignore_ascii_case(prefix)
    };
    if starts_with_ci(b"EXTENDED") {
        Some(DiskImageFormat::ExtendedDSK)
    } else if starts_with_ci(b"MV - CPC") {
        Some(DiskImageFormat::StandardDSK)
    } else {
        None
    }
}

/// Read a Standard DSK file
fn read_standard_dsk(
    mut file: File,
    disk_info: &[u8],
    filename: Option<String>,
    warnings: &mut Vec<String>,
) -> Result<DiskImage> {
    // Parse disk info block
    let num_tracks = disk_info[DISK_INFO_TRACK_COUNT_OFFSET];
    let num_sides = disk_info[DISK_INFO_SIDE_COUNT_OFFSET];
    let track_size = u16::from_le_bytes([
        disk_info[DISK_INFO_TRACK_SIZE_OFFSET],
        disk_info[DISK_INFO_TRACK_SIZE_OFFSET + 1],
    ]) as usize;

    let mut disks = Vec::with_capacity(num_sides as usize);

    // Read tracks for each side
    for side in 0..num_sides {
        let mut disk = Disk::new(side);

        for track_num in 0..num_tracks {
            let track = read_track(&mut file, track_num, side, track_size, warnings)?;
            disk.add_track(track);
        }

        disks.push(disk);
    }

    // Create format spec based on first track
    let spec = build_format_spec(&disks, num_sides, num_tracks);

    Ok(DiskImage {
        format: DiskImageFormat::StandardDSK,
        spec,
        disks,
        changed: false,
        filename,
        warnings: Vec::new(),
    })
}

/// Read an Extended DSK file
fn read_extended_dsk(
    mut file: File,
    disk_info: &[u8],
    filename: Option<String>,
    warnings: &mut Vec<String>,
) -> Result<DiskImage> {
    // Parse disk info block
    let num_tracks = disk_info[DISK_INFO_TRACK_COUNT_OFFSET];
    let num_sides = disk_info[DISK_INFO_SIDE_COUNT_OFFSET];

    // Extended format has per-track sizes (in 256-byte units)
    let mut track_sizes = Vec::new();
    for i in 0..(num_tracks as usize * num_sides as usize) {
        let offset = DISK_INFO_EXT_TRACK_SIZE_OFFSET + i;
        if offset < disk_info.len() {
            let size = disk_info[offset] as usize * 256;
            track_sizes.push(size);
        } else {
            track_sizes.push(0);
        }
    }

    let mut disks = Vec::with_capacity(num_sides as usize);
    let mut recovered_tracks = 0usize;

    // Read tracks for each side
    let mut track_index = 0;
    for side in 0..num_sides {
        let mut disk = Disk::new(side);

        for track_num in 0..num_tracks {
            let track_size = track_sizes[track_index];
            track_index += 1;

            if track_size == 0 {
                // The track-size table reports zero for this track. Some writers
                // (e.g. CPDRead) emit an Extended DSK with a blank size table even
                // though real, uniformly-sized track data follows. Try to recover
                // by reading the actual Track-Info block at the current position.
                match recover_extended_track_size(&mut file, track_num, side)? {
                    Some(recovered) => {
                        let track = read_track(&mut file, track_num, side, recovered, warnings)?;
                        disk.add_track(track);
                        recovered_tracks += 1;
                    }
                    None => {
                        // Genuinely unformatted track - create empty track
                        disk.add_track(Track::new(track_num, side));
                    }
                }
            } else {
                let track = read_track(&mut file, track_num, side, track_size, warnings)?;
                disk.add_track(track);
            }
        }

        disks.push(disk);
    }

    // Create format spec
    let spec = build_format_spec(&disks, num_sides, num_tracks);

    if recovered_tracks > 0 {
        warnings.push(format!(
            "Extended DSK track-size table was missing; recovered {} track(s) by scanning",
            recovered_tracks
        ));
    }

    Ok(DiskImage {
        format: DiskImageFormat::ExtendedDSK,
        spec,
        disks,
        changed: false,
        filename,
        warnings: Vec::new(),
    })
}

/// Recover the size of an Extended DSK track whose size-table entry is zero.
///
/// Peeks at the Track-Info block at the current file position (without consuming
/// it). The recovery only applies when a valid block is present *and* its stored
/// track/side numbers match the expected ones - otherwise the zero entry denotes
/// a genuinely unformatted track and the peek would have landed on the *next*
/// track's data. Returns `Some(size)` with the byte length to read, else `None`.
fn recover_extended_track_size(file: &mut File, track_num: u8, side: u8) -> Result<Option<usize>> {
    let pos = file.stream_position()?;

    // Read up to a full Track-Info block, tolerating EOF (trailing tracks).
    let mut header = [0u8; TRACK_INFO_BLOCK_SIZE];
    let mut filled = 0;
    while filled < header.len() {
        match file.read(&mut header[filled..])? {
            0 => break,
            n => filled += n,
        }
    }
    file.seek(SeekFrom::Start(pos))?;

    if filled < TRACK_INFO_BLOCK_SIZE || !header.starts_with(b"Track-Info") {
        return Ok(None);
    }

    // Only recover when this block actually belongs to the expected track.
    if header[0x10] != track_num || header[0x11] != side {
        return Ok(None);
    }

    let num_sectors = header[0x15] as usize;
    let mut total = TRACK_INFO_BLOCK_SIZE;
    for i in 0..num_sectors {
        let sib = 0x18 + i * SECTOR_INFO_SIZE;
        if sib + SECTOR_INFO_SIZE > TRACK_INFO_BLOCK_SIZE {
            break;
        }
        let size_code = header[sib + 3];
        let data_length = u16::from_le_bytes([header[sib + 6], header[sib + 7]]) as usize;
        total += if data_length > 0 {
            data_length
        } else {
            fdc_size_to_stored_bytes(size_code)
        };
    }

    Ok(Some(total))
}

/// Read a single track from the file
fn read_track(
    file: &mut File,
    track_num: u8,
    side: u8,
    track_size: usize,
    warnings: &mut Vec<String>,
) -> Result<Track> {
    // Read the declared track length, tolerating a truncated file rather than
    // aborting the whole load.
    let mut track_data = vec![0u8; track_size];
    let mut filled = 0;
    while filled < track_size {
        match file.read(&mut track_data[filled..])? {
            0 => break,
            n => filled += n,
        }
    }
    if filled < track_size {
        warnings.push(format!(
            "Side {} track {} declared {} bytes but only {} were available (truncated image).",
            side, track_num, track_size, filled
        ));
        track_data.truncate(filled);
    }

    // Parse track info block (256 bytes)
    if track_data.len() < TRACK_INFO_BLOCK_SIZE {
        return Err(DskError::parse(0, "Track too small"));
    }

    // Verify track marker
    if !track_data.starts_with(b"Track-Info") {
        return Err(DskError::parse(0, "Invalid track marker"));
    }

    // A valid marker is "Track-Info\r\n"; some writers pad with spaces instead.
    if track_data[10..12] != [0x0D, 0x0A]
        && !warnings.iter().any(|w| w.starts_with("Disk image uses incorrect"))
    {
        warnings.push(
            "Disk image uses incorrect \"Track-Info\" markers padded with spaces not CRLF."
                .to_string(),
        );
    }

    let _track_number = track_data[0x10];
    let _side_number = track_data[0x11];
    let data_rate = DataRate::from(track_data[0x12]);
    let recording_mode = RecordingMode::from(track_data[0x13]);
    let _sector_size_code = track_data[0x14];
    let num_sectors = track_data[0x15];
    let gap3_length = track_data[0x16];
    let filler_byte = track_data[0x17];

    let mut track = Track::new(track_num, side);
    track.gap3_length = gap3_length;
    track.filler_byte = filler_byte;
    track.data_rate = data_rate;
    track.recording_mode = recording_mode;

    // Parse sector info list (starts at offset 0x18, 8 bytes per sector)
    let mut sector_offset = TRACK_INFO_BLOCK_SIZE;

    for i in 0..num_sectors as usize {
        let sib_offset = 0x18 + (i * SECTOR_INFO_SIZE);
        if sib_offset + SECTOR_INFO_SIZE > track_data.len() {
            break;
        }

        let sib = &track_data[sib_offset..sib_offset + SECTOR_INFO_SIZE];

        let sector_track = sib[0];
        let sector_side = sib[1];
        let sector_id = sib[2];
        let sector_size_code = sib[3];
        let fdc_st1 = sib[4];
        let fdc_st2 = sib[5];
        let data_length = if sib.len() >= 8 {
            u16::from_le_bytes([sib[6], sib[7]])
        } else {
            fdc_size_to_stored_bytes(sector_size_code) as u16
        };

        // Calculate actual sector data size
        // Use stored size rules when data_length is 0 (standard format fallback)
        let mut actual_size = if data_length > 0 {
            data_length as usize
        } else {
            fdc_size_to_stored_bytes(sector_size_code)
        };

        if actual_size > MAX_SECTOR_SIZE {
            warnings.push(format!(
                "Side {} track {} sector {} exceeds the {} byte size limit.",
                side, track_num, i, MAX_SECTOR_SIZE
            ));
            actual_size = MAX_SECTOR_SIZE;
        }

        // Extract sector data
        let sector_data = if sector_offset + actual_size <= track_data.len() {
            track_data[sector_offset..sector_offset + actual_size].to_vec()
        } else if sector_offset < track_data.len() {
            // Partial data available, pad with filler
            let mut data = track_data[sector_offset..].to_vec();
            data.resize(actual_size, filler_byte);
            data
        } else {
            // No data available, fill entirely with filler byte
            vec![filler_byte; actual_size]
        };

        sector_offset += actual_size;

        let id = SectorId::new(sector_track, sector_side, sector_id, sector_size_code);
        let sector = Sector::with_status(
            id,
            FdcStatus1::new(fdc_st1),
            FdcStatus2::new(fdc_st2),
            sector_data,
        );

        track.add_sector(sector);
    }

    Ok(track)
}

/// Build a format specification from the disk structure
fn build_format_spec(disks: &[Disk], num_sides: u8, num_tracks: u8) -> FormatSpec {
    // Try to detect format from first non-empty track
    let mut sectors_per_track = 9;
    let mut sector_size = 512;
    let mut first_sector_id = 0xC1;
    let mut gap3_length = 0x4E;
    let mut filler_byte = 0xE5;

    if let Some(disk) = disks.first() {
        if let Some(track) = disk.tracks().iter().find(|t| !t.is_empty()) {
            sectors_per_track = track.sector_count() as u8;
            gap3_length = track.gap3_length;
            filler_byte = track.filler_byte;

            if let Some(sector) = track.sectors().first() {
                sector_size = sector.advertised_size() as u16;
                first_sector_id = sector.id.sector;
            }
        }
    }

    FormatSpec {
        num_sides,
        num_tracks,
        sectors_per_track,
        sector_size,
        first_sector_id,
        gap3_length,
        filler_byte,
        interleave: 1,
        side_mode: if num_sides == 1 {
            crate::format::spec::SideMode::SingleSide
        } else {
            crate::format::spec::SideMode::Alternate
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_format_spec() {
        let mut disks = Vec::new();
        let mut disk = Disk::new(0);
        let mut track = Track::new(0, 0);

        for i in 0..9 {
            let id = SectorId::new(0, 0, 0xC1 + i, 2);
            track.add_sector(Sector::new(id));
        }

        disk.add_track(track);
        disks.push(disk);

        let spec = build_format_spec(&disks, 1, 40);

        assert_eq!(spec.num_sides, 1);
        assert_eq!(spec.num_tracks, 40);
        assert_eq!(spec.sectors_per_track, 9);
        assert_eq!(spec.sector_size, 512);
        assert_eq!(spec.first_sector_id, 0xC1);
    }

    /// Regression: some writers (e.g. CPDRead) emit an Extended DSK whose
    /// per-track size table is all zeros even though real track data follows.
    /// The reader must recover the formatted tracks instead of reporting the
    /// whole disk as unformatted, while still treating genuinely-absent
    /// trailing tracks as empty.
    #[test]
    fn test_extended_dsk_zero_size_table_recovery() {
        const SECTORS: usize = 9;
        const SECTOR_SIZE: usize = 512;

        // --- Disk info block (256 bytes): 2 tracks, 1 side, blank size table ---
        let mut buf = vec![0u8; DISK_INFO_BLOCK_SIZE];
        buf[..EXTENDED_DSK_SIGNATURE.len()].copy_from_slice(EXTENDED_DSK_SIGNATURE);
        buf[DISK_INFO_CREATOR_OFFSET..DISK_INFO_CREATOR_OFFSET + 6].copy_from_slice(b"CPDRea");
        buf[DISK_INFO_TRACK_COUNT_OFFSET] = 2;
        buf[DISK_INFO_SIDE_COUNT_OFFSET] = 1;
        // DISK_INFO_EXT_TRACK_SIZE_OFFSET onward intentionally left zero.

        // --- Track 0: a valid Track-Info block + 9x512 sector data ---
        let mut track = vec![0u8; TRACK_INFO_BLOCK_SIZE];
        track[..b"Track-Info\r\n".len()].copy_from_slice(b"Track-Info\r\n");
        track[0x10] = 0; // track number
        track[0x11] = 0; // side
        track[0x14] = 2; // sector size code (512)
        track[0x15] = SECTORS as u8;
        track[0x16] = 0x4E; // gap3
        track[0x17] = 0xE5; // filler
        for i in 0..SECTORS {
            let sib = 0x18 + i * SECTOR_INFO_SIZE;
            track[sib] = 0; // C
            track[sib + 1] = 0; // H
            track[sib + 2] = 0x41 + i as u8; // R (sector id &41..&49)
            track[sib + 3] = 2; // N (size code)
            track[sib + 6] = (SECTOR_SIZE & 0xFF) as u8; // stored length low
            track[sib + 7] = (SECTOR_SIZE >> 8) as u8; // stored length high
        }
        track.extend(std::iter::repeat(0xE5).take(SECTORS * SECTOR_SIZE));
        buf.extend_from_slice(&track);
        // Track 1 is deliberately absent from the file (genuinely unformatted).

        // --- Round-trip through a temp file ---
        let path = std::env::temp_dir()
            .join(format!("dskmgr_zero_table_{}.dsk", std::process::id()));
        std::fs::write(&path, &buf).unwrap();
        let image = read_dsk(&path).unwrap();
        std::fs::remove_file(&path).ok();

        assert_eq!(image.format, DiskImageFormat::ExtendedDSK);
        let tracks = image.disks[0].tracks();
        assert_eq!(tracks.len(), 2);
        assert_eq!(tracks[0].sector_count(), SECTORS, "track 0 must be recovered");
        assert_eq!(tracks[0].sectors()[0].id.sector, 0x41);
        assert!(tracks[1].is_empty(), "absent trailing track stays empty");

        // The recovery must be surfaced as a warning (and only that one).
        assert_eq!(image.warnings().len(), 1);
        assert!(image.warnings()[0].contains("track-size table"));
    }

    /// Build a complete, well-formed single-track Extended DSK (proper size
    /// table, creator present) that loads with no warnings. Tests mutate the
    /// returned buffer to provoke specific warnings.
    fn one_track_ext_dsk() -> Vec<u8> {
        const SECTORS: usize = 9;
        const SECTOR_SIZE: usize = 512;
        let track_len = TRACK_INFO_BLOCK_SIZE + SECTORS * SECTOR_SIZE; // 4864 = 0x1300

        let mut buf = vec![0u8; DISK_INFO_BLOCK_SIZE];
        buf[..EXTENDED_DSK_SIGNATURE.len()].copy_from_slice(EXTENDED_DSK_SIGNATURE);
        buf[DISK_INFO_CREATOR_OFFSET..DISK_INFO_CREATOR_OFFSET + 6].copy_from_slice(b"tester");
        buf[DISK_INFO_TRACK_COUNT_OFFSET] = 1;
        buf[DISK_INFO_SIDE_COUNT_OFFSET] = 1;
        buf[DISK_INFO_EXT_TRACK_SIZE_OFFSET] = (track_len / 256) as u8; // 0x13

        let mut track = vec![0u8; TRACK_INFO_BLOCK_SIZE];
        track[..b"Track-Info\r\n".len()].copy_from_slice(b"Track-Info\r\n");
        track[0x14] = 2;
        track[0x15] = SECTORS as u8;
        track[0x16] = 0x4E;
        track[0x17] = 0xE5;
        for i in 0..SECTORS {
            let sib = 0x18 + i * SECTOR_INFO_SIZE;
            track[sib + 2] = 0x41 + i as u8;
            track[sib + 3] = 2;
            track[sib + 6] = (SECTOR_SIZE & 0xFF) as u8;
            track[sib + 7] = (SECTOR_SIZE >> 8) as u8;
        }
        track.extend(std::iter::repeat(0xE5).take(SECTORS * SECTOR_SIZE));
        buf.extend_from_slice(&track);
        buf
    }

    fn read_buf(buf: &[u8], tag: &str) -> DiskImage {
        let path = std::env::temp_dir()
            .join(format!("dskmgr_{}_{}.dsk", tag, std::process::id()));
        std::fs::write(&path, buf).unwrap();
        let image = read_dsk(&path).unwrap();
        std::fs::remove_file(&path).ok();
        image
    }

    #[test]
    fn test_clean_image_has_no_warnings() {
        let image = read_buf(&one_track_ext_dsk(), "clean");
        assert!(image.warnings().is_empty(), "{:?}", image.warnings());
    }

    #[test]
    fn test_warn_missing_creator() {
        let mut buf = one_track_ext_dsk();
        for b in &mut buf[DISK_INFO_CREATOR_OFFSET..DISK_INFO_CREATOR_OFFSET + 14] {
            *b = 0;
        }
        let image = read_buf(&buf, "nocreator");
        assert!(image.warnings().iter().any(|w| w.contains("Missing creator")));
    }

    #[test]
    fn test_creator_embedded_in_descriptor_is_not_missing() {
        // "MV - CPCEMU / 12 May 97" style: creator in the descriptor, blank field.
        let mut info = vec![0u8; DISK_INFO_BLOCK_SIZE];
        info[..29].copy_from_slice(b"MV - CPCEMU / 12 May 97 20:01");
        assert!(has_creator_signature(&info, DiskImageFormat::StandardDSK));

        // Canonical descriptor + blank field = genuinely no creator.
        let mut canon = vec![0u8; DISK_INFO_BLOCK_SIZE];
        canon[..STANDARD_DSK_SIGNATURE.len()].copy_from_slice(STANDARD_DSK_SIGNATURE);
        assert!(!has_creator_signature(&canon, DiskImageFormat::StandardDSK));

        // Canonical descriptor but populated field = creator present.
        canon[DISK_INFO_CREATOR_OFFSET..DISK_INFO_CREATOR_OFFSET + 4].copy_from_slice(b"SPIN");
        assert!(has_creator_signature(&canon, DiskImageFormat::StandardDSK));
    }

    #[test]
    fn test_warn_signature_wrong_case() {
        let mut buf = one_track_ext_dsk();
        buf[..8].copy_from_slice(b"Extended");
        let image = read_buf(&buf, "case");
        assert!(image.warnings().iter().any(|w| w.contains("incorrect case")));
    }

    #[test]
    fn test_warn_broken_track_markers() {
        let mut buf = one_track_ext_dsk();
        // Replace the "\r\n" after "Track-Info" with spaces in the track block.
        buf[DISK_INFO_BLOCK_SIZE + 10] = b' ';
        buf[DISK_INFO_BLOCK_SIZE + 11] = b' ';
        let image = read_buf(&buf, "markers");
        assert!(image.warnings().iter().any(|w| w.contains("Track-Info")));
    }

    #[test]
    fn test_warn_truncated_track() {
        let mut buf = one_track_ext_dsk();
        buf.truncate(buf.len() - 1000); // chop the tail of the track
        let image = read_buf(&buf, "trunc");
        assert!(image.warnings().iter().any(|w| w.contains("truncated")));
    }
}
