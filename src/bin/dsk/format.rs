//! Presentation for the `dsk` console: how tracks, sectors and disks are shown.
//!
//! Kept apart from the command implementations so a change to a table layout
//! touches one file, and so the command modules stay readable.

use dskmanager::image::SectorStatus;
use dskmanager::*;

/// The interactive command reference.
pub fn print_help() {
    println!("Disk Management:");
    println!("  open <path>                    - Open a disk image file (use quotes for paths with spaces)");
    println!("  create [amstrad|system|spectrum|pcw|mgt|trdos] - Create and format a new image");
    println!("  save <path>                    - Save image to file (use quotes for paths with spaces)");
    println!("  info                           - Show disk information");
    println!("  specification                  - Detect and display disk specification (spec)");
    println!();
    println!("Low-Level Access:");
    println!("  tracks                         - List all tracks");
    println!("  sectors [track] [side]         - List sectors (all or specific track/side)");
    println!("  read-sector <s> <t> <id>       - Read and display a sector");
    println!("  write-sector <s> <t> <id> <path> - Replace sector data from a same-size host file");
    println!("  fill-sector <s> <t> <id> <byte> - Fill a sector with a decimal or 0xNN byte");
    println!("  set-sector-status <s> <t> <id> <st1> <st2> - Set FDC status bytes (decimal or 0xNN)");
    println!();
    println!("Filesystem:");
    println!("  fs-switch [auto|cpm|mgt|trdos]  - Show or set filesystem type (auto detects from image format)");
    println!("  fs-list                        - List files on disk (also: dir, cat, ls)");
    println!("  fs-read <filename>             - Read and hex dump file from disk");
    println!("  fs-show <filename>             - Display AMSDOS and PLUS3DOS BASIC files as text");
    println!("  fs-export <file> [output_path] [raw] - Export file from disk to host filesystem");
    println!("                                         (output_path defaults to filename if not specified)");
    println!("                                         (strips AMSDOS/PLUS3DOS headers by default, use 'raw' to preserve)");
    println!("  fs-import <host_path> [disk_name] - Add a file to the current filesystem");
    println!("  fs-delete <disk_name>          - Delete a file from the current filesystem");
    println!("  fs-format [auto|cpm|mgt|trdos] - Erase and initialize the filesystem");
    println!();
    println!("Analysis:");
    println!("  protection                     - Detect copy protection scheme");
    println!("  disassemble [track] [sector]   - Disassemble Z80 code from sector (dasm)");
    println!("  strings [len] [uniq] [charset] - Find strings (default: 4, 3, A-Za-z0-9...)");
    println!("  map [side]                     - Visual sector map (white=ok, red=error, yellow=deleted)");
    println!("  verify                         - Verify disk image structure and filesystem integrity");
    println!();
    println!("General:");
    println!("  help                           - Show this help");
    println!("  quit, exit                     - Exit");
}

/// Summary of a loaded image, including bootability.
pub fn print_info(image: &DiskImage) {    if let Some(filename) = image.filename() {
        println!("Filename: {}", filename);
        println!("Changed: {}", if image.is_changed() { "Yes" } else { "No" });
    }
    println!("Format: {}", image.format().name());
    println!("Sides: {}", image.spec().num_sides);
    println!("Tracks per side: {}", image.spec().num_tracks);
    println!("Sectors per track: {}", image.spec().sectors_per_track);
    println!("Sector size: {} bytes", image.spec().sector_size);
    println!("First sector ID: {}", image.spec().first_sector_id);

    let boot = BootDetection::detect(image);
    if boot.system.is_empty() {
        println!("Bootable on: Not bootable");
    } else {
        println!("Bootable on: {}", boot.system);
    }
    if !boot.reason.is_empty() {
        println!("Reason: {}", boot.reason);
    }
}

/// Filesystem accounting, in one consistent layout for every driver.
pub fn print_filesystem_info(info: &FileSystemInfo) {
    println!("{} filesystem", info.fs_type);
    println!("Block size: {} bytes", info.block_size);
    println!("Total blocks: {}", info.total_blocks);
    println!(
        "Usable capacity: {} KB",
        info.total_blocks * info.block_size / 1024
    );
    println!("Free blocks: {}", info.free_blocks);
    println!(
        "Free space: {} KB",
        info.free_blocks * info.block_size / 1024
    );
}

/// One row per track, with logical versus physical numbering.
pub fn list_tracks(image: &DiskImage) {
    for (side_idx, disk) in image.disks().iter().enumerate() {
        println!("\nSide {}:", side_idx);
        println!(
            "{:<8} {:<8} {:<12} {:<8} {:<4} {:<8} {:<11}",
            "Logical", "Physical", "Track Size", "Sectors", "Gap", "Filler", "Status"
        );
        println!("{}", "-".repeat(69));

        for (physical_idx, track) in disk.tracks().iter().enumerate() {
            // The logical number is what the sectors claim, which can differ
            // from the physical position on protected disks.
            let logical_track = track
                .sectors()
                .first()
                .map(|sector| sector.id.track)
                .unwrap_or(track.track_number);

            let status = if track.is_empty() {
                "Unformatted"
            } else if track
                .sectors()
                .iter()
                .any(|sector| matches!(sector.status(track.filler_byte), SectorStatus::FormattedInUse))
            {
                "In use"
            } else {
                "Blank"
            };

            println!(
                "{:<8} {:<8} {:<12} {:<8} {:<4} 0x{:02X}     {:<11}",
                logical_track,
                physical_idx,
                track.total_data_size(),
                track.sector_count(),
                track.gap3_length,
                track.filler_byte,
                status
            );
        }
    }
}

