/*!
# dskmanager

A Rust library for reading and writing DSK disk image files with CP/M filesystem support.

## Features

- Read and write Standard, Extended and SamDisk Extended DSK formats
- Track and sector abstraction with FDC status codes
- CP/M filesystem support for reading files
- Idiomatic Rust API with comprehensive error handling

## Quick Start

```rust,no_run
use dskmanager::{AnyFileSystem, DiskImage, FileSystemType, FormatSpec};

// Open an existing image and read a sector
let mut image = DiskImage::open("disk.dsk")?;
let data = image.read_sector(0, 0, 0xC1)?;
println!("sector holds {} bytes", data.len());

// Write a sector and save
let new_data = vec![0xE5; 512];
image.write_sector(0, 0, 0xC1, &new_data)?;
image.save("disk.dsk")?;

// Mount the image's filesystem (Auto picks CP/M, MGT or TR-DOS)
let fs = AnyFileSystem::mount(&image, FileSystemType::Auto)?;
for entry in fs.read_dir_extended()? {
    println!("{}: {} bytes", entry.name, entry.size);
}
let contents = fs.read_file("README.TXT")?;

// Create a new blank image
let new_image = DiskImage::create(FormatSpec::amstrad_data())?;
# Ok::<(), dskmanager::DskError>(())
```

## DSK Format Specifications

The library supports both Standard and Extended DSK formats used by:
- Amstrad CPC
- ZX Spectrum +3
- Amstrad PCW
- SAM Coupe

## Modules

- `format`: DSK format specifications and constants
- `image`: Core image data structures (DiskImage, Track, Sector)
- `filesystem`: Filesystem implementations (CP/M)
- `fdc`: FDC (Floppy Disk Controller) status codes
- `error`: Error types and Result alias
*/

#![warn(missing_docs)]
/// Amstrad BASIC tokenized file decoder (Locomotive BASIC for Amstrad CPC)
pub mod amstrad_basic;
/// Sinclair BASIC tokenized file decoder (ZX Spectrum, ZX81, etc.)
pub mod sinclair_basic;
/// Boot detection for disk images
pub mod boot;
/// Error types and Result alias
pub mod error;
/// FDC (Floppy Disk Controller) status codes
pub mod fdc;
/// Filesystem implementations (CP/M)
pub mod filesystem;
/// DSK format specifications and constants
pub mod format;
/// Core image data structures (DiskImage, Track, Sector)
pub mod image;
/// I/O operations for reading and writing DSK files
pub mod io;
/// Sector map visualization
pub mod map;
/// Copy protection detection
pub mod protection;
/// Disk image verification
pub mod verify;

// Re-export common types
pub use amstrad_basic::{decode_amstrad_basic, decode_amstrad_basic_file, can_decode_amstrad_basic};
pub use sinclair_basic::{SinclairBasicMode, decode_sinclair_basic, decode_sinclair_basic_file, can_decode_sinclair_basic};
pub use boot::BootDetection;
pub use error::{DskError, Result};
pub use fdc::{FdcStatus1, FdcStatus2};
pub use filesystem::{
    AnyFileSystem, CpmFileSystem, DirEntry, DiscipleFileSystem, ExtendedDirEntry, FileAttributes,
    FileHeader, FileSystem, FileSystemInfo, FileSystemType, HeaderType, MgtDirEntry, MgtFileSystem,
    MgtFileType, MgtSystemType, SamFileSystem,
};
pub use filesystem::try_parse_header;
pub use format::{
    AllocationSize, DiskSpecSide, DiskSpecTrack, DiskSpecification, DiskImageFormat,
    FormatSpec, SideMode,
};
pub use image::{
    DataRate, Disk, DiskImage, DiskImageBuilder, RecordingMode, Sector, SectorId, SectorStatus,
    Track,
};
pub use verify::{verify, VerifyIssue, VerifyReport, Severity};
