//! Files-style shell: title-bar tabs, navigation, command bar, sidebar, and home.
use std::path::PathBuf;
use std::time::UNIX_EPOCH;

use egui::{
    pos2, Align, Color32, CornerRadius, FontId, Layout, Rect, RichText, ScrollArea, Sense, Stroke,
    Ui, UiBuilder, Vec2,
};
use rustfiles_core::{format_size, FileRow, SortColumn};

use crate::app::{format_modified, type_name, Pane, RustFilesApp};
use crate::icons::{self, icon_font};
use crate::nav::{self, Action};
use crate::platform::{self, Places};
use crate::theme::{self, FilesPalette};
use crate::worker::Command;

const TITLE_H: f32 = 48.0;
const NAV_H: f32 = 48.0;
const COMMAND_H: f32 = 48.0;
const STATUS_H: f32 = 32.0;
const SIDE_W: f32 = 292.0;
const PREVIEW_W: f32 = 300.0;

impl RustFilesApp {
    pub fn draw(&mut self, ui: &mut Ui) {
        if ui.style().visuals.dark_mode != self.dark {
            crate::app::apply_theme(ui.ctx(), self.dark);
        }
        let palette = theme::palette(self.dark);
        let full = ui.max_rect();
        let backdrop = if self.mica_applied {
            Color32::TRANSPARENT
        } else {
            palette.window
        };
        ui.painter().rect_filled(full, CornerRadius::ZERO, backdrop);
        let title = Rect::from_min_size(full.min, Vec2::new(full.width(), TITLE_H));
        let nav = Rect::from_min_size(
            pos2(full.left(), title.bottom()),
            Vec2::new(full.width(), NAV_H),
        );
        let command = Rect::from_min_size(
            pos2(full.left(), nav.bottom()),
            Vec2::new(full.width(), COMMAND_H),
        );
        let status = Rect::from_min_max(
            pos2(full.left(), full.bottom() - STATUS_H),
            full.right_bottom(),
        );
        let body = Rect::from_min_max(
            pos2(full.left(), command.bottom()),
            pos2(full.right(), status.top()),
        );
        let side = Rect::from_min_size(body.min, Vec2::new(SIDE_W, body.height()));
        let preview_w = if self.show_preview && self.pane == Pane::Files && !self.tab().home {
            PREVIEW_W
        } else {
            0.0
        };
        let content = Rect::from_min_max(
            pos2(side.right(), body.top()),
            pos2(body.right() - preview_w, body.bottom()),
        );
        let preview = Rect::from_min_max(pos2(content.right(), body.top()), body.right_bottom());
        let side_fill = if self.mica_applied {
            Color32::from_rgba_unmultiplied(0, 0, 0, 72)
        } else {
            palette.sidebar
        };
        let file_fill = if self.mica_applied {
            Color32::TRANSPARENT
        } else {
            palette.file_area
        };
        ui.painter()
            .rect_filled(side, CornerRadius::ZERO, side_fill);
        ui.painter()
            .rect_filled(content, CornerRadius::ZERO, file_fill);
        if preview_w > 0.0 {
            ui.painter()
                .rect_filled(preview, CornerRadius::ZERO, palette.sidebar);
            ui.painter().vline(
                preview.left(),
                preview.y_range(),
                Stroke::new(1.0, palette.stroke),
            );
        }
        ui.painter().vline(
            side.right(),
            side.y_range(),
            Stroke::new(1.0, palette.stroke),
        );
        ui.painter().hline(
            full.x_range(),
            command.bottom(),
            Stroke::new(1.0, palette.stroke),
        );
        ui.painter().hline(
            full.x_range(),
            status.top(),
            Stroke::new(1.0, palette.stroke),
        );
        self.draw_title(ui, title, palette);
        self.draw_nav(ui, nav, palette);
        self.draw_command(ui, command, palette);
        self.draw_sidebar(ui, side, palette);
        self.draw_content(ui, content, palette);
        if preview_w > 0.0 {
            self.draw_preview(ui, preview, palette);
        }
        self.draw_status(ui, status, palette);
        self.draw_dialogs(ui);
    }

