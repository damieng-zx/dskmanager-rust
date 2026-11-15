//! Disk specification for CP/M and similar file systems
//!
//! This module provides detection and representation of disk specifications
//! used by CP/M and compatible systems (Amstrad PCW, CPC, Spectrum +3, etc.)

use crate::image::DiskImage;
use std::fmt;


/// Side configuration for double-sided disks
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiskSpecSide {
    /// Single-sided disk
    Single,
    /// Double-sided with alternating tracks (T0S0, T0S1, T1S0, T1S1...)
    DoubleAlternate,
    /// Double-sided with successive tracks (all of side 0, then all of side 1)
    DoubleSuccessive,
    /// Double-sided with reverse order on side 1
    DoubleReverse,
    /// Invalid or unrecognized
    Invalid,
}

impl fmt::Display for DiskSpecSide {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DiskSpecSide::Single => write!(f, "Single"),
            DiskSpecSide::DoubleAlternate => write!(f, "Double (Alternate)"),
            DiskSpecSide::DoubleSuccessive => write!(f, "Double (Successive)"),
            DiskSpecSide::DoubleReverse => write!(f, "Double (Reverse)"),
            DiskSpecSide::Invalid => write!(f, "Invalid"),
        }
    }
}

/// Track density
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiskSpecTrack {
    /// Single density (40 tracks)
    Single,
    /// Double density (80 tracks)
    Double,
    /// Invalid or unrecognized
    Invalid,
}

impl fmt::Display for DiskSpecTrack {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DiskSpecTrack::Single => write!(f, "Single"),
            DiskSpecTrack::Double => write!(f, "Double"),
            DiskSpecTrack::Invalid => write!(f, "Invalid"),
        }
    }
}

/// Allocation block size type (for block allocation map)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AllocationSize {
    /// 8-bit block numbers (max 255 blocks)
    Byte,
    /// 16-bit block numbers (max 65535 blocks)
    Word,
}

impl fmt::Display for AllocationSize {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AllocationSize::Byte => write!(f, "8-bit/byte"),
            AllocationSize::Word => write!(f, "16-bit/word"),
        }
    }
}

/// Disk specification containing all parameters needed to read a CP/M filesystem
#[derive(Debug, Clone)]
pub struct DiskSpecification {
    /// How this specification was determined
    pub source: String,
    /// Disk format name
    pub format: String,
    /// Side configuration
    pub side: DiskSpecSide,
    /// Track density
    pub track: DiskSpecTrack,
    /// Number of tracks per side
    pub tracks_per_side: u8,
    /// Number of sectors per track
    pub sectors_per_track: u8,
    /// Sector size in bytes
    pub sector_size: u16,
    /// FDC sector size code (N value)
    pub fdc_sector_size: u8,
    /// Number of reserved tracks (for boot sector, etc.)
    pub reserved_tracks: u8,
    /// Block shift value (block size = 128 << block_shift)
    pub block_shift: u8,
    /// Number of directory blocks
    pub directory_blocks: u8,
    /// Gap length for read/write operations
    pub gap_read_write: u8,
    /// Gap length for formatting
    pub gap_format: u8,
    /// Checksum byte (from spec block)
    pub checksum: u8,
    /// Allocation block size type
    pub allocation_size: AllocationSize,
}

impl Default for DiskSpecification {
    fn default() -> Self {
        Self {
            source: String::new(),
            format: "Amstrad PCW/+3 DD/SS/ST (Assumed)".to_string(),
            side: DiskSpecSide::Single,
            track: DiskSpecTrack::Single,
            tracks_per_side: 40,
            sectors_per_track: 9,
            sector_size: 512,
            fdc_sector_size: 2,
            reserved_tracks: 1,
            block_shift: 3,
            directory_blocks: 2,
            gap_read_write: 42,
            gap_format: 82,
            checksum: 0,
            allocation_size: AllocationSize::Byte,
        }
    }
}

/// Trait for format detectors that can identify disk specifications
pub trait FormatDetector {
    /// Attempt to detect and return a disk specification for the given image.
    /// Returns `Some(DiskSpecification)` if this detector can handle the disk,
    /// or `None` if it cannot.
    fn detect(&self, image: &DiskImage) -> Option<DiskSpecification>;
}

/// Amstrad PCW format detector
pub struct AmstradPCW;

impl FormatDetector for AmstradPCW {
    fn detect(&self, image: &DiskImage) -> Option<DiskSpecification> {
        let first_sector = get_first_logical_sector(image)?;
        let (_, sector_data) = first_sector;

        if sector_data.len() < 10 {
            return None;
        }

        // Check if first 10 bytes are all the same value (blank spec block)
        let check_byte = sector_data[0];
        let all_same = sector_data[..10].iter().all(|&b| b == check_byte);
        if all_same {
            let mut spec = DiskSpecification::new();
            spec.set_defaults();
            spec.source = format!("Sector 0 spec block is all 0x{:02X}", check_byte);
            spec.update_allocation_size();
            return Some(spec);
        }

        // Check format byte
        match sector_data[0] {
            0 => {
                // PCW Single Sided — refine into 9512 / Spectrum +3 / 8256 via
                // mod-256 checksum of sector 0 (see DskImageManager FormatAnalysis.pas:63)
                let mut spec = DiskSpecification::new();
                let variant = match mod_checksum(&sector_data, 256) {
                    1 => "Amstrad PCW 9512",
                    3 => "Spectrum +3",
                    255 => "Amstrad PCW 8256",
                    _ => "Amstrad PCW/Spectrum +3",
                };
                let suffix = if image.disk_count() == 1 { "CF2" } else { "CF2DD" };
                spec.format = format!("{} {}", variant, suffix);
                spec.source = "Sector 0 spec block (format byte 0)".to_string();
                parse_spec_block(&mut spec, &sector_data);
                append_size_indicator(&mut spec, image);
                spec.update_allocation_size();
                Some(spec)
            }
            3 => {
                // PCW Double Sided
                let mut spec = DiskSpecification::new();
                spec.format = "Amstrad PCW DD/DS/DT CF2DD".to_string();
                spec.source = "Sector 0 spec block (format byte 3)".to_string();
                parse_spec_block(&mut spec, &sector_data);
                append_size_indicator(&mut spec, image);
                spec.update_allocation_size();
                Some(spec)
            }
            _ => None,
        }
    }
}

