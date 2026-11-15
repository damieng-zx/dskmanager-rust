//! Direct sector editing commands: replace, fill and FDC-status changes.

use dskmanager::*;

/// Replace a sector's data from a host file of the same size.
pub fn replace_sector_from_file(
    image: &mut DiskImage,
    side: u8,
    track: u8,
    id: u8,
    path: &str,
) -> Result<usize> {
    let expected = image.read_sector(side, track, id)?.len();
    let data = std::fs::read(path)?;
    if data.len() != expected {
        return Err(DskError::filesystem(format!(
            "Sector has {} bytes but file has {}; use a same-size file",
            expected,
            data.len()
        )));
    }
    image.write_sector(side, track, id, &data)?;
    Ok(data.len())
}

/// Overwrite a sector's FDC status bytes.
pub fn set_sector_status(
    image: &mut DiskImage,
    side: u8,
    track: u8,
    id: u8,
    st1: u8,
    st2: u8,
) -> Result<()> {
    // Confirm the sector exists before taking mutable access.
    image.read_sector(side, track, id)?;
    let sector = image
        .get_disk_mut(side)
        .and_then(|disk| disk.get_track_mut(track))
        .and_then(|track| track.get_sector_mut(id))
        .ok_or(DskError::InvalidSector { side, track, id })?;
    sector.fdc_status1 = FdcStatus1::new(st1);
    sector.fdc_status2 = FdcStatus2::new(st2);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn one_sector() -> DiskImage {
        DiskImage::builder().num_tracks(1).sectors_per_track(1).build().unwrap()
    }

    #[test]
    fn replaced_data_and_status_survive_a_save() {
        let mut image = one_sector();
        let host = std::env::temp_dir().join(format!("dskmgr_sector_{}.bin", std::process::id()));
        let disk = std::env::temp_dir().join(format!("dskmgr_sector_{}.dsk", std::process::id()));
        std::fs::write(&host, [0x99; 512]).unwrap();

        replace_sector_from_file(&mut image, 0, 0, 0xC1, host.to_str().unwrap()).unwrap();
        set_sector_status(&mut image, 0, 0, 0xC1, 0x20, 0x40).unwrap();
        image.save(&disk).unwrap();
        let loaded = DiskImage::open(&disk).unwrap();
        std::fs::remove_file(host).ok();
        std::fs::remove_file(disk).ok();

        assert_eq!(loaded.read_sector(0, 0, 0xC1).unwrap(), &[0x99; 512]);
        let sector = loaded
            .get_disk(0)
            .unwrap()
            .get_track(0)
            .unwrap()
            .get_sector(0xC1)
            .unwrap();
        assert_eq!(sector.fdc_status1.0, 0x20);
        assert_eq!(sector.fdc_status2.0, 0x40);
    }

    #[test]
    fn wrong_size_does_not_modify_the_sector() {
        let mut image = one_sector();
        let host = std::env::temp_dir().join(format!("dskmgr_short_{}.bin", std::process::id()));
        std::fs::write(&host, [1, 2, 3]).unwrap();

        assert!(replace_sector_from_file(&mut image, 0, 0, 0xC1, host.to_str().unwrap()).is_err());
        std::fs::remove_file(host).ok();
        assert!(image
            .read_sector(0, 0, 0xC1)
            .unwrap()
            .iter()
            .all(|&b| b == 0xE5));
    }

    #[test]
    fn missing_sector_is_reported_not_panicked() {
        let mut image = one_sector();
        assert!(set_sector_status(&mut image, 0, 99, 0xC1, 0, 0).is_err());
        assert!(set_sector_status(&mut image, 9, 0, 0xC1, 0, 0).is_err());
    }
}
