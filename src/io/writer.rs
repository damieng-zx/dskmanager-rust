/// DSK file writer
use crate::error::Result;
use crate::fdc::{FdcStatus1, FdcStatus2};
use crate::format::constants::*;
use crate::format::DiskImageFormat;
use crate::image::DiskImage;
use std::fs::File;
use std::io::Write;
use std::path::Path;

/// Write a DSK file to disk
pub fn write_dsk<P: AsRef<Path>>(image: &DiskImage, path: P) -> Result<()> {
    let mut file = File::create(path)?;

    match image.format {
        DiskImageFormat::StandardDSK => write_standard_dsk(&mut file, image),
        DiskImageFormat::ExtendedDSK => write_extended_dsk(&mut file, image),
        DiskImageFormat::RawMgt => write_mgt(&mut file, image),
        DiskImageFormat::RawTrd => write_trd(&mut file, image),
        DiskImageFormat::RawTd0 => write_td0(&mut file, image),
    }
}

/// Write a raw MGT file
fn write_mgt(file: &mut File, image: &DiskImage) -> Result<()> {
    // MGT format: all side 0 tracks, then all side 1 tracks
    // 80 tracks per side, 2 sides, 10 sectors per track, 512 bytes per sector

    let tracks_per_side = 80usize;
    let sectors_per_track = 10usize;
    let sector_size = 512usize;

    for side in 0..2u8 {
        // Get the disk for this side
        if let Some(disk) = image.disks.get(side as usize) {
            for track_num in 0..tracks_per_side {
                if let Some(track) = disk.get_track(track_num as u8) {
                    // Write sectors in ID order (1-10 for MGT)
                    for sector_id in 1..=sectors_per_track as u8 {
                        if let Some(sector) = track.get_sector(sector_id) {
                            let data = sector.data();
                            if data.len() >= sector_size {
                                file.write_all(&data[..sector_size])?;
                            } else {
                                file.write_all(data)?;
                                let padding = vec![0u8; sector_size - data.len()];
                                file.write_all(&padding)?;
                            }
                        } else {
                            let zeros = vec![0u8; sector_size];
                            file.write_all(&zeros)?;
                        }
                    }
                } else {
                    let zeros = vec![0u8; sectors_per_track * sector_size];
                    file.write_all(&zeros)?;
                }
            }
        } else {
            let zeros = vec![0u8; tracks_per_side * sectors_per_track * sector_size];
            file.write_all(&zeros)?;
        }
    }

    Ok(())
}

/// Write a raw TRD file
fn write_trd(file: &mut File, image: &DiskImage) -> Result<()> {
    // TRD format: single-sided, sequential tracks
    // 80 tracks, 16 sectors per track, 256 bytes per sector

    let num_tracks = image.spec.num_tracks as usize;
    let sectors_per_track = 16usize;
    let sector_size = 256usize;

    if let Some(disk) = image.disks.get(0) {
        for track_num in 0..num_tracks {
            if let Some(track) = disk.get_track(track_num as u8) {
                for sector_id in 1..=sectors_per_track as u8 {
                    if let Some(sector) = track.get_sector(sector_id) {
                        let data = sector.data();
                        if data.len() >= sector_size {
                            file.write_all(&data[..sector_size])?;
                        } else {
                            file.write_all(data)?;
                            let padding = vec![0u8; sector_size - data.len()];
                            file.write_all(&padding)?;
                        }
                    } else {
                        let zeros = vec![0u8; sector_size];
                        file.write_all(&zeros)?;
                    }
                }
            } else {
                let zeros = vec![0u8; sectors_per_track * sector_size];
                file.write_all(&zeros)?;
            }
        }
    }

    Ok(())
}

