/// TR-DOS filesystem implementation
///
/// TR-DOS is the disk operating system used with the Beta Disk Interface
/// for the ZX Spectrum.
///
/// Directory format (verified against ZEsarUX scl2trd.c):
/// - Directory entries stored in track 0, sectors 1-8
/// - Each entry is 16 bytes, up to 128 entries (8 sectors * 16 entries)
/// - Disk catalog at track 0, sector 9
///
/// Directory entry (16 bytes):
/// Bytes 0-7:   Filename (8 characters, padded with spaces)
/// Byte  8:     File type ('B'=BASIC, 'C'=CODE, 'D'=data array, '#'=print, etc.)
/// Bytes 9-10:  Data length in bytes (LE u16)
/// Bytes 11-12: Parameter 1 (LE u16) - start address for CODE, autostart for BASIC
/// Byte  13:    Sector count (number of 256-byte sectors)
/// Byte  14:    Start sector (0-based within track)
/// Byte  15:    Start track
///
/// Catalog sector (track 0, sector 9) offsets:
/// Byte 225 (0xE1): First free sector within track
/// Byte 226 (0xE2): First free track
/// Byte 227 (0xE3): Disk type (0x16=80t DS, 0x17=80t SS, 0x18=40t SS, 0x19=40t DS)
/// Byte 228 (0xE4): Number of files
/// Byte 229 (0xE5): Free sectors low byte
/// Byte 230 (0xE6): Free sectors high byte
/// Byte 231 (0xE7): TR-DOS ID (0x10)

use crate::error::{DskError, Result};
use crate::filesystem::{DirEntry, FileAttributes, FileSystem, FileSystemInfo};
use crate::image::DiskImage;

/// Sectors per track in TR-DOS format
pub const TRD_SECTORS_PER_TRACK: u8 = 16;
/// Sector size in bytes for TR-DOS format
pub const TRD_SECTOR_SIZE: usize = 256;
/// Number of directory sectors in TR-DOS (track 0, sectors 1-8)
pub const TRD_DIR_SECTORS: u8 = 8;
/// Number of directory entries per sector (256 / 16)
pub const TRD_DIR_ENTRIES_PER_SECTOR: usize = 16;
/// Maximum number of directory entries (8 sectors * 16 entries)
pub const TRD_MAX_DIR_ENTRIES: usize = TRD_DIR_SECTORS as usize * TRD_DIR_ENTRIES_PER_SECTOR;
/// Sector containing the disk catalog (track 0, sector 9)
pub const TRD_CATALOG_SECTOR: u8 = 9;
/// Size of a TR-DOS directory entry in bytes
pub const TRD_DIR_ENTRY_SIZE: usize = 16;

/// TR-DOS file type
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrdosFileType {
    /// BASIC program
    Basic,
    /// Code/binary file
    Code,
    /// Data array
    DataArray,
    /// Print file
    Print,
    /// Unknown file type
    Unknown(u8),
}

impl TrdosFileType {
    /// Parse file type from type byte
    pub fn from_byte(b: u8) -> Self {
        match b {
            b'B' => TrdosFileType::Basic,
            b'C' => TrdosFileType::Code,
            b'D' => TrdosFileType::DataArray,
            b'#' => TrdosFileType::Print,
            _ => TrdosFileType::Unknown(b),
        }
    }

    /// Convert to type byte
    pub fn to_byte(self) -> u8 {
        match self {
            TrdosFileType::Basic => b'B',
            TrdosFileType::Code => b'C',
            TrdosFileType::DataArray => b'D',
            TrdosFileType::Print => b'#',
            TrdosFileType::Unknown(b) => b,
        }
    }
}

impl std::fmt::Display for TrdosFileType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TrdosFileType::Basic => write!(f, "BASIC"),
            TrdosFileType::Code => write!(f, "CODE"),
            TrdosFileType::DataArray => write!(f, "DATA"),
            TrdosFileType::Print => write!(f, "PRINT"),
            TrdosFileType::Unknown(b) => write!(f, "Type '{}'", *b as char),
        }
    }
}

