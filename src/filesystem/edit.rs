//! File-level changes to sector images. Mutations are staged on a clone so a
//! full disk, bad directory, or missing sector never leaves a half-written file.

use super::{mgt::MgtFileSystem, trdos::TrdosFileSystem, FileSystemType};
use crate::error::{DskError, Result};
use crate::format::{AllocationSize, DiskSpecSide, DiskSpecification};
use crate::image::DiskImage;
use std::collections::HashSet;

/// Import a host file's bytes into the selected filesystem. Existing names are
/// not replaced; delete the old file first. CP/M accepts `N:NAME.EXT` for user N.
pub fn import_file(image: &mut DiskImage, fs: FileSystemType, name: &str, data: &[u8]) -> Result<()> {
    let mut staged = image.clone();
    match effective_type(&staged, fs) {
        FileSystemType::Cpm => cpm_import(&mut staged, name, data)?,
        FileSystemType::Mgt => mgt_import(&mut staged, name, data)?,
        FileSystemType::Trdos => trdos_import(&mut staged, name, data)?,
        FileSystemType::Auto => unreachable!(),
    }
    *image = staged;
    Ok(())
}

/// Remove a file from the selected filesystem. TR-DOS compacts the remaining
/// files so that their disk space can be reused immediately.
pub fn delete_file(image: &mut DiskImage, fs: FileSystemType, name: &str) -> Result<()> {
    let mut staged = image.clone();
    match effective_type(&staged, fs) {
        FileSystemType::Cpm => cpm_delete(&mut staged, name)?,
        FileSystemType::Mgt => mgt_delete(&mut staged, name)?,
        FileSystemType::Trdos => trdos_delete(&mut staged, name)?,
        FileSystemType::Auto => unreachable!(),
    }
    *image = staged;
    Ok(())
}

fn effective_type(image: &DiskImage, fs: FileSystemType) -> FileSystemType {
    if fs == FileSystemType::Auto { image.default_filesystem() } else { fs }
}

fn padded_name(name: &str, length: usize) -> Result<Vec<u8>> {
    if name.is_empty() || name.len() > length || !name.bytes().all(|b| b.is_ascii_alphanumeric() || b"_$-!#@".contains(&b)) {
        return Err(DskError::InvalidFilename(name.to_string()));
    }
    let mut result = vec![b' '; length];
    result[..name.len()].copy_from_slice(name.to_ascii_uppercase().as_bytes());
    Ok(result)
}

fn cpm_name(name: &str) -> Result<(u8, [u8; 11])> {
    let (user, filename) = if let Some((user, filename)) = name.split_once(':') {
        (user.parse::<u8>().ok().filter(|&n| n <= 31).ok_or_else(|| DskError::InvalidFilename(name.to_string()))?, filename)
    } else { (0, name) };
    let (base, ext) = filename.split_once('.').unwrap_or((filename, ""));
    let mut raw = [b' '; 11];
    raw[..8].copy_from_slice(&padded_name(base, 8)?);
    if !ext.is_empty() { raw[8..].copy_from_slice(&padded_name(ext, 3)?); }
    Ok((user, raw))
}

pub(super) fn cpm_address(image: &DiskImage, spec: &DiskSpecification, absolute: usize) -> Result<(u8, u8, u8)> {
    let spt = spec.sectors_per_track as usize;
    let tps = spec.tracks_per_side as usize;
    if spt == 0 || tps == 0 { return Err(DskError::filesystem("Invalid CP/M geometry")); }
    let logical_track = absolute / spt;
    let (side, track) = match spec.side {
        DiskSpecSide::Single => (0, logical_track),
        DiskSpecSide::DoubleAlternate => (logical_track % 2, logical_track / 2),
        DiskSpecSide::DoubleSuccessive => (logical_track / tps, logical_track % tps),
        DiskSpecSide::DoubleReverse => (logical_track / tps, if logical_track >= tps { tps - 1 - logical_track % tps } else { logical_track }),
        DiskSpecSide::Invalid => return Err(DskError::filesystem("Invalid CP/M side mode")),
    };
    let physical = image.get_disk(side as u8).and_then(|d| d.get_track(track as u8))
        .ok_or_else(|| DskError::filesystem("Missing CP/M track"))?;
    let mut ids: Vec<_> = physical.sectors().iter().map(|s| s.id.sector).collect();
    ids.sort_unstable();
    let id = *ids.get(absolute % spt).ok_or_else(|| DskError::filesystem("Missing CP/M sector"))?;
    if image.read_sector(side as u8, track as u8, id)?.len() != spec.sector_size as usize {
        return Err(DskError::filesystem("Unexpected CP/M sector size"));
    }
    Ok((side as u8, track as u8, id))
}

