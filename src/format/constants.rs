//! DSK format magic bytes and constants
/// Standard DSK format signature
pub const STANDARD_DSK_SIGNATURE: &[u8] = b"MV - CPCEMU Disk-File\r\nDisk-Info\r\n";

/// Extended DSK format signature
pub const EXTENDED_DSK_SIGNATURE: &[u8] = b"EXTENDED CPC DSK File\r\nDisk-Info\r\n";

/// Track-Info block marker
pub const TRACK_INFO_MARKER: &[u8] = b"Track-Info\r\n";

/// Offset-Info block marker (V5 extension)
pub const OFFSET_INFO_MARKER: &[u8] = b"Offset-Info\r\n";

/// Creator signature for this library
pub const CREATOR_SIGNATURE: &[u8] = b"dskmanager v0.1\0\0";

/// Maximum number of tracks per side
pub const MAX_TRACKS: usize = 204;

/// Maximum number of sectors per track
pub const MAX_SECTORS_PER_TRACK: usize = 29;

/// Largest sector data size the FDC can address (size code 8)
pub const MAX_SECTOR_SIZE: usize = 32768;

/// Size of disk info block
pub const DISK_INFO_BLOCK_SIZE: usize = 256;

/// Size of track info block
pub const TRACK_INFO_BLOCK_SIZE: usize = 256;

/// Size of sector info entry
pub const SECTOR_INFO_SIZE: usize = 8;

/// FDC sector size code to actual byte size mapping
/// Index: size_code (0-8), Value: actual size in bytes
pub const FDC_SECTOR_SIZES: [usize; 9] = [
    128,    // 0
    256,    // 1
    512,    // 2
    1024,   // 3
    2048,   // 4
    4096,   // 5
    8192,   // 6
    16384,  // 7
    32768,  // 8
];

/// Maximum stored data size per sector in DSK format
/// Size code 6 (8192) is truncated to 6144, size codes 7+ store nothing
pub const MAX_STORED_SECTOR_SIZE: usize = 6144;

/// FDC sector size code to stored byte size mapping for DSK format
/// Index: size_code (0-8), Value: bytes actually stored in DSK file
pub const FDC_STORED_SIZES: [usize; 9] = [
    128,    // 0: full size
    256,    // 1: full size
    512,    // 2: full size
    1024,   // 3: full size
    2048,   // 4: full size
    4096,   // 5: full size
    6144,   // 6: truncated from 8192
    0,      // 7: nothing stored
    0,      // 8: nothing stored
];

/// Convert FDC size code to actual byte size
#[inline]
pub fn fdc_size_to_bytes(size_code: u8) -> usize {
    if size_code as usize >= FDC_SECTOR_SIZES.len() {
        // Invalid size code (9+), return 0
        0
    } else {
        FDC_SECTOR_SIZES[size_code as usize]
    }
}

/// Convert FDC size code to stored byte size in DSK format
/// This accounts for DSK format limitations:
/// - Size codes 0-5: full sector size stored
/// - Size code 6: only 6144 of 8192 bytes stored
/// - Size codes 7+: nothing stored
#[inline]
pub fn fdc_size_to_stored_bytes(size_code: u8) -> usize {
    if size_code as usize >= FDC_STORED_SIZES.len() {
        // Invalid size code (9+), nothing stored
        0
    } else {
        FDC_STORED_SIZES[size_code as usize]
    }
}

/// Convert byte size to FDC size code
#[inline]
pub fn bytes_to_fdc_size(bytes: usize) -> Option<u8> {
    match bytes {
        128 => Some(0),
        256 => Some(1),
        512 => Some(2),
        1024 => Some(3),
        2048 => Some(4),
        4096 => Some(5),
        8192 => Some(6),
        16384 => Some(7),
        32768 => Some(8),
        _ => None,
    }
}

/// Offset of magic bytes in disk info block
pub const DISK_INFO_MAGIC_OFFSET: usize = 0;

/// Offset of creator in disk info block
pub const DISK_INFO_CREATOR_OFFSET: usize = 34;

/// Offset of track count in disk info block
pub const DISK_INFO_TRACK_COUNT_OFFSET: usize = 0x30;

/// Offset of side count in disk info block
pub const DISK_INFO_SIDE_COUNT_OFFSET: usize = 0x31;

/// Offset of track size in disk info block (standard format)
pub const DISK_INFO_TRACK_SIZE_OFFSET: usize = 0x32;

/// Offset of extended track size table in disk info block (extended format)
pub const DISK_INFO_EXT_TRACK_SIZE_OFFSET: usize = 0x34;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fdc_size_to_bytes() {
        assert_eq!(fdc_size_to_bytes(0), 128);
        assert_eq!(fdc_size_to_bytes(1), 256);
        assert_eq!(fdc_size_to_bytes(2), 512);
        assert_eq!(fdc_size_to_bytes(3), 1024);
        assert_eq!(fdc_size_to_bytes(8), 32768);
    }

    #[test]
    fn test_fdc_size_to_bytes_invalid() {
        // Invalid size codes (9+) should return 0
        assert_eq!(fdc_size_to_bytes(9), 0);
        assert_eq!(fdc_size_to_bytes(255), 0);
    }

    #[test]
    fn test_bytes_to_fdc_size() {
        assert_eq!(bytes_to_fdc_size(128), Some(0));
        assert_eq!(bytes_to_fdc_size(256), Some(1));
        assert_eq!(bytes_to_fdc_size(512), Some(2));
        assert_eq!(bytes_to_fdc_size(1024), Some(3));
        assert_eq!(bytes_to_fdc_size(32768), Some(8));
    }

    #[test]
    fn test_bytes_to_fdc_size_invalid() {
        assert_eq!(bytes_to_fdc_size(100), None);
        assert_eq!(bytes_to_fdc_size(1000), None);
    }

    #[test]
    fn test_fdc_size_to_stored_bytes() {
        // Size codes 0-5: full size stored
        assert_eq!(fdc_size_to_stored_bytes(0), 128);
        assert_eq!(fdc_size_to_stored_bytes(1), 256);
        assert_eq!(fdc_size_to_stored_bytes(2), 512);
        assert_eq!(fdc_size_to_stored_bytes(3), 1024);
        assert_eq!(fdc_size_to_stored_bytes(4), 2048);
        assert_eq!(fdc_size_to_stored_bytes(5), 4096);
        // Size code 6: truncated to 6144
        assert_eq!(fdc_size_to_stored_bytes(6), 6144);
        // Size codes 7+: nothing stored
        assert_eq!(fdc_size_to_stored_bytes(7), 0);
        assert_eq!(fdc_size_to_stored_bytes(8), 0);
        assert_eq!(fdc_size_to_stored_bytes(9), 0);
        assert_eq!(fdc_size_to_stored_bytes(255), 0);
    }

    #[test]
    fn test_round_trip_conversion() {
        for size_code in 0..=8 {
            let bytes = fdc_size_to_bytes(size_code);
            assert_eq!(bytes_to_fdc_size(bytes), Some(size_code));
        }
    }
}
