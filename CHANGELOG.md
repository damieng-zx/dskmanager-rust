# Changelog

All notable changes to this project are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project
adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.2.0] - 2026-11-15

This is the first tagged release; earlier development was untagged. The
"Fixed" section below lists the defects corrected since `8d6d350`, which is the
tree the previous version number referred to.

### Added

- **`AnyFileSystem`** — one mount point over the CP/M, MGT and TR-DOS drivers,
  resolving `Auto` against the image. Re-exported from the crate root.
- **`FromStr` for `FileSystemType`**, with a small
  `ParseFileSystemTypeError`.
- **`Sector::data_length()`** — the stored length as a method.
- **`ProtectionResult::confidence`** — `Confidence::{Confirmed, Probable, Maybe}`
  exposes the qualifier that was previously only in the reason text;
  `qualified_reason()` renders the old wording.
- **`MgtFileSystem::read_dir_with`** — build an extended listing from a
  per-entry header/size closure, shared by the DISCiPLE and SAM drivers.
- **`MgtFileSystem::image()`** — accessor for the mounted image.

### Changed

- `Sector` no longer stores a `data_length` field; the length is derived from
  the buffer, so the two can no longer disagree.
- `DiskAnalysis` computes the detected spec, protections, bootability and
  structural quirks once per image instead of repeatedly during a report run.
- `save` writes to a temporary file and renames it into place, so a rejected
  conversion or a mid-write failure leaves an existing file untouched. Saving
  to an unrecognised extension is now an error rather than silently writing a
  Standard DSK.
- `write_sector` rejects a buffer that disagrees with the sector's FDC size
  code; it previously stored data the header did not describe.
- `dsk report` filters match when *any* side of a disk has the characteristic.
- The `dsk` binary is split into modules (`format`, `analysis`, `sector_edit`,
  `session`, `report`), each with its own tests.
- `clipboard-win` updated 5.4.1 → 5.4.3 (5.4.1 was yanked).

### Fixed

Readers and writers, checked against each format's on-disk layout:

- **Standard DSK** no longer reads the two bytes after ST2 as a data length.
  They are unused in that format; treating them as a length produced
  multi-kilobyte sectors from ordinary disks.
- **Extended DSK** rejects a track too large for its single-byte size entry
  instead of writing a wrapped value that shifted every following track.
- **TD0** writes header byte 9 as the number of sides minus one, as the format
  specifies, and the reader decodes the same convention.
- **Duplicate sector IDs** survive `read_logical`. Copy-protected tracks
  previously returned the last duplicate for every occurrence, dropping the
  earlier sectors entirely.
- **CP/M** refuses to mount a spec with a zero sector size (from an out-of-range
  size code) instead of dividing by zero, and clamps the directory read to the
  disk's data area instead of allocating whatever a crafted spec block asked
  for.
- **CP/M** file lookups are case-insensitive, matching the upper-case names that
  import writes.
- **CP/M** verification addresses files by user number, so files outside user 0
  are no longer reported unreadable.
- **Double-sided CPC/PCW** disks are detected as double-sided. They were
  reported single-sided, hiding half the blocks and pruning side-1 directory
  entries from listings.
- **TR-DOS** listings report the recorded length rather than the sector
  allocation, and skip deleted entries.
- **TR-DOS** delete and import identify a file by name *and* type byte, so
  deleting `FOO.C` no longer removes `FOO.B`.
- **TR-DOS** verification now checks file extents against the disk size, the
  catalog's file count and length-versus-allocation.
- **MGT** listings report the recorded file size rather than the allocation, and
  `verify` labels SAM Coupe disks as SAM rather than DISCiPLE.
- **SCL** accepts entries whose allocation exceeds their length, so an archive
  this crate writes from a real TR-DOS disk can be reopened.
- **Boot detection** checksums the same sector as the format detector, so a
  correctly formatted Spectrum +3 disk is no longer reported unbootable.
- **ZX Spectrum floating point** decoding was doubled (`1.5` printed as `3`)
  and did not special-case zero.
- **Amstrad BASIC** tokens 0xE2, 0xE8 and 0xE9 report an unknown token instead
  of silently emitting nothing.
- **`disassemble`** no longer panics on a truncated opcode prefix at the end of
  a sector.
- **`report`** groups files under their true shared ancestor, and expands globs
  in-process rather than shelling out to `cmd /C dir`.
- **`fs-format <type>`** switches the session filesystem mode, so subsequent
  commands read the freshly formatted disk.
- The DSK creator signature is exactly 14 bytes. The previous literal was longer
  and got truncated mid-version when written.
- The library and binary build warning-free, and `cargo clippy --all-targets`
  reports nothing.

[0.2.0]: https://github.com/damieng-zx/dskmanager-rust/releases/tag/v0.2.0
