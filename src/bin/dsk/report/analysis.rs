//! Per-image analysis shared by the filters and both report renderers.
//!
//! Everything derived from a `DiskImage` — the detected specification, the
//! protection findings, boot detection and the structural characteristics — is
//! computed once here. Callers used to re-run `DiskSpecification::identify`
//! (which walks ten detectors) up to five times per file.

use dskmanager::protection::{self, ProtectionResult};
use dskmanager::*;

use crate::report::{md_quirks, report_format};

/// Analysis result for one image.
pub struct DiskAnalysis {
    /// Detected disk specification (geometry and filesystem layout).
    pub spec: DiskSpecification,
    /// Human-readable format label for reports.
    pub format: String,
    /// Protection findings, one per side that reported any.
    pub protections: Vec<(usize, ProtectionResult)>,
    /// Boot detection result.
    pub boot: BootDetection,
    /// Structural quirks relative to the detected format.
    pub quirks: Vec<String>,
}

impl DiskAnalysis {
    /// Analyse an image once.
    pub fn new(image: &DiskImage) -> Self {
        let spec = DiskSpecification::identify(image);
        let format = report_format(image, &spec);

        let protections = image
            .disks()
            .iter()
            .enumerate()
            .filter_map(|(side, disk)| protection::detect(disk).map(|found| (side, found)))
            .collect();

        let boot = BootDetection::detect(image);
        let quirks = md_quirks(image, &spec);

        Self {
            spec,
            format,
            protections,
            boot,
            quirks,
        }
    }

    /// Format label with protection summarised onto one line.
    pub fn protection_summary(&self) -> String {
        if self.protections.is_empty() {
            return "None detected".to_string();
        }
        let multi_side = self.protections.len() > 1;
        self.protections
            .iter()
            .map(|(side, found)| {
                if multi_side {
                    format!("Side {}: {} ({})", side, found.name, found.qualified_reason())
                } else {
                    format!("{} ({})", found.name, found.qualified_reason())
                }
            })
            .collect::<Vec<_>>()
            .join("; ")
    }

    /// Detailed protection lines, prefixed by side when there is more than one.
    pub fn protection_details(&self) -> Vec<String> {
        let multi_side = self.protections.len() > 1;
        let mut lines = Vec::new();
        for (side, found) in &self.protections {
            if found.details.is_empty() {
                continue;
            }
            if multi_side {
                lines.push(format!("Side {}:", side));
            }
            lines.extend(found.details.iter().cloned());
        }
        lines
    }

    /// One-line protection detail for the CSV cell.
    pub fn protection_csv(&self) -> String {
        self.protections
            .iter()
            .map(|(_, found)| {
                let mut entry = format!("{} | {}", found.name, found.qualified_reason());
                if !found.details.is_empty() {
                    entry.push_str(" | ");
                    entry.push_str(&found.details.join(" | "));
                }
                entry
            })
            .collect::<Vec<_>>()
            .join(" ; ")
    }

    /// Boot label for reports.
    pub fn bootable_on(&self) -> &str {
        if self.boot.system.is_empty() {
            "Not bootable"
        } else {
            &self.boot.system
        }
    }

    /// Whether any side has an FDC error.
    pub fn has_fdc_errors(image: &DiskImage) -> bool {
        image.disks().iter().any(|disk| {
            disk.tracks()
                .iter()
                .any(|track| track.sectors().iter().any(|sector| sector.has_error()))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dskmanager::format::FormatSpec;

    fn cpc() -> DiskImage {
        DiskImage::create(FormatSpec::amstrad_data()).unwrap()
    }

    #[test]
    fn analysis_collects_spec_format_and_boot_once() {
        let image = cpc();
        let analysis = DiskAnalysis::new(&image);

        // The cached spec matches a fresh detection and describes this disk.
        assert_eq!(
            analysis.spec.format,
            DiskSpecification::identify(&image).format
        );
        assert_eq!(analysis.format, analysis.spec.format);
        assert!(!analysis.bootable_on().is_empty());
    }

    #[test]
    fn analysis_records_protection_per_side() {
        let mut image = DiskImage::create(FormatSpec::amstrad_data_ds()).unwrap();
        // Sign the Alkatraz loader into side 1 track 0, then make side 1
        // non-uniform (an extra sector on another track) so detection reaches
        // the signature scan instead of bailing out as a uniform disk.
        let mut data = vec![0xE5u8; 512];
        let sig = b" THE ALKATRAZ PROTECTION SYSTEM   (C) 1987  Appleby Associates";
        data[5..5 + sig.len()].copy_from_slice(sig);
        {
            let disk = image.get_disk_mut(1).unwrap();
            disk.get_track_mut(0)
                .unwrap()
                .get_sector_mut(0xC1)
                .unwrap()
                .set_data(data);
            disk.get_track_mut(1)
                .unwrap()
                .add_sector(Sector::with_data(SectorId::new(1, 1, 0xC1, 2), vec![0u8; 512]));
        }

        let analysis = DiskAnalysis::new(&image);
        assert_eq!(analysis.protections.len(), 1);
        assert_eq!(analysis.protections[0].0, 1, "reported against side 1");

        // A single finding is summarised without a side prefix.
        let summary = analysis.protection_summary();
        assert!(summary.starts_with("Alkatraz"), "{summary:?}");
        assert!(!summary.contains("None"), "{summary:?}");
    }

    #[test]
    fn analysis_without_protection_summarises_as_none() {
        let analysis = DiskAnalysis::new(&cpc());
        assert!(analysis.protections.is_empty());
        assert_eq!(analysis.protection_summary(), "None detected");
        assert!(analysis.protection_csv().is_empty());
        assert!(analysis.protection_details().is_empty());
    }

    #[test]
    fn fdc_errors_are_detected_on_any_side() {
        use dskmanager::FdcStatus1;

        let mut image = DiskImage::create(FormatSpec::amstrad_data_ds()).unwrap();
        assert!(!DiskAnalysis::has_fdc_errors(&image));
        image
            .get_disk_mut(1)
            .unwrap()
            .get_track_mut(3)
            .unwrap()
            .get_sector_mut(0xC1)
            .unwrap()
            .fdc_status1 = FdcStatus1::new(FdcStatus1::DE);
        assert!(DiskAnalysis::has_fdc_errors(&image), "side 1 error must count");
    }

}
