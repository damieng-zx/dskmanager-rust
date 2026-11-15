//! Track data structures
use crate::image::sector::Sector;
use std::collections::HashMap;

/// Recording mode for the track
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecordingMode {
    /// Unknown recording mode
    Unknown,
    /// FM (Frequency Modulation) - single density
    FM,
    /// MFM (Modified Frequency Modulation) - double density
    MFM,
}

impl From<u8> for RecordingMode {
    fn from(value: u8) -> Self {
        match value {
            1 => RecordingMode::FM,
            2 => RecordingMode::MFM,
            _ => RecordingMode::Unknown,
        }
    }
}

impl From<RecordingMode> for u8 {
    fn from(mode: RecordingMode) -> Self {
        match mode {
            RecordingMode::Unknown => 0,
            RecordingMode::FM => 1,
            RecordingMode::MFM => 2,
        }
    }
}

/// Data rate for the track
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DataRate {
    /// Unknown data rate
    Unknown,
    /// Single/Double Density
    SingleDouble,
    /// High Density
    High,
    /// Extended Density
    Extended,
}

impl From<u8> for DataRate {
    fn from(value: u8) -> Self {
        match value {
            1 => DataRate::SingleDouble,
            2 => DataRate::High,
            3 => DataRate::Extended,
            _ => DataRate::Unknown,
        }
    }
}

impl From<DataRate> for u8 {
    fn from(rate: DataRate) -> Self {
        match rate {
            DataRate::Unknown => 0,
            DataRate::SingleDouble => 1,
            DataRate::High => 2,
            DataRate::Extended => 3,
        }
    }
}

/// A disk track containing multiple sectors
#[derive(Debug, Clone)]
pub struct Track {
    /// Physical track number
    pub track_number: u8,
    /// Physical side number (0 or 1)
    pub side_number: u8,
    /// GAP#3 length
    pub gap3_length: u8,
    /// Filler byte used for formatting
    pub filler_byte: u8,
    /// Data rate (V5 extension)
    pub data_rate: DataRate,
    /// Recording mode (V5 extension)
    pub recording_mode: RecordingMode,
    /// Sectors in this track
    sectors: Vec<Sector>,
    /// Map from sector ID to index in sectors vector for fast lookup
    sector_map: HashMap<u8, usize>,
}

impl Track {
    /// Create a new track
    pub fn new(track_number: u8, side_number: u8) -> Self {
        Self {
            track_number,
            side_number,
            gap3_length: 0x4E,
            filler_byte: 0xE5,
            data_rate: DataRate::Unknown,
            recording_mode: RecordingMode::Unknown,
            sectors: Vec::new(),
            sector_map: HashMap::new(),
        }
    }

    /// Add a sector to this track
    pub fn add_sector(&mut self, sector: Sector) {
        let sector_id = sector.id.sector;
        let index = self.sectors.len();
        self.sectors.push(sector);
        self.sector_map.insert(sector_id, index);
    }

    /// Get a reference to all sectors
    pub fn sectors(&self) -> &[Sector] {
        &self.sectors
    }

    /// Get a mutable reference to all sectors
    pub fn sectors_mut(&mut self) -> &mut [Sector] {
        &mut self.sectors
    }

    /// Get a sector by its ID
    pub fn get_sector(&self, sector_id: u8) -> Option<&Sector> {
        self.sector_map
            .get(&sector_id)
            .and_then(|&idx| self.sectors.get(idx))
    }

    /// Get a mutable reference to a sector by its ID
    pub fn get_sector_mut(&mut self, sector_id: u8) -> Option<&mut Sector> {
        self.sector_map
            .get(&sector_id)
            .and_then(|&idx| self.sectors.get_mut(idx))
    }

    /// Get a sector by its position index
    pub fn get_sector_by_index(&self, index: usize) -> Option<&Sector> {
        self.sectors.get(index)
    }

    /// Get a mutable reference to a sector by its position index
    pub fn get_sector_by_index_mut(&mut self, index: usize) -> Option<&mut Sector> {
        self.sectors.get_mut(index)
    }

