//! MGT filesystem implementation
//!
//! Base implementation for MGT-format disk filesystems used by:
//! - MGT +D / DISCiPLE for ZX Spectrum
//! - SAM Coupe
//!
//! Directory format:
//! - First 4 tracks (40 sectors) reserved for directory
//! - Each file entry is 256 bytes (2 per sector)
//! - Max 80 directory entries

use crate::error::{DskError, Result};
use crate::filesystem::{ExtendedDirEntry, FileAttributes, FileHeader, HeaderType};
use crate::image::DiskImage;

/// Number of directory tracks
pub const MGT_DIR_TRACKS: usize = 4;

/// Sectors per track
pub const MGT_SECTORS_PER_TRACK: usize = 10;

/// Number of directory sectors (first 4 tracks on both sides)
pub const MGT_DIR_SECTORS: usize = MGT_DIR_TRACKS * MGT_SECTORS_PER_TRACK;

/// Size of each directory entry
pub const MGT_DIR_ENTRY_SIZE: usize = 256;

/// Entries per sector
pub const MGT_ENTRIES_PER_SECTOR: usize = 512 / MGT_DIR_ENTRY_SIZE;

/// Maximum directory entries
pub const MGT_MAX_DIR_ENTRIES: usize = MGT_DIR_SECTORS * MGT_ENTRIES_PER_SECTOR;

/// File type codes
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MgtFileType {
    /// Erased/free entry
    Erased,
    /// ZX Spectrum snapshot (0x05)
    ZxSnapshot,
    /// SAM BASIC program (0x10)
    SamBasic,
    /// Numeric array (0x11)
    NumericArray,
    /// String array (0x12)
    StringArray,
    /// CODE/binary (0x13)
    Code,
    /// SCREEN$ (0x14)
    Screen,
    /// Unknown/other type
    Other(u8),
}

impl MgtFileType {
    /// Parse file type from status byte
    pub fn from_status(status: u8) -> Self {
        // Status byte: bit 7 = hidden, bit 6 = protected, bits 0-5 = type
        let type_code = status & 0x3F;
        match type_code {
            0x00 => MgtFileType::Erased,
            0x05 => MgtFileType::ZxSnapshot,
            0x10 => MgtFileType::SamBasic,
            0x11 => MgtFileType::NumericArray,
            0x12 => MgtFileType::StringArray,
            0x13 => MgtFileType::Code,
            0x14 => MgtFileType::Screen,
            other => MgtFileType::Other(other),
        }
    }

    /// Get type code
    pub fn type_code(&self) -> u8 {
        match self {
            MgtFileType::Erased => 0x00,
            MgtFileType::ZxSnapshot => 0x05,
            MgtFileType::SamBasic => 0x10,
            MgtFileType::NumericArray => 0x11,
            MgtFileType::StringArray => 0x12,
            MgtFileType::Code => 0x13,
            MgtFileType::Screen => 0x14,
            MgtFileType::Other(code) => *code,
        }
    }
}

impl std::fmt::Display for MgtFileType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MgtFileType::Erased => write!(f, "Erased"),
            MgtFileType::ZxSnapshot => write!(f, "ZX Snapshot"),
            MgtFileType::SamBasic => write!(f, "SAM BASIC"),
            MgtFileType::NumericArray => write!(f, "Numeric Array"),
            MgtFileType::StringArray => write!(f, "String Array"),
            MgtFileType::Code => write!(f, "CODE"),
            MgtFileType::Screen => write!(f, "SCREEN$"),
            MgtFileType::Other(code) => write!(f, "Type 0x{:02X}", code),
        }
    }
}

/// MGT directory entry (256 bytes)
#[derive(Debug, Clone)]
pub struct MgtDirEntry {
    /// Directory entry index
    pub index: usize,
    /// File type
    pub file_type: MgtFileType,
    /// Hidden flag
    pub hidden: bool,
    /// Protected flag
    pub protected: bool,
    /// Filename (10 characters)
    pub filename: String,
    /// Number of sectors used
    pub sectors_used: u16,
    /// Start track
    pub start_track: u8,
    /// Start sector
    pub start_sector: u8,
    /// Sector address map (195 bytes bitmap)
    pub sector_map: Vec<u8>,
    /// Raw entry data for system-specific parsing
    pub raw_data: Vec<u8>,
}

