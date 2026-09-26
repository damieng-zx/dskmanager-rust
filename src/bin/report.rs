//! Batch report generation for the `dsk report` subcommand.
//!
//! Walks a directory tree of disk images (including images inside `.zip`
//! archives) and emits a report in one of two formats:
//!
//! - `csv` — one row per disk: format, protection, structural fingerprint and
//!   per-track characteristics. Good for sorting/pivoting a whole collection in
//!   a spreadsheet.
//! - `markdown` — grouped by top-level subfolder, an h3 per disk with format,
//!   detected protection and a bulleted list of structural quirks.
//!
//! Usage: dsk report <root-dir> [output] [--format csv|markdown]
//!
//! If no output path is given, the report is written to stdout. When a format
//! is not specified explicitly it is inferred from the output extension
//! (`.csv` / `.md`), defaulting to CSV.

use std::collections::BTreeMap;
use std::cell::RefCell;
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};


use dskmanager::*;

thread_local! {
    static CURRENT_FILE: RefCell<String> = RefCell::new(String::new());
}

fn set_current_file(path: &str) {
    CURRENT_FILE.with(|f| *f.borrow_mut() = path.to_string());
}

fn install_panic_hook() {
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let file = CURRENT_FILE.with(|f| f.borrow().clone());
        if !file.is_empty() {
            eprintln!("\nPanic while processing: {}", file);
        }
        default_hook(info);
    }));
}

/// Output format for the report.
enum Format {
    Csv,
    Markdown,
}

impl Format {
    fn parse(s: &str) -> Option<Format> {
        match s.to_ascii_lowercase().as_str() {
            "csv" => Some(Format::Csv),
            "md" | "markdown" => Some(Format::Markdown),
            _ => None,
        }
    }
}

#[derive(Default)]
struct Counts {
    images: usize,
    images_from_zips: usize,
    errors: usize,
}

struct Filters {
    format: Option<String>,
    tracks: Option<usize>,
    sides: Option<usize>,
    sector_size: Option<usize>,
    protection: Option<Option<String>>,
    has_errors: bool,
    quirks: bool,
}

impl Filters {
    fn matches(&self, image: &DiskImage) -> bool {
        if self.format.is_none()
            && self.tracks.is_none()
            && self.sides.is_none()
            && self.sector_size.is_none()
            && self.protection.is_none()
            && !self.has_errors
            && !self.quirks
        {
            return true;
        }

        if let Some(ref fmt) = self.format {
            let spec = DiskSpecification::identify(image);
            if !report_format(image, &spec).to_ascii_lowercase().contains(&fmt.to_ascii_lowercase())
                && !image.format().name().to_ascii_lowercase().contains(&fmt.to_ascii_lowercase()) {
                return false;
            }
        }

        if let Some(sides) = self.sides {
            if image.disks().len() != sides {
                return false;
            }
        }

        for disk in image.disks() {
            if let Some(tracks) = self.tracks {
                if disk.track_count() != tracks {
                    return false;
                }
            }

            if let Some(sector_size) = self.sector_size {
                if let Some(t0) = disk.get_track(0) {
                    if let Some(s0) = t0.get_sector_by_index(0) {
                        if s0.advertised_size() != sector_size {
                            return false;
                        }
                    }
                }
            }

            if let Some(ref prot_filter) = self.protection {
                let detected = protection::detect(disk);
                match prot_filter {
                    None => {
                        if detected.is_none() {
                            return false;
                        }
                    }
                    Some(ref text) => {
                        let name = detected.as_ref().map(|d| d.name.as_str()).unwrap_or("");
                        if !name.to_ascii_lowercase().contains(&text.to_ascii_lowercase()) {
                            return false;
                        }
                    }
                }
            }

            if self.has_errors {
                let mut found = false;
                for t_idx in 0..disk.track_count() {
                    if let Some(track) = disk.get_track(t_idx as u8) {
                        for s_idx in 0..track.sector_count() {
                            if let Some(sector) = track.get_sector_by_index(s_idx) {
                                if sector.has_error() {
                                    found = true;
                                    break;
                                }
                            }
                        }
                    }
                    if found {
                        break;
                    }
                }
                if !found {
                    return false;
                }
            }

            if self.quirks {
                let spec = DiskSpecification::identify(image);
                let quirks = md_quirks(image, &spec);
                if quirks.is_empty() {
                    return false;
                }
            }
        }

        true
    }
}

/// CSV header — keep in sync with `csv_row`.
const CSV_HEADER: &str = "file,format,format_source,protection,protection_reason,fingerprint,tracks,sides,sectors_per_track,sector_size,first_sector_id,is_uniform,has_fdc_errors,track_layout,nine_sector_tracks,non_nine_tracks,empty_tracks,biggest_track_bytes,sector_count_pattern,bootable_on,boot_reason";

