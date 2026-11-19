//! Shared column geometry so headers and rows use the same 4px grid.

#[allow(dead_code)]
pub const UNIT: f32 = 4.0;
pub const ROW: f32 = 40.0;
pub const ICON: f32 = 20.0;
pub const PAD: f32 = 8.0;
pub const GAP: f32 = 8.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DetailCols {
    pub name: f32,
    pub date: f32,
    pub kind: f32,
    pub size: f32,
}

pub fn detail_cols(width: f32) -> DetailCols {
    let date = 160.0;
    let kind = 96.0;
    let size = 96.0;
    let fixed = PAD + ICON + GAP + date + kind + size + PAD;
    DetailCols {
        name: (width - fixed).max(80.0),
        date,
        kind,
        size,
    }
}

/// Left edge of each text column, after the leading icon slot.
pub fn detail_text_x(left: f32, cols: DetailCols) -> [f32; 4] {
    let name = left + PAD + ICON + GAP;
    let date = name + cols.name;
    let kind = date + cols.date;
    let size = kind + cols.kind;
    [name, date, kind, size]
}

#[allow(dead_code)]
pub fn row_center_y(top: f32) -> f32 {
    top + ROW / 2.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn size_column_ends_on_the_same_edge_as_the_header() {
        let width = 800.0;
        let cols = detail_cols(width);
        let x = detail_text_x(0.0, cols);
        let size_right = x[3] + cols.size;
        assert!((size_right - (width - PAD)).abs() < 0.5);
        assert!((x[1] - x[0] - cols.name).abs() < 0.5);
    }

    #[test]
    fn label_center_matches_the_icon_center() {
        let top = 120.0;
        let y = row_center_y(top);
        let icon_center = top + (ROW - ICON) / 2.0 + ICON / 2.0;
        assert!((y - icon_center).abs() < 0.5);
    }
}