fn cpm_directory(image: &DiskImage, spec: &DiskSpecification) -> Result<(Vec<(u8,u8,u8)>, Vec<u8>)> {
    let size = spec.directory_entries() * 32;
    if size == 0 || spec.sector_size == 0 || size % spec.sector_size as usize != 0 {
        return Err(DskError::filesystem("Invalid CP/M directory geometry"));
    }
    let mut addresses = Vec::new();
    let mut bytes = Vec::with_capacity(size);
    let first = spec.reserved_tracks as usize * spec.sectors_per_track as usize;
    for absolute in first..first + size / spec.sector_size as usize {
        let address = cpm_address(image, spec, absolute)?;
        bytes.extend_from_slice(image.read_sector(address.0, address.1, address.2)?);
        addresses.push(address);
    }
    Ok((addresses, bytes))
}

fn cpm_write_directory(image: &mut DiskImage, addresses: &[(u8,u8,u8)], bytes: &[u8], size: usize) -> Result<()> {
    for (address, chunk) in addresses.iter().zip(bytes.chunks_exact(size)) {
        image.write_sector(address.0, address.1, address.2, chunk)?;
    }
    Ok(())
}

fn cpm_matches(entry: &[u8], user: u8, name: &[u8;11]) -> bool {
    entry[0] == user && entry[1..12].iter().zip(name).all(|(&a, &b)| a & 0x7f == b)
}

