//! Round-trip precision tests for JSON as an open/save format.
//!
//! These tests assert that converting a disk image to JSON and back loses
//! *nothing*: `dsk > json > dsk`, `mgt > json > mgt` and `td0 > json > td0`
//! reproduce the original binary byte-for-byte.
//!
//! The oracle is deliberately strict. Rather than comparing the JSON detour
//! against the in-memory builder output (which can hold detail a container
//! format legitimately discards), each test:
//!
//!   1. writes the image to its binary container once  -> `direct` bytes
//!   2. routes the *same* image through JSON and writes it again -> `via_json` bytes
//!   3. asserts `direct == via_json`
//!
//! That isolates the JSON step: anything the binary writer already canonicalises
//! is canonicalised identically on both paths, so a difference can only come from
//! JSON dropping or mangling information. Structural field-by-field assertions are
//! layered on top to turn any failure into a precise diagnostic instead of an
//! opaque byte mismatch.

use dskmanager::io::mgt_reader::MGT_FILE_SIZE;
use dskmanager::io::{read_dsk, read_json, read_mgt, read_td0, write_dsk, write_json};
use dskmanager::*;
use std::path::PathBuf;

/// A unique-per-process temp path so parallel test runs don't collide.
fn temp_path(label: &str, ext: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "dskmgr_rt_{}_{}.{}",
        label,
        std::process::id(),
        ext
    ))
}

/// Build a DSK image whose every field carries distinctive, non-default data so
/// the round-trip has something real to lose if it is lossy.
///
/// Every sector is 512 bytes (size code 2) which keeps track sizes exact
/// multiples of 256, so the Standard/Extended writers produce byte-stable output.
fn build_rich_dsk(format: DiskImageFormat, sides: u8, tracks: u8, sectors_per_track: u8) -> DiskImage {
    let mut image = DiskImage::builder()
        .format(format)
        .num_sides(sides)
        .num_tracks(tracks)
        .sectors_per_track(sectors_per_track)
        .sector_size(512)
        .build()
        .expect("failed to build image");

    for side in 0..sides {
        let disk = image.get_disk_mut(side).expect("missing side");
        for t in 0..tracks {
            let track = disk.get_track_mut(t).expect("missing track");

            // Vary every track-level field away from the builder defaults.
            track.gap3_length = 0x40u8.wrapping_add(t);
            track.filler_byte = if t % 2 == 0 { 0xE5 } else { 0x00 };
            track.data_rate = match t % 3 {
                0 => DataRate::SingleDouble,
                1 => DataRate::High,
                _ => DataRate::Extended,
            };
            track.recording_mode = if side == 0 {
                RecordingMode::MFM
            } else {
                RecordingMode::FM
            };

            for (i, sector) in track.sectors_mut().iter_mut().enumerate() {
                // 512 bytes that cycle through every value 0x00..=0xFF, offset
                // per (side, track, sector) so no two sectors share a pattern.
                let seed = i
                    .wrapping_mul(7)
                    .wrapping_add(t as usize * 13)
                    .wrapping_add(side as usize * 31);
                let data: Vec<u8> = (0..512usize)
                    .map(|k| (k.wrapping_add(seed) & 0xFF) as u8)
                    .collect();
                sector.set_data(data);

                // Distinctive FDC status bytes, including error/deleted-mark bits.
                sector.fdc_status1 =
                    FdcStatus1::new((i as u8).wrapping_mul(0x11) ^ t.wrapping_add(0x20));
                sector.fdc_status2 =
                    FdcStatus2::new((i as u8).wrapping_add(0xA0) ^ side.wrapping_mul(0x05));
            }
        }
    }

    image
}

