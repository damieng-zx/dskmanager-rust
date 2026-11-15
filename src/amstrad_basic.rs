//! Amstrad BASIC tokenized file decoder (Locomotive BASIC for Amstrad CPC)
use crate::error::{DskError, Result};
use crate::filesystem::{HeaderType, try_parse_header};

/// Check if data contains a decodable Amstrad BASIC file (AMSDOS only)
/// 
/// Returns true if the data has an AMSDOS header with BASIC type
pub fn can_decode_amstrad_basic(data: &[u8]) -> bool {
    let header = try_parse_header(data);
    matches!(header.header_type, HeaderType::Amsdos) && header.meta.starts_with("BASIC")
}

/// Decode an Amstrad BASIC file from raw file data (with header)
/// 
/// This function checks for an AMSDOS BASIC header, strips it, and decodes the BASIC program.
/// Only works with AMSDOS headers.
/// 
/// # Arguments
/// * `data` - The raw file data (including header if present)
/// 
/// # Returns
/// * `Ok(Some(text))` if successfully decoded
/// * `Ok(None)` if not an AMSDOS BASIC file
/// * `Err(e)` if decoding failed
pub fn decode_amstrad_basic_file(data: &[u8]) -> Result<Option<String>> {
    let header = try_parse_header(data);
    
    // Only decode AMSDOS BASIC files
    if !matches!(header.header_type, HeaderType::Amsdos) || !header.meta.starts_with("BASIC") {
        return Ok(None);
    }
    
    // Strip header if present
    let mut basic_data = data;
    if data.len() >= header.header_size {
        basic_data = &data[header.header_size..];
    }
    
    // Decode the BASIC program
    decode_amstrad_basic(basic_data).map(Some)
}

/// Decode a Locomotive BASIC tokenized file to text
/// 
/// The data should be the BASIC program data (after stripping any AMSDOS header).
/// Locomotive BASIC programs are stored with:
/// - Line length (2 bytes, little-endian) - includes length bytes + line number bytes + data + terminator
/// - Line number (2 bytes, little-endian)
/// - Tokenized line data
/// - Line terminator (0x00)
/// - Program ends when length is 0
/// 
/// # Arguments
/// * `data` - The BASIC program data (tokenized)
pub fn decode_amstrad_basic(data: &[u8]) -> Result<String> {
    if data.is_empty() {
        return Ok(String::new());
    }

    let mut output = String::new();
    let mut pos = 0;

    while pos < data.len() {
        // Check if we have enough bytes for a line header (length = 2 bytes minimum)
        if pos + 2 > data.len() {
            break;
        }

        // Read line length (2 bytes, little-endian) - comes FIRST
        // If 0, signals end of BASIC program
        // Otherwise, includes: 2 bytes (length) + 2 bytes (line number) + data + 1 byte (terminator)
        let line_len = u16::from_le_bytes([data[pos], data[pos + 1]]) as usize;
        pos += 2;
        
        // Check for end of program marker (length = 0)
        if line_len == 0 {
            break;
        }
        
        // Sanity check: length must be at least 5 (2 length + 2 line_num + 1 terminator)
        // and reasonable maximum
        if !(5..=1000).contains(&line_len) {
            break;
        }
        
        // Check if we have enough data for the full line
        if pos + line_len - 2 > data.len() {
            // Incomplete line data
            break;
        }
        
        // Read line number (2 bytes, little-endian) - comes AFTER length
        let line_num = u16::from_le_bytes([data[pos], data[pos + 1]]);
        pos += 2;
        
        // Sanity check: line number should be reasonable (1-65535)
        if line_num == 0 || line_num > 65000 {
            break;
        }
        
        // Calculate where the line data ends
        // line_len includes: 2 (length) + 2 (line_num) + data + 1 (terminator)
        // So data length = line_len - 2 - 2 - 1 = line_len - 5
        // But we've already read length (2) and line_num (2), so we're at pos
        // The data goes from pos to pos + (line_len - 4), with terminator at the end
        let data_len = line_len - 5; // line_len - 2 (length) - 2 (line_num) - 1 (terminator)
        let line_data_end = pos + data_len;
        let actual_data_end = line_data_end.min(data.len());
        
        let line_data = &data[pos..actual_data_end];
        
        output.push_str(&format!("{} ", line_num));
        output.push_str(&decode_amstrad_basic_line(line_data)?);
        output.push('\n');
        
        // Move past the data to the terminator
        pos = line_data_end;
        
        // Skip the line terminator (0x00) - should be present
        if pos < data.len() && data[pos] == 0x00 {
            pos += 1;
        } else {
            // Missing terminator - might be corrupted, but continue anyway
        }
    }

    Ok(output)
}

