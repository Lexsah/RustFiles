//! Squarified treemap (Bruls, Huizing, van Wijk) and a single-ring sunburst.
//! Coordinates are unitless so the widget can scale them.

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct UnitRect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Weighted {
    pub key: u32,
    pub name: String,
    pub size: u64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct LaidOut {
    pub key: u32,
    pub name: String,
    pub size: u64,
    pub rect: UnitRect,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SunSlice {
    pub key: u32,
    pub name: String,
    pub size: u64,
    pub start: f32,
    pub end: f32,
}

pub fn squarify(items: &[Weighted], bounds: UnitRect) -> Vec<LaidOut> {
    let mut indexed: Vec<&Weighted> = items.iter().filter(|item| item.size > 0).collect();
    indexed.sort_by(|a, b| b.size.cmp(&a.size).then(a.name.cmp(&b.name)));
    if indexed.is_empty() || bounds.w <= 0.0 || bounds.h <= 0.0 {
        return Vec::new();
    }
    let total: f64 = indexed.iter().map(|item| item.size as f64).sum();
    let area = f64::from(bounds.w) * f64::from(bounds.h);
    let areas: Vec<f64> = indexed
        .iter()
        .map(|item| item.size as f64 / total * area)
        .collect();
    let mut rects = Vec::new();
    layout(
        &areas,
        &indexed,
        f64::from(bounds.x),
        f64::from(bounds.y),
        f64::from(bounds.w),
        f64::from(bounds.h),
        &mut rects,
    );
    rects
}

fn layout(
    areas: &[f64],
    items: &[&Weighted],
    x: f64,
    y: f64,
    dx: f64,
    dy: f64,
    out: &mut Vec<LaidOut>,
) {
    if areas.is_empty() || dx <= 0.0 || dy <= 0.0 {
        return;
    }
    if areas.len() == 1 {
        push_row(areas, items, x, y, dx, dy, out);
        return;
    }
    let side = dx.min(dy);
    let mut i = 1;
    while i < areas.len() && worst(&areas[..i], side) >= worst(&areas[..=i], side) {
        i += 1;
    }
    let (nx, ny, ndx, ndy) = push_row(&areas[..i], &items[..i], x, y, dx, dy, out);
    layout(&areas[i..], &items[i..], nx, ny, ndx, ndy, out);
}

fn worst(areas: &[f64], side: f64) -> f64 {
    let sum: f64 = areas.iter().sum();
    if areas.is_empty() || sum <= 0.0 || side <= 0.0 {
        return f64::INFINITY;
    }
    let max = areas.iter().copied().fold(0.0, f64::max);
    let min = areas.iter().copied().fold(f64::INFINITY, f64::min);
    let side2 = side * side;
    let sum2 = sum * sum;
    (side2 * max / sum2).max(sum2 / (side2 * min))
}

fn push_row(
    areas: &[f64],
    items: &[&Weighted],
    mut x: f64,
    mut y: f64,
    dx: f64,
    dy: f64,
    out: &mut Vec<LaidOut>,
) -> (f64, f64, f64, f64) {
    let covered: f64 = areas.iter().sum();
    if dx >= dy {
        let width = if dy > 0.0 { covered / dy } else { 0.0 };
        let y0 = y;
        for (area, item) in areas.iter().zip(items.iter()) {
            let height = if width > 0.0 { area / width } else { 0.0 };
            out.push(laid(item, x, y, width, height));
            y += height;
        }
        (x + width, y0, (dx - width).max(0.0), dy)
    } else {
        let height = if dx > 0.0 { covered / dx } else { 0.0 };
        let x0 = x;
        for (area, item) in areas.iter().zip(items.iter()) {
            let width = if height > 0.0 { area / height } else { 0.0 };
            out.push(laid(item, x, y, width, height));
            x += width;
        }
        (x0, y + height, dx, (dy - height).max(0.0))
    }
}

fn laid(item: &Weighted, x: f64, y: f64, w: f64, h: f64) -> LaidOut {
    LaidOut {
        key: item.key,
        name: item.name.clone(),
        size: item.size,
        rect: UnitRect {
            x: x as f32,
            y: y as f32,
            w: w as f32,
            h: h as f32,
        },
    }
}

pub fn sunburst(items: &[Weighted]) -> Vec<SunSlice> {
    let total: u128 = items.iter().map(|item| item.size as u128).sum();
    if total == 0 {
        return Vec::new();
    }
    let mut angle = 0.0f32;
    let mut out = Vec::new();
    for item in items {
        if item.size == 0 {
            continue;
        }
        let sweep = (item.size as f32 / total as f32) * std::f32::consts::TAU;
        out.push(SunSlice {
            key: item.key,
            name: item.name.clone(),
            size: item.size,
            start: angle,
            end: angle + sweep,
        });
        angle += sweep;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(key: u32, size: u64) -> Weighted {
        Weighted {
            key,
            name: format!("n{key}"),
            size,
        }
    }

    #[test]
    fn one_item_fills_the_square() {
        let laid = squarify(
            &[item(1, 10)],
            UnitRect {
                x: 0.0,
                y: 0.0,
                w: 1.0,
                h: 1.0,
            },
        );
        assert_eq!(laid.len(), 1);
        let rect = laid[0].rect;
        assert!((rect.w * rect.h - 1.0).abs() < 1e-4);
    }

    #[test]
    fn areas_match_weights() {
        let items = vec![item(1, 50), item(2, 30), item(3, 20), item(4, 0)];
        let bounds = UnitRect {
            x: 0.0,
            y: 0.0,
            w: 2.0,
            h: 1.0,
        };
        let laid = squarify(&items, bounds);
        assert_eq!(laid.len(), 3);
        let area: f32 = laid.iter().map(|row| row.rect.w * row.rect.h).sum();
        assert!((area - 2.0).abs() < 1e-3, "area {area}");
        let total = 100.0f32;
        for row in &laid {
            let share = row.rect.w * row.rect.h / 2.0;
            let expect = row.size as f32 / total;
            assert!(
                (share - expect).abs() < 1e-3,
                "{} share {share} expect {expect}",
                row.name
            );
        }
    }

    #[test]
    fn sunburst_sweeps_a_full_turn() {
        let slices = sunburst(&[item(1, 3), item(2, 1)]);
        assert_eq!(slices.len(), 2);
        assert!((slices[0].end - slices[0].start - 0.75 * std::f32::consts::TAU).abs() < 1e-4);
        assert!((slices[1].end - std::f32::consts::TAU).abs() < 1e-4);
        assert!(sunburst(&[item(1, 0)]).is_empty());
    }
}
