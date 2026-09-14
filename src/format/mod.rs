/// DSK format specifications and constants

/// Format constants
pub mod constants;
/// Format specification types
pub mod spec;
/// Disk specification for CP/M filesystems
pub mod specification;

pub use constants::*;
pub use spec::{FormatSpec, SideMode};
pub use specification::{
    AllocationSize, DiskSpecSide, DiskSpecTrack, DiskSpecification,
};

use crate::filesystem::FileSystemType;

/// DSK format type
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiskImageFormat {
    /// Standard DSK format with fixed track sizes
    StandardDSK,
    /// Extended DSK format with variable track sizes
    ExtendedDSK,
    /// Raw MGT format (819,200 byte sector dump)
    RawMgt,
    /// Raw TRD format (TR-DOS sector dump, 655,360 bytes)
    RawTrd,
    /// TeleDisk format (TD0), normalized to tracks and sectors
    RawTd0,
}

impl DiskImageFormat {
    /// Get the magic bytes for this format
    pub fn magic_bytes(&self) -> &'static [u8] {
        match self {
            DiskImageFormat::StandardDSK => STANDARD_DSK_SIGNATURE,
            DiskImageFormat::ExtendedDSK => EXTENDED_DSK_SIGNATURE,
            DiskImageFormat::RawMgt => &[], // Raw MGT has no magic bytes
            DiskImageFormat::RawTrd => &[], // Raw TRD has no magic bytes
            DiskImageFormat::RawTd0 => b"TD", // TD0 accepts both TD and td
        }
    }

    /// Get a human-readable name for this format
    pub fn name(&self) -> &'static str {
        match self {
            DiskImageFormat::StandardDSK => "Standard DSK",
            DiskImageFormat::ExtendedDSK => "Extended DSK",
            DiskImageFormat::RawMgt => "Raw MGT",
            DiskImageFormat::RawTrd => "TR-DOS (TRD)",
            DiskImageFormat::RawTd0 => "TeleDisk (TD0)",
        }
    }

    /// Get the default filesystem type for this image format
    pub fn default_filesystem(&self) -> FileSystemType {
        match self {
            DiskImageFormat::StandardDSK => FileSystemType::Cpm,
            DiskImageFormat::ExtendedDSK => FileSystemType::Cpm,
            DiskImageFormat::RawMgt => FileSystemType::Mgt,
            DiskImageFormat::RawTrd => FileSystemType::Trdos,
            DiskImageFormat::RawTd0 => FileSystemType::Cpm,
        }
    }
}

/// Detect DSK format from magic bytes
pub fn detect_format(magic: &[u8]) -> Option<DiskImageFormat> {
    if magic.len() < 8 {
        return None;
    }

    if magic.starts_with(b"EXTENDED") {
        Some(DiskImageFormat::ExtendedDSK)
    } else if magic.starts_with(b"MV - CPC") {
        Some(DiskImageFormat::StandardDSK)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_standard_format() {
        let result = detect_format(STANDARD_DSK_SIGNATURE);
        assert_eq!(result, Some(DiskImageFormat::StandardDSK));
    }

    #[test]
    fn test_detect_extended_format() {
        let result = detect_format(EXTENDED_DSK_SIGNATURE);
        assert_eq!(result, Some(DiskImageFormat::ExtendedDSK));
    }

    #[test]
    fn test_detect_invalid_format() {
        let result = detect_format(b"INVALID DATA");
        assert_eq!(result, None);
    }

    #[test]
    fn test_format_magic_bytes() {
        assert_eq!(
            DiskImageFormat::StandardDSK.magic_bytes(),
            STANDARD_DSK_SIGNATURE
        );
        assert_eq!(
            DiskImageFormat::ExtendedDSK.magic_bytes(),
            EXTENDED_DSK_SIGNATURE
        );
    }
}