/// Enhanced Spectrum +3 / PCW format detector
///
/// Detects custom 10-sector single-sided layouts that pack more data onto a
/// standard 3" disk: HiForm 203 and Ultra 208 (Chris Pile), Ian High and
/// Ian Max (Ian Collier's skewed variants), and Supermat 192 / XCF2 (Ian Cull).
/// Ported from DskImageManager FormatAnalysis.pas:92-122.
pub struct EnhancedPlus3;

impl FormatDetector for EnhancedPlus3 {
    fn detect(&self, image: &DiskImage) -> Option<DiskSpecification> {
        if image.disk_count() != 1 {
            return None;
        }
        let disk = image.get_disk(0)?;
        let track0 = disk.get_track(0)?;
        let t0_sectors = track0.sectors();
        if t0_sectors.len() != 10 {
            return None;
        }

        let first_logical = t0_sectors.iter().min_by_key(|s| s.id.sector)?;
        let data = first_logical.data();
        if data.len() <= 10 {
            return None;
        }

        // Supermat 192 / XCF2 (Ian Cull) — distinct fingerprint, check first
        if data[2] == 40 && data[7] == 3 && data[9] == 23 {
            let mut spec = DiskSpecification::new();
            spec.format = "Supermat 192/XCF2".to_string();
            spec.source = "10 sector spec block matches Supermat (Ian Cull)".to_string();
            parse_spec_block(&mut spec, data);
            spec.update_allocation_size();
            return Some(spec);
        }

        // HiForm / Ultra208 / Ian Collier family — 42 tracks, gap r/w = 12
        if data[2] != 42 || data[8] != 12 {
            return None;
        }

        // Sector ID at physical position 1 of track 0 — interleave marker
        let t0_pos1_id = t0_sectors.get(1).map(|s| s.id.sector);
        // Sector ID at physical position 0 of track 1 — track-to-track skew marker
        let t1_pos0_id = disk
            .get_track(1)
            .and_then(|t| t.sectors().first())
            .map(|s| s.id.sector);

        let name = match data[5] {
            0 => {
                // Ultra 208 / Ian Max — 0 reserved tracks
                if t0_pos1_id == Some(8) {
                    match t1_pos0_id {
                        Some(7) => "Ultra 208/Ian Max",
                        Some(8) => "Maybe Ultra 208 or Ian Max (skew lost)",
                        _ => "Maybe Ultra 208 or Ian Max (custom skew)",
                    }
                } else {
                    "Possibly Ultra 208 or Ian Max (interleave lost)"
                }
            }
            1 => {
                // HiForm 203 / Ian High — 1 reserved track
                if t0_pos1_id == Some(8) {
                    match t1_pos0_id {
                        Some(7) => "Ian High",
                        Some(1) => "HiForm 203",
                        _ => "Maybe HiForm 203 or Ian High (custom skew)",
                    }
                } else {
                    "Possibly HiForm 203 or Ian High (interleave lost)"
                }
            }
            _ => "Possibly HiForm or Ian High (unknown reserved tracks)",
        };

        let mut spec = DiskSpecification::new();
        spec.format = name.to_string();
        spec.source = format!(
            "10 sector +3 spec block (tracks={}, reserved={}, gap r/w={})",
            data[2], data[5], data[8]
        );
        parse_spec_block(&mut spec, data);
        spec.update_allocation_size();
        Some(spec)
    }
}

/// Amstrad CPC System format detector
pub struct AmstradCPCSystem;

impl FormatDetector for AmstradCPCSystem {
    fn detect(&self, image: &DiskImage) -> Option<DiskSpecification> {
        let first_sector = get_first_logical_sector(image)?;
        let (sector_id, sector_data) = first_sector;

        // Check first sector ID for CPC System format
        if sector_id == 0x41 {
            let mut spec = DiskSpecification::new();
            spec.set_defaults();
            spec.source = "First logical sector has ID of 65 (0x41)".to_string();
            spec.format = "Amstrad CPC DD/SS/ST system".to_string();
            spec.reserved_tracks = 2;
            infer_side_from_image(&mut spec, image);
            spec.update_allocation_size();
            return Some(spec);
        }

        // Check spec block format byte
        if sector_data.len() >= 10 && sector_data[0] == 1 {
            let mut spec = DiskSpecification::new();
            spec.format = "Amstrad CPC DD/SS/ST system".to_string();
            spec.source = "Sector 0 spec block (format byte 1)".to_string();
            parse_spec_block(&mut spec, &sector_data);
            spec.update_allocation_size();
            Some(spec)
        } else {
            None
        }
    }
}

/// Amstrad CPC Data format detector
pub struct AmstradCPCData;

impl FormatDetector for AmstradCPCData {
    fn detect(&self, image: &DiskImage) -> Option<DiskSpecification> {
        let first_sector = get_first_logical_sector(image)?;
        let (sector_id, sector_data) = first_sector;

        // Check first sector ID for CPC Data format
        if sector_id == 0xC1 {
            let mut spec = DiskSpecification::new();
            spec.set_defaults();
            spec.source = "First logical sector has ID of 193 (0xC1)".to_string();
            spec.format = "Amstrad CPC DD/SS/ST data".to_string();
            spec.reserved_tracks = 0;
            infer_side_from_image(&mut spec, image);
            spec.update_allocation_size();
            return Some(spec);
        }

        // Check spec block format byte
        if sector_data.len() >= 10 && sector_data[0] == 2 {
            let mut spec = DiskSpecification::new();
            spec.format = "Amstrad CPC DD/SS/ST data".to_string();
            spec.source = "Sector 0 spec block (format byte 2)".to_string();
            parse_spec_block(&mut spec, &sector_data);
            spec.update_allocation_size();
            Some(spec)
        } else {
            None
        }
    }
}

