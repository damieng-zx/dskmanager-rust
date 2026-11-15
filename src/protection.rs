//! Copy protection detection for DSK disk images
//!
//! Uses a fingerprinting flow that identifies every known CPC / ZX Spectrum +3
//! copy-protection scheme with the minimum number of reads. Starts at track 0
//! and branches out only as far as needed.

use crate::image::{Disk, Sector, Track};

/// Result of copy protection detection
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProtectionResult {
    /// Name of the detected protection scheme
    pub name: String,
    /// Description of why this protection was detected
    pub reason: String,
    /// Extra detail lines (e.g. Alkatraz protection track analysis)
    pub details: Vec<String>,
}

impl ProtectionResult {
    fn confirmed(name: impl Into<String>, reason: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            reason: reason.into(),
            details: Vec::new(),
        }
    }

    fn probable(name: impl Into<String>, reason: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            reason: format!("probably, {}", reason.into()),
            details: Vec::new(),
        }
    }

    fn maybe(name: impl Into<String>, reason: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            reason: format!("maybe, {}", reason.into()),
            details: Vec::new(),
        }
    }
}

impl std::fmt::Display for ProtectionResult {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} ({})", self.name, self.reason)?;
        for line in &self.details {
            write!(f, "\n  {}", line)?;
        }
        Ok(())
    }
}

// ============================================================================
// Helpers
// ============================================================================

fn find_pattern(data: &[u8], pattern: &[u8]) -> Option<usize> {
    if pattern.is_empty() || data.len() < pattern.len() {
        return None;
    }
    data.windows(pattern.len())
        .position(|window| window == pattern)
}

fn contains(track: &Track, pattern: &[u8]) -> Option<(usize, usize)> {
    for s_idx in 0..track.sector_count() {
        if let Some(sector) = track.get_sector_by_index(s_idx) {
            if find_pattern(sector.data(), pattern).is_some() {
                return Some((s_idx, 0));
            }
        }
    }
    None
}

fn sector_contains(sector: &Sector, pattern: &[u8]) -> Option<usize> {
    find_pattern(sector.data(), pattern)
}

fn is_uniform(disk: &Disk) -> bool {
    if disk.track_count() == 0 {
        return true;
    }
    let first = match disk.get_track(0) {
        Some(t) => t,
        None => return true,
    };
    let sc = first.sector_count();
    let sz = first.uniform_sector_size();
    for t in 1..disk.track_count() {
        if let Some(track) = disk.get_track(t as u8) {
            if track.sector_count() != sc || track.uniform_sector_size() != sz {
                return false;
            }
        }
    }
    true
}

fn has_fdc_errors(disk: &Disk) -> bool {
    for t in 0..disk.track_count() {
        let Some(track) = disk.get_track(t as u8) else { continue };
        for s in 0..track.sector_count() {
            if let Some(sector) = track.get_sector_by_index(s) {
                if sector.has_error() {
                    return true;
                }
            }
        }
    }
    false
}

fn is_discsys_track(track: &Track) -> bool {
    if track.sector_count() != 16 {
        return false;
    }
    (0..16).all(|i| {
        track
            .get_sector_by_index(i)
            .map(|s| {
                s.id.sector == i as u8
                    && s.id.track == i as u8
                    && s.id.side == i as u8
                    && s.id.size_code == i as u8
            })
            .unwrap_or(false)
    })
}

fn is_players_track(track: &Track) -> bool {
    if track.sector_count() != 16 {
        return false;
    }
    (0..16).all(|i| {
        track
            .get_sector_by_index(i)
            .map(|s| s.id.sector == i as u8 && s.id.size_code == i as u8)
            .unwrap_or(false)
    })
}

fn is_cpc_disk(t0: &Track) -> bool {
    t0.get_sector_by_index(0).map(|s| s.id.sector >= 65).unwrap_or(false)
}

// ============================================================================
// T0 Classification (§3.2)
// ============================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum T0Class {
    SpeedlockPlus3,
    BigSector,
    TenSector,
    TenSectorDDAM,
    EighteenSector,
    SixteenSector,
    NineteenSector,
    EightSector,
    FiveSector,
    Speedlock9x512,
    Standard,
}

fn classify_t0(t0: &Track) -> T0Class {
    let sc = t0.sector_count();

    // Check 10-sector cases first — more specific than the ≥7 + DDAM check below
    if sc == 10 {
        if let Some(s8) = t0.get_sector_by_index(8) {
            if s8.actual_size() == 512 {
                if t0.sectors().iter().any(|s| s.is_deleted()) {
                    return T0Class::TenSectorDDAM;
                }
                return T0Class::TenSector;
            }
        }
    }

    if sc >= 7 && t0.sectors().iter().any(|s| s.is_deleted()) {
        return T0Class::SpeedlockPlus3;
    }

    if sc == 1 {
        if let Some(s0) = t0.get_sector_by_index(0) {
            if s0.id.size_code == 6 && s0.fdc_status1.0 == 0x20 {
                return T0Class::BigSector;
            }
        }
    }

    if sc == 18 {
        return T0Class::EighteenSector;
    }
    if sc == 19 {
        return T0Class::NineteenSector;
    }
    if sc == 16 {
        return T0Class::SixteenSector;
    }

    if sc == 8 {
        if t0.sectors().iter().all(|s| s.advertised_size() == 512) {
            return T0Class::EightSector;
        }
    }

    if sc == 5 {
        if t0.sectors().iter().all(|s| s.advertised_size() == 1024) {
            return T0Class::FiveSector;
        }
    }

    let has_high_id_filler = t0.sectors().iter().any(|s| {
        s.id.sector >= 0x80 && s.id.sector < 0xC1 && s.id.size_code == 2
    });
    let has_weak_undersized = t0.sectors().iter().any(|s| {
        s.id.size_code == 0 && (s.fdc_status1.0 & 0x20 != 0 || s.fdc_status2.0 & 0x20 != 0)
    });
    if has_high_id_filler && has_weak_undersized {
        return T0Class::Speedlock9x512;
    }

    T0Class::Standard
}

// ============================================================================
// Step 1a: T0 Signature Scan (§3.1)
// ============================================================================

