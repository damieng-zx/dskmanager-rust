//! Interactive DSK console application

use dskmanager::filesystem::AnyFileSystem;
use dskmanager::*;
use rustyline::completion::{Completer, Pair};
use rustyline::error::ReadlineError;
use rustyline::highlight::Highlighter;
use rustyline::hint::Hinter;
use rustyline::validate::Validator;
use rustyline::{Context, Editor, Helper};

mod analysis;
mod format;
mod report;
mod sector_edit;
mod session;

use analysis::{
    default_ascii_chars, disassemble_z80, find_lowest_sector_id, find_strings_in_disk,
    parse_charset,
};
use format::{
    list_all_sectors, list_sectors_on_track, list_tracks, print_directory, print_filesystem_info, print_help,
    print_info,
};
use sector_edit::{replace_sector_from_file, set_sector_status};
use session::{
    parse_command_line, parse_hex_or_dec, parse_sector_address, print_hex_dump,
    resolve_format_mode,
};

/// Command completer for the REPL
struct CommandCompleter {
    commands: Vec<&'static str>,
}

impl CommandCompleter {
    fn new() -> Self {
        Self {
            commands: vec![
                "open",
                "load",
                "create",
                "save",
                "info",
                "specification",
                "spec",
                "tracks",
                "sectors",
                "read-sector",
                "write-sector",
                "fill-sector",
                "set-sector-status",
                "fs-switch",
                "fs-list",
                "cat",
                "dir",
                "ls",
                "fs-read",
                "fs-show",
                "fs-export",
                "fs-import",
                "fs-delete",
                "fs-format",
                "fs-info",
                "protection",
                "disassemble",
                "dasm",
                "strings",
                "map",
                "verify",
                "help",
                "quit",
                "exit",
            ],
        }
    }
}

impl Completer for CommandCompleter {
    type Candidate = Pair;

    fn complete(
        &self,
        line: &str,
        pos: usize,
        _ctx: &Context<'_>,
    ) -> rustyline::Result<(usize, Vec<Pair>)> {
        // Only complete the first word (command name)
        let line_to_cursor = &line[..pos];
        if line_to_cursor.contains(' ') {
            // Already past the command, don't complete
            return Ok((pos, vec![]));
        }

        let prefix = line_to_cursor.to_lowercase();
        let matches: Vec<Pair> = self
            .commands
            .iter()
            .filter(|cmd| cmd.starts_with(&prefix))
            .map(|cmd| Pair {
                display: cmd.to_string(),
                replacement: cmd.to_string(),
            })
            .collect();

        Ok((0, matches))
    }
}

impl Hinter for CommandCompleter {
    type Hint = String;
}

impl Highlighter for CommandCompleter {}
impl Validator for CommandCompleter {}
impl Helper for CommandCompleter {}

/// Get the path to the history file
fn history_path() -> Option<std::path::PathBuf> {
    dirs::home_dir().map(|mut p| {
        p.push(".dskmanager_history");
        p
    })
}

/// Open an image for the REPL, printing the result (and any warnings).
/// Returns `None` on failure so the caller can keep any current image.
fn open_image(path: &str) -> Option<DiskImage> {
    match DiskImage::open(path) {
        Ok(img) => {
            println!("Opened: {}", path);
            for w in img.warnings() {
                println!("Warning: {}", w);
            }
            Some(img)
        }
        Err(e) => {
            println!("Error: {}", e);
            None
        }
    }
}