impl MgtDirEntry {
    /// Parse a directory entry from 256 bytes
    pub fn parse(data: &[u8], index: usize) -> Option<Self> {
        if data.len() < MGT_DIR_ENTRY_SIZE {
            return None;
        }

        let status = data[0];
        let file_type = MgtFileType::from_status(status);

        // Skip erased entries
        if matches!(file_type, MgtFileType::Erased) {
            return None;
        }

        let hidden = (status & 0x80) != 0;
        let protected = (status & 0x40) != 0;

        // Filename is bytes 1-10
        let filename_bytes = &data[1..11];
        let filename = String::from_utf8_lossy(filename_bytes)
            .trim_end()
            .to_string();

        // Skip entries with blank filenames
        if filename.is_empty() || filename.chars().all(|c| c == ' ' || c == '\0') {
            return None;
        }

        // Sectors used: bytes 11-12, MSB first (big endian)
        let sectors_used = u16::from_be_bytes([data[11], data[12]]);

        // Start track and sector: bytes 13-14
        let start_track = data[13];
        let start_sector = data[14];

        // Sector address map: bytes 15-209 (195 bytes)
        let sector_map = data[15..210].to_vec();

        // Keep raw data for system-specific parsing
        let raw_data = data[..MGT_DIR_ENTRY_SIZE].to_vec();

        Some(Self {
            index,
            file_type,
            hidden,
            protected,
            filename,
            sectors_used,
            start_track,
            start_sector,
            sector_map,
            raw_data,
        })
    }

    /// Get file size in bytes
    pub fn file_size(&self) -> usize {
        self.sectors_used as usize * 512
    }

    /// Get file attributes
    pub fn attributes(&self) -> FileAttributes {
        FileAttributes {
            read_only: self.protected,
            system: self.hidden,
            archive: false,
        }
    }

    /// Check if this entry is a ZX Spectrum type (for Disciple/+D)
    pub fn is_spectrum_type(&self) -> bool {
        matches!(self.file_type, MgtFileType::ZxSnapshot)
    }

    /// Check if this entry is a SAM type
    pub fn is_sam_type(&self) -> bool {
        matches!(
            self.file_type,
            MgtFileType::SamBasic
                | MgtFileType::NumericArray
                | MgtFileType::StringArray
                | MgtFileType::Code
                | MgtFileType::Screen
        )
    }
}

/// System type for MGT filesystem
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MgtSystemType {
    /// Unknown system
    Unknown,
    /// MGT +D or DISCiPLE (ZX Spectrum)
    Disciple,
    /// SAM Coupe
    Sam,
}

impl std::fmt::Display for MgtSystemType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MgtSystemType::Unknown => write!(f, "MGT"),
            MgtSystemType::Disciple => write!(f, "DISCiPLE/+D"),
            MgtSystemType::Sam => write!(f, "SAM Coupe"),
        }
    }
}

/// Base MGT filesystem implementation
pub struct MgtFileSystem<'a> {
    image: &'a DiskImage,
    directory_entries: Vec<MgtDirEntry>,
    system_type: MgtSystemType,
}

impl<'a> MgtFileSystem<'a> {
    /// Create a new MGT filesystem from an image
    pub fn new(image: &'a DiskImage) -> Result<Self> {
        let directory_entries = Self::read_directory(image)?;
        let system_type = Self::detect_system_type(&directory_entries);

        Ok(Self {
            image,
            directory_entries,
            system_type,
        })
    }

