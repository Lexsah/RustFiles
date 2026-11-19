//! Launches the window under a real GL context and checks that icons and labels
//! share a vertical center. Golden pixels catch a layout that drifts.

use std::fs;
use std::path::PathBuf;
use std::process::Command;

fn display_available() -> bool {
    if cfg!(windows) {
        return false;
    }
    std::env::var_os("DISPLAY").is_some() || std::env::var_os("WAYLAND_DISPLAY").is_some()
}

#[test]
fn tiles_keep_names_beside_icons() {
    if !display_available() {
        eprintln!("skip tiles alignment: no display");
        return;
    }
    let dir = std::env::temp_dir().join(format!("rustfiles-align-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(dir.join("sub")).unwrap();
    fs::write(dir.join("hello.txt"), b"hello").unwrap();
    let shot = dir.join("tiles.png");
    let layout = dir.join("layout.json");
    let output = Command::new(env!("CARGO_BIN_EXE_rustfiles"))
        .env("RUSTFILES_OPEN", &dir)
        .env("RUSTFILES_INDEX_PATHS", &dir)
        .env("RUSTFILES_SHOT", &shot)
        .env("RUSTFILES_LAYOUT", &layout)
        .env("RUSTFILES_VIEW", "tiles")
        .output()
        .expect("launch");
    assert!(
        output.status.success(),
        "shot exited {}\n{}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    let marks: Vec<(String, [f32; 4])> =
        serde_json::from_str(&fs::read_to_string(&layout).unwrap()).unwrap();
    let icon = marks.iter().find(|(name, _)| name == "tile-icon");
    let label = marks.iter().find(|(name, _)| name == "tile-name");
    let (Some((_, icon)), Some((_, label))) = (icon, label) else {
        panic!("missing tile marks: {marks:?}");
    };
    let icon_mid = icon[1] + icon[3] / 2.0;
    let label_mid = label[1] + label[3] / 2.0;
    assert!(
        (icon_mid - label_mid).abs() < 24.0,
        "tile name is not beside the thumbnail: icon {icon:?} label {label:?}"
    );
    assert!(
        label[0] >= icon[0] + icon[2] - 1.0,
        "tile name overlaps the thumbnail"
    );
    assert!(shot.is_file() && fs::metadata(&shot).unwrap().len() > 1000);

    let golden = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/golden/tiles.png");
    if std::env::var_os("RUSTFILES_WRITE_GOLDEN").is_some() || !golden.exists() {
        fs::create_dir_all(golden.parent().unwrap()).unwrap();
        fs::copy(&shot, &golden).unwrap();
        return;
    }
    assert!(
        images_close(&shot, &golden, 28.0),
        "tiles screenshot drifted from tests/golden/tiles.png"
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn detail_names_share_the_icon_row() {
    if !display_available() {
        eprintln!("skip details alignment: no display");
        return;
    }
    let dir = std::env::temp_dir().join(format!("rustfiles-align-details-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("hello.txt"), b"hello").unwrap();
    let shot = dir.join("details.png");
    let layout = dir.join("layout.json");
    let output = Command::new(env!("CARGO_BIN_EXE_rustfiles"))
        .env("RUSTFILES_OPEN", &dir)
        .env("RUSTFILES_INDEX_PATHS", &dir)
        .env("RUSTFILES_SHOT", &shot)
        .env("RUSTFILES_LAYOUT", &layout)
        .env("RUSTFILES_VIEW", "details")
        .output()
        .expect("launch");
    assert!(
        output.status.success(),
        "shot exited {}\n{}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    let marks: Vec<(String, [f32; 4])> =
        serde_json::from_str(&fs::read_to_string(&layout).unwrap()).unwrap();
    let icon = marks
        .iter()
        .find(|(name, _)| name == "detail-icon")
        .map(|(_, rect)| *rect);
    let label = marks
        .iter()
        .find(|(name, _)| name == "detail-name")
        .map(|(_, rect)| *rect);
    let (Some(icon), Some(label)) = (icon, label) else {
        panic!("missing detail marks: {marks:?}");
    };
    let icon_mid = icon[1] + icon[3] / 2.0;
    let label_mid = label[1] + label[3] / 2.0;
    assert!(
        (icon_mid - label_mid).abs() <= 2.0,
        "detail name is not vertically centered with its icon: icon {icon:?} label {label:?}"
    );
    let _ = fs::remove_dir_all(&dir);
}

fn images_close(left: &std::path::Path, right: &std::path::Path, tolerance: f32) -> bool {
    let left = image::open(left).expect("shot").to_rgba8();
    let right = image::open(right).expect("golden").to_rgba8();
    if left.dimensions() != right.dimensions() {
        return false;
    }
    let mut total = 0.0f32;
    let mut count = 0.0f32;
    for (a, b) in left.pixels().zip(right.pixels()) {
        for channel in 0..4 {
            total += (a[channel] as f32 - b[channel] as f32).abs();
            count += 1.0;
        }
    }
    total / count <= tolerance
}
