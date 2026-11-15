//! Sector map visualization

use crate::image::{DiskImage, SectorStatus};

/// ANSI color codes for sector map
mod colors {
    pub const RESET: &str = "\x1b[0m";
    pub const BRIGHT_WHITE: &str = "\x1b[97m";
    pub const DARK_WHITE: &str = "\x1b[37m";
    pub const BRIGHT_RED: &str = "\x1b[91m";
    pub const DARK_RED: &str = "\x1b[2;31m";
    pub const BRIGHT_YELLOW: &str = "\x1b[93m";
    pub const DARK_YELLOW: &str = "\x1b[2;33m";
}

/// Draw a visual sector map for a disk side
pub fn draw_sector_map(image: &DiskImage, side: usize) {
    let disk = match image.disks().get(side) {
        Some(d) => d,
        None => {
            println!("Side {} not found.", side);
            return;
        }
    };

    // Find the maximum number of sectors on any track
    let max_sectors = disk.tracks().iter().map(|t| t.sector_count()).max().unwrap_or(0);
    if max_sectors == 0 {
        println!("No sectors found on side {}.", side);
        return;
    }

    let num_tracks = disk.track_count();
    const BLOCK_NO_DATA: &str = "\u{2591}"; // ░ - Light shade (empty)
    const BLOCK_HAS_DATA: &str = "\u{2593}"; // ▓ - Dark shade (in-use)

    println!("Sector Map (Side {})", side);
    println!(
        "Legend: {}In Use{} {}Filler{} {}Error{} {}Deleted{}",
        colors::BRIGHT_WHITE, colors::RESET,
        colors::DARK_WHITE, colors::RESET,
        colors::BRIGHT_RED, colors::RESET,
        colors::BRIGHT_YELLOW, colors::RESET
    );
    println!();

    // Draw each row (physical sector position), bottom to top (sector 0 at bottom)
    for sector_pos in (0..max_sectors).rev() {
        // Print sector position label
        print!("{:>2} ", sector_pos);

        // Draw each column (track)
        for track_num in 0..num_tracks {
            if let Some(track) = disk.get_track(track_num as u8) {
                if let Some(sector) = track.get_sector_by_index(sector_pos) {
                    let fdc_st1 = sector.fdc_status1.0;
                    let fdc_st2 = sector.fdc_status2.0;
                    let status = sector.status(track.filler_byte);
                    let in_use = status == SectorStatus::FormattedInUse;

                    // Choose block character based on whether sector has data
                    let block = if in_use {
                        BLOCK_HAS_DATA
                    } else {
                        BLOCK_NO_DATA
                    };

                    // Check for errors (ST1 & 32 or ST2 & 32)
                    let color = if (fdc_st1 & 32) == 32 || (fdc_st2 & 32) == 32 {
                        if in_use {
                            colors::BRIGHT_RED
                        } else {
                            colors::DARK_RED
                        }
                    }
                    // Check for deleted data mark (ST2 & 64)
                    else if (fdc_st2 & 64) == 64 {
                        if in_use {
                            colors::BRIGHT_YELLOW
                        } else {
                            colors::DARK_YELLOW
                        }
                    }
                    // Normal sector
                    else if in_use {
                        colors::BRIGHT_WHITE
                    } else {
                        colors::DARK_WHITE
                    };

                    print!("{}{}{}", color, block, colors::RESET);
                } else {
                    // No sector at this position
                    print!(" ");
                }
            } else {
                // Track doesn't exist
                print!(" ");
            }
        }
        println!();
    }

    // Draw track number axis (horizontally)
    print!("   "); // Align with sector labels
    
    // Track which columns we've already printed (for multi-digit numbers)
    let mut printed_cols = vec![false; num_tracks];
    
    for track_num in 0..num_tracks {
        if track_num % 5 == 0 && !printed_cols[track_num] {
            // Print the full track number horizontally
            let track_str = track_num.to_string();
            let digits: Vec<char> = track_str.chars().collect();
            
            // Print each digit at its corresponding column
            for (i, digit) in digits.iter().enumerate() {
                let col = track_num + i;
                if col < num_tracks {
                    print!("{}", digit);
                    printed_cols[col] = true;
                }
            }
        } else if !printed_cols[track_num] {
            // Print space for columns not occupied by track numbers
            print!(" ");
        }
    }
    println!();
}
