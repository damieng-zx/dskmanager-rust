# DSK Manager

[![Crates.io](https://img.shields.io/crates/v/dskmanager-rust.svg)](https://crates.io/crates/dskmanager-rust)
[![License](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](#license)

A command-line tool and Rust library for reading, writing, and analyzing DSK, MGT, TRD, TD0, and SCL disk files. Built for retro computing enthusiasts working with Amstrad CPC, ZX Spectrum +3/TR-DOS, Amstrad PCW, SAM Coupe, and IBM PC floppy images.

## Features

- **Read and write** Standard DSK, Extended DSK, raw MGT, raw TRD, TeleDisk (TD0) and SCL archives
- **Lossless JSON** as an editable intermediate for every supported format
- **Filesystems:** CP/M (Amstrad CPC, Spectrum +3, PCW, Tatung Einstein), MGT (DISCiPLE/+D, SAM Coupe) and TR-DOS — list, read, import, delete and format
- **Sector-level editing:** replace, fill, and edit FDC status bytes
- **Bulk analysis:** `dsk report` walks a collection (including inside ZIPs) and emits CSV or Markdown
- **Copy protection detection** for 20+ Amstrad CPC and ZX Spectrum +3 schemes
- **BASIC decoding** for AMSDOS (Locomotive) and PLUS3DOS (Sinclair) tokenised files
- **Integrity checks:** `verify` inspects structure, FDC status and filesystem consistency

## Install

```bash
cargo install dskmanager-rust
```

Or from a checkout:

```bash
cargo install --path .
```

Or run directly:

```bash
cargo run --bin dsk
```

You can also open a file directly from the command line:

```bash
dsk disk.dsk
dsk disk.mgt
dsk disk.trd
dsk disk.td0
dsk disk.scl
dsk disk.json
```

## CLI

`dsk` is an interactive REPL for exploring disk images. Open a file and poke around:

```
> open game.dsk
> info
> tracks
> sectors
> fs-list
> map
> verify
```

### Commands

**Disk management**

| Command | Description |
|---------|-------------|
| `open <path>` | Open a .dsk, .mgt, .trd, .td0, .scl, or .json file |
| `save <path>` | Save image (format determined by extension, including SCL export) |
| `create [amstrad\|system\|spectrum\|pcw\|mgt\|trdos]` | Create a disk with an initialized filesystem |
| `info` | Show disk information |
| `spec` | Show the disk specification (geometry, filesystem layout) |

**Filesystem**

| Command | Description |
|---------|-------------|
| `fs-list` | List files on the disk (`cat`, `dir`, `ls` also work) |
| `fs-read <filename>` | Display file contents (CP/M: prefix with `N:` for user N; default user 0) |
| `fs-export <filename> [output] [raw]` | Export file to host (strips headers unless `raw`; CP/M user prefix supported) |
| `fs-import <host_path> [disk_name]` | Import a file (CP/M 8.3, MGT 10-character, TR-DOS 8-character + type `.C`/`.B`/`.D`/`.#`) |
| `fs-delete <disk_name>` | Delete a file and release its allocation |
| `fs-format [auto\|cpm\|mgt\|trdos]` | Erase the disk and initialize a filesystem |
| `fs-switch [auto\|cpm\|mgt\|trdos]` | Switch filesystem driver |
| `fs-info` | Show filesystem details |

**Low-level**

| Command | Description |
|---------|-------------|
| `tracks` | List all tracks |
| `sectors` | List all sectors |
| `read-sector <side> <track> <sector>` | Dump a sector (sector ID can be hex like `0xC1`) |
| `write-sector <side> <track> <sector> <path>` | Replace a sector with a same-size host file |
| `fill-sector <side> <track> <sector> <byte>` | Fill a sector with a byte (decimal or `0xNN`) |
| `set-sector-status <side> <track> <sector> <st1> <st2>` | Edit FDC status bytes (decimal or `0xNN`) |

**Analysis**

| Command | Description |
|---------|-------------|
| `detect-protection` | Detect copy protection schemes (`protection`) |
| `disassemble [track] [sector]` | Disassemble Z80 code from a sector (`dasm`) |
| `strings [len] [uniq] [charset]` | Search for strings in the disk image |
| `map [side]` | Visual sector map (▓ in use, ░ filler, red error, yellow deleted) |
| `verify` | Verify disk structure and filesystem integrity |

## File Formats

### Disk image formats

| Format | Extension | Description |
|--------|-----------|-------------|
| Standard DSK | `.dsk` | Fixed track size (Amstrad CPC, Spectrum +3, PCW) |
| Extended DSK | `.dsk` | Variable track sizes, SAMDisk V5 extensions |
| MGT raw | `.mgt` | 800KB DSDD raw sector dump (SAM Coupe, DISCiPLE/+D) |
| TRD raw | `.trd` | TR-DOS raw sector dump (ZX Spectrum Beta Disk Interface) — **experimental** |
| TeleDisk | `.td0` | TeleDisk track/sector image, including LZHUF and legacy LZW compression; writing uses uncompressed TD0 |
| SCL | `.scl` | TR-DOS file archive, exposed as a virtual TR-DOS disk for listing and editing |
| JSON | `.json` | Human-readable, editable representation of any format |

`open` and `save` detect the format from the file extension. You can open a `.dsk`, edit it, and `save` as `.json` — or vice versa. JSON files preserve all metadata (CHRN IDs, FDC status, per-sector data lengths) so the round-trip is lossless.

`saving` to an extension the image cannot be represented in (for example a CPC image as `.scl`) fails rather than writing a misleading file, and an existing file is left untouched.

### Batch analysis

`dsk report` walks a collection and emits one row (CSV) or section (Markdown) per disk:

```bash
dsk report ./disks report.csv
dsk report ./disks report.md
dsk report "C:\\disks\\*.dsk"
```

It scans DSK, MGT, TRD, TD0 and SCL images, including those inside `.zip` archives, and reports format, protection, bootability and structural quirks (unusual sector counts or sizes, FDC errors, deleted-data marks, oversized tracks).

Filters combine with AND:

| Filter | Meaning |
|--------|---------|
| `--filter-format <text>` | Substring match on the detected format |
| `--filter-tracks <N>` | Exact track count |
| `--filter-sides <N>` | Exact side count |
| `--filter-sector-size <N>` | Exact sector size in bytes |
| `--filter-protection [text]` | Any protection, or a substring of its name |
| `--filter-has-errors` | Has FDC errors on any side |
| `--filter-quirks` | Has non-standard characteristics |

### Disk geometry presets

Built-in configurations for common formats:

- Amstrad CPC System/Data (40 tracks, 9 sectors, 512 bytes)
- ZX Spectrum +3 (40 tracks, 9 sectors, 512 bytes)
- Amstrad PCW (40 tracks, 9 sectors, 512 bytes)
- IBM PC 360K/720K (40/80 tracks, 9 sectors, 512 bytes)
- Tatung Einstein
- MGT Disciple/+D/SAM Coupe
- TR-DOS (80 tracks, 16 sectors, 256 bytes, single-sided)

### Filesystems

- **CP/M** — read, import, and delete files on Amstrad CPC, Spectrum +3, PCW, and Tatung Einstein images
- **MGT** — read, import, and delete files on DISCiPLE/+D and SAM Coupe images
- **TR-DOS** — *experimental* read, import, and delete files on ZX Spectrum Beta Disk Interface images

### Copy protection detection

Automatically detects 20+ copy protection schemes used on Amstrad CPC and ZX Spectrum +3 disks: Alkatraz, Speedlock, Hexagon, Frontier, Paul Owens, Three Inch Loader, P.M.S., DiscSYS, Mean Protection System, KBI-19, CAAV, KBI-10, and many more.

## Library

DSK Manager is also a Rust library. Add it to your project:

```toml
[dependencies]
dskmanager-rust = "0.2"
```

The crate's library target is named `dskmanager`:

```rust
use dskmanager::{AnyFileSystem, DiskImage, FileSystemType, FormatSpec};

// Open an existing image and read a sector.
let image = DiskImage::open("disk.dsk")?;
println!("{:?}", image.read_sector(0, 0, 0xC1)?.len());

// Mount whichever filesystem the image uses and list its files.
let fs = AnyFileSystem::mount(&image, FileSystemType::Auto)?;
for entry in fs.read_dir_extended()? {
    println!("{}: {} bytes", entry.name, entry.size);
}
let contents = fs.read_file("README.TXT")?;

// Create, edit and save.
let mut image = DiskImage::create(FormatSpec::amstrad_data())?;
image.write_sector(0, 0, 0xC1, &vec![0xE5; 512])?;
image.save("new_disk.dsk")?;
```

## License

MIT OR Apache-2.0 at your convenience.

## Contributing

Contributions welcome! Please ensure:

1. Code compiles without warnings: `cargo build`
2. Tests pass: `cargo test`
3. Code is formatted: `cargo fmt`
4. No clippy warnings: `cargo clippy --all-targets`

`Cargo.lock` is committed, because this crate ships a binary and the resolved
dependency graph is part of what users build. If you change a dependency, commit
the updated lock file with the change.

See [CHANGELOG.md](CHANGELOG.md) for what has changed between releases.

Run the test suite:

```bash
# Unit tests
cargo test --lib

# Integration tests
cargo test --test integration

# All tests
cargo test
```

## AI Disclosure

Several AI agents & models have been used in the development of this project referencing my original hand-written Lazarus Disk Image Manager tool.
