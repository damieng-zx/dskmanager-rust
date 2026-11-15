//! Floppy Disk Controller (FDC) status register definitions
//!
//! Based on the NEC uPD765/Intel 8272 FDC used in Amstrad CPC, Spectrum +3, etc.

use std::fmt;

/// FDC Status Register 1 (ST1)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FdcStatus1(pub u8);

impl FdcStatus1 {
    /// End of Cylinder (EN) - Bit 7
    /// Set when the FDC tries to access a sector beyond the final sector of a track
    pub const EN: u8 = 0x80;

    /// Data Error (DE) - Bit 5
    /// Set when a CRC error occurs in either the ID field or data field
    pub const DE: u8 = 0x20;

    /// Overrun (OR) - Bit 4
    /// Set if the FDC did not receive DMA service within the required time
    pub const OR: u8 = 0x10;

    /// No Data (ND) - Bit 2
    /// Set if the FDC cannot find the specified sector
    pub const ND: u8 = 0x04;

    /// Not Writable (NW) - Bit 1
    /// Set during a Write command if the disk is write-protected
    pub const NW: u8 = 0x02;

    /// Missing Address Mark (MA) - Bit 0
    /// Set if the FDC does not detect an ID address mark
    pub const MA: u8 = 0x01;

    /// Create a new FdcStatus1 from a raw byte
    #[inline]
    pub fn new(value: u8) -> Self {
        FdcStatus1(value)
    }

    /// Check if end of cylinder bit is set
    #[inline]
    pub fn end_of_cylinder(&self) -> bool {
        (self.0 & Self::EN) != 0
    }

    /// Check if data error bit is set
    #[inline]
    pub fn data_error(&self) -> bool {
        (self.0 & Self::DE) != 0
    }

    /// Check if overrun bit is set
    #[inline]
    pub fn overrun(&self) -> bool {
        (self.0 & Self::OR) != 0
    }

    /// Check if no data bit is set
    #[inline]
    pub fn no_data(&self) -> bool {
        (self.0 & Self::ND) != 0
    }

    /// Check if not writable bit is set
    #[inline]
    pub fn not_writable(&self) -> bool {
        (self.0 & Self::NW) != 0
    }

    /// Check if missing address mark bit is set
    #[inline]
    pub fn missing_address_mark(&self) -> bool {
        (self.0 & Self::MA) != 0
    }

    /// Check if any error flag is set
    #[inline]
    pub fn has_error(&self) -> bool {
        self.0 != 0
    }
}

impl fmt::Display for FdcStatus1 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.0 == 0 {
            write!(f, "OK")?;
        } else {
            let mut flags = Vec::new();
            if self.end_of_cylinder() {
                flags.push("EN");
            }
            if self.data_error() {
                flags.push("DE");
            }
            if self.overrun() {
                flags.push("OR");
            }
            if self.no_data() {
                flags.push("ND");
            }
            if self.not_writable() {
                flags.push("NW");
            }
            if self.missing_address_mark() {
                flags.push("MA");
            }
            write!(f, "{}", flags.join("|"))?;
        }
        Ok(())
    }
}

/// FDC Status Register 2 (ST2)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FdcStatus2(pub u8);

impl FdcStatus2 {
    /// Control Mark (CM) - Bit 6
    /// Set if a sector with deleted data address mark is read
    pub const CM: u8 = 0x40;

    /// Data Error in Data Field (DD) - Bit 5
    /// Set when a CRC error occurs in the data field
    pub const DD: u8 = 0x20;

    /// Wrong Cylinder (WC) - Bit 4
    /// Set if the cylinder address in the ID field does not match
    pub const WC: u8 = 0x10;

    /// Bad Cylinder (BC) - Bit 1
    /// Set if the cylinder address is 0xFF (bad track mark)
    pub const BC: u8 = 0x02;

    /// Missing Address Mark in Data Field (MD) - Bit 0
    /// Set if no data address mark is found
    pub const MD: u8 = 0x01;

    /// Create a new FdcStatus2 from a raw byte
    #[inline]
    pub fn new(value: u8) -> Self {
        FdcStatus2(value)
    }

    /// Check if control mark (deleted data) bit is set
    #[inline]
    pub fn is_deleted(&self) -> bool {
        (self.0 & Self::CM) != 0
    }

    /// Check if data field error bit is set
    #[inline]
    pub fn data_field_error(&self) -> bool {
        (self.0 & Self::DD) != 0
    }

    /// Check if wrong cylinder bit is set
    #[inline]
    pub fn wrong_cylinder(&self) -> bool {
        (self.0 & Self::WC) != 0
    }

    /// Check if bad cylinder bit is set
    #[inline]
    pub fn bad_cylinder(&self) -> bool {
        (self.0 & Self::BC) != 0
    }

    /// Check if missing data mark bit is set
    #[inline]
    pub fn missing_data_mark(&self) -> bool {
        (self.0 & Self::MD) != 0
    }

    /// Check if any error flag is set (excluding deleted data mark)
    #[inline]
    pub fn has_error(&self) -> bool {
        (self.0 & !Self::CM) != 0
    }
}

impl fmt::Display for FdcStatus2 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.0 == 0 {
            write!(f, "OK")?;
        } else {
            let mut flags = Vec::new();
            if self.is_deleted() {
                flags.push("CM");
            }
            if self.data_field_error() {
                flags.push("DD");
            }
            if self.wrong_cylinder() {
                flags.push("WC");
            }
            if self.bad_cylinder() {
                flags.push("BC");
            }
            if self.missing_data_mark() {
                flags.push("MD");
            }
            write!(f, "{}", flags.join("|"))?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fdc_status1_flags() {
        let st1 = FdcStatus1(0x80 | 0x20);
        assert!(st1.end_of_cylinder());
        assert!(st1.data_error());
        assert!(!st1.overrun());
        assert!(st1.has_error());
    }

    #[test]
    fn test_fdc_status1_no_error() {
        let st1 = FdcStatus1(0x00);
        assert!(!st1.has_error());
        assert!(!st1.end_of_cylinder());
        assert!(!st1.data_error());
    }

    #[test]
    fn test_fdc_status2_deleted_data() {
        let st2 = FdcStatus2(0x40);
        assert!(st2.is_deleted());
        assert!(!st2.has_error()); // Deleted data is not an error
    }

    #[test]
    fn test_fdc_status2_errors() {
        let st2 = FdcStatus2(0x20 | 0x10);
        assert!(st2.data_field_error());
        assert!(st2.wrong_cylinder());
        assert!(st2.has_error());
    }

    #[test]
    fn test_fdc_status1_display() {
        let st1 = FdcStatus1(0x80 | 0x04);
        assert_eq!(st1.to_string(), "EN|ND");

        let st1_ok = FdcStatus1(0x00);
        assert_eq!(st1_ok.to_string(), "OK");
    }

    #[test]
    fn test_fdc_status2_display() {
        let st2 = FdcStatus2(0x40 | 0x02);
        assert_eq!(st2.to_string(), "CM|BC");

        let st2_ok = FdcStatus2(0x00);
        assert_eq!(st2_ok.to_string(), "OK");
    }
}
