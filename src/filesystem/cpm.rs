/// CP/M filesystem implementation

use crate::error::{DskError, Result};
use crate::filesystem::{
    try_parse_header, DirEntry, ExtendedDirEntry, FileAttributes, FileHeader, FileSystem,
    FileSystemInfo, HeaderType,
};
use crate::format::{AllocationSize, DiskSpecSide, DiskSpecification};
use crate::image::DiskImage;
use std::collections::HashMap;

/// CP/M directory entry (32 bytes)
#[derive(Debug, Clone)]
struct CpmDirEntry {
    /// Directory entry index (position in directory)
    index: usize,
    user: u8,
    filename: [u8; 8],
    extension: [u8; 3],
    extent_low: u8,
    #[allow(dead_code)]
    bytes_in_last_record: u8,
    extent_high: u8,
    record_count: u8,
    allocation: Vec<u8>,
}

impl CpmDirEntry {
    /// Parse a directory entry from 32 bytes (excludes deleted entries and volume labels)
    fn parse(data: &[u8], index: usize) -> Option<Self> {
        Self::parse_internal(data, index, false)
    }

    /// Parse a directory entry from 32 bytes, optionally including deleted entries
    fn parse_internal(data: &[u8], index: usize, include_deleted: bool) -> Option<Self> {
        if data.len() < 32 {
            return None;
        }

        let user = data[0];

        // Skip volume label entries (0x20)
        if user == 0x20 {
            return None;
        }

        // Skip deleted entries (0xE5) unless explicitly requested
        if !include_deleted && user == 0xE5 {
            return None;
        }

        // Valid user numbers are 0-31 (0x00-0x1F) for normal files, or 0xE5 for deleted files
        // Some extended CP/M implementations support user numbers up to 31
        // We allow 0-31 and 0xE5 (when include_deleted is true)
        if user > 0x1F && user != 0xE5 {
            return None;
        }

        let mut filename = [0u8; 8];
        let mut extension = [0u8; 3];
        filename.copy_from_slice(&data[1..9]);
        extension.copy_from_slice(&data[9..12]);

        let extent_low = data[12];
        let bytes_in_last_record = data[13];
        let extent_high = data[14];
        let record_count = data[15];

        let allocation = data[16..32].to_vec();

        Some(Self {
            index,
            user,
            filename,
            extension,
            extent_low,
            bytes_in_last_record,
            extent_high,
            record_count,
            allocation,
        })
    }

    /// Parse a directory entry including deleted entries (user 0xE5)
    fn parse_with_deleted(data: &[u8], index: usize) -> Option<Self> {
        Self::parse_internal(data, index, true)
    }

    /// Check if this entry is deleted (user == 0xE5)
    #[allow(dead_code)]
    fn is_deleted(&self) -> bool {
        self.user == 0xE5
    }

    /// Get the full filename as a string (strips high bits used for attributes)
    fn filename_str(&self) -> String {
        // Strip high bits (attribute flags) from filename and extension
        let clean_filename: Vec<u8> = self.filename.iter().map(|&b| b & 0x7F).collect();
        let clean_extension: Vec<u8> = self.extension.iter().map(|&b| b & 0x7F).collect();

        let name = String::from_utf8_lossy(&clean_filename)
            .trim_end()
            .replace('\0', " ")
            .trim()
            .to_string();
        let ext = String::from_utf8_lossy(&clean_extension)
            .trim_end()
            .replace('\0', " ")
            .trim()
            .to_string();

        if ext.is_empty() {
            name
        } else {
            format!("{}.{}", name, ext)
        }
    }

    /// Check if read-only attribute is set (high bit of first extension byte)
    fn is_read_only(&self) -> bool {
        (self.extension[0] & 0x80) != 0
    }

    /// Check if system attribute is set (high bit of second extension byte)
    fn is_system(&self) -> bool {
        (self.extension[1] & 0x80) != 0
    }

    /// Check if archive attribute is set (high bit of third extension byte)
    fn is_archive(&self) -> bool {
        (self.extension[2] & 0x80) != 0
    }