/// Entry point for `dsk report`. `args` are the arguments following `report`
/// (i.e. excluding the program name and the `report` subcommand). Returns a
/// process exit code.
pub fn run(args: &[String]) -> i32 {
    let mut positional: Vec<String> = Vec::new();
    let mut format: Option<Format> = None;
    let mut filters = Filters {
        format: None,
        tracks: None,
        sides: None,
        sector_size: None,
        protection: None,
        has_errors: false,
        quirks: false,
    };
    let mut i = 0;
    while i < args.len() {
        let arg = &args[i];
        if arg == "--format" || arg == "-f" {
            i += 1;
            match args.get(i).map(|s| s.as_str()) {
                Some(f) => match Format::parse(f) {
                    Some(parsed) => format = Some(parsed),
                    None => {
                        eprintln!("Unknown format: {} (expected csv or markdown)", f);
                        return 1;
                    }
                },
                None => {
                    eprintln!("--format requires a value (csv or markdown)");
                    return 1;
                }
            }
        } else if let Some(f) = arg.strip_prefix("--format=") {
            match Format::parse(f) {
                Some(parsed) => format = Some(parsed),
                None => {
                    eprintln!("Unknown format: {} (expected csv or markdown)", f);
                    return 1;
                }
            }
        } else if arg == "--filter-format" {
            i += 1;
            match args.get(i) {
                Some(v) => filters.format = Some(v.clone()),
                None => {
                    eprintln!("--filter-format requires a value");
                    return 1;
                }
            }
        } else if let Some(v) = arg.strip_prefix("--filter-format=") {
            filters.format = Some(v.to_string());
        } else if arg == "--filter-tracks" {
            i += 1;
            match args.get(i).and_then(|v| v.parse().ok()) {
                Some(v) => filters.tracks = Some(v),
                None => {
                    eprintln!("--filter-tracks requires a number");
                    return 1;
                }
            }
        } else if let Some(v) = arg.strip_prefix("--filter-tracks=") {
            match v.parse() {
                Ok(n) => filters.tracks = Some(n),
                Err(_) => {
                    eprintln!("--filter-tracks requires a number");
                    return 1;
                }
            }
        } else if arg == "--filter-sides" {
            i += 1;
            match args.get(i).and_then(|v| v.parse().ok()) {
                Some(v) => filters.sides = Some(v),
                None => {
                    eprintln!("--filter-sides requires a number");
                    return 1;
                }
            }
        } else if let Some(v) = arg.strip_prefix("--filter-sides=") {
            match v.parse() {
                Ok(n) => filters.sides = Some(n),
                Err(_) => {
                    eprintln!("--filter-sides requires a number");
                    return 1;
                }
            }
        } else if arg == "--filter-sector-size" {
            i += 1;
            match args.get(i).and_then(|v| v.parse().ok()) {
                Some(v) => filters.sector_size = Some(v),
                None => {
                    eprintln!("--filter-sector-size requires a number");
                    return 1;
                }
            }
        } else if let Some(v) = arg.strip_prefix("--filter-sector-size=") {
            match v.parse() {
                Ok(n) => filters.sector_size = Some(n),
                Err(_) => {
                    eprintln!("--filter-sector-size requires a number");
                    return 1;
                }
            }
        } else if arg == "--filter-protection" {
            i += 1;
            match args.get(i) {
                Some(v) if !v.starts_with('-') => {
                    filters.protection = Some(Some(v.clone()));
                }
                _ => {
                    filters.protection = Some(None);
                    i = i.saturating_sub(1);
                }
            }
        } else if let Some(v) = arg.strip_prefix("--filter-protection=") {
            filters.protection = Some(Some(v.to_string()));
        } else if arg == "--filter-has-errors" {
            filters.has_errors = true;
        } else if arg == "--filter-quirks" {
            filters.quirks = true;
        } else {
            positional.push(arg.clone());
        }
        i += 1;
    }

    let pattern = match positional.first() {
        Some(p) => p.clone(),
        None => {
            eprintln!("Usage: dsk report <pattern> [output] [--format csv|markdown] [filters]");
            return 1;
        }
    };
    let output = positional.get(1);

    // Resolve format: explicit flag > inferred from output extension > CSV.
    let format = format
        .or_else(|| infer_format(output))
        .unwrap_or(Format::Csv);

    let mut writer: Box<dyn Write> = match output {
        Some(path) => match fs::File::create(path) {
            Ok(file) => Box::new(file),
            Err(e) => {
                eprintln!("Failed to create {}: {}", path, e);
                return 1;
            }
        },
        None => Box::new(std::io::stdout()),
    };

    let mut counts = Counts::default();
    install_panic_hook();

    let files = expand_pattern(&pattern);
    if files.is_empty() {
        eprintln!("No disk images found matching: {}", pattern);
        return 1;
    }
    let root = common_root(&files);

    match format {
        Format::Csv => {
            if writeln!(writer, "{}", CSV_HEADER).is_err() {
                eprintln!("Failed to write output");
                return 1;
            }
            process_files(&root, &files, &filters, &mut counts, &mut |title, _folder, image| {
                let line = match image {
                    Ok(img) => csv_row(&title, &img),
                    Err(e) => csv_error_row(&title, &e),
                };
                let _ = writeln!(writer, "{}", line);
            });
        }
        Format::Markdown => {
            let mut sections: BTreeMap<String, Vec<DiskEntry>> = BTreeMap::new();
            process_files(&root, &files, &filters, &mut counts, &mut |title, folder, image| {
                let entry = match image {
                    Ok(img) => {
                        let mut entry = analyze_image(&img);
                        entry.title = title;
                        entry
                    }
                    Err(e) => DiskEntry {
                        title,
                        format: "Error".to_string(),
                        protection: format!("Failed to parse: {}", e),
                        protection_details: vec![],
                        characteristics: vec![],
                        warnings: vec![],
                        bootable_on: String::new(),
                        boot_reason: String::new(),
                    },
                };
                sections.entry(folder).or_default().push(entry);
            });

            // Sort entries within each section by title for stable output.
            for entries in sections.values_mut() {
                entries.sort_by(|a, b| a.title.to_lowercase().cmp(&b.title.to_lowercase()));
            }

            if write_markdown(&mut writer, &root, &sections, &counts).is_err() {
                eprintln!("Failed to write output");
                return 1;
            }
        }
    }

    eprintln!(
        "Scanned {} disk images ({} from zips), {} errors.",
        counts.images, counts.images_from_zips, counts.errors
    );
    0
}

fn infer_format(output: Option<&String>) -> Option<Format> {
    let lower = output?.to_ascii_lowercase();
    if lower.ends_with(".md") || lower.ends_with(".markdown") {
        Some(Format::Markdown)
    } else if lower.ends_with(".csv") {
        Some(Format::Csv)
    } else {
        None
    }
}

