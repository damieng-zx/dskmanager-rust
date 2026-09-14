/// DSK image data structures

/// Image builder for creating DSK images
pub mod builder;
/// Disk structure
pub mod disk;
/// Sector definition and status
pub mod sector;
/// Track definition and data rate
pub mod track;

pub use builder::DiskImageBuilder;
pub use disk::Disk;
pub use sector::{Sector, SectorId, SectorStatus};
pub use track::{DataRate, RecordingMode, Track};

use crate::error::{DskError, Result};
use crate::filesystem::FileSystemType;
use crate::format::{DiskImageFormat, DiskSpecification, FormatSpec, SideMode};
use std::path::Path;

/// Main DSK image container
#[derive(Debug, Clone)]
pub struct DiskImage {
    /// DSK format type (Standard, Extended, or RawMgt)
    pub(crate) format: DiskImageFormat,
    /// Format specification
    pub(crate) spec: FormatSpec,
    /// Disks (one per side)
    pub(crate) disks: Vec<Disk>,
    /// Has the image been modified?
    pub(crate) changed: bool,
    /// Original filename if loaded from disk
    pub(crate) filename: Option<String>,
    /// Non-fatal issues detected while reading the image (e.g. a recovered
    /// structure). Empty for a cleanly-parsed image.
    pub(crate) warnings: Vec<String>,
}

impl DiskImage {
    /// Open a DSK, MGT, TRD, or TD0 file from disk
    ///
    /// Automatically detects file type based on extension:
    /// - `.mgt` files are read as raw MGT format
    /// - All other extensions are read as DSK format
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self> {
        if crate::io::json::is_json_file(&path) {
            crate::io::json::read_json(path)
        } else if crate::io::is_trd_file(&path) {
            crate::io::read_trd(path)
        } else if crate::io::is_mgt_file(&path) {
            crate::io::read_mgt(path)
        } else if crate::io::is_td0_file(&path) {
            crate::io::read_td0(path)
        } else {
            crate::io::reader::read_dsk(path)
        }
    }

    /// Create a new DSK image with the given specification
    pub fn create(spec: FormatSpec) -> Result<Self> {
        DiskImageBuilder::new().spec(spec).build()
    }

    /// Create a new builder for constructing DSK images
    pub fn builder() -> DiskImageBuilder {
        DiskImageBuilder::new()
    }

    /// Get the format type
    pub fn format(&self) -> DiskImageFormat {
        self.format
    }

    /// Get the default filesystem type based on disk specification
    ///
    /// This checks the disk specification to determine the appropriate filesystem:
    /// - Returns `FileSystemType::Mgt` if the spec is recognised as MGT Sam Coupe
    ///   (including SAMDOS / MasterDOS / BDOS variants)
    /// - Otherwise falls back to the image format's default filesystem
    pub fn default_filesystem(&self) -> FileSystemType {
        let spec = DiskSpecification::identify(self);
        if spec.format.starts_with("MGT Sam Coupe") {
            FileSystemType::Mgt
        } else {
            self.format.default_filesystem()
        }
    }

    /// Get the format specification
    pub fn spec(&self) -> &FormatSpec {
        &self.spec
    }

    /// Get the original filename if loaded from disk
    pub fn filename(&self) -> Option<&str> {
        self.filename.as_deref()
    }

    /// Non-fatal issues detected while reading the image.
    ///
    /// Empty for a cleanly-parsed image. Populated by the reader when it has
    /// to recover from a malformed-but-readable image.
    pub fn warnings(&self) -> &[String] {
        &self.warnings
    }

    /// Get all disks (sides)
    pub fn disks(&self) -> &[Disk] {
        &self.disks
    }

    /// Get a mutable reference to all disks
    pub fn disks_mut(&mut self) -> &mut [Disk] {
        self.changed = true;
        &mut self.disks
    }

    /// Get a disk by side number
    pub fn get_disk(&self, side: u8) -> Option<&Disk> {
        self.disks.get(side as usize)
    }

    /// Get a mutable reference to a disk by side number
    pub fn get_disk_mut(&mut self, side: u8) -> Option<&mut Disk> {
        self.changed = true;
        self.disks.get_mut(side as usize)
    }

    /// Get the number of disk sides
    pub fn disk_count(&self) -> usize {
        self.disks.len()
    }

    /// Read sector data
    pub fn read_sector(&self, side: u8, track: u8, sector_id: u8) -> Result<&[u8]> {
        let disk = self.get_disk(side).ok_or(DskError::InvalidTrack {
            side,
            track,
            max: self.spec.num_sides.saturating_sub(1),
        })?;

        let track_obj = disk.get_track(track).ok_or(DskError::InvalidTrack {
            side,
            track,
            max: self.spec.num_tracks.saturating_sub(1),
        })?;

        let sector = track_obj
            .get_sector(sector_id)
            .ok_or(DskError::InvalidSector {
                side,
                track,
                id: sector_id,
            })?;

        Ok(sector.data())
    }

    /// Write sector data
    pub fn write_sector(&mut self, side: u8, track: u8, sector_id: u8, data: &[u8]) -> Result<()> {
        self.changed = true;

        let max_side = self.spec.num_sides.saturating_sub(1);
        let max_track = self.spec.num_tracks.saturating_sub(1);

        let disk = self.get_disk_mut(side).ok_or(DskError::InvalidTrack {
            side,
            track,
            max: max_side,
        })?;

        let track_obj = disk.get_track_mut(track).ok_or(DskError::InvalidTrack {
            side,
            track,
            max: max_track,
        })?;

        let sector = track_obj
            .get_sector_mut(sector_id)
            .ok_or(DskError::InvalidSector {
                side,
                track,
                id: sector_id,
            })?;

        // Resize sector data if needed
        if data.len() != sector.data().len() {
            sector.resize(data.len(), 0);
        }

        sector.data_mut().copy_from_slice(data);
        Ok(())
    }

