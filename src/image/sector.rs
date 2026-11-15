//! Sector data structures
use crate::fdc::{FdcStatus1, FdcStatus2};
use crate::format::constants::fdc_size_to_bytes;

/// Sector ID (CHRN) - addressing information for a sector
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SectorId {
    /// C - Cylinder/Track number
    pub track: u8,
    /// H - Head/Side number
    pub side: u8,
    /// R - Sector ID/Record number
    pub sector: u8,
    /// N - Size code (0=128, 1=256, 2=512, 3=1024, 4=2048, etc.)
    pub size_code: u8,
}

impl SectorId {
    /// Create a new sector ID
    pub fn new(track: u8, side: u8, sector: u8, size_code: u8) -> Self {
        Self {
            track,
            side,
            sector,
            size_code,
        }
    }

    /// Get the advertised sector size in bytes based on size code
    pub fn size_bytes(&self) -> usize {
        fdc_size_to_bytes(self.size_code)
    }
}

/// Sector status classification
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SectorStatus {
    /// Unformatted - data size is 0
    Unformatted,
    /// Formatted but contains only the track filler byte
    FormattedFiller,
    /// Formatted but contains only a single repeated byte (not the filler)
    FormattedOddFiller,
    /// Formatted and contains data (in use)
    FormattedInUse,
}

impl std::fmt::Display for SectorStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SectorStatus::Unformatted => write!(f, "Unformatted"),
            SectorStatus::FormattedFiller => write!(f, "Filler"),
            SectorStatus::FormattedOddFiller => write!(f, "Odd Filler"),
            SectorStatus::FormattedInUse => write!(f, "In Use"),
        }
    }
}

/// A disk sector containing data and metadata
#[derive(Debug, Clone)]
pub struct Sector {
    /// Sector addressing information (CHRN)
    pub id: SectorId,
    /// FDC Status Register 1
    pub fdc_status1: FdcStatus1,
    /// FDC Status Register 2
    pub fdc_status2: FdcStatus2,
    /// Sector data
    data: Vec<u8>,
}

impl Sector {
    /// Create a new sector
    pub fn new(id: SectorId) -> Self {
        let size = id.size_bytes();
        Self {
            id,
            fdc_status1: FdcStatus1::new(0),
            fdc_status2: FdcStatus2::new(0),
            data: vec![0xE5; size], // Default CP/M filler byte
        }
    }

    /// Create a new sector with specific data
    pub fn with_data(id: SectorId, data: Vec<u8>) -> Self {
        Self {
            id,
            fdc_status1: FdcStatus1::new(0),
            fdc_status2: FdcStatus2::new(0),
            data,
        }
    }

    /// Create a new sector with FDC status
    pub fn with_status(
        id: SectorId,
        fdc_status1: FdcStatus1,
        fdc_status2: FdcStatus2,
        data: Vec<u8>,
    ) -> Self {
        Self {
            id,
            fdc_status1,
            fdc_status2,
            data,
        }
    }

    /// Get a reference to the sector data
    pub fn data(&self) -> &[u8] {
        &self.data
    }

    /// Get a mutable reference to the sector data
    pub fn data_mut(&mut self) -> &mut Vec<u8> {
        &mut self.data
    }

    /// Set the sector data
    pub fn set_data(&mut self, data: Vec<u8>) {
        self.data = data;
    }

    /// Check if this sector has any FDC errors
    pub fn has_error(&self) -> bool {
        self.fdc_status1.has_error() || self.fdc_status2.has_error()
    }

    /// Check if this sector is marked as deleted data
    pub fn is_deleted(&self) -> bool {
        self.fdc_status2.is_deleted()
    }

    /// Get the advertised size from the size code
    pub fn advertised_size(&self) -> usize {
        self.id.size_bytes()
    }

    /// Get the actual data size
    pub fn actual_size(&self) -> usize {
        self.data.len()
    }

    /// Actual data length as stored in the sector info block.
    ///
    /// This is always the buffer length: keeping a separate field allowed the
    /// two to disagree, which produced images whose size table did not match
    /// their contents.
    pub fn data_length(&self) -> u16 {
        self.data.len() as u16
    }

    /// Check if the actual size matches the advertised size
    pub fn has_size_mismatch(&self) -> bool {
        self.actual_size() != self.advertised_size()
    }

    /// Analyze the sector status based on data content
    pub fn status(&self, filler_byte: u8) -> SectorStatus {
        if self.data.is_empty() {
            return SectorStatus::Unformatted;
        }

        // Check if all bytes are the same
        let first_byte = self.data[0];
        let all_same = self.data.iter().all(|&b| b == first_byte);
        
        if all_same {
            if first_byte == filler_byte {
                SectorStatus::FormattedFiller
            } else {
                SectorStatus::FormattedOddFiller
            }
        } else {
            SectorStatus::FormattedInUse
        }
    }