/// TR-DOS directory entry (16 bytes)
#[derive(Debug, Clone)]
pub struct TrdosDirEntry {
    /// Directory entry index
    pub index: usize,
    /// File type
    pub file_type: TrdosFileType,
    /// Filename (up to 8 characters)
    pub filename: String,
    /// Data length in bytes (LE u16 from directory entry bytes 9-10)
    pub data_length: u16,
    /// Parameter 1 (LE u16 from bytes 11-12: start addr for CODE, autostart for BASIC)
    pub param1: u16,
    /// Number of sectors occupied (byte 13)
    pub sector_count: u8,
    /// Start sector within track, 0-based (byte 14)
    pub start_sector: u8,
    /// Start track (byte 15)
    pub start_track: u8,
    /// Whether this entry is marked as deleted
    pub deleted: bool,
}

impl TrdosDirEntry {
    /// Parse a 16-byte directory entry
    pub fn parse(data: &[u8], index: usize) -> Option<Self> {
        if data.len() < TRD_DIR_ENTRY_SIZE {
            return None;
        }

        let filename_bytes = &data[0..8];
        let type_byte = data[8];

        if type_byte == 0x00 {
            return None;
        }

        let deleted = type_byte == 0x01;
        if deleted {
            let filename = String::from_utf8_lossy(filename_bytes).trim_end().to_string();
            if filename.is_empty() || filename.chars().all(|c| c == ' ' || c == '\0') {
                return None;
            }
            let file_type = TrdosFileType::from_byte(type_byte);
            return Some(Self {
                index,
                file_type,
                filename,
                data_length: 0,
                param1: 0,
                sector_count: 0,
                start_sector: 0,
                start_track: 0,
                deleted,
            });
        }

        let filename = String::from_utf8_lossy(filename_bytes).trim_end().to_string();
        if filename.is_empty() || filename.chars().all(|c| c == ' ' || c == '\0') {
            return None;
        }

        let file_type = TrdosFileType::from_byte(type_byte);
        let data_length = u16::from_le_bytes([data[9], data[10]]);
        let param1 = u16::from_le_bytes([data[11], data[12]]);
        let sector_count = data[13];
        let start_sector = data[14];
        let start_track = data[15];

        Some(Self {
            index,
            file_type,
            filename,
            data_length,
            param1,
            sector_count,
            start_sector,
            start_track,
            deleted,
        })
    }

    /// Calculate the absolute sector number from start_track and start_sector
    pub fn first_absolute_sector(&self) -> u16 {
        self.start_track as u16 * TRD_SECTORS_PER_TRACK as u16 + self.start_sector as u16
    }

    /// Get a human-readable type/metadata string
    pub fn display_type(&self) -> String {
        match self.file_type {
            TrdosFileType::Basic => {
                if self.param1 != 0 && self.param1 != 0xFFFF {
                    format!("BASIC LINE {}", self.param1)
                } else {
                    "BASIC".to_string()
                }
            }
            TrdosFileType::Code => {
                format!("CODE {},{}", self.param1, self.data_length)
            }
            TrdosFileType::DataArray => {
                format!("DATA {},{}", self.param1, self.data_length)
            }
            _ => format!("{}", self.file_type),
        }
    }
}

/// TR-DOS disk catalog (track 0, sector 9)
#[derive(Debug, Clone)]
pub struct TrdosCatalog {
    /// Number of files on disk
    pub num_files: u8,
    /// Number of free sectors
    pub free_sectors: u16,
    /// Track number of first free sector
    pub first_free_track: u8,
    /// Sector number of first free sector (0-based within track)
    pub first_free_sector: u8,
    /// Disk type (0x16=80t DS, 0x17=80t SS, 0x18=40t SS, 0x19=40t DS)
    pub disk_type: u8,
}