    /// Get file attributes
    fn attributes(&self) -> FileAttributes {
        FileAttributes {
            read_only: self.is_read_only(),
            system: self.is_system(),
            archive: self.is_archive(),
        }
    }

    /// Get the extent number (combines low and high bytes)
    fn extent_number(&self) -> u16 {
        ((self.extent_high as u16) << 5) | ((self.extent_low as u16) & 0x1F)
    }

    /// Calculate the size from this extent's record count
    fn extent_size(&self, bytes_in_last: u8) -> usize {
        let records = self.record_count as usize;
        if records == 0 {
            return 0;
        }

        if bytes_in_last > 0 {
            // Last record is partial
            (records - 1) * 128 + bytes_in_last as usize
        } else {
            records * 128
        }
    }

    /// Extract allocation blocks from this directory entry
    fn extract_blocks_for_validation(&self, spec: &DiskSpecification) -> Vec<u16> {
        let mut blocks = Vec::new();

        // Use allocation size from specification (8-bit or 16-bit block numbers)
        match spec.allocation_size {
            AllocationSize::Byte => {
                // 8-bit allocation numbers (16 entries)
                for &block in &self.allocation {
                    if block != 0 {
                        blocks.push(block as u16);
                    }
                }
            }
            AllocationSize::Word => {
                // 16-bit allocation numbers (8 entries)
                for i in (0..self.allocation.len()).step_by(2) {
                    if i + 1 < self.allocation.len() {
                        let block =
                            u16::from_le_bytes([self.allocation[i], self.allocation[i + 1]]);
                        if block != 0 {
                            blocks.push(block);
                        }
                    }
                }
            }
        }

        blocks
    }

    /// Validate that this entry has valid block allocations
    /// Returns true if the entry is valid, false if it should be pruned
    fn is_valid(&self, spec: &DiskSpecification) -> bool {
        let blocks = self.extract_blocks_for_validation(spec);
        let max_block = spec.block_count();

        // If max_block is 0, the spec is invalid - skip validation
        if max_block == 0 {
            return true;
        }

        // Check that all block numbers are within valid range
        // Block numbers should be 1-based (0 is unused/invalid)
        for &block_num in &blocks {
            if block_num == 0 || block_num > max_block {
                return false;
            }
        }

        // Check that we're not allocating more blocks than exist on the disk
        // This is a sanity check - a single file shouldn't allocate all blocks
        // (though technically possible, it's highly suspicious)
        if blocks.len() as u16 > max_block {
            return false;
        }

        true
    }
}

/// CP/M filesystem implementation using disk specification
pub struct CpmFileSystem<'a> {
    image: &'a DiskImage,
    spec: DiskSpecification,
    directory_entries: Vec<CpmDirEntry>,
}

impl<'a> CpmFileSystem<'a> {
    /// Create a new CP/M filesystem from an image using a detected specification
    pub fn new(image: &'a DiskImage, spec: DiskSpecification) -> Result<Self> {
        let directory_entries = Self::read_directory(image, &spec)?;

        Ok(Self {
            image,
            spec,
            directory_entries,
        })
    }

    /// Read the directory entries from the disk
    fn read_directory(image: &DiskImage, spec: &DiskSpecification) -> Result<Vec<CpmDirEntry>> {
        Self::read_directory_internal(image, spec, false)
    }

    /// Read the directory entries from the disk, optionally including deleted entries
    fn read_directory_internal(
        image: &DiskImage,
        spec: &DiskSpecification,
        include_deleted: bool,
    ) -> Result<Vec<CpmDirEntry>> {
        let mut entries = Vec::new();

        // Calculate max directory entries from specification
        let max_entries = spec.directory_entries();

        // Read directory data starting from the first sector after reserved tracks
        let dir_data = Self::read_directory_data(image, spec, max_entries)?;

        // Parse directory entries (32 bytes each)
        for (index, chunk) in dir_data.chunks(32).enumerate() {
            if chunk.len() < 32 {
                break;
            }
            let entry = if include_deleted {
                CpmDirEntry::parse_with_deleted(chunk, index)
            } else {
                CpmDirEntry::parse(chunk, index)
            };
            if let Some(entry) = entry {
                // Validate entry and prune invalid ones
                if entry.is_valid(spec) {
                    entries.push(entry);
                }
            }
        }

        Ok(entries)
    }