/// Assert two images are identical at the structural level, field by field, so a
/// failure points at the exact thing that diverged.
fn assert_images_match(a: &DiskImage, b: &DiskImage, ctx: &str) {
    assert_eq!(a.format(), b.format(), "{ctx}: format");

    let (sa, sb) = (a.spec(), b.spec());
    assert_eq!(sa.num_sides, sb.num_sides, "{ctx}: spec.num_sides");
    assert_eq!(sa.num_tracks, sb.num_tracks, "{ctx}: spec.num_tracks");
    assert_eq!(
        sa.sectors_per_track, sb.sectors_per_track,
        "{ctx}: spec.sectors_per_track"
    );
    assert_eq!(sa.sector_size, sb.sector_size, "{ctx}: spec.sector_size");
    assert_eq!(
        sa.first_sector_id, sb.first_sector_id,
        "{ctx}: spec.first_sector_id"
    );
    assert_eq!(sa.gap3_length, sb.gap3_length, "{ctx}: spec.gap3_length");
    assert_eq!(sa.filler_byte, sb.filler_byte, "{ctx}: spec.filler_byte");
    assert_eq!(sa.interleave, sb.interleave, "{ctx}: spec.interleave");
    assert_eq!(sa.side_mode, sb.side_mode, "{ctx}: spec.side_mode");

    assert_eq!(a.warnings(), b.warnings(), "{ctx}: warnings");

    assert_eq!(a.disk_count(), b.disk_count(), "{ctx}: disk_count");
    for side in 0..a.disk_count() as u8 {
        let da = a.get_disk(side).expect("a: missing side");
        let db = b.get_disk(side).expect("b: missing side");
        assert_eq!(da.side_number, db.side_number, "{ctx}: side {side} side_number");
        assert_eq!(
            da.track_count(),
            db.track_count(),
            "{ctx}: side {side} track_count"
        );

        for t in 0..da.track_count() as u8 {
            let ta = da.get_track(t).expect("a: missing track");
            let tb = db.get_track(t).expect("b: missing track");
            let tctx = format!("{ctx}: side {side} track {t}");

            assert_eq!(ta.track_number, tb.track_number, "{tctx}: track_number");
            assert_eq!(ta.side_number, tb.side_number, "{tctx}: side_number");
            assert_eq!(ta.gap3_length, tb.gap3_length, "{tctx}: gap3_length");
            assert_eq!(ta.filler_byte, tb.filler_byte, "{tctx}: filler_byte");
            assert_eq!(ta.data_rate, tb.data_rate, "{tctx}: data_rate");
            assert_eq!(
                ta.recording_mode, tb.recording_mode,
                "{tctx}: recording_mode"
            );
            assert_eq!(
                ta.sector_count(),
                tb.sector_count(),
                "{tctx}: sector_count"
            );

            for (i, (sa, sb)) in ta.sectors().iter().zip(tb.sectors()).enumerate() {
                let sctx = format!("{tctx} sector #{i} (id {:#04X})", sa.id.sector);
                assert_eq!(sa.id.track, sb.id.track, "{sctx}: id.track");
                assert_eq!(sa.id.side, sb.id.side, "{sctx}: id.side");
                assert_eq!(sa.id.sector, sb.id.sector, "{sctx}: id.sector");
                assert_eq!(sa.id.size_code, sb.id.size_code, "{sctx}: id.size_code");
                assert_eq!(sa.fdc_status1.0, sb.fdc_status1.0, "{sctx}: fdc_status1");
                assert_eq!(sa.fdc_status2.0, sb.fdc_status2.0, "{sctx}: fdc_status2");
                assert_eq!(sa.data_length, sb.data_length, "{sctx}: data_length");
                assert_eq!(sa.data(), sb.data(), "{sctx}: data");
            }
        }
    }
}

/// Core oracle for `dsk > json > dsk`: the binary produced via the JSON detour
/// must be byte-identical to the binary produced directly.
fn assert_dsk_json_dsk_lossless(format: DiskImageFormat, label: &str) {
    let image = build_rich_dsk(format, 2, 5, 9);

    let src = temp_path(label, "dsk");
    let direct = temp_path(&format!("{label}_direct"), "dsk");
    let json = temp_path(label, "json");
    let via_json = temp_path(&format!("{label}_viajson"), "dsk");

    // Establish the canonical binary form, then read it back. `from` is what the
    // DSK container actually preserves -- our reference point for "no loss".
    write_dsk(&image, &src).expect("write src dsk");
    let from = read_dsk(&src).expect("read src dsk");

    // Path A: straight binary rewrite (no JSON).
    write_dsk(&from, &direct).expect("write direct dsk");

    // Path B: through JSON and back, then rewrite the binary.
    write_json(&from, &json).expect("write json");
    let restored = read_json(&json).expect("read json");
    write_dsk(&restored, &via_json).expect("write via-json dsk");

    // Structural equality first -- precise diagnostics on failure.
    assert_images_match(&from, &restored, &format!("{label} dsk>json>dsk"));

    // Then the strict byte oracle.
    let direct_bytes = std::fs::read(&direct).expect("read direct bytes");
    let via_json_bytes = std::fs::read(&via_json).expect("read via-json bytes");
    assert_eq!(
        direct_bytes.len(),
        via_json_bytes.len(),
        "{label}: binary length differs after json round-trip"
    );
    assert!(
        direct_bytes == via_json_bytes,
        "{label}: binary differs after dsk>json>dsk round-trip"
    );

    for p in [&src, &direct, &json, &via_json] {
        std::fs::remove_file(p).ok();
    }
}

