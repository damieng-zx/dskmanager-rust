use crate::error::{DskError, Result};
use crate::fdc::{FdcStatus1, FdcStatus2};
use crate::format::specification::DiskSpecification;
use crate::format::{DiskImageFormat, FormatSpec, SideMode};
use crate::image::{DataRate, Disk, DiskImage, RecordingMode, Sector, SectorId, Track};
use serde::{Deserialize, Serialize};
use std::fs::File;
use std::io::{Read, Write};
use std::path::Path;

/// Check if a file is a JSON disk image based on extension
pub fn is_json_file<P: AsRef<Path>>(path: P) -> bool {
    path.as_ref()
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.eq_ignore_ascii_case("json"))
        .unwrap_or(false)
}

/// Read a JSON disk image from disk
pub fn read_json<P: AsRef<Path>>(path: P) -> Result<DiskImage> {
    let filename = path
        .as_ref()
        .file_name()
        .and_then(|n| n.to_str())
        .map(|s| s.to_string());

    let mut file = File::open(&path)?;
    let mut contents = String::new();
    file.read_to_string(&mut contents)?;

    let json_image: JsonDiskImage =
        serde_json::from_str(&contents).map_err(|e| DskError::invalid_format(e.to_string()))?;

    json_image.into_disk_image(filename)
}

