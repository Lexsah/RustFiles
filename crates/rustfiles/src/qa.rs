//! Replay a JSON action list without touching the OS input queue.
//!
//! `--qa-script steps.json --qa-out D:\\qa` reads:
//! `{ "op": "click", "label": "Home" }`,
//! `{ "op": "navigate", "path": "C:\\Users" }`,
//! `{ "op": "key", "key": "F5" }`,
//! `{ "op": "context", "path": "C:\\Users" }`,
//! `{ "op": "screenshot", "path": "home.png" }`,
//! `{ "op": "wait", "ms": 200 }`.
//!
//! Clicks are `egui::Event`s (pointer events plus an AccessKit click on the
//! widget id), so they do not need window focus or `SendInput`. Screenshots
//! are the wgpu frame texture copied by eframe, not a desktop BitBlt.
//! The report and every screenshot are written under `--qa-out`.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::app::RustFilesApp;

struct Hit {
    label: String,
    id: egui::Id,
    rect: egui::Rect,
}

thread_local! {
    static HITS: RefCell<Vec<Hit>> = const { RefCell::new(Vec::new()) };
}

pub fn register(label: &str, id: egui::Id, rect: egui::Rect) {
    if label.is_empty() {
        return;
    }
    HITS.with(|hits| {
        hits.borrow_mut().push(Hit {
            label: label.to_string(),
            id,
            rect,
        });
    });
}

/// Real pointer clicks win. A scripted click is an egui event pushed earlier
/// in the frame, which makes `response.clicked()` true on its own.
pub fn consume(label: &str, clicked: bool) -> bool {
    let _ = label;
    clicked
}

#[derive(Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
enum Step {
    Click { label: String },
    Key { key: String },
    Navigate { path: String },
    Context { path: String },
    Screenshot { path: String },
    Wait { ms: u64 },
}

#[derive(Serialize)]
struct Outcome {
    op: String,
    ok: bool,
    detail: String,
    /// egui points-to-pixels factor (`pixels_per_point`, 1.0 at 96 DPI).
    dpi_scale: f32,
    #[serde(skip_serializing_if = "Option::is_none")]
    x_px: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    y_px: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    w_px: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    h_px: Option<f32>,
    source: String,
}

struct Shot {
    width: usize,
    height: usize,
    dpi_scale: f32,
    path: String,
}

pub struct Player {
    steps: Vec<Step>,
    index: usize,
    outcomes: Vec<Outcome>,
    wait_until: Option<Instant>,
    report_path: PathBuf,
    out_dir: PathBuf,
    hits: Vec<Hit>,
    seen_ui: bool,
    shot_rx: Option<Receiver<Shot>>,
    dpi_scale: f32,
}

impl Player {
    pub fn load() -> Option<Self> {
        let path = std::env::var("RUSTFILES_QA").ok()?;
        let text = std::fs::read_to_string(&path).unwrap_or_default();
        let steps: Vec<Step> = serde_json::from_str(&text).unwrap_or_default();
        let script = PathBuf::from(&path);
        let out_dir = std::env::var("RUSTFILES_QA_OUT")
            .map(PathBuf::from)
            .unwrap_or_else(|_| script.parent().unwrap_or(Path::new(".")).to_path_buf());
        let _ = std::fs::create_dir_all(&out_dir);
        let report_path = out_dir.join("report.json");
        Some(Self {
            steps,
            index: 0,
            outcomes: Vec::new(),
            wait_until: None,
            report_path,
            out_dir,
            hits: Vec::new(),
            seen_ui: false,
            shot_rx: None,
            dpi_scale: 1.0,
        })
    }

