//! Disk (side) data structures
use crate::image::track::Track;

/// A disk side containing multiple tracks
#[derive(Debug, Clone)]
pub struct Disk {
    /// Side number (0 or 1)
    pub side_number: u8,
    /// Tracks on this disk side
    tracks: Vec<Track>,
}

impl Disk {
    /// Create a new disk side
    pub fn new(side_number: u8) -> Self {
        Self {
            side_number,
            tracks: Vec::new(),
        }
    }

    /// Create a new disk side with preallocated tracks
    pub fn with_capacity(side_number: u8, num_tracks: usize) -> Self {
        Self {
            side_number,
            tracks: Vec::with_capacity(num_tracks),
        }
    }

    /// Add a track to this disk
    pub fn add_track(&mut self, track: Track) {
        self.tracks.push(track);
    }

    /// Get a reference to all tracks
    pub fn tracks(&self) -> &[Track] {
        &self.tracks
    }

    /// Get a mutable reference to all tracks
    pub fn tracks_mut(&mut self) -> &mut [Track] {
        &mut self.tracks
    }

    /// Get a track by its track number
    pub fn get_track(&self, track_number: u8) -> Option<&Track> {
        self.tracks.get(track_number as usize)
    }

    /// Get a mutable reference to a track by its track number
    pub fn get_track_mut(&mut self, track_number: u8) -> Option<&mut Track> {
        self.tracks.get_mut(track_number as usize)
    }

    /// Get the number of tracks on this disk
    pub fn track_count(&self) -> usize {
        self.tracks.len()
    }

    /// Check if this disk has any tracks
    pub fn is_empty(&self) -> bool {
        self.tracks.is_empty()
    }

    /// Get the total size of all track data in bytes
    pub fn total_size(&self) -> usize {
        self.tracks.iter().map(|t| t.total_data_size()).sum()
    }

    /// Get the total size in kilobytes
    pub fn total_size_kb(&self) -> usize {
        self.total_size() / 1024
    }

    /// Clear all tracks from this disk
    pub fn clear(&mut self) {
        self.tracks.clear();
    }

    /// Reserve space for the specified number of tracks
    pub fn reserve(&mut self, additional: usize) {
        self.tracks.reserve(additional);
    }

    /// Ensure this disk has at least the specified number of tracks
    /// Creates empty tracks if necessary
    pub fn ensure_track_count(&mut self, num_tracks: usize) {
        while self.tracks.len() < num_tracks {
            let track_number = self.tracks.len() as u8;
            self.tracks.push(Track::new(track_number, self.side_number));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::image::sector::{Sector, SectorId};

    #[test]
    fn test_new_disk() {
        let disk = Disk::new(0);
        assert_eq!(disk.side_number, 0);
        assert_eq!(disk.track_count(), 0);
        assert!(disk.is_empty());
    }

    #[test]
    fn test_with_capacity() {
        let disk = Disk::with_capacity(1, 40);
        assert_eq!(disk.side_number, 1);
        assert_eq!(disk.track_count(), 0);
        assert!(disk.tracks.capacity() >= 40);
    }

    #[test]
    fn test_add_track() {
        let mut disk = Disk::new(0);
        let track = Track::new(0, 0);

        disk.add_track(track);

        assert_eq!(disk.track_count(), 1);
        assert!(!disk.is_empty());
    }

    #[test]
    fn test_get_track() {
        let mut disk = Disk::new(0);

        for i in 0..5 {
            disk.add_track(Track::new(i, 0));
        }

        assert_eq!(disk.track_count(), 5);

        let track = disk.get_track(2);
        assert!(track.is_some());
        assert_eq!(track.unwrap().track_number, 2);

        let missing = disk.get_track(10);
        assert!(missing.is_none());
    }

    #[test]
    fn test_get_track_mut() {
        let mut disk = Disk::new(0);

        for i in 0..3 {
            disk.add_track(Track::new(i, 0));
        }

        if let Some(track) = disk.get_track_mut(1) {
            let id = SectorId::new(1, 0, 0xC1, 2);
            track.add_sector(Sector::new(id));
        }

        let track = disk.get_track(1).unwrap();
        assert_eq!(track.sector_count(), 1);
    }

    #[test]
    fn test_total_size() {
        let mut disk = Disk::new(0);

        for track_num in 0..3 {
            let mut track = Track::new(track_num, 0);

            for sector_id in 0xC1..=0xC9 {
                let id = SectorId::new(track_num, 0, sector_id, 2); // 512 bytes each
                track.add_sector(Sector::new(id));
            }

            disk.add_track(track);
        }

        // 3 tracks * 9 sectors * 512 bytes = 13,824 bytes
        assert_eq!(disk.total_size(), 3 * 9 * 512);
        assert_eq!(disk.total_size_kb(), 13); // Rounded down
    }

    #[test]
    fn test_clear() {
        let mut disk = Disk::new(0);

        for i in 0..5 {
            disk.add_track(Track::new(i, 0));
        }

        assert_eq!(disk.track_count(), 5);

        disk.clear();

        assert_eq!(disk.track_count(), 0);
        assert!(disk.is_empty());
    }

    #[test]
    fn test_ensure_track_count() {
        let mut disk = Disk::new(0);

        assert_eq!(disk.track_count(), 0);

        disk.ensure_track_count(5);

        assert_eq!(disk.track_count(), 5);

        // Verify track numbers are set correctly
        for i in 0..5 {
            let track = disk.get_track(i as u8).unwrap();
            assert_eq!(track.track_number, i as u8);
            assert_eq!(track.side_number, 0);
        }

        // Ensure it doesn't shrink
        disk.ensure_track_count(3);
        assert_eq!(disk.track_count(), 5);
    }
}
