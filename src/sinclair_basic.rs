//! Sinclair BASIC tokenized file decoder (ZX Spectrum, ZX81, etc.)

use crate::error::{DskError, Result};
use crate::filesystem::{HeaderType, try_parse_header};

/// Sinclair BASIC mode (48K or 128K)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SinclairBasicMode {
    /// 48K mode - tokens 0xA3 and 0xA4 are UDGs
    Mode48K,
    /// 128K mode - tokens 0xA3 (SPECTRUM) and 0xA4 (PLAY) exist
    Mode128K,
}

/// Check if data contains a decodable Sinclair BASIC file (PLUS3DOS only)
/// 
/// Returns true if the data has a PLUS3DOS header with BASIC type
pub fn can_decode_sinclair_basic(data: &[u8]) -> bool {
    let header = try_parse_header(data);
    matches!(header.header_type, HeaderType::Plus3dos) && header.meta.starts_with("BASIC")
}

/// Decode a Sinclair BASIC file from raw file data (with header)
/// 
/// This function checks for a PLUS3DOS BASIC header, strips it, and decodes the BASIC program.
/// Only works with PLUS3DOS headers (not AMSDOS).
/// 
/// # Arguments
/// * `data` - The raw file data (including header if present)
/// 
/// # Returns
/// * `Ok(Some(text))` if successfully decoded
/// * `Ok(None)` if not a PLUS3DOS BASIC file
/// * `Err(e)` if decoding failed
pub fn decode_sinclair_basic_file(data: &[u8]) -> Result<Option<String>> {
    let header = try_parse_header(data);
    
    // Only decode PLUS3DOS BASIC files (not AMSDOS)
    if !matches!(header.header_type, HeaderType::Plus3dos) || !header.meta.starts_with("BASIC") {
        return Ok(None);
    }
    
    // Strip header if present
    let mut basic_data = data;
    if data.len() >= header.header_size {
        basic_data = &data[header.header_size..];
    }
    
    // Decode using default mode (128K)
    decode_sinclair_basic(basic_data, SinclairBasicMode::Mode128K).map(Some)
}

/// Decode a ZX Spectrum BASIC tokenized file to text
/// 
/// The data should be the BASIC program data (after stripping any PLUS3DOS/AMSDOS header).
/// For ZX Spectrum, BASIC programs start at memory location 0x4009 (16393).
/// 
/// # Arguments
/// * `data` - The BASIC program data (tokenized)
/// * `mode` - Whether to use 48K or 128K mode (affects tokens 0xA3 and 0xA4)
pub fn decode_sinclair_basic(data: &[u8], mode: SinclairBasicMode) -> Result<String> {
    if data.is_empty() {
        return Ok(String::new());
    }

    let mut output = String::new();
    let mut pos = 0;

    // Check for VERSN byte (first byte should be 0 for ZX81, but Spectrum uses different values)
    // For Spectrum, we skip the first byte if it's 0 (ZX81 compatibility)
    // Actually, Spectrum BASIC files start directly with the program lines
    
    while pos < data.len() {
        // Check if we have enough bytes for a line header (line number + length = 4 bytes minimum)
        if pos + 4 > data.len() {
            break;
        }

        // Read line number (2 bytes, big-endian - MSB first, unlike other Z80 values)
        let line_num = u16::from_be_bytes([data[pos], data[pos + 1]]);
        pos += 2;

        // Check for end marker (0x80 0x80 indicates end of program)
        if line_num == 0x8080 {
            break;
        }

        // Read line length (2 bytes, little-endian)
        // The length is the text length including NEWLINE (0x0D), excluding the 4 header bytes
        // So to skip between lines: add line_len + 4 bytes
        let line_len = u16::from_le_bytes([data[pos], data[pos + 1]]) as usize;
        pos += 2;

        // Validate line length - must be at least 1 (the 0x0D NEWLINE)
        if line_len < 1 {
            // Invalid line length - try to recover or break
            break;
        }

        // Calculate where the line data ends
        // line_len is the text length including NEWLINE, so line_end = pos + line_len
        let line_end = pos + line_len;
        
        // Always use what we have, even if it's incomplete
        let actual_end = line_end.min(data.len());
        let line_data = &data[pos..actual_end];
        
        output.push_str(&format!("{} ", line_num));
        output.push_str(&decode_sinclair_basic_line(line_data, mode)?);
        output.push('\n');
        
        // Move to the calculated end position (even if we didn't have all the data)
        pos = line_end;
        
        // If we ran out of data, stop
        if actual_end < line_end {
            break;
        }
    }

    Ok(output)
}

