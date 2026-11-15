//! A single front end over the three filesystem implementations.
//!
//! The CLI, `verify` and other callers would otherwise repeat the same
//! "resolve `Auto`, then match on the type, then build the right driver" block.
//! Keeping it here means the resolution rule lives in one place and the
//! per-driver quirks (CP/M header stripping, TR-DOS entry conversion) are
//! applied consistently.

use crate::error::{DskError, Result};
use crate::filesystem::cpm::CpmFileSystem;
use crate::filesystem::disciple::DiscipleFileSystem;
use crate::filesystem::mgt::MgtFileSystem;
use crate::filesystem::trdos::TrdosFileSystem;
use crate::filesystem::{
    DirEntry, ExtendedDirEntry, FileAttributes, FileHeader, FileSystem, FileSystemInfo,
    FileSystemType, HeaderType,
};
use crate::image::DiskImage;

/// TR-DOS files are allocated in 256-byte sectors.
const TRDOS_SECTOR_SIZE: usize = 256;

/// A mounted filesystem, whichever driver it is.
pub enum AnyFileSystem<'a> {
    /// CP/M (Amstrad CPC, Spectrum +3, PCW, Tatung Einstein)
    Cpm(CpmFileSystem<'a>),
    /// MGT (DISCiPLE, +D, SAM Coupe)
    Mgt(MgtFileSystem<'a>),
    /// TR-DOS (ZX Spectrum Beta Disk Interface)
    Trdos(TrdosFileSystem<'a>),
}

impl<'a> AnyFileSystem<'a> {
    /// Mount the filesystem named by `requested`, resolving `Auto` from the
    /// image itself.
    pub fn mount(image: &'a DiskImage, requested: FileSystemType) -> Result<Self> {
        match Self::resolve(image, requested) {
            FileSystemType::Cpm | FileSystemType::Auto => {
                Ok(Self::Cpm(CpmFileSystem::from_image(image)?))
            }
            FileSystemType::Mgt => Ok(Self::Mgt(MgtFileSystem::new(image)?)),
            FileSystemType::Trdos => Ok(Self::Trdos(TrdosFileSystem::new(image)?)),
        }
    }

    /// The filesystem type that would be used for `requested` on this image.
    pub fn resolve(image: &DiskImage, requested: FileSystemType) -> FileSystemType {
        match requested {
            FileSystemType::Auto => image.default_filesystem(),
            other => other,
        }
    }

    /// Which driver is mounted.
    pub fn kind(&self) -> FileSystemType {
        match self {
            Self::Cpm(_) => FileSystemType::Cpm,
            Self::Mgt(_) => FileSystemType::Mgt,
            Self::Trdos(_) => FileSystemType::Trdos,
        }
    }

    /// Whether the driver stores an AMSDOS/PLUS3DOS header inside the file data.
    pub fn stores_headers_in_data(&self) -> bool {
        matches!(self, Self::Cpm(_))
    }

    /// Plain directory listing.
    pub fn read_dir(&self) -> Result<Vec<DirEntry>> {
        match self {
            Self::Cpm(fs) => fs.read_dir(),
            Self::Trdos(fs) => fs.read_dir(),
            Self::Mgt(fs) => Self::mgt_dir(fs),
        }
    }

    /// Directory listing with per-entry detail.
    pub fn read_dir_extended(&self) -> Result<Vec<ExtendedDirEntry>> {
        match self {
            Self::Cpm(fs) => fs.read_dir_extended_with_deleted(),
            Self::Mgt(fs) => match DiscipleFileSystem::new(fs.image()) {
                Ok(disciple) => disciple.read_dir_extended(),
                Err(error) => Err(error),
            },
            Self::Trdos(fs) => Ok(Self::trdos_entries(fs)),
        }
    }

    /// File contents with any AMSDOS/PLUS3DOS header removed.
    ///
    /// Non-CP/M drivers keep metadata in the directory entry, so this matches
    /// [`Self::read_file_raw`] for them.
    pub fn read_file(&self, name: &str) -> Result<Vec<u8>> {
        match self {
            Self::Cpm(fs) => fs.read_file(name),
            Self::Trdos(fs) => fs.read_file(name),
            Self::Mgt(fs) => Self::mgt_read(fs, name),
        }
    }

    /// File contents exactly as stored, header included for CP/M.
    pub fn read_file_raw(&self, name: &str) -> Result<Vec<u8>> {
        match self {
            Self::Cpm(fs) => fs.read_file_binary(name, true),
            Self::Trdos(fs) => fs.read_file(name),
            Self::Mgt(fs) => Self::mgt_read(fs, name),
        }
    }

    /// Filesystem accounting, normalised across drivers.
    pub fn info(&self) -> FileSystemInfo {
        match self {
            Self::Cpm(fs) => fs.info(),
            Self::Trdos(fs) => fs.info(),
            Self::Mgt(fs) => {
                let info = fs.info();
                FileSystemInfo {
                    fs_type: format!("MGT ({})", info.system_type),
                    total_blocks: info.total_sectors,
                    free_blocks: info.free_sectors,
                    block_size: 512,
                }
            }
        }
    }

    /// TR-DOS has no extended listing of its own; build it from the directory.
    fn trdos_entries(fs: &TrdosFileSystem<'_>) -> Vec<ExtendedDirEntry> {
        fs.directory()
            .iter()
            .filter(|entry| !entry.deleted)
            .map(|entry| {
                // The recorded length is the file size; the sector allocation is
                // only the space it occupies.
                let size = if entry.data_length > 0 {
                    entry.data_length as usize
                } else {
                    entry.sector_count as usize * TRDOS_SECTOR_SIZE
                };
                ExtendedDirEntry {
                    name: entry.filename.clone(),
                    user: 0,
                    index: entry.index,
                    blocks: entry.sector_count as usize,
                    allocated: entry.sector_count as usize * TRDOS_SECTOR_SIZE,
                    size,
                    attributes: FileAttributes::default(),
                    header: FileHeader {
                        header_type: HeaderType::None,
                        checksum_valid: false,
                        file_size: size,
                        header_size: 0,
                        meta: entry.display_type(),
                    },
                }
            })
            .collect()
    }

    fn mgt_read(fs: &MgtFileSystem<'_>, name: &str) -> Result<Vec<u8>> {
        let entry = fs
            .find_file(name)
            .ok_or_else(|| DskError::FileNotFound(name.to_string()))?;
        fs.read_file(entry)
    }

    /// Minimal listing for MGT, which has no `read_dir` of its own.
    fn mgt_dir(fs: &MgtFileSystem<'_>) -> Result<Vec<DirEntry>> {
        Ok(fs
            .directory()
            .iter()
            .map(|entry| DirEntry {
                name: entry.filename.clone(),
                user: 0,
                extent: 0,
                size: entry.file_size(),
                attributes: entry.attributes(),
            })
            .collect())
    }
}

/// Resolve a `FileSystemType` and mount it: the common entry point.
pub fn mount(image: &DiskImage, requested: FileSystemType) -> Result<AnyFileSystem<'_>> {
    AnyFileSystem::mount(image, requested)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::filesystem::{format_filesystem, import_file};
    use crate::format::{DiskImageFormat, FormatSpec};

    fn amstrad() -> DiskImage {
        let mut image = DiskImage::create(FormatSpec::amstrad_data()).unwrap();
        format_filesystem(&mut image, FileSystemType::Cpm).unwrap();
        image
    }

    fn trdos() -> DiskImage {
        let mut image = DiskImage::builder()
            .format(DiskImageFormat::RawTrd)
            .spec(FormatSpec::trdos())
            .build()
            .unwrap();
        format_filesystem(&mut image, FileSystemType::Trdos).unwrap();
        image
    }

    fn mgt() -> DiskImage {
        let mut image = DiskImage::builder()
            .format(DiskImageFormat::RawMgt)
            .spec(FormatSpec::new(2, 80, 10, 512).with_first_sector_id(1).with_filler_byte(0))
            .build()
            .unwrap();
        format_filesystem(&mut image, FileSystemType::Mgt).unwrap();
        image
    }

    #[test]
    fn mount_resolves_auto_from_the_image() {
        for (image, expected) in [
            (amstrad(), FileSystemType::Cpm),
            (trdos(), FileSystemType::Trdos),
            (mgt(), FileSystemType::Mgt),
        ] {
            assert_eq!(AnyFileSystem::resolve(&image, FileSystemType::Auto), expected);
            assert_eq!(mount(&image, FileSystemType::Auto).unwrap().kind(), expected);
        }
    }

    #[test]
    fn explicit_type_overrides_auto() {
        let image = amstrad();
        assert_eq!(
            AnyFileSystem::resolve(&image, FileSystemType::Cpm),
            FileSystemType::Cpm,
            "an explicit request must not be re-detected"
        );
    }

    #[test]
    fn read_file_strips_cpm_headers_but_raw_keeps_them() {
        let mut image = amstrad();
        // 128-byte AMSDOS header declaring a 5-byte payload.
        let mut file = vec![0u8; 128];
        file[18] = 6; // ASCII
        file[64] = 5;
        file.extend_from_slice(b"hello");
        let checksum: u16 = file[0..=66].iter().map(|&b| b as u16).sum();
        file[67..69].copy_from_slice(&checksum.to_le_bytes());
        import_file(&mut image, FileSystemType::Cpm, "HI.TXT", &file).unwrap();

        let fs = mount(&image, FileSystemType::Auto).unwrap();
        assert!(fs.stores_headers_in_data());
        assert_eq!(fs.read_file("HI.TXT").unwrap(), b"hello");
        assert_eq!(fs.read_file_raw("HI.TXT").unwrap().len(), 133);
    }

    #[test]
    fn trdos_listing_reports_recorded_length_not_allocation() {
        let mut image = trdos();
        import_file(&mut image, FileSystemType::Trdos, "TINY.C", b"abc").unwrap();

        let fs = mount(&image, FileSystemType::Auto).unwrap();
        assert!(!fs.stores_headers_in_data());

        let plain = fs.read_dir().unwrap();
        assert_eq!(plain.len(), 1);
        assert_eq!(plain[0].size, 3);

        let extended = fs.read_dir_extended().unwrap();
        assert_eq!(extended.len(), 1);
        assert_eq!(extended[0].size, 3, "size is the recorded length");
        assert_eq!(extended[0].allocated, 256, "allocation is one sector");
    }

    #[test]
    fn trdos_listing_hides_deleted_entries() {
        let mut image = trdos();
        import_file(&mut image, FileSystemType::Trdos, "GONE.C", &[0x11; 256]).unwrap();
        let mut sector = image.read_sector(0, 0, 1).unwrap().to_vec();
        sector[8] = 0x01; // deleted
        image.write_sector(0, 0, 1, &sector).unwrap();

        let fs = mount(&image, FileSystemType::Auto).unwrap();
        assert!(fs.read_dir_extended().unwrap().is_empty());
        assert!(fs.read_dir().unwrap().is_empty());
    }

    #[test]
    fn mgt_listing_and_reads_work() {
        let mut image = mgt();
        let data = vec![0x41u8; 700];
        import_file(&mut image, FileSystemType::Mgt, "TESTFILE", &data).unwrap();

        let fs = mount(&image, FileSystemType::Auto).unwrap();
        assert_eq!(fs.read_file("TESTFILE").unwrap(), data);
        assert_eq!(fs.read_file_raw("TESTFILE").unwrap(), data);

        let entries = fs.read_dir().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "TESTFILE");

        let extended = fs.read_dir_extended().unwrap();
        assert_eq!(extended.len(), 1);
        assert_eq!(extended[0].size, 700);
    }

    #[test]
    fn info_is_normalised_across_drivers() {
        let cpm = amstrad();
        let fs = mount(&cpm, FileSystemType::Auto).unwrap();
        let info = fs.info();
        assert!(info.fs_type.starts_with("CP/M"), "got {:?}", info.fs_type);
        assert!(info.total_blocks > 0);
        assert_eq!(info.block_size, 1024);

        let trd = trdos();
        let fs = mount(&trd, FileSystemType::Auto).unwrap();
        let info = fs.info();
        assert_eq!(info.fs_type, "TR-DOS");
        assert_eq!(info.block_size, 256);

        let mgt_image = mgt();
        let fs = mount(&mgt_image, FileSystemType::Auto).unwrap();
        let info = fs.info();
        assert!(info.fs_type.starts_with("MGT"), "got {:?}", info.fs_type);
        assert_eq!(info.block_size, 512);
        assert_eq!(info.total_blocks, 1600);
    }
}