/// Decode a single BASIC line (without line number and length header)
fn decode_amstrad_basic_line(data: &[u8]) -> Result<String> {
    let mut output = String::new();
    let mut pos = 0;

    while pos < data.len() {
        let byte = data[pos];
        pos += 1;

        match byte {
            // End of line marker
            0x00 => break,
            
            // Statement separator
            0x01 => {
                // Don't output colon if next token is ELSE (0x97)
                if pos < data.len() && data[pos] != 0x97 {
                    output.push(':');
                }
            }
            
            // Variable definitions (0x02-0x0D) - decode variable name with appropriate suffix
            0x02 => {
                // Integer variable (defined with "%" suffix)
                if pos + 2 <= data.len() {
                    pos += 2; // Skip offset
                    // Variable name follows - read until bit 7 of last char is set
                    while pos < data.len() {
                        let ch = data[pos];
                        pos += 1;
                        if ch == 0x00 {
                            break;
                        }
                        // Check if bit 7 is set (last character)
                        if (ch & 0x80) != 0 {
                            output.push((ch & 0x7F) as char);
                            output.push('%');
                            break;
                        } else {
                            output.push(ch as char);
                        }
                    }
                }
            }
            0x03 => {
                // String variable (defined with "$" suffix)
                if pos + 2 <= data.len() {
                    pos += 2; // Skip offset
                    // Variable name follows - read until bit 7 of last char is set
                    while pos < data.len() {
                        let ch = data[pos];
                        pos += 1;
                        if ch == 0x00 {
                            break;
                        }
                        // Check if bit 7 is set (last character)
                        if (ch & 0x80) != 0 {
                            output.push((ch & 0x7F) as char);
                            output.push('$');
                            break;
                        } else {
                            output.push(ch as char);
                        }
                    }
                }
            }
            0x04 => {
                // Floating point variable (defined with "!" suffix)
                if pos + 2 <= data.len() {
                    pos += 2; // Skip offset
                    // Variable name follows - read until bit 7 of last char is set
                    while pos < data.len() {
                        let ch = data[pos];
                        pos += 1;
                        if ch == 0x00 {
                            break;
                        }
                        // Check if bit 7 is set (last character)
                        if (ch & 0x80) != 0 {
                            output.push((ch & 0x7F) as char);
                            output.push('!');
                            break;
                        } else {
                            output.push(ch as char);
                        }
                    }
                }
            }
            0x05..=0x0A => {
                // Variable definitions (unknown types 0x05-0x0A) - decode without suffix
                if pos + 2 <= data.len() {
                    pos += 2; // Skip offset
                    // Variable name follows - read until bit 7 of last char is set
                    while pos < data.len() {
                        let ch = data[pos];
                        pos += 1;
                        if ch == 0x00 {
                            break;
                        }
                        // Check if bit 7 is set (last character)
                        if (ch & 0x80) != 0 {
                            output.push((ch & 0x7F) as char);
                            break;
                        } else {
                            output.push(ch as char);
                        }
                    }
                }
            }
            0x0B..=0x0D => {
                // Variable definition (no suffix)
                if pos + 2 <= data.len() {
                    pos += 2; // Skip offset
                    // Variable name follows - read until bit 7 of last char is set
                    while pos < data.len() {
                        let ch = data[pos];
                        pos += 1;
                        if ch == 0x00 {
                            break;
                        }
                        // Check if bit 7 is set (last character)
                        if (ch & 0x80) != 0 {
                            output.push((ch & 0x7F) as char);
                            break;
                        } else {
                            output.push(ch as char);
                        }
                    }
                }
            }
            
            // Number constants 0-10
            0x0E => output.push('0'),
            0x0F => output.push('1'),
            0x10 => output.push('2'),
            0x11 => output.push('3'),
            0x12 => output.push('4'),
            0x13 => output.push('5'),
            0x14 => output.push('6'),
            0x15 => output.push('7'),
            0x16 => output.push('8'),
            0x17 => output.push('9'),
            0x18 => output.push_str("10"),
            
            // 8-bit integer decimal value
            0x19 => {
                if pos < data.len() {
                    let val = data[pos] as i8 as i32;
                    output.push_str(&format!("{}", val));
                    pos += 1;
                }
            }
            
            // 16-bit integer decimal value
            0x1A => {
                if pos + 2 <= data.len() {
                    let val = i16::from_le_bytes([data[pos], data[pos + 1]]) as i32;
                    output.push_str(&format!("{}", val));
                    pos += 2;
                }
            }
            
            // 16-bit integer binary value (with "&X" prefix)
            0x1B => {
                if pos + 2 <= data.len() {
                    let val = u16::from_le_bytes([data[pos], data[pos + 1]]);
                    output.push_str(&format!("&X{:04X}", val));
                    pos += 2;
                }
            }
            
            // 16-bit integer hexadecimal value (with "&H" or "&" prefix)
            0x1C => {
                if pos + 2 <= data.len() {
                    let val = u16::from_le_bytes([data[pos], data[pos + 1]]);
                    output.push_str(&format!("&{:04X}", val));
                    pos += 2;
                }
            }
            
            // 16-bit BASIC program line memory address pointer (skip 2 bytes)
            0x1D => {
                if pos + 2 <= data.len() {
                    // Skip the pointer - we can't resolve it here
                    pos += 2;
                }
            }
            
            // 16-bit integer BASIC line number
            0x1E => {
                if pos + 2 <= data.len() {
                    let line_num = u16::from_le_bytes([data[pos], data[pos + 1]]);
                    output.push_str(&format!("{}", line_num));
                    pos += 2;
                }
            }
            
            // Floating point value (5 bytes follow)
            0x1F => {
                if pos + 5 <= data.len() {
                    let num_str = decode_locomotive_float(&data[pos..pos + 5]);
                    output.push_str(&num_str);
                    pos += 5;
                } else {
                    // Incomplete number - skip
                    pos = data.len();
                }
            }
            
            // Space symbol
            0x20 => output.push(' '),
            
            // Quoted string value (0x22)
            0x22 => {
                output.push('"');
                // Read string until closing quote or end of line
                while pos < data.len() {
                    let ch = data[pos];
                    pos += 1;
                    if ch == 0x22 {
                        // Closing quote
                        output.push('"');
                        break;
                    } else if ch == 0x00 {
                        // End of line - unclosed string
                        break;
                    } else if (0x20..=0x7E).contains(&ch) {
                        output.push(ch as char);
                    } else {
                        output.push_str(&format!("\\x{:02X}", ch));
                    }
                }
            }
            
            // ASCII printable symbols (0x21, 0x23-0x7B)
            0x21 | 0x23..=0x7B => {
                output.push(byte as char);
            }
            
            // "|" symbol (RSX prefix)
            0x7C => {
                output.push('|');
                // RSX command follows: 1 byte offset, then RSX name
                if pos < data.len() {
                    let _offset = data[pos] as usize;
                    pos += 1;
                    // Read RSX name until bit 7 of last char is set
                    while pos < data.len() {
                        let ch = data[pos];
                        pos += 1;
                        if ch == 0x00 {
                            break;
                        }
                        // Check if bit 7 is set (last character)
                        if (ch & 0x80) != 0 {
                            output.push((ch & 0x7F) as char);
                            break;
                        } else {
                            output.push(ch as char);
                        }
                    }
                }
            }
            
            // 0x7D-0x7F (not used in standard tokens, but may appear)
            0x7D..=0x7F => {
                output.push_str(&format!("\\x{:02X}", byte));
            }
            
            // Single-byte keywords (0x80-0xFE)
            0x80..=0xFE => {
                let token_text = get_token_text(byte)?;
                output.push_str(token_text);
            }
            
            // Prefix byte for two-byte tokens (0xFF)
            0xFF => {
                if pos < data.len() {
                    let second_byte = data[pos];
                    pos += 1;
                    let token_text = get_token_text_ff_prefix(second_byte)?;
                    output.push_str(token_text);
                }
            }
        }
    }

    Ok(output)
}

