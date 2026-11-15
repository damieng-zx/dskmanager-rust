//! Integration tests for dskmanager

use dskmanager::*;

#[test]
fn test_create_and_save_image() {
    let spec = FormatSpec::amstrad_data();
    let mut image = DiskImage::create(spec).expect("Failed to create image");

    assert_eq!(image.format(), DiskImageFormat::StandardDSK);
    assert_eq!(image.disk_count(), 1);
    assert!(image.is_changed());

    // Test reading an initial sector
    let data = image.read_sector(0, 0, 0xC1).expect("Failed to read sector");
    assert_eq!(data.len(), 512);

    // Test writing a sector
    let test_data = vec![0x42; 512];
    image
        .write_sector(0, 0, 0xC1, &test_data)
        .expect("Failed to write sector");

    // Verify the write
    let read_data = image.read_sector(0, 0, 0xC1).expect("Failed to read sector");
    assert_eq!(read_data, test_data.as_slice());
}

#[test]
fn test_image_builder() {
    let image = DiskImage::builder()
        .format(DiskImageFormat::ExtendedDSK)
        .num_sides(2)
        .num_tracks(40)
        .sectors_per_track(9)
        .sector_size(512)
        .build()
        .expect("Failed to build image");

    assert_eq!(image.format(), DiskImageFormat::ExtendedDSK);
    assert_eq!(image.disk_count(), 2);
    assert_eq!(image.spec().num_tracks, 40);
    assert_eq!(image.spec().sectors_per_track, 9);

    // Verify structure
    for side in 0..2 {
        let disk = image.get_disk(side).expect("Failed to get disk");
        assert_eq!(disk.track_count(), 40);

        for track_num in 0..40 {
            let track = disk.get_track(track_num).expect("Failed to get track");
            assert_eq!(track.sector_count(), 9);
        }
    }
}

#[test]
fn test_format_specs() {
    let amstrad = FormatSpec::amstrad_system();
    assert_eq!(amstrad.num_sides, 1);
    assert_eq!(amstrad.num_tracks, 40);
    assert_eq!(amstrad.sectors_per_track, 9);
    assert_eq!(amstrad.total_capacity() / 1024, 180);

    let spectrum = FormatSpec::spectrum_plus3();
    assert_eq!(spectrum.first_sector_id, 0x01);

    let pcw = FormatSpec::pcw_dsdd();
    assert_eq!(pcw.num_sides, 2);
    assert_eq!(pcw.total_capacity() / 1024, 360);
}

#[test]
fn test_sector_operations() {
    let id = SectorId::new(0, 0, 0xC1, 2);
    assert_eq!(id.size_bytes(), 512);

    let mut sector = Sector::new(id);
    assert_eq!(sector.data().len(), 512);
    assert_eq!(sector.advertised_size(), 512);
    assert_eq!(sector.actual_size(), 512);
    assert!(!sector.has_size_mismatch());

    // Test filling
    sector.fill(0xFF);
    assert!(sector.data().iter().all(|&b| b == 0xFF));

    // Test resizing
    sector.resize(256, 0x00);
    assert_eq!(sector.actual_size(), 256);
}

#[test]
fn test_fdc_status() {
    let st1 = FdcStatus1::new(FdcStatus1::DE | FdcStatus1::EN);
    assert!(st1.data_error());
    assert!(st1.end_of_cylinder());
    assert!(!st1.overrun());
    assert!(st1.has_error());

    let st2 = FdcStatus2::new(FdcStatus2::CM);
    assert!(st2.is_deleted());
    assert!(!st2.has_error()); // Deleted mark is not an error
}

