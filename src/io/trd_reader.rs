/// TRD file reader
///
/// TRD files are raw sector dumps used by TR-DOS on the ZX Spectrum
/// with the Beta Disk Interface.
///
/// Format:
/// - Variable size (up to 655,360 bytes for 80-track double-sided)
/// - 16 sectors per track, 256 bytes per sector
/// - Sectors stored sequentially: T0S1..T0S16, T1S1..T1S16, ...
/// - Directory in track 0 (sectors 1-8), disk catalog at sector 9
/// - Double-sided images: sides stored sequentially (side 0 then side 1)

use crate::error::{DskError, Result};
use crate::format::{DiskImageFormat, FormatSpec, SideMode};
use crate::image::{Disk, DiskImage, Sector, SectorId, Track};
use std::fs::File;
use std::io::Read;
use std::path::Path;

/// Sectors per track in TRD format
pub const TRD_SECTORS_PER_TRACK: u8 = 16;
/// Sector size in bytes for TRD format
pub const TRD_SECTOR_SIZE: u16 = 256;
/// First sector ID (1-based)
pub const TRD_FIRST_SECTOR_ID: u8 = 1;
/// Default number of tracks
pub const TRD_DEFAULT_TRACKS: u8 = 80;

/// Calculate expected file size for a given number of tracks (single-sided)
pub fn trd_file_size(tracks: u8) -> usize {
    tracks as usize * TRD_SECTORS_PER_TRACK as usize * TRD_SECTOR_SIZE as usize
}

/// Maximum TRD file size (80 tracks, double-sided = 160 tracks * 16 sectors * 256 bytes)
pub const TRD_MAX_FILE_SIZE: usize = 655_360;

/// Check if a file is likely a TRD file based on extension
pub fn is_trd_file<P: AsRef<Path>>(path: P) -> bool {
    path.as_ref()
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.eq_ignore_ascii_case("trd"))
        .unwrap_or(false)
}

/// Read a TRD file from disk
pub fn read_trd<P: AsRef<Path>>(path: P) -> Result<DiskImage> {
    let filename = path
        .as_ref()
        .file_name()
        .and_then(|n| n.to_str())
        .map(|s| s.to_string());

    let mut file = File::open(&path)?;

    let file_len = file.metadata()?.len() as usize;
    if file_len == 0 || file_len > TRD_MAX_FILE_SIZE {
        return Err(DskError::invalid_format(&format!(
            "TRD file has unexpected size {} bytes (max {})",
            file_len,
            TRD_MAX_FILE_SIZE
        )));
    }

    let mut data = vec![0u8; file_len];
    file.read_exact(&mut data)?;

    let track_size = TRD_SECTORS_PER_TRACK as usize * TRD_SECTOR_SIZE as usize;
    let num_full_tracks = file_len / track_size;
    let remainder = file_len % track_size;

    let actual_tracks = if remainder > 0 {
        num_full_tracks + 1
    } else {
        num_full_tracks
    };

    let mut warnings = Vec::new();
    if remainder > 0 {
        warnings.push(format!(
            "File size {} is not a multiple of track size {}; last track is partial ({} of {} bytes)",
            file_len, track_size, remainder, track_size
        ));
    }

    let is_double_sided = detect_double_sided(&data, file_len, &mut warnings);
    let num_sides: u8 = if is_double_sided { 2 } else { 1 };
    let tracks_per_side = if is_double_sided {
        match data.get(8 * TRD_SECTOR_SIZE as usize + 227).copied() {
            Some(0x19) if actual_tracks <= 80 => 40,
            Some(0x16) => 80,
            _ if actual_tracks > 80 => 80,
            _ => actual_tracks.div_ceil(2),
        }
    } else {
        actual_tracks
    };
    let mut disks: Vec<Disk> = (0..num_sides).map(Disk::new).collect();
    for absolute_track in 0..actual_tracks {
        let side = if is_double_sided { absolute_track / tracks_per_side } else { 0 };
        if side >= disks.len() {
            break;
        }
        let track_num = (absolute_track % tracks_per_side) as u8;
        let offset = absolute_track * track_size;
        let track = if absolute_track < num_full_tracks {
            read_trd_track(&data[offset..offset + track_size], track_num, side as u8)?
        } else {
            read_trd_partial_track(&data[offset..], track_num, side as u8, remainder)?
        };
        disks[side].add_track(track);
    }
    for disk in &mut disks {
        disk.ensure_track_count(tracks_per_side);
    }

    let spec = FormatSpec {
        num_sides,
        num_tracks: tracks_per_side as u8,
        sectors_per_track: TRD_SECTORS_PER_TRACK,
        sector_size: TRD_SECTOR_SIZE,
        first_sector_id: TRD_FIRST_SECTOR_ID,
        gap3_length: 0x1B,
        filler_byte: 0x00,
        interleave: 1,
        side_mode: if is_double_sided {
            SideMode::Successive
        } else {
            SideMode::SingleSide
        },
    };

    Ok(DiskImage {
        format: DiskImageFormat::RawTrd,
        spec,
        disks,
        changed: false,
        filename,
        warnings,
    })
}