    fn draw_title(&mut self, ui: &mut Ui, rect: Rect, palette: FilesPalette) {
        let caption_w = 138.0;
        let tabs = Rect::from_min_max(rect.min, pos2(rect.right() - caption_w, rect.bottom()));
        let mut child = ui.new_child(
            UiBuilder::new()
                .max_rect(tabs)
                .layout(Layout::left_to_right(Align::Center)),
        );
        child.add_space(8.0);
        let mut select = None;
        let mut close = None;
        let mut icon_paths = Vec::new();
        for (index, tab) in self.tabs.iter().enumerate() {
            let title = tab_title(tab, &self.tr);
            let icon = if tab.home {
                None
            } else {
                self.icons_tex.get(&tab.path).cloned()
            };
            let path = if tab.home {
                None
            } else {
                Some(tab.path.clone())
            };
            let (response, close_hit) = tab_chip(
                &mut child,
                &title,
                index == self.current,
                palette,
                icon.as_ref(),
                if tab.home { icons::HOME } else { icons::FOLDER },
            );
            if let Some(path) = path {
                icon_paths.push(path);
            }
            if response.clicked() {
                select = Some(index);
            }
            if close_hit {
                close = Some(index);
            }
        }
        for path in icon_paths {
            self.request_icon(&path, 32);
        }
        if chip_button(&mut child, icons::ADD, palette).clicked() {
            self.new_tab_from_home();
        }
        let drag = child.interact(
            tabs,
            egui::Id::unique("title-drag"),
            Sense::click_and_drag(),
        );
        if drag.drag_started() {
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::StartDrag);
        }
        if let Some(index) = select {
            self.current = index;
            self.pane = Pane::Files;
        }
        if let Some(index) = close {
            self.current = index;
            self.perform(Action::CloseTab);
        }
        let maximized = ui.input(|i| i.viewport().maximized.unwrap_or(false));
        let buttons = [
            (icons::SUBTRACT, "min"),
            (
                if maximized {
                    icons::RESTORE
                } else {
                    icons::MAXIMIZE
                },
                "max",
            ),
            (icons::DISMISS, "close"),
        ];
        for (index, (glyph, id)) in buttons.iter().enumerate() {
            let button = Rect::from_min_size(
                pos2(rect.right() - caption_w + index as f32 * 46.0, rect.top()),
                Vec2::new(46.0, rect.height()),
            );
            let response = ui.interact(button, egui::Id::unique(*id), Sense::click());
            let close = *id == "close";
            if response.hovered() {
                ui.painter().rect_filled(
                    button,
                    CornerRadius::ZERO,
                    if close {
                        Color32::from_rgb(196, 43, 28)
                    } else {
                        palette.hover
                    },
                );
            }
            let color = if close && response.hovered() {
                Color32::WHITE
            } else {
                palette.text
            };
            ui.painter().text(
                button.center(),
                egui::Align2::CENTER_CENTER,
                icons::glyph(*glyph),
                icon_font(12.0),
                color,
            );
            if response.clicked() {
                let cmd = match *id {
                    "min" => egui::ViewportCommand::Minimized(true),
                    "max" => egui::ViewportCommand::Maximized(!maximized),
                    _ => egui::ViewportCommand::Close,
                };
                ui.ctx().send_viewport_cmd(cmd);
            }
        }
    }

    fn draw_nav(&mut self, ui: &mut Ui, rect: Rect, palette: FilesPalette) {
        let mut bar = ui.new_child(
            UiBuilder::new()
                .max_rect(rect)
                .layout(Layout::left_to_right(Align::Center)),
        );
        bar.add_space(8.0);
        if nav_button(
            &mut bar,
            icons::ARROW_LEFT,
            self.tab().hist_pos > 0,
            palette,
        )
        .clicked()
        {
            self.perform(Action::Back);
        }
        if nav_button(
            &mut bar,
            icons::ARROW_RIGHT,
            self.tab().hist_pos + 1 < self.tab().history.len(),
            palette,
        )
        .clicked()
        {
            self.perform(Action::Forward);
        }
        if nav_button(&mut bar, icons::ARROW_UP, !self.tab().home, palette).clicked() {
            if let Some(parent) = nav::parent_dir(&self.tab().path) {
                self.navigate(parent);
            } else {
                self.show_home();
            }
        }
        if nav_button(&mut bar, icons::ARROW_SYNC, true, palette).clicked() {
            self.perform(Action::Refresh);
        }
        bar.add_space(6.0);
        let search_w = 240.0;
        let address_w = (bar.available_width() - search_w - 16.0).max(180.0);
        let (address, _) = bar.allocate_exact_size(Vec2::new(address_w, 36.0), Sense::hover());
        bar.painter().rect(
            address,
            CornerRadius::same(4),
            palette.address,
            Stroke::new(1.0, palette.stroke),
            egui::StrokeKind::Inside,
        );
        if let Some(mut text) = self.path_edit.clone() {
            let mut edit = bar.new_child(UiBuilder::new().max_rect(address.shrink(6.0)));
            let response = edit.add(
                egui::TextEdit::singleline(&mut text)
                    .desired_width(address.width() - 16.0)
                    .frame(egui::Frame::NONE),
            );
            if self.focus_path {
                response.request_focus();
                self.focus_path = false;
            }
            if response.lost_focus() && edit.input(|i| i.key_pressed(egui::Key::Enter)) {
                self.navigate(PathBuf::from(&text));
                self.path_edit = None;
            } else if response.changed() {
                self.path_edit = Some(text);
            }
        } else {
            let mut crumbs = bar.new_child(
                UiBuilder::new()
                    .max_rect(address.shrink2(Vec2::new(10.0, 0.0)))
                    .layout(Layout::left_to_right(Align::Center)),
            );
            if crumb_chip(&mut crumbs, Some(icons::HOME), self.tr.home, palette).clicked() {
                self.show_home();
            }
            if !self.tab().home {
                let parts = nav::breadcrumbs(&self.tab().path);
                for (label, path) in &parts {
                    crumbs.label(
                        RichText::new(icons::glyph(icons::CHEVRON_RIGHT))
                            .font(icon_font(12.0))
                            .color(palette.text_secondary),
                    );
                    if crumb_chip(&mut crumbs, None, label, palette).clicked() {
                        self.navigate(path.clone());
                        return;
                    }
                }
            }
            if bar
                .interact(address, egui::Id::unique("address-edit"), Sense::click())
                .double_clicked()
            {
                self.perform(Action::FocusPath);
            }
        }
        bar.add_space(8.0);
        let (search, _) = bar.allocate_exact_size(Vec2::new(search_w, 36.0), Sense::hover());
        bar.painter().rect(
            search,
            CornerRadius::same(16),
            palette.address,
            Stroke::new(1.0, palette.stroke),
            egui::StrokeKind::Inside,
        );
        let mut search_ui = bar.new_child(
            UiBuilder::new()
                .max_rect(search.shrink2(Vec2::new(12.0, 4.0)))
                .layout(Layout::left_to_right(Align::Center)),
        );
        search_ui.label(
            RichText::new(icons::glyph(icons::SEARCH))
                .font(icon_font(14.0))
                .color(palette.text_secondary),
        );
        let mut query = self.tab().search.clone();
        let response = search_ui.add(
            egui::TextEdit::singleline(&mut query)
                .hint_text(self.tr.search)
                .desired_width(search_w - 44.0)
                .frame(egui::Frame::NONE),
        );
        if self.focus_search {
            response.request_focus();
            self.focus_search = false;
        }
        if response.changed() {
            self.tab_mut().search = query;
            self.tab_mut().home = false;
            self.request_search();
        }
    }

    fn draw_command(&mut self, ui: &mut Ui, rect: Rect, palette: FilesPalette) {
        let mut bar = ui.new_child(
            UiBuilder::new()
                .max_rect(rect)
                .layout(Layout::left_to_right(Align::Center)),
        );
        bar.add_space(8.0);
        let new_button = command_button(&mut bar, icons::ADD, self.tr.new, palette);
        if new_button.clicked() {
            self.new_menu = !self.new_menu;
        }
        let new_pos = new_button.rect.left_bottom();
        for (glyph, action) in [
            (icons::CUT, Action::Cut),
            (icons::COPY, Action::Copy),
            (icons::PASTE, Action::Paste),
            (icons::RENAME, Action::Rename),
            (icons::SHARE, Action::Help),
            (icons::DELETE, Action::Trash),
        ] {
            if icon_button(&mut bar, glyph, palette).clicked() {
                if glyph == icons::SHARE {
                    self.status = self.tr.share_unavailable.into();
                } else {
                    self.perform(action);
                }
            }
        }
        bar.with_layout(Layout::right_to_left(Align::Center), |bar| {
            bar.add_space(8.0);
            if icon_button(bar, icons::DOCUMENT, palette).clicked() {
                self.show_preview = !self.show_preview;
                self.save_config();
            }
            if icon_button(
                bar,
                if self.tab().details {
                    icons::GRID
                } else {
                    icons::LIST
                },
                palette,
            )
            .clicked()
            {
                self.tab_mut().details = !self.tab().details;
            }
            if icon_button(bar, icons::SORT, palette).clicked() {
                let tab = self.tab_mut();
                tab.sort = match tab.sort {
                    SortColumn::Name => SortColumn::Modified,
                    SortColumn::Modified => SortColumn::Type,
                    SortColumn::Type => SortColumn::Size,
                    SortColumn::Size => SortColumn::Name,
                };
                self.sort_visible();
            }
            if icon_button(bar, icons::INFO, palette).clicked() {
                self.perform(Action::Analyzer);
            }
        });
        if self.new_menu {
            egui::Area::new(egui::Id::unique("new-menu"))
                .fixed_pos(new_pos)
                .order(egui::Order::Foreground)
                .show(ui.ctx(), |ui| {
                    egui::Frame::popup(ui.style()).show(ui, |ui| {
                        if ui.button(self.tr.folder).clicked() {
                            self.new_menu = false;
                            self.perform(Action::NewFolder);
                        }
                        if ui.button(self.tr.text_file).clicked() {
                            self.new_menu = false;
                            let dest = self.tab().path.clone();
                            let name = self.tr.new_file_name.to_string();
                            let _ = self.commands.send(Command::NewFile { dest, name });
                        }
                    });
                });
        }
    }

    fn draw_sidebar(&mut self, ui: &mut Ui, rect: Rect, palette: FilesPalette) {
        let settings_h = 44.0;
        let scroll_rect =
            Rect::from_min_max(rect.min, pos2(rect.right(), rect.bottom() - settings_h));
        let settings_rect =
            Rect::from_min_max(pos2(rect.left(), scroll_rect.bottom()), rect.right_bottom());
        let mut side =
            ui.new_child(UiBuilder::new().max_rect(scroll_rect.shrink2(Vec2::new(8.0, 6.0))));
        ScrollArea::vertical()
            .id_salt("sidebar")
            .show(&mut side, |side| {
                if nav_row(side, icons::HOME, self.tr.home, self.tab().home, palette).clicked() {
                    self.show_home();
                }
                let places = self.places.clone();
                let favorites = [
                    (icons::DESKTOP, self.tr.desktop, places.desktop.clone()),
                    (
                        icons::ARROW_DOWNLOAD,
                        self.tr.downloads,
                        places.downloads.clone(),
                    ),
                    (icons::DOCUMENT, self.tr.documents, places.documents.clone()),
                    (icons::IMAGE, self.tr.pictures, places.pictures.clone()),
                    (icons::MUSIC, self.tr.music, places.music.clone()),
                    (icons::VIDEO, self.tr.videos, places.videos.clone()),
                ];
                if section_header(side, self.tr.favorites, &mut self.open_fav, palette) {
                    for (glyph, label, path) in favorites {
                        if nav_entry(side, self, Some(&path), glyph, label, palette).clicked() {
                            self.navigate(path);
                        }
                    }
                    if nav_row(side, icons::PIN, self.tr.pin, false, palette).clicked()
                        && !self.tab().home
                    {
                        let path = self.tab().path.clone();
                        if !self.favorites.iter().any(|have| have == &path) {
                            self.favorites.push(path);
                            self.save_config();
                        }
                    }
                    for path in self.favorites.clone() {
                        if places_contains(&places, &path) {
                            continue;
                        }
                        let name = path
                            .file_name()
                            .map(|n| n.to_string_lossy().into_owned())
                            .unwrap_or_else(|| path.display().to_string());
                        if nav_entry(side, self, Some(&path), icons::STAR, &name, palette).clicked()
                        {
                            self.navigate(path);
                        }
                    }
                }
                if section_header(side, self.tr.drives, &mut self.open_drives, palette) {
                    for volume in self.volumes.clone() {
                        let label = if volume.label.is_empty() {
                            volume.mount.clone()
                        } else {
                            volume.label.clone()
                        };
                        let mount = PathBuf::from(&volume.mount);
                        if nav_entry(side, self, Some(&mount), icons::HARD_DRIVE, &label, palette)
                            .clicked()
                        {
                            self.navigate(mount);
                        }
                    }
                }
                if section_header(side, self.tr.cloud, &mut self.open_cloud, palette)
                    && nav_row(side, icons::GLOBE, self.tr.cloud, false, palette).clicked()
                {
                    self.status = self.tr.cloud_unavailable.into();
                }
                if section_header(side, self.tr.network, &mut self.open_network, palette)
                    && nav_row(side, icons::GLOBE, self.tr.network, false, palette).clicked()
                {
                    self.status = self.tr.network_unavailable.into();
                }
                if section_header(side, self.tr.wsl, &mut self.open_wsl, palette)
                    && nav_row(side, icons::DESKTOP, self.tr.wsl, false, palette).clicked()
                {
                    self.status = self.tr.wsl_unavailable.into();
                }
                if section_header(side, self.tr.tags, &mut self.open_tags, palette)
                    && nav_row(side, icons::TAG, self.tr.tags, false, palette).clicked()
                {
                    self.status = self.tr.tags_unavailable.into();
                }
                if nav_row(side, icons::RECYCLE, self.tr.recycle, false, palette).clicked() {
                    self.open_trash();
                }
            });
        let mut settings =
            ui.new_child(UiBuilder::new().max_rect(settings_rect.shrink2(Vec2::new(8.0, 4.0))));
        if nav_row(&mut settings, icons::MORE, self.tr.settings, false, palette).clicked() {
            self.settings_menu = !self.settings_menu;
        }
        if self.settings_menu {
            egui::Area::new(egui::Id::unique("settings-menu"))
                .fixed_pos(settings_rect.left_top() - Vec2::new(0.0, 120.0))
                .order(egui::Order::Foreground)
                .show(ui.ctx(), |ui| {
                    egui::Frame::popup(ui.style()).show(ui, |ui| {
                        if ui
                            .button(if self.dark {
                                self.tr.light
                            } else {
                                self.tr.dark
                            })
                            .clicked()
                        {
                            self.dark = !self.dark;
                            self.mica_applied = false;
                            self.save_config();
                        }
                        if ui
                            .button(if self.show_hidden {
                                self.tr.hide_hidden
                            } else {
                                self.tr.show_hidden
                            })
                            .clicked()
                        {
                            self.show_hidden = !self.show_hidden;
                            self.save_config();
                            self.request_list();
                        }
                        if ui.button(self.tr.reindex).clicked() {
                            let _ = self.commands.send(Command::Rescan);
                            self.indexing = true;
                            self.settings_menu = false;
                        }
                    });
                });
        }
    }

    fn draw_content(&mut self, ui: &mut Ui, rect: Rect, palette: FilesPalette) {
        let mut content =
            ui.new_child(UiBuilder::new().max_rect(rect.shrink2(Vec2::new(16.0, 10.0))));
        if self.pane == Pane::Analyzer {
            self.draw_analyzer(&mut content, palette);
            return;
        }
        if self.tab().home && self.tab().search.is_empty() {
            self.draw_home(&mut content, palette);
            return;
        }
        if !self.tab().search.is_empty() {
            content.label(
                RichText::new(format!("Search results for \"{}\"", self.tab().search))
                    .color(palette.text)
                    .strong(),
            );
            content.add_space(8.0);
        }
        if self.tab().details {
            self.draw_details(&mut content, palette);
        } else {
            self.draw_grid(&mut content, palette);
        }
    }

    fn draw_home(&mut self, ui: &mut Ui, palette: FilesPalette) {
        ui.label(
            RichText::new(self.tr.drives)
                .color(palette.text)
                .strong()
                .size(18.0),
        );
        ui.add_space(10.0);
        ui.horizontal_wrapped(|ui| {
            for volume in self.volumes.clone() {
                let (total, free) =
                    platform::disk_usage(std::path::Path::new(&volume.mount)).unwrap_or((0, 0));
                if drive_card(
                    ui,
                    &volume.label,
                    &volume.mount,
                    total.saturating_sub(free),
                    total,
                    palette,
                )
                .clicked()
                {
                    self.navigate(PathBuf::from(volume.mount));
                }
            }
        });
        ui.add_space(22.0);
        ui.label(
            RichText::new(self.tr.recent)
                .color(palette.text)
                .strong()
                .size(18.0),
        );
        ui.add_space(8.0);
        let recent = recent_files(&self.places);
        if recent.is_empty() {
            ui.label(RichText::new(self.tr.no_recent).color(palette.text_secondary));
            return;
        }
        for row in recent {
            self.request_icon(&row.path, 32);
            let icon = self.icons_tex.get(&row.path).cloned();
            if file_line(
                ui,
                icons::type_glyph(row.is_dir, &row.name),
                &row.name,
                &format_modified(row.modified),
                palette,
                icon.as_ref(),
            )
            .clicked()
            {
                if row.is_dir {
                    self.navigate(row.path);
                } else if let Err(err) = platform::open_path(&row.path) {
                    self.status = err.to_string();
                }
            }
        }
    }

    fn draw_details(&mut self, ui: &mut Ui, palette: FilesPalette) {
        let width = ui.available_width();
        let name_w = (width - 340.0).max(160.0);
        ui.horizontal(|ui| {
            column_header(ui, self.tr.name, name_w, SortColumn::Name, self, palette);
            column_header(
                ui,
                self.tr.modified,
                170.0,
                SortColumn::Modified,
                self,
                palette,
            );
            column_header(ui, self.tr.kind, 90.0, SortColumn::Type, self, palette);
            column_header(ui, self.tr.size, 90.0, SortColumn::Size, self, palette);
        });
        let len = self.visible_rows().len();
        if len == 0 {
            ui.add_space(28.0);
            ui.label(
                RichText::new(if self.tab().search.is_empty() {
                    self.tr.empty
                } else {
                    self.tr.no_matches
                })
                .color(palette.text_secondary),
            );
            return;
        }
        let mut clicked = None;
        let mut opened = None;
        ScrollArea::vertical()
            .id_salt("details")
            .auto_shrink([false, false])
            .show_rows(ui, 32.0, len, |ui, range| {
                for index in range {
                    let row = &self.visible_rows()[index];
                    let selected = self.tab().selected.iter().any(|path| path == &row.path);
                    let name = row.name.clone();
                    let modified = format_modified(row.modified);
                    let kind = type_name(row);
                    let size = if row.is_dir && row.size == 0 {
                        String::new()
                    } else {
                        format_size(row.size)
                    };
                    let glyph = icons::type_glyph(row.is_dir, &name);
                    let path = row.path.clone();
                    let is_dir = row.is_dir;
                    let (rect, response) = ui
                        .allocate_exact_size(Vec2::new(ui.available_width(), 36.0), Sense::click());
                    if selected || response.hovered() {
                        ui.painter().rect_filled(
                            rect,
                            CornerRadius::same(4),
                            if selected {
                                palette.selected
                            } else {
                                palette.hover
                            },
                        );
                    }
                    let mut line = ui.new_child(
                        UiBuilder::new()
                            .max_rect(rect)
                            .layout(Layout::left_to_right(Align::Center)),
                    );
                    line.add_space(6.0);
                    paint_type_icon(&mut line, &path, is_dir, glyph, palette, self);
                    left_cell(&mut line, name_w - 28.0, &name, palette.text, 13.0);
                    left_cell(&mut line, 150.0, &modified, palette.text_secondary, 12.0);
                    left_cell(&mut line, 80.0, &kind, palette.text_secondary, 12.0);
                    left_cell(&mut line, 80.0, &size, palette.text_secondary, 12.0);
                    if response.clicked() {
                        clicked = Some((index, ui.input(|i| i.modifiers)));
                    }
                    if response.double_clicked() {
                        opened = Some(index);
                    }
                }
            });
        if let Some((index, mods)) = clicked {
            self.select_row(index, mods);
        }
        if let Some(index) = opened {
            self.select_row(index, egui::Modifiers::NONE);
            self.open_selection();
        }
    }

    fn draw_grid(&mut self, ui: &mut Ui, palette: FilesPalette) {
        let cols = ((ui.available_width() / 156.0).floor() as usize).max(1);
        let len = self.visible_rows().len();
        if len == 0 {
            ui.label(RichText::new(self.tr.empty).color(palette.text_secondary));
            return;
        }
        let mut clicked = None;
        let mut opened = None;
        ScrollArea::vertical()
            .id_salt("icons")
            .auto_shrink([false, false])
            .show_rows(ui, 156.0, len.div_ceil(cols), |ui, range| {
                for row_index in range {
                    ui.horizontal(|ui| {
                        for col in 0..cols {
                            let index = row_index * cols + col;
                            if index >= len {
                                break;
                            }
                            let (path, name, is_dir, selected) = {
                                let row = &self.visible_rows()[index];
                                let selected =
                                    self.tab().selected.iter().any(|have| have == &row.path);
                                (row.path.clone(), row.name.clone(), row.is_dir, selected)
                            };
                            let response =
                                ui.allocate_response(Vec2::new(148.0, 148.0), Sense::click());
                            if selected || response.hovered() {
                                ui.painter().rect_filled(
                                    response.rect,
                                    CornerRadius::same(6),
                                    if selected {
                                        palette.selected
                                    } else {
                                        palette.hover
                                    },
                                );
                            }
                            let glyph = icons::type_glyph(is_dir, &name);
                            self.request_icon(&path, 128);
                            self.request_thumb(&path, is_dir);
                            if let Some(texture) =
                                self.thumbs.get(&path).or_else(|| self.icons_tex.get(&path))
                            {
                                let image = Rect::from_center_size(
                                    response.rect.center() - Vec2::new(18.0, 0.0),
                                    Vec2::splat(72.0),
                                );
                                ui.painter().image(
                                    texture.id(),
                                    image,
                                    Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                                    Color32::WHITE,
                                );
                            } else {
                                ui.painter().text(
                                    response.rect.center() - Vec2::new(0.0, 14.0),
                                    egui::Align2::CENTER_CENTER,
                                    icons::glyph(glyph),
                                    icon_font(48.0),
                                    palette.accent,
                                );
                            }
                            let kind = if is_dir { self.tr.folder_kind } else { "" };
                            ui.painter().text(
                                response.rect.center() + Vec2::new(0.0, 36.0),
                                egui::Align2::CENTER_CENTER,
                                truncate(&name, 16),
                                FontId::proportional(13.0),
                                palette.text,
                            );
                            if !kind.is_empty() {
                                ui.painter().text(
                                    response.rect.center() + Vec2::new(0.0, 54.0),
                                    egui::Align2::CENTER_CENTER,
                                    kind,
                                    FontId::proportional(12.0),
                                    palette.text_secondary,
                                );
                            }
                            if response.clicked() {
                                clicked = Some((index, ui.input(|i| i.modifiers)));
                            }
                            if response.double_clicked() {
                                opened = Some(index);
                            }
                        }
                    });
                }
            });
        if let Some((index, mods)) = clicked {
            self.select_row(index, mods);
        }
        if let Some(index) = opened {
            self.select_row(index, egui::Modifiers::NONE);
            self.open_selection();
        }
    }

    fn draw_preview(&mut self, ui: &mut Ui, rect: Rect, palette: FilesPalette) {
        let mut pane = ui.new_child(UiBuilder::new().max_rect(rect.shrink2(Vec2::new(16.0, 12.0))));
        pane.label(
            RichText::new(self.tr.details_title)
                .color(palette.text)
                .strong()
                .size(16.0),
        );
        pane.add_space(8.0);
        let selected = self.tab().selected.clone();
        if selected.len() != 1 {
            pane.label(
                RichText::new(if selected.is_empty() {
                    self.tr.select_one
                } else {
                    self.tr.multiple
                })
                .color(palette.text_secondary),
            );
            return;
        }
        let path = &selected[0];
        let row = self
            .visible_rows()
            .iter()
            .find(|row| &row.path == path)
            .cloned();
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let is_dir = row.as_ref().map(|row| row.is_dir).unwrap_or(false);
        pane.label(
            RichText::new(icons::glyph(icons::type_glyph(is_dir, &name)))
                .font(icon_font(40.0))
                .color(palette.accent),
        );
        pane.label(RichText::new(&name).color(palette.text).strong());
        if let Some(row) = &row {
            detail_line(&mut pane, "Type", &type_name(row), palette);
            detail_line(
                &mut pane,
                "Size",
                &if row.is_dir && row.size == 0 {
                    "—".into()
                } else {
                    format_size(row.size)
                },
                palette,
            );
            detail_line(
                &mut pane,
                "Modified",
                &format_modified(row.modified),
                palette,
            );
        }
        detail_line(&mut pane, "Path", &path.display().to_string(), palette);
        if let Some(texture) = &self.preview_image {
            if self.preview_path == *path {
                pane.add_space(8.0);
                pane.image((texture.id(), texture.size_vec2()));
            }
        } else if self.preview_path == *path && !self.preview_text.is_empty() {
            ScrollArea::vertical()
                .id_salt("preview-text")
                .max_height(180.0)
                .show(&mut pane, |pane| {
                    pane.label(
                        RichText::new(&self.preview_text)
                            .color(palette.text_secondary)
                            .size(12.0),
                    );
                });
        }
    }

    fn draw_analyzer(&mut self, ui: &mut Ui, palette: FilesPalette) {
        ui.horizontal(|ui| {
            if command_button(ui, icons::ARROW_UP, "Up", palette).clicked() {
                if let Some(parent) = nav::parent_dir(&self.analyzer_path) {
                    self.request_analyze(parent);
                }
            }
            if command_button(ui, icons::FOLDER, "Files", palette).clicked() {
                self.pane = Pane::Files;
            }
            if command_button(
                ui,
                icons::GRID,
                if self.sunburst { "Treemap" } else { "Sunburst" },
                palette,
            )
            .clicked()
            {
                self.sunburst = !self.sunburst;
            }
            ui.label(
                RichText::new(format!(
                    "{}   {}",
                    self.analyzer_path.display(),
                    format_size(self.analyzer_total)
                ))
                .color(palette.text)
                .strong(),
            );
        });
        ui.add_space(8.0);
        let cells = self.analyzer_cells.clone();
        let rows = self.analyzer_rows.clone();
        ui.horizontal_top(|ui| {
            ui.vertical(|ui| {
                ui.set_width(280.0);
                ScrollArea::vertical().id_salt("analyzer").show(ui, |ui| {
                    for row in &rows {
                        let label = format!(
                            "{}   {}",
                            row.name,
                            if row.size == 0 {
                                String::new()
                            } else {
                                format_size(row.size)
                            }
                        );
                        if file_line(
                            ui,
                            icons::type_glyph(row.is_dir, &row.name),
                            &label,
                            "",
                            palette,
                            None,
                        )
                        .clicked()
                            && row.is_dir
                        {
                            self.request_analyze(row.path.clone());
                        }
                    }
                });
            });
            let available = ui.available_size().max(Vec2::new(240.0, 240.0));
            let (rect, response) = ui.allocate_exact_size(available, Sense::click());
            ui.painter()
                .rect_filled(rect, CornerRadius::same(8), palette.card);
            if self.sunburst {
                paint_sunburst(ui, rect, &cells, palette);
            } else {
                paint_treemap(ui, rect, &cells, palette);
            }
            if response.clicked() {
                if let Some(pointer) = response.interact_pointer_pos() {
                    if let Some(cell) = hit_cell(&cells, rect, pointer, self.sunburst) {
                        if cell.is_dir {
                            self.request_analyze(cell.path.clone());
                        }
                    }
                }
            }
        });
    }

    fn draw_status(&mut self, ui: &mut Ui, rect: Rect, palette: FilesPalette) {
        let mut bar = ui.new_child(
            UiBuilder::new()
                .max_rect(rect)
                .layout(Layout::left_to_right(Align::Center)),
        );
        bar.add_space(12.0);
        let count = if self.tab().home && self.tab().search.is_empty() {
            0
        } else {
            self.visible_rows().len()
        };
        bar.label(
            RichText::new(format!("{count} {}", self.tr.items))
                .color(palette.text_secondary)
                .size(13.0),
        );
        let selected = self.tab().selected.len();
        if selected > 0 {
            bar.label(
                RichText::new(format!("   {selected} {}", self.tr.selected))
                    .color(palette.text_secondary)
                    .size(13.0),
            );
        }
    }

    fn draw_dialogs(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        if self.help {
            egui::Window::new("Keyboard shortcuts")
                .open(&mut self.help)
                .show(&ctx, |ui| {
                    ui.label("Ctrl+T new tab    Ctrl+W close tab    Ctrl+Tab next tab");
                    ui.label("Alt+Left / Right back and forward    Alt+Up parent");
                    ui.label("Ctrl+L path    Ctrl+F search    Enter open    F5 refresh");
                    ui.label("Ctrl+C / X / V copy, cut, paste    Ctrl+Shift+N new folder");
                    ui.label("F2 rename    Delete trash    Shift+Delete permanent delete");
                });
        }
        if let Some((path, mut name)) = self.rename.clone() {
            let mut open = true;
            let mut commit = false;
            egui::Window::new("Rename")
                .open(&mut open)
                .show(&ctx, |ui| {
                    let response =
                        ui.add(egui::TextEdit::singleline(&mut name).desired_width(280.0));
                    if ui.button("Rename").clicked()
                        || (response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)))
                    {
                        commit = true;
                    }
                });
            if commit {
                let _ = self.commands.send(Command::Rename {
                    path,
                    new_name: name,
                });
                self.rename = None;
            } else if open {
                self.rename = Some((path, name));
            } else {
                self.rename = None;
            }
        }
        if !self.confirm_delete.is_empty() {
            let paths = self.confirm_delete.clone();
            let mut open = true;
            let mut confirm = false;
            egui::Window::new("Delete permanently")
                .open(&mut open)
                .show(&ctx, |ui| {
                    ui.label(format!(
                        "Delete {} item(s) without using the Recycle Bin?",
                        paths.len()
                    ));
                    if ui.button("Delete").clicked() {
                        confirm = true;
                    }
                });
            if confirm {
                let _ = self.commands.send(Command::DeletePermanent { paths });
                self.confirm_delete.clear();
            } else if !open {
                self.confirm_delete.clear();
            }
        }
    }

    fn sort_visible(&mut self) {
        let sort = self.tab().sort;
        let ascending = self.tab().ascending;
        let searching = !self.tab().search.is_empty();
        let tab = self.tab_mut();
        if searching {
            rustfiles_core::sort_rows(&mut tab.search_rows, sort, ascending, true);
        } else {
            rustfiles_core::sort_rows(&mut tab.rows, sort, ascending, true);
        }
    }

    fn request_thumb(&mut self, path: &std::path::Path, is_dir: bool) {
        if is_dir || self.thumbs.contains_key(path) || !self.thumb_asked.insert(path.to_path_buf())
        {
            return;
        }
        let ext = path
            .extension()
            .and_then(|ext| ext.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        if matches!(ext.as_str(), "png" | "jpg" | "jpeg" | "gif" | "bmp") {
            let _ = self.commands.send(Command::Thumb {
                path: path.to_path_buf(),
            });
        }
    }

    pub fn show_home(&mut self) {
        self.pane = Pane::Files;
        let tab = self.tab_mut();
        tab.home = true;
        tab.search.clear();
        tab.selected.clear();
    }

    pub fn new_tab_from_home(&mut self) {
        self.tabs
            .push(crate::app::Tab::at(self.places.home.clone(), true));
        self.current = self.tabs.len() - 1;
        self.pane = Pane::Files;
    }

    fn open_trash(&mut self) {
        #[cfg(windows)]
        {
            let _ = std::process::Command::new("explorer")
                .arg("shell:RecycleBinFolder")
                .spawn();
            self.status = "Opened the Recycle Bin.".into();
        }
        #[cfg(not(windows))]
        {
            let trash = self
                .places
                .home
                .join(".local")
                .join("share")
                .join("Trash")
                .join("files");
            if trash.is_dir() {
                self.navigate(trash);
            } else {
                self.status = "The trash folder is empty.".into();
            }
        }
    }
}

