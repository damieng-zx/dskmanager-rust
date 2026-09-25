/// Disk format specifications and presets

/// Disk format specification
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FormatSpec {
    /// Number of sides (1 or 2)
    pub num_sides: u8,
    /// Number of tracks per side
    pub num_tracks: u8,
    /// Sectors per track
    pub sectors_per_track: u8,
    /// Sector size in bytes
    pub sector_size: u16,
    /// First sector ID (usually 0x01, 0x41, or 0xC1)
    pub first_sector_id: u8,
    /// GAP#3 length
    pub gap3_length: u8,
    /// Filler byte for formatting
    pub filler_byte: u8,
    /// Interleave factor (1 = no interleave)
    pub interleave: u8,
    /// Side arrangement mode
    pub side_mode: SideMode,
}

/// Side arrangement mode for double-sided disks
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SideMode {
    /// Single-sided
    SingleSide,
    /// Tracks alternate: side 0 track 0, side 1 track 0, side 0 track 1, side 1 track 1, ...
    Alternate,
    /// Tracks successive: side 0 tracks 0-N, side 1 tracks 0-N
    Successive,
}

impl FormatSpec {
    /// Create a new format specification
    pub fn new(
        num_sides: u8,
        num_tracks: u8,
        sectors_per_track: u8,
        sector_size: u16,
    ) -> Self {
        Self {
            num_sides,
            num_tracks,
            sectors_per_track,
            sector_size,
            first_sector_id: 0xC1,
            gap3_length: 0x4E,
            filler_byte: 0xE5,
            interleave: 1,
            side_mode: if num_sides == 1 {
                SideMode::SingleSide
            } else {
                SideMode::Alternate
            },
        }
    }

    /// Amstrad CPC System format (40 tracks, 9 sectors, 512 bytes)
    pub fn amstrad_system() -> Self {
        Self {
            num_sides: 1,
            num_tracks: 40,
            sectors_per_track: 9,
            sector_size: 512,
            first_sector_id: 0x41,
            gap3_length: 0x4E,
            filler_byte: 0xE5,
            interleave: 1,
            side_mode: SideMode::SingleSide,
        }
    }

    /// Amstrad CPC Data format (40 tracks, 9 sectors, 512 bytes)
    pub fn amstrad_data() -> Self {
        Self {
            num_sides: 1,
            num_tracks: 40,
            sectors_per_track: 9,
            sector_size: 512,
            first_sector_id: 0xC1,
            gap3_length: 0x4E,
            filler_byte: 0xE5,
            interleave: 1,
            side_mode: SideMode::SingleSide,
        }
    }

    /// Amstrad CPC Data Double-Sided format (40 tracks, 9 sectors, 512 bytes, 2 sides)
    pub fn amstrad_data_ds() -> Self {
        Self {
            num_sides: 2,
            num_tracks: 40,
            sectors_per_track: 9,
            sector_size: 512,
            first_sector_id: 0xC1,
            gap3_length: 0x4E,
            filler_byte: 0xE5,
            interleave: 1,
            side_mode: SideMode::Alternate,
        }
    }

    /// Spectrum +3 format (40 tracks, 9 sectors, 512 bytes)
    pub fn spectrum_plus3() -> Self {
        Self {
            num_sides: 1,
            num_tracks: 40,
            sectors_per_track: 9,
            sector_size: 512,
            first_sector_id: 0x01,
            gap3_length: 0x2A,
            filler_byte: 0xE5,
            interleave: 1,
            side_mode: SideMode::SingleSide,
        }
    }

    /// Spectrum +3 Double-Sided format (40 tracks, 9 sectors, 512 bytes, 2 sides)
    pub fn spectrum_plus3_ds() -> Self {
        Self {
            num_sides: 2,
            num_tracks: 40,
            sectors_per_track: 9,
            sector_size: 512,
            first_sector_id: 0x01,
            gap3_length: 0x2A,
            filler_byte: 0xE5,
            interleave: 1,
            side_mode: SideMode::Alternate,
        }
    }

    /// Amstrad PCW Single-Sided Single Density format (40 tracks, 9 sectors, 512 bytes)
    pub fn pcw_ssdd() -> Self {
        Self {
            num_sides: 1,
            num_tracks: 40,
            sectors_per_track: 9,
            sector_size: 512,
            first_sector_id: 0x01,
            gap3_length: 0x2A,
            filler_byte: 0xE5,
            interleave: 1,
            side_mode: SideMode::SingleSide,
        }
    }