    /// Get the number of sectors in this track
    pub fn sector_count(&self) -> usize {
        self.sectors.len()
    }

    /// Check if this track has any sectors
    pub fn is_empty(&self) -> bool {
        self.sectors.is_empty()
    }

    /// Get the total data size of all sectors in bytes
    pub fn total_data_size(&self) -> usize {
        self.sectors.iter().map(|s| s.actual_size()).sum()
    }

    /// Check if all sectors have the same size
    pub fn has_uniform_sector_size(&self) -> bool {
        if self.sectors.is_empty() {
            return true;
        }

        let first_size = self.sectors[0].advertised_size();
        self.sectors
            .iter()
            .all(|s| s.advertised_size() == first_size)
    }

    /// Get the sector size if all sectors are uniform, None otherwise
    pub fn uniform_sector_size(&self) -> Option<usize> {
        if self.has_uniform_sector_size() && !self.sectors.is_empty() {
            Some(self.sectors[0].advertised_size())
        } else {
            None
        }
    }

    /// Clear all sectors from this track
    pub fn clear(&mut self) {
        self.sectors.clear();
        self.sector_map.clear();
    }

    /// Get list of all sector IDs in this track
    pub fn sector_ids(&self) -> Vec<u8> {
        self.sectors.iter().map(|s| s.id.sector).collect()
    }

    /// Check if this track contains a sector with the given ID
    pub fn has_sector(&self, sector_id: u8) -> bool {
        self.sector_map.contains_key(&sector_id)
    }

    /// Read sectors in logical order (sorted by sector ID)
    ///
    /// Appends sector data to the provided buffer, reading sectors
    /// in ascending order by their sector ID (as a real FDC would).
    pub fn read_logical(&self, data: &mut Vec<u8>) {
        // Sort by ID but read the physical sectors, not by looking IDs back up:
        // copy-protected tracks repeat IDs and a map lookup would return the
        // last duplicate for every occurrence.
        let mut sectors: Vec<&Sector> = self.sectors.iter().collect();
        sectors.sort_by_key(|sector| sector.id.sector);

        for sector in sectors {
            data.extend_from_slice(sector.data());
        }
    }

