//! DISCiPLE/+D filesystem implementation
//!
//! ZX Spectrum specific filesystem using MGT format.
//! Used by:
//! - Miles Gordon Technology DISCiPLE
//! - MGT +D
//!
//! Directory entry extensions at offset 0xD2 (210):
//! - 10 bytes for Disciple/+D metadata

use crate::error::Result;
use crate::filesystem::mgt::{MgtDirEntry, MgtFileSystem, MgtFileType};
use crate::filesystem::{ExtendedDirEntry, FileHeader, HeaderType};
use crate::image::DiskImage;

/// Disciple/+D specific file metadata
#[derive(Debug, Clone)]
pub struct DiscipleHeader {
    /// File type description
    pub file_type: DiscipleFileType,
    /// Load address (for CODE files)
    pub load_address: u16,
    /// Length in bytes
    pub length: u16,
    /// Start address/line (for BASIC/CODE)
    pub start: u16,
}

/// Disciple/+D file types
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiscipleFileType {
    /// BASIC program
    Basic,
    /// Numeric array
    NumericArray,
    /// String array
    StringArray,
    /// CODE (binary)
    Code,
    /// 48K Snapshot
    Snapshot48k,
    /// Microdrive file
    Microdrive,
    /// SCREEN$
    Screen,
    /// Special file
    Special,
    /// 128K Snapshot
    Snapshot128k,
    /// Opentype file
    Opentype,
    /// Execute file
    Execute,
    /// Unknown type
    Unknown(u8),
}

impl DiscipleFileType {
    /// Parse from MGT file type
    pub fn from_mgt_type(mgt_type: &MgtFileType, raw_data: &[u8]) -> Self {
        match mgt_type {
            MgtFileType::ZxSnapshot => {
                // Check if it's 48K or 128K based on size or header
                if raw_data.len() >= 215 && raw_data[214] != 0 {
                    DiscipleFileType::Snapshot128k
                } else {
                    DiscipleFileType::Snapshot48k
                }
            }
            MgtFileType::Code => DiscipleFileType::Code,
            MgtFileType::Screen => DiscipleFileType::Screen,
            MgtFileType::SamBasic => DiscipleFileType::Basic,
            MgtFileType::NumericArray => DiscipleFileType::NumericArray,
            MgtFileType::StringArray => DiscipleFileType::StringArray,
            MgtFileType::Other(code) => match code {
                1 => DiscipleFileType::Basic,
                2 => DiscipleFileType::NumericArray,
                3 => DiscipleFileType::StringArray,
                4 => DiscipleFileType::Code,
                5 => DiscipleFileType::Snapshot48k,
                6 => DiscipleFileType::Microdrive,
                7 => DiscipleFileType::Screen,
                8 => DiscipleFileType::Special,
                9 => DiscipleFileType::Snapshot128k,
                10 => DiscipleFileType::Opentype,
                11 => DiscipleFileType::Execute,
                _ => DiscipleFileType::Unknown(*code),
            },
            _ => DiscipleFileType::Unknown(mgt_type.type_code()),
        }
    }
}

impl std::fmt::Display for DiscipleFileType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DiscipleFileType::Basic => write!(f, "BASIC"),
            DiscipleFileType::NumericArray => write!(f, "Number Array"),
            DiscipleFileType::StringArray => write!(f, "String Array"),
            DiscipleFileType::Code => write!(f, "CODE"),
            DiscipleFileType::Snapshot48k => write!(f, "48K Snapshot"),
            DiscipleFileType::Microdrive => write!(f, "Microdrive"),
            DiscipleFileType::Screen => write!(f, "SCREEN$"),
            DiscipleFileType::Special => write!(f, "Special"),
            DiscipleFileType::Snapshot128k => write!(f, "128K Snapshot"),
            DiscipleFileType::Opentype => write!(f, "Opentype"),
            DiscipleFileType::Execute => write!(f, "Execute"),
            DiscipleFileType::Unknown(code) => write!(f, "Type {}", code),
        }
    }
}

/// DISCiPLE/+D filesystem
pub struct DiscipleFileSystem<'a> {
    mgt: MgtFileSystem<'a>,
}

impl<'a> DiscipleFileSystem<'a> {
    /// Create a new Disciple filesystem from an image
    pub fn new(image: &'a DiskImage) -> Result<Self> {
        let mgt = MgtFileSystem::new(image)?;
        Ok(Self { mgt })
    }