/// Decode a Locomotive BASIC 5-byte floating point number
/// Format according to CPCwiki:
/// - Bytes 0-3: mantissa (little-endian, bits reversed)
/// - Byte 3 bit 7: sign bit (1=negative, 0=positive)
/// - Byte 4: exponent (with bias 128)
/// - Mantissa has implied leading 1 bit
fn decode_locomotive_float(bytes: &[u8]) -> String {
    if bytes.len() < 5 {
        return "0".to_string();
    }

    // Check for zero (exponent = 0)
    if bytes[4] == 0 {
        return "0".to_string();
    }

    // Extract exponent (byte 4)
    let exponent_biased = bytes[4];
    let exponent = exponent_biased as i32 - 128;

    // Extract sign from bit 7 of byte 3
    let is_negative = (bytes[3] & 0x80) != 0;

    // Mantissa is stored in bytes 0-3 in little-endian order
    // Need to reverse the bytes to get the correct mantissa
    let mantissa_bytes_le = [bytes[0], bytes[1], bytes[2], bytes[3]];
    
    // Clear the sign bit from byte 3
    let mantissa_bytes_no_sign = [
        mantissa_bytes_le[0],
        mantissa_bytes_le[1],
        mantissa_bytes_le[2],
        mantissa_bytes_le[3] & 0x7F,
    ];
    
    // Convert to u32 (little-endian)
    let mantissa_lower31 = u32::from_le_bytes(mantissa_bytes_no_sign) & 0x7FFFFFFF;
    
    // Reconstruct full mantissa: set bit 31 (implied 1) and use lower 31 bits
    let mantissa = 0x80000000u32 | mantissa_lower31;

    // Calculate value: (mantissa / 2^32) * 2^exponent
    let mantissa_f = mantissa as f64 / (1u64 << 32) as f64;
    let exp_f = 2.0_f64.powi(exponent);
    let value = mantissa_f * exp_f;

    let result = if is_negative {
        -value
    } else {
        value
    };

    // Format the number nicely (BASIC displays to 9 decimal places)
    if result.abs() >= 1.0 && result.abs() < 1e10 {
        if result.fract().abs() < 1e-10 {
            // Integer value
            format!("{}", result as i64)
        } else {
            // Decimal value - round to 9 decimal places
            let rounded = (result * 1e9).round() / 1e9;
            let formatted = format!("{:.9}", rounded);
            formatted.trim_end_matches('0').trim_end_matches('.').to_string()
        }
    } else if result.abs() < 1.0 && result.abs() > 1e-10 {
        // Small decimal numbers
        let rounded = (result * 1e9).round() / 1e9;
        let formatted = format!("{:.9}", rounded);
        formatted.trim_end_matches('0').trim_end_matches('.').to_string()
    } else {
        // Very large or very small numbers - use scientific notation
        format!("{:.9e}", result)
    }
}