    /// Read raw directory data from disk
    fn read_directory_data(
        image: &DiskImage,
        spec: &DiskSpecification,
        max_entries: usize,
    ) -> Result<Vec<u8>> {
        let dir_size_bytes = max_entries * 32;
        let mut dir_data = Vec::with_capacity(dir_size_bytes);

        // Directory starts at the first sector of the first track after reserved tracks
        let start_track = spec.reserved_tracks;

        let total_tracks = spec.tracks_per_side as usize * spec.side_count() as usize;
        let first_sector = start_track as usize * spec.sectors_per_track as usize;
        let last_sector = total_tracks * spec.sectors_per_track as usize;
        for absolute_sector in first_sector..last_sector {
            if let Some(data) = Self::logical_sector(image, spec, absolute_sector) {
                let to_copy = (dir_size_bytes - dir_data.len()).min(data.len());
                dir_data.extend_from_slice(&data[..to_copy]);
                if dir_data.len() >= dir_size_bytes {
                    break;
                }
            }
        }

        Ok(dir_data)
    }

    /// Map a CP/M logical sector onto its physical side, track, and sector ID.
    fn logical_sector<'b>(image: &'b DiskImage, spec: &DiskSpecification, absolute: usize) -> Option<&'b [u8]> {
        let sectors_per_track = spec.sectors_per_track as usize;
        let tracks_per_side = spec.tracks_per_side as usize;
        if sectors_per_track == 0 || tracks_per_side == 0 {
            return None;
        }
        let logical_track = absolute / sectors_per_track;
        let sector_index = absolute % sectors_per_track;
        let (side, track) = match spec.side {
            DiskSpecSide::Single => (0, logical_track),
            DiskSpecSide::DoubleAlternate => (logical_track % 2, logical_track / 2),
            DiskSpecSide::DoubleSuccessive => (logical_track / tracks_per_side, logical_track % tracks_per_side),
            DiskSpecSide::DoubleReverse => {
                let side = logical_track / tracks_per_side;
                let track = logical_track % tracks_per_side;
                (side, if side == 1 { tracks_per_side - 1 - track } else { track })
            }
            DiskSpecSide::Invalid => return None,
        };
        let physical_track = image.get_disk(side as u8)?.get_track(track as u8)?;
        let mut sectors: Vec<_> = physical_track.sectors().iter().collect();
        sectors.sort_by_key(|sector| sector.id.sector);
        Some(sectors.get(sector_index)?.data())
    }

    /// Merge extents for files that span multiple directory entries
    fn merge_extents(&self) -> HashMap<(u8, String), Vec<&CpmDirEntry>> {
        Self::merge_extents_from_entries(&self.directory_entries)
    }

    /// Merge extents from a slice of directory entries
    fn merge_extents_from_entries(entries: &[CpmDirEntry]) -> HashMap<(u8, String), Vec<&CpmDirEntry>> {
        let mut files: HashMap<(u8, String), Vec<&CpmDirEntry>> = HashMap::new();

        for entry in entries {
            let filename = entry.filename_str();
            files.entry((entry.user, filename)).or_default().push(entry);
        }

        // Sort extents by extent number
        for extents in files.values_mut() {
            extents.sort_by_key(|e| e.extent_number());
        }

        files
    }

    /// Bare names address user 0; `N:NAME.EXT` selects another CP/M user.
    fn file_key(name: &str) -> Result<(u8, String)> {
        if let Some((user, filename)) = name.split_once(':') {
            let user = user.parse::<u8>().ok().filter(|&u| u <= 31)
                .ok_or_else(|| DskError::filesystem("Invalid CP/M user number"))?;
            Ok((user, filename.to_string()))
        } else {
            Ok((0, name.to_string()))
        }
    }

    /// Convert a block number to a logical sector number (accounting for reserved tracks)
    fn block_to_sector(&self, block_num: u16) -> usize {
        let block_size = self.spec.block_size();
        let sector_size = self.spec.sector_size as usize;
        let sectors_per_block = block_size / sector_size;
        let sectors_per_track = self.spec.sectors_per_track as usize;

        // Blocks start after reserved tracks
        let reserved_sectors = self.spec.reserved_tracks as usize * sectors_per_track;

        // Calculate the absolute sector number
        (block_num as usize) * sectors_per_block + reserved_sectors
    }

    /// Read data from allocation blocks
    fn read_blocks(&self, blocks: &[u16]) -> Result<Vec<u8>> {
        let block_size = self.spec.block_size();
        let sector_size = self.spec.sector_size as usize;
        let sectors_per_block = block_size / sector_size;

        let mut data = Vec::new();

        for &block_num in blocks {
            if block_num == 0 {
                continue;
            }

            // Convert block to starting sector
            let start_sector = self.block_to_sector(block_num);

            // Read all sectors for this block
            for i in 0..sectors_per_block {
                let absolute_sector = start_sector + i;
                if let Some(sector_data) = Self::logical_sector(self.image, &self.spec, absolute_sector) {
                    data.extend_from_slice(sector_data);
                } else {
                    data.extend(std::iter::repeat(0).take(sector_size));
                }
            }
        }

        Ok(data)
    }

    /// Extract allocation blocks from directory entry
    fn extract_blocks(&self, entry: &CpmDirEntry) -> Vec<u16> {
        let mut blocks = Vec::new();

        // Use allocation size from specification (8-bit or 16-bit block numbers)
        match self.spec.allocation_size {
            AllocationSize::Byte => {
                // 8-bit allocation numbers (16 entries)
                for &block in &entry.allocation {
                    if block != 0 {
                        blocks.push(block as u16);
                    }
                }
            }
            AllocationSize::Word => {
                // 16-bit allocation numbers (8 entries)
                for i in (0..entry.allocation.len()).step_by(2) {
                    if i + 1 < entry.allocation.len() {
                        let block =
                            u16::from_le_bytes([entry.allocation[i], entry.allocation[i + 1]]);
                        if block != 0 {
                            blocks.push(block);
                        }
                    }
                }
            }
        }

        blocks
    }

    /// Get the disk specification
    pub fn specification(&self) -> &DiskSpecification {
        &self.spec
    }

    /// Read the first block of a file to parse headers
    fn read_first_block(&self, blocks: &[u16]) -> Result<Vec<u8>> {
        if blocks.is_empty() {
            return Ok(Vec::new());
        }

        // Just read the first block
        self.read_blocks(&blocks[..1])
    }

    /// List directory entries with extended information including headers
    pub fn read_dir_extended(&self) -> Result<Vec<ExtendedDirEntry>> {
        self.read_dir_extended_internal(false)
    }

    /// List directory entries with extended information, optionally including deleted files
    pub fn read_dir_extended_with_deleted(&self) -> Result<Vec<ExtendedDirEntry>> {
        self.read_dir_extended_internal(true)
    }

    /// Internal method to list directory entries with extended information
    fn read_dir_extended_internal(&self, include_deleted: bool) -> Result<Vec<ExtendedDirEntry>> {
        // Read directory entries (with or without deleted)
        let dir_entries = Self::read_directory_internal(self.image, &self.spec, include_deleted)?;
        
        // Merge extents from the directory entries
        let files = Self::merge_extents_from_entries(&dir_entries);
        let mut entries = Vec::new();
        let block_size = self.spec.block_size();

        for ((_, filename), extents) in files {
            if extents.is_empty() {
                continue;
            }

            let first_extent = extents[0];

            // Collect all blocks from all extents
            let mut all_blocks = Vec::new();
            for extent in &extents {
                all_blocks.extend(self.extract_blocks(extent));
            }

            // Calculate total file size from all extents
            let mut total_size = 0;
            for (i, extent) in extents.iter().enumerate() {
                let is_last = i == extents.len() - 1;
                if is_last {
                    total_size += extent.extent_size(extent.bytes_in_last_record);
                } else {
                    total_size += extent.record_count as usize * 128;
                }
            }

            // Calculate allocated size
            let allocated = all_blocks.len() * block_size;

            // Try to read the first block and parse header
            let header = if !all_blocks.is_empty() {
                match self.read_first_block(&all_blocks) {
                    Ok(data) => try_parse_header(&data),
                    Err(_) => FileHeader::default(),
                }
            } else {
                FileHeader::default()
            };

            entries.push(ExtendedDirEntry {
                name: filename,
                user: first_extent.user,
                index: first_extent.index,
                blocks: all_blocks.len(),
                allocated,
                size: total_size,
                attributes: first_extent.attributes(),
                header,
            });
        }

        // Sort by filename
        entries.sort_by(|a, b| a.name.cmp(&b.name));

        Ok(entries)
    }
}

