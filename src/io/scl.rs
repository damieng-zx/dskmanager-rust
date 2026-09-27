//! SCL TR-DOS file archive support.
//!
//! SCL stores a `SINCLAIR` signature, a file count, 14-byte TR-DOS directory
//! records, then each file's sector-padded data. Opening an SCL materializes a
//! standard 80-track TR-DOS image so the existing filesystem tools can operate
//! on the archive contents.

use crate::error::{DskError, Result};
use crate::format::{DiskImageFormat, FormatSpec};
use crate::image::DiskImage;
use crate::filesystem::TrdosFileSystem;
use std::fs::File;
use std::io::Write;
use std::path::Path;

const SIGNATURE: &[u8; 8] = b"SINCLAIR";
const HEADER_SIZE: usize = 9;
const ENTRY_SIZE: usize = 14;
const TRDOS_ENTRY_SIZE: usize = 16;
const SECTOR_SIZE: usize = 256;
const SECTORS_PER_TRACK: usize = 16;
const FIRST_DATA_SECTOR: usize = SECTORS_PER_TRACK;
const DISK_SECTORS: usize = 80 * SECTORS_PER_TRACK;

/// Return true for paths with an `.scl` extension.
pub fn is_scl_file<P: AsRef<Path>>(path: P) -> bool {
    path.as_ref().extension().and_then(|e| e.to_str())
        .map(|e| e.eq_ignore_ascii_case("scl")).unwrap_or(false)
}

/// Read an SCL archive as an in-memory 80-track TR-DOS disk.
pub fn read_scl<P: AsRef<Path>>(path: P) -> Result<DiskImage> {
    let filename = path.as_ref().file_name().and_then(|n| n.to_str()).map(str::to_string);
    let bytes = std::fs::read(path)?;
    if bytes.len() < HEADER_SIZE || &bytes[..8] != SIGNATURE {
        return Err(DskError::invalid_format("Invalid SCL signature or truncated header"));
    }
    let file_count = bytes[8] as usize;
    if file_count > 128 {
        return Err(DskError::invalid_format("SCL archive contains more than 128 files"));
    }
    let directory_end = HEADER_SIZE.checked_add(file_count.checked_mul(ENTRY_SIZE)
        .ok_or_else(|| DskError::invalid_format("SCL directory size overflow"))?)
        .ok_or_else(|| DskError::invalid_format("SCL directory size overflow"))?;
    if directory_end > bytes.len() {
        return Err(DskError::invalid_format("Truncated SCL directory"));
    }

    let mut parsed = Vec::with_capacity(file_count);
    let mut total_sectors = 0usize;
    for index in 0..file_count {
        let offset = HEADER_SIZE + index * ENTRY_SIZE;
        let entry = &bytes[offset..offset + ENTRY_SIZE];
        let length = u16::from_le_bytes([entry[9], entry[10]]) as usize;
        let sector_count = entry[13] as usize;
        if length.div_ceil(SECTOR_SIZE) != sector_count {
            return Err(DskError::invalid_format(format!("Invalid length/sector count in SCL entry {}", index)));
        }
        total_sectors = total_sectors.checked_add(sector_count)
            .ok_or_else(|| DskError::invalid_format("SCL data size overflow"))?;
        parsed.push((entry.to_vec(), sector_count));
    }
    if total_sectors > DISK_SECTORS - FIRST_DATA_SECTOR {
        return Err(DskError::invalid_format("SCL files do not fit on an 80-track TR-DOS disk"));
    }
    let required = directory_end.checked_add(total_sectors * SECTOR_SIZE)
        .ok_or_else(|| DskError::invalid_format("SCL data size overflow"))?;
    if required > bytes.len() {
        return Err(DskError::invalid_format("Truncated SCL file data"));
    }

    let mut image = DiskImage::builder()
        .format(DiskImageFormat::RawScl)
        .spec(FormatSpec::trdos())
        .build()?;
    // SCL has no boot/catalog sectors. Materialize an empty TR-DOS volume first.
    for track in 0..80u8 {
        for sector in 1..=16u8 {
            image.write_sector(0, track, sector, &[0; SECTOR_SIZE])?;
        }
    }

    let mut directory = vec![0u8; 8 * SECTOR_SIZE];
    let mut data_offset = directory_end;
    let mut next_sector = FIRST_DATA_SECTOR;
    for (index, (source, sector_count)) in parsed.into_iter().enumerate() {
        let entry_offset = index * TRDOS_ENTRY_SIZE;
        directory[entry_offset..entry_offset + 13].copy_from_slice(&source[..13]);
        directory[entry_offset + 13] = sector_count as u8;
        let entry = &mut directory[entry_offset..entry_offset + TRDOS_ENTRY_SIZE];
        entry[14] = (next_sector % SECTORS_PER_TRACK) as u8;
        entry[15] = (next_sector / SECTORS_PER_TRACK) as u8;
        for relative in 0..sector_count {
            let sector_index = next_sector + relative;
            let track = (sector_index / SECTORS_PER_TRACK) as u8;
            let sector_id = (sector_index % SECTORS_PER_TRACK + 1) as u8;
            let from = data_offset + relative * SECTOR_SIZE;
            image.write_sector(0, track, sector_id, &bytes[from..from + SECTOR_SIZE])?;
        }
        next_sector += sector_count;
        data_offset += sector_count * SECTOR_SIZE;
    }
    for sector_index in 0..8 {
        image.write_sector(0, 0, sector_index as u8 + 1,
            &directory[sector_index * SECTOR_SIZE..(sector_index + 1) * SECTOR_SIZE])?;
    }
    let mut catalog = [0u8; SECTOR_SIZE];
    catalog[225] = (next_sector % SECTORS_PER_TRACK) as u8;
    catalog[226] = (next_sector / SECTORS_PER_TRACK) as u8;
    catalog[227] = 0x17;
    catalog[228] = file_count as u8;
    catalog[229..231].copy_from_slice(&((DISK_SECTORS - next_sector) as u16).to_le_bytes());
    catalog[231] = 0x10;
    image.write_sector(0, 0, 9, &catalog)?;
    image.filename = filename;
    image.changed = false;
    Ok(image)
}