fn cpm_import(image: &mut DiskImage, name: &str, data: &[u8]) -> Result<()> {
    let (user, name) = cpm_name(name)?;
    let spec = DiskSpecification::identify(image);
    let (addresses, mut dir) = cpm_directory(image, &spec)?;
    let block_size = spec.block_size();
    let sector_size = spec.sector_size as usize;
    if block_size < sector_size || block_size % sector_size != 0 || block_size > 16384 {
        return Err(DskError::filesystem("Unsupported CP/M block size"));
    }
    let capacity = match spec.allocation_size { AllocationSize::Byte => 16, AllocationSize::Word => 8 };
    let blocks_per_extent = capacity.min(16384 / block_size);
    let bytes_per_extent = blocks_per_extent * block_size;
    let extents = data.len().max(1).div_ceil(bytes_per_extent);
    if extents > 2048 || extents > dir.chunks_exact(32).filter(|e| e[0] == 0xE5).count() {
        return Err(DskError::DiskFull);
    }
    let mut used = HashSet::new();
    for block in 0..spec.directory_blocks as u16 { used.insert(block); }
    for entry in dir.chunks_exact(32).filter(|e| e[0] <= 31) {
        if cpm_matches(entry, user, &name) { return Err(DskError::filesystem("File already exists")); }
        match spec.allocation_size {
            AllocationSize::Byte => { for &b in &entry[16..32] { if b != 0 { used.insert(b as u16); } } }
            AllocationSize::Word => { for p in entry[16..32].chunks_exact(2) {
                let b = u16::from_le_bytes([p[0], p[1]]); if b != 0 { used.insert(b); }
            } }
        }
    }
    let count = spec.block_count();
    let needed = data.len().div_ceil(block_size);
    let free: Vec<_> = (1..count).filter(|b| !used.contains(b)).take(needed).collect();
    if free.len() != needed { return Err(DskError::DiskFull); }
    for (extent_number, chunk) in data.chunks(bytes_per_extent).enumerate() {
        let slot = dir.chunks_exact_mut(32).find(|e| e[0] == 0xE5).ok_or(DskError::DiskFull)?;
        slot.fill(0);
        slot[0] = user;
        slot[1..12].copy_from_slice(&name);
        slot[12] = (extent_number & 31) as u8;
        slot[14] = (extent_number >> 5) as u8;
        slot[15] = chunk.len().div_ceil(128) as u8;
        slot[13] = (chunk.len() % 128) as u8;
        for (i, block) in free[extent_number * blocks_per_extent..(extent_number * blocks_per_extent + chunk.len().div_ceil(block_size))].iter().enumerate() {
            match spec.allocation_size {
                AllocationSize::Byte => slot[16 + i] = *block as u8,
                AllocationSize::Word => slot[16 + i*2..18 + i*2].copy_from_slice(&block.to_le_bytes()),
            }
        }
    }
    if data.is_empty() {
        let slot = dir.chunks_exact_mut(32).find(|e| e[0] == 0xE5).ok_or(DskError::DiskFull)?;
        slot.fill(0); slot[0] = user; slot[1..12].copy_from_slice(&name);
    }
    for (index, &block) in free.iter().enumerate() {
        for sector in 0..block_size / sector_size {
            let absolute = (spec.reserved_tracks as usize * spec.sectors_per_track as usize)
                + block as usize * (block_size / sector_size) + sector;
            let (side, track, id) = cpm_address(image, &spec, absolute)?;
            let offset = index * block_size + sector * sector_size;
            let mut bytes = vec![0x1A; sector_size];
            if let Some(part) = data.get(offset..data.len().min(offset + sector_size)) {
                bytes[..part.len()].copy_from_slice(part);
            }
            image.write_sector(side, track, id, &bytes)?;
        }
    }
    cpm_write_directory(image, &addresses, &dir, sector_size)
}

fn cpm_delete(image: &mut DiskImage, name: &str) -> Result<()> {
    let (user, name_bytes) = cpm_name(name)?;
    let spec = DiskSpecification::identify(image);
    let (addresses, mut dir) = cpm_directory(image, &spec)?;
    let mut found = false;
    for entry in dir.chunks_exact_mut(32) {
        if cpm_matches(entry, user, &name_bytes) { entry[0] = 0xE5; found = true; }
    }
    if !found { return Err(DskError::FileNotFound(name.to_string())); }
    cpm_write_directory(image, &addresses, &dir, spec.sector_size as usize)
}

fn mgt_location(index: usize) -> (u8,u8,u8,usize) {
    let sector = index / 2;
    (0, (sector / 10) as u8, (sector % 10 + 1) as u8, (index % 2) * 256)
}