/// Decode a single BASIC line (without line number and length header)
fn decode_sinclair_basic_line(data: &[u8], mode: SinclairBasicMode) -> Result<String> {
    let mut output = String::new();
    let mut pos = 0;

    while pos < data.len() {
        let byte = data[pos];
        pos += 1;

        match byte {
            // End of line marker
            0x0D => break,
            
            // Number marker (0x0E) - integral format (integers -65535 to +65535)
            0x0E => {
                let available = data.len().saturating_sub(pos);
                if available >= 5 {
                    let num_str = decode_spectrum_integral(&data[pos..pos + 5]);
                    output.push_str(&num_str);
                    pos += 5; // Skip the 5-byte number
                } else {
                    // Incomplete number data - skip what we have and continue
                    pos = data.len();
                }
            }
            
            // Number marker (0x7E) - floating point format (all other numbers)
            0x7E => {
                let available = data.len().saturating_sub(pos);
                if available >= 5 {
                    let num_str = decode_spectrum_float(&data[pos..pos + 5]);
                    output.push_str(&num_str);
                    pos += 5; // Skip the 5-byte number
                } else {
                    // Incomplete number data - skip what we have and continue
                    pos = data.len();
                }
            }
            
            // String marker (0x0F) - followed by length byte and string data
            // Note: Numbers are stored twice (as string and as floating point).
            // We skip the string version and only show the floating point version.
            0x0F => {
                if pos < data.len() {
                    let str_len = data[pos] as usize;
                    pos += 1;
                    let available = data.len().saturating_sub(pos);
                    let actual_len = str_len.min(available);
                    
                    if actual_len > 0 {
                        // Check if this string is a number representation (all digits, spaces, decimal point, +, -, E)
                        let is_number_string = data[pos..pos + actual_len].iter().all(|&ch| {
                            ch.is_ascii_digit() ||
                            ch == b' ' || ch == b'.' || ch == b'+' || ch == b'-' || 
                            ch == b'E' || ch == b'e'
                        });
                        
                        if is_number_string && actual_len == str_len {
                            // Skip number strings - we'll show the floating point version instead
                            pos += actual_len;
                        } else {
                            // Regular string - show what we have
                            output.push('"');
                            for i in 0..actual_len {
                                let ch = data[pos + i];
                                // Handle string characters - but don't process 0x0D as part of string
                                if ch == 0x0D {
                                    // NEWLINE ends the string
                                    break;
                                } else if (0x20..=0x7E).contains(&ch) {
                                    output.push(ch as char);
                                } else {
                                    output.push_str(&format!("\\x{:02X}", ch));
                                }
                            }
                            output.push('"');
                            // Advance past the string (or to 0x0D if we hit it)
                            pos += actual_len;
                        }
                    }
                    // If we don't have enough data, continue processing (don't break)
                }
                // If no string length byte, continue (don't break)
            }
            
            // Token 0xA3: SPECTRUM (128K) or UDG A (48K)
            0xA3 => {
                ensure_space_before_keyword(&mut output);
                if mode == SinclairBasicMode::Mode128K {
                    output.push_str("SPECTRUM");
                } else {
                    output.push_str("UDG-A");
                }
                output.push(' ');
            }
            
            // Token 0xA4: PLAY (128K) or UDG B (48K)
            0xA4 => {
                ensure_space_before_keyword(&mut output);
                if mode == SinclairBasicMode::Mode128K {
                    output.push_str("PLAY");
                } else {
                    output.push_str("UDG-B");
                }
                output.push(' ');
            }
            
            // Tokens 0xA5-0xFF are BASIC keywords
            0xA5..=0xFF => {
                ensure_space_before_keyword(&mut output);
                let token_text = get_token_text(byte, mode)?;
                output.push_str(token_text);
                output.push(' ');
            }
            
            // Regular ASCII characters (0x20-0x7D, excluding 0x7E which is handled above)
            0x20..=0x7D => {
                output.push(byte as char);
            }
            
            // Extended characters (0x7F-0xA2)
            0x7F..=0xA2 => {
                // These are special characters in Spectrum character set
                let char_text = get_special_char(byte);
                output.push_str(char_text);
            }
            
            // All other control characters (0x00-0x0C, 0x10-0x1F)
            // Note: 0x0D, 0x0E, 0x0F are handled above
            // Skip unknown control characters silently
            _ => {
                // Unknown byte - skip it
            }
        }
    }

    Ok(output)
}

/// Ensure there's a space before a keyword if needed
fn ensure_space_before_keyword(output: &mut String) {
    if let Some(last_ch) = output.chars().last() {
        // Add space if last character is not already a space
        // Always add space after colon, comma, etc. (only skip if already a space)
        if last_ch != ' ' {
            output.push(' ');
        }
    } else {
        // Empty output - no need for space
    }
}

