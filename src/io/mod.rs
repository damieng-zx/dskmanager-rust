/// I/O operations for reading and writing DSK files

/// Reader implementation for DSK files
pub mod reader;
/// Reader implementation for MGT files
pub mod mgt_reader;
/// Reader implementation for TRD files
pub mod trd_reader;
/// Reader implementation for TeleDisk files
pub mod td0_reader;
/// Writer implementation for DSK files
pub mod writer;
/// Reader/writer for JSON format
pub mod json;

pub use json::{is_json_file, read_json, write_json};
pub use mgt_reader::{is_mgt_file, read_mgt};
pub use trd_reader::{is_trd_file, read_trd};
pub use td0_reader::{is_td0_file, read_td0};
pub use reader::read_dsk;
pub use writer::write_dsk;