fn report_format(image: &DiskImage, spec: &DiskSpecification) -> String {
    match image.format() {
        DiskImageFormat::RawTrd => "TR-DOS (TRD)".to_string(),
        DiskImageFormat::RawMgt if !spec.format.starts_with("MGT") => "MGT".to_string(),
        DiskImageFormat::RawTd0 => format!("{} [{}]", spec.format, image.format().name()),
        _ => spec.format.clone(),
    }
}

// ---------------------------------------------------------------------------
// Pattern expansion and file processing (shared by both output formats)
// ---------------------------------------------------------------------------

/// Expand a pattern into a sorted list of disk images and `.zip` file paths.
///
/// - If the pattern is a directory, recursively finds all supported images and `.zip` files.
/// - If the pattern contains `*` or `?`, uses Windows-style glob expansion.
/// - If the pattern is a single file, returns it as-is.
fn expand_pattern(pattern: &str) -> Vec<PathBuf> {
    let path = PathBuf::from(pattern);

    if path.is_dir() {
        return walk_dir(&path);
    }

    if pattern.contains('*') || pattern.contains('?') {
        return expand_glob(pattern);
    }

    if path.is_file() {
        vec![path]
    } else {
        eprintln!("Path not found: {}", pattern);
        vec![]
    }
}

fn is_image_extension(extension: &str) -> bool {
    matches!(extension.to_ascii_lowercase().as_str(), "dsk" | "mgt" | "trd" | "td0")
}

fn walk_dir(dir: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return files,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if name == ".AppleDouble" || name == "__MACOSX" || name.starts_with("._") {
            continue;
        }
        if path.is_dir() {
            files.extend(walk_dir(&path));
            continue;
        }
        let ext = path.extension().and_then(|e| e.to_str()).map(|s| s.to_ascii_lowercase());
        if ext.as_deref().map(is_image_extension).unwrap_or(false) || ext.as_deref() == Some("zip") {
            files.push(path);
        }
    }
    files.sort();
    files
}

fn expand_glob(pattern: &str) -> Vec<PathBuf> {
    let mut files = Vec::new();

    let root = glob_root(pattern);
    if !root.is_dir() {
        eprintln!("Glob base directory not found: {}", root.display());
        return files;
    }

    let mut cmd = std::process::Command::new("cmd");
    cmd.args(["/C", "dir", "/B", "/S", pattern]);
    let output = match cmd.output() {
        Ok(o) => o,
        Err(_) => return files,
    };
    let stdout = String::from_utf8_lossy(&output.stdout);
    for line in stdout.lines() {
        let path = PathBuf::from(line.trim());
        if !path.is_file() {
            continue;
        }
        let ext = path.extension().and_then(|e| e.to_str()).map(|s| s.to_ascii_lowercase());
        if ext.as_deref().map(is_image_extension).unwrap_or(false) || ext.as_deref() == Some("zip") {
            files.push(path);
        }
    }
    files.sort();
    files
}

fn glob_root(pattern: &str) -> PathBuf {
    let path = PathBuf::from(pattern);
    let mut root = PathBuf::new();
    for component in path.components() {
        let s = component.as_os_str().to_string_lossy();
        if s.contains('*') || s.contains('?') {
            break;
        }
        root.push(component);
    }
    if root.as_os_str().is_empty() {
        PathBuf::from(".")
    } else {
        root
    }
}

fn common_root(files: &[PathBuf]) -> PathBuf {
    if files.is_empty() {
        return PathBuf::from(".");
    }
    let mut root = files[0].clone();
    root.pop();
    if root.as_os_str().is_empty() {
        PathBuf::from(".")
    } else {
        root
    }
}

fn process_files(
    root: &Path,
    files: &[PathBuf],
    filters: &Filters,
    counts: &mut Counts,
    emit: &mut dyn FnMut(String, String, std::result::Result<DiskImage, String>),
) {
    for path in files {
        let ext = path.extension().and_then(|e| e.to_str()).map(|s| s.to_ascii_lowercase());
        match ext.as_deref() {
            Some(ext) if is_image_extension(ext) => {
                counts.images += 1;
                set_current_file(&path.display().to_string());
                let image = DiskImage::open(path).map_err(|e| e.to_string());
                if let Ok(ref img) = image {
                    if !filters.matches(img) {
                        continue;
                    }
                }
                if image.is_err() {
                    counts.errors += 1;
                }
                emit(relative_display(root, path), top_folder(root, path), image);
                progress(counts);
            }
            Some("zip") => {
                scan_zip(root, path, filters, counts, emit);
            }
            _ => {}
        }
    }
}

fn progress(counts: &Counts) {
    if counts.images.is_multiple_of(250) {
        eprintln!("  {} disk images processed...", counts.images);
    }
}

fn scan_zip(
    root: &Path,
    zip_path: &Path,
    filters: &Filters,
    counts: &mut Counts,
    emit: &mut dyn FnMut(String, String, std::result::Result<DiskImage, String>),
) {
    let file = match fs::File::open(zip_path) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("zip open {}: {}", zip_path.display(), e);
            return;
        }
    };
    let mut archive = match zip::ZipArchive::new(file) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("zip read {}: {}", zip_path.display(), e);
            return;
        }
    };

    let zip_rel = relative_display(root, zip_path);
    let folder = top_folder(root, zip_path);

    for i in 0..archive.len() {
        let mut zentry = match archive.by_index(i) {
            Ok(e) => e,
            Err(_) => continue,
        };
        if !zentry.is_file() {
            continue;
        }
        let name = zentry.name().to_string();
        let extension = match Path::new(&name).extension().and_then(|e| e.to_str()) {
            Some(ext) if is_image_extension(ext) => ext,
            _ => continue,
        };
        let mut bytes = Vec::with_capacity(zentry.size() as usize);
        if zentry.read_to_end(&mut bytes).is_err() {
            continue;
        }
        counts.images += 1;
        counts.images_from_zips += 1;
        let title = format!("{} → {}", zip_rel, name);
        set_current_file(&title);
        let image = open_bytes(&bytes, extension);
        if let Ok(ref img) = image {
            if !filters.matches(img) {
                continue;
            }
        }
        if image.is_err() {
            counts.errors += 1;
        }
        emit(title, folder.clone(), image);
        progress(counts);
    }
}