/// Get the text representation of a BASIC token
fn get_token_text(token: u8, _mode: SinclairBasicMode) -> Result<&'static str> {
    match token {
        0xA5 => Ok("RND"),
        0xA6 => Ok("INKEY$"),
        0xA7 => Ok("PI"),
        0xA8 => Ok("FN"),
        0xA9 => Ok("POINT"),
        0xAA => Ok("SCREEN$"),
        0xAB => Ok("ATTR"),
        0xAC => Ok("AT"),
        0xAD => Ok("TAB"),
        0xAE => Ok("VAL$"),
        0xAF => Ok("CODE"),
        0xB0 => Ok("VAL"),
        0xB1 => Ok("LEN"),
        0xB2 => Ok("SIN"),
        0xB3 => Ok("COS"),
        0xB4 => Ok("TAN"),
        0xB5 => Ok("ASN"),
        0xB6 => Ok("ACS"),
        0xB7 => Ok("ATN"),
        0xB8 => Ok("LN"),
        0xB9 => Ok("EXP"),
        0xBA => Ok("INT"),
        0xBB => Ok("SQR"),
        0xBC => Ok("SGN"),
        0xBD => Ok("ABS"),
        0xBE => Ok("PEEK"),
        0xBF => Ok("IN"),
        0xC0 => Ok("USR"),
        0xC1 => Ok("STR$"),
        0xC2 => Ok("CHR$"),
        0xC3 => Ok("NOT"),
        0xC4 => Ok("BIN"),
        0xC5 => Ok("OR"),
        0xC6 => Ok("AND"),
        0xC7 => Ok("<="),
        0xC8 => Ok(">="),
        0xC9 => Ok("<>"),
        0xCA => Ok("LINE"),
        0xCB => Ok("THEN"),
        0xCC => Ok("TO"),
        0xCD => Ok("STEP"),
        0xCE => Ok("DEF FN"),
        0xCF => Ok("CAT"),
        0xD0 => Ok("FORMAT"),
        0xD1 => Ok("MOVE"),
        0xD2 => Ok("ERASE"),
        0xD3 => Ok("OPEN #"),
        0xD4 => Ok("CLOSE #"),
        0xD5 => Ok("MERGE"),
        0xD6 => Ok("VERIFY"),
        0xD7 => Ok("BEEP"),
        0xD8 => Ok("CIRCLE"),
        0xD9 => Ok("INK"),
        0xDA => Ok("PAPER"),
        0xDB => Ok("FLASH"),
        0xDC => Ok("BRIGHT"),
        0xDD => Ok("INVERSE"),
        0xDE => Ok("OVER"),
        0xDF => Ok("OUT"),
        0xE0 => Ok("LPRINT"),
        0xE1 => Ok("LLIST"),
        0xE2 => Ok("STOP"),
        0xE3 => Ok("READ"),
        0xE4 => Ok("DATA"),
        0xE5 => Ok("RESTORE"),
        0xE6 => Ok("NEW"),
        0xE7 => Ok("BORDER"),
        0xE8 => Ok("CONTINUE"),
        0xE9 => Ok("DIM"),
        0xEA => Ok("REM"),
        0xEB => Ok("FOR"),
        0xEC => Ok("GO TO"),
        0xED => Ok("GO SUB"),
        0xEE => Ok("INPUT"),
        0xEF => Ok("LOAD"),
        0xF0 => Ok("LIST"),
        0xF1 => Ok("LET"),
        0xF2 => Ok("PAUSE"),
        0xF3 => Ok("NEXT"),
        0xF4 => Ok("POKE"),
        0xF5 => Ok("PRINT"),
        0xF6 => Ok("PLOT"),
        0xF7 => Ok("RUN"),
        0xF8 => Ok("SAVE"),
        0xF9 => Ok("RANDOMIZE"),
        0xFA => Ok("IF"),
        0xFB => Ok("CLS"),
        0xFC => Ok("DRAW"),
        0xFD => Ok("CLEAR"),
        0xFE => Ok("RETURN"),
        0xFF => Ok("COPY"),
        _ => Err(DskError::filesystem(&format!("Unknown BASIC token: 0x{:02X}", token))),
    }
}

/// Decode a ZX Spectrum 5-byte integral number format
/// Format: [0, sign_byte, low_byte, high_byte, 0]
/// sign_byte: 0 for positive, 0xFF for negative
/// Value: low_byte | (high_byte << 8), subtract 65536 if negative
fn decode_spectrum_integral(bytes: &[u8]) -> String {
    if bytes.len() < 5 {
        return "0".to_string();
    }

    // Check format: bytes 0 and 4 should be 0
    if bytes[0] != 0 || bytes[4] != 0 {
        return "0".to_string();
    }

    let sign_byte = bytes[1];
    let low = bytes[2] as u16;
    let high = bytes[3] as u16;
    let unsigned_value = low | (high << 8);

    let value = if sign_byte == 0xFF {
        // Negative: subtract 65536
        (unsigned_value as i32) - 65536
    } else {
        // Positive
        unsigned_value as i32
    };

    format!("{}", value)
}