/// Get the text representation of a Locomotive BASIC token (0x80-0xFE)
/// Based on the tokenization table from CPCwiki
fn get_token_text(token: u8) -> Result<&'static str> {
    match token {
        0x80 => Ok("AFTER"),
        0x81 => Ok("AUTO"),
        0x82 => Ok("BORDER"),
        0x83 => Ok("CALL"),
        0x84 => Ok("CAT"),
        0x85 => Ok("CHAIN"),
        0x86 => Ok("CLEAR"),
        0x87 => Ok("CLG"),
        0x88 => Ok("CLOSEIN"),
        0x89 => Ok("CLOSEOUT"),
        0x8A => Ok("CLS"),
        0x8B => Ok("CONT"),
        0x8C => Ok("DATA"),
        0x8D => Ok("DEF"),
        0x8E => Ok("DEFINT"),
        0x8F => Ok("DEFREAL"),
        0x90 => Ok("DEFSTR"),
        0x91 => Ok("DEG"),
        0x92 => Ok("DELETE"),
        0x93 => Ok("DIM"),
        0x94 => Ok("DRAW"),
        0x95 => Ok("DRAWR"),
        0x96 => Ok("EDIT"),
        0x97 => Ok("ELSE"),
        0x98 => Ok("END"),
        0x99 => Ok("ENT"),
        0x9A => Ok("ENV"),
        0x9B => Ok("ERASE"),
        0x9C => Ok("ERROR"),
        0x9D => Ok("EVERY"),
        0x9E => Ok("FOR"),
        0x9F => Ok("GOSUB"),
        0xA0 => Ok("GOTO"),
        0xA1 => Ok("IF"),
        0xA2 => Ok("INK"),
        0xA3 => Ok("INPUT"),
        0xA4 => Ok("KEY"),
        0xA5 => Ok("LET"),
        0xA6 => Ok("LINE"),
        0xA7 => Ok("LIST"),
        0xA8 => Ok("LOAD"),
        0xA9 => Ok("LOCATE"),
        0xAA => Ok("MEMORY"),
        0xAB => Ok("MERGE"),
        0xAC => Ok("MID$"),
        0xAD => Ok("MODE"),
        0xAE => Ok("MOVE"),
        0xAF => Ok("MOVER"),
        0xB0 => Ok("NEXT"),
        0xB1 => Ok("NEW"),
        0xB2 => Ok("ON"),
        0xB3 => Ok("ON BREAK"),
        0xB4 => Ok("ON ERROR GOTO"),
        0xB5 => Ok("ON SQ"),
        0xB6 => Ok("OPENIN"),
        0xB7 => Ok("OPENOUT"),
        0xB8 => Ok("ORIGIN"),
        0xB9 => Ok("OUT"),
        0xBA => Ok("PAPER"),
        0xBB => Ok("PEN"),
        0xBC => Ok("PLOT"),
        0xBD => Ok("PLOTR"),
        0xBE => Ok("POKE"),
        0xBF => Ok("PRINT"),
        0xC0 => Ok("'"),
        0xC1 => Ok("RAD"),
        0xC2 => Ok("RANDOMIZE"),
        0xC3 => Ok("READ"),
        0xC4 => Ok("RELEASE"),
        0xC5 => Ok("REM"),
        0xC6 => Ok("RENUM"),
        0xC7 => Ok("RESTORE"),
        0xC8 => Ok("RESUME"),
        0xC9 => Ok("RETURN"),
        0xCA => Ok("RUN"),
        0xCB => Ok("SAVE"),
        0xCC => Ok("SOUND"),
        0xCD => Ok("SPEED"),
        0xCE => Ok("STOP"),
        0xCF => Ok("SYMBOL"),
        0xD0 => Ok("TAG"),
        0xD1 => Ok("TAGOFF"),
        0xD2 => Ok("TROFF"),
        0xD3 => Ok("TRON"),
        0xD4 => Ok("WAIT"),
        0xD5 => Ok("WEND"),
        0xD6 => Ok("WHILE"),
        0xD7 => Ok("WIDTH"),
        0xD8 => Ok("WINDOW"),
        0xD9 => Ok("WRITE"),
        0xDA => Ok("ZONE"),
        0xDB => Ok("DI"),
        0xDC => Ok("EI"),
        0xDD => Ok("FILL"),
        0xDE => Ok("GRAPHICS"),
        0xDF => Ok("MASK"),
        0xE0 => Ok("FRAME"),
        0xE1 => Ok("CURSOR"),
        // 0xE2 is unassigned in Locomotive BASIC.
        0xE3 => Ok("ERL"),
        0xE4 => Ok("FN"),
        0xE5 => Ok("SPC"),
        0xE6 => Ok("STEP"),
        0xE7 => Ok("SWAP"),
        // 0xE8 is unassigned in Locomotive BASIC.
        // 0xE9 is unassigned in Locomotive BASIC.
        0xEA => Ok("TAB"),
        0xEB => Ok("THEN"),
        0xEC => Ok("TO"),
        0xED => Ok("USING"),
        0xEE => Ok(">"),
        0xEF => Ok("="),
        0xF0 => Ok(">="),
        0xF1 => Ok("<"),
        0xF2 => Ok("<>"),
        0xF3 => Ok("<="),
        0xF4 => Ok("+"),
        0xF5 => Ok("-"),
        0xF6 => Ok("*"),
        0xF7 => Ok("/"),
        0xF8 => Ok("^"),
        0xF9 => Ok("\\"),
        0xFA => Ok("AND"),
        0xFB => Ok("MOD"),
        0xFC => Ok("OR"),
        0xFD => Ok("XOR"),
        0xFE => Ok("NOT"),
        _ => Err(DskError::filesystem(format!("Unknown BASIC token: 0x{:02X}", token))),
    }
}