#[test]
fn test_track_operations() {
    let mut track = Track::new(0, 0);

    for i in 0xC1..=0xC9 {
        let id = SectorId::new(0, 0, i, 2);
        track.add_sector(Sector::new(id));
    }

    assert_eq!(track.sector_count(), 9);
    assert!(track.has_uniform_sector_size());
    assert_eq!(track.uniform_sector_size(), Some(512));

    let sector = track.get_sector(0xC5);
    assert!(sector.is_some());
    assert_eq!(sector.unwrap().id.sector, 0xC5);

    let sector_ids = track.sector_ids();
    assert_eq!(sector_ids.len(), 9);
}

#[test]
fn test_disk_operations() {
    let mut disk = Disk::new(0);

    for track_num in 0..5 {
        let mut track = Track::new(track_num, 0);

        for sector_id in 0xC1..=0xC9 {
            let id = SectorId::new(track_num, 0, sector_id, 2);
            track.add_sector(Sector::new(id));
        }

        disk.add_track(track);
    }

    assert_eq!(disk.track_count(), 5);
    assert_eq!(disk.total_size(), 5 * 9 * 512);

    let track = disk.get_track(2).expect("Failed to get track");
    assert_eq!(track.track_number, 2);
    assert_eq!(track.sector_count(), 9);
}


#[test]
fn test_error_handling() {
    let image = DiskImage::builder()
        .num_sides(1)
        .num_tracks(10)
        .build()
        .expect("Failed to build image");

    // Test invalid side
    let result = image.read_sector(5, 0, 0xC1);
    assert!(result.is_err());
    assert!(matches!(result, Err(DskError::InvalidTrack { .. })));

    // Test invalid track
    let result = image.read_sector(0, 50, 0xC1);
    assert!(result.is_err());
    assert!(matches!(result, Err(DskError::InvalidTrack { .. })));

    // Test invalid sector
    let result = image.read_sector(0, 0, 0xFF);
    assert!(result.is_err());
    assert!(matches!(result, Err(DskError::InvalidSector { .. })));
}

#[test]
fn test_format_presets() {
    // Test all preset formats create valid images
    let formats = vec![
        FormatSpec::amstrad_system(),
        FormatSpec::amstrad_data(),
        FormatSpec::amstrad_data_ds(),
        FormatSpec::spectrum_plus3(),
        FormatSpec::spectrum_plus3_ds(),
        FormatSpec::pcw_ssdd(),
        FormatSpec::pcw_dsdd(),
        FormatSpec::ibm_pc_360k(),
        FormatSpec::ibm_pc_720k(),
    ];

    for spec in formats {
        let image = DiskImage::create(spec.clone()).expect("Failed to create image");
        assert_eq!(image.spec().num_sides, spec.num_sides);
        assert_eq!(image.spec().num_tracks, spec.num_tracks);
        assert_eq!(image.spec().sectors_per_track, spec.sectors_per_track);
    }
}

#[test]
fn test_capacity_calculations() {
    let spec = FormatSpec::new(2, 80, 9, 512);
    assert_eq!(spec.total_capacity(), 2 * 80 * 9 * 512);
    assert_eq!(spec.total_capacity() / 1024, 720);

    let image = DiskImage::create(spec).expect("Failed to create image");
    assert_eq!(image.total_capacity() / 1024, 720);
}

#[test]
fn test_side_modes() {
    let spec = FormatSpec::amstrad_data().with_side_mode(SideMode::Successive);
    assert_eq!(spec.side_mode, SideMode::Successive);

    let spec = FormatSpec::amstrad_data_ds();
    assert_eq!(spec.side_mode, SideMode::Alternate);
}

#[test]
fn test_builder_fluent_api() {
    let image = DiskImage::builder()
        .format(DiskImageFormat::ExtendedDSK)
        .num_sides(2)
        .num_tracks(80)
        .sectors_per_track(9)
        .sector_size(512)
        .build()
        .expect("Failed to build image");

    assert_eq!(image.format(), DiskImageFormat::ExtendedDSK);
    assert_eq!(image.disk_count(), 2);
    assert_eq!(image.spec().num_tracks, 80);
}