fn mgt_import(image: &mut DiskImage, name: &str, data: &[u8]) -> Result<()> {
    let name = padded_name(name, 10)?;
    if data.len() > u16::MAX as usize { return Err(DskError::filesystem("MGT import limited to 65535 bytes")); }
    let fs = MgtFileSystem::new(image)?;
    if fs.directory().iter().any(|e| e.filename.eq_ignore_ascii_case(String::from_utf8_lossy(&name).trim_end())) {
        return Err(DskError::filesystem("File already exists"));
    }
    let mut used = vec![false; 1600];
    used[..40].fill(true);
    for entry in fs.directory() {
        // Preserve both the bitmap and the contiguous layout used by the reader.
        for (i, byte) in entry.sector_map.iter().enumerate() {
            for bit in 0..8 { if byte & (0x80 >> bit) != 0 { used[40 + i*8 + bit] = true; } }
        }
        let side = if entry.start_track >= 128 { 1 } else { 0 };
        let track = (entry.start_track & 0x7f) as usize;
        let start = side * 800 + track * 10 + entry.start_sector.saturating_sub(1) as usize;
        for index in start..start.saturating_add(entry.sectors_used as usize).min(1600) { used[index] = true; }
    }
    let slot = (0..80).find(|&i| {
        let (side, track, id, offset) = mgt_location(i);
        image.read_sector(side, track, id).map(|s| s.len() >= offset + 256 && s[offset] & 0x3f == 0).unwrap_or(false)
    }).ok_or(DskError::DiskFull)?;
    let count = data.len().div_ceil(512);
    let start = (40..1600).find(|&i| {
        let boundary = if i < 800 { 800 } else { 1600 };
        i + count <= boundary && used[i..i + count].iter().all(|&occupied| !occupied)
    }).ok_or(DskError::DiskFull)?;
    let (side, track, id, offset) = mgt_location(slot);
    let mut entry = [0u8; 256];
    entry[0] = 0x13; // CODE
    entry[1..11].copy_from_slice(&name);
    entry[11..13].copy_from_slice(&(count as u16).to_be_bytes());
    entry[13] = ((start / 800) * 128 + (start % 800) / 10) as u8;
    entry[14] = (start % 10 + 1) as u8;
    entry[212..214].copy_from_slice(&(data.len() as u16).to_le_bytes());
    entry[211] = 3; // Disciple CODE tape header
    for i in start..start + count {
        let bit = i - 40;
        entry[15 + bit / 8] |= 0x80 >> (bit % 8);
    }
    for index in 0..count {
        let linear = start + index;
        let s = (linear / 800) as u8;
        let t = ((linear % 800) / 10) as u8;
        let r = (linear % 10 + 1) as u8;
        let mut bytes = [0u8; 512];
        let from = index * 512;
        let to = data.len().min(from + 512);
        bytes[..to - from].copy_from_slice(&data[from..to]);
        image.write_sector(s, t, r, &bytes)?;
    }
    let mut sector = image.read_sector(side, track, id)?.to_vec();
    sector[offset..offset + 256].copy_from_slice(&entry);
    image.write_sector(side, track, id, &sector)
}

fn mgt_delete(image: &mut DiskImage, name: &str) -> Result<()> {
    let index = MgtFileSystem::new(image)?.find_file(name)
        .ok_or_else(|| DskError::FileNotFound(name.to_string()))?.index;
    let (side, track, id, offset) = mgt_location(index);
    let mut sector = image.read_sector(side, track, id)?.to_vec();
    sector[offset..offset + 256].fill(0);
    image.write_sector(side, track, id, &sector)
}

fn trdos_sector(image: &DiskImage, absolute: usize) -> Result<(u8,u8,u8)> {
    let tps = image.spec().num_tracks as usize;
    if tps == 0 { return Err(DskError::filesystem("Invalid TR-DOS geometry")); }
    let linear_track = absolute / 16;
    let side = (linear_track / tps) as u8;
    let track = (linear_track % tps) as u8;
    let id = (absolute % 16 + 1) as u8;
    image.read_sector(side, track, id)?;
    Ok((side, track, id))
}

