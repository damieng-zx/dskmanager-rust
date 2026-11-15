//! Disk image verification
//!
//! Validates a disk image across structural, filesystem, and image-level
//! dimensions, producing a report of issues categorized by severity.

use std::collections::HashMap;

use crate::filesystem::{FileSystemType, FileSystem};
use crate::format::DiskSpecification;
use crate::image::{DiskImage, SectorStatus};

/// Severity of a verification issue
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    /// Informational note
    Info,
    /// Potential problem
    Warning,
    /// Definite error
    Error,
}

impl std::fmt::Display for Severity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Severity::Info => write!(f, "INFO"),
            Severity::Warning => write!(f, "WARN"),
            Severity::Error => write!(f, "ERROR"),
        }
    }
}

/// Location of a verification issue
#[derive(Debug, Clone)]
pub enum IssueLocation {
    /// Image-level issue
    Image,
    /// Side-specific issue
    Side(usize),
    /// Specific sector: side, track, sector_id
    Sector(usize, u8, u8),
    /// Specific track: side, track
    Track(usize, u8),
    /// Filesystem file issue
    File(String),
    /// Filesystem-level issue
    Filesystem,
}

impl std::fmt::Display for IssueLocation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            IssueLocation::Image => write!(f, "Image"),
            IssueLocation::Side(s) => write!(f, "Side {}", s),
            IssueLocation::Sector(s, t, id) => write!(f, "S{}:T{}:0x{:02X}", s, t, id),
            IssueLocation::Track(s, t) => write!(f, "S{}:T{}", s, t),
            IssueLocation::File(name) => write!(f, "{}", name),
            IssueLocation::Filesystem => write!(f, "Filesystem"),
        }
    }
}

/// A single verification issue
#[derive(Debug, Clone)]
pub struct VerifyIssue {
    /// Severity level
    pub severity: Severity,
    /// Where the issue was found
    pub location: IssueLocation,
    /// Human-readable description
    pub message: String,
}

/// Result of verifying a disk image
#[derive(Debug, Clone)]
pub struct VerifyReport {
    /// All issues found, sorted by severity (errors first)
    pub issues: Vec<VerifyIssue>,
}

impl VerifyReport {
    /// Count of errors
    pub fn error_count(&self) -> usize {
        self.issues.iter().filter(|i| i.severity == Severity::Error).count()
    }

    /// Count of warnings
    pub fn warning_count(&self) -> usize {
        self.issues.iter().filter(|i| i.severity == Severity::Warning).count()
    }

    /// Count of infos
    pub fn info_count(&self) -> usize {
        self.issues.iter().filter(|i| i.severity == Severity::Info).count()
    }

    /// Whether the image passed verification (no errors)
    pub fn passed(&self) -> bool {
        self.error_count() == 0
    }
}

impl std::fmt::Display for VerifyReport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.issues.is_empty() {
            writeln!(f, "No issues found. Disk image is valid.")?;
            return Ok(());
        }

        for issue in &self.issues {
            writeln!(f, "  {:<10} {:<20} {}", issue.severity, issue.location, issue.message)?;
        }

        writeln!(f)?;
        write!(
            f,
            "Summary: {} error(s), {} warning(s), {} info",
            self.error_count(),
            self.warning_count(),
            self.info_count()
        )?;

        if self.passed() {
            write!(f, " — PASSED")?;
        } else {
            write!(f, " — FAILED")?;
        }

        Ok(())
    }
}

/// Verify a disk image, checking structure, filesystem, and data integrity.
pub fn verify(image: &DiskImage) -> VerifyReport {
    let mut issues = Vec::new();

    check_load_warnings(image, &mut issues);
    check_spec_mismatch(image, &mut issues);
    check_structure(image, &mut issues);
    check_filesystem(image, &mut issues);

    issues.sort_by(|a, b| b.severity.cmp(&a.severity));

    VerifyReport { issues }
}