/// Tatung Einstein format detector
pub struct Einstein;

impl FormatDetector for Einstein {
    fn detect(&self, image: &DiskImage) -> Option<DiskSpecification> {
        let first_sector = get_first_logical_sector(image)?;
        let (_, data) = first_sector;

        if data.len() >= 6 {
            // Einstein boot sector signature: 00 E1 00 FB 00 FA
            if data[0] == 0x00
                && data[1] == 0xE1
                && data[2] == 0x00
                && data[3] == 0xFB
                && data[4] == 0x00
                && data[5] == 0xFA
            {
                let mut spec = DiskSpecification::new();
                spec.format = "Tatung Einstein".to_string();
                spec.source = "Signature 00 E1 00 FB 00 FA on first logical sector".to_string();
                spec.sector_size = 512;
                spec.sectors_per_track = 10;
                spec.tracks_per_side = 40;
                spec.block_shift = 4;
                spec.reserved_tracks = 2;
                spec.directory_blocks = 1;
                spec.allocation_size = AllocationSize::Word;
                spec.fdc_sector_size = 2;
                return Some(spec);
            }
        }

        None
    }
}

/// MGT Sam Coupe format detector
pub struct Mgt;

impl FormatDetector for Mgt {
    fn detect(&self, image: &DiskImage) -> Option<DiskSpecification> {
        let disk = image.get_disk(0)?;
        let track = disk.get_track(0)?;

        // Check for MGT format: double-sided, 80 tracks, 10 sectors of 512 bytes
        let total_tracks: usize = image.disks().iter().map(|d| d.track_count()).sum();
        let is_double_sided = image.disk_count() == 2;
        if is_double_sided && total_tracks >= 160 {
            let sectors = track.sectors();
            if sectors.len() == 10 {
                let all_512 = sectors.iter().all(|s| s.advertised_size() == 512);
                if all_512 {
                    // Identify DOS variant from first logical sector data
                    let first_logical = sectors.iter().min_by_key(|s| s.id.sector);
                    let dos = first_logical
                        .map(|s| identify_mgt_dos(s.data()))
                        .unwrap_or("");

                    let mut spec = DiskSpecification::new();
                    spec.format = if dos.is_empty() {
                        "MGT Sam Coupe".to_string()
                    } else {
                        format!("MGT Sam Coupe {}", dos)
                    };
                    spec.source = "Double sided 80 track 10 sectors of 512 bytes".to_string();
                    spec.sector_size = 512;
                    spec.sectors_per_track = 10;
                    spec.tracks_per_side = 80;
                    spec.side = DiskSpecSide::DoubleSuccessive;
                    spec.track = DiskSpecTrack::Double;
                    spec.reserved_tracks = 0;
                    spec.directory_blocks = 4;
                    spec.fdc_sector_size = 2;
                    spec.update_allocation_size();
                    return Some(spec);
                }
            }
        }

        None
    }
}

/// Identify MGT DOS variant from the first logical sector data.
/// Matches DskImageManager FormatAnalysis.pas:129-136.
fn identify_mgt_dos(data: &[u8]) -> &'static str {
    if data.len() >= 236 && &data[232..236] == b"BDOS" {
        return "BDOS";
    }
    if data.len() > 210 {
        match data[210] {
            0 | 255 => "SAMDOS",
            _ => "MasterDOS",
        }
    } else {
        ""
    }
}

/// Timex/Sinclair TS2068 format detector
pub struct Ts2068;

impl FormatDetector for Ts2068 {
    fn detect(&self, image: &DiskImage) -> Option<DiskSpecification> {
        let disk = image.get_disk(0)?;
        let track = disk.get_track(0)?;

        // Check for TS2068 format: 16 sectors of 256 bytes, starting at ID 0
        let sectors = track.sectors();
        if sectors.len() == 16 {
            let all_256 = sectors.iter().all(|s| s.advertised_size() == 256);
            let starts_at_0 = sectors.iter().any(|s| s.id.sector == 0);
            if all_256 && starts_at_0 {
                let mut spec = DiskSpecification::new();
                spec.format = "Timex/Sinclair TS2068".to_string();
                spec.source = "16x 256 byte sectors per track, starting ID 0".to_string();
                spec.sector_size = 256;
                spec.sectors_per_track = 16;
                spec.tracks_per_side = 40;
                spec.gap_read_write = 12;
                spec.gap_format = 23;
                spec.reserved_tracks = 2;
                spec.directory_blocks = 1;
                spec.fdc_sector_size = 1;
                spec.update_allocation_size();
                return Some(spec);
            }
        }

        None
    }
}

/// Assumed PCW Single Sided format detector (fallback for blank spec blocks)
pub struct AssumedPcwSingleSided;

impl FormatDetector for AssumedPcwSingleSided {
    fn detect(&self, image: &DiskImage) -> Option<DiskSpecification> {
        // This detects blank spec blocks (all same byte)
        let first_sector = get_first_logical_sector(image)?;
        let (_, sector_data) = first_sector;

        if sector_data.len() < 10 {
            return None;
        }

        // Check if first 10 bytes are all the same value (blank spec block)
        let check_byte = sector_data[0];
        let all_same = sector_data[..10].iter().all(|&b| b == check_byte);
        if all_same {
            let mut spec = DiskSpecification::new();
            spec.set_defaults();
            spec.source = format!("Sector 0 spec block is all 0x{:02X}", check_byte);
            spec.update_allocation_size();
            Some(spec)
        } else {
            None
        }
    }
}

/// Invalid format detector (for unrecognized format bytes)
pub struct InvalidFormat;