fn scan_t0_signatures(t0: &Track) -> Option<ProtectionResult> {
    let s0 = t0.get_sector_by_index(0)?;

    if sector_contains(s0, b" THE ALKATRAZ PROTECTION SYSTEM   (C) 1987  Appleby Associates").is_some() {
        return Some(ProtectionResult::confirmed("Alkatraz +3", "signed T0/S0"));
    }

    let ti_addr = b"***Loader Copyright Three Inch Software 1988, All Rights Reserved. Three Inch Software, 73 Surbiton Road, Kingston upon Thames, KT1 2HG***";
    let ti_phone = b"***Loader Copyright Three Inch Software 1988, All Rights Reserved. 01-546 2754";

    if sector_contains(s0, ti_addr).is_some() {
        return Some(ProtectionResult::confirmed("Three Inch Loader type 1", "signed T0/S0"));
    }
    if t0.sector_count() > 7 {
        if let Some(s7) = t0.get_sector_by_index(7) {
            if sector_contains(s7, ti_addr).is_some() {
                return Some(ProtectionResult::confirmed(
                    "Three Inch Loader type 1-0-7",
                    "signed T0/S7",
                ));
            }
        }
    }
    if sector_contains(s0, ti_phone).is_some() {
        return Some(ProtectionResult::confirmed("Three Inch Loader type 2", "signed T0/S0"));
    }

    if t0.sector_count() > 2 {
        if let Some(s2) = t0.get_sector_by_index(2) {
            if sector_contains(s2, b"Laser Load   By C.J.Pink For Consult Computer    Systems").is_some() {
                return Some(ProtectionResult::confirmed(
                    "Laser Load by C.J. Pink",
                    "signed T0/S2",
                ));
            }
        }
    }

    let pms_sigs: &[(&str, &[u8])] = &[
        ("P.M.S. 1986", b"[C] P.M.S. 1986"),
        ("P.M.S. Loader 1986 v1", b"P.M.S. LOADER [C]1986"),
        ("P.M.S. Loader 1986 v2", b"P.M.S.LOADER [C]1986"),
        ("P.M.S. 1987", b"P.M.S.LOADER [C]1987"),
    ];
    for (name, sig) in pms_sigs {
        if sector_contains(s0, sig).is_some() {
            return Some(ProtectionResult::confirmed(*name, "signed T0/S0"));
        }
    }

    if t0.sector_count() > 6 {
        for s_idx in 0..t0.sector_count() {
            if let Some(sector) = t0.get_sector_by_index(s_idx) {
                if sector_contains(sector, b"PROTECTION      Remi HERBULOT").is_some() {
                    return Some(ProtectionResult::confirmed(
                        "ERE/Remi HERBULOT",
                        "signed T0",
                    ));
                }
                if sector_contains(sector, b"PROTECTION  V2.1Remi HERBULOT").is_some() {
                    return Some(ProtectionResult::confirmed(
                        "ERE/Remi HERBULOT 2.1",
                        "signed T0",
                    ));
                }
            }
        }
    }

    if t0.sector_count() == 9 {
        if find_pattern(s0.data(), b"0K free") == Some(2) {
            return Some(ProtectionResult::confirmed(
                "ARMOURLOC",
                "anti-hacker protection",
            ));
        }
    }

    if sector_contains(s0, b"Disc format (c) 1986 Studio B Ltd.").is_some() {
        return Some(ProtectionResult::confirmed(
            "Studio B Disc format",
            "signed T0/S0",
        ));
    }

    None
}

// ============================================================================
// Step 1 Resolvers (§4)
// ============================================================================

/// §4.1 — T0 has deleted marks (Speedlock +3)
fn resolve_speedlock_plus3(disk: &Disk, t0: &Track) -> Option<ProtectionResult> {
    let t1 = disk.get_track(1)?;
    if t1.sector_count() == 5 {
        if let Some(t1s0) = t1.get_sector_by_index(0) {
            if t1s0.advertised_size() == 1024 {
                if t0.sector_count() == 9 {
                    if let (Some(s6), Some(s8)) = (
                        t0.get_sector_by_index(6),
                        t0.get_sector_by_index(8),
                    ) {
                        if s6.fdc_status2.0 == 0x40 && s8.fdc_status2.0 == 0x00 {
                            return Some(ProtectionResult::probable(
                                "Speedlock +3 1987",
                                "unsigned",
                            ));
                        }
                        if s6.fdc_status2.0 == 0x40 && s8.fdc_status2.0 == 0x40 {
                            return Some(ProtectionResult::probable(
                                "Speedlock +3 1988",
                                "unsigned",
                            ));
                        }
                    }
                }
                return Some(ProtectionResult::probable(
                    "Speedlock +3 1987/1988",
                    format!("unsigned (T0={} sectors with deleted data)", t0.sector_count()),
                ));
            }
        }
    }
    None
}

/// §4.2 — T0 is 1 giant weak sector
fn resolve_big_sector(disk: &Disk, t0: &Track) -> Option<ProtectionResult> {
    let cpc = is_cpc_disk(t0);

    if let Some(t1) = disk.get_track(1) {
        if t1.sector_count() == 1 {
            if let Some(t1s0) = t1.get_sector_by_index(0) {
                if t1s0.fdc_status1.0 == 0x20 {
                    if cpc {
                        return Some(ProtectionResult::probable(
                            "Hexagon",
                            "CPC big-sector engine",
                        ));
                    }
                    return Some(ProtectionResult::probable(
                        "Speedlock 1989/1990",
                        "+3 big-sector engine",
                    ));
                }
            }
        }
    }

    if cpc {
        return Some(ProtectionResult::probable(
            "Hexagon",
            "CPC, T0 big sector",
        ));
    }
    Some(ProtectionResult::probable(
        "Speedlock 1989/1990",
        "+3, T0 big sector",
    ))
}

/// §4.3 — T0 has 10 clean sectors, no DDAM (Hexagon)
fn resolve_hexagon(disk: &Disk, t0: &Track) -> Option<ProtectionResult> {
    let cpc = is_cpc_disk(t0);
    let limit = 4.min(disk.track_count());
    for t in 0..limit {
        let Some(track) = disk.get_track(t as u8) else { continue };

        for pattern in [
            &b"HEXAGON DISK PROTECTION c 1989"[..],
            &b"HEXAGON Disk Protection c 1989"[..],
        ] {
            if let Some((s_idx, _)) = contains(track, pattern) {
                return Some(ProtectionResult::confirmed(
                    "Hexagon",
                    format!("signed T{}/S{}", t, s_idx),
                ));
            }
        }

        if track.sector_count() == 1 {
            if let Some(s0) = track.get_sector_by_index(0) {
                if s0.id.size_code == 6
                    && s0.fdc_status1.0 == 0x20
                    && s0.fdc_status2.0 == 0x60
                {
                    if cpc {
                        return Some(ProtectionResult::probable(
                            "Hexagon",
                            "CPC, unsigned",
                        ));
                    }
                    return Some(ProtectionResult::probable(
                        "Hexagon",
                        "unsigned",
                    ));
                }
            }
        }
    }
    None
}

/// §4.3b — T0 has 10 sectors with DDAM (Speedlock 1989 CPC)
fn resolve_speedlock_1989_cpc(disk: &Disk, t0: &Track) -> Option<ProtectionResult> {
    let cpc = is_cpc_disk(t0);

    if cpc {
        if let Some(t1) = disk.get_track(1) {
            if t1.sector_count() == 1 {
                if let Some(t1s0) = t1.get_sector_by_index(0) {
                    if t1s0.fdc_status1.0 == 0x20 {
                        return Some(ProtectionResult::probable(
                            "Speedlock 1989",
                            "CPC, 10-sector T0 + DDAM + big-sector T1",
                        ));
                    }
                }
            }
        }
        return Some(ProtectionResult::probable(
            "Speedlock 1989",
            "CPC, 10-sector T0 + DDAM",
        ));
    }

    if let Some(t1) = disk.get_track(1) {
        if t1.sector_count() == 1 {
            if let Some(t1s0) = t1.get_sector_by_index(0) {
                if t1s0.fdc_status1.0 == 0x20 {
                    return Some(ProtectionResult::probable(
                        "Speedlock 1989/1990",
                        "+3, 10-sector T0 + DDAM + big-sector T1",
                    ));
                }
            }
        }
    }

    None
}

/// §4.4 — T0 has 18 sectors (Alkatraz CPC)
fn resolve_18sector(disk: &Disk, t0: &Track) -> Option<ProtectionResult> {
    if let Some(s0) = t0.get_sector_by_index(0) {
        if s0.actual_size() == 256 || s0.advertised_size() == 256 {
            return Some(ProtectionResult::confirmed(
                "Alkatraz CPC",
                "18-sector T0, 256B sectors",
            ));
        }
    }

    if let Some(t1) = disk.get_track(1) {
        if t1.sector_count() > 0 {
            if let Some(t1s0) = t1.get_sector_by_index(0) {
                if t1s0.fdc_status2.0 == 0x40 {
                    return Some(ProtectionResult::confirmed(
                        "Alkatraz CPC",
                        "18-sector T0",
                    ));
                }
            }
        }
    }

    Some(ProtectionResult::maybe(
        "18-sector track",
        "unknown scheme",
    ))
}