#[test]
fn dsk_json_dsk_standard_is_lossless() {
    assert_dsk_json_dsk_lossless(DiskImageFormat::StandardDSK, "std");
}

#[test]
fn dsk_json_dsk_extended_is_lossless() {
    assert_dsk_json_dsk_lossless(DiskImageFormat::ExtendedDSK, "ext");
}

/// Build a deterministic, fully-populated 819,200-byte MGT image. Every byte is
/// distinct enough that a misordered or dropped sector during the round-trip
/// would change the output.
fn build_mgt_bytes() -> Vec<u8> {
    let mut data = vec![0u8; MGT_FILE_SIZE];
    for (i, b) in data.iter_mut().enumerate() {
        // Cheap deterministic hash so adjacent bytes/sectors differ.
        let x = (i as u64).wrapping_mul(2_654_435_761) ^ (i as u64 >> 3);
        *b = (x & 0xFF) as u8;
    }
    data
}

#[test]
fn mgt_json_mgt_is_lossless() {
    let label = "mgt";
    let bytes = build_mgt_bytes();

    let src = temp_path(label, "mgt");
    let json = temp_path(label, "json");
    let via_json = temp_path(&format!("{label}_viajson"), "mgt");

    std::fs::write(&src, &bytes).expect("write src mgt");
    let from = read_mgt(&src).expect("read src mgt");

    // Sanity: the reader produced the canonical MGT geometry.
    assert_eq!(from.format(), DiskImageFormat::RawMgt, "mgt: format");
    assert_eq!(from.disk_count(), 2, "mgt: sides");
    for side in 0..2u8 {
        let disk = from.get_disk(side).unwrap();
        assert_eq!(disk.track_count(), 80, "mgt: tracks on side {side}");
        for t in 0..80u8 {
            let track = disk.get_track(t).unwrap();
            assert_eq!(track.sector_count(), 10, "mgt: sectors side {side} track {t}");
            let ids: Vec<u8> = track.sectors().iter().map(|s| s.id.sector).collect();
            assert_eq!(ids, (1..=10).collect::<Vec<u8>>(), "mgt: ids side {side} track {t}");
        }
    }

    // Through JSON and back.
    write_json(&from, &json).expect("write json");
    let restored = read_json(&json).expect("read json");
    assert_images_match(&from, &restored, "mgt>json>mgt");

    // Rewrite the binary (RawMgt dispatches to the MGT writer) and compare to the
    // original bytes -- the true precision oracle.
    write_dsk(&restored, &via_json).expect("write via-json mgt");
    let out = std::fs::read(&via_json).expect("read via-json bytes");
    assert_eq!(out.len(), MGT_FILE_SIZE, "mgt: output size");
    assert!(
        out == bytes,
        "mgt: binary differs after mgt>json>mgt round-trip"
    );

    for p in [&src, &json, &via_json] {
        std::fs::remove_file(p).ok();
    }
}

/// A double-sided preset routed through JSON must come back identical -- guards
/// the realistic "open a real disk, save as JSON, reopen" workflow.
#[test]
fn preset_dsk_survives_json_roundtrip() {
    let mut image = DiskImage::create(FormatSpec::spectrum_plus3_ds()).expect("create preset");

    // Stamp recognisable data so empty-disk equality can't pass trivially.
    let first = image.spec().first_sector_id;
    let data: Vec<u8> = (0..512u16).map(|i| (i % 251) as u8).collect();
    image.write_sector(0, 0, first, &data).expect("write sector");
    image
        .write_sector(1, 39, first + 2, &data)
        .expect("write sector");

    let src = temp_path("preset", "dsk");
    let json = temp_path("preset", "json");
    write_dsk(&image, &src).expect("write src");
    let from = read_dsk(&src).expect("read src");

    write_json(&from, &json).expect("write json");
    let restored = read_json(&json).expect("read json");

    assert_images_match(&from, &restored, "preset dsk>json>dsk");

    for p in [&src, &json] {
        std::fs::remove_file(p).ok();
    }
}