/// Write an uncompressed TeleDisk file.
fn write_td0(file: &mut File, image: &DiskImage) -> Result<()> {
    let side_count = image.disks.len().clamp(1, 2) as u8;
    let mut header = [0u8; 12];
    header[0..2].copy_from_slice(b"TD");
    header[4] = 0x15; // TeleDisk 2.1
    header[5] = 0; // 250 kbps, MFM
    header[6] = if image.spec.num_tracks > 40 { 3 } else { 1 };
    header[7] = 0; // single-step, no comment block
    header[8] = 0; // include all sectors
    header[9] = side_count;
    let header_crc = crate::io::td0_reader::teledisk_crc(&header[..10]);
    header[10..12].copy_from_slice(&header_crc.to_le_bytes());
    file.write_all(&header)?;

    for disk in image.disks.iter().take(2) {
        for track in disk.tracks() {
            write_td0_track(file, track)?;
        }
    }

    let end_marker = [0xFF, 0, 0, 0];
    let crc = crate::io::td0_reader::teledisk_crc(&end_marker[..3]) as u8;
    file.write_all(&[end_marker[0], end_marker[1], end_marker[2], crc])?;
    Ok(())
}

fn write_td0_track(file: &mut File, track: &crate::image::Track) -> Result<()> {
    if track.sector_count() > u8::MAX as usize {
        return Err(crate::error::DskError::UnsupportedFormat(
            "TD0 tracks cannot contain more than 255 sectors".to_string(),
        ));
    }

    let track_header = [
        track.sector_count() as u8,
        track.track_number,
        track.side_number & 1,
        0,
    ];
    let track_crc = crate::io::td0_reader::teledisk_crc(&track_header[..3]) as u8;
    file.write_all(&[track_header[0], track_header[1], track_header[2], track_crc])?;

    for sector in track.sectors() {
        if sector.id.size_code > 8 {
            return Err(crate::error::DskError::UnsupportedFormat(
                "TD0 sector size code must be between 0 and 8".to_string(),
            ));
        }

        let mut flags = 0u8;
        if sector.fdc_status1.0 & FdcStatus1::DE != 0 || sector.fdc_status2.0 & FdcStatus2::DD != 0
        {
            flags |= 0x02;
        }
        if sector.fdc_status2.0 & FdcStatus2::CM != 0 {
            flags |= 0x04;
        }

        let sector_size = sector.advertised_size();
        let mut data = sector.data().to_vec();
        if data.is_empty() {
            flags |= 0x20;
        } else {
            data.resize(sector_size, track.filler_byte);
            data.truncate(sector_size);
        }

        let sector_header = [
            sector.id.track,
            sector.id.side & 1,
            sector.id.sector,
            sector.id.size_code,
            flags,
            0,
        ];
        let mut crc_data = sector_header[..5].to_vec();
        if flags & 0x20 == 0 {
            crc_data.extend_from_slice(&data);
        }
        let sector_crc = crate::io::td0_reader::teledisk_crc(&crc_data) as u8;
        file.write_all(&[
            sector_header[0],
            sector_header[1],
            sector_header[2],
            sector_header[3],
            sector_header[4],
            sector_crc,
        ])?;

        if flags & 0x20 == 0 {
            let block_size = (data.len() + 1) as u16;
            file.write_all(&block_size.to_le_bytes())?;
            file.write_all(&[0])?; // raw sector encoding
            file.write_all(&data)?;
        }
    }

    Ok(())
}