/// Open a disk image from raw bytes by round-tripping through a temp file, since
/// `DiskImage::open` works from a path.
fn open_bytes(bytes: &[u8], extension: &str) -> std::result::Result<DiskImage, String> {
    let tmp = std::env::temp_dir().join(format!(
        "dskreport-{}-{}.{}",
        std::process::id(),
        rand_suffix(),
        extension
    ));
    fs::write(&tmp, bytes).map_err(|e| e.to_string())?;
    let result = DiskImage::open(&tmp).map_err(|e| e.to_string());
    let _ = fs::remove_file(&tmp);
    result
}

#[cfg(test)]
mod format_tests {
    use super::*;

    #[test]
    fn recognizes_all_image_extensions() {
        for extension in ["dsk", "MGT", "TrD", "td0"] {
            assert!(is_image_extension(extension));
        }
        assert!(!is_image_extension("txt"));
        assert_eq!(open_bytes(&vec![0; 40 * 16 * 256], "trd").unwrap().format(), DiskImageFormat::RawTrd);
        assert_eq!(open_bytes(&vec![0; 819_200], "mgt").unwrap().format(), DiskImageFormat::RawMgt);
    }

    #[test]
    fn scans_trd_images_inside_zip_archives() {
        let path = std::env::temp_dir().join(format!("dskreport_formats_{}.zip", std::process::id()));
        let file = fs::File::create(&path).unwrap();
        let mut writer = zip::ZipWriter::new(file);
        writer.start_file("disk.trd", zip::write::SimpleFileOptions::default()).unwrap();
        writer.write_all(&vec![0; 40 * 16 * 256]).unwrap();
        writer.finish().unwrap();

        let filters = Filters {
            format: None, tracks: None, sides: None, sector_size: None,
            protection: None, has_errors: false, quirks: false,
        };
        let mut counts = Counts::default();
        let mut formats = Vec::new();
        scan_zip(&std::env::temp_dir(), &path, &filters, &mut counts, &mut |_, _, image| {
            formats.push(image.unwrap().format());
        });
        fs::remove_file(path).ok();
        assert_eq!(formats, vec![DiskImageFormat::RawTrd]);
        assert_eq!(counts.images_from_zips, 1);
    }
}

fn rand_suffix() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let n = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("{:x}", n)
}

fn top_folder(root: &Path, path: &Path) -> String {
    let rel = path.strip_prefix(root).unwrap_or(path);
    let mut comps = rel.components();
    match comps.next() {
        Some(first) => {
            if comps.next().is_some() {
                first.as_os_str().to_string_lossy().into_owned()
            } else {
                "(root)".to_string()
            }
        }
        None => "(root)".to_string(),
    }
}

