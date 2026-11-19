//! Glyphs from Fluent System Icons (MIT), the same family Files uses for chrome.
//! Codepoints come from `FluentSystemIcons-Regular.json`.

use egui::{Color32, FontFamily, FontId, RichText, Ui};

pub const HOME: u32 = 62592;
pub const FOLDER: u32 = 62488;
#[allow(dead_code)]
pub const FOLDER_OPEN: u32 = 62510;
pub const ARROW_LEFT: u32 = 61787;
pub const ARROW_RIGHT: u32 = 61825;
pub const ARROW_UP: u32 = 61851;
pub const ARROW_SYNC: u32 = 61840;
pub const ARROW_DOWNLOAD: u32 = 61776;
pub const ADD: u32 = 61705;
pub const CUT: u32 = 62266;
pub const COPY: u32 = 62251;
pub const PASTE: u32 = 62165;
pub const RENAME: u32 = 63081;
pub const DELETE: u32 = 62284;
pub const SHARE: u32 = 63151;
pub const SEARCH: u32 = 63119;
pub const DESKTOP: u32 = 62297;
pub const DOCUMENT: u32 = 62328;
pub const DOCUMENT_TEXT: u32 = 58711;
pub const IMAGE: u32 = 62600;
pub const MUSIC: u32 = 59476;
pub const VIDEO: u32 = 63564;
pub const GLOBE: u32 = 62554;
pub const TAG: u32 = 63356;
#[allow(dead_code)]
pub const LIST: u32 = 62701;
pub const GRID: u32 = 62562;
pub const CHEVRON_RIGHT: u32 = 62128;
#[allow(dead_code)]
pub const CHEVRON_DOWN: u32 = 62115;
#[allow(dead_code)]
pub const MORE: u32 = 59428;
pub const HARD_DRIVE: u32 = 59107;
pub const SORT: u32 = 61834;
pub const DISMISS: u32 = 62313;
pub const MAXIMIZE: u32 = 59371;
pub const SUBTRACT: u32 = 60368;
pub const RESTORE: u32 = 60310;
pub const INFO: u32 = 62627;
pub const PIN: u32 = 62977;
#[allow(dead_code)]
pub const HISTORY: u32 = 62590;
pub const STAR: u32 = 63247;
pub const RECYCLE: u32 = 983492;
#[allow(dead_code)]
pub const CHECK: u32 = 62100;

pub fn glyph(codepoint: u32) -> String {
    char::from_u32(codepoint)
        .map(|ch| ch.to_string())
        .unwrap_or_default()
}

pub fn icon_font(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name("fluent".into()))
}

#[allow(dead_code)]
pub fn icon(ui: &mut Ui, codepoint: u32, size: f32, color: Color32) {
    ui.label(
        RichText::new(glyph(codepoint))
            .font(icon_font(size))
            .color(color),
    );
}

pub fn type_glyph(is_dir: bool, name: &str) -> u32 {
    if is_dir {
        return FOLDER;
    }
    let ext = name
        .rsplit_once('.')
        .map(|(_, ext)| ext.to_ascii_lowercase())
        .unwrap_or_default();
    match ext.as_str() {
        "png" | "jpg" | "jpeg" | "gif" | "bmp" | "webp" | "tif" | "tiff" => IMAGE,
        "mp3" | "flac" | "wav" | "aac" | "ogg" | "m4a" => MUSIC,
        "mp4" | "mkv" | "mov" | "avi" | "webm" => VIDEO,
        "txt" | "md" | "rs" | "json" | "toml" | "xml" | "csv" | "log" => DOCUMENT_TEXT,
        "pdf" | "doc" | "docx" | "xls" | "xlsx" | "ppt" | "pptx" => DOCUMENT,
        _ => DOCUMENT,
    }
}
