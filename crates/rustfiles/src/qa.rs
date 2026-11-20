//! Replay a JSON action list and write whether each step did something.
//!
//! `--qa-script steps.json` reads an array of objects:
//! `{ "op": "click", "label": "Home" }`,
//! `{ "op": "navigate", "path": "C:\\Users" }`,
//! `{ "op": "key", "key": "F5" }`,
//! `{ "op": "context", "path": "C:\\Users" }`,
//! `{ "op": "screenshot", "path": "home.png" }`,
//! `{ "op": "wait", "ms": 200 }`.
//!
//! The report is written next to the script as `<name>.report.json`.

use std::cell::RefCell;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::app::RustFilesApp;
use crate::nav::Action;

thread_local! {
    static FORCE_LABEL: RefCell<Option<String>> = const { RefCell::new(None) };
}

pub fn consume(label: &str, clicked: bool) -> bool {
    if clicked {
        return true;
    }
    FORCE_LABEL.with(|slot| {
        let hit = slot.borrow().as_deref() == Some(label);
        if hit {
            *slot.borrow_mut() = None;
        }
        hit
    })
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
}

pub struct Player {
    steps: Vec<Step>,
    index: usize,
    outcomes: Vec<Outcome>,
    wait_until: Option<Instant>,
    armed: bool,
    report_path: PathBuf,
    shot_pending: bool,
}

impl Player {
    pub fn load() -> Option<Self> {
        let path = std::env::var("RUSTFILES_QA").ok()?;
        let text = std::fs::read_to_string(&path).ok()?;
        let steps: Vec<Step> = serde_json::from_str(&text).unwrap_or_default();
        let report_path = PathBuf::from(path).with_extension("report.json");
        Some(Self {
            steps,
            index: 0,
            outcomes: Vec::new(),
            wait_until: None,
            armed: false,
            report_path,
            shot_pending: false,
        })
    }

    pub fn tick(&mut self, app: &mut RustFilesApp, ctx: &egui::Context) {
        if let Some(until) = self.wait_until {
            if Instant::now() < until {
                ctx.request_repaint_after(Duration::from_millis(50));
                return;
            }
            self.wait_until = None;
        }
        if self.shot_pending {
            return;
        }
        if self.armed {
            let missed = FORCE_LABEL.with(|slot| slot.borrow().is_some());
            if missed {
                FORCE_LABEL.with(|slot| *slot.borrow_mut() = None);
                self.finish_step(false, "widget not found");
            } else {
                self.finish_step(true, "clicked");
            }
            self.armed = false;
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
                FORCE_LABEL.with(|slot| *slot.borrow_mut() = Some(label));
                self.armed = true;
                ctx.request_repaint();
            }
            Step::Navigate { path } => {
                app.navigate(PathBuf::from(path));
                self.finish_step(true, "navigated");
                ctx.request_repaint();
            }
            Step::Key { key } => {
                let ok = match key.as_str() {
                    "F5" => {
                        app.perform(Action::Refresh);
                        true
                    }
                    "Delete" => {
                        app.perform(Action::Trash);
                        true
                    }
                    "Escape" => {
                        app.perform(Action::Escape);
                        true
                    }
                    "Alt+Up" => {
                        app.perform(Action::Up);
                        true
                    }
                    _ => false,
                };
                self.finish_step(ok, if ok { "key" } else { "unknown key" });
                ctx.request_repaint();
            }
            Step::Context { path } => {
                app.context_menu = Some((egui::pos2(200.0, 200.0), PathBuf::from(path)));
                self.finish_step(true, "context menu opened");
                ctx.request_repaint();
            }
            Step::Screenshot { path } => {
                let path = PathBuf::from(path);
                let report_note = path.display().to_string();
                self.shot_pending = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(
                    egui::ScreenshotCallback::new(move |image| {
                        crate::app::save_screenshot(&path, &image);
                    }),
                ));
                self.finish_step(true, &report_note);
                ctx.request_repaint();
                self.shot_pending = false;
            }
            Step::Wait { ms } => {
                self.wait_until = Some(Instant::now() + Duration::from_millis(ms));
                self.finish_step(true, "wait");
                ctx.request_repaint_after(Duration::from_millis(ms));
            }
        }
    }

    fn finish_step(&mut self, ok: bool, detail: &str) {
        let op = self
            .steps
            .get(self.index)
            .map(|step| match step {
                Step::Click { .. } => "click",
                Step::Key { .. } => "key",
                Step::Navigate { .. } => "navigate",
                Step::Context { .. } => "context",
                Step::Screenshot { .. } => "screenshot",
                Step::Wait { .. } => "wait",
            })
            .unwrap_or("done")
            .to_string();
        self.outcomes.push(Outcome {
            op,
            ok,
            detail: detail.to_string(),
        });
        self.index += 1;
        if self.index >= self.steps.len() {
            let _ = std::fs::write(
                &self.report_path,
                serde_json::to_string_pretty(&self.outcomes).unwrap_or_else(|_| "[]".into()),
            );
        }
    }
}