    /// Read directory entries from the disk
    fn read_directory(image: &DiskImage) -> Result<Vec<MgtDirEntry>> {
        let mut entries = Vec::new();
        let mut entry_index = 0;

        // Directory is in the first 4 tracks on side 0
        // (In MGT format, side 0 and side 1 alternate, but directory is on side 0 tracks 0-3)
        let disk = image
            .get_disk(0)
            .ok_or_else(|| DskError::filesystem("No disk side 0"))?;

        for track_num in 0..MGT_DIR_TRACKS as u8 {
            let track = match disk.get_track(track_num) {
                Some(t) => t,
                None => continue,
            };

            // Read sectors in order (1-10)
            for sector_id in 1..=MGT_SECTORS_PER_TRACK as u8 {
                let sector = match track.get_sector(sector_id) {
                    Some(s) => s,
                    None => continue,
                };

                let sector_data = sector.data();

                // Each sector has 2 directory entries (256 bytes each)
                for i in 0..MGT_ENTRIES_PER_SECTOR {
                    let offset = i * MGT_DIR_ENTRY_SIZE;
                    if offset + MGT_DIR_ENTRY_SIZE <= sector_data.len() {
                        if let Some(entry) =
                            MgtDirEntry::parse(&sector_data[offset..], entry_index)
                        {
                            entries.push(entry);
                        }
                    }
                    entry_index += 1;
                }
            }
        }

        Ok(entries)
    }

    /// Detect the system type based on file types present
    fn detect_system_type(entries: &[MgtDirEntry]) -> MgtSystemType {
        let has_spectrum = entries.iter().any(|e| e.is_spectrum_type());
        let has_sam = entries.iter().any(|e| e.is_sam_type());

        if has_sam && !has_spectrum {
            MgtSystemType::Sam
        } else if has_spectrum {
            MgtSystemType::Disciple
        } else {
            MgtSystemType::Unknown
        }
    }

    /// Get the detected system type
    pub fn system_type(&self) -> MgtSystemType {
        self.system_type
    }

    /// Get directory entries
    pub fn directory(&self) -> &[MgtDirEntry] {
        &self.directory_entries
    }