/// Write a Standard DSK file
fn write_standard_dsk(file: &mut File, image: &DiskImage) -> Result<()> {
    // Calculate track size (assume all tracks are the same size)
    let track_size = calculate_track_size(image);

    // Write disk info block
    let mut disk_info = vec![0u8; DISK_INFO_BLOCK_SIZE];

    // Copy magic bytes
    disk_info[..STANDARD_DSK_SIGNATURE.len()].copy_from_slice(STANDARD_DSK_SIGNATURE);

    // Copy creator signature
    let creator_len = CREATOR_SIGNATURE.len().min(14);
    disk_info[DISK_INFO_CREATOR_OFFSET..DISK_INFO_CREATOR_OFFSET + creator_len]
        .copy_from_slice(&CREATOR_SIGNATURE[..creator_len]);

    // Set track and side counts
    disk_info[DISK_INFO_TRACK_COUNT_OFFSET] = image.spec.num_tracks;
    disk_info[DISK_INFO_SIDE_COUNT_OFFSET] = image.spec.num_sides;

    // Set track size (in bytes, little-endian)
    let track_size_bytes = (track_size as u16).to_le_bytes();
    disk_info[DISK_INFO_TRACK_SIZE_OFFSET] = track_size_bytes[0];
    disk_info[DISK_INFO_TRACK_SIZE_OFFSET + 1] = track_size_bytes[1];

    file.write_all(&disk_info)?;

    // Write tracks for each side
    for disk in &image.disks {
        for track in disk.tracks() {
            write_track(file, track, track_size)?;
        }
    }

    Ok(())
}

/// Write an Extended DSK file
fn write_extended_dsk(file: &mut File, image: &DiskImage) -> Result<()> {
    // Write disk info block
    let mut disk_info = vec![0u8; DISK_INFO_BLOCK_SIZE];

    // Copy magic bytes
    disk_info[..EXTENDED_DSK_SIGNATURE.len()].copy_from_slice(EXTENDED_DSK_SIGNATURE);

    // Copy creator signature
    let creator_len = CREATOR_SIGNATURE.len().min(14);
    disk_info[DISK_INFO_CREATOR_OFFSET..DISK_INFO_CREATOR_OFFSET + creator_len]
        .copy_from_slice(&CREATOR_SIGNATURE[..creator_len]);

    // Set track and side counts
    disk_info[DISK_INFO_TRACK_COUNT_OFFSET] = image.spec.num_tracks;
    disk_info[DISK_INFO_SIDE_COUNT_OFFSET] = image.spec.num_sides;

    // Set per-track sizes (in 256-byte units)
    let mut track_index = 0;
    for disk in &image.disks {
        for track in disk.tracks() {
            let track_size = calculate_single_track_size(track);
            let size_units = ((track_size + 255) / 256) as u8; // Round up
            let offset = DISK_INFO_EXT_TRACK_SIZE_OFFSET + track_index;
            if offset < disk_info.len() {
                disk_info[offset] = size_units;
            }
            track_index += 1;
        }
    }

    file.write_all(&disk_info)?;

    // Write tracks for each side
    for disk in &image.disks {
        for track in disk.tracks() {
            let track_size = calculate_single_track_size(track);
            write_track(file, track, track_size)?;
        }
    }

    Ok(())
}

/// Write a single track to the file
fn write_track(file: &mut File, track: &crate::image::Track, track_size: usize) -> Result<()> {
    let mut track_data = vec![0u8; track_size];

    // Write track info block (256 bytes)
    track_data[..TRACK_INFO_MARKER.len()].copy_from_slice(TRACK_INFO_MARKER);

    track_data[0x10] = track.track_number;
    track_data[0x11] = track.side_number;
    track_data[0x12] = track.data_rate.into();
    track_data[0x13] = track.recording_mode.into();

    // Use first sector's size code if available, otherwise default to 2 (512 bytes)
    let sector_size_code = track.sectors().first().map(|s| s.id.size_code).unwrap_or(2);
    track_data[0x14] = sector_size_code;
    track_data[0x15] = track.sector_count() as u8;
    track_data[0x16] = track.gap3_length;
    track_data[0x17] = track.filler_byte;

    // Write sector info list (starts at offset 0x18, 8 bytes per sector)
    let mut sector_offset = TRACK_INFO_BLOCK_SIZE;

    for (i, sector) in track.sectors().iter().enumerate() {
        let sib_offset = 0x18 + (i * SECTOR_INFO_SIZE);
        if sib_offset + SECTOR_INFO_SIZE > track_data.len() {
            break;
        }

        let sib = &mut track_data[sib_offset..sib_offset + SECTOR_INFO_SIZE];

        sib[0] = sector.id.track;
        sib[1] = sector.id.side;
        sib[2] = sector.id.sector;
        sib[3] = sector.id.size_code;
        sib[4] = sector.fdc_status1.0;
        sib[5] = sector.fdc_status2.0;

        // Calculate stored size based on DSK format rules
        let max_stored = fdc_size_to_stored_bytes(sector.id.size_code);
        let sector_data = sector.data();
        let stored_len = sector_data.len().min(max_stored);

        // Write data length for extended format
        let data_len_bytes = (stored_len as u16).to_le_bytes();
        sib[6] = data_len_bytes[0];
        sib[7] = data_len_bytes[1];

        // Copy sector data (limited by DSK format stored size rules)
        let copy_len = stored_len.min(track_data.len() - sector_offset);
        if copy_len > 0 {
            track_data[sector_offset..sector_offset + copy_len]
                .copy_from_slice(&sector_data[..copy_len]);
        }
        sector_offset += stored_len;
    }

    file.write_all(&track_data)?;
    Ok(())
}