/// Write the TR-DOS files on an image as an SCL archive.
pub(crate) fn write_scl(file: &mut File, image: &DiskImage) -> Result<()> {
    if image.disk_count() != 1 || image.spec().num_sides != 1 || image.spec().num_tracks > 80
        || image.spec().sectors_per_track != 16 || image.spec().sector_size != 256 {
        return Err(DskError::invalid_format("SCL export requires a single-sided TR-DOS image with 16x256-byte sectors"));
    }
    let fs = TrdosFileSystem::new(image)?;
    let entries: Vec<_> = fs.directory().iter().filter(|entry| !entry.deleted).collect();
    if entries.len() > 128 {
        return Err(DskError::invalid_format("SCL supports at most 128 files"));
    }
    let mut archive = Vec::new();
    archive.extend_from_slice(SIGNATURE);
    archive.push(entries.len() as u8);
    let mut file_data = Vec::new();
    let mut sectors_total = 0usize;
    for entry in &entries {
        let data = fs.read_file_data(entry)?;
        if data.len() > u16::MAX as usize {
            return Err(DskError::invalid_format(format!("File {} cannot be represented in SCL", entry.filename)));
        }
        let mut raw = [0u8; ENTRY_SIZE];
        let name = entry.filename.as_bytes();
        if name.len() > 8 {
            return Err(DskError::invalid_format(format!("TR-DOS name {} exceeds 8 bytes", entry.filename)));
        }
        raw[..8].fill(b' ');
        raw[..name.len()].copy_from_slice(name);
        raw[8] = entry.file_type.to_byte();
        raw[9..11].copy_from_slice(&(data.len() as u16).to_le_bytes());
        raw[11..13].copy_from_slice(&entry.param1.to_le_bytes());
        raw[13] = entry.sector_count;
        archive.extend_from_slice(&raw);
        file_data.extend_from_slice(&data);
        let padded = entry.sector_count as usize * SECTOR_SIZE;
        if data.len() > padded {
            return Err(DskError::invalid_format(format!("File {} exceeds its TR-DOS allocation", entry.filename)));
        }
        file_data.resize(file_data.len() + padded - data.len(), 0);
        sectors_total += entry.sector_count as usize;
    }
    if sectors_total > DISK_SECTORS - FIRST_DATA_SECTOR {
        return Err(DskError::invalid_format("TR-DOS files exceed SCL capacity"));
    }
    archive.extend_from_slice(&file_data);
    file.write_all(&archive)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::filesystem::{format_filesystem, import_file, FileSystem, FileSystemType, TrdosFileSystem};

    #[test]
    fn scl_roundtrip_preserves_file_metadata_and_data() {
        let mut image = DiskImage::builder().format(DiskImageFormat::RawTrd)
            .spec(FormatSpec::trdos()).build().unwrap();
        format_filesystem(&mut image, FileSystemType::Trdos).unwrap();
        import_file(&mut image, FileSystemType::Trdos, "HELLO.C", &[0x42; 513]).unwrap();
        let fs = TrdosFileSystem::new(&image).unwrap();
        let entry = fs.find_file("HELLO").unwrap();
        let expected_param = entry.param1;

        let path = std::env::temp_dir().join(format!("dskmgr_scl_{}.scl", std::process::id()));
        image.save(&path).unwrap();
        let restored = DiskImage::open(&path).unwrap();
        std::fs::remove_file(path).ok();
        let restored_fs = TrdosFileSystem::new(&restored).unwrap();
        let restored_entry = restored_fs.find_file("HELLO").unwrap();
        assert_eq!(restored_fs.read_file("HELLO").unwrap(), vec![0x42; 513]);
        assert_eq!(restored_entry.param1, expected_param);
        assert_eq!(restored.format(), DiskImageFormat::RawScl);
    }

    #[test]
    fn rejects_bad_signature_and_truncated_records() {
        let path = std::env::temp_dir().join(format!("dskmgr_bad_scl_{}.scl", std::process::id()));
        std::fs::write(&path, b"NOTSCL!!\0").unwrap();
        assert!(read_scl(&path).is_err());
        std::fs::write(&path, b"SINCLAIR\x01").unwrap();
        assert!(read_scl(&path).is_err());
        std::fs::remove_file(path).ok();
    }
}