impl CpmFileSystem<'_> {
    /// Create a CP/M filesystem from an image, auto-detecting the specification
    pub fn from_image(image: &DiskImage) -> Result<CpmFileSystem<'_>> {
        // Use DiskSpecification to detect the disk format
        let spec = DiskSpecification::identify(image);

        // Validate the specification
        if spec.sector_size == 0 || spec.sectors_per_track == 0 {
            return Err(DskError::filesystem("Invalid disk specification"));
        }

        CpmFileSystem::new(image, spec)
    }

}

impl<'a> FileSystem for CpmFileSystem<'a> {
    fn from_image<'b>(_image: &'b DiskImage) -> Result<Self>
    where
        Self: Sized,
    {
        Err(DskError::filesystem(
            "Use CpmFileSystem::from_image() directly",
        ))
    }

    fn from_image_mut<'b>(_image: &'b mut DiskImage) -> Result<Self>
    where
        Self: Sized,
    {
        Err(DskError::filesystem(
            "Mutable CP/M filesystem not yet implemented",
        ))
    }

    fn read_dir(&self) -> Result<Vec<DirEntry>> {
        let files = self.merge_extents();
        let mut entries = Vec::new();

        for ((_, filename), extents) in files {
            if extents.is_empty() {
                continue;
            }

            let first_extent = extents[0];

            // Calculate total file size from all extents
            let mut total_size = 0;
            for (i, extent) in extents.iter().enumerate() {
                let is_last = i == extents.len() - 1;
                if is_last {
                    // Last extent may have partial last record
                    total_size += extent.extent_size(extent.bytes_in_last_record);
                } else {
                    // Full extent
                    total_size += extent.record_count as usize * 128;
                }
            }

            entries.push(DirEntry {
                name: filename,
                user: first_extent.user,
                extent: first_extent.extent_low,
                size: total_size,
                attributes: first_extent.attributes(),
            });
        }

        // Sort by filename
        entries.sort_by(|a, b| a.name.cmp(&b.name));

        Ok(entries)
    }

    fn read_file(&self, name: &str) -> Result<Vec<u8>> {
        let files = self.merge_extents();

        let extents = files
            .get(&Self::file_key(name)?)
            .ok_or_else(|| DskError::FileNotFound(name.to_string()))?;

        if extents.is_empty() {
            return Ok(Vec::new());
        }

        // Read all allocation blocks from all extents
        let mut file_data = Vec::new();

        for extent in extents {
            let blocks = self.extract_blocks(extent);
            let block_data = self.read_blocks(&blocks)?;
            file_data.extend_from_slice(&block_data);
        }

        // Trim to actual file size
        let mut actual_size = 0;
        for (i, extent) in extents.iter().enumerate() {
            let is_last = i == extents.len() - 1;
            if is_last {
                actual_size += extent.extent_size(extent.bytes_in_last_record);
            } else {
                actual_size += extent.record_count as usize * 128;
            }
        }

        if file_data.len() > actual_size {
            file_data.truncate(actual_size);
        }

        // Strip headers if present (CP/M filesystems may have AMSDOS/PLUS3DOS headers)
        let header = try_parse_header(&file_data);
        if header.header_size > 0 {
            match header.header_type {
                HeaderType::Amsdos | HeaderType::Plus3dos => {
                    // Strip the header
                    if file_data.len() >= header.header_size {
                        file_data.drain(0..header.header_size);
                    }
                }
                HeaderType::None => {}
            }
        }

        Ok(file_data)
    }

    fn write_file(&mut self, _name: &str, _data: &[u8]) -> Result<()> {
        Err(DskError::filesystem("Writing files not yet implemented"))
    }

    fn delete_file(&mut self, _name: &str) -> Result<()> {
        Err(DskError::filesystem("Deleting files not yet implemented"))
    }

    fn info(&self) -> FileSystemInfo {
        let total_blocks = self.spec.block_count() as usize;

        // Calculate used blocks
        let mut used_blocks = 0;
        for entry in &self.directory_entries {
            let blocks = self.extract_blocks(entry);
            used_blocks += blocks.len();
        }

        // Account for directory blocks
        let dir_blocks = self.spec.directory_blocks as usize;

        FileSystemInfo {
            fs_type: format!("CP/M ({})", self.spec.format),
            total_blocks,
            free_blocks: total_blocks.saturating_sub(used_blocks + dir_blocks),
            block_size: self.spec.block_size(),
        }
    }
}