impl TrdosCatalog {
    /// Parse catalog from sector 9 data (256 bytes)
    pub fn parse(data: &[u8]) -> Option<Self> {
        if data.len() < 232 {
            return None;
        }

        Some(Self {
            first_free_sector: data[225],
            first_free_track: data[226],
            disk_type: data[227],
            num_files: data[228],
            free_sectors: u16::from_le_bytes([data[229], data[230]]),
        })
    }

    /// Get total number of tracks based on disk type
    pub fn total_tracks(&self) -> u8 {
        match self.disk_type {
            0x16 | 0x17 => 80,
            0x18 | 0x19 => 40,
            _ => 80,
        }
    }

    /// Whether the disk is double-sided
    pub fn is_double_sided(&self) -> bool {
        matches!(self.disk_type, 0x16 | 0x19)
    }
}

/// TR-DOS filesystem implementation
pub struct TrdosFileSystem<'a> {
    image: &'a DiskImage,
    directory: Vec<TrdosDirEntry>,
    catalog: Option<TrdosCatalog>,
}

impl<'a> TrdosFileSystem<'a> {
    /// Create a new TR-DOS filesystem from an image
    pub fn new(image: &'a DiskImage) -> Result<Self> {
        let directory = Self::read_directory(image)?;
        let catalog = Self::read_catalog(image);

        Ok(Self {
            image,
            directory,
            catalog,
        })
    }

    fn read_directory(image: &DiskImage) -> Result<Vec<TrdosDirEntry>> {
        let disk = image
            .get_disk(0)
            .ok_or_else(|| DskError::filesystem("No disk side 0"))?;

        let track = disk
            .get_track(0)
            .ok_or_else(|| DskError::filesystem("Track 0 not found"))?;

        let mut entries = Vec::new();
        let mut entry_index = 0;

        for sector_id in 1..=TRD_DIR_SECTORS {
            let sector = match track.get_sector(sector_id) {
                Some(s) => s,
                None => continue,
            };

            let data = sector.data();
            for i in 0..TRD_DIR_ENTRIES_PER_SECTOR {
                let offset = i * TRD_DIR_ENTRY_SIZE;
                if offset + TRD_DIR_ENTRY_SIZE <= data.len() {
                    if let Some(entry) = TrdosDirEntry::parse(
                        &data[offset..offset + TRD_DIR_ENTRY_SIZE],
                        entry_index,
                    ) {
                        entries.push(entry);
                    }
                }
                entry_index += 1;
            }
        }

        Ok(entries)
    }

    fn read_catalog(image: &DiskImage) -> Option<TrdosCatalog> {
        let data = image.read_sector(0, 0, TRD_CATALOG_SECTOR).ok()?;
        TrdosCatalog::parse(data)
    }

    /// Get the parsed directory entries
    pub fn directory(&self) -> &[TrdosDirEntry] {
        &self.directory
    }

    /// Get the disk catalog information
    pub fn catalog(&self) -> Option<&TrdosCatalog> {
        self.catalog.as_ref()
    }

    /// Read file data, trimmed to data_length bytes
    pub fn read_file_data(&self, entry: &TrdosDirEntry) -> Result<Vec<u8>> {
        let mut data = Vec::new();
        let sectors_to_read = entry.sector_count as usize;

        if sectors_to_read == 0 {
            return Ok(data);
        }

        let mut absolute_sector = entry.first_absolute_sector();
        let max_sector = self.image.spec.num_tracks as usize
            * self.image.spec.num_sides as usize * TRD_SECTORS_PER_TRACK as usize;

        for _ in 0..sectors_to_read {
            if absolute_sector as usize >= max_sector {
                data.extend_from_slice(&[0u8; TRD_SECTOR_SIZE]);
                absolute_sector += 1;
                continue;
            }

            let linear_track = absolute_sector / TRD_SECTORS_PER_TRACK as u16;
            let side = (linear_track / self.image.spec.num_tracks as u16) as u8;
            let track_num = (linear_track % self.image.spec.num_tracks as u16) as u8;
            let sector_id = (absolute_sector % TRD_SECTORS_PER_TRACK as u16) as u8 + 1;

            let sector_data = self.image.read_sector(side, track_num, sector_id)?;
            data.extend_from_slice(sector_data);
            absolute_sector += 1;
        }

        let actual_len = entry.data_length as usize;
        if actual_len > 0 && actual_len < data.len() {
            data.truncate(actual_len);
        }

        Ok(data)
    }