fn add(issues: &mut Vec<VerifyIssue>, severity: Severity, location: IssueLocation, message: impl Into<String>) {
    issues.push(VerifyIssue {
        severity,
        location,
        message: message.into(),
    });
}

fn check_load_warnings(image: &DiskImage, issues: &mut Vec<VerifyIssue>) {
    for warning in image.warnings() {
        add(issues, Severity::Warning, IssueLocation::Image, warning.clone());
    }
}

fn check_spec_mismatch(image: &DiskImage, issues: &mut Vec<VerifyIssue>) {
    let spec = image.spec();
    let expected_tracks = spec.num_tracks as usize;
    let expected_sectors = spec.sectors_per_track as usize;

    for (side_idx, disk) in image.disks().iter().enumerate() {
        let actual_tracks = disk.track_count();
        if actual_tracks != expected_tracks {
            add(
                issues,
                Severity::Warning,
                IssueLocation::Side(side_idx),
                format!("Expected {} tracks, found {}", expected_tracks, actual_tracks),
            );
        }

        for (track_idx, track) in disk.tracks().iter().enumerate() {
            let actual_sectors = track.sector_count();
            if actual_sectors != expected_sectors && actual_sectors > 0 {
                add(
                    issues,
                    Severity::Info,
                    IssueLocation::Track(side_idx, track_idx as u8),
                    format!("Expected {} sectors, found {}", expected_sectors, actual_sectors),
                );
            }
        }
    }
}

fn check_structure(image: &DiskImage, issues: &mut Vec<VerifyIssue>) {
    let spec = image.spec();

    for (side_idx, disk) in image.disks().iter().enumerate() {
        let mut prev_had_sectors = true;
        for (track_idx, track) in disk.tracks().iter().enumerate() {
            let track_num = track_idx as u8;

            if track.is_empty() {
                if track_idx < spec.num_tracks as usize
                    && prev_had_sectors
                    && track_idx > 0
                {
                    add(
                        issues,
                        Severity::Warning,
                        IssueLocation::Track(side_idx, track_num),
                        "Unformatted track (no sectors)",
                    );
                }
                prev_had_sectors = false;
                continue;
            }

            prev_had_sectors = true;

            check_duplicate_sector_ids(side_idx, track_num, track.sectors(), issues);
            check_chr_consistency(side_idx, track_num, track.sectors(), issues);

            let filler = track.filler_byte;
            for sector in track.sectors() {
                check_sector_size(side_idx, track_num, sector, issues);
                check_fdc_errors(side_idx, track_num, sector, issues);
                check_deleted_mark(side_idx, track_num, sector, issues);
                check_odd_filler(side_idx, track_num, sector, filler, issues);
            }
        }
    }
}

fn check_duplicate_sector_ids(
    side: usize,
    track: u8,
    sectors: &[crate::image::Sector],
    issues: &mut Vec<VerifyIssue>,
) {
    let mut seen: HashMap<u8, usize> = HashMap::new();
    for sector in sectors {
        let id = sector.id.sector;
        if let Some(&count) = seen.get(&id) {
            add(
                issues,
                Severity::Error,
                IssueLocation::Sector(side, track, id),
                format!("Duplicate sector ID 0x{:02X} ({} occurrences)", id, count + 1),
            );
        }
        *seen.entry(id).or_insert(0) += 1;
    }
}

fn check_chr_consistency(
    side: usize,
    track: u8,
    sectors: &[crate::image::Sector],
    issues: &mut Vec<VerifyIssue>,
) {
    for sector in sectors {
        if sector.id.track != track {
            add(
                issues,
                Severity::Warning,
                IssueLocation::Sector(side, track, sector.id.sector),
                format!("CHRN track {} doesn't match physical track {}", sector.id.track, track),
            );
        }
        if sector.id.side as usize != side {
            add(
                issues,
                Severity::Warning,
                IssueLocation::Sector(side, track, sector.id.sector),
                format!("CHRN side {} doesn't match physical side {}", sector.id.side, side),
            );
        }
    }
}