impl FormatDetector for InvalidFormat {
    fn detect(&self, image: &DiskImage) -> Option<DiskSpecification> {
        let first_sector = get_first_logical_sector(image)?;
        let (_, sector_data) = first_sector;

        if sector_data.len() < 10 {
            return None;
        }

        // Check if we have a spec block with an invalid format byte
        // Valid format bytes are 0, 1, 2, 3
        let format_byte = sector_data[0];
        if format_byte > 3 {
            // Check if it's not a blank spec block (all same byte)
            let check_byte = sector_data[0];
            let all_same = sector_data[..10].iter().all(|&b| b == check_byte);
            if !all_same {
                let mut spec = DiskSpecification::new();
                spec.format = "Invalid".to_string();
                spec.source = format!("Unknown format byte: 0x{:02X}", format_byte);
                return Some(spec);
            }
        }

        None
    }
}

/// Default fallback detector (always matches if image has sectors)
pub struct DefaultFallback;

impl FormatDetector for DefaultFallback {
    fn detect(&self, image: &DiskImage) -> Option<DiskSpecification> {
        // This is the final fallback - if we have sectors but nothing else matched,
        // return a default assumed PCW spec
        if get_first_logical_sector(image).is_some() {
            let mut spec = DiskSpecification::new();
            spec.set_defaults();
            spec.source = "Default fallback (no specific format detected)".to_string();
            spec.update_allocation_size();
            Some(spec)
        } else {
            None
        }
    }
}

/// Derive the side configuration from the image when the spec block says
/// nothing. Detectors that key off a sector ID alone otherwise report every
/// double-sided disk as single-sided, halving the visible blocks and pruning
/// directory entries that point at side 1.
fn infer_side_from_image(spec: &mut DiskSpecification, image: &DiskImage) {
    if image.disk_count() != 2 {
        return;
    }
    spec.side = match image.spec().side_mode {
        crate::format::SideMode::Alternate => DiskSpecSide::DoubleAlternate,
        crate::format::SideMode::Successive => DiskSpecSide::DoubleSuccessive,
        crate::format::SideMode::SingleSide => DiskSpecSide::DoubleAlternate,
    };
    if let Some(disk) = image.get_disk(0) {
        if disk.track_count() > 0 {
            spec.tracks_per_side = disk.track_count() as u8;
        }
    }
}

/// Modular sum of the bytes of `data` (used to discriminate PCW disk variants)./// Mirrors `TDSKSector.GetModChecksum` in DskImageManager DskImage.pas:1631.
fn mod_checksum(data: &[u8], modulus: u32) -> u32 {
    data.iter().fold(0u32, |acc, &b| (acc + b as u32) % modulus)
}

/// Append " (oversized)" or " (undersized)" to a PCW-family spec's format
/// label when the actual track count on side 0 differs from the standard 40.
fn append_size_indicator(spec: &mut DiskSpecification, image: &DiskImage) {
    let Some(disk) = image.get_disk(0) else { return };
    let high_track_count = disk
        .tracks()
        .iter()
        .rposition(|t| !t.is_empty())
        .map(|i| i + 1)
        .unwrap_or(0);
    if high_track_count > 40 {
        spec.format.push_str(" (oversized)");
    } else if high_track_count > 0 && high_track_count < 40 {
        spec.format.push_str(" (undersized)");
    }
}

/// Parse a spec block from sector data
fn parse_spec_block(spec: &mut DiskSpecification, sector_data: &[u8]) {
    if sector_data.len() < 10 {
        return;
    }

    // Parse side configuration
    spec.side = match sector_data[1] & 0x03 {
        0 => DiskSpecSide::Single,
        1 => DiskSpecSide::DoubleAlternate,
        2 => DiskSpecSide::DoubleSuccessive,
        _ => DiskSpecSide::Invalid,
    };

    // Parse track density
    spec.track = if (sector_data[1] & 0x80) == 0x80 {
        DiskSpecTrack::Double
    } else {
        DiskSpecTrack::Single
    };

    spec.tracks_per_side = sector_data[2];
    spec.sectors_per_track = sector_data[3];

    // Parse sector size (stored as log2(size) - 7)
    let size_code = sector_data[4];
    if size_code <= 8 {
        let calculated_size = 1u16 << (size_code + 7);
        if calculated_size <= 8192 {
            spec.sector_size = calculated_size;
            spec.fdc_sector_size = size_code;
        } else {
            spec.sector_size = 0;
        }
    } else {
        spec.sector_size = 0;
    }

    spec.reserved_tracks = sector_data[5];
    spec.block_shift = if sector_data[6] <= 24 {
        sector_data[6]
    } else {
        0
    };
    spec.directory_blocks = sector_data[7];
    spec.gap_read_write = sector_data[8];
    spec.gap_format = sector_data[9];

    if sector_data.len() > 15 {
        spec.checksum = sector_data[15];
    }
}

/// The detector list, in the order they must be tried.
///
/// Order is load-bearing: the first detector to match wins, so anything that
/// could be claimed by a general detector has to come first. `EnhancedPlus3`
/// must precede `AmstradPCW` because HiForm/Ultra208 and friends all present as
/// format-byte-0 PCW disks; only the extra structural checks separate them.
/// [`detector_order_is_stable`] guards that invariant.
fn detectors() -> Vec<(&'static str, Box<dyn FormatDetector>)> {
    vec![
        ("Einstein", Box::new(Einstein)),
        ("Ts2068", Box::new(Ts2068)),
        ("Mgt", Box::new(Mgt)),
        ("AmstradCPCSystem", Box::new(AmstradCPCSystem)),
        ("AmstradCPCData", Box::new(AmstradCPCData)),
        ("EnhancedPlus3", Box::new(EnhancedPlus3)),
        ("AmstradPCW", Box::new(AmstradPCW)),
        ("AssumedPcwSingleSided", Box::new(AssumedPcwSingleSided)),
        ("InvalidFormat", Box::new(InvalidFormat)),
        ("DefaultFallback", Box::new(DefaultFallback)),
    ]
}