    /// The image this filesystem was mounted from.
    pub fn image(&self) -> &'a DiskImage {
        self.image
    }

    /// Read file data by following the sector map
    /// Returns file data truncated to actual file length (not allocated size)
    pub fn read_file(&self, entry: &MgtDirEntry) -> Result<Vec<u8>> {
        let mut data = Vec::new();
        let sectors_to_read = entry.sectors_used as usize;

        if sectors_to_read == 0 {
            return Ok(data);
        }

        // Start from the start track/sector and follow the allocation
        let mut current_track = entry.start_track;
        let mut current_sector = entry.start_sector;
        let mut sectors_read = 0;

        while sectors_read < sectors_to_read {
            // Determine which side this track is on
            // In MGT, logical track N maps to:
            // - Physical track N/2 on side N%2 (for standard layout)
            // But our image already has the tracks properly separated
            let side = if current_track >= 128 {
                1
            } else {
                0
            };
            let phys_track = if current_track >= 128 {
                current_track - 128
            } else {
                current_track
            };

            let disk = self.image.get_disk(side).ok_or_else(|| {
                DskError::filesystem(format!("No disk side {}", side))
            })?;

            let track = disk.get_track(phys_track).ok_or_else(|| {
                DskError::filesystem(format!("Track {} not found", phys_track))
            })?;

            let sector = track.get_sector(current_sector).ok_or_else(|| {
                DskError::filesystem(format!(
                    "Sector {} not found on track {}",
                    current_sector, phys_track
                ))
            })?;

            data.extend_from_slice(sector.data());
            sectors_read += 1;

            // Move to next sector
            current_sector += 1;
            if current_sector > 10 {
                current_sector = 1;
                current_track += 1;
            }
        }

        // Trim to actual file size (MGT stores metadata in directory entry, not headers in file data)
        // For Disciple/+D, actual file size is stored at offset 212-213 in directory entry
        if entry.raw_data.len() >= 214 {
            let actual_file_size = u16::from_le_bytes([entry.raw_data[212], entry.raw_data[213]]) as usize;
            if actual_file_size > 0 && actual_file_size < data.len() {
                data.truncate(actual_file_size);
            }
        }

        Ok(data)
    }

    /// Read extended directory listing
    pub fn read_dir_extended(&self) -> Result<Vec<ExtendedDirEntry>> {
        let mut entries = Vec::new();

        for dir_entry in &self.directory_entries {
            // Parse metadata from directory entry (MGT stores metadata in directory, not headers in file data)
            let header = if dir_entry.sectors_used > 0 {
                match self.read_file(dir_entry) {
                    Ok(data) => self.parse_file_header(dir_entry, &data),
                    Err(_) => FileHeader::default(),
                }
            } else {
                FileHeader::default()
            };

            entries.push(ExtendedDirEntry {
                name: dir_entry.filename.clone(),
                user: 0, // MGT doesn't have user numbers
                index: dir_entry.index,
                blocks: dir_entry.sectors_used as usize,
                allocated: dir_entry.file_size(),
                size: self.recorded_file_size(dir_entry),
                attributes: dir_entry.attributes(),
                header,
            });
        }

        Ok(entries)
    }

    /// Actual file length. Disciple/+D store it at 0xD4; when that field is
    /// absent or zero the whole allocation is in use.
    fn recorded_file_size(&self, entry: &MgtDirEntry) -> usize {
        let raw = &entry.raw_data;
        if raw.len() >= 214 {
            let recorded = u16::from_le_bytes([raw[212], raw[213]]) as usize;
            if recorded > 0 && recorded <= entry.file_size() {
                return recorded;
            }
        }
        entry.file_size()
    }

    /// Parse file metadata - can be overridden by subclasses
    /// MGT filesystems store metadata in directory entries, not headers in file data
    fn parse_file_header(&self, entry: &MgtDirEntry, _data: &[u8]) -> FileHeader {
        // Create metadata string based on file type
        let meta = format!("{}", entry.file_type);

        FileHeader {
            header_type: HeaderType::None,
            checksum_valid: false,
            file_size: entry.file_size(),
            header_size: 0,
            meta,
        }
    }

    /// Find a file by name
    pub fn find_file(&self, name: &str) -> Option<&MgtDirEntry> {
        let name_upper = name.to_uppercase();
        self.directory_entries
            .iter()
            .find(|e| e.filename.to_uppercase() == name_upper)
    }

    /// Get filesystem information
    pub fn info(&self) -> MgtFileSystemInfo {
        let total_sectors = 80 * 10 * 2; // 80 tracks * 10 sectors * 2 sides
        let dir_sectors = MGT_DIR_SECTORS;
        let data_sectors = total_sectors - dir_sectors;

        let used_sectors: usize = self
            .directory_entries
            .iter()
            .map(|e| e.sectors_used as usize)
            .sum();

        MgtFileSystemInfo {
            system_type: self.system_type,
            total_sectors,
            dir_sectors,
            used_sectors,
            free_sectors: data_sectors.saturating_sub(used_sectors),
            file_count: self.directory_entries.len(),
        }
    }
}

/// MGT filesystem information
#[derive(Debug)]
pub struct MgtFileSystemInfo {
    /// Detected system type
    pub system_type: MgtSystemType,
    /// Total sectors on disk
    pub total_sectors: usize,
    /// Sectors used for directory
    pub dir_sectors: usize,
    /// Sectors used by files
    pub used_sectors: usize,
    /// Free sectors
    pub free_sectors: usize,
    /// Number of files
    pub file_count: usize,
}