fn trdos_import(image: &mut DiskImage, name: &str, data: &[u8]) -> Result<()> {
    let (base, file_type) = name.rsplit_once('.').unwrap_or((name, "C"));
    let name_bytes = padded_name(base, 8)?;
    let type_byte = match file_type.to_ascii_uppercase().as_str() { "B" => b'B', "C" => b'C', "D" => b'D', "#" => b'#', _ => return Err(DskError::InvalidFilename(name.to_string())) };
    if data.len() > u16::MAX as usize || data.len().div_ceil(256) > 255 {
        return Err(DskError::filesystem("TR-DOS file exceeds format limits"));
    }
    let fs = TrdosFileSystem::new(image)?;
    let catalog = fs.catalog().filter(|_| image.read_sector(0,0,9).map(|s| s.get(231) == Some(&0x10)).unwrap_or(false)).cloned()
        .ok_or_else(|| DskError::filesystem("Format the TR-DOS disk before importing"))?;
    if fs.directory().iter().any(|e| !e.deleted && e.filename.eq_ignore_ascii_case(base)) {
        return Err(DskError::filesystem("File already exists"));
    }
    let count = data.len().div_ceil(256);
    let first = catalog.first_free_track as usize * 16 + catalog.first_free_sector as usize;
    let total = image.spec().num_tracks as usize * image.spec().num_sides as usize * 16;
    if first < 16 || first + count > total || count > catalog.free_sectors as usize { return Err(DskError::DiskFull); }
    let slot = (0..128).find(|&i| {
        let sector = i / 16 + 1;
        image.read_sector(0, 0, sector as u8).map(|s| s[(i % 16)*16 + 8] == 0).unwrap_or(false)
    }).ok_or(DskError::DiskFull)?;
    let mut entry = [0u8;16];
    entry[..8].copy_from_slice(&name_bytes);
    entry[8] = type_byte;
    entry[9..11].copy_from_slice(&(data.len() as u16).to_le_bytes());
    entry[13] = count as u8;
    entry[14] = catalog.first_free_sector;
    entry[15] = catalog.first_free_track;
    for (i, chunk) in data.chunks(256).enumerate() {
        let (side, track, id) = trdos_sector(image, first + i)?;
        let mut sector = [0u8;256];
        sector[..chunk.len()].copy_from_slice(chunk);
        image.write_sector(side, track, id, &sector)?;
    }
    let dir_sector = (slot / 16 + 1) as u8;
    let mut sector = image.read_sector(0,0,dir_sector)?.to_vec();
    sector[(slot % 16)*16..(slot % 16+1)*16].copy_from_slice(&entry);
    image.write_sector(0,0,dir_sector,&sector)?;
    let mut catalog_bytes = image.read_sector(0,0,9)?.to_vec();
    let next = first + count;
    catalog_bytes[225] = (next % 16) as u8;
    catalog_bytes[226] = (next / 16) as u8;
    catalog_bytes[228] = catalog_bytes[228].saturating_add(1);
    catalog_bytes[229..231].copy_from_slice(&(catalog.free_sectors - count as u16).to_le_bytes());
    image.write_sector(0,0,9,&catalog_bytes)
}

