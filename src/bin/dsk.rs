/// Interactive DSK console application

use dez80::Instruction;

use dskmanager::*;
use dskmanager::amstrad_basic::decode_amstrad_basic_file;
use dskmanager::filesystem::TrdosFileSystem;
use dskmanager::sinclair_basic::decode_sinclair_basic_file;
use rustyline::completion::{Completer, Pair};
use rustyline::error::ReadlineError;
use rustyline::highlight::Highlighter;
use rustyline::hint::Hinter;
use rustyline::validate::Validator;
use rustyline::{Context, Editor, Helper};

mod report;

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
            println!("      Batch-analyze .dsk/.mgt/.trd/.td0 files (dir, glob, or single file).");
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
                    // Determine effective filesystem type
                    let effective_fs = match filesystem_mode {
                        FileSystemType::Auto => img.default_filesystem(),
                        other => other,
                    };

                    // Use appropriate filesystem
                    match effective_fs {
                        FileSystemType::Mgt => {
                            match DiscipleFileSystem::new(img) {
                                Ok(fs) => {
                                    let mgt_info = fs.mgt().info();
                                    println!("MGT filesystem ({})", mgt_info.system_type);
                                    let sector_size = 512;
                                    let total_sectors = mgt_info.total_sectors;
                                    let free_sectors = mgt_info.free_sectors;
                                    println!("Block size: {} bytes", sector_size);
                                    println!("Total blocks: {}", total_sectors);
                                    println!("Usable capacity: {} KB", total_sectors * sector_size / 1024);
                                    println!("Free blocks: {}", free_sectors);
                                    println!("Free space: {} KB", free_sectors * sector_size / 1024);
                                }
                                Err(e) => println!("Error: {}", e),
                            }
                        }
                        FileSystemType::Trdos => {
                            match TrdosFileSystem::new(img) {
                                Ok(fs) => {
                                    let info = fs.info();
                                    println!("{} filesystem", info.fs_type);
                                    println!("Block size: {} bytes", info.block_size);
                                    println!("Total blocks: {}", info.total_blocks);
                                    println!("Usable capacity: {} KB", info.total_blocks * info.block_size / 1024);
                                    println!("Free blocks: {}", info.free_blocks);
                                    println!("Free space: {} KB", info.free_blocks * info.block_size / 1024);
                                }
                                Err(e) => println!("Error: {}", e),
                            }
                        }
                        FileSystemType::Cpm | FileSystemType::Auto => {
                            match CpmFileSystem::from_image(img) {
                                Ok(fs) => {
                                    let info = fs.info();
                                    println!("{} filesystem", info.fs_type);
                                    println!("Block size: {} bytes", info.block_size);
                                    println!("Total blocks: {}", info.total_blocks);
                                    println!("Usable capacity: {} KB", info.total_blocks * info.block_size / 1024);
                                    println!("Free blocks: {}", info.free_blocks);
                                    println!("Free space: {} KB", info.free_blocks * info.block_size / 1024);
                                }
                                Err(e) => println!("Error: {}", e),
                            }
                        }
                    }
                } else {
                    println!("No image loaded.");
                }
            }
            "fs-list" | "dir" | "cat" | "ls" => {
                if let Some(ref img) = image {
                    // Determine effective filesystem type
                    let effective_fs = match filesystem_mode {
                        FileSystemType::Auto => img.default_filesystem(),
                        other => other,
                    };

                    // Use appropriate filesystem
                    let entries_result: Result<Vec<ExtendedDirEntry>> = match effective_fs {
                        FileSystemType::Mgt => {
                            match DiscipleFileSystem::new(img) {
                                Ok(fs) => fs.read_dir_extended(),
                                Err(e) => Err(e),
                            }
                        }
                        FileSystemType::Trdos => {
                            match TrdosFileSystem::new(img) {
                                Ok(fs) => {
                                    let trd_dir = fs.directory();
                                    let mut ext_entries = Vec::new();
                                    for e in trd_dir {
                                        let header = crate::filesystem::FileHeader {
                                            header_type: crate::filesystem::HeaderType::None,
                                            checksum_valid: false,
                                            file_size: e.sector_count as usize * 256,
                                            header_size: 0,
                                            meta: e.display_type(),
                                        };
                                        ext_entries.push(ExtendedDirEntry {
                                            name: e.filename.clone(),
                                            user: 0,
                                            index: e.index,
                                            blocks: e.sector_count as usize,
                                            allocated: e.sector_count as usize * 256,
                                            size: e.sector_count as usize * 256,
                                            attributes: crate::filesystem::FileAttributes::default(),
                                            header,
                                        });
                                    }
                                    Ok(ext_entries)
                                }
                                Err(e) => Err(e),
                            }
                        }
                        FileSystemType::Cpm | FileSystemType::Auto => {
                            match CpmFileSystem::from_image(img) {
                                Ok(fs) => fs.read_dir_extended_with_deleted(),
                                Err(e) => Err(e),
                            }
                        }
                    };

                    match entries_result {
                        Ok(entries) => {
                            if entries.is_empty() {
                                println!("No files found.");
                            } else {
                                // Always show all columns including Usr and Del
                                println!(
                                    "{:<14} {:>3} {:>3} {:>4} {:>5} {:>7} {:>3} {:>3} {:<8} {:>3} {}",
                                    "Name", "Idx", "Usr", "Blks", "Alloc", "Size", "Att", "Del", "Header", "Chk", "Meta"
                                );
                                println!("{}", "-".repeat(96));

                                for entry in entries {
                                    let is_deleted = entry.user == 0xE5;
                                    let user_display = if is_deleted { "E5".to_string() } else { format!("{}", entry.user) };
                                    let attrs = format!(
                                        "{}{}{}",
                                        if entry.attributes.read_only { "R" } else { "-" },
                                        if entry.attributes.system { "S" } else { "-" },
                                        if entry.attributes.archive { "A" } else { "-" }
                                    );
                                    let header_type = format!("{}", entry.header.header_type);
                                    let checksum = if entry.header.header_type != HeaderType::None {
                                        if entry.header.checksum_valid { "Yes" } else { "No" }
                                    } else {
                                        ""
                                    };

                                    println!(
                                        "{:<14} {:>3} {:>3} {:>4} {:>4}K {:>7} {:>3} {:>3} {:<8} {:>3} {}",
                                        entry.name,
                                        entry.index,
                                        user_display,
                                        entry.blocks,
                                        entry.allocated / 1024,
                                        entry.size,
                                        attrs,
                                        if is_deleted { "Yes" } else { "" },
                                        header_type,
                                        checksum,
                                        entry.header.meta
                                    );
                                }
                            }
                        }
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

                    // Determine effective filesystem type
                    let effective_fs = match filesystem_mode {
                        FileSystemType::Auto => img.default_filesystem(),
                        other => other,
                    };

                    let data_result: Result<Vec<u8>> = match effective_fs {
                        FileSystemType::Mgt => {
                            match DiscipleFileSystem::new(img) {
                                Ok(fs) => fs.read_file(&parts[1]),
                                Err(e) => Err(e),
                            }
                        }
                        FileSystemType::Trdos => {
                            match TrdosFileSystem::new(img) {
                                Ok(fs) => fs.read_file(&parts[1]),
                                Err(e) => Err(e),
                            }
                        }
                        FileSystemType::Cpm | FileSystemType::Auto => {
                            match CpmFileSystem::from_image(img) {
                                Ok(fs) => fs.read_file(&parts[1]),
                                Err(e) => Err(e),
                            }
                        }
                    };

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

                    // Determine effective filesystem type
                    let effective_fs = match filesystem_mode {
                        FileSystemType::Auto => img.default_filesystem(),
                        other => other,
                    };

                    // Read file with header (we need to check the header)
                    let data_result: Result<Vec<u8>> = match effective_fs {
                        FileSystemType::Mgt => {
                            match DiscipleFileSystem::new(img) {
                                Ok(fs) => fs.read_file(&parts[1]),
                                Err(e) => Err(e),
                            }
                        }
                        FileSystemType::Trdos => {
                            match TrdosFileSystem::new(img) {
                                Ok(fs) => fs.read_file(&parts[1]),
                                Err(e) => Err(e),
                            }
                        }
                        FileSystemType::Cpm | FileSystemType::Auto => {
                            match CpmFileSystem::from_image(img) {
                                Ok(fs) => {
                                    fs.read_file_binary(&parts[1], true)
                                }
                                Err(e) => Err(e),
                            }
                        }
                    };

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

                    // Determine effective filesystem type
                    let effective_fs = match filesystem_mode {
                        FileSystemType::Auto => img.default_filesystem(),
                        other => other,
                    };

                    // Read file data
                    // Raw mode only applies to CP/M filesystems (they have headers in file data)
                    // MGT filesystems store metadata in directory entries, so raw mode is ignored
                    let data_result: Result<Vec<u8>> = match effective_fs {
                        FileSystemType::Mgt => {
                            match DiscipleFileSystem::new(img) {
                                Ok(fs) => fs.read_file(src_filename),
                                Err(e) => Err(e),
                            }
                        }
                        FileSystemType::Trdos => {
                            match TrdosFileSystem::new(img) {
                                Ok(fs) => fs.read_file(src_filename),
                                Err(e) => Err(e),
                            }
                        }
                        FileSystemType::Cpm | FileSystemType::Auto => {
                            match CpmFileSystem::from_image(img) {
                                Ok(fs) => {
                                    if raw_mode {
                                        fs.read_file_binary(src_filename, true)
                                    } else {
                                        fs.read_file(src_filename)
                                    }
                                }
                                Err(e) => Err(e),
                            }
                        }
                    };

                    match data_result {
                        Ok(data) => {
                            match std::fs::write(&output_path, &data) {
                                Ok(_) => {
                                    if raw_mode && matches!(effective_fs, FileSystemType::Cpm | FileSystemType::Auto) {
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
                    let mode = if let Some(arg) = parts.get(1) {
                        match FileSystemType::from_str(arg) {
                            Some(mode) => mode,
                            None => {
                                println!("Usage: fs-format [auto|cpm|mgt|trdos]");
                                continue;
                            }
                        }
                    } else { filesystem_mode };
                    match filesystem::format_filesystem(img, mode) {
                        Ok(()) => println!("Filesystem formatted ({}). All previous files erased.", mode),
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
                        match filesystem_mode {
                            FileSystemType::Auto => img.default_filesystem(),
                            other => other,
                        }
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
                                println!("{}{} [{}]", prefix, result.name, result.reason);
                            } else {
                                println!("{}{} [{}]", prefix, result.name, result.reason);
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

/// Parse command line input, respecting quoted strings
fn parse_command_line(input: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;
    let mut chars = input.chars().peekable();

    while let Some(ch) = chars.next() {
        match ch {
            '"' => {
                in_quotes = !in_quotes;
            }
            ' ' | '\t' if !in_quotes => {
                if !current.is_empty() {
                    parts.push(current.clone());
                    current.clear();
                }
            }
            _ => {
                current.push(ch);
            }
        }
    }

    if !current.is_empty() {
        parts.push(current);
    }

    parts
}

fn print_help() {
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

fn print_info(image: &DiskImage) {
    if let Some(filename) = image.filename() {
        println!("Filename: {}", filename);
        println!("Changed: {}", if image.is_changed() { "Yes" } else { "No" });
    }
    println!("Format: {}", image.format().name());
    println!("Sides: {}", image.spec().num_sides);
    println!("Tracks per side: {}", image.spec().num_tracks);
    println!("Sectors per track: {}", image.spec().sectors_per_track);
    println!("Sector size: {} bytes", image.spec().sector_size);
    println!("First sector ID: {}", image.spec().first_sector_id);
    
    // Bootability detection
    let boot_detection = BootDetection::detect(image);
    if !boot_detection.system.is_empty() {
        println!("Bootable on: {}", boot_detection.system);
        println!("Reason: {}", boot_detection.reason);
    } else {
        println!("Bootable on: Not bootable");
        if !boot_detection.reason.is_empty() {
            println!("Reason: {}", boot_detection.reason);
        }
    }
}

fn list_tracks(image: &DiskImage) {
    for (side_idx, disk) in image.disks().iter().enumerate() {
        println!("\nSide {}:", side_idx);
        println!(
            "{:<8} {:<8} {:<12} {:<8} {:<4} {:<8} {:<11}",
            "Logical", "Physical", "Track Size", "Sectors", "Gap", "Filler", "Status"
        );
        println!("{}", "-".repeat(69));
        
        for (physical_idx, track) in disk.tracks().iter().enumerate() {
            // Logical track number is what the sectors claim (use first sector's track if available, otherwise track_number)
            let logical_track = if let Some(first_sector) = track.sectors().first() {
                first_sector.id.track
            } else {
                track.track_number
            };
            
            // Determine track status
            let status = if track.is_empty() {
                "Unformatted"
            } else {
                let filler = track.filler_byte;
                let has_in_use = track.sectors().iter().any(|s| {
                    matches!(s.status(filler), crate::image::SectorStatus::FormattedInUse)
                });
                if has_in_use {
                    "In use"
                } else {
                    "Blank"
                }
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

fn list_sectors_on_track(image: &DiskImage, side: u8, track: u8) {
    if let Some(disk) = image.disks().get(side as usize) {
        if let Some(track_data) = disk.get_track(track) {
            println!(
                "{:<6} {:<6} {:<6} {:<6} {:<12} {:<10} {:<10} {:<12}",
                "Sector", "Track", "Side", "ID", "FDC Size", "FDC Flags", "Data Size", "Status"
            );
            println!("{}", "-".repeat(80));

            let filler = track_data.filler_byte;
            for (idx, sector) in track_data.sectors().iter().enumerate() {
                let fdc_size = format!("{} ({})", sector.id.size_code, sector.advertised_size());
                let fdc_flags = format!("{},{}", sector.fdc_status1.0, sector.fdc_status2.0);
                let status = sector.status(filler);
                println!(
                    "{:<6} {:<6} {:<6} {:<6} {:<12} {:<10} {:<10} {:<12}",
                    idx,
                    sector.id.track,
                    sector.id.side,
                    sector.id.sector,
                    fdc_size,
                    fdc_flags,
                    sector.actual_size(),
                    status
                );
            }

        } else {
            println!("Track {} not found on side {}.", track, side);
        }
    } else {
        println!("Side {} not found.", side);
    }
}

fn list_all_sectors(image: &DiskImage) {
    for (_side_idx, disk) in image.disks().iter().enumerate() {
        for track in disk.tracks() {
            println!(
                "{:<6} {:<6} {:<6} {:<6} {:<12} {:<10} {:<10} {:<12}",
                "Sector", "Track", "Side", "ID", "FDC Size", "FDC Flags", "Data Size", "Status"
            );
            println!("{}", "-".repeat(80));

            let filler = track.filler_byte;
            for (idx, sector) in track.sectors().iter().enumerate() {
                let fdc_size = format!("{} ({})", sector.id.size_code, sector.advertised_size());
                let fdc_flags = format!("{},{}", sector.fdc_status1.0, sector.fdc_status2.0);
                let status = sector.status(filler);
                println!(
                    "{:<6} {:<6} {:<6} {:<6} {:<12} {:<10} {:<10} {:<12}",
                    idx,
                    sector.id.track,
                    sector.id.side,
                    sector.id.sector,
                    fdc_size,
                    fdc_flags,
                    sector.actual_size(),
                    status
                );
            }

        }
    }
}

fn print_hex_dump(data: &[u8], max_bytes: usize) {
    let len = data.len().min(max_bytes);

    for (i, chunk) in data[..len].chunks(16).enumerate() {
        print!("{:04X}: ", i * 16);

        // Print hex
        for (j, byte) in chunk.iter().enumerate() {
            print!("{:02X} ", byte);
            if j == 7 {
                print!(" ");
            }
        }

        // Pad if less than 16 bytes
        for j in chunk.len()..16 {
            print!("   ");
            if j == 7 {
                print!(" ");
            }
        }

        print!(" |");

        // Print ASCII
        for byte in chunk {
            let c = if *byte >= 32 && *byte < 127 {
                *byte as char
            } else {
                '.'
            };
            print!("{}", c);
        }

        println!("|");
    }

    if data.len() > max_bytes {
        println!("... ({} more bytes)", data.len() - max_bytes);
    }
}

fn parse_hex_or_dec(s: &str) -> Option<u8> {
    if s.starts_with("0x") || s.starts_with("0X") {
        u8::from_str_radix(&s[2..], 16).ok()
    } else {
        s.parse().ok()
    }
}

fn find_lowest_sector_id(image: &DiskImage, side: u8, track: u8) -> Option<u8> {
    let disk = image.disks().get(side as usize)?;
    let track_data = disk.get_track(track)?;
    track_data
        .sectors()
        .iter()
        .map(|s| s.id.sector)
        .min()
}

fn disassemble_z80(data: &[u8]) {
    let mut slice: &[u8] = data;
    let mut address: u16 = 0;

    while !slice.is_empty() {
        let start_len = slice.len();

        match Instruction::decode_one(&mut slice) {
            Ok(instruction) => {
                let bytes_consumed = start_len - slice.len();
                let bytes: Vec<String> = data[address as usize..address as usize + bytes_consumed]
                    .iter()
                    .map(|b| format!("{:02X}", b))
                    .collect();

                println!(
                    "{:04X}  {:<12} {}",
                    address,
                    bytes.join(" "),
                    instruction
                );

                address += bytes_consumed as u16;
            }
            Err(_) => {
                // Invalid instruction - show as data byte
                println!("{:04X}  {:02X}           DB {:02X}h", address, slice[0], slice[0]);
                slice = &slice[1..];
                address += 1;
            }
        }
    }
}

/// Default ASCII character set for strings command
/// Conservative set to match English-like words, not random byte sequences
fn default_ascii_chars() -> Vec<u8> {
    let mut chars = Vec::new();
    // A-Z
    chars.extend(b'A'..=b'Z');
    // a-z
    chars.extend(b'a'..=b'z');
    // 0-9
    chars.extend(b'0'..=b'9');
    // Space and common punctuation found in text
    chars.extend(b" !\"'()*+,-.:;=?".iter());
    chars
}

/// Parse a charset specification like "A-Za-z0-9 " or "32-126"
fn parse_charset(spec: &str) -> Vec<u8> {
    let mut chars = Vec::new();
    let mut iter = spec.chars().peekable();

    while let Some(ch) = iter.next() {
        if iter.peek() == Some(&'-') {
            // Range like A-Z
            iter.next(); // consume '-'
            if let Some(end_ch) = iter.next() {
                let start = ch as u8;
                let end = end_ch as u8;
                if start <= end {
                    chars.extend(start..=end);
                }
            }
        } else {
            chars.push(ch as u8);
        }
    }

    if chars.is_empty() {
        default_ascii_chars()
    } else {
        chars
    }
}

/// A string found in the disk with its location
struct StringHit {
    side: usize,
    track: u8,
    sector: u8,
    offset: usize,
    text: String,
}

/// Find strings in a disk, iterating in logical order
fn find_strings_in_disk(image: &DiskImage, min_length: usize, min_unique: usize, charset: &[u8]) -> Vec<StringHit> {
    let mut hits = Vec::new();
    let spec = image.spec();

    match spec.side_mode {
        SideMode::SingleSide => {
            if let Some(disk) = image.get_disk(0) {
                for track_num in 0..disk.track_count() {
                    find_strings_in_track(disk, 0, track_num as u8, min_length, min_unique, charset, &mut hits);
                }
            }
        }
        SideMode::Alternate => {
            let max_tracks = image
                .disks()
                .iter()
                .map(|d| d.track_count())
                .max()
                .unwrap_or(0);

            for track_num in 0..max_tracks {
                for (side_idx, disk) in image.disks().iter().enumerate() {
                    find_strings_in_track(disk, side_idx, track_num as u8, min_length, min_unique, charset, &mut hits);
                }
            }
        }
        SideMode::Successive => {
            for (side_idx, disk) in image.disks().iter().enumerate() {
                for track_num in 0..disk.track_count() {
                    find_strings_in_track(disk, side_idx, track_num as u8, min_length, min_unique, charset, &mut hits);
                }
            }
        }
    }

    hits
}

/// Count unique characters in a string
fn unique_char_count(s: &str) -> usize {
    let mut seen = std::collections::HashSet::new();
    for c in s.chars() {
        seen.insert(c);
    }
    seen.len()
}

/// Find strings in a single track
fn find_strings_in_track(
    disk: &image::Disk,
    side: usize,
    track_num: u8,
    min_length: usize,
    min_unique: usize,
    charset: &[u8],
    hits: &mut Vec<StringHit>,
) {
    let Some(track) = disk.get_track(track_num) else {
        return;
    };

    // Get sectors sorted by ID
    let mut sector_ids: Vec<u8> = track.sectors().iter().map(|s| s.id.sector).collect();
    sector_ids.sort();

    for sector_id in sector_ids {
        let Some(sector) = track.get_sector(sector_id) else {
            continue;
        };

        let data = sector.data();
        let mut current_string = String::new();
        let mut start_offset = 0;

        for (i, &byte) in data.iter().enumerate() {
            if charset.contains(&byte) {
                if current_string.is_empty() {
                    start_offset = i;
                }
                current_string.push(byte as char);
            } else {
                if current_string.len() >= min_length && unique_char_count(&current_string) >= min_unique {
                    hits.push(StringHit {
                        side,
                        track: track_num,
                        sector: sector_id,
                        offset: start_offset,
                        text: current_string.clone(),
                    });
                }
                current_string.clear();
            }
        }

        // Don't forget trailing string
        if current_string.len() >= min_length && unique_char_count(&current_string) >= min_unique {
            hits.push(StringHit {
                side,
                track: track_num,
                sector: sector_id,
                offset: start_offset,
                text: current_string,
            });
        }
    }
}