/// §4.5 — T0 has 16 sectors (DiscSYS / Players / Mean PS)
fn resolve_16sector(disk: &Disk, t0: &Track) -> Option<ProtectionResult> {
    if is_discsys_track(t0) {
        for s_idx in 0..t0.sector_count() {
            if let Some(sector) = t0.get_sector_by_index(s_idx) {
                if sector_contains(sector, b"MEAN PROTECTION SYSTEM").is_some() {
                    return Some(ProtectionResult::confirmed(
                        "Mean Protection System",
                        "signed T0",
                    ));
                }
            }
        }

        let mut reason = "16-sector CHRN ramp on T0".to_string();
        if let Some(t2) = disk.get_track(2) {
            if let Some(s4) = t2.get_sector_by_index(4) {
                if s4.actual_size() > 160 && s4.data().len() > 107 {
                    let data = s4.data();
                    let start = 85.min(data.len());
                    let end = (start + 22).min(data.len());
                    let extracted: String = data[start..end]
                        .iter()
                        .filter(|&&b| b >= 32 && b < 127)
                        .map(|&b| b as char)
                        .collect();
                    let cleaned = extracted.trim().to_lowercase();
                    if cleaned.starts_with("discsys") && cleaned.len() > 8 {
                        reason = format!("{} ({})", reason, &cleaned[8..].trim());
                    } else if cleaned.starts_with("multi-") {
                        reason = format!("{} ({})", reason, cleaned);
                    }
                }
            }
        }

        return Some(ProtectionResult::confirmed("DiscSYS", reason));
    }

    if is_players_track(t0) {
        let largest = (0..disk.track_count())
            .filter_map(|t| disk.get_track(t as u8))
            .map(|t| t.total_data_size())
            .max()
            .unwrap_or(0);
        return Some(ProtectionResult::maybe(
            "Players",
            format!("super-sized {} byte track", largest),
        ));
    }

    None
}

/// §4.6 — T0 has 19 sectors (KBI-19 / CAAV)
fn resolve_19sector(t0: &Track) -> Option<ProtectionResult> {
    if t0.sector_count() > 1 {
        if let Some(s1) = t0.get_sector_by_index(1) {
            if sector_contains(s1, b"(c) 1986 for KBI ").is_some() {
                return Some(ProtectionResult::confirmed("KBI-19", "signed T0/S1"));
            }
        }
    }

    if let Some(s0) = t0.get_sector_by_index(0) {
        if sector_contains(s0, b"ALAIN LAURENT GENERATION 5 1989").is_some() {
            return Some(ProtectionResult::confirmed("CAAV", "signed T0/S0"));
        }
    }

    Some(ProtectionResult::probable(
        "KBI-19 or CAAV",
        "unsigned, 19-sector T0",
    ))
}

/// §4.7 — T0 has 8 x 512-byte sectors (unsigned Alkatraz +3 or CPC)
fn resolve_8sector(disk: &Disk, t0: &Track) -> Option<ProtectionResult> {
    let cpc = is_cpc_disk(t0);

    let t1 = match disk.get_track(1) {
        Some(t) => t,
        None => return None,
    };

    if t1.sector_count() == 8 {
        if let Some(t1s0) = t1.get_sector_by_index(0) {
            if t1s0.advertised_size() == 512 {
                let limit = disk.track_count().min(42);
                for t in 2..limit {
                    let Some(ht) = disk.get_track(t as u8) else { continue };
                    if ht.sector_count() == 18 {
                        if let Some(hs0) = ht.get_sector_by_index(0) {
                            if hs0.advertised_size() == 256 || hs0.actual_size() == 256 {
                                let name = if cpc { "Alkatraz CPC" } else { "Alkatraz +3" };
                                return Some(ProtectionResult::probable(
                                    name,
                                    format!(
                                        "unsigned (8-sector data + 18-sector protection at T{})",
                                        t
                                    ),
                                ));
                            }
                        }
                        break;
                    }
                    if ht.sector_count() == 8 {
                        continue;
                    }
                    if ht.sector_count() == 9 {
                        break;
                    }
                }
                if cpc {
                    return None;
                }
                return Some(ProtectionResult::maybe(
                    "Alkatraz +3",
                    "unsigned (uniform 8x512 data tracks, no signature found)",
                ));
            }
        }
    }

    None
}

/// §4.8 — T0 has 5 x 1024-byte sectors (unsigned Speedlock data side)
fn resolve_5sector(disk: &Disk, t0: &Track) -> Option<ProtectionResult> {
    let cpc = is_cpc_disk(t0);

    if let Some(t1) = disk.get_track(1) {
        if t1.sector_count() == 5 {
            if let Some(t1s0) = t1.get_sector_by_index(0) {
                if t1s0.advertised_size() == 1024 {
                    if cpc {
                        return Some(ProtectionResult::probable(
                            "Speedlock (CPC)",
                            "unsigned data side (5x1024 uniform)",
                        ));
                    }
                    return Some(ProtectionResult::probable(
                        "Speedlock +3 1987/1988",
                        "unsigned data side (5x1024 uniform)",
                    ));
                }
            }
        }
    }

    None
}

/// Speedlock 9x512 variant: high-ID fillers + weak N=0 sector + DDAM payload
fn resolve_speedlock_9x512(disk: &Disk, _t0: &Track) -> Option<ProtectionResult> {
    let bulk_is_ddam = (4..disk.track_count().min(40)).any(|t_idx| {
        disk.get_track(t_idx as u8)
            .map(|t| t.sectors().iter().any(|s| s.is_deleted()))
            .unwrap_or(false)
    });

    if bulk_is_ddam {
        return Some(ProtectionResult::confirmed(
            "Speedlock +3 1987",
            "9x512 variant: high-ID T0 sectors + weak N=0 sector, DDAM data",
        ));
    }

    None
}

// ============================================================================
// Step 2: Track 1 (§5)
// ============================================================================

/// §5.1 — T1 is empty (track-1-gap family)
fn resolve_empty_t1_family(disk: &Disk, t0: &Track) -> Option<ProtectionResult> {
    let t2 = match disk.get_track(2) {
        Some(t) => t,
        None => {
            return Some(ProtectionResult::maybe(
                "P.M.S. Loader 1986/1987",
                "unsigned (T0 used / T1 empty / T2 missing)",
            ));
        }
    };

    let mut sig = b"PAUL OWENS".to_vec();
    sig.push(0x80);
    sig.extend_from_slice(b"PROTECTION SYS");

    if t0.sector_count() == 9 {
        if let Some(s2) = t0.get_sector_by_index(2) {
            if sector_contains(s2, &sig).is_some() {
                return Some(ProtectionResult::confirmed(
                    "Paul Owens",
                    "signed T0/S2",
                ));
            }
        }
    }

    if t2.sector_count() > 0 {
        if let Some(t2s0) = t2.get_sector_by_index(0) {
            if sector_contains(t2s0, b"DISCLOC").is_some() {
                return Some(ProtectionResult::confirmed(
                    "DiscLoc/Oddball",
                    "signed T2/S0",
                ));
            }
        }
    }

    if is_discsys_track(t2) {
        for s_idx in 0..t0.sector_count() {
            if let Some(sector) = t0.get_sector_by_index(s_idx) {
                if sector_contains(sector, b"MEAN PROTECTION SYSTEM").is_some() {
                    return Some(ProtectionResult::confirmed(
                        "Mean Protection System",
                        "signed T0 + DiscSYS T2",
                    ));
                }
            }
        }
    }

    if t2.sector_count() == 6 {
        if let Some(t2s0) = t2.get_sector_by_index(0) {
            if t2s0.actual_size() == 256 {
                return Some(ProtectionResult::probable(
                    "Paul Owens",
                    "unsigned",
                ));
            }
        }
    }

    Some(ProtectionResult::maybe(
        "P.M.S. Loader 1986/1987",
        "unsigned (T0 used / T1 empty / T2 used)",
    ))
}