    /// Find a file by name (case-insensitive)
    pub fn find_file(&self, name: &str) -> Option<&TrdosDirEntry> {
        let name_upper = name.to_uppercase();
        self.directory
            .iter()
            .find(|e| e.filename.to_uppercase() == name_upper)
    }
}

impl<'a> FileSystem for TrdosFileSystem<'a> {
    fn from_image<'b>(_image: &'b DiskImage) -> Result<Self>
    where
        Self: Sized,
    {
        Err(DskError::filesystem(
            "Use TrdosFileSystem::new() directly",
        ))
    }

    fn from_image_mut<'b>(_image: &'b mut DiskImage) -> Result<Self>
    where
        Self: Sized,
    {
        Err(DskError::filesystem(
            "Mutable TR-DOS filesystem not yet implemented",
        ))
    }

    fn read_dir(&self) -> Result<Vec<DirEntry>> {
        let mut entries = Vec::new();

        for dir_entry in &self.directory {
            entries.push(DirEntry {
                name: dir_entry.filename.clone(),
                user: 0,
                extent: 0,
                size: dir_entry.sector_count as usize * TRD_SECTOR_SIZE,
                attributes: FileAttributes::default(),
            });
        }

        Ok(entries)
    }

    fn read_file(&self, name: &str) -> Result<Vec<u8>> {
        let entry = self
            .find_file(name)
            .ok_or_else(|| DskError::FileNotFound(name.to_string()))?;

        self.read_file_data(entry)
    }

    fn write_file(&mut self, _name: &str, _data: &[u8]) -> Result<()> {
        Err(DskError::filesystem("TR-DOS write support not yet implemented"))
    }

    fn delete_file(&mut self, _name: &str) -> Result<()> {
        Err(DskError::filesystem("TR-DOS delete support not yet implemented"))
    }

    fn info(&self) -> FileSystemInfo {
        let total_sectors = self.image.spec.num_tracks as usize
            * self.image.spec.num_sides as usize * TRD_SECTORS_PER_TRACK as usize;
        let free_sectors = self
            .catalog
            .as_ref()
            .map(|c| c.free_sectors as usize)
            .unwrap_or(0);

        FileSystemInfo {
            fs_type: "TR-DOS".to_string(),
            total_blocks: total_sectors,
            free_blocks: free_sectors,
            block_size: TRD_SECTOR_SIZE,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_file_type_from_byte() {
        assert_eq!(TrdosFileType::from_byte(b'B'), TrdosFileType::Basic);
        assert_eq!(TrdosFileType::from_byte(b'C'), TrdosFileType::Code);
        assert_eq!(TrdosFileType::from_byte(b'D'), TrdosFileType::DataArray);
        assert_eq!(TrdosFileType::from_byte(b'#'), TrdosFileType::Print);
    }

    #[test]
    fn test_file_type_display() {
        assert_eq!(format!("{}", TrdosFileType::Basic), "BASIC");
        assert_eq!(format!("{}", TrdosFileType::Code), "CODE");
        assert_eq!(format!("{}", TrdosFileType::DataArray), "DATA");
        assert_eq!(format!("{}", TrdosFileType::Print), "PRINT");
    }

    #[test]
    fn test_dir_entry_parse_basic() {
        let mut data = [0u8; 16];
        data[0..8].copy_from_slice(b"boot    ");
        data[8] = b'B';
        data[9] = 0x42;
        data[10] = 0x00;
        data[11] = 0x42;
        data[12] = 0x00;
        data[13] = 19;
        data[14] = 0;
        data[15] = 1;

        let entry = TrdosDirEntry::parse(&data, 0).unwrap();
        assert_eq!(entry.filename, "boot");
        assert_eq!(entry.file_type, TrdosFileType::Basic);
        assert_eq!(entry.data_length, 0x42);
        assert_eq!(entry.param1, 0x42);
        assert_eq!(entry.sector_count, 19);
        assert_eq!(entry.start_sector, 0);
        assert_eq!(entry.start_track, 1);
        assert_eq!(entry.first_absolute_sector(), 16);
    }

    #[test]
    fn test_dir_entry_parse_code() {
        let mut data = [0u8; 16];
        data[0..8].copy_from_slice(b"BOOT.PIX");
        data[8] = b'C';
        data[9] = 0x18;
        data[10] = 0xC4;
        data[11] = 0x00;
        data[12] = 0x18;
        data[13] = 24;
        data[14] = 8;
        data[15] = 2;

        let entry = TrdosDirEntry::parse(&data, 1).unwrap();
        assert_eq!(entry.filename, "BOOT.PIX");
        assert_eq!(entry.file_type, TrdosFileType::Code);
        assert_eq!(entry.data_length, 0xC418);
        assert_eq!(entry.param1, 0x1800);
        assert_eq!(entry.sector_count, 24);
        assert_eq!(entry.start_sector, 8);
        assert_eq!(entry.start_track, 2);
        assert_eq!(entry.first_absolute_sector(), 40);
    }

    #[test]
    fn test_dir_entry_parse_empty() {
        let data = [0u8; 16];
        let entry = TrdosDirEntry::parse(&data, 0);
        assert!(entry.is_none());
    }

    #[test]
    fn test_dir_entry_too_short() {
        let data = [0u8; 15];
        let entry = TrdosDirEntry::parse(&data, 0);
        assert!(entry.is_none());
    }

    #[test]
    fn test_catalog_parse() {
        let mut data = [0u8; 256];
        data[225] = 10;
        data[226] = 9;
        data[227] = 0x16;
        data[228] = 1;
        data[229] = 0xE6;
        data[230] = 0x01;
        data[231] = 0x10;

        let catalog = TrdosCatalog::parse(&data).unwrap();
        assert_eq!(catalog.first_free_sector, 10);
        assert_eq!(catalog.first_free_track, 9);
        assert_eq!(catalog.disk_type, 0x16);
        assert_eq!(catalog.num_files, 1);
        assert_eq!(catalog.free_sectors, 0x01E6);
    }

    #[test]
    fn test_catalog_disk_types() {
        let mut data = [0u8; 256];
        data[227] = 0x16;
        let cat = TrdosCatalog::parse(&data).unwrap();
        assert_eq!(cat.total_tracks(), 80);
        assert!(cat.is_double_sided());

        data[227] = 0x17;
        let cat = TrdosCatalog::parse(&data).unwrap();
        assert_eq!(cat.total_tracks(), 80);
        assert!(!cat.is_double_sided());

        data[227] = 0x18;
        let cat = TrdosCatalog::parse(&data).unwrap();
        assert_eq!(cat.total_tracks(), 40);
        assert!(!cat.is_double_sided());

        data[227] = 0x19;
        let cat = TrdosCatalog::parse(&data).unwrap();
        assert_eq!(cat.total_tracks(), 40);
        assert!(cat.is_double_sided());
    }

    #[test]
    fn test_first_absolute_sector() {
        let mut data = [0u8; 16];
        data[0..8].copy_from_slice(b"TEST    ");
        data[8] = b'C';
        data[14] = 5;
        data[15] = 3;

        let entry = TrdosDirEntry::parse(&data, 0).unwrap();
        assert_eq!(entry.first_absolute_sector(), 3 * 16 + 5);
    }
}