fn tab_title(tab: &crate::app::Tab, tr: &crate::i18n::Strings) -> String {
    if tab.home {
        return tr.home.to_string();
    }
    tab.path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| tab.path.display().to_string())
}
fn tab_chip(
    ui: &mut Ui,
    title: &str,
    selected: bool,
    palette: FilesPalette,
    icon: Option<&egui::TextureHandle>,
    fallback: u32,
) -> (egui::Response, bool) {
    let width = (title.chars().count() as f32 * 8.0 + 64.0).clamp(140.0, 220.0);
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, 36.0), Sense::click());
    let fill = if selected {
        palette.tab_selected
    } else if response.hovered() {
        palette.hover
    } else {
        Color32::TRANSPARENT
    };
    let radius = if selected {
        CornerRadius {
            nw: 8,
            ne: 8,
            sw: 0,
            se: 0,
        }
    } else {
        CornerRadius::same(8)
    };
    ui.painter().rect_filled(rect, radius, fill);
    let icon_rect =
        Rect::from_center_size(rect.left_center() + Vec2::new(18.0, 0.0), Vec2::splat(20.0));
    if let Some(texture) = icon {
        ui.painter().image(
            texture.id(),
            icon_rect,
            Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
            Color32::WHITE,
        );
    } else {
        ui.painter().text(
            icon_rect.center(),
            egui::Align2::CENTER_CENTER,
            icons::glyph(fallback),
            icon_font(18.0),
            palette.accent,
        );
    }
    ui.painter().text(
        rect.left_center() + Vec2::new(36.0, 0.0),
        egui::Align2::LEFT_CENTER,
        truncate(title, 22),
        FontId::proportional(14.0),
        palette.text,
    );
    let close = Rect::from_center_size(
        rect.right_center() - Vec2::new(14.0, 0.0),
        Vec2::splat(16.0),
    );
    let close_response = ui.interact(close, response.id.with("x"), Sense::click());
    if selected || response.hovered() {
        ui.painter().text(
            close.center(),
            egui::Align2::CENTER_CENTER,
            icons::glyph(icons::DISMISS),
            icon_font(10.0),
            palette.text_secondary,
        );
    }
    ui.add_space(4.0);
    (response, close_response.clicked())
}
fn chip_button(ui: &mut Ui, glyph: u32, palette: FilesPalette) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(28.0), Sense::click());
    if response.hovered() {
        ui.painter()
            .rect_filled(rect, CornerRadius::same(6), palette.hover);
    }
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        icons::glyph(glyph),
        icon_font(14.0),
        palette.text,
    );
    response
}
fn nav_button(ui: &mut Ui, glyph: u32, enabled: bool, palette: FilesPalette) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(32.0), Sense::click());
    if enabled && response.hovered() {
        ui.painter()
            .rect_filled(rect, CornerRadius::same(4), palette.hover);
    }
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        icons::glyph(glyph),
        icon_font(16.0),
        if enabled {
            palette.text
        } else {
            palette.text_secondary.gamma_multiply(0.4)
        },
    );
    response
}
fn command_button(ui: &mut Ui, glyph: u32, label: &str, palette: FilesPalette) -> egui::Response {
    let width = 30.0 + label.len() as f32 * 7.0;
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, 30.0), Sense::click());
    if response.hovered() {
        ui.painter()
            .rect_filled(rect, CornerRadius::same(4), palette.hover);
    }
    ui.painter().text(
        rect.left_center() + Vec2::new(6.0, 0.0),
        egui::Align2::LEFT_CENTER,
        icons::glyph(glyph),
        icon_font(16.0),
        palette.text,
    );
    ui.painter().text(
        rect.left_center() + Vec2::new(26.0, 0.0),
        egui::Align2::LEFT_CENTER,
        label,
        FontId::proportional(13.0),
        palette.text,
    );
    ui.add_space(2.0);
    response
}
#[allow(dead_code)]
fn text_button(ui: &mut Ui, label: &str, palette: FilesPalette) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(
        Vec2::new(label.len() as f32 * 7.0 + 16.0, 22.0),
        Sense::click(),
    );
    if response.hovered() {
        ui.painter()
            .rect_filled(rect, CornerRadius::same(4), palette.hover);
    }
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        label,
        FontId::proportional(12.0),
        palette.text,
    );
    response
}
fn icon_button(ui: &mut Ui, glyph: u32, palette: FilesPalette) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(36.0), Sense::click());
    if response.hovered() {
        ui.painter()
            .rect_filled(rect, CornerRadius::same(4), palette.hover);
    }
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        icons::glyph(glyph),
        icon_font(18.0),
        palette.text,
    );
    ui.add_space(2.0);
    response
}

