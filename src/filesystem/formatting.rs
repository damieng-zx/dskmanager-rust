//! Initialize directory structures on a sector image.

use super::{edit::cpm_address, FileSystemType};
use crate::error::{DskError, Result};
use crate::format::{DiskSpecSide, DiskSpecification, SideMode};
use crate::image::DiskImage;

/// Initialize a CP/M, MGT, or TR-DOS filesystem on an image. All files and
/// existing sector contents are erased. `Auto` selects the image's default.
pub fn format_filesystem(image: &mut DiskImage, fs: FileSystemType) -> Result<()> {
    let fs = if fs == FileSystemType::Auto { image.default_filesystem() } else { fs };
    let mut staged = image.clone();
    match fs {
        FileSystemType::Cpm => format_cpm(&mut staged)?,
        FileSystemType::Mgt => format_mgt(&mut staged)?,
        FileSystemType::Trdos => format_trdos(&mut staged)?,
        FileSystemType::Auto => unreachable!(),
    }
    *image = staged;
    Ok(())
}

fn fill_image(image: &mut DiskImage, size: usize, filler: u8) -> Result<()> {
    let addresses: Vec<_> = image.disks().iter().enumerate().flat_map(|(side, disk)| {
        disk.tracks().iter().enumerate().flat_map(move |(track, t)| {
            t.sectors().iter().map(move |s| (side as u8, track as u8, s.id.sector, s.data().len()))
        })
    }).collect();
    for (side, track, id, len) in addresses {
        if len != size { return Err(DskError::filesystem("Sector geometry does not match filesystem")); }
        image.write_sector(side, track, id, &vec![filler; size])?;
    }
    Ok(())
}

fn format_cpm(image: &mut DiskImage) -> Result<()> {
    let mut spec = DiskSpecification::identify(image);
    if image.disk_count() == 2 && image.spec().first_sector_id == 1 {
        spec.side = match image.spec().side_mode {
            SideMode::Alternate => DiskSpecSide::DoubleAlternate,
            SideMode::Successive => DiskSpecSide::DoubleSuccessive,
            SideMode::SingleSide => DiskSpecSide::Single,
        };
    }
    let sector_size = spec.sector_size as usize;
    if sector_size == 0 || spec.sectors_per_track != image.spec().sectors_per_track
        || spec.tracks_per_side != image.spec().num_tracks || spec.directory_blocks == 0 {
        return Err(DskError::filesystem("Unrecognized CP/M disk geometry"));
    }
    let filler = image.spec().filler_byte;
    fill_image(image, sector_size, filler)?;
    let first = spec.reserved_tracks as usize * spec.sectors_per_track as usize;
    let dir_size = spec.directory_blocks as usize * spec.block_size();
    if dir_size % sector_size != 0 { return Err(DskError::filesystem("Invalid CP/M directory size")); }
    for absolute in first..first + dir_size / sector_size {
        let (side, track, id) = cpm_address(image, &spec, absolute)?;
        image.write_sector(side, track, id, &vec![0xE5; sector_size])?;
    }
    if image.spec().first_sector_id == 1 {
        // PCW/+3 boot specification block; unlike a blank sector this retains
        // the layout when the image is opened again.
        let mut boot = vec![image.spec().filler_byte; sector_size];
        boot[0] = 0;
        boot[1] = match image.spec().side_mode {
            SideMode::SingleSide => 0,
            SideMode::Alternate => 1,
            SideMode::Successive => 2,
        };
        boot[2] = spec.tracks_per_side;
        boot[3] = spec.sectors_per_track;
        boot[4] = spec.fdc_sector_size;
        boot[5] = spec.reserved_tracks;
        boot[6] = spec.block_shift;
        boot[7] = spec.directory_blocks;
        boot[8] = spec.gap_read_write;
        boot[9] = spec.gap_format;
        image.write_sector(0, 0, 1, &boot)?;
    }
    Ok(())
}

fn format_mgt(image: &mut DiskImage) -> Result<()> {
    if image.disk_count() != 2 || image.spec().num_tracks != 80 || image.spec().sectors_per_track != 10
        || image.spec().sector_size != 512 || image.spec().first_sector_id != 1 {
        return Err(DskError::filesystem("MGT requires 2 sides, 80 tracks, 10x512-byte sectors"));
    }
    fill_image(image, 512, 0)
}

fn format_trdos(image: &mut DiskImage) -> Result<()> {
    let spec = image.spec().clone();
    if !matches!(spec.num_tracks, 40 | 80) || !matches!(spec.num_sides, 1 | 2)
        || image.disk_count() != spec.num_sides as usize || spec.sectors_per_track != 16
        || spec.sector_size != 256 || spec.first_sector_id != 1 {
        return Err(DskError::filesystem("TR-DOS requires 40/80 tracks, 1/2 sides, 16x256-byte sectors"));
    }
    fill_image(image, 256, 0)?;
    let mut catalog = vec![0u8;256];
    catalog[226] = 1; // first free track, sector zero
    catalog[227] = match (spec.num_tracks, spec.num_sides) {
        (80, 2) => 0x16, (80, 1) => 0x17,
        (40, 1) => 0x18, (40, 2) => 0x19, _ => unreachable!(),
    };
    let free = (spec.num_tracks as u16 * spec.num_sides as u16 - 1) * 16;
    catalog[229..231].copy_from_slice(&free.to_le_bytes());
    catalog[231] = 0x10;
    image.write_sector(0,0,9,&catalog)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::filesystem::{import_file, CpmFileSystem, FileSystem, MgtFileSystem, TrdosFileSystem};
    use crate::format::{DiskImageFormat, FormatSpec};

    #[test]
    fn formatted_amstrad_and_plus3_accept_files() {
        for spec in [FormatSpec::amstrad_data(), FormatSpec::amstrad_system(), FormatSpec::spectrum_plus3(), FormatSpec::spectrum_plus3_ds()] {
            let mut image = DiskImage::create(spec).unwrap();
            format_filesystem(&mut image, FileSystemType::Cpm).unwrap();
            import_file(&mut image, FileSystemType::Cpm, "HELLO.TXT", b"hello").unwrap();
            assert_eq!(CpmFileSystem::from_image(&image).unwrap().read_file_binary("HELLO.TXT", true).unwrap(), b"hello");
        }
    }

    #[test]
    fn formatted_raw_disks_accept_files() {
        let mut mgt = DiskImage::builder().format(DiskImageFormat::RawMgt)
            .spec(FormatSpec::new(2,80,10,512).with_first_sector_id(1)).build().unwrap();
        format_filesystem(&mut mgt, FileSystemType::Mgt).unwrap();
        import_file(&mut mgt, FileSystemType::Mgt, "HELLO", b"hello").unwrap();
        assert_eq!(MgtFileSystem::new(&mgt).unwrap().directory().len(), 1);

        let mut trd = DiskImage::builder().format(DiskImageFormat::RawTrd)
            .spec(FormatSpec::trdos()).build().unwrap();
        format_filesystem(&mut trd, FileSystemType::Trdos).unwrap();
        import_file(&mut trd, FileSystemType::Trdos, "HELLO.C", b"hello").unwrap();
        assert_eq!(TrdosFileSystem::new(&trd).unwrap().read_file("HELLO").unwrap(), b"hello");
    }
}