fn check_sector_size(
    side: usize,
    track: u8,
    sector: &crate::image::Sector,
    issues: &mut Vec<VerifyIssue>,
) {
    if sector.has_size_mismatch() {
        add(
            issues,
            Severity::Warning,
            IssueLocation::Sector(side, track, sector.id.sector),
            format!(
                "Size mismatch: advertised {} bytes, actual {} bytes",
                sector.advertised_size(),
                sector.actual_size()
            ),
        );
    }
}

fn check_fdc_errors(
    side: usize,
    track: u8,
    sector: &crate::image::Sector,
    issues: &mut Vec<VerifyIssue>,
) {
    let st1 = &sector.fdc_status1;
    let st2 = &sector.fdc_status2;

    if st1.data_error() {
        add(
            issues,
            Severity::Error,
            IssueLocation::Sector(side, track, sector.id.sector),
            "FDC data error (ST1: DE) — CRC error in ID or data field".to_string(),
        );
    }

    if st1.overrun() {
        add(
            issues,
            Severity::Error,
            IssueLocation::Sector(side, track, sector.id.sector),
            "FDC overrun (ST1: OR)",
        );
    }

    if st1.no_data() {
        add(
            issues,
            Severity::Error,
            IssueLocation::Sector(side, track, sector.id.sector),
            "FDC no data (ST1: ND) — sector not found",
        );
    }

    if st1.not_writable() {
        add(
            issues,
            Severity::Warning,
            IssueLocation::Sector(side, track, sector.id.sector),
            "FDC not writable (ST1: NW) — write protected",
        );
    }

    if st1.missing_address_mark() {
        add(
            issues,
            Severity::Error,
            IssueLocation::Sector(side, track, sector.id.sector),
            "FDC missing address mark (ST1: MA)",
        );
    }

    if st1.end_of_cylinder() {
        add(
            issues,
            Severity::Info,
            IssueLocation::Sector(side, track, sector.id.sector),
            "FDC end of cylinder (ST1: EN)",
        );
    }

    if st2.data_field_error() {
        add(
            issues,
            Severity::Error,
            IssueLocation::Sector(side, track, sector.id.sector),
            "FDC data field error (ST2: DD) — CRC error in data field",
        );
    }

    if st2.wrong_cylinder() {
        add(
            issues,
            Severity::Warning,
            IssueLocation::Sector(side, track, sector.id.sector),
            "FDC wrong cylinder (ST2: WC)",
        );
    }

    if st2.bad_cylinder() {
        add(
            issues,
            Severity::Warning,
            IssueLocation::Sector(side, track, sector.id.sector),
            "FDC bad cylinder (ST2: BC) — cylinder 0xFF",
        );
    }

    if st2.missing_data_mark() {
        add(
            issues,
            Severity::Error,
            IssueLocation::Sector(side, track, sector.id.sector),
            "FDC missing data address mark (ST2: MD)",
        );
    }
}

fn check_deleted_mark(
    side: usize,
    track: u8,
    sector: &crate::image::Sector,
    issues: &mut Vec<VerifyIssue>,
) {
    if sector.is_deleted() {
        add(
            issues,
            Severity::Info,
            IssueLocation::Sector(side, track, sector.id.sector),
            "Deleted data address mark (ST2: CM)",
        );
    }
}

fn check_odd_filler(
    side: usize,
    track: u8,
    sector: &crate::image::Sector,
    filler: u8,
    issues: &mut Vec<VerifyIssue>,
) {
    if sector.status(filler) == SectorStatus::FormattedOddFiller {
        add(
            issues,
            Severity::Info,
            IssueLocation::Sector(side, track, sector.id.sector),
            format!("Filled with 0x{:02X} (expected filler 0x{:02X})", sector.data()[0], filler),
        );
    }
}