fn crumb_chip(
    ui: &mut Ui,
    glyph: Option<u32>,
    label: &str,
    palette: FilesPalette,
) -> egui::Response {
    let width =
        16.0 + label.chars().count() as f32 * 7.6 + if glyph.is_some() { 22.0 } else { 0.0 };
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width.max(28.0), 28.0), Sense::click());
    if response.hovered() {
        ui.painter()
            .rect_filled(rect, CornerRadius::same(4), palette.hover);
    }
    let mut x = rect.left() + 6.0;
    if let Some(glyph) = glyph {
        ui.painter().text(
            pos2(x + 8.0, rect.center().y),
            egui::Align2::CENTER_CENTER,
            icons::glyph(glyph),
            icon_font(16.0),
            palette.text,
        );
        x += 20.0;
    }
    ui.painter().text(
        pos2(x, rect.center().y),
        egui::Align2::LEFT_CENTER,
        label,
        FontId::proportional(14.0),
        palette.text,
    );
    ui.add_space(2.0);
    response
}

fn section_header(ui: &mut Ui, title: &str, open: &mut bool, palette: FilesPalette) -> bool {
    let (rect, response) =
        ui.allocate_exact_size(Vec2::new(ui.available_width(), 32.0), Sense::click());
    if response.clicked() {
        *open = !*open;
    }
    let glyph = if *open {
        icons::CHEVRON_DOWN
    } else {
        icons::CHEVRON_RIGHT
    };
    ui.painter().text(
        rect.left_center() + Vec2::new(8.0, 0.0),
        egui::Align2::LEFT_CENTER,
        icons::glyph(glyph),
        icon_font(12.0),
        palette.text_secondary,
    );
    ui.painter().text(
        rect.left_center() + Vec2::new(26.0, 0.0),
        egui::Align2::LEFT_CENTER,
        title,
        FontId::proportional(13.0),
        palette.text_secondary,
    );
    *open
}