/// Identify the disk specification by trying all format detectors in order
pub fn identify_specification(image: &DiskImage) -> Option<DiskSpecification> {
    for (_, detector) in detectors() {
        if let Some(spec) = detector.detect(image) {
            return Some(spec);
        }
    }

    None
}

impl DiskSpecification {
    /// Create a new disk specification with default values
    pub fn new() -> Self {
        Self::default()
    }

    /// Calculate the block size in bytes
    ///
    /// A shift large enough to overflow yields 0, which callers already treat as
    /// "no blocks" rather than panicking.
    pub fn block_size(&self) -> usize {
        128usize.checked_shl(self.block_shift as u32).unwrap_or(0)
    }

    /// Calculate the number of blocks on the disk
    pub fn block_count(&self) -> u16 {
        let usable = self.usable_capacity();
        usable
            .checked_div(self.block_size())
            .map(|blocks| blocks as u16)
            .unwrap_or(0)
    }

    /// Calculate the usable capacity in bytes (excluding reserved tracks)
    pub fn usable_capacity(&self) -> usize {
        let mut usable_tracks = self.tracks_per_side as usize;
        if self.side != DiskSpecSide::Single {
            usable_tracks *= 2;
        }
        usable_tracks = usable_tracks.saturating_sub(self.reserved_tracks as usize);
        usable_tracks * self.sectors_per_track as usize * self.sector_size as usize
    }

    /// Calculate the number of 128-byte records per track
    pub fn records_per_track(&self) -> usize {
        (self.sector_size as usize * self.sectors_per_track as usize) / 128
    }

    /// Calculate the number of directory entries
    pub fn directory_entries(&self) -> usize {
        (self.directory_blocks as usize * self.block_size()) / 32
    }

    /// Get the number of sides
    pub fn side_count(&self) -> u8 {
        if self.side == DiskSpecSide::Single {
            1
        } else {
            2
        }
    }

    /// Calculate total disk capacity in bytes
    pub fn total_capacity(&self) -> usize {
        let tracks = self.tracks_per_side as usize * self.side_count() as usize;
        tracks * self.sectors_per_track as usize * self.sector_size as usize
    }

    /// Update allocation size based on block count
    fn update_allocation_size(&mut self) {
        if self.block_count() > 255 {
            self.allocation_size = AllocationSize::Word;
        } else {
            self.allocation_size = AllocationSize::Byte;
        }
    }

    /// Identify the disk specification from a disk image
    /// 
    /// This is a convenience method that calls `identify_specification`.
    /// For more control, use `identify_specification` directly.
    pub fn identify(image: &DiskImage) -> Self {
        // Check if we have any sectors first
        if get_first_logical_sector(image).is_none() {
            let mut spec = Self::new();
            spec.format = "Invalid".to_string();
            spec.source = "No sectors found".to_string();
            return spec;
        }

        identify_specification(image).unwrap_or_else(|| {
            let mut spec = Self::new();
            spec.format = "Invalid".to_string();
            spec.source = "No matching format detector found".to_string();
            spec
        })
    }

    /// Set default PCW/+3 values
    fn set_defaults(&mut self) {
        self.format = "Amstrad PCW/+3 DD/SS/ST (Assumed)".to_string();
        self.side = DiskSpecSide::Single;
        self.track = DiskSpecTrack::Single;
        self.tracks_per_side = 40;
        self.sectors_per_track = 9;
        self.sector_size = 512;
        self.fdc_sector_size = 2;
        self.reserved_tracks = 1;
        self.block_shift = 3;
        self.directory_blocks = 2;
        self.gap_read_write = 42;
        self.gap_format = 82;
    }
}

/// Get the first logical sector (lowest sector ID on track 0)
fn get_first_logical_sector(image: &DiskImage) -> Option<(u8, Vec<u8>)> {
    let disk = image.get_disk(0)?;
    let track = disk.get_track(0)?;

    let sectors = track.sectors();
    if sectors.is_empty() {
        return None;
    }

    // Find the sector with the lowest ID
    let min_sector = sectors.iter().min_by_key(|s| s.id.sector)?;
    Some((min_sector.id.sector, min_sector.data().to_vec()))
}