/// Decode a ZX Spectrum 5-byte floating point number format
/// Format: [exponent+128, mantissa_bytes (big-endian)]
/// Mantissa is normalized with MSB always 1 (assumed, not stored), bit 7 of byte 1 is sign bit
/// Mantissa bits: [assumed 1][bits 30-24 from byte 1 bits 6-0][bits 23-0 from bytes 2-4]
/// Value = (mantissa / 2^31) * 2^exponent
fn decode_spectrum_float(bytes: &[u8]) -> String {
    if bytes.len() < 5 {
        return "0".to_string();
    }

    let exponent_biased = bytes[0];
    // Zero is stored as five zero bytes.
    if exponent_biased == 0 {
        return "0".to_string();
    }
    let exponent = exponent_biased as i32 - 128;

    // Extract sign from bit 7 of byte 1
    let is_negative = (bytes[1] & 0x80) != 0;

    // The mantissa occupies bytes 1-4 with its leading 1 implied: the top bit
    // of byte 1 holds the sign instead. Restore that implicit bit.
    let mantissa_bytes = [bytes[1] | 0x80, bytes[2], bytes[3], bytes[4]];
    let mantissa = u32::from_be_bytes(mantissa_bytes);

    // The mantissa represents a fraction in [0.5, 1.0), i.e. mantissa / 2^32.
    let mantissa_f = mantissa as f64 / (1u64 << 32) as f64;
    let exp_f = 2.0_f64.powi(exponent);
    let value = mantissa_f * exp_f;

    let result = if is_negative {
        -value
    } else {
        value
    };

    // Format the number nicely
    if result.abs() >= 1.0 && result.abs() < 1e10 {
        if result.fract().abs() < 1e-10 {
            // Integer value
            format!("{}", result as i64)
        } else {
            // Decimal value - show up to 10 significant digits, remove trailing zeros
            let formatted = format!("{:.10}", result);
            formatted.trim_end_matches('0').trim_end_matches('.').to_string()
        }
    } else if result.abs() < 1.0 && result.abs() > 1e-10 {
        // Small decimal numbers
        let formatted = format!("{:.10}", result);
        formatted.trim_end_matches('0').trim_end_matches('.').to_string()
    } else {
        // Very large or very small numbers - use scientific notation
        format!("{:.10e}", result)
    }
}

/// Get special character representation
fn get_special_char(byte: u8) -> &'static str {
    match byte {
        0x7F => "©",  // Copyright symbol
        0x80..=0x8F => "UDG", // User Defined Graphics (simplified)
        0x90..=0x9F => "GRAPH", // Graphics characters
        0xA0 => " ",
        0xA1 => "£",
        0xA2 => "$",
        _ => "?",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_spectrum_float_decoding() {
        // The decoder reconstructed the mantissa but scaled it by 2^31 instead
        // of 2^32, so every value came out doubled (and negative-by-default for
        // zero). These are the canonical ZX Spectrum encodings: exponent+128,
        // then the mantissa with its implied leading 1 replaced by the sign bit.
        assert_eq!(decode_spectrum_float(&[0x81, 0x40, 0x00, 0x00, 0x00]), "1.5");
        assert_eq!(decode_spectrum_float(&[0x82, 0x00, 0x00, 0x00, 0x00]), "2");
        assert_eq!(decode_spectrum_float(&[0x80, 0x00, 0x00, 0x00, 0x00]), "0.5");
        assert_eq!(decode_spectrum_float(&[0x87, 0x48, 0x00, 0x00, 0x00]), "100");
        assert_eq!(decode_spectrum_float(&[0x00, 0x00, 0x00, 0x00, 0x00]), "0");
    }

    #[test]
    fn test_spectrum_float_negative() {
        // The sign bit is the top bit of the first mantissa byte.
        assert_eq!(decode_spectrum_float(&[0x81, 0xC0, 0x00, 0x00, 0x00]), "-1.5");
    }

    #[test]
    fn test_printable_ascii_boundaries() {
        // The line decoder emits 0x20..=0x7D literally and handles 0x7E (the
        // float marker) and 0xA3.. separately. Guards the range tests.
        let mut program = Vec::new();
        program.extend_from_slice(&10u16.to_be_bytes());
        // '!' (0x21, lower printable edge) and '}' (0x7D, upper printable edge).
        let mut text = vec![0x21u8, 0x7D];
        text.push(0x0D);
        program.extend_from_slice(&(text.len() as u16).to_le_bytes());
        program.extend_from_slice(&text);
        program.extend_from_slice(&[0x80, 0x80, 0x00, 0x00]);

        let decoded = decode_sinclair_basic(&program, SinclairBasicMode::Mode128K).unwrap();
        assert_eq!(decoded, "10 !}\n");
    }
}