/// §5.2 — T1 is 5 x 1024, T0 has no DDAM (Speedlock data side)
fn resolve_speedlock_5x1024(t0: &Track, _t1: &Track) -> Option<ProtectionResult> {
    if t0.sectors().iter().any(|s| s.is_deleted()) {
        return None;
    }
    let name = if is_cpc_disk(t0) {
        "Speedlock (CPC)"
    } else {
        "Speedlock +3 1987/1988"
    };
    Some(ProtectionResult::probable(
        name,
        "data side (5x1024 T1, no deleted-data marks on T0)",
    ))
}

/// §5.3 — T1 is 1 weak big sector
fn resolve_speedlock_1989(t0: &Track, _t1: &Track) -> Option<ProtectionResult> {
    let cpc = is_cpc_disk(t0);
    let has_ddam = t0.sectors().iter().any(|s| s.is_deleted());

    if cpc {
        return Some(ProtectionResult::probable(
            "Hexagon",
            "CPC, standard T0 + big-sector T1",
        ));
    }

    if has_ddam {
        return Some(ProtectionResult::probable(
            "Speedlock 1989/1990",
            "+3, T0 DDAM + big-sector T1",
        ));
    }

    Some(ProtectionResult::probable(
        "Speedlock 1989/1990",
        "+3, standard T0 + big-sector T1",
    ))
}

// ============================================================================
// Step 3: High-track probes (§6)
// ============================================================================

fn scan_high_tracks(disk: &Disk, t0: &Track) -> Option<ProtectionResult> {
    let tc = disk.track_count();

    if tc > 3 {
        if let Some(t3) = disk.get_track(3) {
            if t3.sector_count() > 0 {
                if let Some(t3s0) = t3.get_sector_by_index(0) {
                    if t3s0.actual_size() == 512 {
                        if let Some(offset) = find_pattern(t3s0.data(), b"Amsoft disc protection system") {
                            if offset > 1 && sector_contains(t3s0, b"EXOPAL").is_some() {
                                return Some(ProtectionResult::confirmed(
                                    "Amsoft/EXOPAL",
                                    "signed T3/S0",
                                ));
                            }
                        }
                    }
                }
            }
        }
    }

    if tc > 9 {
        if let Some(t8) = disk.get_track(8) {
            if t8.sector_count() > 9 {
                if let Some(s9) = t8.get_sector_by_index(9) {
                    if s9.actual_size() > 128 {
                        let data = s9.data();
                        if find_pattern(data, b"W.R.M Disc").map(|o| o == 0).unwrap_or(false)
                            && find_pattern(data, b"Protection").is_some()
                            && find_pattern(data, b"System (c) 1987").is_some()
                        {
                            return Some(ProtectionResult::confirmed(
                                "W.R.M Disc Protection",
                                "signed T8/S9",
                            ));
                        }
                    }
                }
            }
        }
    }

    if tc > 10 {
        if let Some(t9) = disk.get_track(9) {
            if t9.sector_count() == 1 {
                if let Some(t0s0) = t0.get_sector_by_index(0) {
                    if t0s0.actual_size() == 4096 && t0s0.fdc_status1.0 == 0 {
                        return Some(ProtectionResult::probable(
                            "Frontier",
                            "unsigned (T9 single sector, T0/S0 = 4096)",
                        ));
                    }
                }
            }
        }
    }

    if tc > 1 {
        if let Some(t1) = disk.get_track(1) {
            for s_idx in 0..t1.sector_count() {
                if let Some(sector) = t1.get_sector_by_index(s_idx) {
                    if sector_contains(
                        sector,
                        b"NEW DISK PROTECTION SYSTEM. (C) 1990 BY NEW FRONTIER SOFT.",
                    )
                    .is_some()
                    {
                        return Some(ProtectionResult::confirmed(
                            "Frontier",
                            "signed T1",
                        ));
                    }
                }
            }

            if t1.sector_count() > 4 {
                if let Some(s4) = t1.get_sector_by_index(4) {
                    let mut sig = b"Loader ".to_vec();
                    sig.push(0x7F);
                    sig.extend_from_slice(b"1988 Three Inch Software");
                    if sector_contains(s4, &sig).is_some() {
                        return Some(ProtectionResult::confirmed(
                            "Three Inch Loader type 3-1-4",
                            "signed T1/S4",
                        ));
                    }
                }
            }
        }
    }

    if tc >= 40 {
        let t38 = disk.get_track(38)?;
        let t39 = disk.get_track(39)?;
        if t39.sector_count() == 10 && t38.sector_count() == 9 {
            if let Some(s9) = t39.get_sector_by_index(9) {
                if s9.fdc_status1.0 == 0x20 && s9.fdc_status2.0 == 0x20 {
                    return Some(ProtectionResult::probable(
                        "KBI-10",
                        "weak sector T39/S9",
                    ));
                }
            }
        }
    }

    if tc > 39 {
        if let Some(t39) = disk.get_track(39) {
            if t39.sector_count() == 9 {
                for s_idx in 0..t39.sector_count() {
                    if let Some(sector) = t39.get_sector_by_index(s_idx) {
                        if sector.id.size_code == 2 && sector.actual_size() == 540 {
                            return Some(ProtectionResult::probable(
                                "Infogrames/Logiciel",
                                format!("gap data sector T39/S{}", s_idx),
                            ));
                        }
                    }
                }
            }
        }
    }

    if tc > 40 {
        if let Some(t40) = disk.get_track(40) {
            if t40.sector_count() == 9 {
                for s_idx in 0..t40.sector_count() {
                    if let Some(sector) = t40.get_sector_by_index(s_idx) {
                        if sector.id.sector == 198
                            && sector.fdc_status1.0 == 0x20
                            && sector.fdc_status2.0 == 0x20
                        {
                            return Some(ProtectionResult::probable(
                                "Rainbow Arts",
                                format!("weak sector T40/S{}", s_idx),
                            ));
                        }
                    }
                }
            }
        }
    }

    None
}

// ============================================================================
// Step 4: Mid-disk sweep (§7)
// ============================================================================