/// Calculate the size of a track in bytes (including track info block)
/// Uses DSK format stored size rules (max 6144 bytes per sector)
fn calculate_single_track_size(track: &crate::image::Track) -> usize {
    let mut size = TRACK_INFO_BLOCK_SIZE;
    for sector in track.sectors() {
        let max_stored = fdc_size_to_stored_bytes(sector.id.size_code);
        size += sector.actual_size().min(max_stored);
    }
    size
}

/// Calculate a uniform track size for standard format
fn calculate_track_size(image: &DiskImage) -> usize {
    // Find the largest track size
    let mut max_size = TRACK_INFO_BLOCK_SIZE;

    for disk in &image.disks {
        for track in disk.tracks() {
            let track_size = calculate_single_track_size(track);
            if track_size > max_size {
                max_size = track_size;
            }
        }
    }

    max_size
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fdc::FdcStatus2;
    use crate::image::{Sector, SectorId, Track};

    #[test]
    fn test_calculate_single_track_size() {
        let mut track = Track::new(0, 0);

        for i in 0..9 {
            let id = SectorId::new(0, 0, 0xC1 + i, 2);
            track.add_sector(Sector::new(id));
        }

        let size = calculate_single_track_size(&track);
        assert_eq!(size, 256 + 9 * 512); // Track info + 9 * 512-byte sectors
    }

    #[test]
    fn test_td0_round_trip() {
        let mut image = DiskImage::builder()
            .format(DiskImageFormat::RawTd0)
            .num_sides(1)
            .num_tracks(1)
            .sectors_per_track(2)
            .sector_size(512)
            .build()
            .unwrap();
        let data: Vec<u8> = (0..512).map(|value| value as u8).collect();
        image.write_sector(0, 0, 0xC1, &data).unwrap();
        image
            .get_disk_mut(0)
            .unwrap()
            .get_track_mut(0)
            .unwrap()
            .get_sector_mut(0xC2)
            .unwrap()
            .fdc_status2 = FdcStatus2::new(FdcStatus2::CM);

        let path =
            std::env::temp_dir().join(format!("dskmgr_td0_writer_{}.td0", std::process::id()));
        write_dsk(&image, &path).unwrap();
        let parsed = crate::io::read_td0(&path).unwrap();
        std::fs::remove_file(path).ok();

        assert_eq!(parsed.format(), DiskImageFormat::RawTd0);
        assert_eq!(parsed.read_sector(0, 0, 0xC1).unwrap(), data.as_slice());
        assert!(parsed
            .get_disk(0)
            .unwrap()
            .get_track(0)
            .unwrap()
            .get_sector(0xC2)
            .unwrap()
            .is_deleted());
        assert!(parsed.warnings().is_empty(), "{:?}", parsed.warnings());
    }
}