impl CpmFileSystem<'_> {
    /// Read file binary data with optional header inclusion (CP/M only)
    /// 
    /// # Arguments
    /// * `name` - Filename to read (prefix with `N:` to select CP/M user N; defaults to user 0)
    /// * `include_header` - If true, returns raw data including AMSDOS/PLUS3DOS headers if present.
    ///                      If false, strips headers and returns only file data.
    pub fn read_file_binary(&self, name: &str, include_header: bool) -> Result<Vec<u8>> {
        let files = self.merge_extents();

        let extents = files
            .get(&Self::file_key(name)?)
            .ok_or_else(|| DskError::FileNotFound(name.to_string()))?;

        if extents.is_empty() {
            return Ok(Vec::new());
        }

        // Read all allocation blocks from all extents
        let mut file_data = Vec::new();

        for extent in extents {
            let blocks = self.extract_blocks(extent);
            let block_data = self.read_blocks(&blocks)?;
            file_data.extend_from_slice(&block_data);
        }

        // Trim to actual file size
        let mut actual_size = 0;
        for (i, extent) in extents.iter().enumerate() {
            let is_last = i == extents.len() - 1;
            if is_last {
                actual_size += extent.extent_size(extent.bytes_in_last_record);
            } else {
                actual_size += extent.record_count as usize * 128;
            }
        }

        if file_data.len() > actual_size {
            file_data.truncate(actual_size);
        }

        // If include_header is false, strip headers if present
        if !include_header {
            let header = try_parse_header(&file_data);
            if header.header_size > 0 {
                match header.header_type {
                    HeaderType::Amsdos | HeaderType::Plus3dos => {
                        // Strip the header
                        if file_data.len() >= header.header_size {
                            file_data.drain(0..header.header_size);
                        }
                    }
                    HeaderType::None => {}
                }
            }
        }

        Ok(file_data)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_dir_entry() {
        let mut data = [0u8; 32];
        data[0] = 0; // User 0
        data[1..9].copy_from_slice(b"TESTFILE");
        data[9..12].copy_from_slice(b"TXT");
        data[12] = 0; // Extent low
        data[15] = 10; // Record count

        let entry = CpmDirEntry::parse(&data, 0).unwrap();
        assert_eq!(entry.user, 0);
        assert_eq!(entry.index, 0);
        assert_eq!(entry.filename_str(), "TESTFILE.TXT");
        assert_eq!(entry.record_count, 10);
    }

    #[test]
    fn test_parse_deleted_entry() {
        let mut data = [0u8; 32];
        data[0] = 0xE5; // Deleted

        let entry = CpmDirEntry::parse(&data, 0);
        assert!(entry.is_none());
    }

    #[test]
    fn test_filename_with_attributes() {
        let mut data = [0u8; 32];
        data[0] = 0; // User 0
        data[1..9].copy_from_slice(b"TESTFILE");
        // Extension with high bits set (attributes)
        data[9] = b'T' | 0x80; // Read-only
        data[10] = b'X' | 0x80; // System
        data[11] = b'T' | 0x80; // Archive

        let entry = CpmDirEntry::parse(&data, 5).unwrap();
        assert_eq!(entry.filename_str(), "TESTFILE.TXT");
        assert_eq!(entry.index, 5);
        assert!(entry.is_read_only());
        assert!(entry.is_system());
        assert!(entry.is_archive());
    }

    #[test]
    fn test_extent_number() {
        let mut data = [0u8; 32];
        data[0] = 0;
        data[1..9].copy_from_slice(b"TEST    ");
        data[9..12].copy_from_slice(b"   ");
        data[12] = 3; // Extent low
        data[14] = 1; // Extent high

        let entry = CpmDirEntry::parse(&data, 10).unwrap();
        assert_eq!(entry.index, 10);
        // Extent number = (high << 5) | (low & 0x1F) = (1 << 5) | 3 = 35
        assert_eq!(entry.extent_number(), 35);
    }

    #[test]
    fn test_read_file_on_second_side_of_alternate_disk() {
        let mut image = DiskImage::builder()
            .num_sides(2)
            .num_tracks(2)
            .sectors_per_track(2)
            .build()
            .unwrap();
        let mut directory = [0xE5; 512];
        directory[0] = 0;
        directory[1..9].copy_from_slice(b"SIDE1   ");
        directory[9..12].copy_from_slice(b"TXT");
        directory[15] = 8; // 1024 bytes in block 1
        directory[16] = 1;
        image.write_sector(0, 0, 0xC1, &directory).unwrap();
        image.write_sector(1, 0, 0xC1, &[0x11; 512]).unwrap();
        image.write_sector(1, 0, 0xC2, &[0x22; 512]).unwrap();

        let mut spec = DiskSpecification::new();
        spec.side = DiskSpecSide::DoubleAlternate;
        spec.tracks_per_side = 2;
        spec.sectors_per_track = 2;
        spec.reserved_tracks = 0;
        spec.directory_blocks = 1;
        let fs = CpmFileSystem::new(&image, spec).unwrap();
        let data = fs.read_file_binary("SIDE1.TXT", true).unwrap();
        assert_eq!(&data[..512], &[0x11; 512]);
        assert_eq!(&data[512..], &[0x22; 512]);
    }

    #[test]
    fn test_identical_names_on_different_cpm_users_stay_separate() {
        let mut image = DiskImage::builder()
            .num_tracks(3)
            .sectors_per_track(2)
            .build()
            .unwrap();
        let mut directory = [0xE5; 512];
        for (index, user, block) in [(0, 0, 1), (1, 1, 2)] {
            let offset = index * 32;
            directory[offset] = user;
            directory[offset + 1..offset + 9].copy_from_slice(b"SAME    ");
            directory[offset + 9..offset + 12].copy_from_slice(b"TXT");
            directory[offset + 15] = 8;
            directory[offset + 16] = block;
        }
        image.write_sector(0, 0, 0xC1, &directory).unwrap();
        image.write_sector(0, 1, 0xC1, &[0x11; 512]).unwrap();
        image.write_sector(0, 1, 0xC2, &[0x11; 512]).unwrap();
        image.write_sector(0, 2, 0xC1, &[0x22; 512]).unwrap();
        image.write_sector(0, 2, 0xC2, &[0x22; 512]).unwrap();

        let mut spec = DiskSpecification::new();
        spec.tracks_per_side = 3;
        spec.sectors_per_track = 2;
        spec.reserved_tracks = 0;
        spec.directory_blocks = 1;
        let fs = CpmFileSystem::new(&image, spec).unwrap();
        let entries = fs.read_dir().unwrap();
        assert_eq!(entries.len(), 2);
        assert!(entries.iter().any(|entry| entry.user == 0 && entry.name == "SAME.TXT"));
        assert!(entries.iter().any(|entry| entry.user == 1 && entry.name == "SAME.TXT"));
        assert_eq!(fs.read_file_binary("SAME.TXT", true).unwrap(), vec![0x11; 1024]);
        assert_eq!(fs.read_file_binary("1:SAME.TXT", true).unwrap(), vec![0x22; 1024]);
    }
}
