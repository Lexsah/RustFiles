//! RustFiles — a cross-platform file manager with an Everything-style index.

mod app;
mod controls;
mod i18n;
mod icons;
mod layout;
mod nav;
mod platform;
mod theme;
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
    if let Some(path) = take_option(&args, "--perf-report") {
        std::env::set_var("RUSTFILES_PERF", path);
    }
    if let Some(path) = positional(&args) {
        std::env::set_var("RUSTFILES_OPEN", path);
    }
    if let Ok(smoke) = std::env::var("RUSTFILES_SMOKE") {
        // The smoke directory is the whole index, even if a developer shell
        // already pointed RUSTFILES_INDEX_PATHS somewhere else.
        std::env::set_var("RUSTFILES_INDEX_PATHS", &smoke);
    }

    let mut width = 1280.0;
    let mut height = 800.0;
    if let Ok(spec) = std::env::var("RUSTFILES_SIZE") {
        if let Some((w, h)) = spec.split_once('x') {
            width = w.parse().unwrap_or(width);
            height = h.parse().unwrap_or(height);
        }
    }
    let viewport = egui::ViewportBuilder::default()
        .with_inner_size([width, height])
        .with_min_inner_size([860.0, 520.0])
        .with_title("RustFiles")
        .with_decorations(false);
    // Real Mica needs a transparent window. Linux screenshots stay opaque so
    // the solid Files fallback colors stay visible under Xvfb.
    #[cfg(windows)]
    let viewport = viewport.with_transparent(true);
    // Windows uses a DXGI swapchain so DWM can composite premultiplied alpha
    // over Mica. Glow/WGL has no alpha channel, which is why the real PC stayed
    // flat #1c1c1c. Linux and the llvmpipe CI tour stay on glow.
    let use_wgpu =
        cfg!(windows) && std::env::var("RUSTFILES_RENDERER").ok().as_deref() != Some("glow");
    let mut options = eframe::NativeOptions {
        viewport,
        renderer: if use_wgpu {
            eframe::Renderer::Wgpu
        } else {
            eframe::Renderer::Glow
        },
        ..Default::default()
    };
    if use_wgpu {
        if let eframe::egui_wgpu::WgpuSetup::CreateNew(ref mut create) =
            options.wgpu_options.wgpu_setup
        {
            create.instance_descriptor.backends = eframe::wgpu::Backends::DX12;
        }
        std::env::set_var("RUSTFILES_ALPHA_SURFACE", "wgpu-dx12-premultiplied");
    } else {
        std::env::set_var("RUSTFILES_ALPHA_SURFACE", "glow");
    }
    eframe::run_native(
        "RustFiles",
        options,
        Box::new(|cc| Ok(Box::new(app::RustFilesApp::new(cc)))),
    )
}

impl eframe::App for app::RustFilesApp {
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        apply_mica(self, frame, ui.ctx());
        if let Some(scale) = platform::window_scale() {
            let native = ui.ctx().native_pixels_per_point().unwrap_or(1.0);
            let current = ui.ctx().pixels_per_point();
            if scale > native + 0.08 && (current - scale).abs() > 0.08 {
                ui.ctx().set_pixels_per_point(scale);
            }
        }
        self.pump(ui.ctx());
        self.keys(ui.ctx());
        if self.layout_path.is_some() {
            self.layout_log.clear();
        }
        self.draw(ui);
        self.shot_step(ui.ctx());
        self.smoke_step(ui.ctx());
        if self.indexing {
            let files = self
                .progress
                .files
                .load(std::sync::atomic::Ordering::Relaxed);
            if files != self.indexed {
                self.indexed = files;
            }
            // Status text follows the file counter, at 2 Hz and no faster.
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(500));
        }
        self.perf_tick();
    }

    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        if self.mica_applied {
            [0.0, 0.0, 0.0, 0.0]
        } else {
            theme::palette(self.dark).window.to_normalized_gamma_f32()
        }
    }
}

fn apply_mica(app: &mut app::RustFilesApp, frame: &eframe::Frame, ctx: &egui::Context) {
    #[cfg(windows)]
    if !app.mica_applied {
        use raw_window_handle::HasWindowHandle as _;
        if let Ok(handle) = frame.window_handle() {
            if let raw_window_handle::RawWindowHandle::Win32(win) = handle.as_raw() {
                let hwnd = win.hwnd.get();
                // DWM ignores the backdrop until the HWND is on screen.
                if !platform::window_is_visible(hwnd) {
                    ctx.request_repaint();
                    return;
                }
                if !app.mica_armed {
                    let line = platform::enable_window_backdrop(hwnd, app.dark);
                    platform::write_diag(&line);
                    app.mica_log = line;
                    app.mica_armed = true;
                    ctx.request_repaint();
                    return;
                }
                let _ = window_vibrancy::apply_mica(handle, Some(app.dark));
                let line = platform::enable_window_backdrop(hwnd, app.dark);
                platform::write_diag(&line);
                app.mica_log = line;
                app.mica_applied = true;
                let mut visuals = ctx.global_style().visuals.clone();
                visuals.window_fill = egui::Color32::TRANSPARENT;
                visuals.panel_fill = egui::Color32::TRANSPARENT;
                ctx.set_visuals(visuals);
            }
        }
    }
    #[cfg(not(windows))]
    {
        let _ = (app, frame, ctx);
    }
}

fn take_option(args: &[String], name: &str) -> Option<String> {
    let index = args.iter().position(|arg| arg == name)?;
    args.get(index + 1).cloned()
}

fn positional(args: &[String]) -> Option<&String> {
    let mut skip_value = false;
    for arg in args {
        if skip_value {
            skip_value = false;
            continue;
        }
        if arg == "--perf-report" {
            skip_value = true;
            continue;
        }
        if !arg.starts_with('-') {
            return Some(arg);
        }
    }
    None
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
    let mut loaded = Index::load(&saved).map_err(|err| err.to_string())?;
    if loaded.search("needle", 5).is_empty() {
        return Err("reloaded index could not search".into());
    }
    controls::exercise(&dir).map_err(|err| format!("command exercise: {err}"))?;
    let _ = fs::remove_dir_all(&dir);
    for control in controls::catalog() {
        println!(
            "control {} / {} — {}",
            control.surface, control.name, control.behavior
        );
    }
    println!("self-test ok");
    let _ = PathBuf::from(".");
    Ok(())
}
