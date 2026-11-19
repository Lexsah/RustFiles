//! On-disk and in-memory file flags. Windows attribute bits are mapped in
//! [`map_win_attrs`] so the rest of the crate never sees Win32 constants.

pub const DIRECTORY: u16 = 1 << 0;
pub const HIDDEN: u16 = 1 << 1;
pub const SYSTEM: u16 = 1 << 2;
pub const REPARSE: u16 = 1 << 3;
pub const READONLY: u16 = 1 << 4;
pub const TOMBSTONE: u16 = 1 << 15;

pub const WIN_READONLY: u32 = 0x0000_0001;
pub const WIN_HIDDEN: u32 = 0x0000_0002;
pub const WIN_SYSTEM: u32 = 0x0000_0004;
pub const WIN_DIRECTORY: u32 = 0x0000_0010;
pub const WIN_REPARSE: u32 = 0x0000_0400;

pub fn map_win_attrs(attrs: u32) -> u16 {
    let mut flags = 0u16;
    if attrs & WIN_DIRECTORY != 0 {
        flags |= DIRECTORY;
    }
    if attrs & WIN_HIDDEN != 0 {
        flags |= HIDDEN;
    }
    if attrs & WIN_SYSTEM != 0 {
        flags |= SYSTEM;
    }
    if attrs & WIN_REPARSE != 0 {
        flags |= REPARSE;
    }
    if attrs & WIN_READONLY != 0 {
        flags |= READONLY;
    }
    flags
}

pub fn is_hidden_name(name: &str, flags: u16) -> bool {
    flags & HIDDEN != 0 || name.starts_with('.')
}
