/// Filesystem implementations

/// CP/M filesystem implementation
pub mod cpm;
/// DISCiPLE/+D filesystem implementation (ZX Spectrum)
pub mod disciple;
/// Filesystem file import and deletion
pub mod edit;
/// MGT filesystem base implementation
pub mod mgt;
/// SAM Coupe filesystem implementation
pub mod sam;
/// TR-DOS filesystem implementation (ZX Spectrum Beta Disk Interface)
pub mod trdos;

pub use cpm::CpmFileSystem;
pub use disciple::DiscipleFileSystem;
pub use edit::{delete_file, import_file};
pub use mgt::{MgtDirEntry, MgtFileSystem, MgtFileType, MgtSystemType};
pub use sam::SamFileSystem;
pub use trdos::{TrdosCatalog, TrdosDirEntry, TrdosFileSystem, TrdosFileType};

use crate::error::Result;
use crate::image::DiskImage;

/// Filesystem type for disk operations
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FileSystemType {
    /// Auto-detect filesystem based on disk specification
    #[default]
    Auto,
    /// CP/M filesystem (Amstrad CPC, Spectrum +3, PCW, etc.)
    Cpm,
    /// MGT filesystem (DISCiPLE/+D, SAM Coupe)
    Mgt,
    /// TR-DOS filesystem (ZX Spectrum Beta Disk Interface)
    Trdos,
}

impl std::fmt::Display for FileSystemType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FileSystemType::Auto => write!(f, "Auto"),
            FileSystemType::Cpm => write!(f, "CP/M"),
            FileSystemType::Mgt => write!(f, "MGT"),
            FileSystemType::Trdos => write!(f, "TR-DOS"),
        }
    }
}

impl FileSystemType {
    /// Parse from string (case-insensitive)
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "auto" => Some(FileSystemType::Auto),
            "cpm" | "cp/m" => Some(FileSystemType::Cpm),
            "mgt" | "disciple" | "sam" => Some(FileSystemType::Mgt),
            "trdos" | "tr-dos" | "trd" => Some(FileSystemType::Trdos),
            _ => None,
        }
    }
}

/// File attributes
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileAttributes {
    /// Read-only flag
    pub read_only: bool,
    /// System file flag
    pub system: bool,
    /// Archive flag
    pub archive: bool,
}

impl Default for FileAttributes {
    fn default() -> Self {
        Self {
            read_only: false,
            system: false,
            archive: false,
        }
    }
}

/// File header type
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HeaderType {
    /// No recognized header
    None,
    /// AMSDOS header (Amstrad CPC)
    Amsdos,
    /// PLUS3DOS header (Spectrum +3)
    Plus3dos,
}

impl std::fmt::Display for HeaderType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HeaderType::None => write!(f, ""),
            HeaderType::Amsdos => write!(f, "AMSDOS"),
            HeaderType::Plus3dos => write!(f, "PLUS3DOS"),
        }
    }
}

/// Parsed file header information
#[derive(Debug, Clone)]
pub struct FileHeader {
    /// Header type
    pub header_type: HeaderType,
    /// Whether checksum is valid
    pub checksum_valid: bool,
    /// Actual file size from header
    pub file_size: usize,
    /// Header size in bytes (typically 128)
    pub header_size: usize,
    /// Metadata description (e.g., "BASIC", "BINARY 0x1234 EXEC 0x1234")
    pub meta: String,
}

impl Default for FileHeader {
    fn default() -> Self {
        Self {
            header_type: HeaderType::None,
            checksum_valid: false,
            file_size: 0,
            header_size: 0,
            meta: String::new(),
        }
    }
}

/// Directory entry
#[derive(Debug, Clone)]
pub struct DirEntry {
    /// Filename (8.3 format, e.g., "FILENAME.TXT")
    pub name: String,
    /// User number (0-15)
    pub user: u8,
    /// Extent number
    pub extent: u8,
    /// File size in bytes
    pub size: usize,
    /// File attributes
    pub attributes: FileAttributes,
}

/// Extended directory entry with additional information
#[derive(Debug, Clone)]
pub struct ExtendedDirEntry {
    /// Filename (8.3 format, e.g., "FILENAME.TXT")
    pub name: String,
    /// User number (0-15)
    pub user: u8,
    /// Directory entry index
    pub index: usize,
    /// Number of allocation blocks
    pub blocks: usize,
    /// Allocated size in bytes (blocks * block_size)
    pub allocated: usize,
    /// File size in bytes (from record count)
    pub size: usize,
    /// File attributes
    pub attributes: FileAttributes,
    /// Parsed header information
    pub header: FileHeader,
}