fn check_filesystem(image: &DiskImage, issues: &mut Vec<VerifyIssue>) {
    let effective_fs = match image.default_filesystem() {
        FileSystemType::Auto => return,
        FileSystemType::Cpm => FileSystemType::Cpm,
        FileSystemType::Mgt => FileSystemType::Mgt,
        FileSystemType::Trdos => FileSystemType::Trdos,
    };

    match effective_fs {
        FileSystemType::Cpm => check_cpm_filesystem(image, issues),
        FileSystemType::Mgt => check_mgt_filesystem(image, issues),
        FileSystemType::Trdos => check_trdos_filesystem(image, issues),
        FileSystemType::Auto => {}
    }
}

fn check_trdos_filesystem(image: &DiskImage, issues: &mut Vec<VerifyIssue>) {
    let fs = match crate::filesystem::TrdosFileSystem::new(image) {
        Ok(fs) => fs,
        Err(_) => {
            add(issues, Severity::Warning, IssueLocation::Filesystem, "Could not mount TR-DOS filesystem");
            return;
        }
    };

    let total_sectors = image.spec().num_tracks as usize
        * image.spec().num_sides as usize
        * crate::filesystem::trdos::TRD_SECTORS_PER_TRACK as usize;

    for entry in fs.directory() {
        if entry.deleted {
            continue;
        }

        let start = entry.first_absolute_sector() as usize;
        let end = start.saturating_add(entry.sector_count as usize);
        if end > total_sectors {
            add(
                issues,
                Severity::Error,
                IssueLocation::File(entry.filename.clone()),
                format!(
                    "File starts at sector {} and needs {} sectors, past the {}-sector disk",
                    start, entry.sector_count, total_sectors
                ),
            );
            continue;
        }

        if entry.data_length as usize > entry.sector_count as usize * 256 {
            add(
                issues,
                Severity::Warning,
                IssueLocation::File(entry.filename.clone()),
                format!(
                    "File length {} exceeds the {} bytes allocated",
                    entry.data_length,
                    entry.sector_count as usize * 256
                ),
            );
        }
    }

    if let Some(catalog) = fs.catalog() {
        let live: usize = fs.directory().iter().filter(|e| !e.deleted).count();
        if catalog.num_files as usize != live {
            add(
                issues,
                Severity::Warning,
                IssueLocation::Filesystem,
                format!("Catalog records {} files but {} entries are present", catalog.num_files, live),
            );
        }
    }
}

