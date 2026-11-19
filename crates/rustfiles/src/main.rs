//! RustFiles — a cross-platform file manager with an Everything-style index.

mod app;
mod nav;
mod platform;
mod ui;
mod worker;

use std::fs;
use std::path::PathBuf;
use std::sync::atomic::AtomicU64;

use rustfiles_core::{copy_into, squarify, sunburst, walk_tree, Index, IndexBuilder, UnitRect};

fn main() -> eframe::Result {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|arg| arg == "--version" || arg == "-V") {
        println!("rustfiles {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    if args.iter().any(|arg| arg == "--self-test") {
        if let Err(err) = self_test() {
            eprintln!("{err}");
            std::process::exit(1);
        }
        return Ok(());
    }
    if let Ok(smoke) = std::env::var("RUSTFILES_SMOKE") {
        if std::env::var_os("RUSTFILES_INDEX_PATHS").is_none() {
            std::env::set_var("RUSTFILES_INDEX_PATHS", &smoke);
        }
    }

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1280.0, 800.0])
            .with_min_inner_size([860.0, 520.0])
            .with_title("RustFiles"),
        ..Default::default()
    };
    eframe::run_native(
        "RustFiles",
        options,
        Box::new(|cc| Ok(Box::new(app::RustFilesApp::new(cc)))),
    )
}

impl eframe::App for app::RustFilesApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.pump(ui.ctx());
        self.keys(ui.ctx());
        self.draw(ui);
        self.smoke_step(ui.ctx());
        if self.indexing {
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(100));
        }
    }
}

fn self_test() -> Result<(), String> {
    let dir = std::env::temp_dir().join(format!("rustfiles-self-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(dir.join("sub")).map_err(|err| err.to_string())?;
    fs::write(dir.join("hello.txt"), b"hello").map_err(|err| err.to_string())?;
    fs::write(dir.join("sub").join("nested-needle.txt"), b"abcdef")
        .map_err(|err| err.to_string())?;
    let progress = AtomicU64::new(0);
    let files = walk_tree(&dir, &progress);
    let mut builder = IndexBuilder::new();
    let volume = builder.add_volume(dir.display().to_string(), "self-test".into(), false);
    for file in files {
        builder.add_file(volume, file);
    }
    let mut index = builder.finish();
    if index.search("needle", 10).is_empty() {
        return Err("search did not find nested-needle.txt".into());
    }
    let root = index
        .find_path(&dir.display().to_string())
        .ok_or_else(|| format!("index missed {}", dir.display()))?;
    let weights = index.child_weights(root);
    if squarify(
        &weights,
        UnitRect {
            x: 0.0,
            y: 0.0,
            w: 1.0,
            h: 1.0,
        },
    )
    .is_empty()
    {
        return Err("treemap was empty".into());
    }
    if sunburst(&weights).is_empty() {
        return Err("sunburst was empty".into());
    }
    let copies = dir.join("copies");
    fs::create_dir(&copies).map_err(|err| err.to_string())?;
    copy_into(&dir.join("hello.txt"), &copies).map_err(|err| err.to_string())?;
    let saved = dir.join("index.rfi");
    index.save(&saved).map_err(|err| err.to_string())?;
    let loaded = Index::load(&saved).map_err(|err| err.to_string())?;
    if loaded.search("needle", 5).is_empty() {
        return Err("reloaded index could not search".into());
    }
    let _ = fs::remove_dir_all(&dir);
    println!("self-test ok");
    let _ = PathBuf::from(".");
    Ok(())
}