fn trdos_delete(image: &mut DiskImage, name: &str) -> Result<()> {
    let fs = TrdosFileSystem::new(image)?;
    let (base, _) = name.rsplit_once('.').unwrap_or((name, ""));
    if !fs.directory().iter().any(|e| !e.deleted && e.filename.eq_ignore_ascii_case(base)) {
        return Err(DskError::FileNotFound(name.to_string()));
    }
    let survivors: Result<Vec<_>> = fs.directory().iter().filter(|e| !e.deleted && !e.filename.eq_ignore_ascii_case(base))
        .map(|e| Ok((e.filename.clone(), e.file_type.to_byte(), e.param1, fs.read_file_data(e)?))).collect();
    let survivors = survivors?;
    // Preserve the disk type and all non-filesystem sectors; reset the directory
    // and catalog before repacking surviving files at track 1.
    for id in 1..=8 { image.write_sector(0,0,id,&[0u8;256])?; }
    let total = image.spec().num_tracks as usize * image.spec().num_sides as usize * 16;
    let mut catalog = image.read_sector(0,0,9)?.to_vec();
    catalog[225] = 0; catalog[226] = 1; catalog[228] = 0;
    catalog[229..231].copy_from_slice(&((total - 16) as u16).to_le_bytes());
    image.write_sector(0,0,9,&catalog)?;
    for (base, kind, param, bytes) in survivors {
        let suffix = kind as char;
        trdos_import(image, &format!("{}.{}", base, suffix), &bytes)?;
        let fs = TrdosFileSystem::new(image)?;
        let index = fs.directory().iter().find(|e| e.filename.eq_ignore_ascii_case(&base) && !e.deleted).unwrap().index;
        let id = (index / 16 + 1) as u8;
        let mut sector = image.read_sector(0,0,id)?.to_vec();
        sector[(index % 16)*16 + 11..(index % 16)*16 + 13].copy_from_slice(&param.to_le_bytes());
        image.write_sector(0,0,id,&sector)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::filesystem::{CpmFileSystem, FileSystem};
    use crate::format::{DiskImageFormat, FormatSpec};

    #[test]
    fn cpm_import_multiextent_and_delete() {
        let mut image = DiskImage::create(FormatSpec::amstrad_data()).unwrap();
        let data: Vec<_> = (0..20017).map(|i| (i % 251) as u8).collect();
        import_file(&mut image, FileSystemType::Cpm, "2:DEMO.BIN", &data).unwrap();
        let fs = CpmFileSystem::from_image(&image).unwrap();
        assert_eq!(fs.read_file_binary("2:DEMO.BIN", true).unwrap(), data);
        assert_eq!(fs.read_dir().unwrap().iter().filter(|e| e.user == 2).count(), 1);
        delete_file(&mut image, FileSystemType::Cpm, "2:DEMO.BIN").unwrap();
        assert!(CpmFileSystem::from_image(&image).unwrap().read_file("2:DEMO.BIN").is_err());
        import_file(&mut image, FileSystemType::Cpm, "2:DEMO.BIN", &data).unwrap();
    }

    #[test]
    fn mgt_import_delete_and_reuse() {
        let mut image = DiskImage::builder()
            .format(DiskImageFormat::RawMgt)
            .num_sides(2).num_tracks(80).sectors_per_track(10)
            .spec(FormatSpec::new(2, 80, 10, 512).with_first_sector_id(1).with_filler_byte(0))
            .build().unwrap();
        let data = vec![0x41; 763];
        import_file(&mut image, FileSystemType::Mgt, "TESTFILE", &data).unwrap();
        let fs = MgtFileSystem::new(&image).unwrap();
        assert_eq!(fs.read_file(fs.find_file("TESTFILE").unwrap()).unwrap(), data);
        delete_file(&mut image, FileSystemType::Mgt, "TESTFILE").unwrap();
        assert!(MgtFileSystem::new(&image).unwrap().find_file("TESTFILE").is_none());
        import_file(&mut image, FileSystemType::Mgt, "TESTFILE", &data).unwrap();
    }

    #[test]
    fn trdos_delete_compacts_surviving_files() {
        let mut image = DiskImage::builder().format(DiskImageFormat::RawTrd)
            .spec(FormatSpec::trdos()).build().unwrap();
        let mut catalog = [0u8; 256];
        catalog[226] = 1;
        catalog[227] = 0x17;
        catalog[229..231].copy_from_slice(&(80u16 * 16 - 16).to_le_bytes());
        catalog[231] = 0x10;
        image.write_sector(0, 0, 9, &catalog).unwrap();
        for id in 1..=8 { image.write_sector(0, 0, id, &[0;256]).unwrap(); }
        import_file(&mut image, FileSystemType::Trdos, "FIRST.C", &[0x11; 300]).unwrap();
        import_file(&mut image, FileSystemType::Trdos, "SECOND.C", &[0x22; 400]).unwrap();
        delete_file(&mut image, FileSystemType::Trdos, "FIRST.C").unwrap();
        let fs = TrdosFileSystem::new(&image).unwrap();
        assert_eq!(fs.read_file("SECOND").unwrap(), vec![0x22; 400]);
        assert_eq!(fs.catalog().unwrap().first_free_track, 1);
        assert_eq!(fs.catalog().unwrap().first_free_sector, 2);
        import_file(&mut image, FileSystemType::Trdos, "THIRD.C", &[0x33; 256]).unwrap();
    }
}