impl fmt::Display for DiskSpecification {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "Format: {}", self.format)?;
        writeln!(f, "Source: {}", self.source)?;
        writeln!(f, "Density: {}", self.track)?;
        writeln!(f, "Sides: {}", self.side)?;
        writeln!(f, "Tracks: {} ({} per side)", self.tracks_per_side * self.side_count(), self.tracks_per_side)?;
        writeln!(f, "Sectors per track: {}", self.sectors_per_track)?;
        writeln!(f, "Sector size: {} bytes (FDC N={})", self.sector_size, self.fdc_sector_size)?;
        writeln!(f, "Reserved tracks: {}", self.reserved_tracks)?;
        writeln!(f, "Block shift: {}", self.block_shift)?;
        writeln!(f, "Directory: {} blocks ({} entries)", self.directory_blocks, self.directory_entries())?;
        writeln!(f, "Gap: R/W {}, format {}", self.gap_read_write, self.gap_format)?;
        writeln!(f, "Block size: {} bytes ({} blocks)", self.block_size(), self.block_count())?;
        writeln!(f, "Allocation size: {}", self.allocation_size)?;
        writeln!(f, "Total capacity: {} KB", self.total_capacity() / 1024)?;
        writeln!(f, "Usable capacity: {} KB", self.usable_capacity() / 1024)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_spec() {
        let spec = DiskSpecification::new();
        assert_eq!(spec.format, "Amstrad PCW/+3 DD/SS/ST (Assumed)");
        assert_eq!(spec.side, DiskSpecSide::Single);
        assert_eq!(spec.tracks_per_side, 40);
        assert_eq!(spec.sectors_per_track, 9);
        assert_eq!(spec.sector_size, 512);
    }

    #[test]
    fn test_block_size() {
        let mut spec = DiskSpecification::new();
        spec.block_shift = 3;
        assert_eq!(spec.block_size(), 1024);

        spec.block_shift = 4;
        assert_eq!(spec.block_size(), 2048);
    }

    #[test]
    fn test_usable_capacity() {
        let spec = DiskSpecification::new();
        // 40 tracks - 1 reserved = 39 tracks
        // 39 * 9 sectors * 512 bytes = 179712 bytes
        assert_eq!(spec.usable_capacity(), 179712);
    }

    #[test]
    fn test_block_count() {
        let spec = DiskSpecification::new();
        // 179712 bytes / 1024 bytes per block = 175 blocks
        assert_eq!(spec.block_count(), 175);
    }

    #[test]
    fn test_block_count_with_zero_block_size() {
        // A spec block can push block_shift high enough that the shift
        // overflows to zero; block_count must report 0 rather than divide.
        let mut spec = DiskSpecification::new();
        spec.block_shift = 200;
        assert_eq!(spec.block_size(), 0);
        assert_eq!(spec.block_count(), 0);
    }

    #[test]
    fn test_directory_entries() {
        let spec = DiskSpecification::new();
        // 2 blocks * 1024 bytes / 32 bytes per entry = 64 entries
        assert_eq!(spec.directory_entries(), 64);
    }

    #[test]
    fn test_records_per_track() {
        let spec = DiskSpecification::new();
        // 9 sectors * 512 bytes / 128 bytes per record = 36 records
        assert_eq!(spec.records_per_track(), 36);
    }

    #[test]
    fn test_format_string() {
        let mut spec = DiskSpecification::new();
        spec.format = "Amstrad PCW/+3 DD/SS/ST".to_string();
        assert_eq!(spec.format, "Amstrad PCW/+3 DD/SS/ST");
        
        spec.format = "Amstrad CPC DD/SS/ST system".to_string();
        assert_eq!(spec.format, "Amstrad CPC DD/SS/ST system");
    }

    #[test]
    fn test_side_display() {
        assert_eq!(format!("{}", DiskSpecSide::Single), "Single");
        assert_eq!(
            format!("{}", DiskSpecSide::DoubleAlternate),
            "Double (Alternate)"
        );
    }

    #[test]
    fn test_detect_double_sided_cpc_data() {
        // A 360K CPC data disk has two sides. Detection picked the side mode
        // from the sector ID alone, so the second side was ignored: only 180 of
        // the 355 blocks were visible and side-1 directory entries were pruned.
        let image = crate::image::DiskImage::builder()
            .spec(crate::format::FormatSpec::amstrad_data_ds())
            .build()
            .unwrap();
        let spec = DiskSpecification::identify(&image);
        assert_eq!(spec.side, DiskSpecSide::DoubleAlternate);
        assert_eq!(spec.side_count(), 2);
        // 2 sides x 40 tracks x 9 x 512 = 368,640 bytes / 1 KB blocks.
        assert_eq!(spec.block_count(), 360);
    }

    #[test]
    fn detector_order_is_stable() {
        // The first matching detector wins, so order defines the result. Any
        // reordering changes how ambiguous disks are identified.
        let names: Vec<&str> = detectors().iter().map(|(name, _)| *name).collect();
        assert_eq!(
            names,
            vec![
                "Einstein",
                "Ts2068",
                "Mgt",
                "AmstradCPCSystem",
                "AmstradCPCData",
                "EnhancedPlus3",
                "AmstradPCW",
                "AssumedPcwSingleSided",
                "InvalidFormat",
                "DefaultFallback",
            ]
        );

        // EnhancedPlus3 must be checked before AmstradPCW: a HiForm disk is
        // otherwise a valid PCW format-byte-0 disk.
        let plus3 = names.iter().position(|n| *n == "EnhancedPlus3").unwrap();
        let pcw = names.iter().position(|n| *n == "AmstradPCW").unwrap();
        assert!(plus3 < pcw, "EnhancedPlus3 must precede AmstradPCW");

        // The catch-all detectors must come last, or they would shadow
        // everything after them.
        assert_eq!(names.last(), Some(&"DefaultFallback"));
        assert_eq!(names[names.len() - 2], "InvalidFormat");
    }

    #[test]
    fn hiform_disk_identifies_as_hiform_not_pcw() {
        // End-to-end guard for the ordering rule above: this image also matches
        // AmstradPCW, so only the ordering keeps the HiForm label.
        let spec_data = spec_block(0, 0, 42, 10, 2, 1, 4, 2, 12, 23);
        let mut t0 = vec![(1, spec_data)];
        for id in 2..=10 {
            t0.push((id, vec![]));
        }
        let mut reordered = vec![t0[0].clone(), t0[7].clone()];
        for (idx, s) in t0.into_iter().enumerate() {
            if idx == 0 || idx == 7 {
                continue;
            }
            reordered.push(s);
        }
        let t1: Vec<(u8, Vec<u8>)> = (1..=10).map(|id| (id, vec![])).collect();
        let image = make_image(1, vec![reordered, t1]);

        let spec = DiskSpecification::identify(&image);
        assert_eq!(spec.format, "HiForm 203");
        assert!(!spec.format.contains("PCW"), "HiForm must not be labelled PCW");
    }

    // --- Helpers for detector tests ---

    use crate::image::{Disk, Sector, SectorId, Track};

    /// Build an image with the given per-track sector IDs and per-sector data.
    /// `tracks[t][s] = (sector_id, data)`. Single-sided. Empty data → 512-byte
    /// 0xE5-filled sector.
    fn make_image(num_sides: u8, tracks: Vec<Vec<(u8, Vec<u8>)>>) -> DiskImage {
        let mut image = DiskImage::builder()
            .num_sides(num_sides)
            .num_tracks(tracks.len() as u8)
            .build()
            .unwrap();

        for side in 0..num_sides {
            let disk = image.get_disk_mut(side).unwrap();
            *disk = Disk::new(side);
            for (track_num, sector_list) in tracks.iter().enumerate() {
                let mut track = Track::new(track_num as u8, side);
                for (id, data) in sector_list {
                    let sector_id = SectorId::new(track_num as u8, side, *id, 2);
                    let body = if data.is_empty() {
                        vec![0xE5; 512]
                    } else {
                        let mut v = data.clone();
                        v.resize(512, 0xE5);
                        v
                    };
                    track.add_sector(Sector::with_data(sector_id, body));
                }
                disk.add_track(track);
            }
        }
        image
    }

    /// Build a spec block (first 16 bytes of sector 0) for the PCW/+3 family.
    ///
    /// One parameter per on-disk field keeps the call sites aligned with the
    /// byte layout, which is what these tests are checking.
    #[allow(clippy::too_many_arguments)]
    fn spec_block(
        format_byte: u8,
        sides_track_bits: u8,
        tracks_per_side: u8,
        sectors_per_track: u8,
        size_code: u8,
        reserved_tracks: u8,
        block_shift: u8,
        directory_blocks: u8,
        gap_rw: u8,
        gap_format: u8,
    ) -> Vec<u8> {
        let mut block = vec![0u8; 16];
        block[0] = format_byte;
        block[1] = sides_track_bits;
        block[2] = tracks_per_side;
        block[3] = sectors_per_track;
        block[4] = size_code;
        block[5] = reserved_tracks;
        block[6] = block_shift;
        block[7] = directory_blocks;
        block[8] = gap_rw;
        block[9] = gap_format;
        block
    }

    // --- EnhancedPlus3 ---

    #[test]
    fn test_detect_hiform_203() {
        // HiForm: 10 sectors, tracks=42, gap r/w=12, reserved=1, T0.S1.id=8, T1.S0.id=1
        let spec_data = spec_block(0, 0, 42, 10, 2, 1, 4, 2, 12, 23);
        let mut t0 = vec![(1, spec_data)];
        for id in 2..=10 {
            t0.push((id, vec![]));
        }
        // Track 0 physical positions: [0]=ID1, [1]=ID8 (skew)
        // Reorder so physical position 1 has ID 8
        let mut reordered = vec![t0[0].clone(), t0[7].clone()];
        for (idx, s) in t0.into_iter().enumerate() {
            if idx == 0 || idx == 7 {
                continue;
            }
            reordered.push(s);
        }
        // Track 1: physical position 0 has ID 1 → HiForm 203
        let mut t1 = Vec::new();
        for id in 1..=10 {
            t1.push((id, vec![]));
        }

        let image = make_image(1, vec![reordered, t1]);
        let spec = DiskSpecification::identify(&image);
        assert_eq!(spec.format, "HiForm 203");
        assert_eq!(spec.tracks_per_side, 42);
        assert_eq!(spec.sectors_per_track, 10);
        assert_eq!(spec.reserved_tracks, 1);
    }

    #[test]
    fn test_detect_ian_high() {
        // Ian High: same as HiForm but T1.S0.id = 7
        let spec_data = spec_block(0, 0, 42, 10, 2, 1, 4, 2, 12, 23);
        let t0: Vec<(u8, Vec<u8>)> = vec![
            (1, spec_data),
            (8, vec![]),
            (2, vec![]),
            (9, vec![]),
            (3, vec![]),
            (10, vec![]),
            (4, vec![]),
            (5, vec![]),
            (6, vec![]),
            (7, vec![]),
        ];
        let t1: Vec<(u8, Vec<u8>)> = vec![(7, vec![]), (8, vec![]), (9, vec![]), (10, vec![])];
        let image = make_image(1, vec![t0, t1]);
        let spec = DiskSpecification::identify(&image);
        assert_eq!(spec.format, "Ian High");
    }

    #[test]
    fn test_detect_ultra_208() {
        // Ultra 208: reserved=0, T0.S1.id=8, T1.S0.id=7
        let spec_data = spec_block(0, 0, 42, 10, 2, 0, 4, 2, 12, 23);
        let t0: Vec<(u8, Vec<u8>)> = vec![
            (1, spec_data),
            (8, vec![]),
            (2, vec![]),
            (3, vec![]),
            (4, vec![]),
            (5, vec![]),
            (6, vec![]),
            (7, vec![]),
            (9, vec![]),
            (10, vec![]),
        ];
        let t1: Vec<(u8, Vec<u8>)> = vec![(7, vec![]), (8, vec![])];
        let image = make_image(1, vec![t0, t1]);
        let spec = DiskSpecification::identify(&image);
        assert_eq!(spec.format, "Ultra 208/Ian Max");
        assert_eq!(spec.reserved_tracks, 0);
    }

    #[test]
    fn test_detect_supermat_192() {
        // Supermat: 10 sectors, tracks=40, dir blocks=3, gap format=23
        let spec_data = spec_block(0, 0, 40, 10, 2, 1, 4, 3, 12, 23);
        let mut t0: Vec<(u8, Vec<u8>)> = vec![(1, spec_data)];
        for id in 2..=10 {
            t0.push((id, vec![]));
        }
        let image = make_image(1, vec![t0]);
        let spec = DiskSpecification::identify(&image);
        assert_eq!(spec.format, "Supermat 192/XCF2");
    }

    #[test]
    fn test_enhanced_plus3_interleave_lost() {
        // HiForm spec block but T0.S1.id != 8 → interleave-lost variant
        let spec_data = spec_block(0, 0, 42, 10, 2, 1, 4, 2, 12, 23);
        let mut t0: Vec<(u8, Vec<u8>)> = vec![(1, spec_data)];
        for id in 2..=10 {
            t0.push((id, vec![]));
        }
        let image = make_image(1, vec![t0]);
        let spec = DiskSpecification::identify(&image);
        assert_eq!(spec.format, "Possibly HiForm 203 or Ian High (interleave lost)");
    }

    // --- AmstradPCW refinements ---

    #[test]
    fn test_pcw_variant_spectrum_plus3() {
        // mod-256 checksum of sector 0 must equal 3.
        // Spec block format byte = 0 contributes 0 to the sum; sectors_per_track=9
        // gives byte 3 = 9, and the rest of the 16-byte block sums to some value.
        // Easiest: use known-empty rest, then pad with one byte to hit checksum=3.
        let mut data = spec_block(0, 0, 40, 9, 2, 1, 3, 2, 42, 82);
        // Sum so far: 0+0+40+9+2+1+3+2+42+82 = 181, plus 6 zeros = 181.
        // 181 mod 256 = 181. We need final sum mod 256 = 3.
        // Need to add bytes summing to (3 - 181) mod 256 = (3 + 75) mod 256 = ...
        // actually (3 - 181 + 256) mod 256 = 78. Add 78 in one byte.
        data.push(78);
        // The full sector will be 512 bytes — remaining bytes are 0xE5.
        // 0xE5 * (512 - 17) bytes adds (0xE5 * 495) mod 256.
        // We need the full sector to sum to 3 mod 256.
        // Simpler approach: build full 512-byte sector then patch byte 16 to fix checksum.
        // Restart with the full sector:
        let mut full = spec_block(0, 0, 40, 9, 2, 1, 3, 2, 42, 82);
        full.resize(512, 0xE5);
        let current: u32 = full.iter().fold(0u32, |acc, &b| (acc + b as u32) % 256);
        // Patch byte 15 (the existing 0 in spec block) to make checksum = 3
        let target = 3u32;
        let delta = (target + 256 - current) % 256;
        full[15] = (full[15] as u32 + delta) as u8;

        // Sanity check
        let new_sum: u32 = full.iter().fold(0u32, |acc, &b| (acc + b as u32) % 256);
        assert_eq!(new_sum % 256, 3);

        let mut t0: Vec<(u8, Vec<u8>)> = vec![(1, full)];
        for id in 2..=9 {
            t0.push((id, vec![]));
        }
        // Fill out 40 tracks of dummy data so we don't get an oversized indicator
        let mut tracks = vec![t0];
        for _ in 1..40 {
            let mut t = Vec::new();
            for id in 1..=9 {
                t.push((id, vec![]));
            }
            tracks.push(t);
        }
        let image = make_image(1, tracks);
        let spec = DiskSpecification::identify(&image);
        assert_eq!(spec.format, "Spectrum +3 CF2");
    }

    #[test]
    fn test_pcw_oversized() {
        // 42 tracks instead of 40 → oversized suffix
        let spec_data = spec_block(0, 0, 42, 9, 2, 1, 3, 2, 42, 82);
        let mut tracks = vec![{
            let mut t = vec![(1, spec_data)];
            for id in 2..=9 {
                t.push((id, vec![]));
            }
            t
        }];
        for _ in 1..42 {
            let mut t = Vec::new();
            for id in 1..=9 {
                t.push((id, vec![]));
            }
            tracks.push(t);
        }
        let image = make_image(1, tracks);
        let spec = DiskSpecification::identify(&image);
        assert!(
            spec.format.ends_with("(oversized)"),
            "expected oversized suffix, got {:?}",
            spec.format
        );
    }

    // --- MGT DOS variants ---

    #[test]
    fn test_mgt_bdos() {
        let mut sector0 = vec![0u8; 512];
        sector0[232..236].copy_from_slice(b"BDOS");
        let tracks_per_side = 80;
        let mut tracks: Vec<Vec<(u8, Vec<u8>)>> = Vec::new();
        for t in 0..tracks_per_side {
            let mut track = Vec::new();
            for id in 1..=10 {
                if t == 0 && id == 1 {
                    track.push((id, sector0.clone()));
                } else {
                    track.push((id, vec![]));
                }
            }
            tracks.push(track);
        }
        // Duplicate the same tracks for side 2
        let mut image = DiskImage::builder()
            .num_sides(2)
            .num_tracks(tracks_per_side)
            .build()
            .unwrap();
        for side in 0..2 {
            let disk = image.get_disk_mut(side).unwrap();
            *disk = Disk::new(side);
            for (t, sector_list) in tracks.iter().enumerate() {
                let mut track = Track::new(t as u8, side);
                for (id, data) in sector_list {
                    let body = if data.is_empty() { vec![0xE5; 512] } else { data.clone() };
                    track.add_sector(Sector::with_data(SectorId::new(t as u8, side, *id, 2), body));
                }
                disk.add_track(track);
            }
        }
        let spec = DiskSpecification::identify(&image);
        assert_eq!(spec.format, "MGT Sam Coupe BDOS");
    }

    #[test]
    fn test_mgt_dos_helper() {
        let mut samdos = vec![0u8; 512];
        samdos[210] = 0;
        assert_eq!(identify_mgt_dos(&samdos), "SAMDOS");

        samdos[210] = 255;
        assert_eq!(identify_mgt_dos(&samdos), "SAMDOS");

        let mut masterdos = vec![0u8; 512];
        masterdos[210] = 0x42;
        assert_eq!(identify_mgt_dos(&masterdos), "MasterDOS");

        let mut bdos = vec![0u8; 512];
        bdos[232..236].copy_from_slice(b"BDOS");
        assert_eq!(identify_mgt_dos(&bdos), "BDOS");
    }

    // --- Helpers ---

    #[test]
    fn test_mod_checksum() {
        assert_eq!(mod_checksum(&[1, 2, 3], 256), 6);
        assert_eq!(mod_checksum(&[100, 100, 100], 256), 44);
        assert_eq!(mod_checksum(&[0xFF, 0xFF, 0xFF], 256), 253);
    }
}