impl std::fmt::Display for MgtFileSystemInfo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "MGT Filesystem ({})", self.system_type)?;
        writeln!(f, "  Files: {}", self.file_count)?;
        writeln!(
            f,
            "  Used: {} sectors ({} KB)",
            self.used_sectors,
            self.used_sectors / 2
        )?;
        writeln!(
            f,
            "  Free: {} sectors ({} KB)",
            self.free_sectors,
            self.free_sectors / 2
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_file_type_parsing() {
        assert_eq!(
            MgtFileType::from_status(0x00),
            MgtFileType::Erased
        );
        assert_eq!(
            MgtFileType::from_status(0x05),
            MgtFileType::ZxSnapshot
        );
        assert_eq!(
            MgtFileType::from_status(0x10),
            MgtFileType::SamBasic
        );
        assert_eq!(
            MgtFileType::from_status(0x13),
            MgtFileType::Code
        );
        assert_eq!(
            MgtFileType::from_status(0x14),
            MgtFileType::Screen
        );
        // With hidden flag
        assert_eq!(
            MgtFileType::from_status(0x85),
            MgtFileType::ZxSnapshot
        );
        // With protected flag
        assert_eq!(
            MgtFileType::from_status(0x53),
            MgtFileType::Code
        );
    }

    #[test]
    fn test_file_type_display() {
        assert_eq!(format!("{}", MgtFileType::ZxSnapshot), "ZX Snapshot");
        assert_eq!(format!("{}", MgtFileType::Code), "CODE");
        assert_eq!(format!("{}", MgtFileType::Screen), "SCREEN$");
    }

    #[test]
    fn test_dir_entry_parse_erased() {
        let mut data = vec![0u8; 256];
        data[0] = 0x00; // Erased

        let entry = MgtDirEntry::parse(&data, 0);
        assert!(entry.is_none());
    }

    #[test]
    fn test_dir_entry_parse_valid() {
        let mut data = vec![0u8; 256];
        data[0] = 0x13; // CODE file
        data[1..11].copy_from_slice(b"TESTFILE  ");
        data[11] = 0x00; // Sectors high byte
        data[12] = 0x10; // Sectors low byte (16 sectors)
        data[13] = 4; // Start track
        data[14] = 1; // Start sector

        let entry = MgtDirEntry::parse(&data, 5).unwrap();
        assert_eq!(entry.index, 5);
        assert_eq!(entry.file_type, MgtFileType::Code);
        assert_eq!(entry.filename, "TESTFILE");
        assert_eq!(entry.sectors_used, 16);
        assert_eq!(entry.start_track, 4);
        assert_eq!(entry.start_sector, 1);
        assert!(!entry.hidden);
        assert!(!entry.protected);
    }

    #[test]
    fn test_read_dir_extended_reports_recorded_file_size() {
        // The listing reported the allocation as both `size` and `allocated`,
        // so a 300-byte file occupying one 512-byte sector listed as 512 bytes.
        let mut data = vec![0u8; 512];
        data[0] = 0x13; // CODE
        data[1..11].copy_from_slice(b"TESTFILE  ");
        data[11..13].copy_from_slice(&1u16.to_be_bytes()); // 1 sector
        data[13] = 4; // start track
        data[14] = 1; // start sector
        // Disciple/+D record the true file length at 0xD4.
        data[212..214].copy_from_slice(&300u16.to_le_bytes());

        let image = DiskImage::builder()
            .format(crate::format::DiskImageFormat::RawMgt)
            .spec(crate::format::FormatSpec::new(2, 80, 10, 512).with_first_sector_id(1))
            .build()
            .unwrap();
        let mut image = image;
        // Zero the directory area so only our entry is present.
        for track in 0..4u8 {
            for id in 1..=10u8 {
                image.write_sector(0, track, id, &[0u8; 512]).unwrap();
            }
        }
        image.write_sector(0, 0, 1, &data).unwrap();
        // Deliberately does not write the sector the file claims to occupy.

        let fs = MgtFileSystem::new(&image).unwrap();
        let entries = fs.read_dir_extended().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].blocks, 1);
        // One 512-byte sector is allocated, but the entry records a 300-byte
        // file. Reporting the allocation as the size overstates it.
        assert_eq!(entries[0].allocated, 512);
        assert_eq!(entries[0].size, 300);
    }

    #[test]
    fn test_dir_entry_flags() {
        let mut data = vec![0u8; 256];
        data[0] = 0xD3; // Hidden + Protected + CODE
        data[1..11].copy_from_slice(b"HIDDEN    ");
        data[12] = 0x01;

        let entry = MgtDirEntry::parse(&data, 0).unwrap();
        assert!(entry.hidden);
        assert!(entry.protected);
        assert_eq!(entry.file_type, MgtFileType::Code);
    }
}
