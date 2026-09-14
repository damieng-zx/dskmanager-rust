# DSK Manager

A command-line tool and Rust library for reading, writing, and analyzing DSK, MGT, TRD, and TD0 disk image files. Built for retro computing enthusiasts working with Amstrad CPC, ZX Spectrum +3, Amstrad PCW, SAM Coupe, and IBM PC floppy images.

## Install

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
| `open <path>` | Open a .dsk, .mgt, .trd, .td0, or .json file |
| `save <path>` | Save image (format determined by extension) |
| `create [amstrad\|spectrum\|pcw]` | Create a new blank disk image |
| `info` | Show disk information |
| `spec` | Show the disk specification (geometry, filesystem layout) |

**Filesystem**

| Command | Description |
|---------|-------------|
| `fs-list` | List files on the disk (`cat`, `dir`, `ls` also work) |
| `fs-read <filename>` | Display file contents |
| `fs-export <filename> [output] [raw]` | Export file to host (strips headers unless `raw`) |
| `fs-switch [auto\|cpm\|mgt\|trdos]` | Switch filesystem driver |
| `fs-info` | Show filesystem details |

**Low-level**

| Command | Description |
|---------|-------------|
| `tracks` | List all tracks |
| `sectors` | List all sectors |
| `read-sector <side> <track> <sector>` | Dump a sector (sector ID can be hex like `0xC1`) |

**Analysis**

| Command | Description |
|---------|-------------|
| `detect-protection` | Detect copy protection schemes |
| `disassemble [track] [sector]` | Disassemble Z80 code from a sector |
| `strings [len] [uniq] [charset]` | Search for strings in the disk image |
| `map [side]` | Visual sector map (`▓` = in use, `░` = empty) |
| `verify` | Verify disk structure and filesystem integrity |

## File Formats

### Disk image formats

| Format | Extension | Description |
|--------|-----------|-------------|
| Standard DSK | `.dsk` | Fixed track size (Amstrad CPC, Spectrum +3, PCW) |
| Extended DSK | `.dsk` | Variable track sizes, SAMDisk V5 extensions |
| MGT raw | `.mgt` | 800KB DSDD raw sector dump (SAM Coupe, DISCiPLE/+D) |
| TRD raw | `.trd` | TR-DOS raw sector dump (ZX Spectrum Beta Disk Interface) — **experimental** |
| TeleDisk | `.td0` | TeleDisk track/sector image, including LZHUF and legacy LZW compression |
| JSON | `.json` | Human-readable, editable representation of any format |

`open` and `save` detect the format from the file extension. You can open a `.dsk`, edit it, and `save` as `.json` — or vice versa. JSON files preserve all metadata (CHRN IDs, FDC status, per-sector data lengths) so the round-trip is lossless.

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

- **CP/M** — read-only support for Amstrad CPC, Spectrum +3, PCW, and Tatung Einstein
- **MGT** — read-only support for DISCiPLE/+D and SAM Coupe (SAMDOS, MasterDOS, BDOS)
- **TR-DOS** — *experimental* read-only support for the ZX Spectrum Beta Disk Interface (directory listing and file extraction; writing not yet implemented)

### Copy protection detection

Automatically detects 20+ copy protection schemes used on Amstrad CPC and ZX Spectrum +3 disks: Alkatraz, Speedlock, Hexagon, Frontier, Paul Owens, Three Inch Loader, P.M.S., DiscSYS, Mean Protection System, KBI-19, CAAV, KBI-10, and many more.

## Library

DSK Manager is also a Rust library. Add it to your project:

```toml
[dependencies]
dskmanager = "0.1"
```

```rust
use dskmanager::{DiskImage, FormatSpec, CpmFileSystem, FileSystem};

let image = DiskImage::open("disk.dsk")?;

let data = image.read_sector(0, 0, 0xC1)?;
println!("Sector data: {} bytes", data.len());

let fs = CpmFileSystem::from_image(&image)?;
for entry in fs.read_dir()? {
    println!("{}: {} bytes", entry.name, entry.size);
}

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
4. No clippy warnings: `cargo clippy`

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