    /// Push egui events before widgets and keyboard handling run.
    pub fn prepare(&mut self, app: &mut RustFilesApp, ctx: &egui::Context) {
        self.dpi_scale = ctx.pixels_per_point();
        if let Some(rx) = &self.shot_rx {
            match rx.try_recv() {
                Ok(shot) => {
                    self.dpi_scale = shot.dpi_scale;
                    self.shot_rx = None;
                    self.push(
                        "screenshot",
                        true,
                        &format!("wgpu-frame {}", shot.path),
                        "wgpu-frame",
                        Some((0.0, 0.0, shot.width as f32, shot.height as f32)),
                    );
                }
                Err(mpsc::TryRecvError::Empty) => {
                    ctx.request_repaint();
                    return;
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.shot_rx = None;
                    self.push(
                        "screenshot",
                        false,
                        "wgpu readback dropped",
                        "wgpu-frame",
                        None,
                    );
                }
            }
        }
        if let Some(until) = self.wait_until {
            if Instant::now() < until {
                ctx.request_repaint_after(Duration::from_millis(50));
                return;
            }
            self.wait_until = None;
        }
        if self.index >= self.steps.len() {
            return;
        }
        let step = match &self.steps[self.index] {
            Step::Click { label } => Step::Click {
                label: label.clone(),
            },
            Step::Key { key } => Step::Key { key: key.clone() },
            Step::Navigate { path } => Step::Navigate { path: path.clone() },
            Step::Context { path } => Step::Context { path: path.clone() },
            Step::Screenshot { path } => Step::Screenshot { path: path.clone() },
            Step::Wait { ms } => Step::Wait { ms: *ms },
        };
        match step {
            Step::Click { label } => {
                if !self.seen_ui {
                    ctx.request_repaint();
                    return;
                }
                let Some(hit) = self.hits.iter().find(|hit| hit.label == label) else {
                    self.push("click", false, "widget not found", "egui-event", None);
                    return;
                };
                let rect = hit.rect;
                let id = hit.id;
                inject_click(ctx, id, rect);
                let (x, y, w, h) = physical(ctx, rect);
                self.push(
                    "click",
                    true,
                    "egui AccessKitActionRequest + PointerButton",
                    "egui-event",
                    Some((x, y, w, h)),
                );
                ctx.request_repaint();
            }
            Step::Navigate { path } => {
                app.navigate(PathBuf::from(path));
                self.push("navigate", true, "navigated", "app", None);
                ctx.request_repaint();
            }
            Step::Key { key } => {
                let ok = push_key(ctx, &key);
                self.push(
                    "key",
                    ok,
                    if ok { "egui Key event" } else { "unknown key" },
                    "egui-event",
                    None,
                );
                ctx.request_repaint();
            }
            Step::Context { path } => {
                app.context_menu = Some((egui::pos2(240.0, 160.0), PathBuf::from(path)));
                let (x, y, _, _) = physical(
                    ctx,
                    egui::Rect::from_min_size(egui::pos2(240.0, 160.0), egui::Vec2::ZERO),
                );
                self.push(
                    "context",
                    true,
                    "context menu opened",
                    "app",
                    Some((x, y, 0.0, 0.0)),
                );
                ctx.request_repaint();
            }
            Step::Screenshot { path } => {
                let dest = self.place(&path);
                let note = dest.display().to_string();
                let (tx, rx) = mpsc::channel();
                let scale = ctx.pixels_per_point();
                ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(
                    egui::ScreenshotCallback::new(move |image| {
                        // eframe copied the offscreen render target, not the swapchain
                        // and not a desktop BitBlt/PrintWindow.
                        crate::app::save_screenshot(&dest, &image);
                        let _ = tx.send(Shot {
                            width: image.width(),
                            height: image.height(),
                            dpi_scale: scale,
                            path: note,
                        });
                    }),
                ));
                self.shot_rx = Some(rx);
                ctx.request_repaint();
            }
            Step::Wait { ms } => {
                self.wait_until = Some(Instant::now() + Duration::from_millis(ms));
                self.push("wait", true, "wait", "app", None);
                ctx.request_repaint_after(Duration::from_millis(ms));
            }
        }
    }

    pub fn finish_frame(&mut self) {
        self.hits = HITS.with(|hits| std::mem::take(&mut *hits.borrow_mut()));
        self.seen_ui = true;
    }

    fn place(&self, path: &str) -> PathBuf {
        let path = PathBuf::from(path);
        if path.is_absolute() && path.starts_with(&self.out_dir) {
            return path;
        }
        let name = path
            .file_name()
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("shot.png"));
        self.out_dir.join(name)
    }

    fn push(
        &mut self,
        op: &str,
        ok: bool,
        detail: &str,
        source: &str,
        rect: Option<(f32, f32, f32, f32)>,
    ) {
        let (x_px, y_px, w_px, h_px) = match rect {
            Some((x, y, w, h)) => (Some(x), Some(y), Some(w), Some(h)),
            None => (None, None, None, None),
        };
        self.outcomes.push(Outcome {
            op: op.to_string(),
            ok,
            detail: detail.to_string(),
            dpi_scale: self.dpi_scale,
            x_px,
            y_px,
            w_px,
            h_px,
            source: source.to_string(),
        });
        self.index += 1;
        if self.index >= self.steps.len() && self.shot_rx.is_none() {
            let _ = std::fs::write(
                &self.report_path,
                serde_json::to_string_pretty(&self.outcomes).unwrap_or_else(|_| "[]".into()),
            );
        }
    }
}