fn check_cpm_filesystem(image: &DiskImage, issues: &mut Vec<VerifyIssue>) {
    let spec = DiskSpecification::identify(image);
    let fs = match crate::filesystem::CpmFileSystem::new(image, spec.clone()) {
        Ok(fs) => fs,
        Err(_) => {
            add(issues, Severity::Warning, IssueLocation::Filesystem, "Could not mount CP/M filesystem");
            return;
        }
    };

    let max_block = spec.block_count();

    let entries = match fs.read_dir_extended_with_deleted() {
        Ok(e) => e,
        Err(e) => {
            add(
                issues,
                Severity::Error,
                IssueLocation::Filesystem,
                format!("Failed to read directory: {}", e),
            );
            return;
        }
    };

    let mut file_blocks: HashMap<String, usize> = HashMap::new();
    let mut deleted_count = 0usize;

    for entry in &entries {
        let is_deleted = entry.user == 0xE5;
        if is_deleted {
            deleted_count += 1;
            continue;
        }

        if entry.name.trim().is_empty() {
            continue;
        }

        if entry.header.header_type != crate::filesystem::HeaderType::None && !entry.header.checksum_valid {
            add(
                issues,
                Severity::Warning,
                IssueLocation::File(entry.name.clone()),
                format!("{:?} header checksum invalid", entry.header.header_type),
            );
        }

        if entry.size > entry.allocated {
            add(
                issues,
                Severity::Warning,
                IssueLocation::File(entry.name.clone()),
                format!(
                    "File size {} bytes exceeds allocated {} bytes",
                    entry.size, entry.allocated
                ),
            );
        }

        if max_block > 0 && entry.blocks > 0 {
            let estimated_max_blocks = (entry.allocated / spec.block_size().max(1)).max(entry.blocks);
            if estimated_max_blocks > max_block as usize {
                add(
                    issues,
                    Severity::Warning,
                    IssueLocation::File(entry.name.clone()),
                    format!("File uses {} blocks (max {} on disk)", entry.blocks, max_block),
                );
            }
        }

        // Entries from users other than 0 must be addressed with their prefix,
        // otherwise the lookup resolves to user 0 and fails.
        let qualified_name = if entry.user == 0 {
            entry.name.clone()
        } else {
            format!("{}:{}", entry.user, entry.name)
        };

        match fs.read_file(&qualified_name) {
            Ok(data) => {
                let header = crate::filesystem::try_parse_header(&data);
                if header.header_type != crate::filesystem::HeaderType::None && !header.checksum_valid {
                    add(
                        issues,
                        Severity::Info,
                        IssueLocation::File(entry.name.clone()),
                        format!("{:?} header checksum mismatch in file data", header.header_type),
                    );
                }
                let _ = data;
            }
            Err(e) => {
                add(
                    issues,
                    Severity::Error,
                    IssueLocation::File(entry.name.clone()),
                    format!("Failed to read file: {}", e),
                );
            }
        }

        file_blocks.insert(entry.name.clone(), entry.blocks);    }

    let fs_info = fs.info();
    let total_blocks = fs_info.total_blocks;
    let used_blocks: usize = file_blocks.values().sum();
    if used_blocks > total_blocks {
        add(
            issues,
            Severity::Error,
            IssueLocation::Filesystem,
            format!("Total file blocks ({}) exceeds disk blocks ({})", used_blocks, total_blocks),
        );
    }

    if deleted_count > 0 {
        add(
            issues,
            Severity::Info,
            IssueLocation::Filesystem,
            format!("{} deleted file(s) found", deleted_count),
        );
    }
}

