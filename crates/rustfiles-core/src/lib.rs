//! Filename index, search, NTFS/USN parsing, and disk-analyzer layout.
//!
//! This crate does not draw a window and does not use `unsafe`. Platform IO
//! lives in the `rustfiles` binary.

#![forbid(unsafe_code)]

mod analyze;
mod error;
pub mod flags;
mod index;
mod mft;
mod model;
mod ops;
mod persist;
mod sort;
mod usn;
mod walk;

pub use analyze::{squarify, sunburst, LaidOut, SunSlice, UnitRect, Weighted};
pub use error::{Error, Result};
pub use flags::map_win_attrs;
pub use index::{Index, IndexBuilder, LiveChild, METHOD_MFT, METHOD_WALK, NO_PARENT};
pub use mft::{
    apply_fixup, parse_boot_sector, parse_data_runs, parse_mft_stream, parse_mft_stream_from,
    read_runs_to_vec, unnamed_data_runs, BootSector, DataRun, MftError,
};
pub use model::FileRow;
pub use model::{filetime_to_unix, format_size, RawFile, SearchHit, Volume, FILE_ROW_SEP};
pub use ops::{
    copy_into, create_directory, create_folder, create_text_file, delete_to_trash, paste,
    remove_permanently, rename_path, Clipboard,
};
pub use sort::{natural_cmp, sort_rows, SortColumn};
pub use usn::{
    parse_usn_page, UsnError, UsnRecord, USN_REASON_CLOSE, USN_REASON_DATA_EXTEND,
    USN_REASON_DATA_OVERWRITE, USN_REASON_DATA_TRUNCATION, USN_REASON_FILE_CREATE,
    USN_REASON_FILE_DELETE, USN_REASON_RENAME_NEW_NAME, USN_REASON_RENAME_OLD_NAME,
};
pub use walk::{list_directory, set_walk_thread_hook, walk_tree, walk_tree_into};