fn relative_display(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

// ---------------------------------------------------------------------------
// CSV output
// ---------------------------------------------------------------------------

fn csv_row(title: &str, image: &DiskImage) -> String {
    let spec = DiskSpecification::identify(image);

    let mut protections = Vec::new();
    for disk in image.disks() {
        if let Some(prot) = protection::detect(disk) {
            let mut entry = format!("{} | {}", prot.name, prot.reason);
            if !prot.details.is_empty() {
                entry.push_str(" | ");
                entry.push_str(&prot.details.join(" | "));
            }
            protections.push(entry);
        }
    }
    let protection_str = if protections.is_empty() {
        String::new()
    } else {
        protections.join(" ; ")
    };

    let chars = compute_csv_chars(image);
    let fingerprint = chars.fingerprint();
    let file = title.replace(',', ";");

    let boot = BootDetection::detect(image);
    let bootable_on = if boot.system.is_empty() {
        "Not bootable"
    } else {
        &boot.system
    };

    format!(
        "{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{}",
        file,
        escape_csv(&report_format(image, &spec)),
        escape_csv(&spec.source),
        escape_csv(&protection_str),
        escape_csv(&protection_str),
        escape_csv(&fingerprint),
        chars.total_tracks,
        chars.num_sides,
        chars.sectors_per_track,
        chars.sector_size,
        chars.first_sector_id,
        chars.is_uniform,
        chars.has_fdc_errors,
        escape_csv(&chars.track_layout),
        chars.nine_sector_tracks,
        chars.non_nine_tracks,
        chars.empty_tracks,
        chars.biggest_track_bytes,
        escape_csv(&chars.sector_count_pattern),
        escape_csv(bootable_on),
        escape_csv(&boot.reason),
    )
}

fn csv_error_row(title: &str, err: &str) -> String {
    let file = title.replace(',', ";");
    format!("{},ERROR,ERROR,ERROR,{},ERROR,,,,,,,ERROR,,,,,,,,", file, err)
}

fn escape_csv(s: &str) -> String {
    if s.contains(',') || s.contains('"') || s.contains('\n') {
        let escaped = s.replace('"', "\"\"");
        format!("\"{}\"", escaped)
    } else {
        s.to_string()
    }
}

#[derive(Debug, Clone)]
struct CsvChars {
    total_tracks: usize,
    num_sides: usize,
    sectors_per_track: String,
    sector_size: String,
    first_sector_id: String,
    is_uniform: bool,
    has_fdc_errors: bool,
    track_layout: String,
    nine_sector_tracks: usize,
    non_nine_tracks: String,
    empty_tracks: String,
    biggest_track_bytes: usize,
    sector_count_pattern: String,
}

impl CsvChars {
    fn fingerprint(&self) -> String {
        let size_key = match (self.sectors_per_track.as_str(), self.sector_size.as_str()) {
            ("9", "512") => "9x512",
            ("10", "512") => "10x512",
            ("9", "256") => "9x256",
            ("5", "1024") => "5x1K",
            ("16", "256") => "16x256",
            ("8", "512") => "8x512",
            _ => &format!("{}x{}", self.sectors_per_track, self.sector_size),
        };

        let track_key = if self.total_tracks > 0 {
            format!("{}T", self.total_tracks)
        } else {
            "?T".to_string()
        };

        let side_key = if self.num_sides == 2 { "DS" } else { "SS" };

        let uniform_key = if self.is_uniform { "U" } else { "NU" };
        let error_key = if self.has_fdc_errors { "+ERR" } else { "" };

        let mut parts = vec![
            format!("{}_{}_{}", size_key, track_key, side_key),
            uniform_key.to_string(),
        ];
        if !error_key.is_empty() {
            parts.push(error_key.to_string());
        }

        let sig = parts.join("-");

        let id_key = match self.first_sector_id.as_str() {
            "1" => "id1",
            "0xC1" => "idC1",
            "0x41" => "id41",
            "0" => "id0",
            "41" => "id41",
            _ => "id?",
        };

        format!("{}-{}", sig, id_key)
    }
}

fn compute_csv_chars(image: &DiskImage) -> CsvChars {
    let num_sides = image.disks().len();
    let disk = match image.get_disk(0) {
        Some(d) => d,
        None => return empty_csv_chars(),
    };

    let total_tracks = disk.track_count();

    let mut sector_counts: Vec<usize> = Vec::new();
    let mut sector_sizes: Vec<Option<usize>> = Vec::new();
    let mut first_sector_ids: Vec<u8> = Vec::new();
    let mut empty_track_indices: Vec<usize> = Vec::new();
    let mut non_9_indices: Vec<(usize, usize)> = Vec::new();
    let mut max_track_size = 0usize;
    let mut has_any_fdc_error = false;
    let mut uniform = true;
    let mut first_sector_count: Option<usize> = None;
    let mut first_sector_size: Option<usize> = None;

    for t_idx in 0..total_tracks {
        let track = match disk.get_track(t_idx as u8) {
            Some(t) => t,
            None => continue,
        };

        let sc = track.sector_count();
        sector_counts.push(sc);
        if sc == 0 {
            empty_track_indices.push(t_idx);
        }

        if first_sector_count.is_none() {
            first_sector_count = Some(sc);
        }
        if let Some(fsc) = first_sector_count {
            if sc != fsc {
                uniform = false;
            }
        }

        let size = track.uniform_sector_size();
        sector_sizes.push(size);
        if first_sector_size.is_none() {
            first_sector_size = size;
        }
        if first_sector_size.is_some() && size != first_sector_size && size.is_some() {
            uniform = false;
        }

        if sc != 9 {
            non_9_indices.push((t_idx, sc));
        }

        let track_size = track.total_data_size();
        if track_size > max_track_size {
            max_track_size = track_size;
        }

        if let Some(first_sector) = track.get_sector_by_index(0) {
            first_sector_ids.push(first_sector.id.sector);
        }

        for s_idx in 0..sc {
            if let Some(sector) = track.get_sector_by_index(s_idx) {
                if sector.has_error() {
                    has_any_fdc_error = true;
                }
            }
        }
    }

    let most_common_count = most_common(&sector_counts).unwrap_or(0);
    let most_common_size = most_common_option(&sector_sizes).unwrap_or(None).unwrap_or(0);
    let first_id = first_sector_ids
        .first()
        .copied()
        .map(format_sector_id)
        .unwrap_or_default();

    let empty_ranges = format_index_ranges(&empty_track_indices);

    let non9_str = if non_9_indices.is_empty() {
        String::new()
    } else {
        non_9_indices
            .iter()
            .map(|(t, sc)| format!("T{}:{}s", t, sc))
            .collect::<Vec<_>>()
            .join("; ")
    };

    let layout = build_track_layout(disk, total_tracks);
    let count_pattern = compress_sector_counts(&sector_counts);

    CsvChars {
        total_tracks,
        num_sides,
        sectors_per_track: most_common_count.to_string(),
        sector_size: most_common_size.to_string(),
        first_sector_id: first_id,
        is_uniform: uniform,
        has_fdc_errors: has_any_fdc_error,
        track_layout: layout,
        nine_sector_tracks: sector_counts.iter().filter(|&&c| c == 9).count(),
        non_nine_tracks: non9_str,
        empty_tracks: empty_ranges,
        biggest_track_bytes: max_track_size,
        sector_count_pattern: count_pattern,
    }
}

fn empty_csv_chars() -> CsvChars {
    CsvChars {
        total_tracks: 0,
        num_sides: 0,
        sectors_per_track: String::new(),
        sector_size: String::new(),
        first_sector_id: String::new(),
        is_uniform: false,
        has_fdc_errors: false,
        track_layout: String::new(),
        nine_sector_tracks: 0,
        non_nine_tracks: String::new(),
        empty_tracks: String::new(),
        biggest_track_bytes: 0,
        sector_count_pattern: String::new(),
    }
}

fn most_common<T: PartialEq + Clone>(items: &[T]) -> Option<T> {
    if items.is_empty() {
        return None;
    }
    let mut best: Option<&T> = None;
    let mut best_count = 0usize;
    for item in items {
        let count = items.iter().filter(|&x| x == item).count();
        if count > best_count {
            best = Some(item);
            best_count = count;
        }
    }
    best.cloned()
}

fn most_common_option<T: PartialEq + Clone>(items: &[Option<T>]) -> Option<Option<T>> {
    let non_none: Vec<&T> = items.iter().filter_map(|x| x.as_ref()).collect();
    if non_none.is_empty() {
        return None;
    }
    let best = most_common(&non_none)?;
    Some(Some(best.clone()))
}

fn format_sector_id(id: u8) -> String {
    match id {
        0 => "0".to_string(),
        1..=9 => id.to_string(),
        0x41 => "0x41".to_string(),
        0xC1 => "0xC1".to_string(),
        _ => format!("0x{:02X}", id),
    }
}

/// Compress a sorted-ish list of track indices into comma-separated ranges,
/// e.g. `[0,1,2,5]` -> `"0-2,5"`. Used for the CSV `empty_tracks` column.
fn format_index_ranges(indices: &[usize]) -> String {
    if indices.is_empty() {
        return String::new();
    }
    let mut result = Vec::new();
    let mut start = indices[0];
    let mut end = indices[0];
    for &i in &indices[1..] {
        if i == end + 1 {
            end = i;
        } else {
            if start == end {
                result.push(start.to_string());
            } else {
                result.push(format!("{}-{}", start, end));
            }
            start = i;
            end = i;
        }
    }
    if start == end {
        result.push(start.to_string());
    } else {
        result.push(format!("{}-{}", start, end));
    }
    result.join(",")
}

fn build_track_layout(disk: &Disk, total_tracks: usize) -> String {
    let mut parts = Vec::new();
    let mut i = 0;

    // Group consecutive tracks with the same sector count, sector size and filler.
    while i < total_tracks {
        let track = match disk.get_track(i as u8) {
            Some(t) => t,
            None => break,
        };
        let sc = track.sector_count();
        let sz = track.uniform_sector_size().unwrap_or(0);
        let filler = track.filler_byte;

        let mut j = i + 1;
        while j < total_tracks {
            let next = match disk.get_track(j as u8) {
                Some(t) => t,
                None => break,
            };
            if next.sector_count() == sc
                && next.uniform_sector_size().unwrap_or(0) == sz
                && next.filler_byte == filler
            {
                j += 1;
            } else {
                break;
            }
        }

        let range = if j - i == 1 {
            format!("T{}", i)
        } else {
            format!("T{}-{}", i, j - 1)
        };

        if sc == 0 {
            parts.push(format!("{}=empty", range));
        } else {
            let status = track_status(disk, i);
            parts.push(format!("{}=({}x{},{})", range, sc, sz, status));
        }

        i = j;
    }

    parts.join("|")
}

fn track_status(disk: &Disk, track_idx: usize) -> String {
    let track = match disk.get_track(track_idx as u8) {
        Some(t) => t,
        None => return "?".to_string(),
    };

    let has_error = track.sectors().iter().any(|s| s.has_error());
    let has_deleted = track.sectors().iter().any(|s| s.is_deleted());
    let has_mismatch = track.sectors().iter().any(|s| s.has_size_mismatch());
    let sizes: Vec<String> = track
        .sectors()
        .iter()
        .map(|s| format!("{}", s.actual_size()))
        .collect();
    let all_same_size = sizes.windows(2).all(|w| w[0] == w[1]);

    let mut flags = Vec::new();
    if has_error {
        flags.push("ERR");
    }
    if has_deleted {
        flags.push("DEL");
    }
    if has_mismatch {
        flags.push("SZMIS");
    }
    if !all_same_size {
        flags.push("VAR");
    }

    if flags.is_empty() {
        "ok".to_string()
    } else {
        flags.join("+")
    }
}

fn compress_sector_counts(counts: &[usize]) -> String {
    if counts.is_empty() {
        return String::new();
    }

    let mut parts = Vec::new();
    let mut current = counts[0];
    let mut run_len = 1usize;

    for &c in &counts[1..] {
        if c == current {
            run_len += 1;
        } else {
            parts.push(format!("{}x{}", current, run_len));
            current = c;
            run_len = 1;
        }
    }
    parts.push(format!("{}x{}", current, run_len));

    parts.join("|")
}

// ---------------------------------------------------------------------------
// Markdown output
// ---------------------------------------------------------------------------

struct DiskEntry {
    title: String,
    format: String,
    protection: String,
    protection_details: Vec<String>,
    characteristics: Vec<String>,
    warnings: Vec<String>,
    bootable_on: String,
    boot_reason: String,
}

fn analyze_image(image: &DiskImage) -> DiskEntry {
    let spec = DiskSpecification::identify(image);

    let mut prot_strs = Vec::new();
    let mut all_details = Vec::new();
    for (side_idx, disk) in image.disks().iter().enumerate() {
        if let Some(p) = protection::detect(disk) {
            if image.disks().len() > 1 {
                prot_strs.push(format!("Side {}: {} ({})", side_idx, p.name, p.reason));
                if !p.details.is_empty() {
                    all_details.push(format!("Side {}:", side_idx));
                    all_details.extend(p.details.clone());
                }
            } else {
                prot_strs.push(format!("{} ({})", p.name, p.reason));
                all_details.extend(p.details);
            }
        }
    }
    let protection = if prot_strs.is_empty() {
        "None detected".to_string()
    } else {
        prot_strs.join("; ")
    };

    let characteristics = md_quirks(image, &spec);

    let boot = BootDetection::detect(image);
    let bootable_on = if boot.system.is_empty() {
        "Not bootable".to_string()
    } else {
        boot.system.clone()
    };

    DiskEntry {
        title: String::new(),
        format: report_format(image, &spec),
        protection,
        protection_details: all_details,
        characteristics,
        warnings: image.warnings().to_vec(),
        bootable_on,
        boot_reason: boot.reason,
    }
}

fn md_quirks(image: &DiskImage, spec: &DiskSpecification) -> Vec<String> {
    let mut quirks = Vec::new();

    let is_raw = matches!(image.format(), DiskImageFormat::RawMgt | DiskImageFormat::RawTrd);
    let standard_sectors = if is_raw { image.spec().sectors_per_track } else { spec.sectors_per_track } as usize;
    let standard_size = if is_raw { image.spec().sector_size } else { spec.sector_size } as usize;
    let standard_first_id: Option<u8> = match spec.format.as_str() {
        f if f.starts_with("Amstrad CPC") && f.contains("data") => Some(0xC1),
        f if f.starts_with("Amstrad CPC") && f.contains("system") => Some(0x41),
        f if f.starts_with("Timex") => Some(0),
        _ => Some(1),
    };
    let standard_track_count: Option<u8> = match spec.format.as_str() {
        f if f.contains("MGT") => Some(80),
        _ if is_raw => Some(image.spec().num_tracks),
        _ => Some(spec.tracks_per_side),
    };

    let expected_sides = if is_raw { image.spec().num_sides } else { spec.side_count() };
    if image.disks().len() != expected_sides as usize {
        quirks.push(format!(
            "Image has {} side(s) but format implies {}",
            image.disks().len(),
            expected_sides
        ));
    }

    for (side_idx, disk) in image.disks().iter().enumerate() {
        let prefix = if image.disks().len() > 1 {
            format!("Side {} ", side_idx)
        } else {
            String::new()
        };

        let total_tracks = disk.track_count();
        if let Some(expected) = standard_track_count {
            if total_tracks != expected as usize {
                quirks.push(format!(
                    "{}has {} tracks (expected {})",
                    prefix, total_tracks, expected
                ));
            }
        }

        // Unformatted tracks
        let unformatted: Vec<usize> = (0..total_tracks)
            .filter(|&i| {
                disk.get_track(i as u8)
                    .map(|t| t.sector_count() == 0)
                    .unwrap_or(false)
            })
            .collect();
        if !unformatted.is_empty() {
            quirks.push(format!(
                "{}unformatted tracks: {}",
                prefix,
                format_track_ranges(&unformatted)
            ));
        }

        // Per-track quirks
        let mut sector_count_quirks: Vec<(usize, usize)> = Vec::new();
        let mut sector_size_quirks: Vec<(usize, String)> = Vec::new();
        let mut fdc_error_tracks: Vec<(usize, Vec<u8>)> = Vec::new();
        let mut deleted_tracks: Vec<(usize, Vec<u8>)> = Vec::new();
        let mut size_mismatch_tracks: Vec<(usize, Vec<u8>)> = Vec::new();
        let mut large_tracks: Vec<(usize, usize)> = Vec::new();
        let mut alt_first_id_tracks: Vec<(usize, usize)> = Vec::new();
        let mut alt_filler_tracks: Vec<(usize, usize)> = Vec::new();

        let standard_track_size = standard_sectors * standard_size;
        let standard_filler = if is_raw { image.spec().filler_byte } else { 0xE5u8 };

        for t_idx in 0..total_tracks {
            let track = match disk.get_track(t_idx as u8) {
                Some(t) => t,
                None => continue,
            };
            let sc = track.sector_count();
            if sc == 0 {
                continue;
            }

            if sc != standard_sectors {
                sector_count_quirks.push((t_idx, sc));
            }

            // Sector sizes per track
            let sizes: Vec<usize> = track.sectors().iter().map(|s| s.advertised_size()).collect();
            let uniform_size = sizes.iter().all(|&s| s == sizes[0]);
            if !uniform_size {
                let summary = summarise_sizes(&sizes);
                sector_size_quirks.push((t_idx, summary));
            } else if sizes[0] != standard_size {
                sector_size_quirks.push((t_idx, format!("{}x{}", sizes.len(), sizes[0])));
            }

            // FDC errors
            let err_ids: Vec<u8> = track
                .sectors()
                .iter()
                .filter(|s| s.has_error())
                .map(|s| s.id.sector)
                .collect();
            if !err_ids.is_empty() {
                fdc_error_tracks.push((t_idx, err_ids));
            }

            // Deleted-data markers
            let del_ids: Vec<u8> = track
                .sectors()
                .iter()
                .filter(|s| s.is_deleted())
                .map(|s| s.id.sector)
                .collect();
            if !del_ids.is_empty() {
                deleted_tracks.push((t_idx, del_ids));
            }

            // Advertised-vs-actual size mismatches (weak/duplicated sectors)
            let mismatch_ids: Vec<u8> = track
                .sectors()
                .iter()
                .filter(|s| s.has_size_mismatch())
                .map(|s| s.id.sector)
                .collect();
            if !mismatch_ids.is_empty() {
                size_mismatch_tracks.push((t_idx, mismatch_ids));
            }

            // Oversized track
            let ts = track.total_data_size();
            if ts > standard_track_size {
                large_tracks.push((t_idx, ts));
            }

            // First sector ID
            if let Some(min_id) = track.sectors().iter().map(|s| s.id.sector).min() {
                if let Some(expected) = standard_first_id {
                    if min_id != expected {
                        alt_first_id_tracks.push((t_idx, min_id as usize));
                    }
                }
            }

            // Filler byte
            if track.filler_byte != standard_filler {
                alt_filler_tracks.push((t_idx, track.filler_byte as usize));
            }
        }

        push_track_int_quirks(&mut quirks, &prefix, "sector count", &sector_count_quirks);
        push_track_str_quirks(&mut quirks, &prefix, "unusual sector sizes", &sector_size_quirks);
        push_track_id_quirks(&mut quirks, &prefix, "FDC errors on", &fdc_error_tracks);
        push_track_id_quirks(&mut quirks, &prefix, "deleted-data markers on", &deleted_tracks);
        push_track_id_quirks(
            &mut quirks,
            &prefix,
            "size mismatches (weak/duplicated sectors) on",
            &size_mismatch_tracks,
        );
        push_track_int_quirks(&mut quirks, &prefix, "oversized tracks", &large_tracks);
        push_track_int_quirks(
            &mut quirks,
            &prefix,
            "non-standard first sector ID on",
            &alt_first_id_tracks,
        );
        push_track_int_quirks(
            &mut quirks,
            &prefix,
            "non-standard filler byte on",
            &alt_filler_tracks,
        );
    }

    quirks
}

fn push_track_int_quirks(
    quirks: &mut Vec<String>,
    prefix: &str,
    label: &str,
    entries: &[(usize, usize)],
) {
    if entries.is_empty() {
        return;
    }
    // Compress consecutive tracks with the same value.
    let mut grouped: Vec<(usize, usize, usize)> = Vec::new(); // (start, end, value)
    for &(t, v) in entries {
        if let Some(last) = grouped.last_mut() {
            if last.2 == v && last.1 + 1 == t {
                last.1 = t;
                continue;
            }
        }
        grouped.push((t, t, v));
    }
    let parts: Vec<String> = grouped
        .iter()
        .map(|&(s, e, v)| {
            if s == e {
                format!("T{}={}", s, v)
            } else {
                format!("T{}-T{}={}", s, e, v)
            }
        })
        .collect();
    quirks.push(format!("{}{}: {}", prefix, label, parts.join(", ")));
}

fn push_track_str_quirks(
    quirks: &mut Vec<String>,
    prefix: &str,
    label: &str,
    entries: &[(usize, String)],
) {
    if entries.is_empty() {
        return;
    }
    // Collapse consecutive tracks with identical values into ranges.
    let mut grouped: Vec<(usize, usize, &str)> = Vec::new();
    for (t, s) in entries {
        if let Some(last) = grouped.last_mut() {
            if last.2 == s.as_str() && last.1 + 1 == *t {
                last.1 = *t;
                continue;
            }
        }
        grouped.push((*t, *t, s.as_str()));
    }
    let parts: Vec<String> = grouped
        .iter()
        .map(|(s, e, v)| {
            if s == e {
                format!("T{}={}", s, v)
            } else {
                format!("T{}-T{}={}", s, e, v)
            }
        })
        .collect();
    quirks.push(format!("{}{}: {}", prefix, label, parts.join(", ")));
}

fn push_track_id_quirks(
    quirks: &mut Vec<String>,
    prefix: &str,
    label: &str,
    entries: &[(usize, Vec<u8>)],
) {
    if entries.is_empty() {
        return;
    }
    // Collapse consecutive tracks with the same sector ID set into ranges.
    let mut grouped: Vec<(usize, usize, Vec<u8>)> = Vec::new();
    for (t, ids) in entries {
        let mut sorted = ids.clone();
        sorted.sort();
        if let Some(last) = grouped.last_mut() {
            if last.2 == sorted && last.1 + 1 == *t {
                last.1 = *t;
                continue;
            }
        }
        grouped.push((*t, *t, sorted));
    }
    let parts: Vec<String> = grouped
        .iter()
        .map(|(s, e, ids)| {
            let id_strs: Vec<String> = ids.iter().map(|id| format!("{}", id)).collect();
            let trange = if s == e {
                format!("T{}", s)
            } else {
                format!("T{}-T{}", s, e)
            };
            format!("{}/S{{{}}}", trange, id_strs.join(","))
        })
        .collect();
    quirks.push(format!("{}{}: {}", prefix, label, parts.join(", ")));
}

fn summarise_sizes(sizes: &[usize]) -> String {
    // Count each unique size.
    let mut counts: BTreeMap<usize, usize> = BTreeMap::new();
    for &s in sizes {
        *counts.entry(s).or_insert(0) += 1;
    }
    counts
        .iter()
        .map(|(size, count)| format!("{}x{}", count, size))
        .collect::<Vec<_>>()
        .join("+")
}

/// Compress a list of track indices into `T`-prefixed ranges, e.g.
/// `[0,1,2,5]` -> `"T0-T2, T5"`. Used for markdown quirk lines.
fn format_track_ranges(indices: &[usize]) -> String {
    if indices.is_empty() {
        return String::new();
    }
    let mut sorted: Vec<usize> = indices.to_vec();
    sorted.sort();
    let mut out = Vec::new();
    let mut start = sorted[0];
    let mut end = sorted[0];
    for &i in &sorted[1..] {
        if i == end + 1 {
            end = i;
        } else {
            out.push(if start == end {
                format!("T{}", start)
            } else {
                format!("T{}-T{}", start, end)
            });
            start = i;
            end = i;
        }
    }
    out.push(if start == end {
        format!("T{}", start)
    } else {
        format!("T{}-T{}", start, end)
    });
    out.join(", ")
}

fn write_markdown(
    out: &mut dyn Write,
    root: &Path,
    sections: &BTreeMap<String, Vec<DiskEntry>>,
    counts: &Counts,
) -> std::io::Result<()> {
    writeln!(out, "# Disk analysis")?;
    writeln!(out)?;
    writeln!(
        out,
        "Scanned `{}` — {} disk images across {} folders ({} from zips, {} errors).",
        root.display(),
        counts.images,
        sections.len(),
        counts.images_from_zips,
        counts.errors,
    )?;
    writeln!(out)?;

    for (folder, entries) in sections {
        writeln!(out, "## {}", folder)?;
        writeln!(out)?;
        for entry in entries {
            // Strip the leading "<folder>/" from the title since the h2 already says it.
            let title = entry
                .title
                .strip_prefix(&format!("{}/", folder))
                .unwrap_or(&entry.title);
            writeln!(out, "### {}", title)?;
            writeln!(out)?;
            writeln!(out, "- Format: {}", entry.format)?;
            writeln!(out, "- Protection: {}", entry.protection)?;
            if !entry.protection_details.is_empty() {
                writeln!(out, "  - Details:")?;
                for d in &entry.protection_details {
                    writeln!(out, "    - {}", d)?;
                }
            }
            if !entry.bootable_on.is_empty() {
                writeln!(out, "- Bootable on: {}", entry.bootable_on)?;
                if !entry.boot_reason.is_empty() {
                    writeln!(out, "  - {}", entry.boot_reason)?;
                }
            }
            if entry.characteristics.is_empty() {
                writeln!(out, "- Characteristics: standard")?;
            } else {
                writeln!(out, "- Characteristics:")?;
                for c in &entry.characteristics {
                    writeln!(out, "  - {}", c)?;
                }
            }
            if !entry.warnings.is_empty() {
                writeln!(out, "- Warnings:")?;
                for w in &entry.warnings {
                    writeln!(out, "  - {}", w)?;
                }
            }
            writeln!(out)?;
        }
    }
    Ok(())
}