/// Write a JSON disk image to disk
pub fn write_json<P: AsRef<Path>>(image: &DiskImage, path: P) -> Result<()> {
    let json_image = JsonDiskImage::from_disk_image(image);
    let contents = serde_json::to_string_pretty(&json_image)
        .map_err(|e| DskError::invalid_format(e.to_string()))?;

    let mut file = File::create(path)?;
    file.write_all(contents.as_bytes())?;
    Ok(())
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct JsonDiskImage {
    info: JsonInfo,
    sides: Vec<JsonSide>,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct JsonInfo {
    format: String,
    source_format: String,
    filename: Option<String>,
    warnings: Vec<String>,
    geometry: JsonGeometry,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct JsonGeometry {
    num_sides: u8,
    num_tracks: u8,
    sectors_per_track: u8,
    sector_size: u16,
    first_sector_id: String,
    gap3_length: String,
    filler_byte: String,
    interleave: u8,
    side_mode: String,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct JsonSide {
    side_number: u8,
    tracks: Vec<JsonTrack>,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct JsonTrack {
    track_number: u8,
    side_number: u8,
    gap3_length: String,
    filler_byte: String,
    data_rate: String,
    recording_mode: String,
    sectors: Vec<JsonSector>,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct JsonSector {
    id: JsonSectorId,
    fdc_status1: String,
    fdc_status2: String,
    data_length: u16,
    data: String,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct JsonSectorId {
    track: u8,
    side: u8,
    sector: String,
    size_code: u8,
}

fn encode_hex(byte: u8) -> String {
    format!("0x{:02X}", byte)
}

fn decode_hex(hex: &str) -> std::result::Result<u8, String> {
    let trimmed = hex.trim();
    let digits = trimmed.strip_prefix("0x").or_else(|| trimmed.strip_prefix("0X"));
    let s = digits.unwrap_or(trimmed);
    u8::from_str_radix(s, 16).map_err(|e| format!("Invalid hex '{}': {}", hex, e))
}

fn encode_data_hex(data: &[u8]) -> String {
    data.iter().map(|b| format!("{:02X}", b)).collect()
}

fn decode_data_hex(hex: &str) -> std::result::Result<Vec<u8>, String> {
    let trimmed = hex.trim();
    if trimmed.is_empty() {
        return Ok(Vec::new());
    }
    if trimmed.len() % 2 != 0 {
        return Err(format!("Hex data has odd length: {}", trimmed.len()));
    }
    let mut data = Vec::with_capacity(trimmed.len() / 2);
    for chunk in trimmed.as_bytes().chunks(2) {
        let byte_str = std::str::from_utf8(chunk)
            .map_err(|e| format!("Invalid UTF-8 in hex data: {}", e))?;
        let byte = u8::from_str_radix(byte_str, 16)
            .map_err(|e| format!("Invalid hex byte '{}': {}", byte_str, e))?;
        data.push(byte);
    }
    Ok(data)
}

fn format_name(fmt: DiskImageFormat) -> &'static str {
    match fmt {
        DiskImageFormat::StandardDSK => "StandardDSK",
        DiskImageFormat::ExtendedDSK => "ExtendedDSK",
        DiskImageFormat::RawMgt => "RawMgt",
        DiskImageFormat::RawTrd => "RawTrd",
        DiskImageFormat::RawTd0 => "RawTd0",
    }
}

fn parse_format_name(name: &str) -> std::result::Result<DiskImageFormat, String> {
    match name {
        "StandardDSK" => Ok(DiskImageFormat::StandardDSK),
        "ExtendedDSK" => Ok(DiskImageFormat::ExtendedDSK),
        "RawMgt" => Ok(DiskImageFormat::RawMgt),
        "RawTrd" => Ok(DiskImageFormat::RawTrd),
        "RawTd0" => Ok(DiskImageFormat::RawTd0),
        _ => Err(format!("Unknown format: '{}'", name)),
    }
}

fn data_rate_name(rate: DataRate) -> &'static str {
    match rate {
        DataRate::Unknown => "Unknown",
        DataRate::SingleDouble => "SingleDouble",
        DataRate::High => "High",
        DataRate::Extended => "Extended",
    }
}

fn parse_data_rate(name: &str) -> std::result::Result<DataRate, String> {
    match name {
        "Unknown" => Ok(DataRate::Unknown),
        "SingleDouble" => Ok(DataRate::SingleDouble),
        "High" => Ok(DataRate::High),
        "Extended" => Ok(DataRate::Extended),
        _ => Err(format!("Unknown data rate: '{}'", name)),
    }
}

fn recording_mode_name(mode: RecordingMode) -> &'static str {
    match mode {
        RecordingMode::Unknown => "Unknown",
        RecordingMode::FM => "FM",
        RecordingMode::MFM => "MFM",
    }
}

fn parse_recording_mode(name: &str) -> std::result::Result<RecordingMode, String> {
    match name {
        "Unknown" => Ok(RecordingMode::Unknown),
        "FM" => Ok(RecordingMode::FM),
        "MFM" => Ok(RecordingMode::MFM),
        _ => Err(format!("Unknown recording mode: '{}'", name)),
    }
}

fn side_mode_name(mode: SideMode) -> &'static str {
    match mode {
        SideMode::SingleSide => "SingleSide",
        SideMode::Alternate => "Alternate",
        SideMode::Successive => "Successive",
    }
}

fn parse_side_mode(name: &str) -> std::result::Result<SideMode, String> {
    match name {
        "SingleSide" => Ok(SideMode::SingleSide),
        "Alternate" => Ok(SideMode::Alternate),
        "Successive" => Ok(SideMode::Successive),
        _ => Err(format!("Unknown side mode: '{}'", name)),
    }
}

impl JsonDiskImage {
    fn from_disk_image(image: &DiskImage) -> Self {
        let spec = DiskSpecification::identify(image);
        let format_spec = image.spec();

        let sides: Vec<JsonSide> = image
            .disks()
            .iter()
            .map(|disk| JsonSide {
                side_number: disk.side_number,
                tracks: disk
                    .tracks()
                    .iter()
                    .map(|track| JsonTrack {
                        track_number: track.track_number,
                        side_number: track.side_number,
                        gap3_length: encode_hex(track.gap3_length),
                        filler_byte: encode_hex(track.filler_byte),
                        data_rate: data_rate_name(track.data_rate).to_string(),
                        recording_mode: recording_mode_name(track.recording_mode).to_string(),
                        sectors: track
                            .sectors()
                            .iter()
                            .map(|sector| JsonSector {
                                id: JsonSectorId {
                                    track: sector.id.track,
                                    side: sector.id.side,
                                    sector: encode_hex(sector.id.sector),
                                    size_code: sector.id.size_code,
                                },
                                fdc_status1: encode_hex(sector.fdc_status1.0),
                                fdc_status2: encode_hex(sector.fdc_status2.0),
                                data_length: sector.data_length,
                                data: encode_data_hex(sector.data()),
                            })
                            .collect(),
                    })
                    .collect(),
            })
            .collect();

        let info = JsonInfo {
            format: format_name(image.format()).to_string(),
            source_format: spec.format,
            filename: image.filename().map(|s| s.to_string()),
            warnings: image.warnings().to_vec(),
            geometry: JsonGeometry {
                num_sides: format_spec.num_sides,
                num_tracks: format_spec.num_tracks,
                sectors_per_track: format_spec.sectors_per_track,
                sector_size: format_spec.sector_size,
                first_sector_id: encode_hex(format_spec.first_sector_id),
                gap3_length: encode_hex(format_spec.gap3_length),
                filler_byte: encode_hex(format_spec.filler_byte),
                interleave: format_spec.interleave,
                side_mode: side_mode_name(format_spec.side_mode).to_string(),
            },
        };

        JsonDiskImage { info, sides }
    }

    fn into_disk_image(self, filename: Option<String>) -> Result<DiskImage> {
        let format = parse_format_name(&self.info.format)
            .map_err(DskError::invalid_format)?;

        let geometry = &self.info.geometry;
        let first_sector_id = decode_hex(&geometry.first_sector_id)
            .map_err(DskError::invalid_format)?;
        let gap3_length = decode_hex(&geometry.gap3_length)
            .map_err(DskError::invalid_format)?;
        let filler_byte = decode_hex(&geometry.filler_byte)
            .map_err(DskError::invalid_format)?;
        let side_mode = parse_side_mode(&geometry.side_mode)
            .map_err(DskError::invalid_format)?;

        let spec = FormatSpec {
            num_sides: geometry.num_sides,
            num_tracks: geometry.num_tracks,
            sectors_per_track: geometry.sectors_per_track,
            sector_size: geometry.sector_size,
            first_sector_id,
            gap3_length,
            filler_byte,
            interleave: geometry.interleave,
            side_mode,
        };

        let mut disks = Vec::with_capacity(self.sides.len());
        for json_side in self.sides {
            let mut disk = Disk::new(json_side.side_number);
            for json_track in json_side.tracks {
                let track_gap3 = decode_hex(&json_track.gap3_length)
                    .map_err(DskError::invalid_format)?;
                let track_filler = decode_hex(&json_track.filler_byte)
                    .map_err(DskError::invalid_format)?;
                let data_rate = parse_data_rate(&json_track.data_rate)
                    .map_err(DskError::invalid_format)?;
                let recording_mode = parse_recording_mode(&json_track.recording_mode)
                    .map_err(DskError::invalid_format)?;

                let mut track = Track::new(json_track.track_number, json_track.side_number);
                track.gap3_length = track_gap3;
                track.filler_byte = track_filler;
                track.data_rate = data_rate;
                track.recording_mode = recording_mode;

                for json_sector in json_track.sectors {
                    let sector_id = decode_hex(&json_sector.id.sector)
                        .map_err(DskError::invalid_format)?;
                    let st1 = decode_hex(&json_sector.fdc_status1)
                        .map_err(DskError::invalid_format)?;
                    let st2 = decode_hex(&json_sector.fdc_status2)
                        .map_err(DskError::invalid_format)?;
                    let data = decode_data_hex(&json_sector.data)
                        .map_err(DskError::invalid_format)?;

                    let id = SectorId::new(
                        json_sector.id.track,
                        json_sector.id.side,
                        sector_id,
                        json_sector.id.size_code,
                    );
                    let sector = Sector::with_status(
                        id,
                        FdcStatus1::new(st1),
                        FdcStatus2::new(st2),
                        data,
                    );
                    track.add_sector(sector);
                }

                disk.add_track(track);
            }
            disks.push(disk);
        }

        Ok(DiskImage {
            format,
            spec,
            disks,
            changed: false,
            filename,
            warnings: self.info.warnings,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hex_encode_decode_roundtrip() {
        let data = vec![0x00, 0xE5, 0xFF, 0x41, 0xC1];
        let hex = encode_data_hex(&data);
        assert_eq!(hex, "00E5FF41C1");
        let decoded = decode_data_hex(&hex).unwrap();
        assert_eq!(decoded, data);
    }

    #[test]
    fn test_hex_byte_encode_decode() {
        assert_eq!(encode_hex(0xE5), "0xE5");
        assert_eq!(encode_hex(0x00), "0x00");
        assert_eq!(decode_hex("0xE5").unwrap(), 0xE5);
        assert_eq!(decode_hex("0xC1").unwrap(), 0xC1);
        assert_eq!(decode_hex("0xFF").unwrap(), 0xFF);
    }

    #[test]
    fn test_empty_data_hex() {
        let hex = encode_data_hex(&[]);
        assert_eq!(hex, "");
        let decoded = decode_data_hex("").unwrap();
        assert!(decoded.is_empty());
    }

    #[test]
    fn test_format_name_roundtrip() {
        for fmt in [
            DiskImageFormat::StandardDSK,
            DiskImageFormat::ExtendedDSK,
            DiskImageFormat::RawMgt,
            DiskImageFormat::RawTd0,
        ] {
            assert_eq!(parse_format_name(format_name(fmt)).unwrap(), fmt);
        }
    }

    #[test]
    fn test_json_roundtrip() {
        let mut image = DiskImage::builder()
            .num_sides(1)
            .num_tracks(2)
            .sectors_per_track(3)
            .sector_size(512)
            .build()
            .unwrap();

        let test_data: Vec<u8> = (0..512).map(|i| (i % 256) as u8).collect();
        image.write_sector(0, 0, 0xC1, &test_data).unwrap();

        let json_str = serde_json::to_string_pretty(&JsonDiskImage::from_disk_image(&image))
            .unwrap();

        let roundtrip: JsonDiskImage = serde_json::from_str(&json_str).unwrap();
        let restored = roundtrip.into_disk_image(None).unwrap();

        assert_eq!(restored.format(), image.format());
        assert_eq!(restored.disk_count(), image.disk_count());
        assert_eq!(
            restored.get_disk(0).unwrap().track_count(),
            image.get_disk(0).unwrap().track_count()
        );

        let original_data = image.read_sector(0, 0, 0xC1).unwrap();
        let restored_data = restored.read_sector(0, 0, 0xC1).unwrap();
        assert_eq!(original_data, restored_data);
    }

    #[test]
    fn test_json_roundtrip_file() {
        let mut image = DiskImage::builder()
            .num_sides(2)
            .num_tracks(3)
            .sectors_per_track(4)
            .sector_size(512)
            .build()
            .unwrap();

        for side in 0..2u8 {
            for track in 0..3u8 {
                for sector in 0..4u8 {
                    let sector_id = 0xC1 + sector;
                    let data: Vec<u8> = (0..512)
                        .map(|i| ((side as u16 * 1000 + track as u16 * 100 + sector as u16 * 10 + i as u16) % 256) as u8)
                        .collect();
                    image.write_sector(side, track, sector_id, &data).unwrap();
                }
            }
        }

        let dir = std::env::temp_dir();
        let path = dir.join(format!("dskmgr_json_rt_{}.json", std::process::id()));

        write_json(&image, &path).unwrap();
        let restored = read_json(&path).unwrap();
        std::fs::remove_file(&path).ok();

        assert_eq!(restored.format(), image.format());
        assert_eq!(restored.disk_count(), image.disk_count());

        for side in 0..2u8 {
            for track in 0..3u8 {
                for sector in 0..4u8 {
                    let sector_id = 0xC1 + sector;
                    let original = image.read_sector(side, track, sector_id).unwrap();
                    let restored_sector = restored.read_sector(side, track, sector_id).unwrap();
                    assert_eq!(original, restored_sector, "Mismatch at side={} track={} sector={}", side, track, sector_id);
                }
            }
        }
    }
}
