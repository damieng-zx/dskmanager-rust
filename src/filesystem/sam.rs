//! SAM Coupe filesystem implementation
//!
//! SAM Coupe specific filesystem using MGT format.
//!
//! Directory entry extensions at offset 0xDC (220):
//! - 33 bytes for SAM-specific metadata

use crate::error::Result;
use crate::filesystem::mgt::{MgtDirEntry, MgtFileSystem, MgtFileType};
use crate::filesystem::{ExtendedDirEntry, FileHeader, HeaderType};
use crate::image::DiskImage;

/// SAM Coupe file types
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SamFileType {
    /// SAM BASIC program (0x10)
    Basic,
    /// Numeric array (0x11)
    NumericArray,
    /// String array (0x12)
    StringArray,
    /// CODE/binary (0x13)
    Code,
    /// SCREEN$ (0x14)
    Screen,
    /// Directory (0x15)
    Directory,
    /// Unknown type
    Unknown(u8),
}

impl SamFileType {
    /// Parse from MGT file type
    pub fn from_mgt_type(mgt_type: &MgtFileType) -> Self {
        match mgt_type {
            MgtFileType::SamBasic => SamFileType::Basic,
            MgtFileType::NumericArray => SamFileType::NumericArray,
            MgtFileType::StringArray => SamFileType::StringArray,
            MgtFileType::Code => SamFileType::Code,
            MgtFileType::Screen => SamFileType::Screen,
            MgtFileType::Other(0x15) => SamFileType::Directory,
            MgtFileType::Other(code) => SamFileType::Unknown(*code),
            _ => SamFileType::Unknown(mgt_type.type_code()),
        }
    }
}

impl std::fmt::Display for SamFileType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SamFileType::Basic => write!(f, "SAM BASIC"),
            SamFileType::NumericArray => write!(f, "Numeric Array"),
            SamFileType::StringArray => write!(f, "String Array"),
            SamFileType::Code => write!(f, "CODE"),
            SamFileType::Screen => write!(f, "SCREEN$"),
            SamFileType::Directory => write!(f, "Directory"),
            SamFileType::Unknown(code) => write!(f, "Type 0x{:02X}", code),
        }
    }
}

/// SAM-specific metadata extracted from directory entry
#[derive(Debug, Clone)]
pub struct SamHeader {
    /// File type
    pub file_type: SamFileType,
    /// Start address/page (for CODE)
    pub start_address: u32,
    /// Length in bytes
    pub length: u32,
    /// Execute address (for CODE)
    pub execute_address: u32,
    /// Auto-start line (for BASIC)
    pub auto_line: u16,
}

/// SAM Coupe filesystem
pub struct SamFileSystem<'a> {
    mgt: MgtFileSystem<'a>,
}

impl<'a> SamFileSystem<'a> {
    /// Create a new SAM filesystem from an image
    pub fn new(image: &'a DiskImage) -> Result<Self> {
        let mgt = MgtFileSystem::new(image)?;
        Ok(Self { mgt })
    }