fn nav_entry(
    ui: &mut Ui,
    app: &mut RustFilesApp,
    path: Option<&std::path::Path>,
    glyph: u32,
    label: &str,
    palette: FilesPalette,
) -> egui::Response {
    let selected = path.is_some_and(|path| !app.tab().home && app.tab().path == path);
    let texture = path.and_then(|path| app.icons_tex.get(path).cloned());
    if let Some(path) = path {
        app.request_icon(path, 32);
    }
    let (rect, response) =
        ui.allocate_exact_size(Vec2::new(ui.available_width(), 40.0), Sense::click());
    ui.painter().rect_filled(
        rect,
        CornerRadius::same(4),
        if selected {
            palette.selected
        } else if response.hovered() {
            palette.hover
        } else {
            Color32::TRANSPARENT
        },
    );
    let icon_rect =
        Rect::from_center_size(rect.left_center() + Vec2::new(18.0, 0.0), Vec2::splat(20.0));
    if let Some(texture) = texture {
        ui.painter().image(
            texture.id(),
            icon_rect,
            Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
            Color32::WHITE,
        );
    } else {
        ui.painter().text(
            icon_rect.center(),
            egui::Align2::CENTER_CENTER,
            icons::glyph(glyph),
            icon_font(18.0),
            palette.text,
        );
    }
    ui.painter().text(
        rect.left_center() + Vec2::new(40.0, 0.0),
        egui::Align2::LEFT_CENTER,
        truncate(label, 28),
        FontId::proportional(14.0),
        palette.text,
    );
    response
}