fn sweep_mid_disk(disk: &Disk, t0: &Track) -> Option<ProtectionResult> {
    let cpc = is_cpc_disk(t0);
    let limit = disk.track_count().min(42);
    for t in 2..limit {
        let Some(ht) = disk.get_track(t as u8) else { continue };
        if ht.is_empty() || ht.sector_count() == 9 {
            continue;
        }

        if ht.sector_count() == 18 {
            if let Some(hs0) = ht.get_sector_by_index(0) {
                if hs0.actual_size() == 256 || hs0.advertised_size() == 256 {
                    return Some(ProtectionResult::confirmed(
                        "Alkatraz CPC",
                        format!("18-sector T{}", t),
                    ));
                }
            }
        }

        if ht.sector_count() == 16 {
            if is_discsys_track(ht) {
                return Some(ProtectionResult::confirmed(
                    "DiscSYS",
                    format!("16-sector CHRN ramp at T{}", t),
                ));
            }
            if is_players_track(ht) {
                return Some(ProtectionResult::maybe(
                    "Players",
                    format!("16-sector id==size at T{}", t),
                ));
            }
        }

        if ht.sector_count() == 19 {
            if let Some(d) = disk.get_track(t as u8) {
                if d.sector_count() > 1 {
                    if let Some(s1) = d.get_sector_by_index(1) {
                        if sector_contains(s1, b"(c) 1986 for KBI ").is_some() {
                            return Some(ProtectionResult::confirmed(
                                "KBI-19",
                                format!("signed T{}/S1", t),
                            ));
                        }
                    }
                }
                if let Some(s0) = d.get_sector_by_index(0) {
                    if sector_contains(s0, b"ALAIN LAURENT GENERATION 5 1989").is_some() {
                        return Some(ProtectionResult::confirmed(
                            "CAAV",
                            format!("signed T{}/S0", t),
                        ));
                    }
                }
            }
            return Some(ProtectionResult::probable(
                "KBI-19 or CAAV",
                format!("unsigned, 19-sector T{}", t),
            ));
        }

        if ht.sector_count() == 5 {
            if let Some(hs0) = ht.get_sector_by_index(0) {
                if hs0.advertised_size() == 1024 {
                    let name = if cpc {
                        "Speedlock (CPC)"
                    } else {
                        "Speedlock +3 1987/1988"
                    };
                    return Some(ProtectionResult::probable(
                        name,
                        format!("5x1024 at T{}", t),
                    ));
                }
            }
        }

        if ht.sector_count() == 8 {
            if let Some(hs0) = ht.get_sector_by_index(0) {
                if hs0.advertised_size() == 512 && !cpc {
                    return Some(ProtectionResult::maybe(
                        "Alkatraz +3",
                        format!("unsigned (8x512 data at T{})", t),
                    ));
                }
            }
        }

        if ht.sector_count() == 1 {
            if let Some(hs0) = ht.get_sector_by_index(0) {
                if hs0.id.size_code == 6 && hs0.fdc_status1.0 == 0x20 {
                    if cpc {
                        return Some(ProtectionResult::probable(
                            "Hexagon",
                            format!("big-sector at T{} (CPC)", t),
                        ));
                    }
                    return Some(ProtectionResult::probable(
                        "Speedlock 1989/1990",
                        format!("big-sector at T{} (+3)", t),
                    ));
                }
            }
        }
    }

    None
}

// ============================================================================
// Alkatraz detail extraction
// ============================================================================

fn find_alkatraz_protection_track(disk: &Disk) -> Option<(usize, &Track)> {
    let limit = disk.track_count().min(42);
    for t in 0..limit {
        let track = disk.get_track(t as u8)?;
        if track.sector_count() == 18 {
            if let Some(s0) = track.get_sector_by_index(0) {
                if s0.actual_size() == 256 || s0.advertised_size() == 256 {
                    return Some((t, track));
                }
            }
        }
    }
    None
}

fn analyze_alkatraz(disk: &Disk) -> Vec<String> {
    let mut lines = Vec::new();

    if let Some((prot_track_idx, prot_track)) = find_alkatraz_protection_track(disk) {
        let boundary = prot_track_idx + 1;
        lines.push(format!("Protection track: T{} (BOUNDRY={})", prot_track_idx, boundary));

        let mut btrnum: Option<u8> = None;
        let mut sector_rs: Vec<u8> = Vec::new();
        let mut head_hs: Vec<u8> = Vec::new();
        let mut fill_bytes: Vec<u8> = Vec::new();
        let mut uniform_c = true;

        for i in 0..prot_track.sector_count() {
            if let Some(sector) = prot_track.get_sector_by_index(i) {
                let c = sector.id.track;
                match btrnum {
                    None => btrnum = Some(c),
                    Some(prev) if prev != c => uniform_c = false,
                    _ => {}
                }
                sector_rs.push(sector.id.sector);
                head_hs.push(sector.id.side);
                if let Some(&first_byte) = sector.data().first() {
                    fill_bytes.push(first_byte);
                }
            }
        }

        if let Some(btr) = btrnum {
            lines.push(format!("BTRNUM: 0x{:02X} ({})", btr, btr));
        }

        if !uniform_c {
            lines.push("C values vary across sectors (non-standard)".to_string());
        }

        lines.push(format!("GAP3: {}", prot_track.gap3_length));

        lines.push(format!("{} sectors x {}B", prot_track.sector_count(),
            prot_track.uniform_sector_size().unwrap_or(256)));

        let fmt_ids = |vals: &[u8]| -> String {
            if vals.len() <= 8 {
                vals.iter().map(|v| format!("0x{:02X}", v)).collect::<Vec<_>>().join(", ")
            } else {
                let first: Vec<String> = vals[..4].iter().map(|v| format!("0x{:02X}", v)).collect();
                let last: Vec<String> = vals[vals.len()-4..].iter().map(|v| format!("0x{:02X}", v)).collect();
                format!("{}, ..., {}", first.join(", "), last.join(", "))
            }
        };

        lines.push(format!("Sector IDs (R): [{}]", fmt_ids(&sector_rs)));
        lines.push(format!("Head values (H): [{}]", fmt_ids(&head_hs)));
        lines.push(format!("Fill bytes: [{}]", fmt_ids(&fill_bytes)));
    }

    if let Some(t0) = disk.get_track(0) {
        if let Some(s0) = t0.get_sector_by_index(0) {
            let data = s0.data();
            if data.len() >= 512 {
                let u16_le = |off: usize| -> u16 {
                    u16::from_le_bytes([data[off], data[off + 1]])
                };

                let scrncol = u16_le(504);
                let bordercl = u16_le(506);
                let llen1 = u16_le(508);
                let lsta = u16_le(510);

                if lsta > 0 || llen1 > 0 {
                    lines.push(format!("Loader: ${:04X} ({} bytes)", lsta, llen1));
                }
                if scrncol > 0 || bordercl > 0 {
                    lines.push(format!("Screen: attr=0x{:02X} border=0x{:02X}", scrncol as u8, bordercl as u8));
                }
            }
        }
    }

    lines
}

fn attach_alkatraz_details(result: &mut ProtectionResult, disk: &Disk) {
    let details = analyze_alkatraz(disk);
    if !details.is_empty() {
        result.details = details;
    }
}

// ============================================================================
// Stripped-FDC fallbacks (§9)
// ============================================================================

fn stripped_fdc_fallbacks(disk: &Disk, t0: &Track) -> Option<ProtectionResult> {
    if has_fdc_errors(disk) {
        return None;
    }

    if t0.sector_count() >= 8 {
        if let Some(t1) = disk.get_track(1) {
            if t1.sector_count() == 5 {
                if let Some(t1s0) = t1.get_sector_by_index(0) {
                    if t1s0.advertised_size() == 1024 {
                        let cpc = is_cpc_disk(t0);
                        let name = if cpc {
                            "Speedlock (CPC)"
                        } else {
                            "Speedlock +3 1987/1988"
                        };
                        return Some(ProtectionResult::probable(
                            name,
                            "layout matches, stripped FDC flags",
                        ));
                    }
                }
            }
        }
    }

    if t0.sector_count() >= 8 && disk.track_count() > 40 {
        if let Some(t1) = disk.get_track(1) {
            if t1.sector_count() == 1 {
                if let Some(t1s0) = t1.get_sector_by_index(0) {
                    if t1s0.id.size_code == 6 {
                        let cpc = is_cpc_disk(t0);
                        if cpc {
                            return Some(ProtectionResult::probable(
                                "Hexagon",
                                "layout matches, stripped FDC flags (CPC)",
                            ));
                        }
                        return Some(ProtectionResult::probable(
                            "Speedlock 1989/1990",
                            "layout matches, stripped FDC flags",
                        ));
                    }
                }
            }
        }
    }

    None
}

// ============================================================================
// Main detection function
// ============================================================================

