//! Files-style shell: title-bar tabs, navigation, command bar, sidebar, and home.
use std::path::PathBuf;
use std::time::UNIX_EPOCH;

use egui::{
    pos2, Align, Color32, CornerRadius, FontId, Layout, Rect, RichText, ScrollArea, Sense, Stroke,
    Ui, UiBuilder, Vec2,
};
use rustfiles_core::{format_size, FileRow, SortColumn};

use crate::app::{format_bytes, format_free, format_modified_lang, Pane, RustFilesApp};
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
            pos2(side.right() + 8.0, body.top() + 6.0),
            pos2(body.right() - preview_w - 6.0, body.bottom() - 6.0),
        );
        let preview = Rect::from_min_max(pos2(content.right(), body.top()), body.right_bottom());
        let side_fill = if self.mica_applied {
            Color32::from_rgba_unmultiplied(0, 0, 0, 72)
        } else {
            palette.sidebar
        };
        let file_fill = if self.mica_applied {
            if self.dark {
                Color32::from_rgba_unmultiplied(44, 44, 44, 220)
            } else {
                Color32::from_rgba_unmultiplied(252, 252, 252, 230)
            }
        } else {
            palette.file_area
        };
        let file_radius = CornerRadius {
            nw: 12,
            ne: 0,
            sw: 0,
            se: 0,
        };
        ui.painter()
            .rect_filled(side, CornerRadius::ZERO, side_fill);
        ui.painter().rect_filled(content, file_radius, file_fill);
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
        let caption_w = 46.0 * 3.0;
        let tabs = Rect::from_min_max(rect.min, pos2(rect.right() - caption_w, rect.bottom()));
        let mut child = ui.new_child(
            UiBuilder::new()
                .max_rect(tabs)
                .layout(Layout::left_to_right(Align::BOTTOM)),
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
        let gap = Rect::from_min_max(pos2(child.cursor().left(), tabs.top()), tabs.max);
        if gap.width() > 8.0 {
            let drag = child.interact(gap, egui::Id::unique("title-drag"), Sense::click_and_drag());
            if drag.drag_started() {
                ui.ctx().send_viewport_cmd(egui::ViewportCommand::StartDrag);
            }
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
        for (index, (_glyph, id)) in buttons.iter().enumerate() {
            let button = Rect::from_min_size(
                pos2(rect.right() - caption_w + index as f32 * 46.0, rect.top()),
                Vec2::new(46.0, 32.0),
            );
            let response = ui.interact(button, egui::Id::unique(*id), Sense::click());
            let close = *id == "close";
            let hover = ui
                .ctx()
                .animate_bool_with_time(response.id, response.hovered(), 0.1);
            if hover > 0.01 {
                let fill = if close {
                    Color32::from_rgb(196, 43, 28).gamma_multiply(hover)
                } else {
                    palette.hover.gamma_multiply(hover)
                };
                ui.painter().rect_filled(button, CornerRadius::ZERO, fill);
            }
            let color = if close && response.hovered() {
                Color32::WHITE
            } else {
                palette.text
            };
            paint_caption_glyph(ui, button, id, maximized, color);
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
        let (address, address_response) =
            bar.allocate_exact_size(Vec2::new(address_w, 32.0), Sense::click());
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
                let mut previous: Option<PathBuf> = None;
                for (label, path) in &parts {
                    let chevron = crumb_chevron(&mut crumbs, palette);
                    let parent = previous.clone();
                    egui::Popup::menu(&chevron).show(|ui| {
                        let entries = crumb_children(parent.as_deref(), &self.volumes, self.lang);
                        if entries.is_empty() {
                            ui.label(self.tr.no_matches);
                        }
                        for (name, dest) in entries {
                            if ui.button(name).clicked() {
                                self.navigate(dest);
                                egui::Popup::close_all(ui.ctx());
                            }
                        }
                    });
                    let shown = drive_crumb(self, label, path);
                    if crumb_chip(&mut crumbs, None, &shown, palette).clicked() {
                        self.navigate(path.clone());
                        return;
                    }
                    previous = Some(path.clone());
                }
            }
            if address_response.double_clicked() {
                self.perform(Action::FocusPath);
            }
        }
        bar.add_space(8.0);
        let (search, _) = bar.allocate_exact_size(Vec2::new(search_w, 32.0), Sense::hover());
        bar.painter().rect(
            search,
            CornerRadius::same(4),
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
        bar_separator(&mut bar, palette);
        let has_selection = !self.tab().selected.is_empty();
        let commands = [
            (icons::CUT, self.tr.cut, Action::Cut, has_selection),
            (icons::COPY, self.tr.copy, Action::Copy, has_selection),
            (
                icons::PASTE,
                self.tr.paste,
                Action::Paste,
                self.clipboard.is_some(),
            ),
            (icons::RENAME, self.tr.rename, Action::Rename, has_selection),
            (icons::DELETE, self.tr.delete, Action::Trash, has_selection),
        ];
        for (glyph, tip, action, enabled) in commands {
            if icon_button(&mut bar, glyph, tip, palette, enabled).clicked() {
                self.perform(action);
            }
        }
        let layout_button = bar.with_layout(Layout::right_to_left(Align::Center), |bar| {
            bar.add_space(8.0);
            if icon_button(bar, icons::DOCUMENT, self.tr.preview, palette, true).clicked() {
                self.show_preview = !self.show_preview;
                self.save_config();
            }
            let layout_button = command_button(bar, icons::GRID, self.tr.layout, palette);
            if icon_button(bar, icons::SORT, self.tr.group, palette, true).clicked() {
                let tab = self.tab_mut();
                tab.sort = match tab.sort {
                    SortColumn::Name => SortColumn::Type,
                    SortColumn::Type => SortColumn::Modified,
                    SortColumn::Modified => SortColumn::Size,
                    SortColumn::Size => SortColumn::Name,
                };
                let grouped = match tab.sort {
                    SortColumn::Name => self.tr.name,
                    SortColumn::Type => self.tr.kind,
                    SortColumn::Modified => self.tr.modified,
                    SortColumn::Size => self.tr.size,
                };
                self.status = format!("{}: {grouped}", self.tr.group);
                self.sort_visible();
            }
            if icon_button(bar, icons::SORT, self.tr.sort, palette, true).clicked() {
                let tab = self.tab_mut();
                tab.ascending = !tab.ascending;
                self.sort_visible();
            }
            if icon_button(bar, icons::INFO, self.tr.analyze, palette, true).clicked() {
                self.perform(Action::Analyzer);
            }
            layout_button
        });
        if layout_button.inner.clicked() {
            self.layout_menu = !self.layout_menu;
        }
        let layout_open = ui.ctx().animate_bool_with_time(
            egui::Id::unique("layout-flyout"),
            self.layout_menu,
            0.14,
        );
        if layout_open > 0.02 {
            let pos = layout_button.inner.rect.left_bottom()
                + Vec2::new(0.0, 4.0 + (1.0 - layout_open) * 6.0);
            let modes = [
                (crate::app::ViewMode::Details, self.tr.details, icons::LIST),
                (crate::app::ViewMode::List, self.tr.list, icons::LIST),
                (crate::app::ViewMode::Tiles, self.tr.tiles, icons::GRID),
                (crate::app::ViewMode::Grid, self.tr.grid, icons::GRID),
                (
                    crate::app::ViewMode::Large,
                    self.tr.large_icons,
                    icons::IMAGE,
                ),
            ];
            let current = self.tab().view;
            if let Some(chosen) = flyout(ui.ctx(), pos, layout_open, &modes, Some(current), palette)
            {
                self.tab_mut().view = chosen;
                self.layout_menu = false;
            }
        }
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
                if section_header(
                    side,
                    self.tr.favorites,
                    &mut self.open_fav,
                    icons::STAR,
                    Color32::from_rgb(255, 185, 40),
                    palette,
                ) {
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
                if section_header(
                    side,
                    self.tr.drives,
                    &mut self.open_drives,
                    icons::HARD_DRIVE,
                    Color32::from_rgb(80, 170, 255),
                    palette,
                ) {
                    for volume in self.volumes.clone() {
                        let label = drive_title(&volume.label, &volume.mount, self.lang);
                        let mount = PathBuf::from(&volume.mount);
                        if nav_entry(side, self, Some(&mount), icons::HARD_DRIVE, &label, palette)
                            .clicked()
                        {
                            self.navigate(mount);
                        }
                    }
                }
                if section_header(
                    side,
                    self.tr.cloud,
                    &mut self.open_cloud,
                    icons::GLOBE,
                    Color32::from_rgb(90, 190, 255),
                    palette,
                ) && nav_row(side, icons::GLOBE, self.tr.cloud, false, palette).clicked()
                {
                    self.status = self.tr.cloud_unavailable.into();
                }
                if section_header(
                    side,
                    self.tr.network,
                    &mut self.open_network,
                    icons::GLOBE,
                    Color32::from_rgb(70, 190, 130),
                    palette,
                ) && nav_row(side, icons::GLOBE, self.tr.network, false, palette).clicked()
                {
                    self.status = self.tr.network_unavailable.into();
                }
                if section_header(
                    side,
                    self.tr.wsl,
                    &mut self.open_wsl,
                    icons::DESKTOP,
                    Color32::from_rgb(160, 120, 230),
                    palette,
                ) && nav_row(side, icons::DESKTOP, self.tr.wsl, false, palette).clicked()
                {
                    self.status = self.tr.wsl_unavailable.into();
                }
                if section_header(
                    side,
                    self.tr.tags,
                    &mut self.open_tags,
                    icons::TAG,
                    Color32::from_rgb(255, 150, 70),
                    palette,
                ) && nav_row(side, icons::TAG, self.tr.tags, false, palette).clicked()
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
        match self.tab().view {
            crate::app::ViewMode::Details => self.draw_details(&mut content, palette),
            crate::app::ViewMode::List => self.draw_list(&mut content, palette),
            crate::app::ViewMode::Tiles => self.draw_tiles(&mut content, palette),
            crate::app::ViewMode::Grid => self.draw_grid(&mut content, palette, false),
            crate::app::ViewMode::Large => self.draw_grid(&mut content, palette, true),
        }
    }

    fn draw_home(&mut self, ui: &mut Ui, palette: FilesPalette) {
        ui.label(
            RichText::new(self.tr.quick_access)
                .color(palette.text)
                .strong()
                .size(18.0),
        );
        ui.add_space(8.0);
        let places = self.places.clone();
        let quick = [
            (self.tr.desktop, places.desktop.clone()),
            (self.tr.downloads, places.downloads.clone()),
            (self.tr.documents, places.documents.clone()),
            (self.tr.pictures, places.pictures.clone()),
        ];
        ui.horizontal_wrapped(|ui| {
            for (label, path) in quick {
                self.request_icon(&path, 48);
                let icon = self.icons_tex.get(&path).cloned();
                if quick_card(ui, label, palette, icon.as_ref()).clicked() {
                    self.navigate(path);
                }
            }
        });
        ui.add_space(18.0);
        ui.label(
            RichText::new(self.tr.drives)
                .color(palette.text)
                .strong()
                .size(18.0),
        );
        ui.add_space(10.0);
        ui.horizontal_wrapped(|ui| {
            for volume in self.volumes.clone() {
                let mount = PathBuf::from(&volume.mount);
                self.request_icon(&mount, 32);
                let icon = self.icons_tex.get(&mount).cloned();
                let (total, free) = platform::disk_usage(&mount).unwrap_or((0, 0));
                let title = drive_title(&volume.label, &volume.mount, self.lang);
                if drive_card(
                    ui,
                    &title,
                    total.saturating_sub(free),
                    total,
                    palette,
                    self.lang,
                    icon.as_ref(),
                )
                .clicked()
                {
                    self.navigate(mount);
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
                &format_modified_lang(row.modified, self.lang),
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
        let cols = crate::layout::detail_cols(width);
        let (header, _) =
            ui.allocate_exact_size(Vec2::new(width, crate::layout::ROW), Sense::hover());
        self.paint_detail_header(ui, header, cols, palette);
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
        let mut context = None;
        let mut opened = None;
        ScrollArea::vertical()
            .id_salt("details")
            .auto_shrink([false, false])
            .show_rows(ui, crate::layout::ROW, len, |ui, range| {
                for index in range {
                    let row = &self.visible_rows()[index];
                    let selected = self.tab().selected.iter().any(|path| path == &row.path);
                    let name = row.name.clone();
                    let path = row.path.clone();
                    let is_dir = row.is_dir;
                    let modified = format_modified_lang(row.modified, self.lang);
                    let kind = if is_dir {
                        self.tr.folder_kind.to_string()
                    } else {
                        platform::describe_type(&path, self.lang == crate::i18n::Lang::Fr)
                    };
                    let size = if row.is_dir && row.size == 0 {
                        String::new()
                    } else {
                        format_bytes(row.size, self.lang)
                    };
                    let glyph = icons::type_glyph(row.is_dir, &name);
                    let (rect, response) = ui.allocate_exact_size(
                        Vec2::new(ui.available_width(), crate::layout::ROW),
                        Sense::click(),
                    );
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
                    self.paint_detail_row(
                        ui, rect, cols, &path, is_dir, glyph, &name, &modified, &kind, &size,
                        palette,
                    );
                    if response.clicked() {
                        clicked = Some((index, ui.input(|i| i.modifiers)));
                    }
                    if response.double_clicked() {
                        opened = Some(index);
                    }
                    if response.secondary_clicked() {
                        context = Some(index);
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
        if let Some(index) = context {
            self.select_row(index, egui::Modifiers::NONE);
            if let Some(path) = self.visible_rows().get(index).map(|row| row.path.clone()) {
                if let Some(pos) = ui.ctx().input(|input| input.pointer.interact_pos()) {
                    self.context_menu = Some((pos, path));
                    self.layout_menu = false;
                    self.new_menu = false;
                }
            }
        }
    }

    fn paint_detail_header(
        &mut self,
        ui: &mut Ui,
        rect: Rect,
        cols: crate::layout::DetailCols,
        palette: FilesPalette,
    ) {
        let x = crate::layout::detail_text_x(rect.left(), cols);
        let titles = [
            (self.tr.name, SortColumn::Name, false),
            (self.tr.modified, SortColumn::Modified, false),
            (self.tr.kind, SortColumn::Type, false),
            (self.tr.size, SortColumn::Size, true),
        ];
        let widths = [cols.name, cols.date, cols.kind, cols.size];
        for (index, ((title, column, right), width)) in titles.into_iter().zip(widths).enumerate() {
            let cell =
                Rect::from_min_size(pos2(x[index], rect.top()), Vec2::new(width, rect.height()));
            let response = ui.interact(cell, egui::Id::unique(("col", index)), Sense::click());
            let mark = if self.tab().sort != column {
                ""
            } else if self.tab().ascending {
                "  ↑"
            } else {
                "  ↓"
            };
            let anchor = if right {
                pos2(cell.right() - crate::layout::PAD, cell.center().y)
            } else {
                pos2(cell.left(), cell.center().y)
            };
            let align = if right {
                egui::Align2::RIGHT_CENTER
            } else {
                egui::Align2::LEFT_CENTER
            };
            ui.painter().text(
                anchor,
                align,
                format!("{title}{mark}"),
                FontId::proportional(12.0),
                palette.text_secondary,
            );
            if response.clicked() {
                let tab = self.tab_mut();
                if tab.sort == column {
                    tab.ascending = !tab.ascending;
                } else {
                    tab.sort = column;
                    tab.ascending = !matches!(column, SortColumn::Size | SortColumn::Modified);
                }
                self.sort_visible();
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn paint_detail_row(
        &mut self,
        ui: &mut Ui,
        rect: Rect,
        cols: crate::layout::DetailCols,
        path: &std::path::Path,
        is_dir: bool,
        glyph: u32,
        name: &str,
        modified: &str,
        kind: &str,
        size: &str,
        palette: FilesPalette,
    ) {
        let y = rect.center().y;
        let icon = Rect::from_center_size(
            pos2(
                rect.left() + crate::layout::PAD + crate::layout::ICON / 2.0,
                y,
            ),
            Vec2::splat(crate::layout::ICON),
        );
        self.mark_layout("detail-icon", icon);
        self.request_icon(path, 32);
        if let Some(texture) = self.icons_tex.get(path) {
            ui.painter().image(
                texture.id(),
                icon,
                Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                Color32::WHITE,
            );
        } else {
            ui.painter().text(
                icon.center(),
                egui::Align2::CENTER_CENTER,
                icons::glyph(glyph),
                icon_font(crate::layout::ICON - 4.0),
                palette.accent,
            );
        }
        let x = crate::layout::detail_text_x(rect.left(), cols);
        let values = [name, modified, kind, size];
        let widths = [cols.name, cols.date, cols.kind, cols.size];
        let colors = [
            palette.text,
            palette.text_secondary,
            palette.text_secondary,
            palette.text_secondary,
        ];
        for index in 0..4 {
            let cell = Rect::from_min_size(
                pos2(x[index], rect.top()),
                Vec2::new(widths[index], rect.height()),
            );
            let right = index == 3;
            let anchor = if right {
                pos2(cell.right() - crate::layout::PAD, y)
            } else {
                pos2(cell.left(), y)
            };
            ui.painter().text(
                anchor,
                if right {
                    egui::Align2::RIGHT_CENTER
                } else {
                    egui::Align2::LEFT_CENTER
                },
                truncate(values[index], if index == 0 { 48 } else { 24 }),
                FontId::proportional(if index == 0 { 14.0 } else { 12.0 }),
                colors[index],
            );
            if index == 0 {
                self.mark_layout("detail-name", cell);
            }
        }
        let _ = is_dir;
    }

    fn draw_list(&mut self, ui: &mut Ui, palette: FilesPalette) {
        let len = self.visible_rows().len();
        if len == 0 {
            ui.label(RichText::new(self.tr.empty).color(palette.text_secondary));
            return;
        }
        let mut clicked = None;
        let mut context = None;
        let mut opened = None;
        ScrollArea::vertical()
            .id_salt("list")
            .auto_shrink([false, false])
            .show_rows(ui, 40.0, len, |ui, range| {
                for index in range {
                    let (path, name, is_dir, selected) = {
                        let row = &self.visible_rows()[index];
                        let selected = self.tab().selected.iter().any(|have| have == &row.path);
                        (row.path.clone(), row.name.clone(), row.is_dir, selected)
                    };
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
                    self.request_icon(&path, 32);
                    let glyph = icons::type_glyph(is_dir, &name);
                    if let Some(texture) = self.icons_tex.get(&path) {
                        let image = Rect::from_center_size(
                            rect.left_center() + Vec2::new(16.0, 0.0),
                            Vec2::splat(20.0),
                        );
                        ui.painter().image(
                            texture.id(),
                            image,
                            Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                            Color32::WHITE,
                        );
                    } else {
                        ui.painter().text(
                            rect.left_center() + Vec2::new(16.0, 0.0),
                            egui::Align2::CENTER_CENTER,
                            icons::glyph(glyph),
                            icon_font(18.0),
                            palette.accent,
                        );
                    }
                    ui.painter().text(
                        rect.left_center() + Vec2::new(36.0, 0.0),
                        egui::Align2::LEFT_CENTER,
                        name,
                        FontId::proportional(14.0),
                        palette.text,
                    );
                    if response.clicked() {
                        clicked = Some((index, ui.input(|i| i.modifiers)));
                    }
                    if response.double_clicked() {
                        opened = Some(index);
                    }
                    if response.secondary_clicked() {
                        context = Some(index);
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
        if let Some(index) = context {
            self.select_row(index, egui::Modifiers::NONE);
            if let Some(path) = self.visible_rows().get(index).map(|row| row.path.clone()) {
                if let Some(pos) = ui.ctx().input(|input| input.pointer.interact_pos()) {
                    self.context_menu = Some((pos, path));
                    self.layout_menu = false;
                    self.new_menu = false;
                }
            }
        }
    }

    fn draw_tiles(&mut self, ui: &mut Ui, palette: FilesPalette) {
        let len = self.visible_rows().len();
        if len == 0 {
            ui.label(RichText::new(self.tr.empty).color(palette.text_secondary));
            return;
        }
        let cols = ((ui.available_width() / 308.0).floor() as usize).max(1);
        let mut clicked = None;
        let mut context = None;
        let mut opened = None;
        ScrollArea::vertical()
            .id_salt("tiles")
            .auto_shrink([false, false])
            .show_rows(ui, 116.0, len.div_ceil(cols), |ui, range| {
                for row_index in range {
                    ui.horizontal(|ui| {
                        for col in 0..cols {
                            let index = row_index * cols + col;
                            if index >= len {
                                break;
                            }
                            let (path, name, is_dir, size, selected) = {
                                let row = &self.visible_rows()[index];
                                let selected =
                                    self.tab().selected.iter().any(|have| have == &row.path);
                                (
                                    row.path.clone(),
                                    row.name.clone(),
                                    row.is_dir,
                                    row.size,
                                    selected,
                                )
                            };
                            let response =
                                ui.allocate_response(Vec2::new(300.0, 108.0), Sense::click());
                            let card = if selected {
                                palette.selected
                            } else {
                                Color32::from_rgba_unmultiplied(255, 255, 255, 13)
                            };
                            ui.painter().rect(
                                response.rect.shrink(4.0),
                                CornerRadius::same(8),
                                card,
                                Stroke::new(
                                    1.0,
                                    if selected || response.hovered() {
                                        palette.accent
                                    } else {
                                        Color32::TRANSPARENT
                                    },
                                ),
                                egui::StrokeKind::Inside,
                            );
                            self.request_icon(&path, 96);
                            self.request_thumb(&path, is_dir);
                            let glyph = icons::type_glyph(is_dir, &name);
                            let image = Rect::from_center_size(
                                response.rect.left_center() + Vec2::new(48.0, 0.0),
                                Vec2::splat(64.0),
                            );
                            if let Some(texture) =
                                self.thumbs.get(&path).or_else(|| self.icons_tex.get(&path))
                            {
                                ui.painter().image(
                                    texture.id(),
                                    image,
                                    Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                                    Color32::WHITE,
                                );
                            } else {
                                ui.painter().text(
                                    image.center(),
                                    egui::Align2::CENTER_CENTER,
                                    icons::glyph(glyph),
                                    icon_font(36.0),
                                    palette.accent,
                                );
                            }
                            let text_x = image.right() + 12.0;
                            let kind = if is_dir {
                                self.tr.folder_kind.to_string()
                            } else {
                                platform::describe_type(&path, self.lang == crate::i18n::Lang::Fr)
                            };
                            let lines = if is_dir { 2.0 } else { 3.0 };
                            let mut text_y = response.rect.center().y - (lines * 16.0) / 2.0;
                            let name_rect =
                                Rect::from_min_size(pos2(text_x, text_y), Vec2::new(180.0, 16.0));
                            self.mark_layout("tile-icon", image);
                            self.mark_layout("tile-name", name_rect);
                            ui.painter().text(
                                pos2(text_x, text_y),
                                egui::Align2::LEFT_TOP,
                                truncate(&name, 28),
                                FontId::proportional(14.0),
                                palette.text,
                            );
                            text_y += 18.0;
                            ui.painter().text(
                                pos2(text_x, text_y),
                                egui::Align2::LEFT_TOP,
                                kind,
                                FontId::proportional(12.0),
                                palette.text_secondary,
                            );
                            if !is_dir {
                                ui.painter().text(
                                    pos2(text_x, response.rect.top() + 68.0),
                                    egui::Align2::LEFT_TOP,
                                    format_bytes(size, self.lang),
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
                            if response.secondary_clicked() {
                                context = Some(index);
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
        if let Some(index) = context {
            self.select_row(index, egui::Modifiers::NONE);
            if let Some(path) = self.visible_rows().get(index).map(|row| row.path.clone()) {
                if let Some(pos) = ui.ctx().input(|input| input.pointer.interact_pos()) {
                    self.context_menu = Some((pos, path));
                    self.layout_menu = false;
                    self.new_menu = false;
                }
            }
        }
    }

    fn draw_grid(&mut self, ui: &mut Ui, palette: FilesPalette, large: bool) {
        let cell = if large { 180.0 } else { 128.0 };
        let cols = ((ui.available_width() / cell).floor() as usize).max(1);
        let len = self.visible_rows().len();
        if len == 0 {
            ui.label(RichText::new(self.tr.empty).color(palette.text_secondary));
            return;
        }
        let mut clicked = None;
        let mut context = None;
        let mut opened = None;
        ScrollArea::vertical()
            .id_salt("icons")
            .auto_shrink([false, false])
            .show_rows(ui, cell + 8.0, len.div_ceil(cols), |ui, range| {
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
                                ui.allocate_response(Vec2::splat(cell - 8.0), Sense::click());
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
                            let icon_side = if large { 96.0 } else { 64.0 };
                            self.request_icon(&path, icon_side as u32);
                            self.request_thumb(&path, is_dir);
                            let image = Rect::from_center_size(
                                pos2(
                                    response.rect.center().x,
                                    response.rect.top() + 8.0 + icon_side / 2.0,
                                ),
                                Vec2::splat(icon_side),
                            );
                            self.mark_layout("grid-icon", image);
                            if let Some(texture) =
                                self.thumbs.get(&path).or_else(|| self.icons_tex.get(&path))
                            {
                                ui.painter().image(
                                    texture.id(),
                                    image,
                                    Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                                    Color32::WHITE,
                                );
                            } else {
                                ui.painter().text(
                                    image.center(),
                                    egui::Align2::CENTER_CENTER,
                                    icons::glyph(glyph),
                                    icon_font(icon_side * 0.6),
                                    palette.accent,
                                );
                            }
                            let (first, second) = split_label(&name, 14);
                            let name_pos = pos2(response.rect.center().x, image.bottom() + 8.0);
                            self.mark_layout(
                                "grid-name",
                                Rect::from_center_size(
                                    name_pos + Vec2::new(0.0, 8.0),
                                    Vec2::new(cell - 16.0, 16.0),
                                ),
                            );
                            ui.painter().text(
                                name_pos,
                                egui::Align2::CENTER_TOP,
                                first,
                                FontId::proportional(13.0),
                                palette.text,
                            );
                            if !second.is_empty() {
                                ui.painter().text(
                                    name_pos + Vec2::new(0.0, 16.0),
                                    egui::Align2::CENTER_TOP,
                                    &second,
                                    FontId::proportional(13.0),
                                    palette.text,
                                );
                            }
                            if is_dir {
                                let kind_y = if second.is_empty() { 16.0 } else { 32.0 };
                                ui.painter().text(
                                    name_pos + Vec2::new(0.0, kind_y),
                                    egui::Align2::CENTER_TOP,
                                    self.tr.folder_kind,
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
                            if response.secondary_clicked() {
                                context = Some(index);
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
        if let Some(index) = context {
            self.select_row(index, egui::Modifiers::NONE);
            if let Some(path) = self.visible_rows().get(index).map(|row| row.path.clone()) {
                if let Some(pos) = ui.ctx().input(|input| input.pointer.interact_pos()) {
                    self.context_menu = Some((pos, path));
                    self.layout_menu = false;
                    self.new_menu = false;
                }
            }
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
        if selected.is_empty() && !self.tab().home {
            let name = tab_title(self.tab(), &self.tr);
            let path = self.tab().path.clone();
            self.request_icon(&path, 96);
            if let Some(texture) = self.icons_tex.get(&path) {
                let (rect, _) = pane.allocate_exact_size(Vec2::splat(72.0), Sense::hover());
                pane.painter().image(
                    texture.id(),
                    rect,
                    Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                    Color32::WHITE,
                );
            }
            pane.label(RichText::new(name).color(palette.text).strong());
            detail_line(
                &mut pane,
                self.tr.items,
                &self.visible_rows().len().to_string(),
                palette,
            );
            detail_line(
                &mut pane,
                self.tr.path,
                &path.display().to_string(),
                palette,
            );
            return;
        }
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
            let kind = if row.is_dir {
                self.tr.folder_kind.to_string()
            } else {
                platform::describe_type(path, self.lang == crate::i18n::Lang::Fr)
            };
            detail_line(&mut pane, self.tr.kind, &kind, palette);
            detail_line(
                &mut pane,
                self.tr.size,
                &if row.is_dir && row.size == 0 {
                    "—".into()
                } else {
                    format_bytes(row.size, self.lang)
                },
                palette,
            );
            detail_line(
                &mut pane,
                self.tr.modified,
                &format_modified_lang(row.modified, self.lang),
                palette,
            );
        }
        detail_line(
            &mut pane,
            self.tr.path,
            &path.display().to_string(),
            palette,
        );
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
            if command_button(ui, icons::FOLDER, self.tr.files, palette).clicked() {
                self.pane = Pane::Files;
            }
            if command_button(ui, icons::DELETE, self.tr.delete, palette).clicked() {
                self.begin_delete(false);
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
                        let response = file_line(
                            ui,
                            icons::type_glyph(row.is_dir, &row.name),
                            &label,
                            "",
                            palette,
                            None,
                        );
                        if response.clicked() {
                            self.analyzer_selected = Some(row.path.clone());
                        }
                        if response.double_clicked() && row.is_dir {
                            self.request_analyze(row.path.clone());
                        }
                        if response.secondary_clicked() {
                            self.analyzer_selected = Some(row.path.clone());
                            if let Some(pos) = ui.ctx().input(|input| input.pointer.interact_pos())
                            {
                                self.context_menu = Some((pos, row.path.clone()));
                            }
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
                paint_treemap(ui, rect, &cells, palette, self.analyzer_selected.as_deref());
            }
            if let Some(pointer) = response.interact_pointer_pos() {
                if let Some(cell) = hit_cell(&cells, rect, pointer, self.sunburst) {
                    if response.secondary_clicked() {
                        self.analyzer_selected = Some(cell.path.clone());
                        self.context_menu = Some((pointer, cell.path.clone()));
                    } else if response.double_clicked() && cell.is_dir {
                        self.request_analyze(cell.path.clone());
                    } else if response.clicked() {
                        self.analyzer_selected = Some(cell.path.clone());
                    }
                }
            }
        });
    }

    fn draw_status(&mut self, ui: &mut Ui, rect: Rect, palette: FilesPalette) {
        let count = if self.tab().home && self.tab().search.is_empty() {
            0
        } else {
            self.visible_rows().len()
        };
        let y = rect.center().y;
        let mut text = format!("{count} {}", self.tr.items);
        let selected = self.tab().selected.len();
        if selected > 0 {
            text.push_str(&format!("    {selected} {}", self.tr.selected));
        }
        ui.painter().text(
            pos2(rect.left() + 12.0, y),
            egui::Align2::LEFT_CENTER,
            text,
            FontId::proportional(12.0),
            palette.text_secondary,
        );
        if !self.status.is_empty() {
            ui.painter().text(
                pos2(rect.right() - 12.0, y),
                egui::Align2::RIGHT_CENTER,
                truncate(&self.status, 72),
                FontId::proportional(12.0),
                palette.text_secondary,
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
        if self.delete_prompt.is_some() {
            self.paint_delete_dialog(ui);
        }
        if let Some((pos, path)) = self.context_menu.clone() {
            self.paint_context_menu(ui, pos, path);
        }
    }

    fn paint_delete_dialog(&mut self, ui: &mut Ui) {
        let Some(prompt) = self.delete_prompt.clone() else {
            return;
        };
        let palette = theme::palette(self.dark);
        let size = format_bytes(prompt.bytes, self.lang);
        let title = if prompt.permanent {
            self.tr.delete_permanently
        } else {
            self.tr.move_to_trash
        };
        let body = format!(
            "{}\n{}\n{} {}",
            prompt.label, size, prompt.items, self.tr.items
        );
        let screen = ui.ctx().content_rect();
        let dialog = Rect::from_center_size(screen.center(), Vec2::new(420.0, 188.0));
        let painter = ui.painter();
        painter.rect_filled(screen, CornerRadius::ZERO, Color32::from_black_alpha(90));
        painter.rect_filled(dialog, CornerRadius::same(8), palette.card);
        painter.rect_stroke(
            dialog,
            CornerRadius::same(8),
            Stroke::new(1.0, palette.stroke),
            egui::StrokeKind::Inside,
        );
        painter.text(
            dialog.left_top() + Vec2::new(20.0, 20.0),
            egui::Align2::LEFT_TOP,
            title,
            FontId::proportional(16.0),
            palette.text,
        );
        painter.text(
            dialog.left_top() + Vec2::new(20.0, 52.0),
            egui::Align2::LEFT_TOP,
            body,
            FontId::proportional(14.0),
            palette.text_secondary,
        );
        let mut child = ui.new_child(UiBuilder::new().max_rect(dialog));
        child.add_space(128.0);
        child.horizontal(|child| {
            child.add_space(20.0);
            if command_button(child, icons::DISMISS, self.tr.cancel, palette).clicked() {
                self.delete_prompt = None;
            }
            if command_button(child, icons::DELETE, self.tr.delete, palette).clicked() {
                self.commit_delete();
            }
        });
    }

    fn paint_context_menu(&mut self, ui: &mut Ui, pos: egui::Pos2, path: PathBuf) {
        let palette = theme::palette(self.dark);
        let rows = [
            (self.tr.open, Action::Open),
            (self.tr.copy, Action::Copy),
            (self.tr.cut, Action::Cut),
            (self.tr.paste, Action::Paste),
            (self.tr.rename, Action::Rename),
            (self.tr.delete, Action::Trash),
            (self.tr.delete_permanently, Action::DeletePermanent),
            (self.tr.properties, Action::TogglePreview),
        ];
        let width = 240.0;
        let height = 8.0 + rows.len() as f32 * 34.0;
        let rect = Rect::from_min_size(pos, Vec2::new(width, height));
        let painter = ui.painter();
        painter.rect_filled(
            rect.translate(Vec2::new(0.0, 3.0)),
            CornerRadius::same(8),
            Color32::from_black_alpha(80),
        );
        painter.rect_filled(rect, CornerRadius::same(8), palette.card);
        painter.rect_stroke(
            rect,
            CornerRadius::same(8),
            Stroke::new(1.0, palette.stroke),
            egui::StrokeKind::Inside,
        );
        let mut child = ui.new_child(
            UiBuilder::new()
                .max_rect(rect.shrink(4.0))
                .layout(Layout::top_down(Align::Min)),
        );
        let mut chosen = None;
        for (label, action) in rows {
            if command_button(&mut child, icons::CHEVRON_RIGHT, label, palette).clicked() {
                chosen = Some(action);
            }
        }
        if let Some(action) = chosen {
            if self.pane != Pane::Analyzer {
                self.tab_mut().selected = vec![path.clone()];
            }
            if action == Action::TogglePreview {
                self.show_preview = true;
                let _ = self.commands.send(Command::Preview { path });
            } else if action == Action::Open && self.pane == Pane::Analyzer && path.is_dir() {
                self.request_analyze(path);
            } else if action == Action::Open {
                self.open_selection();
            } else {
                self.perform(action);
            }
            self.context_menu = None;
        } else if ui.input(|input| input.pointer.primary_clicked()) {
            let inside = ui
                .input(|input| input.pointer.interact_pos())
                .is_some_and(|pointer| rect.contains(pointer));
            if !inside {
                self.context_menu = None;
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

fn drive_crumb(app: &RustFilesApp, label: &str, path: &std::path::Path) -> String {
    let trimmed = label.trim_end_matches(['\\', '/']);
    let is_drive = trimmed.len() == 2 && trimmed.as_bytes().get(1) == Some(&b':');
    if !is_drive && label != "/" {
        return label.to_string();
    }
    app.volumes
        .iter()
        .find(|volume| {
            let mount = volume.mount.trim_end_matches(['\\', '/']);
            mount.eq_ignore_ascii_case(trimmed)
                || volume
                    .mount
                    .eq_ignore_ascii_case(&path.display().to_string())
        })
        .map(|volume| drive_title(&volume.label, &volume.mount, app.lang))
        .unwrap_or_else(|| label.to_string())
}

fn snap_pos(ui: &Ui, pos: egui::Pos2) -> egui::Pos2 {
    let scale = ui.pixels_per_point();
    egui::pos2(
        (pos.x * scale).round() / scale,
        (pos.y * scale).round() / scale,
    )
}

fn paint_caption_glyph(ui: &Ui, button: Rect, id: &str, maximized: bool, color: Color32) {
    let center = snap_pos(ui, button.center());
    let painter = ui.painter();
    let stroke = Stroke::new(1.0, color);
    match id {
        "min" => {
            painter.hline((center.x - 5.0)..=(center.x + 5.0), center.y, stroke);
        }
        "max" if maximized => {
            let back = Rect::from_center_size(center + Vec2::new(1.0, -1.0), Vec2::splat(8.0));
            let front = Rect::from_center_size(center + Vec2::new(-1.0, 1.0), Vec2::splat(8.0));
            painter.rect_stroke(back, CornerRadius::ZERO, stroke, egui::StrokeKind::Inside);
            painter.rect_filled(front, CornerRadius::ZERO, Color32::TRANSPARENT);
            painter.rect_stroke(front, CornerRadius::ZERO, stroke, egui::StrokeKind::Inside);
        }
        "max" => {
            let mark = Rect::from_center_size(center, Vec2::splat(10.0));
            painter.rect_stroke(mark, CornerRadius::ZERO, stroke, egui::StrokeKind::Inside);
        }
        _ => {
            let half = 5.0;
            painter.line_segment(
                [
                    center + Vec2::new(-half, -half),
                    center + Vec2::new(half, half),
                ],
                stroke,
            );
            painter.line_segment(
                [
                    center + Vec2::new(half, -half),
                    center + Vec2::new(-half, half),
                ],
                stroke,
            );
        }
    }
}

fn flyout<T: Copy + PartialEq>(
    ctx: &egui::Context,
    pos: egui::Pos2,
    openness: f32,
    rows: &[(T, &str, u32)],
    current: Option<T>,
    palette: FilesPalette,
) -> Option<T> {
    let width = 220.0;
    let height = 8.0 + rows.len() as f32 * 34.0;
    let rect = Rect::from_min_size(pos, Vec2::new(width, height));
    let mut chosen = None;
    egui::Area::new(egui::Id::unique("layout-flyout-area"))
        .fixed_pos(rect.min)
        .order(egui::Order::Foreground)
        .show(ctx, |ui| {
            let painter = ui.painter();
            painter.rect_filled(
                rect.translate(Vec2::new(0.0, 4.0)),
                CornerRadius::same(8),
                Color32::from_black_alpha((120.0 * openness) as u8),
            );
            painter.rect_filled(
                rect,
                CornerRadius::same(8),
                palette.card.gamma_multiply(openness),
            );
            painter.rect_stroke(
                rect,
                CornerRadius::same(8),
                Stroke::new(1.0, palette.stroke),
                egui::StrokeKind::Inside,
            );
            ui.set_min_size(rect.size());
            ui.add_space(4.0);
            for (mode, label, glyph) in rows {
                let (row, response) =
                    ui.allocate_exact_size(Vec2::new(width - 8.0, 34.0), Sense::click());
                if response.hovered() {
                    ui.painter()
                        .rect_filled(row, CornerRadius::same(4), palette.hover);
                }
                let checked = current.is_some_and(|current| current == *mode);
                ui.painter().text(
                    row.left_center() + Vec2::new(12.0, 0.0),
                    egui::Align2::LEFT_CENTER,
                    icons::glyph(*glyph),
                    icon_font(16.0),
                    palette.text,
                );
                ui.painter().text(
                    row.left_center() + Vec2::new(36.0, 0.0),
                    egui::Align2::LEFT_CENTER,
                    *label,
                    FontId::proportional(14.0),
                    palette.text,
                );
                if checked {
                    ui.painter().text(
                        row.right_center() - Vec2::new(16.0, 0.0),
                        egui::Align2::RIGHT_CENTER,
                        icons::glyph(icons::CHECK),
                        icon_font(14.0),
                        palette.accent,
                    );
                }
                if response.clicked() {
                    chosen = Some(*mode);
                }
            }
        });
    chosen
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
    let width = (title.chars().count() as f32 * 8.0 + 72.0).clamp(180.0, 240.0);
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, 40.0), Sense::click());
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
    if !selected {
        ui.painter().vline(
            rect.right(),
            rect.shrink(10.0).y_range(),
            Stroke::new(1.0, palette.stroke),
        );
    }
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
    let width = 44.0 + label.chars().count() as f32 * 8.2;
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
fn icon_button(
    ui: &mut Ui,
    glyph: u32,
    tip: &str,
    palette: FilesPalette,
    enabled: bool,
) -> egui::Response {
    let sense = if enabled {
        Sense::click()
    } else {
        Sense::hover()
    };
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(32.0), sense);
    let hover = ui
        .ctx()
        .animate_bool_with_time(response.id, enabled && response.hovered(), 0.1);
    if hover > 0.01 {
        ui.painter().rect_filled(
            rect,
            CornerRadius::same(4),
            palette.hover.gamma_multiply(hover),
        );
    }
    let color = if enabled {
        palette.text
    } else {
        palette.text_secondary.gamma_multiply(0.45)
    };
    ui.painter().text(
        snap_pos(ui, rect.center()),
        egui::Align2::CENTER_CENTER,
        icons::glyph(glyph),
        icon_font(16.0),
        color,
    );
    ui.add_space(2.0);
    response.on_hover_text(tip)
}

fn bar_separator(ui: &mut Ui, palette: FilesPalette) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(12.0, 24.0), Sense::hover());
    ui.painter().vline(
        rect.center().x,
        rect.y_range(),
        Stroke::new(1.0, palette.stroke),
    );
}

fn crumb_chevron(ui: &mut Ui, palette: FilesPalette) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::new(16.0, 28.0), Sense::click());
    if response.hovered() {
        ui.painter()
            .rect_filled(rect, CornerRadius::same(4), palette.hover);
    }
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        icons::glyph(icons::CHEVRON_RIGHT),
        icon_font(12.0),
        palette.text_secondary,
    );
    response
}

fn crumb_children(
    parent: Option<&std::path::Path>,
    volumes: &[crate::platform::DetectedVolume],
    lang: crate::i18n::Lang,
) -> Vec<(String, PathBuf)> {
    let Some(parent) = parent else {
        return volumes
            .iter()
            .map(|volume| {
                (
                    drive_title(&volume.label, &volume.mount, lang),
                    PathBuf::from(&volume.mount),
                )
            })
            .collect();
    };
    let mut entries = Vec::new();
    let Ok(read) = std::fs::read_dir(parent) else {
        return entries;
    };
    for entry in read.flatten() {
        let path = entry.path();
        if path.is_dir() {
            let name = entry.file_name().to_string_lossy().into_owned();
            entries.push((name, path));
        }
        if entries.len() == 40 {
            break;
        }
    }
    entries.sort_by_key(|entry| entry.0.to_lowercase());
    entries
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

fn section_header(
    ui: &mut Ui,
    title: &str,
    open: &mut bool,
    mark: u32,
    mark_color: Color32,
    palette: FilesPalette,
) -> bool {
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
        rect.left_center() + Vec2::new(28.0, 0.0),
        egui::Align2::LEFT_CENTER,
        icons::glyph(mark),
        icon_font(16.0),
        mark_color,
    );
    ui.painter().text(
        rect.left_center() + Vec2::new(52.0, 0.0),
        egui::Align2::LEFT_CENTER,
        title,
        FontId::proportional(13.0),
        palette.text,
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
    if selected {
        ui.painter().rect_filled(
            Rect::from_min_size(rect.left_top(), Vec2::new(3.0, rect.height())),
            CornerRadius::ZERO,
            palette.accent,
        );
    }
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
        pos2(icon_rect.right() + 12.0, rect.center().y),
        egui::Align2::LEFT_CENTER,
        truncate(label, 28),
        FontId::proportional(14.0),
        palette.text,
    );
    if glyph == icons::HARD_DRIVE {
        ui.painter().text(
            rect.right_center() - Vec2::new(14.0, 0.0),
            egui::Align2::RIGHT_CENTER,
            icons::glyph(icons::CHEVRON_RIGHT),
            icon_font(12.0),
            palette.text_secondary,
        );
    }
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
    if selected {
        ui.painter().rect_filled(
            Rect::from_min_size(rect.left_top(), Vec2::new(3.0, rect.height())),
            CornerRadius::ZERO,
            palette.accent,
        );
    }
    let y = rect.center().y;
    let icon_rect = Rect::from_center_size(pos2(rect.left() + 12.0 + 10.0, y), Vec2::splat(20.0));
    ui.painter().text(
        icon_rect.center(),
        egui::Align2::CENTER_CENTER,
        icons::glyph(glyph),
        icon_font(18.0),
        if selected {
            palette.accent
        } else {
            palette.text
        },
    );
    ui.painter().text(
        pos2(icon_rect.right() + 12.0, y),
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
fn drive_title(label: &str, mount: &str, lang: crate::i18n::Lang) -> String {
    let trimmed = mount.trim_end_matches(['\\', '/']);
    let letter = if trimmed.len() == 2 && trimmed.as_bytes()[1] == b':' {
        trimmed.to_string()
    } else {
        trimmed
            .rsplit(['\\', '/'])
            .next()
            .unwrap_or(trimmed)
            .to_string()
    };
    if label.is_empty() || label == mount || label.eq_ignore_ascii_case(&letter) {
        match lang {
            crate::i18n::Lang::Fr => format!("Disque local ({letter})"),
            crate::i18n::Lang::En => format!("Local disk ({letter})"),
        }
    } else {
        format!("{label} ({letter})")
    }
}

fn drive_card(
    ui: &mut Ui,
    label: &str,
    used: u64,
    total: u64,
    palette: FilesPalette,
    lang: crate::i18n::Lang,
    icon: Option<&egui::TextureHandle>,
) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::new(260.0, 112.0), Sense::click());
    ui.painter().rect(
        rect,
        CornerRadius::same(8),
        palette.card,
        Stroke::new(1.0, palette.stroke),
        egui::StrokeKind::Inside,
    );
    let icon_rect = Rect::from_min_size(rect.left_top() + Vec2::new(16.0, 16.0), Vec2::splat(40.0));
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
            icons::glyph(icons::HARD_DRIVE),
            icon_font(22.0),
            palette.accent,
        );
    }
    ui.painter().text(
        pos2(icon_rect.right() + 12.0, icon_rect.center().y),
        egui::Align2::LEFT_CENTER,
        truncate(label, 22),
        FontId::proportional(15.0),
        palette.text,
    );
    let caption = if total == 0 {
        String::new()
    } else {
        format_free(total - used, total, lang)
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
    let y = rect.center().y;
    let icon_rect = Rect::from_center_size(pos2(rect.left() + 18.0, y), Vec2::splat(20.0));
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
        pos2(icon_rect.right() + 8.0, y),
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
fn quick_card(
    ui: &mut Ui,
    label: &str,
    palette: FilesPalette,
    icon: Option<&egui::TextureHandle>,
) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::new(168.0, 64.0), Sense::click());
    ui.painter().rect(
        rect,
        CornerRadius::same(8),
        palette.card,
        Stroke::new(1.0, palette.stroke),
        egui::StrokeKind::Inside,
    );
    let y = rect.center().y;
    let icon_rect = Rect::from_center_size(pos2(rect.left() + 24.0, y), Vec2::splat(32.0));
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
            icons::glyph(icons::FOLDER),
            icon_font(22.0),
            palette.accent,
        );
    }
    ui.painter().text(
        pos2(icon_rect.right() + 8.0, y),
        egui::Align2::LEFT_CENTER,
        truncate(label, 16),
        FontId::proportional(14.0),
        palette.text,
    );
    ui.add_space(8.0);
    response
}

fn split_label(text: &str, width: usize) -> (String, String) {
    let chars: Vec<char> = text.chars().collect();
    if chars.len() <= width {
        return (text.to_string(), String::new());
    }
    let first: String = chars.iter().take(width).collect();
    let mut second: String = chars.iter().skip(width).take(width).collect();
    if chars.len() > width * 2 {
        second = format!(
            "{}…",
            second
                .chars()
                .take(width.saturating_sub(1))
                .collect::<String>()
        );
    }
    (first, second)
}

#[allow(dead_code)]
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
#[allow(dead_code)]
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
#[allow(dead_code)]
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
fn paint_treemap(
    ui: &Ui,
    rect: Rect,
    cells: &[crate::worker::MapCell],
    palette: FilesPalette,
    selected: Option<&std::path::Path>,
) {
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
        if selected.is_some_and(|path| path == cell.path) {
            ui.painter().rect_stroke(
                visual.shrink(1.0),
                CornerRadius::same(3),
                Stroke::new(2.0, palette.accent),
                egui::StrokeKind::Inside,
            );
        }
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