#[allow(dead_code)]
fn section(ui: &mut Ui, _palette: FilesPalette) {
    ui.add_space(6.0);
}
#[allow(dead_code)]
fn section_title(ui: &mut Ui, title: &str, palette: FilesPalette) {
    ui.add_space(12.0);
    ui.label(
        RichText::new(title)
            .color(palette.text_secondary)
            .size(12.0)
            .strong(),
    );
    ui.add_space(2.0);
}
fn nav_row(
    ui: &mut Ui,
    glyph: u32,
    label: &str,
    selected: bool,
    palette: FilesPalette,
) -> egui::Response {
    let (rect, response) =
        ui.allocate_exact_size(Vec2::new(ui.available_width(), 40.0), Sense::click());
    ui.painter().rect_filled(
        rect,
        CornerRadius::same(4),
        if selected {
            palette.selected
        } else if response.hovered() {
            palette.hover
        } else {
            Color32::TRANSPARENT
        },
    );
    ui.painter().text(
        rect.left_center() + Vec2::new(10.0, 0.0),
        egui::Align2::LEFT_CENTER,
        icons::glyph(glyph),
        icon_font(20.0),
        if selected {
            palette.accent
        } else {
            palette.text
        },
    );
    ui.painter().text(
        rect.left_center() + Vec2::new(40.0, 0.0),
        egui::Align2::LEFT_CENTER,
        truncate(label, 28),
        FontId::proportional(14.0),
        palette.text,
    );
    response
}
#[allow(dead_code)]
fn drive_row(
    ui: &mut Ui,
    label: &str,
    mount: &str,
    used: u64,
    total: u64,
    palette: FilesPalette,
) -> egui::Response {
    let (rect, response) =
        ui.allocate_exact_size(Vec2::new(ui.available_width(), 46.0), Sense::click());
    if response.hovered() {
        ui.painter()
            .rect_filled(rect, CornerRadius::same(4), palette.hover);
    }
    ui.painter().text(
        rect.left_top() + Vec2::new(10.0, 8.0),
        egui::Align2::LEFT_TOP,
        icons::glyph(icons::HARD_DRIVE),
        icon_font(16.0),
        palette.text,
    );
    ui.painter().text(
        rect.left_top() + Vec2::new(34.0, 4.0),
        egui::Align2::LEFT_TOP,
        truncate(label, 20),
        FontId::proportional(13.0),
        palette.text,
    );
    let caption = if total == 0 {
        mount.to_string()
    } else {
        format!(
            "{} free of {}",
            format_size(total - used),
            format_size(total)
        )
    };
    ui.painter().text(
        rect.left_top() + Vec2::new(34.0, 20.0),
        egui::Align2::LEFT_TOP,
        caption,
        FontId::proportional(11.0),
        palette.text_secondary,
    );
    usage_bar(
        ui.painter(),
        Rect::from_min_size(
            rect.left_bottom() + Vec2::new(34.0, -8.0),
            Vec2::new((rect.width() - 48.0).max(40.0), 3.0),
        ),
        used,
        total,
        palette,
    );
    response
}
fn drive_card(
    ui: &mut Ui,
    label: &str,
    mount: &str,
    used: u64,
    total: u64,
    palette: FilesPalette,
) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::new(230.0, 96.0), Sense::click());
    ui.painter().rect(
        rect,
        CornerRadius::same(8),
        palette.card,
        Stroke::new(1.0, palette.stroke),
        egui::StrokeKind::Inside,
    );
    ui.painter().text(
        rect.left_top() + Vec2::new(14.0, 14.0),
        egui::Align2::LEFT_TOP,
        icons::glyph(icons::HARD_DRIVE),
        icon_font(22.0),
        palette.accent,
    );
    ui.painter().text(
        rect.left_top() + Vec2::new(46.0, 16.0),
        egui::Align2::LEFT_TOP,
        truncate(if label.is_empty() { mount } else { label }, 16),
        FontId::proportional(15.0),
        palette.text,
    );
    let caption = if total == 0 {
        mount.to_string()
    } else {
        format!("{} free", format_size(total - used))
    };
    ui.painter().text(
        rect.left_top() + Vec2::new(14.0, 48.0),
        egui::Align2::LEFT_TOP,
        caption,
        FontId::proportional(12.0),
        palette.text_secondary,
    );
    usage_bar(
        ui.painter(),
        Rect::from_min_size(
            rect.left_bottom() + Vec2::new(14.0, -18.0),
            Vec2::new(rect.width() - 28.0, 6.0),
        ),
        used,
        total,
        palette,
    );
    ui.add_space(10.0);
    response
}
fn usage_bar(painter: &egui::Painter, track: Rect, used: u64, total: u64, palette: FilesPalette) {
    painter.rect_filled(track, CornerRadius::same(3), palette.usage_track);
    if total > 0 {
        let width = track.width() * (used as f32 / total as f32).clamp(0.0, 1.0);
        painter.rect_filled(
            Rect::from_min_size(track.min, Vec2::new(width, track.height())),
            CornerRadius::same(3),
            palette.usage,
        );
    }
}
fn file_line(
    ui: &mut Ui,
    glyph: u32,
    title: &str,
    meta: &str,
    palette: FilesPalette,
    icon: Option<&egui::TextureHandle>,
) -> egui::Response {
    let (rect, response) =
        ui.allocate_exact_size(Vec2::new(ui.available_width(), 36.0), Sense::click());
    if response.hovered() {
        ui.painter()
            .rect_filled(rect, CornerRadius::same(4), palette.hover);
    }
    let icon_rect =
        Rect::from_center_size(rect.left_center() + Vec2::new(14.0, 0.0), Vec2::splat(20.0));
    if let Some(texture) = icon {
        ui.painter().image(
            texture.id(),
            icon_rect,
            Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
            Color32::WHITE,
        );
    } else {
        ui.painter().text(
            icon_rect.center(),
            egui::Align2::CENTER_CENTER,
            icons::glyph(glyph),
            icon_font(18.0),
            palette.accent,
        );
    }
    ui.painter().text(
        rect.left_center() + Vec2::new(30.0, 0.0),
        egui::Align2::LEFT_CENTER,
        truncate(title, 48),
        FontId::proportional(14.0),
        palette.text,
    );
    if !meta.is_empty() {
        ui.painter().text(
            rect.right_center() - Vec2::new(8.0, 0.0),
            egui::Align2::RIGHT_CENTER,
            meta,
            FontId::proportional(12.0),
            palette.text_secondary,
        );
    }
    response
}
fn column_header(
    ui: &mut Ui,
    title: &str,
    width: f32,
    column: SortColumn,
    app: &mut RustFilesApp,
    palette: FilesPalette,
) {
    let active = app.tab().sort == column;
    let mark = if !active {
        ""
    } else if app.tab().ascending {
        "  ↑"
    } else {
        "  ↓"
    };
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, 24.0), Sense::click());
    if response.hovered() {
        ui.painter()
            .rect_filled(rect, CornerRadius::same(4), palette.hover);
    }
    ui.painter().text(
        rect.left_center() + Vec2::new(4.0, 0.0),
        egui::Align2::LEFT_CENTER,
        format!("{title}{mark}"),
        FontId::proportional(12.0),
        palette.text_secondary,
    );
    if response.clicked() {
        let tab = app.tab_mut();
        if tab.sort == column {
            tab.ascending = !tab.ascending;
        } else {
            tab.sort = column;
            tab.ascending = !matches!(column, SortColumn::Size | SortColumn::Modified);
        }
        app.sort_visible();
    }
}
fn left_cell(ui: &mut Ui, width: f32, text: &str, color: Color32, size: f32) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, 20.0), Sense::hover());
    ui.painter().text(
        rect.left_center(),
        egui::Align2::LEFT_CENTER,
        text,
        FontId::proportional(size),
        color,
    );
}