    /// Save the DSK image to a file
    pub fn save<P: AsRef<Path>>(&mut self, path: P) -> Result<()> {
        if crate::io::json::is_json_file(&path) {
            crate::io::json::write_json(self, path)?;
        } else {
            crate::io::writer::write_dsk(self, path)?;
        }
        self.changed = false;
        Ok(())
    }

    /// Check if the image has been modified
    pub fn is_changed(&self) -> bool {
        self.changed
    }

    /// Mark the image as unchanged
    pub fn mark_unchanged(&mut self) {
        self.changed = false;
    }

    /// Get the total capacity of the disk in bytes
    pub fn total_capacity(&self) -> usize {
        self.spec.total_capacity()
    }

    /// Read the entire disk in logical order
    ///
    /// This reads the disk as a real FDC would, respecting:
    /// - Sector ID order (sectors sorted by ID, not physical position)
    /// - Side mode (how double-sided disks interleave tracks)
    ///
    /// # Side Modes
    ///
    /// - `SingleSide`: All tracks sequentially from side 0
    /// - `Alternate`: T0S0, T0S1, T1S0, T1S1... (interleaved by track)
    /// - `Successive`: All of side 0, then all of side 1
    ///
    /// # Example
    ///
    /// ```no_run
    /// use dskmanager::DiskImage;
    ///
    /// let image = DiskImage::open("disk.dsk")?;
    /// let data = image.read_logical();
    /// println!("Read {} bytes in logical order", data.len());
    /// # Ok::<(), dskmanager::DskError>(())
    /// ```
    pub fn read_logical(&self) -> Vec<u8> {
        let mut data = Vec::new();

        match self.spec.side_mode {
            SideMode::SingleSide => {
                // Single side: just read all tracks in order
                if let Some(disk) = self.get_disk(0) {
                    for track_num in 0..disk.track_count() {
                        if let Some(track) = disk.get_track(track_num as u8) {
                            track.read_logical(&mut data);
                        }
                    }
                }
            }
            SideMode::Alternate => {
                // Alternating: T0S0, T0S1, T1S0, T1S1, ...
                let max_tracks = self
                    .disks
                    .iter()
                    .map(|d| d.track_count())
                    .max()
                    .unwrap_or(0);

                for track_num in 0..max_tracks {
                    for disk in &self.disks {
                        if let Some(track) = disk.get_track(track_num as u8) {
                            track.read_logical(&mut data);
                        }
                    }
                }
            }
            SideMode::Successive => {
                // Successive: all of side 0, then all of side 1
                for disk in &self.disks {
                    for track_num in 0..disk.track_count() {
                        if let Some(track) = disk.get_track(track_num as u8) {
                            track.read_logical(&mut data);
                        }
                    }
                }
            }
        }

        data
    }

    // Note: with_filesystem methods are not provided due to lifetime complexity.
    // Use CpmFileSystem::from_image(&image) directly instead.
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_image() {
        let spec = FormatSpec::amstrad_system();
        let image = DiskImage::create(spec).unwrap();

        assert_eq!(image.format(), DiskImageFormat::StandardDSK);
        assert_eq!(image.disk_count(), 1);
        assert!(image.is_changed());
    }

    #[test]
    fn test_builder() {
        let image = DiskImage::builder()
            .num_sides(2)
            .num_tracks(40)
            .build()
            .unwrap();

        assert_eq!(image.disk_count(), 2);
    }

    #[test]
    fn test_get_disk() {
        let image = DiskImage::builder().num_sides(2).build().unwrap();

        assert!(image.get_disk(0).is_some());
        assert!(image.get_disk(1).is_some());
        assert!(image.get_disk(2).is_none());
    }

    #[test]
    fn test_read_write_sector() {
        let mut image = DiskImage::builder()
            .num_sides(1)
            .num_tracks(2)
            .sectors_per_track(3)
            .build()
            .unwrap();

        // Write data
        let test_data = vec![0x42; 512];
        image
            .write_sector(0, 0, 0xC1, &test_data)
            .unwrap();

        // Read it back
        let read_data = image.read_sector(0, 0, 0xC1).unwrap();
        assert_eq!(read_data, test_data.as_slice());
        assert!(image.is_changed());
    }

    #[test]
    fn test_read_invalid_sector() {
        let image = DiskImage::builder().build().unwrap();

        let result = image.read_sector(0, 0, 0xFF);
        assert!(result.is_err());
    }

    #[test]
    fn test_write_invalid_track() {
        let mut image = DiskImage::builder().num_tracks(10).build().unwrap();

        let data = vec![0; 512];
        let result = image.write_sector(0, 50, 0xC1, &data);
        assert!(result.is_err());
    }

    #[test]
    fn test_capacity() {
        let image = DiskImage::builder()
            .num_sides(2)
            .num_tracks(40)
            .sectors_per_track(9)
            .sector_size(512)
            .build()
            .unwrap();

        assert_eq!(image.total_capacity(), 2 * 40 * 9 * 512);
        assert_eq!(image.total_capacity() / 1024, 360);
    }
}
