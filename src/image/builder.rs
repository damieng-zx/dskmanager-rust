//! Builder for creating DSK images
use crate::error::Result;
use crate::format::{DiskImageFormat, FormatSpec};
use crate::image::{Disk, DiskImage, Sector, SectorId, Track};

/// Builder for constructing DSK images
pub struct DiskImageBuilder {
    format: DiskImageFormat,
    spec: FormatSpec,
}

impl DiskImageBuilder {
    /// Create a new builder with default values
    pub fn new() -> Self {
        Self {
            format: DiskImageFormat::StandardDSK,
            spec: FormatSpec::amstrad_data(),
        }
    }

    /// Set the DSK format
    pub fn format(mut self, format: DiskImageFormat) -> Self {
        self.format = format;
        self
    }

    /// Set the format specification
    pub fn spec(mut self, spec: FormatSpec) -> Self {
        self.spec = spec;
        self
    }

    /// Set the number of sides
    pub fn num_sides(mut self, num_sides: u8) -> Self {
        self.spec.num_sides = num_sides;
        self
    }

    /// Set the number of tracks
    pub fn num_tracks(mut self, num_tracks: u8) -> Self {
        self.spec.num_tracks = num_tracks;
        self
    }

    /// Set sectors per track
    pub fn sectors_per_track(mut self, sectors_per_track: u8) -> Self {
        self.spec.sectors_per_track = sectors_per_track;
        self
    }

    /// Set sector size
    pub fn sector_size(mut self, sector_size: u16) -> Self {
        self.spec.sector_size = sector_size;
        self
    }

    /// Build the DSK image with the specified configuration
    pub fn build(self) -> Result<DiskImage> {
        let mut disks = Vec::with_capacity(self.spec.num_sides as usize);

        // Create disks (one per side)
        for side in 0..self.spec.num_sides {
            let mut disk = Disk::with_capacity(side, self.spec.num_tracks as usize);

            // Create tracks
            for track_num in 0..self.spec.num_tracks {
                let mut track = Track::new(track_num, side);
                track.gap3_length = self.spec.gap3_length;
                track.filler_byte = self.spec.filler_byte;

                // Create sectors
                for sector_idx in 0..self.spec.sectors_per_track {
                    let sector_id = self.spec.first_sector_id + sector_idx;

                    // Determine size code from sector size
                    let size_code = match self.spec.sector_size {
                        128 => 0,
                        256 => 1,
                        512 => 2,
                        1024 => 3,
                        2048 => 4,
                        4096 => 5,
                        8192 => 6,
                        _ => 2, // Default to 512
                    };

                    let id = SectorId::new(track_num, side, sector_id, size_code);
                    let mut sector = Sector::new(id);
                    sector.fill(self.spec.filler_byte);

                    track.add_sector(sector);
                }

                disk.add_track(track);
            }

            disks.push(disk);
        }

        Ok(DiskImage {
            format: self.format,
            spec: self.spec,
            disks,
            changed: true, // Newly created image is considered changed
            filename: None,
            warnings: Vec::new(),
        })
    }
}

impl Default for DiskImageBuilder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_builder_default() {
        let image = DiskImageBuilder::new().build().unwrap();

        assert_eq!(image.format(), DiskImageFormat::StandardDSK);
        assert_eq!(image.spec().num_sides, 1);
        assert_eq!(image.spec().num_tracks, 40);
    }

    #[test]
    fn test_builder_custom() {
        let image = DiskImageBuilder::new()
            .format(DiskImageFormat::ExtendedDSK)
            .num_sides(2)
            .num_tracks(80)
            .sectors_per_track(9)
            .sector_size(512)
            .build()
            .unwrap();

        assert_eq!(image.format(), DiskImageFormat::ExtendedDSK);
        assert_eq!(image.spec().num_sides, 2);
        assert_eq!(image.spec().num_tracks, 80);
        assert_eq!(image.disk_count(), 2);
    }

    #[test]
    fn test_builder_with_spec() {
        let spec = FormatSpec::spectrum_plus3();
        let image = DiskImageBuilder::new().spec(spec.clone()).build().unwrap();

        assert_eq!(image.spec().first_sector_id, spec.first_sector_id);
        assert_eq!(image.spec().num_tracks, spec.num_tracks);
    }

    #[test]
    fn test_builder_creates_sectors() {
        let image = DiskImageBuilder::new()
            .num_sides(1)
            .num_tracks(2)
            .sectors_per_track(3)
            .build()
            .unwrap();

        let disk = image.get_disk(0).unwrap();
        assert_eq!(disk.track_count(), 2);

        let track = disk.get_track(0).unwrap();
        assert_eq!(track.sector_count(), 3);
    }
}