/// Detect copy protection on a disk side
///
/// Returns `Some(ProtectionResult)` if a protection scheme is detected,
/// or `None` if the disk appears to be unprotected.
///
/// Uses a fingerprinting flow: T0 signatures → T0 geometry classification →
/// resolvers → T1 checks → high-track probes → mid-disk sweep.
pub fn detect(disk: &Disk) -> Option<ProtectionResult> {
    let result = detect_inner(disk)?;
    if result.name.contains("Alkatraz") {
        let mut result = result;
        attach_alkatraz_details(&mut result, disk);
        Some(result)
    } else {
        Some(result)
    }
}

fn detect_inner(disk: &Disk) -> Option<ProtectionResult> {
    if disk.track_count() < 2 {
        return None;
    }

    let t0 = disk.get_track(0)?;
    if t0.sector_count() < 1 {
        return None;
    }
    let t0s0 = t0.get_sector_by_index(0)?;
    if t0s0.actual_size() < 128 {
        return None;
    }

    if is_uniform(disk) && !has_fdc_errors(disk) {
        let t0_sc = t0.sector_count();
        let t0_sz = t0.uniform_sector_size();
        if t0_sc == 5 && t0_sz == Some(1024) {
            // Could be unsigned Speedlock data side
        } else if t0_sc == 8 && t0_sz == Some(512) {
            // Could be unsigned Alkatraz +3
        } else {
            return None;
        }
    }

    // ── STEP 1a: Scan T0 signatures ──────────────────────────────────────
    if let Some(hit) = scan_t0_signatures(t0) {
        return Some(hit);
    }

    // ── STEP 1b: Classify T0 geometry → resolver ─────────────────────────
    match classify_t0(t0) {
        T0Class::SpeedlockPlus3 => {
            if let Some(hit) = resolve_speedlock_plus3(disk, t0) {
                return Some(hit);
            }
        }
        T0Class::BigSector => {
            if let Some(hit) = resolve_big_sector(disk, t0) {
                return Some(hit);
            }
        }
        T0Class::TenSector => {
            if let Some(hit) = resolve_hexagon(disk, t0) {
                return Some(hit);
            }
        }
        T0Class::TenSectorDDAM => {
            if let Some(hit) = resolve_speedlock_1989_cpc(disk, t0) {
                return Some(hit);
            }
        }
        T0Class::EighteenSector => {
            return resolve_18sector(disk, t0);
        }
        T0Class::SixteenSector => {
            if let Some(hit) = resolve_16sector(disk, t0) {
                return Some(hit);
            }
        }
        T0Class::NineteenSector => {
            return resolve_19sector(t0);
        }
        T0Class::EightSector => {
            if let Some(hit) = resolve_8sector(disk, t0) {
                return Some(hit);
            }
        }
        T0Class::FiveSector => {
            if let Some(hit) = resolve_5sector(disk, t0) {
                return Some(hit);
            }
        }
        T0Class::Speedlock9x512 => {
            if let Some(hit) = resolve_speedlock_9x512(disk, t0) {
                return Some(hit);
            }
        }
        T0Class::Standard => {}
    }

    // ── STEP 2: Read Track 1 ─────────────────────────────────────────────
    if let Some(t1) = disk.get_track(1) {
        if t1.is_empty() {
            return resolve_empty_t1_family(disk, t0);
        }

        if t1.sector_count() == 5 {
            if let Some(t1s0) = t1.get_sector_by_index(0) {
                if t1s0.advertised_size() == 1024 {
                    return resolve_speedlock_5x1024(t0, t1);
                }
            }
        }

        if t1.sector_count() == 1 {
            if let Some(t1s0) = t1.get_sector_by_index(0) {
                if t1s0.fdc_status1.0 == 0x20 {
                    return resolve_speedlock_1989(t0, t1);
                }
            }
        }

        if t1.sector_count() == 16 {
            if is_discsys_track(t1) {
                for s_idx in 0..t0.sector_count() {
                    if let Some(sector) = t0.get_sector_by_index(s_idx) {
                        if sector_contains(sector, b"MEAN PROTECTION SYSTEM").is_some() {
                            return Some(ProtectionResult::confirmed(
                                "Mean Protection System",
                                format!("signed T0/S{} + DiscSYS T1", s_idx),
                            ));
                        }
                    }
                }
                return Some(ProtectionResult::confirmed(
                    "DiscSYS",
                    "16-sector CHRN ramp on T1",
                ));
            }
            if is_players_track(t1) {
                return Some(ProtectionResult::maybe(
                    "Players",
                    "16-sector id==size pattern on T1",
                ));
            }
        }
    }

    // ── STEP 3: High-track probes ────────────────────────────────────────
    if let Some(hit) = scan_high_tracks(disk, t0) {
        return Some(hit);
    }

    // ── STEP 4: Mid-disk sweep ───────────────────────────────────────────
    if let Some(hit) = sweep_mid_disk(disk, t0) {
        return Some(hit);
    }

    // ── Stripped-FDC fallbacks ───────────────────────────────────────────
    if let Some(hit) = stripped_fdc_fallbacks(disk, t0) {
        return Some(hit);
    }

    // ── Unknown protection fallback ──────────────────────────────────────
    if !is_uniform(disk) {
        let used_tracks = disk.tracks().iter().filter(|t| !t.is_empty()).count();
        let max_valid = used_tracks.min(40);
        let error_tracks: Vec<usize> = (0..max_valid)
            .filter(|&t_idx| {
                disk.get_track(t_idx as u8)
                    .map(|t| t.sectors().iter().any(|s| s.has_error()))
                    .unwrap_or(false)
            })
            .collect();

        if !error_tracks.is_empty() {
            let is_lone_high_error = error_tracks.len() <= 2
                && error_tracks.iter().all(|&t| t >= 35)
                && disk.tracks().iter().enumerate().all(|(i, t)| {
                    t.is_empty()
                        || error_tracks.contains(&i)
                        || (t.sector_count() == 9
                            && t.uniform_sector_size() == Some(512)
                            && !t.sectors().iter().any(|s| s.is_deleted()))
                });

            if !is_lone_high_error {
                return Some(ProtectionResult::new(
                    "Unknown copy protection",
                    "non-uniform disk with FDC errors",
                ));
            }
        }
    }

    None
}

