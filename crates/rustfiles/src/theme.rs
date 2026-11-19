//! Colors taken from Files' `App.xaml` theme dictionaries.
//!
//! Files paints the file area with WinUI card brushes over a Mica backdrop.
//! These are the solid equivalents of those brushes, so the layout matches on
//! Linux and on Windows when Mica is unavailable.

use egui::Color32;

#[derive(Clone, Copy)]
pub struct FilesPalette {
    pub window: Color32,
    pub sidebar: Color32,
    pub file_area: Color32,
    pub card: Color32,
    pub address: Color32,
    pub stroke: Color32,
    pub text: Color32,
    pub text_secondary: Color32,
    pub hover: Color32,
    #[allow(dead_code)]
    pub pressed: Color32,
    pub selected: Color32,
    pub accent: Color32,
    #[allow(dead_code)]
    pub on_accent: Color32,
    pub tab_selected: Color32,
    pub usage: Color32,
    pub usage_track: Color32,
}

pub fn palette(dark: bool) -> FilesPalette {
    if dark {
        FilesPalette {
            // Mica base and the dark card fills from WinUI.
            window: Color32::from_rgb(32, 32, 32),
            sidebar: Color32::from_rgb(37, 37, 37),
            file_area: Color32::from_rgb(44, 44, 44),
            card: Color32::from_rgb(50, 50, 50),
            address: Color32::from_rgba_unmultiplied(255, 255, 255, 14),
            stroke: Color32::from_rgba_unmultiplied(255, 255, 255, 18),
            text: Color32::from_rgb(255, 255, 255),
            text_secondary: Color32::from_rgb(200, 200, 200),
            hover: Color32::from_rgba_unmultiplied(255, 255, 255, 16),
            pressed: Color32::from_rgba_unmultiplied(255, 255, 255, 10),
            selected: Color32::from_rgba_unmultiplied(80, 192, 255, 48),
            // App.Theme.FillColorAttention in the dark dictionary.
            accent: Color32::from_rgb(80, 192, 255),
            on_accent: Color32::from_rgb(0, 0, 0),
            tab_selected: Color32::from_rgb(44, 44, 44),
            usage: Color32::from_rgb(80, 192, 255),
            usage_track: Color32::from_rgba_unmultiplied(255, 255, 255, 24),
        }
    } else {
        FilesPalette {
            window: Color32::from_rgb(243, 243, 243),
            sidebar: Color32::from_rgb(249, 249, 249),
            // App.Theme.FileArea.BackgroundBrush is #C0FCFCFC over light Mica.
            file_area: Color32::from_rgb(252, 252, 252),
            card: Color32::WHITE,
            address: Color32::WHITE,
            stroke: Color32::from_rgba_unmultiplied(0, 0, 0, 18),
            text: Color32::from_rgb(26, 26, 26),
            text_secondary: Color32::from_rgb(95, 95, 95),
            hover: Color32::from_rgba_unmultiplied(0, 0, 0, 12),
            pressed: Color32::from_rgba_unmultiplied(0, 0, 0, 8),
            selected: Color32::from_rgba_unmultiplied(0, 112, 203, 36),
            accent: Color32::from_rgb(0, 112, 203),
            on_accent: Color32::WHITE,
            tab_selected: Color32::WHITE,
            usage: Color32::from_rgb(0, 112, 203),
            usage_track: Color32::from_rgba_unmultiplied(0, 0, 0, 16),
        }
    }
}