    /// Get the underlying MGT filesystem
    pub fn mgt(&self) -> &MgtFileSystem<'a> {
        &self.mgt
    }

    /// Get file size from Disciple directory entry (offsets 212-213)
    fn get_file_size(&self, entry: &MgtDirEntry) -> usize {
        let raw = &entry.raw_data;
        if raw.len() >= 214 {
            // File length at offset 212-213 ($d4-$d5), little endian
            u16::from_le_bytes([raw[212], raw[213]]) as usize
        } else {
            // Fallback to allocated size if header not available
            entry.file_size()
        }
    }

    /// Read directory with Disciple-specific information
    pub fn read_dir_extended(&self) -> Result<Vec<ExtendedDirEntry>> {
        self.mgt.read_dir_with(|entry| {
            (self.parse_disciple_header(entry), self.get_file_size(entry))
        })
    }

    /// Parse Disciple-specific metadata from directory entry
    fn parse_disciple_header(&self, entry: &MgtDirEntry) -> FileHeader {
        let raw = &entry.raw_data;
        
        // Determine file type from tape header ID at offset 211 ($d3) if available
        let file_type = if raw.len() >= 212 {
            let tape_header_id = raw[211];
            match tape_header_id {
                0 => DiscipleFileType::Basic,
                1 => DiscipleFileType::NumericArray,
                2 => DiscipleFileType::StringArray,
                3 => DiscipleFileType::Code,
                _ => DiscipleFileType::from_mgt_type(&entry.file_type, raw),
            }
        } else {
            DiscipleFileType::from_mgt_type(&entry.file_type, raw)
        };

        // Extract header info from offset 0xD2 (210)
        let meta = if raw.len() >= 220 {
            match file_type {
                DiscipleFileType::Code | DiscipleFileType::Screen => {
                    // Start address at offset 214-215 ($d6-$d7), little endian
                    let start_addr = if raw.len() >= 216 {
                        u16::from_le_bytes([raw[214], raw[215]])
                    } else {
                        0
                    };
                    // File length at offset 212-213 ($d4-$d5), little endian
                    let length = if raw.len() >= 214 {
                        u16::from_le_bytes([raw[212], raw[213]])
                    } else {
                        0
                    };
                    format!("{} {},{}", file_type, start_addr, length)
                }
                DiscipleFileType::Basic => {
                    // Autostart line/address at offset 218-219 ($da-$db), little endian
                    let auto_line = if raw.len() >= 220 {
                        u16::from_le_bytes([raw[218], raw[219]])
                    } else {
                        0
                    };
                    if auto_line > 0 && auto_line < 10000 {
                        format!("{} LINE {}", file_type, auto_line)
                    } else {
                        format!("{}", file_type)
                    }
                }
                DiscipleFileType::Snapshot48k | DiscipleFileType::Snapshot128k => {
                    format!("{}", file_type)
                }
                _ => format!("{}", file_type),
            }
        } else {
            format!("{}", file_type)
        };

        // File size from offset 212-213 ($d4-$d5)
        let file_size = self.get_file_size(entry);

        FileHeader {
            header_type: HeaderType::None, // Disciple stores metadata in directory entry, not headers in file data
            checksum_valid: true,
            file_size,
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
            "DISCiPLE/+D Filesystem\n  Files: {}\n  Used: {} KB\n  Free: {} KB",
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
    fn read_dir_extended_reports_disciple_metadata_per_entry() {
        // The DISCIPLE listing decodes its own header and its 0xD4 length; the
        // shared loop must still surface both per entry.
        use crate::filesystem::{format_filesystem, import_file, FileSystemType};
        use crate::format::{DiskImageFormat, FormatSpec};

        let mut image = DiskImage::builder()
            .format(DiskImageFormat::RawMgt)
            .spec(FormatSpec::new(2, 80, 10, 512).with_first_sector_id(1).with_filler_byte(0))
            .build()
            .unwrap();
        format_filesystem(&mut image, FileSystemType::Mgt).unwrap();
        import_file(&mut image, FileSystemType::Mgt, "PROG", &[0x41; 700]).unwrap();

        let fs = DiscipleFileSystem::new(&image).unwrap();
        let entries = fs.read_dir_extended().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "PROG");
        assert_eq!(entries[0].allocated, 1024, "two sectors allocated");
        assert!(!entries[0].header.meta.is_empty());
    }

    #[test]
    fn test_disciple_file_type_display() {
        assert_eq!(format!("{}", DiscipleFileType::Basic), "BASIC");
        assert_eq!(format!("{}", DiscipleFileType::Code), "CODE");
        assert_eq!(format!("{}", DiscipleFileType::Snapshot48k), "48K Snapshot");
        assert_eq!(format!("{}", DiscipleFileType::Screen), "SCREEN$");
    }
}