impl ProtectionResult {
    /// Create a new protection result
    pub fn new(name: impl Into<String>, reason: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            reason: reason.into(),
            details: Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fdc::{FdcStatus1, FdcStatus2};
    use crate::image::{Sector, SectorId};

    #[test]
    fn test_find_pattern() {
        let data = b"Hello SPEEDLOCK PROTECTION SYSTEM world";
        assert!(find_pattern(data, b"SPEEDLOCK").is_some());
        assert_eq!(find_pattern(data, b"SPEEDLOCK"), Some(6));
        assert!(find_pattern(data, b"NOTFOUND").is_none());
    }

    #[test]
    fn test_uniform_disk_no_protection() {
        let mut disk = Disk::new(0);
        for t in 0..40 {
            let mut track = Track::new(t, 0);
            for s in 0..9 {
                let id = SectorId::new(t, 0, 0xC1 + s, 2);
                track.add_sector(Sector::new(id));
            }
            disk.add_track(track);
        }
        assert!(is_uniform(&disk));
        assert!(!has_fdc_errors(&disk));
        assert!(detect(&disk).is_none());
    }

    #[test]
    fn test_speedlock_plus3_1987_9x512_variant() {
        let mut disk = Disk::new(0);

        let mut track0 = Track::new(0, 0);
        track0.add_sector(Sector::new(SectorId::new(0, 0, 1, 2)));
        for r in 130u8..=136 {
            let mut s = Sector::with_data(SectorId::new(0, 0, r, 2), vec![0xA7; 512]);
            s.fdc_status1 = FdcStatus1::new(0);
            s.fdc_status2 = FdcStatus2::new(0);
            track0.add_sector(s);
        }
        let mut weak = Sector::with_data(SectorId::new(0, 0, 121, 0), vec![0xA7; 128]);
        weak.fdc_status1 = FdcStatus1::new(FdcStatus1::DE);
        weak.fdc_status2 = FdcStatus2::new(FdcStatus2::DD);
        track0.add_sector(weak);
        disk.add_track(track0);

        for t in 1u8..=3 {
            let mut track = Track::new(t, 0);
            for r in 1u8..=9 {
                track.add_sector(Sector::new(SectorId::new(t, 0, r, 2)));
            }
            disk.add_track(track);
        }

        for t in 4u8..42 {
            let mut track = Track::new(t, 0);
            for r in 1u8..=9 {
                let mut s = Sector::new(SectorId::new(t, 0, r, 2));
                s.fdc_status2 = FdcStatus2::new(FdcStatus2::CM);
                track.add_sector(s);
            }
            disk.add_track(track);
        }

        let result = detect(&disk).expect("variant should be detected");
        assert_eq!(result.name, "Speedlock +3 1987");
    }

    #[test]
    fn test_protection_result_display() {
        let result = ProtectionResult::new("Speedlock 1987", "signed T0/S0 +42");
        assert_eq!(result.to_string(), "Speedlock 1987 (signed T0/S0 +42)");
    }

    #[test]
    fn test_unsigned_alkatraz_8sector() {
        let mut disk = Disk::new(0);

        let mut t0 = Track::new(0, 0);
        for r in 1u8..=8 {
            t0.add_sector(Sector::new(SectorId::new(0, 0, r, 2)));
        }
        disk.add_track(t0);

        for t in 1u8..5 {
            let mut track = Track::new(t, 0);
            for r in 1u8..=8 {
                track.add_sector(Sector::new(SectorId::new(t, 0, r, 2)));
            }
            disk.add_track(track);
        }

        let mut prot_track = Track::new(5, 0);
        for r in 1u8..=18 {
            prot_track.add_sector(Sector::with_data(
                SectorId::new(5, 0, r, 1),
                vec![0xE5; 256],
            ));
        }
        disk.add_track(prot_track);

        for t in 6u8..10 {
            let mut track = Track::new(t, 0);
            for r in 1u8..=8 {
                track.add_sector(Sector::new(SectorId::new(t, 0, r, 2)));
            }
            disk.add_track(track);
        }

        let result = detect(&disk).expect("unsigned Alkatraz should be detected");
        assert!(
            result.name.contains("Alkatraz"),
            "expected Alkatraz, got: {}",
            result.name
        );
        assert!(
            result.reason.contains("8-sector"),
            "expected 8-sector mention, got: {}",
            result.reason
        );
    }

    #[test]
    fn test_unsigned_speedlock_5sector() {
        let mut disk = Disk::new(0);

        let mut t0 = Track::new(0, 0);
        for r in 1u8..=5 {
            t0.add_sector(Sector::new(SectorId::new(0, 0, r, 3)));
        }
        disk.add_track(t0);

        let mut t1 = Track::new(1, 0);
        for r in 1u8..=5 {
            t1.add_sector(Sector::new(SectorId::new(1, 0, r, 3)));
        }
        disk.add_track(t1);

        for t in 2u8..10 {
            let mut track = Track::new(t, 0);
            for r in 1u8..=5 {
                track.add_sector(Sector::new(SectorId::new(t, 0, r, 3)));
            }
            disk.add_track(track);
        }

        let result = detect(&disk).expect("unsigned Speedlock should be detected");
        assert!(
            result.name.contains("Speedlock"),
            "expected Speedlock, got: {}",
            result.name
        );
        assert!(
            result.reason.contains("5x1024"),
            "expected 5x1024 mention, got: {}",
            result.reason
        );
    }

    #[test]
    fn test_mid_disk_alkatraz_cpc() {
        let mut disk = Disk::new(0);

        let mut t0 = Track::new(0, 0);
        for r in 0xC1u8..=0xC9 {
            t0.add_sector(Sector::new(SectorId::new(0, 0, r, 2)));
        }
        disk.add_track(t0);

        for t in 1u8..5 {
            let mut track = Track::new(t, 0);
            for r in 0xC1u8..=0xC9 {
                track.add_sector(Sector::new(SectorId::new(t, 0, r, 2)));
            }
            disk.add_track(track);
        }

        let mut prot_track = Track::new(5, 0);
        for r in 0xC1u8..=0xC1 + 17 {
            prot_track.add_sector(Sector::with_data(
                SectorId::new(5, 0, r, 1),
                vec![0xE5; 256],
            ));
        }
        disk.add_track(prot_track);

        for t in 6u8..10 {
            let mut track = Track::new(t, 0);
            for r in 0xC1u8..=0xC9 {
                track.add_sector(Sector::new(SectorId::new(t, 0, r, 2)));
            }
            disk.add_track(track);
        }

        let result = detect(&disk).expect("mid-disk Alkatraz CPC should be detected");
        assert_eq!(result.name, "Alkatraz CPC");
    }

    #[test]
    fn test_t0_signature_before_geometry() {
        let mut disk = Disk::new(0);

        let mut t0 = Track::new(0, 0);
        let mut data = vec![0x00u8; 512];
        let sig = b" THE ALKATRAZ PROTECTION SYSTEM   (C) 1987  Appleby Associates";
        data[..sig.len()].copy_from_slice(sig);
        let mut s0 = Sector::with_data(SectorId::new(0, 0, 1, 2), data);
        s0.fdc_status2 = FdcStatus2::new(FdcStatus2::CM);
        t0.add_sector(s0);
        for r in 2u8..=8 {
            t0.add_sector(Sector::new(SectorId::new(0, 0, r, 2)));
        }
        disk.add_track(t0);

        let mut t1 = Track::new(1, 0);
        for r in 1u8..=5 {
            t1.add_sector(Sector::new(SectorId::new(1, 0, r, 3)));
        }
        disk.add_track(t1);

        for t in 2u8..10 {
            let mut track = Track::new(t, 0);
            for r in 1u8..=9 {
                track.add_sector(Sector::new(SectorId::new(t, 0, r, 2)));
            }
            disk.add_track(track);
        }

        let result = detect(&disk).expect("should detect via signature");
        assert_eq!(result.name, "Alkatraz +3");
        assert!(result.reason.contains("signed T0/S0"));
    }

    fn make_hexagon_track(t: u8) -> Track {
        let mut track = Track::new(t, 0);
        let mut s = Sector::with_data(SectorId::new(t, 0, 1, 6), vec![0xE5; 6144]);
        s.fdc_status1 = FdcStatus1::new(FdcStatus1::DE);
        s.fdc_status2 = FdcStatus2::new(0x60);
        track.add_sector(s);
        track
    }

    #[test]
    fn test_hexagon_plus3_clean_t0() {
        let mut disk = Disk::new(0);

        let mut t0 = Track::new(0, 0);
        for r in 1u8..=10 {
            t0.add_sector(Sector::new(SectorId::new(0, 0, r, 2)));
        }
        disk.add_track(t0);

        for t in 1u8..10 {
            disk.add_track(make_hexagon_track(t));
        }

        let result = detect(&disk).expect("Hexagon +3 should be detected");
        assert!(
            result.name.contains("Hexagon"),
            "expected Hexagon, got: {}",
            result.name
        );
        assert!(
            !result.name.contains("Speedlock"),
            "should not be Speedlock, got: {}",
            result.name
        );
    }

    #[test]
    fn test_speedlock_1989_cpc_ten_sector_ddam_t0() {
        let mut disk = Disk::new(0);

        let mut t0 = Track::new(0, 0);
        let ddam_sectors: &[u8] = &[194, 195, 196, 197, 198, 199, 201];
        for r in 193u8..=202 {
            let mut s = Sector::new(SectorId::new(0, 0, r, 2));
            if ddam_sectors.contains(&r) {
                s.fdc_status2 = FdcStatus2::new(FdcStatus2::CM);
            }
            t0.add_sector(s);
        }
        disk.add_track(t0);

        let mut t1 = Track::new(1, 0);
        let mut s = Sector::with_data(SectorId::new(1, 0, 193, 6), vec![0xE5; 6144]);
        s.fdc_status1 = FdcStatus1::new(FdcStatus1::DE);
        s.fdc_status2 = FdcStatus2::new(0x60);
        t1.add_sector(s);
        disk.add_track(t1);

        for t in 2u8..10 {
            let mut track = Track::new(t, 0);
            for r in 0xC1u8..=0xCA {
                track.add_sector(Sector::new(SectorId::new(t, 0, r, 2)));
            }
            disk.add_track(track);
        }

        let result = detect(&disk).expect("Speedlock 1989 CPC should be detected");
        assert!(
            result.name.contains("Speedlock 1989"),
            "expected Speedlock 1989, got: {}",
            result.name
        );
        assert!(
            !result.name.contains("Hexagon"),
            "should not be Hexagon, got: {}",
            result.name
        );
    }

    #[test]
    fn test_hexagon_cpc_clean_ten_sector_t0() {
        let mut disk = Disk::new(0);

        let mut t0 = Track::new(0, 0);
        for r in 193u8..=202 {
            t0.add_sector(Sector::new(SectorId::new(0, 0, r, 2)));
        }
        disk.add_track(t0);

        for t in 1u8..10 {
            disk.add_track(make_hexagon_track(t));
        }

        let result = detect(&disk).expect("Hexagon CPC should be detected");
        assert!(
            result.name.contains("Hexagon"),
            "expected Hexagon, got: {}",
            result.name
        );
        assert!(
            !result.name.contains("Speedlock"),
            "should not be Speedlock, got: {}",
            result.name
        );
    }

    #[test]
    fn test_classify_t0_ten_sector_clean_vs_ddam() {
        let mut clean_t0 = Track::new(0, 0);
        for r in 1u8..=10 {
            clean_t0.add_sector(Sector::new(SectorId::new(0, 0, r, 2)));
        }
        assert_eq!(classify_t0(&clean_t0), T0Class::TenSector);

        let mut ddam_t0 = Track::new(0, 0);
        for r in 193u8..=202 {
            let mut s = Sector::new(SectorId::new(0, 0, r, 2));
            if r != 193 && r != 200 && r != 202 {
                s.fdc_status2 = FdcStatus2::new(FdcStatus2::CM);
            }
            ddam_t0.add_sector(s);
        }
        assert_eq!(classify_t0(&ddam_t0), T0Class::TenSectorDDAM);
    }

    #[test]
    fn test_alkatraz_details_signed() {
        let mut disk = Disk::new(0);

        let mut t0 = Track::new(0, 0);
        let mut boot_data = vec![0x00u8; 512];
        let sig = b" THE ALKATRAZ PROTECTION SYSTEM   (C) 1987  Appleby Associates";
        boot_data[..sig.len()].copy_from_slice(sig);
        boot_data[504] = 0x38;
        boot_data[505] = 0x00;
        boot_data[506] = 0x02;
        boot_data[507] = 0x00;
        boot_data[508] = 0x00;
        boot_data[509] = 0x1C;
        boot_data[510] = 0x00;
        boot_data[511] = 0x80;
        t0.add_sector(Sector::with_data(SectorId::new(0, 0, 1, 2), boot_data));
        for r in 2u8..=9 {
            t0.add_sector(Sector::new(SectorId::new(0, 0, r, 2)));
        }
        disk.add_track(t0);

        for t in 1u8..5 {
            let mut track = Track::new(t, 0);
            for r in 1u8..=9 {
                track.add_sector(Sector::new(SectorId::new(t, 0, r, 2)));
            }
            disk.add_track(track);
        }

        let mut prot_track = Track::new(5, 0);
        prot_track.gap3_length = 12;
        let btrnum: u8 = 0xE9;
        for i in 0u8..18 {
            let fill = 0xAA + i;
            let r_val = 0xC1u8.wrapping_add(i.wrapping_mul(7));
            let h_val = (i as u8).wrapping_mul(32) & 0xF8;
            let s = Sector::with_data(
                SectorId::new(btrnum, h_val, r_val, 1),
                vec![fill; 256],
            );
            prot_track.add_sector(s);
        }
        disk.add_track(prot_track);

        for t in 6u8..10 {
            let mut track = Track::new(t, 0);
            for r in 1u8..=9 {
                track.add_sector(Sector::new(SectorId::new(t, 0, r, 2)));
            }
            disk.add_track(track);
        }

        let result = detect(&disk).expect("signed Alkatraz should be detected");
        assert_eq!(result.name, "Alkatraz +3");
        assert!(!result.details.is_empty(), "should have details");

        let joined = result.details.join("\n");
        assert!(joined.contains("Protection track: T5"), "should mention track 5, got: {}", joined);
        assert!(joined.contains("BOUNDRY=6"), "should mention BOUNDRY, got: {}", joined);
        assert!(joined.contains("BTRNUM: 0xE9"), "should mention BTRNUM, got: {}", joined);
        assert!(joined.contains("GAP3: 12"), "should mention GAP3, got: {}", joined);
        assert!(joined.contains("18 sectors x 256B"), "should mention sector layout, got: {}", joined);
        assert!(joined.contains("Loader: $8000"), "should mention loader address, got: {}", joined);
    }

    #[test]
    fn test_alkatraz_details_no_protection_track() {
        let mut disk = Disk::new(0);

        let mut t0 = Track::new(0, 0);
        let mut boot_data = vec![0x00u8; 512];
        let sig = b" THE ALKATRAZ PROTECTION SYSTEM   (C) 1987  Appleby Associates";
        boot_data[..sig.len()].copy_from_slice(sig);
        t0.add_sector(Sector::with_data(SectorId::new(0, 0, 1, 2), boot_data));
        for r in 2u8..=8 {
            t0.add_sector(Sector::new(SectorId::new(0, 0, r, 2)));
        }
        disk.add_track(t0);

        for t in 1u8..10 {
            let mut track = Track::new(t, 0);
            for r in 1u8..=9 {
                track.add_sector(Sector::new(SectorId::new(t, 0, r, 2)));
            }
            disk.add_track(track);
        }

        let result = detect(&disk).expect("should detect Alkatraz");
        assert_eq!(result.name, "Alkatraz +3");
        assert!(result.details.is_empty() || !result.details.iter().any(|d| d.contains("Protection track")),
            "no protection track details expected when no 18-sector track present");
    }

    #[test]
    fn test_non_alkatraz_has_no_details() {
        let mut disk = Disk::new(0);

        let mut t0 = Track::new(0, 0);
        for r in 1u8..=9 {
            let mut s = Sector::new(SectorId::new(0, 0, r, 2));
            if r >= 7 {
                s.fdc_status2 = FdcStatus2::new(FdcStatus2::CM);
            }
            t0.add_sector(s);
        }
        disk.add_track(t0);

        let mut t1 = Track::new(1, 0);
        for r in 1u8..=5 {
            t1.add_sector(Sector::new(SectorId::new(1, 0, r, 3)));
        }
        disk.add_track(t1);

        for t in 2u8..10 {
            let mut track = Track::new(t, 0);
            for r in 1u8..=9 {
                track.add_sector(Sector::new(SectorId::new(t, 0, r, 2)));
            }
            disk.add_track(track);
        }

        let result = detect(&disk).expect("Speedlock should be detected");
        assert!(result.details.is_empty(), "non-Alkatraz should have no details");
    }
}