    /// Amstrad PCW Double-Sided Single Density format (40 tracks, 9 sectors, 512 bytes, 2 sides)
    pub fn pcw_dsdd() -> Self {
        Self {
            num_sides: 2,
            num_tracks: 40,
            sectors_per_track: 9,
            sector_size: 512,
            first_sector_id: 0x01,
            gap3_length: 0x2A,
            filler_byte: 0xE5,
            interleave: 1,
            side_mode: SideMode::Successive,
        }
    }

    /// IBM PC 360K format (40 tracks, 9 sectors, 512 bytes, 2 sides)
    pub fn ibm_pc_360k() -> Self {
        Self {
            num_sides: 2,
            num_tracks: 40,
            sectors_per_track: 9,
            sector_size: 512,
            first_sector_id: 0x01,
            gap3_length: 0x50,
            filler_byte: 0xF6,
            interleave: 1,
            side_mode: SideMode::Alternate,
        }
    }

    /// IBM PC 720K format (80 tracks, 9 sectors, 512 bytes, 2 sides)
    pub fn ibm_pc_720k() -> Self {
        Self {
            num_sides: 2,
            num_tracks: 80,
            sectors_per_track: 9,
            sector_size: 512,
            first_sector_id: 0x01,
            gap3_length: 0x50,
            filler_byte: 0xF6,
            interleave: 1,
            side_mode: SideMode::Alternate,
        }
    }

    /// TR-DOS format (80 tracks, 16 sectors, 256 bytes, single-sided)
    pub fn trdos() -> Self {
        Self {
            num_sides: 1,
            num_tracks: 80,
            sectors_per_track: 16,
            sector_size: 256,
            first_sector_id: 0x01,
            gap3_length: 0x1B,
            filler_byte: 0x00,
            interleave: 1,
            side_mode: SideMode::SingleSide,
        }
    }

    /// Calculate total disk capacity in bytes
    pub fn total_capacity(&self) -> usize {
        self.num_sides as usize
            * self.num_tracks as usize
            * self.sectors_per_track as usize
            * self.sector_size as usize
    }


    /// Set the interleave factor
    pub fn with_interleave(mut self, interleave: u8) -> Self {
        self.interleave = interleave;
        self
    }

    /// Set the side mode
    pub fn with_side_mode(mut self, side_mode: SideMode) -> Self {
        self.side_mode = side_mode;
        self
    }

    /// Set the first sector ID
    pub fn with_first_sector_id(mut self, first_sector_id: u8) -> Self {
        self.first_sector_id = first_sector_id;
        self
    }

    /// Set the filler byte
    pub fn with_filler_byte(mut self, filler_byte: u8) -> Self {
        self.filler_byte = filler_byte;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_amstrad_system_capacity() {
        let spec = FormatSpec::amstrad_system();
        assert_eq!(spec.total_capacity(), 40 * 9 * 512);
        assert_eq!(spec.total_capacity() / 1024, 180);
        assert_eq!(spec.first_sector_id, 0x41);
        let image = crate::image::DiskImage::create(spec).unwrap();
        assert!(image.read_sector(0, 0, 0x41).is_ok());
        assert!(crate::format::DiskSpecification::identify(&image).format.contains("system"));
    }

    #[test]
    fn test_amstrad_data_ds_capacity() {
        let spec = FormatSpec::amstrad_data_ds();
        assert_eq!(spec.total_capacity(), 2 * 40 * 9 * 512);
        assert_eq!(spec.total_capacity() / 1024, 360);
    }

    #[test]
    fn test_spectrum_plus3() {
        let spec = FormatSpec::spectrum_plus3();
        assert_eq!(spec.num_sides, 1);
        assert_eq!(spec.num_tracks, 40);
        assert_eq!(spec.first_sector_id, 0x01);
    }

    #[test]
    fn test_ibm_pc_360k() {
        let spec = FormatSpec::ibm_pc_360k();
        assert_eq!(spec.total_capacity() / 1024, 360);
        assert_eq!(spec.side_mode, SideMode::Alternate);
    }

    #[test]
    fn test_with_methods() {
        let spec = FormatSpec::amstrad_system()
            .with_interleave(2)
            .with_first_sector_id(0x01)
            .with_filler_byte(0x00);

        assert_eq!(spec.interleave, 2);
        assert_eq!(spec.first_sector_id, 0x01);
        assert_eq!(spec.filler_byte, 0x00);
    }
}