fn physical(ctx: &egui::Context, rect: egui::Rect) -> (f32, f32, f32, f32) {
    let scale = ctx.pixels_per_point();
    let rect = egui::Rect::from_min_size(rect.min * scale, rect.size() * scale);
    (rect.min.x, rect.min.y, rect.width(), rect.height())
}

fn inject_click(ctx: &egui::Context, id: egui::Id, rect: egui::Rect) {
    let pos = rect.center();
    ctx.input_mut(|input| {
        input.events.push(egui::Event::PointerMoved(pos));
        input.events.push(egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed: true,
            modifiers: egui::Modifiers::default(),
        });
        input.events.push(egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: egui::Modifiers::default(),
        });
        input.events.push(egui::Event::AccessKitActionRequest(
            egui::accesskit::ActionRequest {
                action: egui::accesskit::Action::Click,
                target_tree: egui::accesskit::TreeId::ROOT,
                target_node: id.accesskit_id(),
                data: None,
            },
        ));
    });
}

fn push_key(ctx: &egui::Context, spec: &str) -> bool {
    let mut modifiers = egui::Modifiers::default();
    let mut key_name = spec;
    if let Some((mods, key)) = spec.rsplit_once('+') {
        key_name = key;
        for part in mods.split('+') {
            match part.to_ascii_lowercase().as_str() {
                "alt" => modifiers.alt = true,
                "ctrl" | "control" => {
                    modifiers.ctrl = true;
                    modifiers.command = true;
                }
                "shift" => modifiers.shift = true,
                _ => return false,
            }
        }
    }
    let Some(key) = parse_key(key_name) else {
        return false;
    };
    ctx.input_mut(|input| {
        input.events.push(egui::Event::Key {
            key,
            physical_key: Some(key),
            pressed: true,
            repeat: false,
            modifiers,
        });
    });
    true
}

fn parse_key(name: &str) -> Option<egui::Key> {
    Some(match name {
        "F5" => egui::Key::F5,
        "F2" => egui::Key::F2,
        "F1" => egui::Key::F1,
        "Delete" => egui::Key::Delete,
        "Escape" => egui::Key::Escape,
        "Enter" => egui::Key::Enter,
        "Backspace" => egui::Key::Backspace,
        "Tab" => egui::Key::Tab,
        "Up" => egui::Key::ArrowUp,
        "Down" => egui::Key::ArrowDown,
        "Left" => egui::Key::ArrowLeft,
        "Right" => egui::Key::ArrowRight,
        other if other.chars().count() == 1 => egui::Key::from_name(other)?,
        _ => return None,
    })
}
