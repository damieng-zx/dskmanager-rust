//! Boot detection for disk images
//!
//! Examines disk images to determine what system they are bootable on
//! and provides a reason for the detection.

use crate::filesystem::CpmFileSystem;
use crate::filesystem::try_plus3dos_header;
use crate::image::DiskImage;

/// Result of boot detection
#[derive(Debug, Clone)]
pub struct BootDetection {
    /// System name the disk is bootable on, or empty string if not bootable
    pub system: String,
    /// Reason for the detection
    pub reason: String,
}

impl BootDetection {
    /// Detect what system a disk image is bootable on
    ///
    /// Returns a `BootDetection` with the system name and reason.
    /// If the disk is not bootable, `system` will be an empty string.
    pub fn detect(image: &DiskImage) -> Self {
        // Compressed check: no sector on track 0 if side 0 missing, track 0 missing/unformatted, or 0 sectors
        let track = match image.get_disk(0)
            .and_then(|disk| disk.get_track(0))
        {
            Some(t) if !t.is_empty() && t.sector_count() > 0 => t,
            _ => {
                return BootDetection {
                    system: String::new(),
                    reason: "No sector on track 0".to_string(),
                };
            }
        };

        // Check sector 0 for corruption flags
        let mut corrupt = false;
        if let Some(sector0) = track.get_sector_by_index(0) {
            // Check FDCStatus1 bit 5 (DE = 0x20 = 32) or FDCStatus2 bit 6 (CM = 0x40 = 64)
            if (sector0.fdc_status1.0 & 0x20) != 0 || (sector0.fdc_status2.0 & 0x40) != 0 {
                corrupt = true;
            }
        }

        // Check CPC system disk (sector ID check) before mod checks
        let low_sector_id = track
            .sectors()
            .iter()
            .map(|s| s.id.sector)
            .min()
            .unwrap_or(0);

        if low_sector_id == 65 {
            let reason = if corrupt {
                format!("Amstrad CPC system disk - first sector is {} (Corrupt?)", low_sector_id)
            } else {
                format!("Amstrad CPC system disk - first sector is {}", low_sector_id)
            };
            return BootDetection {
                system: "Amstrad CPC 664/6128".to_string(),
                reason,
            };
        }

        // Need the first logical sector (lowest ID) for the mod checks, matching
        // the sector the format detector examines.
        let first_logical = match track.sectors().iter().min_by_key(|s| s.id.sector) {
            Some(s) => s,
            None => {
                return BootDetection {
                    system: String::new(),
                    reason: "No sector on track 0".to_string(),
                };
            }
        };

        // Calculate mod 256 checksum of the first logical sector's data
        let mod256 = calculate_mod_checksum(first_logical.data(), 256);

        // Determine system based on checksum
        let (system, reason) = match mod256 {
            1 => (
                "Amstrad PCW 9512".to_string(),
                format!("Sector 1 checksum {}", mod256),
            ),
            3 => (
                "Spectrum +3".to_string(),
                format!("Sector 1 checksum {}", mod256),
            ),
            255 => (
                "Amstrad PCW 8256".to_string(),
                format!("Sector 1 checksum {}", mod256),
            ),
            _ => (
                String::new(),
                format!("No valid checksum ({})", mod256),
            ),
        };

        // Add corruption note if detected
        let final_reason = if corrupt {
            format!("{} (Corrupt?)", reason)
        } else {
            reason
        };

        // If no system detected yet, try CP/M filesystem check for DISK file
        if system.is_empty() {
            // Try to mount CP/M filesystem and check for DISK file
            if let Ok(cpm_fs) = CpmFileSystem::from_image(image) {
                // Try to read DISK file with headers included
                if let Ok(disk_file_data) = cpm_fs.read_file_binary("DISK", true) {
                    // Check if it has a PLUS3DOS header
                    if try_plus3dos_header(&disk_file_data).is_some() {
                        // Check if header type is BASIC (type 0)
                        // In PLUS3DOS, byte 15 is the file type, 0 = BASIC
                        if disk_file_data.len() >= 128 && disk_file_data[15] == 0 {
                            return BootDetection {
                                system: "Spectrum +3".to_string(),
                                reason: "BASIC 'DISK' program exists".to_string(),
                            };
                        }
                    }
                }
            }
        }

        BootDetection {
            system,
            reason: final_reason,
        }
    }
}