    /// Fill the sector with a specific byte value
    pub fn fill(&mut self, byte: u8) {
        self.data.fill(byte);
    }

    /// Resize the sector data
    pub fn resize(&mut self, new_size: usize, fill_byte: u8) {
        self.data.resize(new_size, fill_byte);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sector_id_size() {
        let id = SectorId::new(0, 0, 0xC1, 2);
        assert_eq!(id.size_bytes(), 512);

        let id2 = SectorId::new(0, 0, 1, 3);
        assert_eq!(id2.size_bytes(), 1024);
    }

    #[test]
    fn test_new_sector() {
        let id = SectorId::new(0, 0, 0xC1, 2);
        let sector = Sector::new(id);

        assert_eq!(sector.data().len(), 512);
        assert_eq!(sector.actual_size(), 512);
        assert_eq!(sector.advertised_size(), 512);
        assert!(!sector.has_size_mismatch());
        assert!(!sector.has_error());
    }

    #[test]
    fn test_sector_with_data() {
        let id = SectorId::new(0, 0, 1, 1);
        let data = vec![0x42; 256];
        let sector = Sector::with_data(id, data);

        assert_eq!(sector.data().len(), 256);
        assert!(sector.data().iter().all(|&b| b == 0x42));
    }

    #[test]
    fn test_sector_status_blank() {
        let id = SectorId::new(0, 0, 1, 2);
        let sector = Sector::new(id);
        assert_eq!(sector.status(0xE5), SectorStatus::FormattedFiller);
    }

    #[test]
    fn test_sector_status_in_use() {
        let id = SectorId::new(0, 0, 1, 2);
        let data = vec![0x00, 0x01, 0x02, 0x03, 0x04, 0x05];
        let sector = Sector::with_data(id, data);
        assert_eq!(sector.status(0xE5), SectorStatus::FormattedInUse);
    }

    #[test]
    fn test_sector_status_odd_filler() {
        let id = SectorId::new(0, 0, 1, 2);
        let data = vec![0xFF; 512]; // All 0xFF, but filler is 0xE5
        let sector = Sector::with_data(id, data);
        assert_eq!(sector.status(0xE5), SectorStatus::FormattedOddFiller);
    }

    #[test]
    fn test_sector_fill() {
        let id = SectorId::new(0, 0, 1, 1);
        let mut sector = Sector::new(id);
        sector.fill(0xFF);
        assert!(sector.data().iter().all(|&b| b == 0xFF));
    }

    #[test]
    fn test_sector_resize() {
        let id = SectorId::new(0, 0, 1, 2);
        let mut sector = Sector::new(id);
        sector.resize(128, 0x00);

        assert_eq!(sector.actual_size(), 128);
        assert_eq!(sector.data().len(), 128);
    }

    #[test]
    fn test_data_mut_length_cannot_desynchronise() {
        // data_length used to be a separate field, so mutating the buffer through
        // data_mut() left it stale and the DSK writer silently dropped the extra
        // bytes. It is now derived from the buffer, so the two cannot disagree.
        use crate::format::DiskImageFormat;

        let mut image = crate::image::DiskImage::builder()
            .format(DiskImageFormat::ExtendedDSK)
            .num_tracks(1)
            .sectors_per_track(1)
            .build()
            .unwrap();
        {
            let sector = image
                .get_disk_mut(0)
                .unwrap()
                .get_track_mut(0)
                .unwrap()
                .get_sector_mut(0xC1)
                .unwrap();
            sector.data_mut().extend_from_slice(&[0x99; 512]);
            // The length follows the buffer instead of going stale.
            assert_eq!(sector.data_length(), 1024);
            assert_eq!(sector.actual_size(), 1024);
        }

        // A sector larger than its size code allows is now reported rather than
        // written with its tail dropped.
        let path = std::env::temp_dir().join(format!("dskmgr_desync_{}.dsk", std::process::id()));
        let saved = image.save(&path);
        if path.exists() {
            std::fs::remove_file(&path).ok();
        }
        assert!(
            saved.is_err(),
            "a 1024-byte buffer in a 512-byte sector must be rejected"
        );
    }

    #[test]
    fn test_sector_errors() {
        let id = SectorId::new(0, 0, 1, 2);
        let mut sector = Sector::new(id);

        assert!(!sector.has_error());

        sector.fdc_status1 = FdcStatus1::new(FdcStatus1::DE);
        assert!(sector.has_error());
    }

    #[test]
    fn test_sector_deleted() {
        let id = SectorId::new(0, 0, 1, 2);
        let mut sector = Sector::new(id);

        assert!(!sector.is_deleted());

        sector.fdc_status2 = FdcStatus2::new(FdcStatus2::CM);
        assert!(sector.is_deleted());
    }
}