/// Detect whether a TRD image is double-sided based on disk type byte and file size
fn detect_double_sided(data: &[u8], file_len: usize, _warnings: &mut Vec<String>) -> bool {
    let catalog_offset = 8 * TRD_SECTOR_SIZE as usize;
    if data.len() > catalog_offset + 231 {
        let disk_type = data[catalog_offset + 227];
        match disk_type {
            0x16 | 0x19 => return true,
            0x17 | 0x18 => return false,
            _ => {}
        }
    }

    file_len > trd_file_size(80)
}

fn read_trd_track(data: &[u8], track_num: u8, side: u8) -> Result<Track> {
    let mut track = Track::new(track_num, side);
    track.filler_byte = 0x00;

    let sector_size = TRD_SECTOR_SIZE as usize;

    for sector_idx in 0..TRD_SECTORS_PER_TRACK {
        let offset = sector_idx as usize * sector_size;
        let sector_data = data[offset..offset + sector_size].to_vec();

        let sector_id = TRD_FIRST_SECTOR_ID + sector_idx;
        let id = SectorId::new(track_num, side, sector_id, 1); // Size code 1 = 256 bytes
        let sector = Sector::with_data(id, sector_data);

        track.add_sector(sector);
    }

    Ok(track)
}

fn read_trd_partial_track(data: &[u8], track_num: u8, side: u8, data_len: usize) -> Result<Track> {
    let mut track = Track::new(track_num, side);
    track.filler_byte = 0x00;

    let sector_size = TRD_SECTOR_SIZE as usize;
    let full_sectors = data_len / sector_size;
    let remainder = data_len % sector_size;

    for sector_idx in 0..full_sectors {
        let offset = sector_idx * sector_size;
        let sector_data = data[offset..offset + sector_size].to_vec();

        let sector_id = TRD_FIRST_SECTOR_ID + sector_idx as u8;
        let id = SectorId::new(track_num, side, sector_id, 1);
        let sector = Sector::with_data(id, sector_data);

        track.add_sector(sector);
    }

    if remainder > 0 {
        let offset = full_sectors * sector_size;
        let mut sector_data = vec![0u8; sector_size];
        sector_data[..remainder].copy_from_slice(&data[offset..data_len]);

        let sector_id = TRD_FIRST_SECTOR_ID + full_sectors as u8;
        let id = SectorId::new(track_num, side, sector_id, 1);
        let sector = Sector::with_data(id, sector_data);

        track.add_sector(sector);
    }

    // Pad remaining sectors with empty data
    let existing = full_sectors + if remainder > 0 { 1 } else { 0 };
    for sector_idx in existing..TRD_SECTORS_PER_TRACK as usize {
        let sector_id = TRD_FIRST_SECTOR_ID + sector_idx as u8;
        let id = SectorId::new(track_num, side, sector_id, 1);
        let sector = Sector::new(id);
        track.add_sector(sector);
    }

    Ok(track)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::filesystem::FileSystem;

    #[test]
    fn test_is_trd_file() {
        assert!(is_trd_file("test.trd"));
        assert!(is_trd_file("TEST.TRD"));
        assert!(is_trd_file("/path/to/disk.trd"));
        assert!(!is_trd_file("test.dsk"));
        assert!(!is_trd_file("test.mgt"));
    }

    #[test]
    fn test_trd_file_size() {
        assert_eq!(trd_file_size(80), 327_680);
        assert_eq!(trd_file_size(40), 163_840);
    }

    #[test]
    fn test_trd_constants() {
        let expected = TRD_DEFAULT_TRACKS as usize
            * TRD_SECTORS_PER_TRACK as usize
            * TRD_SECTOR_SIZE as usize;
        assert_eq!(expected, 327_680);
        assert_eq!(TRD_MAX_FILE_SIZE, 655_360);
    }

    #[test]
    fn test_double_sided_trd_roundtrip_and_file_on_side_one() {
        let mut bytes = vec![0u8; trd_file_size(80)]; // 40 tracks on each side
        bytes[8 * 256 + 227] = 0x19; // 40-track, double-sided catalog
        bytes[..8].copy_from_slice(b"SIDEONE ");
        bytes[8] = b'C';
        bytes[9..11].copy_from_slice(&256u16.to_le_bytes());
        bytes[13] = 1;
        bytes[14] = 0;
        bytes[15] = 40; // first track on side 1 in TR-DOS linear order
        bytes[40 * 4096] = 0x66;

        let src = std::env::temp_dir().join(format!("dskmgr_ds_{}.trd", std::process::id()));
        let dst = std::env::temp_dir().join(format!("dskmgr_ds_out_{}.trd", std::process::id()));
        std::fs::write(&src, &bytes).unwrap();
        let image = read_trd(&src).unwrap();
        assert_eq!(image.disk_count(), 2);
        assert_eq!(image.spec().num_tracks, 40);
        assert_eq!(image.read_sector(1, 0, 1).unwrap()[0], 0x66);
        let fs = crate::filesystem::TrdosFileSystem::new(&image).unwrap();
        assert_eq!(fs.read_file("SIDEONE").unwrap()[0], 0x66);
        crate::io::write_dsk(&image, &dst).unwrap();
        let output = std::fs::read(&dst).unwrap();
        std::fs::remove_file(src).ok();
        std::fs::remove_file(dst).ok();
        assert_eq!(output, bytes);
    }
}