/// Calculate a modulo checksum of data
fn calculate_mod_checksum(data: &[u8], mod_value: usize) -> u8 {
    let sum: usize = data.iter().map(|&b| b as usize).sum();
    (sum % mod_value) as u8
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::format::{DiskSpecification, FormatSpec};
    use crate::image::DiskImageBuilder;

    #[test]
    fn test_mod_checksum() {
        let data = vec![1, 2, 3, 4, 5];
        let checksum = calculate_mod_checksum(&data, 256);
        assert_eq!(checksum, ((1u16 + 2 + 3 + 4 + 5) % 256) as u8);
    }

    #[test]
    fn test_boot_detection_no_disk() {
        let image = DiskImageBuilder::new().build().unwrap();
        let detection = BootDetection::detect(&image);
        assert_eq!(detection.system, "");
    }

    #[test]
    fn test_boot_detection_uses_first_logical_sector() {
        // The spec detector checksums the lowest-ID sector of track 0, but boot
        // detection checksummed the second physical sector. When the two differ
        // the disk is identified by one and called unbootable by the other.
        let mut image = DiskImageBuilder::new()
            .spec(FormatSpec::spectrum_plus3())
            .build()
            .unwrap();

        let mod256 = |d: &[u8]| d.iter().map(|&b| b as usize).sum::<usize>() % 256;
        let disk = image.get_disk_mut(0).unwrap();
        let track = disk.get_track_mut(0).unwrap();

        // First logical sector (ID 1) -> 3 mod 256, touching a byte past the
        // spec block so the blank-format fallback still applies.
        let sector1 = track.get_sector_mut(1).unwrap();
        let delta = (3 + 256 - mod256(sector1.data())) % 256;
        let byte = &mut sector1.data_mut()[100];
        *byte = byte.wrapping_add(delta as u8);
        // Second sector (ID 2) -> anything but 3.
        let sector2 = track.get_sector_mut(2).unwrap();
        if mod256(sector2.data()) == 3 {
            sector2.data_mut()[100] = sector2.data()[100].wrapping_add(1);
        }

        assert_eq!(DiskSpecification::identify(&image).format, "Amstrad PCW/+3 DD/SS/ST (Assumed)");
        let detection = BootDetection::detect(&image);
        assert_eq!(detection.system, "Spectrum +3", "reason: {}", detection.reason);
    }

    #[test]
    fn test_boot_detection_spectrum_plus3() {
        let mut image = DiskImageBuilder::new()
            .num_sides(1)
            .num_tracks(1)
            .sectors_per_track(2)
            .spec(FormatSpec::spectrum_plus3())
            .build()
            .unwrap();

        // Modify existing sectors created by builder
        let disk = image.get_disk_mut(0).unwrap();
        let track = disk.get_track_mut(0).unwrap();

        // The checksum is taken over the first logical sector (lowest ID), which
        // is sector index 0 here.
        if let Some(sector0) = track.get_sector_by_index_mut(0) {
            // Fill with varied data
            for (i, byte) in sector0.data_mut().iter_mut().enumerate() {
                *byte = (i % 256) as u8;
            }
            // Calculate current sum mod 256
            let current_sum: usize = sector0.data().iter().map(|&b| b as usize).sum();
            let current_mod = current_sum % 256;
            // Adjust to get mod 256 = 3
            let adjustment = if current_mod <= 3 {
                3 - current_mod
            } else {
                256 + 3 - current_mod
            };
            // Add adjustment to first byte (wrapping if needed)
            sector0.data_mut()[0] = sector0.data()[0].wrapping_add(adjustment as u8);
        }

        let detection = BootDetection::detect(&image);
        assert_eq!(detection.system, "Spectrum +3");
        assert!(detection.reason.contains("checksum 3"));
    }
}