/// Filesystem information
#[derive(Debug)]
pub struct FileSystemInfo {
    /// Filesystem type name
    pub fs_type: String,
    /// Total blocks on disk
    pub total_blocks: usize,
    /// Free blocks
    pub free_blocks: usize,
    /// Block size in bytes
    pub block_size: usize,
}

/// Filesystem trait for accessing files on DSK images
pub trait FileSystem {
    /// Attempt to mount a filesystem from a DSK image (read-only)
    fn from_image<'a>(image: &'a DiskImage) -> Result<Self> where Self: Sized;

    /// Attempt to mount a filesystem from a DSK image (read-write)
    fn from_image_mut<'a>(image: &'a mut DiskImage) -> Result<Self> where Self: Sized;

    /// List directory entries
    fn read_dir(&self) -> Result<Vec<DirEntry>>;

    /// Read a file's contents
    fn read_file(&self, name: &str) -> Result<Vec<u8>>;

    /// Write a file (requires mutable filesystem)
    fn write_file(&mut self, name: &str, data: &[u8]) -> Result<()>;

    /// Delete a file (requires mutable filesystem)
    fn delete_file(&mut self, name: &str) -> Result<()>;

    /// Get filesystem information
    fn info(&self) -> FileSystemInfo;
}

/// Try to parse an AMSDOS header from data
pub fn try_amsdos_header(data: &[u8]) -> Option<FileHeader> {
    if data.len() < 128 {
        return None;
    }

    // Calculate checksum of bytes 0-66
    let calc_checksum: u16 = data[0..=66].iter().map(|&b| b as u16).sum();
    let stored_checksum = u16::from_le_bytes([data[67], data[68]]);

    if calc_checksum != stored_checksum {
        return None;
    }

    // Valid AMSDOS header
    let file_size = data[64] as usize
        | ((data[65] as usize) << 8)
        | ((data[66] as usize) << 16);

    let load_addr = u16::from_le_bytes([data[21], data[22]]);
    let exec_addr = u16::from_le_bytes([data[26], data[27]]);

    let meta = match data[18] {
        0 => "BASIC".to_string(),
        1 => "BASIC (protected)".to_string(),
        2 => format!("BINARY {} EXEC {}", load_addr, exec_addr),
        3 => format!("BINARY (protected) {} EXEC {}", load_addr, exec_addr),
        4 => "SCREEN".to_string(),
        5 => "SCREEN (protected)".to_string(),
        6 => "ASCII".to_string(),
        7 => "ASCII (protected)".to_string(),
        other => format!("Custom 0x{:02X}", other),
    };

    Some(FileHeader {
        header_type: HeaderType::Amsdos,
        checksum_valid: true,
        file_size,
        header_size: 128,
        meta,
    })
}

/// Try to parse a PLUS3DOS header from data
pub fn try_plus3dos_header(data: &[u8]) -> Option<FileHeader> {
    if data.len() < 128 {
        return None;
    }

    // Check signature
    if &data[0..8] != b"PLUS3DOS" {
        return None;
    }

    // Calculate checksum of bytes 0-126
    let calc_checksum: u8 = data[0..=126].iter().fold(0u8, |acc, &b| acc.wrapping_add(b));
    let stored_checksum = data[127];
    let checksum_valid = calc_checksum == stored_checksum;

    let file_size = data[11] as usize
        | ((data[12] as usize) << 8)
        | ((data[13] as usize) << 16)
        | ((data[14] as usize) << 24);

    let param1 = u16::from_le_bytes([data[18], data[19]]);
    let param2 = u16::from_le_bytes([data[16], data[17]]);

    let meta = match data[15] {
        0 => {
            if param1 != 0x8000 {
                format!("BASIC LINE {}", param1)
            } else {
                "BASIC".to_string()
            }
        }
        1 => {
            // DATA array - variable name is at data[19]
            let var_char = if data[19] >= 64 {
                (data[19] - 64) as char
            } else {
                '?'
            };
            // Array dimensions would be at data[129..131] but we may not have that data
            format!("DATA {}()", var_char)
        }
        2 => {
            // DATA string array
            let var_char = if data[19] >= 128 {
                (data[19] - 128) as char
            } else {
                '?'
            };
            format!("DATA {}$()", var_char)
        }
        3 => format!("CODE {},{}", param1, param2),
        other => format!("Custom 0x{:02X}", other),
    };

    Some(FileHeader {
        header_type: HeaderType::Plus3dos,
        checksum_valid,
        file_size,
        header_size: 128,
        meta,
    })
}

/// Try to parse any recognized header from data
pub fn try_parse_header(data: &[u8]) -> FileHeader {
    // Try PLUS3DOS first (has explicit signature)
    if let Some(header) = try_plus3dos_header(data) {
        return header;
    }

    // Try AMSDOS (checksum-based detection)
    if let Some(header) = try_amsdos_header(data) {
        return header;
    }

    FileHeader::default()
}
