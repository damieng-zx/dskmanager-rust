//! MGT file reader
//!
//! MGT files are raw sector dumps used by:
//! - MGT +D / DISCiPLE for ZX Spectrum
//! - SAM Coupe
//!
//! Format:
//! - Fixed 819,200 bytes (800KB)
//! - 80 tracks per side, 2 sides
//! - 10 sectors per track, 512 bytes each
//! - Tracks alternate: S0T0, S1T0, S0T1, S1T1, ...

use crate::error::{DskError, Result};
use crate::format::{DiskImageFormat, FormatSpec, SideMode};
use crate::image::{Disk, DiskImage, Sector, SectorId, Track};
use std::fs::File;
use std::io::Read;
use std::path::Path;

/// Expected size of an MGT file
pub const MGT_FILE_SIZE: usize = 819_200;

/// Tracks per side
pub const MGT_TRACKS_PER_SIDE: u8 = 80;

/// Number of sides
pub const MGT_SIDES: u8 = 2;

/// Sectors per track
pub const MGT_SECTORS_PER_TRACK: u8 = 10;

/// Sector size in bytes
pub const MGT_SECTOR_SIZE: u16 = 512;

/// First sector ID (1-based)
pub const MGT_FIRST_SECTOR_ID: u8 = 1;

/// Check if a file is likely an MGT file based on extension
pub fn is_mgt_file<P: AsRef<Path>>(path: P) -> bool {
    path.as_ref()
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.eq_ignore_ascii_case("mgt"))
        .unwrap_or(false)
}

/// Read an MGT file from disk
pub fn read_mgt<P: AsRef<Path>>(path: P) -> Result<DiskImage> {
    let filename = path.as_ref()
        .file_name()
        .and_then(|n| n.to_str())
        .map(|s| s.to_string());

    let mut file = File::open(&path)?;

    // Check file size
    let metadata = file.metadata()?;
    if metadata.len() != MGT_FILE_SIZE as u64 {
        return Err(DskError::invalid_format(format!(
            "MGT file should be {} bytes, got {}",
            MGT_FILE_SIZE,
            metadata.len()
        )));
    }

    // Read entire file
    let mut data = vec![0u8; MGT_FILE_SIZE];
    file.read_exact(&mut data)?;

    // Create disks for both sides
    let mut disk0 = Disk::new(0);
    let mut disk1 = Disk::new(1);

    // Track data position
    let track_size = MGT_SECTORS_PER_TRACK as usize * MGT_SECTOR_SIZE as usize;
    let mut offset = 0;

    // MGT stores all side 0 tracks, then all side 1 tracks
    for track_num in 0..MGT_TRACKS_PER_SIDE {
        let track0 = read_mgt_track(&data[offset..offset + track_size], track_num, 0)?;
        disk0.add_track(track0);
        offset += track_size;
    }

    for track_num in 0..MGT_TRACKS_PER_SIDE {
        let track1 = read_mgt_track(&data[offset..offset + track_size], track_num, 1)?;
        disk1.add_track(track1);
        offset += track_size;
    }

    // Create format spec
    let spec = FormatSpec {
        num_sides: MGT_SIDES,
        num_tracks: MGT_TRACKS_PER_SIDE,
        sectors_per_track: MGT_SECTORS_PER_TRACK,
        sector_size: MGT_SECTOR_SIZE,
        first_sector_id: MGT_FIRST_SECTOR_ID,
        gap3_length: 0x17,
        filler_byte: 0x00,
        interleave: 1,
        side_mode: SideMode::Successive,
    };

    Ok(DiskImage {
        format: DiskImageFormat::RawMgt,
        spec,
        disks: vec![disk0, disk1],
        changed: false,
        filename,
        warnings: Vec::new(),
    })
}

/// Read a single MGT track from raw data
fn read_mgt_track(data: &[u8], track_num: u8, side: u8) -> Result<Track> {
    let mut track = Track::new(track_num, side);
    track.filler_byte = 0x00;

    let sector_size = MGT_SECTOR_SIZE as usize;

    for sector_idx in 0..MGT_SECTORS_PER_TRACK {
        let offset = sector_idx as usize * sector_size;
        let sector_data = data[offset..offset + sector_size].to_vec();

        // MGT uses 1-based sector IDs
        let sector_id = MGT_FIRST_SECTOR_ID + sector_idx;
        let id = SectorId::new(track_num, side, sector_id, 2); // Size code 2 = 512 bytes
        let sector = Sector::with_data(id, sector_data);

        track.add_sector(sector);
    }

    Ok(track)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_mgt_file() {
        assert!(is_mgt_file("test.mgt"));
        assert!(is_mgt_file("TEST.MGT"));
        assert!(is_mgt_file("/path/to/disk.mgt"));
        assert!(!is_mgt_file("test.dsk"));
        assert!(!is_mgt_file("test.txt"));
    }

    #[test]
    fn test_mgt_constants() {
        // Verify expected file size
        let expected_size = MGT_TRACKS_PER_SIDE as usize
            * MGT_SIDES as usize
            * MGT_SECTORS_PER_TRACK as usize
            * MGT_SECTOR_SIZE as usize;
        assert_eq!(expected_size, MGT_FILE_SIZE);
    }

    #[test]
    fn test_wrong_size_reports_both_sizes() {
        let path = std::env::temp_dir().join(format!("dskmgr_short_{}.mgt", std::process::id()));
        std::fs::write(&path, vec![0u8; 1000]).unwrap();
        let error = read_mgt(&path).unwrap_err().to_string();
        std::fs::remove_file(&path).ok();
        assert!(error.contains("819200"), "error should name the expected size: {error}");
        assert!(error.contains("1000"), "error should name the actual size: {error}");
    }
}