/// Build a TeleDisk-shaped image whose sectors exercise every condition TD0 can
/// actually encode: a clean sector, a CRC error, a deleted address mark, and a
/// sector with no stored data at all.
///
/// Sector data is 512 bytes (size code 2) and unique per (side, track, sector),
/// so a misordered or dropped sector shows up as a byte mismatch.
fn build_rich_td0(sides: u8, tracks: u8, sectors_per_track: u8) -> DiskImage {
    let mut image = DiskImage::builder()
        .format(DiskImageFormat::RawTd0)
        .num_sides(sides)
        .num_tracks(tracks)
        .sectors_per_track(sectors_per_track)
        .sector_size(512)
        .build()
        .expect("failed to build td0 image");

    for side in 0..sides {
        let disk = image.get_disk_mut(side).expect("missing side");
        for t in 0..tracks {
            let track = disk.get_track_mut(t).expect("missing track");

            for (i, sector) in track.sectors_mut().iter_mut().enumerate() {
                let seed = i
                    .wrapping_mul(11)
                    .wrapping_add(t as usize * 17)
                    .wrapping_add(side as usize * 37);
                let data: Vec<u8> = (0..512usize)
                    .map(|k| (k.wrapping_mul(3).wrapping_add(seed) & 0xFF) as u8)
                    .collect();
                sector.set_data(data);

                // Cycle through the four sector states TeleDisk records.
                match (i + t as usize + side as usize) % 4 {
                    0 => {}
                    1 => sector.fdc_status1 = FdcStatus1::new(FdcStatus1::DE),
                    2 => sector.fdc_status2 = FdcStatus2::new(FdcStatus2::CM),
                    // No stored data: the writer emits the no-data flag and the
                    // reader hands back filler plus a missing-data status.
                    _ => sector.set_data(Vec::new()),
                }
            }
        }
    }

    image
}

/// `td0 > json > td0` must be byte-identical to writing the TD0 directly.
#[test]
fn td0_json_td0_is_lossless() {
    let label = "td0";
    let image = build_rich_td0(2, 5, 9);

    let src = temp_path(label, "td0");
    let direct = temp_path(&format!("{label}_direct"), "td0");
    let json = temp_path(label, "json");
    let via_json = temp_path(&format!("{label}_viajson"), "td0");

    // Canonical binary form, read back. `from` is what TD0 actually preserves.
    write_dsk(&image, &src).expect("write src td0");
    let from = read_td0(&src).expect("read src td0");

    // Sanity: the reader recovered the geometry and the per-sector conditions,
    // so the round-trip below is carrying real detail rather than blank tracks.
    assert_eq!(from.format(), DiskImageFormat::RawTd0, "td0: format");
    assert_eq!(from.disk_count(), 2, "td0: sides");
    let probe = from.get_disk(0).unwrap().get_track(0).unwrap();
    assert_eq!(probe.sector_count(), 9, "td0: sectors on side 0 track 0");
    assert!(
        probe.sectors().iter().any(|s| s.has_error()),
        "td0: expected a sector carrying an FDC error"
    );
    assert!(
        probe.sectors().iter().any(|s| s.is_deleted()),
        "td0: expected a sector with a deleted address mark"
    );

    // Path A: straight binary rewrite (no JSON).
    write_dsk(&from, &direct).expect("write direct td0");

    // Path B: through JSON and back, then rewrite the binary.
    write_json(&from, &json).expect("write json");
    let restored = read_json(&json).expect("read json");
    write_dsk(&restored, &via_json).expect("write via-json td0");

    // Structural equality first -- precise diagnostics on failure.
    assert_images_match(&from, &restored, "td0>json>td0");

    // Then the strict byte oracle.
    let direct_bytes = std::fs::read(&direct).expect("read direct bytes");
    let via_json_bytes = std::fs::read(&via_json).expect("read via-json bytes");
    assert_eq!(
        direct_bytes.len(),
        via_json_bytes.len(),
        "td0: binary length differs after json round-trip"
    );
    assert!(
        direct_bytes == via_json_bytes,
        "td0: binary differs after td0>json>td0 round-trip"
    );

    for p in [&src, &direct, &json, &via_json] {
        std::fs::remove_file(p).ok();
    }
}