fn detail_line(ui: &mut Ui, label: &str, value: &str, palette: FilesPalette) {
    ui.add_space(6.0);
    ui.label(
        RichText::new(label)
            .color(palette.text_secondary)
            .size(12.0),
    );
    ui.label(RichText::new(value).color(palette.text).size(13.0));
}
fn paint_type_icon(
    ui: &mut Ui,
    path: &std::path::Path,
    is_dir: bool,
    glyph: u32,
    palette: FilesPalette,
    app: &mut RustFilesApp,
) {
    let _ = is_dir;
    app.request_icon(path, 32);
    if let Some(texture) = app.icons_tex.get(path) {
        let (rect, _) = ui.allocate_exact_size(Vec2::splat(20.0), Sense::hover());
        ui.painter().image(
            texture.id(),
            rect,
            Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
            Color32::WHITE,
        );
        return;
    }
    ui.label(
        RichText::new(icons::glyph(glyph))
            .font(icon_font(20.0))
            .color(palette.accent),
    );
}
fn paint_treemap(ui: &Ui, rect: Rect, cells: &[crate::worker::MapCell], palette: FilesPalette) {
    for cell in cells {
        let visual = Rect::from_min_size(
            rect.min + Vec2::new(cell.rect.x * rect.width(), cell.rect.y * rect.height()),
            Vec2::new(cell.rect.w * rect.width(), cell.rect.h * rect.height()),
        );
        if visual.width() < 2.0 || visual.height() < 2.0 {
            continue;
        }
        ui.painter().rect_filled(
            visual.shrink(1.0),
            CornerRadius::same(3),
            color_for(&cell.name, palette),
        );
        if visual.width() > 64.0 && visual.height() > 28.0 {
            ui.painter().text(
                visual.left_top() + Vec2::new(8.0, 6.0),
                egui::Align2::LEFT_TOP,
                truncate(&cell.name, 20),
                FontId::proportional(12.0),
                Color32::WHITE,
            );
            ui.painter().text(
                visual.left_top() + Vec2::new(8.0, 22.0),
                egui::Align2::LEFT_TOP,
                format_size(cell.size),
                FontId::proportional(11.0),
                Color32::from_white_alpha(220),
            );
        }
    }
}
fn paint_sunburst(ui: &Ui, rect: Rect, cells: &[crate::worker::MapCell], palette: FilesPalette) {
    let center = rect.center();
    let outer = rect.width().min(rect.height()) * 0.42;
    let inner = outer * 0.48;
    for cell in cells {
        paint_arc(
            ui.painter(),
            center,
            inner,
            outer,
            cell.start,
            cell.end,
            color_for(&cell.name, palette),
        );
    }
    ui.painter()
        .circle_filled(center, inner * 0.88, palette.file_area);
}
fn paint_arc(
    painter: &egui::Painter,
    center: egui::Pos2,
    inner: f32,
    outer: f32,
    start: f32,
    end: f32,
    color: Color32,
) {
    let span = end - start;
    if span <= 0.0 {
        return;
    }
    if span > std::f32::consts::PI {
        let mid = start + span * 0.5;
        paint_arc(painter, center, inner, outer, start, mid, color);
        paint_arc(painter, center, inner, outer, mid, end, color);
        return;
    }
    let steps = ((span * 16.0).ceil() as i32).max(2);
    let mut points = Vec::new();
    for step in 0..=steps {
        let t = start + span * (step as f32 / steps as f32);
        points.push(center + Vec2::new(outer * t.cos(), outer * t.sin()));
    }
    for step in (0..=steps).rev() {
        let t = start + span * (step as f32 / steps as f32);
        points.push(center + Vec2::new(inner * t.cos(), inner * t.sin()));
    }
    painter.add(egui::Shape::convex_polygon(points, color, Stroke::NONE));
}
fn hit_cell(
    cells: &[crate::worker::MapCell],
    rect: Rect,
    pointer: egui::Pos2,
    sunburst: bool,
) -> Option<&crate::worker::MapCell> {
    if sunburst {
        let delta = pointer - rect.center();
        let radius = delta.length();
        let outer = rect.width().min(rect.height()) * 0.42;
        let inner = outer * 0.48;
        if !(inner..=outer).contains(&radius) {
            return None;
        }
        let mut angle = delta.y.atan2(delta.x);
        if angle < 0.0 {
            angle += std::f32::consts::TAU;
        }
        cells
            .iter()
            .find(|cell| angle >= cell.start && angle < cell.end)
    } else {
        let local = egui::Pos2::new(
            (pointer.x - rect.min.x) / rect.width(),
            (pointer.y - rect.min.y) / rect.height(),
        );
        cells.iter().find(|cell| {
            local.x >= cell.rect.x
                && local.y >= cell.rect.y
                && local.x <= cell.rect.x + cell.rect.w
                && local.y <= cell.rect.y + cell.rect.h
        })
    }
}
fn color_for(name: &str, palette: FilesPalette) -> Color32 {
    let mut hash = 2_166_136_261u32;
    for byte in name.bytes() {
        hash ^= u32::from(byte);
        hash = hash.wrapping_mul(16_777_619);
    }
    let (r, g, b) = hsv(
        (hash % 360) as f32,
        0.42,
        if palette.text.r() > 200 { 0.58 } else { 0.70 },
    );
    Color32::from_rgb(r, g, b)
}
fn hsv(hue: f32, sat: f32, value: f32) -> (u8, u8, u8) {
    let c = value * sat;
    let x = c * (1.0 - ((hue / 60.0) % 2.0 - 1.0).abs());
    let m = value - c;
    let (r, g, b) = match hue as i32 {
        0..=59 => (c, x, 0.0),
        60..=119 => (x, c, 0.0),
        120..=179 => (0.0, c, x),
        180..=239 => (0.0, x, c),
        240..=299 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    (
        ((r + m) * 255.0) as u8,
        ((g + m) * 255.0) as u8,
        ((b + m) * 255.0) as u8,
    )
}
fn truncate(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        text.to_string()
    } else {
        format!(
            "{}…",
            text.chars().take(max.saturating_sub(1)).collect::<String>()
        )
    }
}
fn places_contains(places: &Places, path: &std::path::Path) -> bool {
    [
        &places.desktop,
        &places.downloads,
        &places.documents,
        &places.pictures,
        &places.music,
        &places.videos,
        &places.home,
    ]
    .iter()
    .any(|known| *known == path)
}
fn recent_files(places: &Places) -> Vec<FileRow> {
    let mut rows = Vec::new();
    for dir in [&places.desktop, &places.downloads, &places.documents] {
        let Ok(entries) = std::fs::read_dir(dir) else {
            continue;
        };
        for entry in entries.flatten().take(24) {
            let Ok(meta) = std::fs::symlink_metadata(entry.path()) else {
                continue;
            };
            if meta.file_type().is_symlink() {
                continue;
            }
            let modified = meta
                .modified()
                .ok()
                .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
                .map(|d| d.as_secs())
                .unwrap_or(0);
            rows.push(FileRow {
                name: entry.file_name().to_string_lossy().into_owned(),
                path: entry.path(),
                is_dir: meta.is_dir(),
                hidden: false,
                size: if meta.is_dir() { 0 } else { meta.len() },
                modified,
            });
        }
    }
    rows.sort_by_key(|row| std::cmp::Reverse(row.modified));
    rows.truncate(8);
    rows
}