/// Get the text representation of a Locomotive BASIC token with 0xFF prefix
/// Based on the tokenization table from CPCwiki
fn get_token_text_ff_prefix(token: u8) -> Result<&'static str> {
    match token {
        0x00 => Ok("ABS"),
        0x01 => Ok("ASC"),
        0x02 => Ok("ATN"),
        0x03 => Ok("CHR$"),
        0x04 => Ok("CINT"),
        0x05 => Ok("COS"),
        0x06 => Ok("CREAL"),
        0x07 => Ok("EXP"),
        0x08 => Ok("FIX"),
        0x09 => Ok("FRE"),
        0x0A => Ok("INKEY"),
        0x0B => Ok("INP"),
        0x0C => Ok("INT"),
        0x0D => Ok("JOY"),
        0x0E => Ok("LEN"),
        0x0F => Ok("LOG"),
        0x10 => Ok("LOG10"),
        0x11 => Ok("LOWER$"),
        0x12 => Ok("PEEK"),
        0x13 => Ok("REMAIN"),
        0x14 => Ok("SGN"),
        0x15 => Ok("SIN"),
        0x16 => Ok("SPACE$"),
        0x17 => Ok("SQ"),
        0x18 => Ok("SQR"),
        0x19 => Ok("STR$"),
        0x1A => Ok("TAN"),
        0x1B => Ok("UNT"),
        0x1C => Ok("UPPER$"),
        0x1D => Ok("VAL"),
        0x40 => Ok("EOF"),
        0x41 => Ok("ERR"),
        0x42 => Ok("HIMEM"),
        0x43 => Ok("INKEY$"),
        0x44 => Ok("PI"),
        0x45 => Ok("RND"),
        0x46 => Ok("TIME"),
        0x47 => Ok("XPOS"),
        0x48 => Ok("YPOS"),
        0x49 => Ok("DERR"),
        0x71 => Ok("BIN$"),
        0x72 => Ok("DEC$"),
        0x73 => Ok("HEX$"),
        0x74 => Ok("INSTR"),
        0x75 => Ok("LEFT$"),
        0x76 => Ok("MAX"),
        0x77 => Ok("MIN"),
        0x78 => Ok("POS"),
        0x79 => Ok("RIGHT$"),
        0x7A => Ok("ROUND"),
        0x7B => Ok("STRING$"),
        0x7C => Ok("TEST"),
        0x7D => Ok("TESTR"),
        0x7E => Ok("COPYCHR$"),
        0x7F => Ok("VPOS"),
        _ => Err(DskError::filesystem(format!("Unknown BASIC token with FF prefix: 0x{:02X}", token))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a one-line tokenised program: length, line number, body, terminator.
    fn program(body: &[u8]) -> Vec<u8> {
        let len = (2 + 2 + body.len() + 1) as u16;
        let mut out = Vec::new();
        out.extend_from_slice(&len.to_le_bytes());
        out.extend_from_slice(&10u16.to_le_bytes());
        out.extend_from_slice(body);
        out.push(0x00);
        out.extend_from_slice(&[0x00, 0x00]);
        out
    }

    #[test]
    fn test_numeric_tokens_decode_as_digits() {
        // The single-digit constants are emitted one character at a time.
        for (token, text) in [
            (0x0Eu8, "0"), (0x0F, "1"), (0x10, "2"), (0x11, "3"), (0x12, "4"),
            (0x13, "5"), (0x14, "6"), (0x15, "7"), (0x16, "8"), (0x17, "9"),
            (0x18, "10"),
        ] {
            let decoded = decode_amstrad_basic(&program(&[token])).unwrap();
            assert_eq!(decoded, format!("10 {}\n", text), "token 0x{:02X}", token);
        }
    }

    #[test]
    fn test_token_table_is_complete_and_well_formed() {
        // Every single-byte and 0xFF-prefixed token must decode to a non-empty
        // name, and no two tokens may share a name (a sign of a transcription
        // slip in the table).
        use std::collections::HashMap;

        let mut seen: HashMap<&'static str, u8> = HashMap::new();
        // Locomotive BASIC leaves these three opcodes unassigned; decoding them
        // reports an error rather than silently emitting nothing.
        const UNASSIGNED: [u8; 3] = [0xE2, 0xE8, 0xE9];

        for token in 0x80u8..=0xFE {
            if UNASSIGNED.contains(&token) {
                assert!(
                    get_token_text(token).is_err(),
                    "unassigned token 0x{token:02X} should not decode"
                );
                continue;
            }
            let text = get_token_text(token)
                .unwrap_or_else(|e| panic!("token 0x{token:02X} failed: {e}"));
            assert!(!text.is_empty(), "token 0x{token:02X} is empty");
            if let Some(previous) = seen.insert(text, token) {
                panic!("tokens 0x{previous:02X} and 0x{token:02X} both decode to {text:?}");
            }
        }

        // The 0xFF prefix table covers a separate range; check it is non-empty
        // and that its entries are distinct too.
        let mut prefixed = 0;
        let mut prefixed_names: HashMap<&'static str, u8> = HashMap::new();
        for token in 0u8..=0xFF {
            if let Ok(text) = get_token_text_ff_prefix(token) {
                assert!(!text.is_empty(), "prefixed token 0x{token:02X} is empty");
                prefixed += 1;
                if let Some(previous) = prefixed_names.insert(text, token) {
                    panic!(
                        "prefixed tokens 0x{previous:02X} and 0x{token:02X} both decode to {text:?}"
                    );
                }
            }
        }
        assert!(prefixed > 0, "the 0xFF prefix table is empty");
    }
}