    /// Get the underlying MGT filesystem
    pub fn mgt(&self) -> &MgtFileSystem<'a> {
        &self.mgt
    }

    /// Read directory with SAM-specific information
    pub fn read_dir_extended(&self) -> Result<Vec<ExtendedDirEntry>> {
        self.mgt.read_dir_with(|entry| {
            // SAM entries carry no Disciple length field; the allocation is the
            // best available size.
            (self.parse_sam_header(entry), entry.file_size())
        })
    }

    /// Parse SAM-specific metadata from directory entry
    fn parse_sam_header(&self, entry: &MgtDirEntry) -> FileHeader {
        let raw = &entry.raw_data;
        let file_type = SamFileType::from_mgt_type(&entry.file_type);

        // SAM metadata starts at offset 0xDC (220)
        let meta = if raw.len() >= 253 {
            let sam_offset = 220;

            match file_type {
                SamFileType::Code => {
                    // Start page at offset 220
                    let start_page = raw[sam_offset];
                    // Start offset at 221-222 (little endian)
                    let start_offset = u16::from_le_bytes([raw[sam_offset + 1], raw[sam_offset + 2]]);
                    // Length modulo 16384 at 223-224
                    let length_mod = u16::from_le_bytes([raw[sam_offset + 3], raw[sam_offset + 4]]);
                    // Length pages at 225
                    let length_pages = raw[sam_offset + 5];
                    // Execute page at 226
                    let exec_page = raw[sam_offset + 6];
                    // Execute offset at 227-228
                    let exec_offset = u16::from_le_bytes([raw[sam_offset + 7], raw[sam_offset + 8]]);

                    let start_addr = (start_page as u32) * 16384 + start_offset as u32;
                    let length = (length_pages as u32) * 16384 + length_mod as u32;
                    let exec_addr = (exec_page as u32) * 16384 + exec_offset as u32;

                    if exec_addr != start_addr {
                        format!("{} {},{}  EXEC {}", file_type, start_addr, length, exec_addr)
                    } else {
                        format!("{} {},{}", file_type, start_addr, length)
                    }
                }
                SamFileType::Screen => {
                    // Screen mode at offset 220
                    let mode = raw[sam_offset];
                    let mode_name = match mode {
                        1 => "Mode 1",
                        2 => "Mode 2",
                        3 => "Mode 3",
                        4 => "Mode 4",
                        _ => "Screen",
                    };
                    format!("{} ({})", file_type, mode_name)
                }
                SamFileType::Basic => {
                    // Auto-start line at offset 232-233
                    let auto_line = if raw.len() >= sam_offset + 14 {
                        u16::from_le_bytes([raw[sam_offset + 12], raw[sam_offset + 13]])
                    } else {
                        0
                    };
                    if auto_line > 0 && auto_line < 10000 {
                        format!("{} LINE {}", file_type, auto_line)
                    } else {
                        format!("{}", file_type)
                    }
                }
                SamFileType::NumericArray | SamFileType::StringArray => {
                    // Array name at offset 220
                    let var_name = if raw[sam_offset] >= b'A' && raw[sam_offset] <= b'Z' {
                        raw[sam_offset] as char
                    } else {
                        '?'
                    };
                    if matches!(file_type, SamFileType::StringArray) {
                        format!("{} {}$()", file_type, var_name)
                    } else {
                        format!("{} {}()", file_type, var_name)
                    }
                }
                _ => format!("{}", file_type),
            }
        } else {
            format!("{}", file_type)
        };

        FileHeader {
            header_type: HeaderType::None,
            checksum_valid: true,
            file_size: entry.file_size(),
            header_size: 0,
            meta,
        }
    }

    /// Read a file by name
    /// Returns file data truncated to actual file length (not allocated size)
    pub fn read_file(&self, name: &str) -> Result<Vec<u8>> {
        let entry = self
            .mgt
            .find_file(name)
            .ok_or_else(|| crate::error::DskError::FileNotFound(name.to_string()))?;
        self.mgt.read_file(entry)
    }

    /// List all files
    pub fn list_files(&self) -> Vec<&MgtDirEntry> {
        self.mgt.directory().iter().collect()
    }

    /// Get filesystem info
    pub fn info(&self) -> String {
        let mgt_info = self.mgt.info();
        format!(
            "SAM Coupe Filesystem\n  Files: {}\n  Used: {} KB\n  Free: {} KB",
            mgt_info.file_count,
            mgt_info.used_sectors / 2,
            mgt_info.free_sectors / 2
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_dir_extended_reports_sam_metadata_per_entry() {
        // SAM listings decode their own header and use the allocation as size.
        use crate::filesystem::{format_filesystem, import_file, FileSystemType};
        use crate::format::{DiskImageFormat, FormatSpec};

        let mut image = DiskImage::builder()
            .format(DiskImageFormat::RawMgt)
            .spec(FormatSpec::new(2, 80, 10, 512).with_first_sector_id(1).with_filler_byte(0))
            .build()
            .unwrap();
        format_filesystem(&mut image, FileSystemType::Mgt).unwrap();
        import_file(&mut image, FileSystemType::Mgt, "PROG", &[0x41; 700]).unwrap();

        let fs = SamFileSystem::new(&image).unwrap();
        let entries = fs.read_dir_extended().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "PROG");
        assert_eq!(entries[0].size, 1024);
        assert!(!entries[0].header.meta.is_empty());
    }

    #[test]
    fn test_sam_file_type_display() {
        assert_eq!(format!("{}", SamFileType::Basic), "SAM BASIC");
        assert_eq!(format!("{}", SamFileType::Code), "CODE");
        assert_eq!(format!("{}", SamFileType::Screen), "SCREEN$");
        assert_eq!(format!("{}", SamFileType::Directory), "Directory");
    }

    #[test]
    fn test_sam_file_type_from_mgt() {
        assert_eq!(
            SamFileType::from_mgt_type(&MgtFileType::SamBasic),
            SamFileType::Basic
        );
        assert_eq!(
            SamFileType::from_mgt_type(&MgtFileType::Code),
            SamFileType::Code
        );
        assert_eq!(
            SamFileType::from_mgt_type(&MgtFileType::Screen),
            SamFileType::Screen
        );
    }
}