fn main() {
    // Batch subcommand: `dsk report <dir> [output] [--format csv|markdown]`.
    // Runs non-interactively and exits without starting the REPL.
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(|s| s.as_str()) {
        Some("--help") | Some("-h") => {
            println!("Usage: dsk [command]");
            println!();
            println!("Commands:");
            println!("  report <pattern> [output] [--format csv|markdown]");
            println!("      Batch-analyze .dsk/.mgt/.trd/.td0/.scl files (dir, glob, or single file).");
            println!("      Also scans inside .zip archives found in the pattern.");
            println!("      Writes a report to [output] (stdout if omitted).");
            println!("      Format defaults to CSV, or inferred from extension (.csv / .md).");
            println!();
            println!("  Report filters (combine with AND):");
            println!("      --filter-format <text>        Substring match on disk format");
            println!("      --filter-tracks <N>           Exact track count");
            println!("      --filter-sides <N>            Exact side count");
            println!("      --filter-sector-size <N>      Exact sector size in bytes");
            println!("      --filter-protection [text]    Any protection, or substring on name");
            println!("      --filter-has-errors           Has FDC errors");
            println!("      --filter-quirks               Has non-standard characteristics");
            println!();
            println!("  <file>");
            println!("      Open the given disk image, then start the interactive REPL.");
            println!();
            println!("  (no arguments)");
            println!("      Start the interactive REPL for exploring disk images.");
            println!();
            println!("Options:");
            println!("  -h, --help    Show this help message");
            return;
        }
        Some("report") => {
            std::process::exit(report::run(&args[2..]));
        }
        _ => {}
    }

    println!("=== DSKManager ===");
    println!("Interactive console for exploring DSK format disk images.");
    println!("Type 'help' for available commands\n");

    let mut rl = Editor::new().expect("Failed to create editor");
    rl.set_helper(Some(CommandCompleter::new()));

    // Load history if available
    if let Some(history_path) = history_path() {
        let _ = rl.load_history(&history_path);
    }

    // If a path to a file was given on the command line, open it on startup.
    let mut image: Option<DiskImage> = match args.get(1) {
        Some(path) if std::path::Path::new(path).is_file() => open_image(path),
        _ => None,
    };
    let mut filesystem_mode = FileSystemType::Auto;

    loop {
        let readline = rl.readline("> ");
        let input = match readline {
            Ok(line) => line,
            Err(ReadlineError::Interrupted) => {
                println!("^C");
                continue;
            }
            Err(ReadlineError::Eof) => {
                // Save history before exiting
                if let Some(history_path) = history_path() {
                    let _ = rl.save_history(&history_path);
                }
                println!("Goodbye!");
                break;
            }
            Err(err) => {
                println!("Error: {:?}", err);
                break;
            }
        };

        let input = input.trim();
        if input.is_empty() {
            continue;
        }

        // Add to history
        let _ = rl.add_history_entry(input);

        let parts = parse_command_line(input);
        if parts.is_empty() {
            continue;
        }
        let command = parts[0].to_lowercase();

        match command.as_str() {
            "help" => {
                print_help();
            }
            "quit" | "exit" => {
                // Save history before exiting
                if let Some(history_path) = history_path() {
                    let _ = rl.save_history(&history_path);
                }
                println!("Goodbye!");
                break;
            }
            "open" | "load" => {
                if parts.len() < 2 {
                    println!("Usage: open <path>");
                    continue;
                }
                if let Some(img) = open_image(&parts[1]) {
                    image = Some(img);
                }
            }
            "create" => {
                let created = match parts.get(1).map(String::as_str).unwrap_or("amstrad") {
                    "system" => DiskImage::create(FormatSpec::amstrad_system()),
                    "spectrum" => DiskImage::create(FormatSpec::spectrum_plus3()),
                    "pcw" => DiskImage::create(FormatSpec::pcw_ssdd()),
                    "mgt" => DiskImage::builder().format(DiskImageFormat::RawMgt)
                        .spec(FormatSpec::new(2, 80, 10, 512).with_first_sector_id(1).with_filler_byte(0)).build(),
                    "trdos" => DiskImage::builder().format(DiskImageFormat::RawTrd)
                        .spec(FormatSpec::trdos()).build(),
                    "amstrad" => DiskImage::create(FormatSpec::amstrad_data()),
                    _ => {
                        println!("Usage: create [amstrad|system|spectrum|pcw|mgt|trdos]");
                        continue;
                    }
                };
                match created.and_then(|mut img| {
                    filesystem::format_filesystem(&mut img, FileSystemType::Auto)?;
                    Ok(img)
                }) {
                    Ok(img) => {
                        println!("Created formatted {} image", img.format().name());
                        image = Some(img);
                        filesystem_mode = FileSystemType::Auto;
                    }
                    Err(e) => println!("Error: {}", e),
                }
            }
            "info" => {
                if let Some(ref img) = image {
                    print_info(img);
                } else {
                    println!("No image loaded. Use 'open <path>' or 'create' first.");
                }
            }
            "tracks" => {
                if let Some(ref img) = image {
                    list_tracks(img);
                } else {
                    println!("No image loaded.");
                }
            }
            "read-sector" => {
                if let Some(ref img) = image {
                    if parts.len() < 4 {
                        println!("Usage: read-sector <side> <track> <sector_id>");
                        continue;
                    }
                    let side: u8 = parts[1].parse().unwrap_or(0);
                    let track: u8 = parts[2].parse().unwrap_or(0);
                    let sector_id: u8 = if parts[3].starts_with("0x") || parts[3].starts_with("0X") {
                        u8::from_str_radix(&parts[3].as_str()[2..], 16).unwrap_or(0xC1)
                    } else {
                        parts[3].parse().unwrap_or(0xC1)
                    };

                    match img.read_sector(side, track, sector_id) {
                        Ok(data) => {
                            println!("Sector {}:{}:{} ({} bytes):", side, track, sector_id, data.len());
                            print_hex_dump(data, 256);
                        }
                        Err(e) => println!("Error: {}", e),
                    }
                } else {
                    println!("No image loaded.");
                }
            }
            "fs-info" => {
                if let Some(ref img) = image {
                    match AnyFileSystem::mount(img, filesystem_mode) {
                        Ok(fs) => print_filesystem_info(&fs.info()),
                        Err(e) => println!("Error: {}", e),
                    }
                } else {
                    println!("No image loaded.");
                }
            }
            "fs-list" | "dir" | "cat" | "ls" => {
                if let Some(ref img) = image {
                    let entries_result: Result<Vec<ExtendedDirEntry>> =
                        AnyFileSystem::mount(img, filesystem_mode)
                            .and_then(|fs| fs.read_dir_extended());

                    match entries_result {
                        Ok(entries) => print_directory(&entries),
                        Err(e) => println!("Error: {}", e),
                    }
                } else {
                    println!("No image loaded.");
                }
            }
            "fs-read" => {
                if let Some(ref img) = image {
                    if parts.len() < 2 {
                        println!("Usage: fs-read <filename>");
                        continue;
                    }

                    let data_result: Result<Vec<u8>> = AnyFileSystem::mount(img, filesystem_mode)
                        .and_then(|fs| fs.read_file(&parts[1]));

                    match data_result {
                        Ok(data) => {
                            println!("File: {} ({} bytes)", parts[1], data.len());
                            print_hex_dump(&data, 256);
                        }
                        Err(e) => println!("Error: {}", e),
                    }
                } else {
                    println!("No image loaded.");
                }
            }
            "fs-show" => {
                if let Some(ref img) = image {
                    if parts.len() < 2 {
                        println!("Usage: fs-show <filename>");
                        println!("  Shows AMSDOS and PLUS3DOS BASIC files as text.");
                        continue;
                    }

                    // Read the file with its header so the decoder can inspect it.
                    let data_result: Result<Vec<u8>> = AnyFileSystem::mount(img, filesystem_mode)
                        .and_then(|fs| fs.read_file_raw(&parts[1]));

                    match data_result {
                        Ok(data) => {
                            match decode_amstrad_basic_file(&data) {
                                Ok(Some(text)) => {
                                    print!("{}", text);
                                }
                                Ok(None) => {
                                    // Try Sinclair BASIC (PLUS3DOS)
                                    match decode_sinclair_basic_file(&data) {
                                        Ok(Some(text)) => {
                                            print!("{}", text);
                                        }
                                        Ok(None) => {
                                            println!("File '{}' is not a recognized BASIC file (AMSDOS or PLUS3DOS).", parts[1]);
                                        }
                                        Err(e) => println!("Error decoding BASIC: {}", e),
                                    }
                                }
                                Err(e) => println!("Error decoding BASIC: {}", e),
                            }
                        }
                        Err(e) => println!("Error reading file: {}", e),
                    }
                } else {
                    println!("No image loaded.");
                }
            }
            "fs-export" => {
                if let Some(ref img) = image {
                    if parts.len() < 2 {
                        println!("Usage: fs-export <filename> [output_path] [raw]");
                        println!("  CP/M files: AMSDOS and PLUS3DOS headers are stripped by default.");
                        println!("             Use 'raw' option to preserve headers (CP/M only).");
                        println!("  MGT files: Data is truncated to actual file length (raw option ignored).");
                        continue;
                    }
                    let src_filename = &parts[1];
                    // Parse arguments: filename [output_path] [raw]
                    let mut output_path = None;
                    let mut raw_mode = false;

                    for arg in parts.iter().skip(2) {
                        if arg.to_lowercase() == "raw" {
                            raw_mode = true;
                        } else if output_path.is_none() {
                            output_path = Some(arg.clone());
                        }
                    }

                    // If no output path specified, use the source filename
                    let output_path = output_path.unwrap_or_else(|| src_filename.clone());

                    // Raw mode only applies to CP/M, which stores headers in the
                    // file data; other drivers keep metadata in the directory.
                    let mounted = AnyFileSystem::mount(img, filesystem_mode);
                    let data_result: Result<Vec<u8>> = match mounted.as_ref() {
                        Ok(fs) => {
                            if raw_mode {
                                fs.read_file_raw(src_filename)
                            } else {
                                fs.read_file(src_filename)
                            }
                        }
                        Err(error) => Err(DskError::filesystem(error.to_string())),
                    };
                    let raw_applied = raw_mode
                        && mounted.as_ref().map(|fs| fs.stores_headers_in_data()).unwrap_or(false);

                    match data_result {
                        Ok(data) => {
                            match std::fs::write(&output_path, &data) {
                                Ok(_) => {
                                    if raw_applied {
                                        println!("Exported {} ({} bytes, raw) to {}",
                                            src_filename, data.len(), output_path);
                                    } else {
                                        println!("Exported {} ({} bytes) to {}",
                                            src_filename, data.len(), output_path);
                                    }
                                }
                                Err(e) => println!("Error writing file: {}", e),
                            }
                        }
                        Err(e) => println!("Error reading file: {}", e),
                    }
                } else {
                    println!("No image loaded.");
                }
            }
            "write-sector" => {
                if let Some(ref mut img) = image {
                    if parts.len() != 5 {
                        println!("Usage: write-sector <side> <track> <sector_id> <host_path>");
                        continue;
                    }
                    match parse_sector_address(&parts).and_then(|(side, track, id)| {
                        replace_sector_from_file(img, side, track, id, &parts[4]).map_err(|e| e.to_string())
                    }) {
                        Ok(size) => println!("Wrote {} bytes to sector.", size),
                        Err(e) => println!("Error: {}", e),
                    }
                } else {
                    println!("No image loaded.");
                }
            }
            "fill-sector" => {
                if let Some(ref mut img) = image {
                    if parts.len() != 5 {
                        println!("Usage: fill-sector <side> <track> <sector_id> <byte>");
                        continue;
                    }
                    let result = parse_sector_address(&parts).and_then(|(side, track, id)| {
                        let byte = parse_hex_or_dec(&parts[4]).ok_or("Invalid byte (use decimal or 0xNN)")?;
                        let len = img.read_sector(side, track, id).map_err(|e| e.to_string())?.len();
                        img.write_sector(side, track, id, &vec![byte; len]).map_err(|e| e.to_string())
                    });
                    match result {
                        Ok(()) => println!("Sector filled."),
                        Err(e) => println!("Error: {}", e),
                    }
                } else {
                    println!("No image loaded.");
                }
            }
            "set-sector-status" => {
                if let Some(ref mut img) = image {
                    if parts.len() != 6 {
                        println!("Usage: set-sector-status <side> <track> <sector_id> <st1> <st2>");
                        continue;
                    }
                    let result = parse_sector_address(&parts).and_then(|(side, track, id)| {
                        let st1 = parse_hex_or_dec(&parts[4]).ok_or("Invalid ST1 byte")?;
                        let st2 = parse_hex_or_dec(&parts[5]).ok_or("Invalid ST2 byte")?;
                        set_sector_status(img, side, track, id, st1, st2).map_err(|e| e.to_string())
                    });
                    match result {
                        Ok(()) => println!("FDC status updated."),
                        Err(e) => println!("Error: {}", e),
                    }
                } else {
                    println!("No image loaded.");
                }
            }
            "fs-import" => {
                if let Some(ref mut img) = image {
                    if parts.len() < 2 {
                        println!("Usage: fs-import <host_path> [disk_name]");
                        continue;
                    }
                    let name = parts.get(2).cloned().or_else(|| std::path::Path::new(&parts[1])
                        .file_name().map(|n| n.to_string_lossy().into_owned()));
                    match (std::fs::read(&parts[1]), name) {
                        (Ok(bytes), Some(name)) => match filesystem::import_file(img, filesystem_mode, &name, &bytes) {
                            Ok(()) => println!("Imported {} ({} bytes) as {}", parts[1], bytes.len(), name),
                            Err(e) => println!("Error: {}", e),
                        },
                        (Err(e), _) => println!("Error reading file: {}", e),
                        (_, None) => println!("Error: host path has no filename"),
                    }
                } else {
                    println!("No image loaded.");
                }
            }
            "fs-delete" => {
                if let Some(ref mut img) = image {
                    if parts.len() < 2 {
                        println!("Usage: fs-delete <disk_name>");
                        continue;
                    }
                    match filesystem::delete_file(img, filesystem_mode, &parts[1]) {
                        Ok(()) => println!("Deleted {}", parts[1]),
                        Err(e) => println!("Error: {}", e),
                    }
                } else {
                    println!("No image loaded.");
                }
            }
            "fs-format" => {
                if let Some(ref mut img) = image {
                    let requested = if let Some(arg) = parts.get(1) {
                        match FileSystemType::from_str(arg) {
                            Some(mode) => Some(mode),
                            None => {
                                println!("Usage: fs-format [auto|cpm|mgt|trdos]");
                                continue;
                            }
                        }
                    } else { None };
                    let mode = resolve_format_mode(filesystem_mode, requested);
                    match filesystem::format_filesystem(img, mode) {
                        Ok(()) => {
                            // An explicit argument also selects the new driver so
                            // later commands read the freshly formatted disk.
                            if requested.is_some() {
                                filesystem_mode = mode;
                            }
                            println!("Filesystem formatted ({}). All previous files erased.", mode);
                        }
                        Err(e) => println!("Error: {}", e),
                    }
                } else {
                    println!("No image loaded.");
                }
            }
            "fs-switch" => {
                if parts.len() < 2 {
                    // Show current mode
                    let effective = if let Some(ref img) = image {
                        AnyFileSystem::resolve(img, filesystem_mode)
                    } else {
                        filesystem_mode
                    };
                    println!("Filesystem mode: {} (effective: {})", filesystem_mode, effective);
                    println!("Options: auto, cpm, mgt, trdos");
                } else {
                    match FileSystemType::from_str(&parts[1]) {
                        Some(mode) => {
                            filesystem_mode = mode;
                            println!("Filesystem mode set to: {}", filesystem_mode);
                        }
                        None => {
                            println!("Unknown filesystem type: {}", parts[1]);
                            println!("Options: auto, cpm, mgt, trdos");
                        }
                    }
                }
            }
            "save" => {
                if let Some(ref mut img) = image {
                    if parts.len() < 2 {
                        println!("Usage: save <path>");
                        continue;
                    }
                    match img.save(&parts[1]) {
                        Ok(_) => println!("Saved to: {}", parts[1]),
                        Err(e) => println!("Error: {}", e),
                    }
                } else {
                    println!("No image loaded.");
                }
            }
            "protection" => {
                if let Some(ref img) = image {
                    let mut found_any = false;
                    let has_multiple_sides = img.disks().len() > 1;
                    for (side_idx, disk) in img.disks().iter().enumerate() {
                        if let Some(result) = dskmanager::protection::detect(disk) {
                            let prefix = if has_multiple_sides {
                                format!("Side {}: ", side_idx)
                            } else {
                                String::new()
                            };
                            if result.details.is_empty() {
                                println!("{}{} [{}]", prefix, result.name, result.qualified_reason());
                            } else {
                                println!("{}{} [{}]", prefix, result.name, result.qualified_reason());
                                for line in &result.details {
                                    println!("  {}", line);
                                }
                            }
                            found_any = true;
                        }
                    }
                    if !found_any {
                        println!("No copy protection detected.");
                    }
                } else {
                    println!("No image loaded.");
                }
            }
            "sectors" => {
                if let Some(ref img) = image {
                    if parts.len() >= 2 {
                        // Track number specified
                        let track: u8 = parts[1].parse().unwrap_or(0);
                        let side: u8 = if parts.len() >= 3 {
                            parts[2].parse().unwrap_or(0)
                        } else {
                            0
                        };
                        list_sectors_on_track(img, side, track);
                    } else {
                        // List all sectors on all tracks
                        list_all_sectors(img);
                    }
                } else {
                    println!("No image loaded.");
                }
            }
            "specification" | "spec" => {
                if let Some(ref img) = image {
                    let spec = DiskSpecification::identify(img);
                    print!("{}", spec);
                } else {
                    println!("No image loaded.");
                }
            }
            "disassemble" | "dasm" => {
                if let Some(ref img) = image {
                    let side: u8 = 0;
                    let (track, sector_id) = if parts.len() >= 3 {
                        let t: u8 = parts[1].parse().unwrap_or(0);
                        let s: u8 = parse_hex_or_dec(&parts[2]).unwrap_or(0);
                        (t, s)
                    } else if parts.len() == 2 {
                        let t: u8 = parts[1].parse().unwrap_or(0);
                        // Find lowest sector ID on specified track
                        match find_lowest_sector_id(img, side, t) {
                            Some(s) => (t, s),
                            None => {
                                println!("No sectors found on track {}.", t);
                                continue;
                            }
                        }
                    } else {
                        // Default: track 0, lowest sector ID
                        match find_lowest_sector_id(img, side, 0) {
                            Some(s) => (0, s),
                            None => {
                                println!("No sectors found on track 0.");
                                continue;
                            }
                        }
                    };

                    match img.read_sector(side, track, sector_id) {
                        Ok(data) => {
                            disassemble_z80(data);
                        }
                        Err(e) => println!("Error: {}", e),
                    }
                } else {
                    println!("No image loaded.");
                }
            }
            "strings" => {
                if let Some(ref img) = image {
                    // Parse optional arguments: strings [min_length] [min_unique] [charset]
                    let min_length: usize = if parts.len() >= 2 {
                        parts[1].parse().unwrap_or(4)
                    } else {
                        4
                    };

                    let min_unique: usize = if parts.len() >= 3 {
                        parts[2].parse().unwrap_or(3)
                    } else {
                        3
                    };

                    let charset: Vec<u8> = if parts.len() >= 4 {
                        parse_charset(&parts[3])
                    } else {
                        default_ascii_chars()
                    };

                    let strings = find_strings_in_disk(img, min_length, min_unique, &charset);

                    if strings.is_empty() {
                        println!("No strings found (min length: {}, min unique: {}).", min_length, min_unique);
                    } else {
                        for hit in &strings {
                            println!(
                                "S{}:T{}:{}+{:03X}: {}",
                                hit.side, hit.track, hit.sector, hit.offset, hit.text
                            );
                        }
                        println!("\nFound {} strings.", strings.len());
                    }
                } else {
                    println!("No image loaded.");
                }
            }
            "map" => {
                if let Some(ref img) = image {
                    // Parse optional side argument
                    let side: usize = if parts.len() >= 2 {
                        parts[1].parse().unwrap_or(0)
                    } else {
                        0
                    };
                    dskmanager::map::draw_sector_map(img, side);
                } else {
                    println!("No image loaded.");
                }
            }
            "verify" => {
                if let Some(ref img) = image {
                    let report = dskmanager::verify::verify(img);
                    println!("{}", report);
                } else {
                    println!("No image loaded.");
                }
            }
            _ => {
                println!("Unknown command: {}. Type 'help' for available commands.", command);
            }
        }
    }
}