fn check_mgt_filesystem(image: &DiskImage, issues: &mut Vec<VerifyIssue>) {
    // DiscipleFileSystem mounts any MGT image, so trying it first always
    // succeeded and every SAM disk was labelled DISCiPLE. Use the detected
    // system type instead.
    let fs = match crate::filesystem::MgtFileSystem::new(image) {
        Ok(fs) => fs,
        Err(_) => {
            add(issues, Severity::Warning, IssueLocation::Filesystem, "Could not mount MGT filesystem");
            return;
        }
    };

    let system_name = match fs.system_type() {
        crate::filesystem::MgtSystemType::Sam => "SAM Coupe",
        crate::filesystem::MgtSystemType::Disciple => "DISCiPLE/+D",
        crate::filesystem::MgtSystemType::Unknown => "MGT",
    };
    let info = fs.info();
    let dir_entries: Vec<_> = fs.directory().to_vec();

    for entry in &dir_entries {
        if entry.hidden {
            add(
                issues,
                Severity::Info,
                IssueLocation::File(entry.filename.clone()),
                "Hidden file",
            );
        }

        if entry.sectors_used == 0 {
            add(
                issues,
                Severity::Warning,
                IssueLocation::File(entry.filename.clone()),
                "File has zero sectors",
            );
        }
    }

    if dir_entries.is_empty() && info.total_sectors > 0 {
        add(
            issues,
            Severity::Info,
            IssueLocation::Filesystem,
            format!("No files found on {} filesystem", system_name),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::format::FormatSpec;

    #[test]
    fn test_verify_blank_image() {
        let image = DiskImage::builder()
            .spec(FormatSpec::amstrad_data())
            .build()
            .unwrap();
        let report = verify(&image);
        assert!(report.passed());
    }

    #[test]
    fn test_verify_reports_size_mismatch() {
        let mut image = DiskImage::builder()
            .num_sides(1)
            .num_tracks(1)
            .sectors_per_track(2)
            .build()
            .unwrap();

        let disk = image.get_disk_mut(0).unwrap();
        let track = disk.get_track_mut(0).unwrap();
        if let Some(sector) = track.get_sector_by_index_mut(0) {
            sector.resize(64, 0x00);
        }

        let report = verify(&image);
        assert!(report.issues.iter().any(|i| i.message.contains("Size mismatch")));
    }

    #[test]
    fn test_verify_reads_files_belonging_to_other_users() {
        // A file imported as "2:HIDDEN.BIN" lives in user 2. Reading it by bare
        // name resolved to user 0, so verify reported "File not found" and
        // failed a perfectly good disk.
        use crate::filesystem::import_file;
        use crate::filesystem::FileSystemType;

        let mut image = DiskImage::create(FormatSpec::amstrad_data()).unwrap();
        crate::filesystem::format_filesystem(&mut image, FileSystemType::Cpm).unwrap();
        import_file(&mut image, FileSystemType::Cpm, "2:HIDDEN.BIN", &[0x41; 400]).unwrap();

        let report = verify(&image);
        assert!(
            report.error_count() == 0,
            "unexpected errors: {:?}",
            report.issues.iter().map(|i| &i.message).collect::<Vec<_>>()
        );
    }

    #[test]
    fn test_verify_names_the_detected_mgt_variant() {
        // check_mgt_filesystem always tried DiscipleFileSystem first, which
        // succeeds for any MGT image, so SAM disks were labelled DISCiPLE/+D.
        use crate::format::{DiskImageFormat, FormatSpec};

        let image = DiskImage::builder()
            .format(DiskImageFormat::RawMgt)
            .spec(FormatSpec::new(2, 80, 10, 512).with_first_sector_id(1).with_filler_byte(0))
            .build()
            .unwrap();
        let spec = DiskSpecification::identify(&image);
        assert!(spec.format.starts_with("MGT Sam Coupe"), "got {:?}", spec.format);

        let report = verify(&image);
        let named: Vec<String> = report
            .issues
            .iter()
            .map(|i| i.message.clone())
            .filter(|m| m.contains("DISCiPLE") || m.contains("SAM Coupe"))
            .collect();
        assert!(
            named.iter().all(|m| !m.contains("DISCiPLE")),
            "SAM disk reported as DISCiPLE: {named:?}"
        );
    }

    #[test]
    fn test_verify_detects_trdos_directory_overrun() {
        // TR-DOS verification did nothing, so a directory entry pointing past
        // the end of the disk "passed" while read_file wrapped around and
        // returned garbage.
        use crate::filesystem::{format_filesystem, FileSystemType};
        use crate::format::{DiskImageFormat, FormatSpec};

        let mut image = DiskImage::builder()
            .format(DiskImageFormat::RawTrd)
            .spec(FormatSpec::trdos())
            .build()
            .unwrap();
        format_filesystem(&mut image, FileSystemType::Trdos).unwrap();

        let mut sector = image.read_sector(0, 0, 1).unwrap().to_vec();
        sector[..8].copy_from_slice(b"BOGUS   ");
        sector[8] = b'C';
        sector[9..11].copy_from_slice(&51200u16.to_le_bytes());
        sector[13] = 200; // 200 sectors from track 70 runs off the disk
        sector[14] = 0;
        sector[15] = 70;
        image.write_sector(0, 0, 1, &sector).unwrap();

        let report = verify(&image);
        assert!(
            report.issues.iter().any(|i| i.location.to_string().contains("BOGUS")),
            "expected the overrunning file to be reported, got {:?}",
            report.issues.iter().map(|i| &i.message).collect::<Vec<_>>()
        );
        assert!(!report.passed());
    }

    #[test]
    fn test_verify_report_display() {        let mut report = VerifyReport { issues: vec![] };
        add(&mut report.issues, Severity::Error, IssueLocation::Sector(0, 0, 0xC1), "Test error");
        let s = format!("{}", report);
        assert!(s.contains("ERROR"));
        assert!(s.contains("FAILED"));
    }
}