    /// Read sectors in logical order, returning a new Vec
    ///
    /// Convenience method that returns sector data as a new Vec
    /// instead of appending to an existing buffer.
    pub fn read_logical_to_vec(&self) -> Vec<u8> {
        let mut data = Vec::new();
        self.read_logical(&mut data);
        data
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::image::sector::SectorId;

    #[test]
    fn test_new_track() {
        let track = Track::new(0, 0);
        assert_eq!(track.track_number, 0);
        assert_eq!(track.side_number, 0);
        assert_eq!(track.sector_count(), 0);
        assert!(track.is_empty());
    }

    #[test]
    fn test_add_sector() {
        let mut track = Track::new(0, 0);
        let id = SectorId::new(0, 0, 0xC1, 2);
        let sector = Sector::new(id);

        track.add_sector(sector);

        assert_eq!(track.sector_count(), 1);
        assert!(!track.is_empty());
        assert!(track.has_sector(0xC1));
    }

    #[test]
    fn test_get_sector() {
        let mut track = Track::new(0, 0);

        for i in 0xC1..=0xC9 {
            let id = SectorId::new(0, 0, i, 2);
            track.add_sector(Sector::new(id));
        }

        assert_eq!(track.sector_count(), 9);

        let sector = track.get_sector(0xC5);
        assert!(sector.is_some());
        assert_eq!(sector.unwrap().id.sector, 0xC5);

        let missing = track.get_sector(0xFF);
        assert!(missing.is_none());
    }

    #[test]
    fn test_get_sector_by_index() {
        let mut track = Track::new(0, 0);

        for i in 0xC1..=0xC9 {
            let id = SectorId::new(0, 0, i, 2);
            track.add_sector(Sector::new(id));
        }

        let sector = track.get_sector_by_index(0);
        assert!(sector.is_some());
        assert_eq!(sector.unwrap().id.sector, 0xC1);

        let sector = track.get_sector_by_index(8);
        assert!(sector.is_some());
        assert_eq!(sector.unwrap().id.sector, 0xC9);
    }

    #[test]
    fn test_sector_ids() {
        let mut track = Track::new(0, 0);

        for i in 1..=5 {
            let id = SectorId::new(0, 0, i, 2);
            track.add_sector(Sector::new(id));
        }

        let ids = track.sector_ids();
        assert_eq!(ids.len(), 5);
        assert_eq!(ids, vec![1, 2, 3, 4, 5]);
    }

    #[test]
    fn test_total_data_size() {
        let mut track = Track::new(0, 0);

        for i in 1..=4 {
            let id = SectorId::new(0, 0, i, 2); // 512 bytes each
            track.add_sector(Sector::new(id));
        }

        assert_eq!(track.total_data_size(), 4 * 512);
    }

    #[test]
    fn test_uniform_sector_size() {
        let mut track = Track::new(0, 0);

        for i in 1..=3 {
            let id = SectorId::new(0, 0, i, 2); // All 512 bytes
            track.add_sector(Sector::new(id));
        }

        assert!(track.has_uniform_sector_size());
        assert_eq!(track.uniform_sector_size(), Some(512));
    }

    #[test]
    fn test_non_uniform_sector_size() {
        let mut track = Track::new(0, 0);

        let id1 = SectorId::new(0, 0, 1, 2); // 512 bytes
        track.add_sector(Sector::new(id1));

        let id2 = SectorId::new(0, 0, 2, 3); // 1024 bytes
        track.add_sector(Sector::new(id2));

        assert!(!track.has_uniform_sector_size());
        assert_eq!(track.uniform_sector_size(), None);
    }

    #[test]
    fn test_clear() {
        let mut track = Track::new(0, 0);

        for i in 1..=5 {
            let id = SectorId::new(0, 0, i, 2);
            track.add_sector(Sector::new(id));
        }

        assert_eq!(track.sector_count(), 5);

        track.clear();

        assert_eq!(track.sector_count(), 0);
        assert!(track.is_empty());
    }

    #[test]
    fn test_read_logical_keeps_duplicate_sector_ids() {
        // Copy-protected disks deliberately repeat sector IDs. read_logical
        // used to re-look each ID up in the map and emit the last duplicate
        // for every occurrence, dropping the earlier sector's data.
        let mut track = Track::new(0, 0);
        track.add_sector(Sector::with_data(SectorId::new(0, 0, 1, 2), vec![0xAA; 512]));
        track.add_sector(Sector::with_data(SectorId::new(0, 0, 1, 2), vec![0xBB; 512]));
        track.add_sector(Sector::with_data(SectorId::new(0, 0, 2, 2), vec![0xCC; 512]));

        let data = track.read_logical_to_vec();
        assert_eq!(data.len(), 3 * 512);
        assert_eq!(data[..512], [0xAA; 512][..]);
        assert_eq!(data[512..1024], [0xBB; 512][..]);
        assert_eq!(data[1024..], [0xCC; 512][..]);
    }

    #[test]
    fn test_recording_mode_conversion() {
        assert_eq!(RecordingMode::from(1), RecordingMode::FM);
        assert_eq!(RecordingMode::from(2), RecordingMode::MFM);
        assert_eq!(RecordingMode::from(99), RecordingMode::Unknown);

        assert_eq!(u8::from(RecordingMode::FM), 1);
        assert_eq!(u8::from(RecordingMode::MFM), 2);
    }

    #[test]
    fn test_data_rate_conversion() {
        assert_eq!(DataRate::from(1), DataRate::SingleDouble);
        assert_eq!(DataRate::from(2), DataRate::High);
        assert_eq!(DataRate::from(99), DataRate::Unknown);

        assert_eq!(u8::from(DataRate::High), 2);
    }
}