/// One row per sector in a single track.
pub fn list_sectors_on_track(image: &DiskImage, side: u8, track: u8) {
    let Some(disk) = image.disks().get(side as usize) else {
        println!("Side {} not found.", side);
        return;
    };
    let Some(track_data) = disk.get_track(track) else {
        println!("Track {} not found on side {}.", track, side);
        return;
    };
    print_sector_table(track_data);
}

/// One table per track across every side.
pub fn list_all_sectors(image: &DiskImage) {
    for disk in image.disks() {
        for track in disk.tracks() {
            print_sector_table(track);
        }
    }
}

fn print_sector_table(track: &Track) {
    println!(
        "{:<6} {:<6} {:<6} {:<6} {:<12} {:<10} {:<10} {:<12}",
        "Sector", "Track", "Side", "ID", "FDC Size", "FDC Flags", "Data Size", "Status"
    );
    println!("{}", "-".repeat(80));

    for (idx, sector) in track.sectors().iter().enumerate() {
        println!(
            "{:<6} {:<6} {:<6} {:<6} {:<12} {:<10} {:<10} {:<12}",
            idx,
            sector.id.track,
            sector.id.side,
            sector.id.sector,
            format!("{} ({})", sector.id.size_code, sector.advertised_size()),
            format!("{},{}", sector.fdc_status1.0, sector.fdc_status2.0),
            sector.actual_size(),
            sector.status(track.filler_byte)
        );
    }
}

/// Directory listing table shared by all filesystems.
pub fn print_directory(entries: &[ExtendedDirEntry]) {
    if entries.is_empty() {
        println!("No files found.");
        return;
    }

    println!(
        "{:<14} {:>3} {:>3} {:>4} {:>5} {:>7} {:>3} {:>3} {:<8} {:>3} Meta",
        "Name", "Idx", "Usr", "Blks", "Alloc", "Size", "Att", "Del", "Header", "Chk"
    );
    println!("{}", "-".repeat(96));

    for entry in entries {
        let is_deleted = entry.user == 0xE5;
        let user = if is_deleted {
            "E5".to_string()
        } else {
            entry.user.to_string()
        };
        let attributes = format!(
            "{}{}{}",
            if entry.attributes.read_only { "R" } else { "-" },
            if entry.attributes.system { "S" } else { "-" },
            if entry.attributes.archive { "A" } else { "-" }
        );
        let checksum = if entry.header.header_type == HeaderType::None {
            ""
        } else if entry.header.checksum_valid {
            "Yes"
        } else {
            "No"
        };

        println!(
            "{:<14} {:>3} {:>3} {:>4} {:>4}K {:>7} {:>3} {:>3} {:<8} {:>3} {}",
            entry.name,
            entry.index,
            user,
            entry.blocks,
            entry.allocated / 1024,
            entry.size,
            attributes,
            if is_deleted { "Yes" } else { "" },
            entry.header.header_type,
            checksum,
            entry.header.meta
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cpc() -> DiskImage {
        let mut image = DiskImage::create(FormatSpec::amstrad_data()).unwrap();
        crate::filesystem::format_filesystem(&mut image, FileSystemType::Cpm).unwrap();
        image
    }

    #[test]
    fn directory_table_renders_without_panicking() {
        // Smoke test for the shared table: empty and populated listings.
        print_directory(&[]);

        let mut image = cpc();
        crate::filesystem::import_file(&mut image, FileSystemType::Cpm, "A.TXT", b"hi").unwrap();
        let fs = crate::filesystem::AnyFileSystem::mount(&image, FileSystemType::Auto).unwrap();
        let entries = fs.read_dir_extended().unwrap();
        assert_eq!(entries.len(), 1);
        print_directory(&entries);
    }

    #[test]
    fn sector_listing_covers_empty_and_populated_tracks() {
        let mut image = cpc();
        list_sectors_on_track(&image, 0, 0);
        // Missing side and track must report rather than panic.
        list_sectors_on_track(&image, 5, 0);
        list_sectors_on_track(&image, 0, 200);
        list_all_sectors(&image);
        list_tracks(&image);
        image.write_sector(0, 0, 0xC1, &[0x42; 512]).unwrap();
        list_tracks(&image);
    }

    #[test]
    fn filesystem_info_matches_the_driver_numbers() {
        let image = cpc();
        let fs = crate::filesystem::AnyFileSystem::mount(&image, FileSystemType::Auto).unwrap();
        let info = fs.info();
        // A freshly created CPC data disk reserves no tracks: 40 x 9 x 512 =
        // 180 KB, i.e. 180 one-KB blocks.
        assert_eq!(info.total_blocks, 180);
        assert_eq!(info.block_size, 1024);
        print_filesystem_info(&info);
    }
}
