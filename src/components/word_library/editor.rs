//! 用户词库编辑器组件。
//!
//! `WordEditor` 使用共享状态维护结构化词条、JSON 原文、搜索结果和弹层状态。
//! 本文件按“数据加载与校验、表格分页编辑、搜索定位、批量导入导出、保存反馈”
//! 组织代码；表格每页最多展示 `TABLE_PAGE_SIZE` 条，以控制大词库的绘制成本。

use crate::{WORD_EDITOR_FONT_FAMILY, prepare_word_library_file, ui_theme, word_library_directory};
use eframe::egui;
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::sync::{Arc, Mutex, MutexGuard};

const TABLE_PAGE_SIZE: usize = 50;
// 保证打开编辑器时可以同时看到多行词条，而不是为了操作栏过度压缩表格区域。
const MIN_EDITOR_VIEWPORT_HEIGHT: f32 = 220.0;

#[derive(Clone, Default)]
pub(crate) struct WordEditor {
    state: Arc<Mutex<WordEditorState>>,
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum EditorMode {
    #[default]
    Table,
    AdvancedJson,
}

#[derive(Clone, Default)]
struct WordRow {
    key: String,
    value: String,
}

#[derive(Default)]
struct WordEditorState {
    open: bool,
    file_name: String,
    mode: EditorMode,
    rows: Vec<WordRow>,
    content: String,
    message: Option<(String, bool)>,
    search_query: String,
    search_from: usize,
    pending_selection: Option<(usize, usize)>,
    error_line: Option<usize>,
    pending_scroll_char: Option<usize>,
    batch_import_open: bool,
    batch_import_content: String,
    batch_export_open: bool,
    batch_export_file_name: String,
    scroll_table_to_top: bool,
    table_page: usize,
    reload_requested: bool,
}

impl WordEditor {
    pub(crate) fn is_open(&self) -> bool {
        self.lock().open
    }

    pub(crate) fn open(&self, file_name: &str) {
        let mut state = self.lock();
        state.open = true;
        state.file_name = file_name.to_string();
        state.mode = EditorMode::Table;
        state.message = None;
        state.search_query.clear();
        state.search_from = 0;
        state.pending_selection = None;
        state.error_line = None;
        state.batch_import_open = false;
        state.batch_export_open = false;
        state.scroll_table_to_top = false;
        state.table_page = 0;

        let path = match prepare_word_library_file(file_name) {
            Ok(path) => path,
            Err(error) => {
                state.rows.clear();
                state.content.clear();
                state.message = Some((format!("无法准备词库目录：{error:#}"), true));
                return;
            }
        };
        match fs::read_to_string(&path) {
            Ok(content) => {
                state.content = content;
                match parse_words(&state.content) {
                    Ok(words) => {
                        state.rows = map_to_rows(&words);
                        state.message = Some((format!("已载入 {} 个词条。", words.len()), false));
                    }
                    Err(error) => {
                        state.rows.clear();
                        state.mode = EditorMode::AdvancedJson;
                        state.show_json_error("现有词库格式错误，已进入高级编辑模式", &error);
                    }
                }
            }
            Err(error) => {
                state.rows.clear();
                state.content.clear();
                state.message = Some((format!("无法读取 {file_name}：{error}"), true));
            }
        }
    }

    pub(crate) fn close(&self) {
        let mut state = self.lock();
        state.open = false;
        state.batch_import_open = false;
        state.batch_export_open = false;
    }

    pub(crate) fn take_reload_requested(&self) -> bool {
        let mut state = self.lock();
        std::mem::take(&mut state.reload_requested)
    }

    pub(crate) fn show(&self, ui: &mut egui::Ui) {
        let mut state = self.lock();
        if !state.open {
            return;
        }

        ui_theme::apply(ui);
        ui.painter()
            .rect_filled(ui.max_rect(), 0.0, ui_theme::canvas());

        if ui.input(|input| input.key_pressed(egui::Key::Escape)) {
            if state.batch_import_open {
                state.batch_import_open = false;
            } else if state.batch_export_open {
                state.batch_export_open = false;
            } else {
                state.open = false;
            }
            return;
        }

        if state.batch_import_open {
            if state.draw_batch_import(ui) {
                state.batch_import_open = false;
            }
        } else if state.batch_export_open {
            if state.draw_batch_export(ui) {
                state.batch_export_open = false;
            }
        } else if state.draw_editor(ui) {
            state.open = false;
        }
    }

    fn lock(&self) -> MutexGuard<'_, WordEditorState> {
        self.state.lock().unwrap_or_else(|error| error.into_inner())
    }
}

impl WordEditorState {
    fn draw_editor(&mut self, ui: &mut egui::Ui) -> bool {
        ui.spacing_mut().item_spacing = egui::vec2(10.0, 10.0);
        let mut close = false;

        egui::Frame::new()
            .inner_margin(egui::Margin::same(24))
            .show(ui, |ui| {
                if back_button(ui, "返回设置").clicked() {
                    close = true;
                }
                ui.add_space(8.0);
                ui.label(
                    egui::RichText::new("TO WORDS / LIBRARY")
                        .monospace()
                        .size(11.0)
                        .color(ui_theme::muted_text()),
                );
                ui.label(
                    egui::RichText::new("词库编辑器")
                        .size(27.0)
                        .strong()
                        .color(ui_theme::primary_text()),
                );
                ui.label(
                    egui::RichText::new("使用结构化表格维护词条，或在高级模式中直接编辑 JSON。")
                        .size(13.0)
                        .color(ui_theme::muted_text()),
                );
                ui.label(
                    egui::RichText::new(format!("当前文件：{}", self.file_name))
                        .monospace()
                        .size(12.0)
                        .color(ui_theme::muted_text()),
                );
                ui.add_space(16.0);
                self.draw_mode_toolbar(ui);
                match self.mode {
                    EditorMode::Table => self.draw_table_editor(ui),
                    EditorMode::AdvancedJson => self.draw_json_editor(ui),
                }

                ui.add_space(15.0);
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 12.0;
                    if self.mode == EditorMode::AdvancedJson
                        && action_button(
                            ui,
                            "检查格式",
                            ui_theme::control_surface(),
                            ui_theme::accent_text(),
                        )
                        .clicked()
                    {
                        self.check_format();
                    }
                    if action_button(
                        ui,
                        "保存",
                        egui::Color32::from_rgb(31, 31, 30),
                        egui::Color32::WHITE,
                    )
                    .clicked()
                        && self.save()
                    {
                        close = true;
                    }
                    if action_button(
                        ui,
                        "取消",
                        ui_theme::control_surface(),
                        ui_theme::primary_text(),
                    )
                    .clicked()
                    {
                        close = true;
                    }
                });

                if let Some((message, is_error)) = &self.message {
                    editor_status(ui, message, *is_error);
                }
            });

        close
    }

    fn draw_mode_toolbar(&mut self, ui: &mut egui::Ui) {
        let mut requested_mode = self.mode;
        ui.horizontal_wrapped(|ui| {
            if toolbar_button(ui, "结构化表格", self.mode == EditorMode::Table, 112.0).clicked()
            {
                requested_mode = EditorMode::Table;
            }
            if toolbar_button(
                ui,
                "高级 JSON 编辑",
                self.mode == EditorMode::AdvancedJson,
                132.0,
            )
            .clicked()
            {
                requested_mode = EditorMode::AdvancedJson;
            }
            ui.add_space(6.0);
            if self.mode == EditorMode::Table {
                if filled_toolbar_button(
                    ui,
                    "＋ 新增",
                    88.0,
                    egui::Color32::from_rgb(31, 31, 30),
                    egui::Color32::WHITE,
                )
                .clicked()
                {
                    self.rows.insert(0, WordRow::default());
                    self.table_page = 0;
                    self.scroll_table_to_top = true;
                }
                if filled_toolbar_button(
                    ui,
                    "批量导入",
                    104.0,
                    ui_theme::control_surface(),
                    ui_theme::accent_text(),
                )
                .clicked()
                {
                    self.batch_import_content.clear();
                    self.batch_import_open = true;
                }
                if ui_theme::opaque_hover_text(
                    filled_toolbar_button(
                        ui,
                        "批量导出",
                        104.0,
                        ui_theme::pale_green(),
                        ui_theme::green_text(),
                    ),
                    "将格式化后的 JSON 导出到 word_libraries 词库目录",
                )
                .clicked()
                {
                    self.batch_export_file_name = "user_words_export.json".to_string();
                    self.batch_export_open = true;
                }
            }
        });

        if requested_mode != self.mode {
            match requested_mode {
                EditorMode::AdvancedJson => {
                    let (words, _) = clean_rows(&self.rows);
                    if let Ok(content) = format_words(&words) {
                        self.content = content;
                        self.mode = EditorMode::AdvancedJson;
                    }
                }
                EditorMode::Table => match parse_words(&self.content) {
                    Ok(words) => {
                        self.rows = map_to_rows(&words);
                        self.error_line = None;
                        self.table_page = 0;
                        self.scroll_table_to_top = true;
                        self.mode = EditorMode::Table;
                    }
                    Err(error) => self.show_json_error("无法切换到表格，JSON 格式错误", &error),
                },
            }
        }
    }

    fn draw_table_editor(&mut self, ui: &mut egui::Ui) {
        ui.label("默认使用 key/value 表格编辑；保存时会自动剔除重复 key 和空白项。");
        let mut search_changed = false;
        egui::Frame::new()
            .fill(ui_theme::surface())
            .stroke(egui::Stroke::new(1.0, ui_theme::border()))
            .corner_radius(8)
            .inner_margin(egui::Margin::symmetric(10, 5))
            .show(ui, |ui| {
                search_changed = ui
                    .add(
                        egui::TextEdit::singleline(&mut self.search_query)
                            .hint_text("筛选 key 或 value…")
                            .font(editor_font(15.0))
                            .desired_width(f32::INFINITY)
                            .frame(egui::Frame::NONE),
                    )
                    .changed();
            });
        if search_changed {
            self.table_page = 0;
            self.scroll_table_to_top = true;
        }

        let editor_height = (ui.available_height() - 158.0).max(MIN_EDITOR_VIEWPORT_HEIGHT);
        let query = self.search_query.trim();
        let filtered_indices = (!query.is_empty()).then(|| {
            let query = query.to_lowercase();
            self.rows
                .iter()
                .enumerate()
                .filter_map(|(index, row)| {
                    (row.key.to_lowercase().contains(&query)
                        || row.value.to_lowercase().contains(&query))
                    .then_some(index)
                })
                .collect::<Vec<_>>()
        });
        let visible_rows = filtered_indices.as_ref().map_or(self.rows.len(), Vec::len);
        let (table_page, page_count, page_start, page_end) =
            table_page_bounds(visible_rows, self.table_page);
        self.table_page = table_page;
        let page_rows = page_end.saturating_sub(page_start);
        let mut delete_index = None;
        let table_width = (ui.available_width() - 14.0).max(500.0);
        let row_content_width = table_width - 20.0;
        let action_width = 52.0;
        let field_width = (row_content_width - action_width * 2.0 - 24.0).max(300.0);
        let key_width = (field_width * 0.38).max(120.0);
        let value_width = (field_width - key_width).max(180.0);
        let scroll_to_top = std::mem::take(&mut self.scroll_table_to_top);

        ui.scope(|ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            egui::Frame::new()
                .fill(ui_theme::control_surface())
                .stroke(egui::Stroke::new(1.0, ui_theme::border()))
                .inner_margin(egui::Margin::symmetric(10, 6))
                .show(ui, |ui| {
                    ui.set_width(row_content_width);
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 8.0;
                        ui.add_sized(
                            [key_width, 22.0],
                            egui::Label::new(egui::RichText::new("Key").strong()),
                        );
                        ui.add_sized(
                            [value_width, 22.0],
                            egui::Label::new(egui::RichText::new("Value").strong()),
                        );
                        ui.add_sized([action_width, 22.0], egui::Label::new("复制"));
                        ui.add_sized([action_width, 22.0], egui::Label::new("删除"));
                    });
                });

            let mut scroll_area = egui::ScrollArea::vertical()
                .id_salt("structured_words_scroll")
                .max_height(editor_height)
                .auto_shrink([false, false]);
            if scroll_to_top {
                scroll_area = scroll_area.vertical_scroll_offset(0.0);
            }
            scroll_area.show_rows(ui, 42.0, page_rows, |ui, visible_range| {
                ui.set_width(table_width);
                ui.spacing_mut().item_spacing.y = 0.0;
                for page_index in visible_range {
                    let visible_index = page_start + page_index;
                    let index = filtered_indices
                        .as_ref()
                        .map_or(visible_index, |indices| indices[visible_index]);
                    let row = &mut self.rows[index];
                    egui::Frame::new()
                        .fill(ui_theme::surface())
                        .stroke(egui::Stroke::new(1.0, ui_theme::border()))
                        .inner_margin(egui::Margin::symmetric(10, 4))
                        .show(ui, |ui| {
                            ui.set_width(row_content_width);
                            ui.horizontal(|ui| {
                                ui.spacing_mut().item_spacing.x = 8.0;
                                ui.add_sized(
                                    [key_width, 34.0],
                                    egui::TextEdit::singleline(&mut row.key)
                                        .font(editor_font(15.0))
                                        .margin(egui::Margin::symmetric(8, 5)),
                                );
                                ui.add_sized(
                                    [value_width, 34.0],
                                    egui::TextEdit::singleline(&mut row.value)
                                        .font(editor_font(15.0))
                                        .margin(egui::Margin::symmetric(8, 5)),
                                );
                                if row_action_button(
                                    ui,
                                    "复制",
                                    ui_theme::accent_fill(),
                                    ui_theme::accent_text(),
                                )
                                .clicked()
                                {
                                    ui.ctx().copy_text(format!("{}\t{}", row.key, row.value));
                                }
                                if row_action_button(
                                    ui,
                                    "删除",
                                    ui_theme::pale_red(),
                                    ui_theme::red_text(),
                                )
                                .clicked()
                                {
                                    delete_index = Some(index);
                                }
                            });
                        });
                }
            });
        });
        if visible_rows == 0 {
            ui.centered_and_justified(|ui| {
                ui.label(egui::RichText::new("没有符合筛选条件的词条").weak());
            });
        } else {
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                if page_button(ui, "上一页", self.table_page > 0).clicked() {
                    self.table_page -= 1;
                    self.scroll_table_to_top = true;
                }
                ui.label(
                    egui::RichText::new(format!(
                        "第 {} / {} 页　{}–{} / {} 条",
                        self.table_page + 1,
                        page_count,
                        page_start + 1,
                        page_end,
                        visible_rows
                    ))
                    .monospace()
                    .size(12.0)
                    .color(ui_theme::muted_text()),
                );
                if page_button(ui, "下一页", self.table_page + 1 < page_count).clicked() {
                    self.table_page += 1;
                    self.scroll_table_to_top = true;
                }
            });
        }
        if let Some(index) = delete_index {
            self.rows.remove(index);
        }
    }

    fn draw_json_editor(&mut self, ui: &mut egui::Ui) {
        ui.label("高级模式：直接修改 JSON。key 和 value 都必须是字符串。");
        let search_id = egui::Id::new("user_words_search_input");
        let editor_id = egui::Id::new("user_words_json_editor");
        if ui.memory(|memory| memory.has_focus(search_id)) && !self.search_query.is_empty() {
            let enter_pressed = ui.input_mut(|input| {
                let modifiers = input.modifiers;
                input.consume_key(modifiers, egui::Key::Enter)
            });
            if enter_pressed {
                self.find_next();
            }
        }
        self.draw_json_search(ui, search_id);

        let editor_height = (ui.available_height() - 118.0).max(MIN_EDITOR_VIEWPORT_HEIGHT);
        let pending_selection = self.pending_selection.take();
        egui::ScrollArea::both()
            .id_salt("user_words_json_scroll")
            .max_height(editor_height)
            .show(ui, |ui| {
                let editor_width = ui.available_width().max(520.0);
                let desired_rows = ((editor_height - 20.0) / 22.0).max(1.0) as usize;
                let error_range = self
                    .error_line
                    .and_then(|line| line_byte_range(&self.content, line));
                let font_id = editor_font(15.0);
                let text_color = ui.visuals().text_color();
                let mut layouter =
                    move |ui: &egui::Ui, text: &dyn egui::TextBuffer, wrap_width: f32| {
                        let mut job = egui::text::LayoutJob::default();
                        job.wrap.max_width = wrap_width;
                        let normal = egui::TextFormat {
                            font_id: font_id.clone(),
                            color: text_color,
                            ..Default::default()
                        };
                        if let Some(range) = error_range.clone().filter(|range| {
                            range.end <= text.as_str().len()
                                && text.as_str().is_char_boundary(range.start)
                                && text.as_str().is_char_boundary(range.end)
                        }) {
                            job.append(&text.as_str()[..range.start], 0.0, normal.clone());
                            let mut error_format = normal.clone();
                            error_format.background = ui_theme::pale_red();
                            job.append(&text.as_str()[range.clone()], 0.0, error_format);
                            job.append(&text.as_str()[range.end..], 0.0, normal);
                        } else {
                            job.append(text.as_str(), 0.0, normal);
                        }
                        ui.fonts_mut(|fonts| fonts.layout_job(job))
                    };
                let mut output = egui::TextEdit::multiline(&mut self.content)
                    .id(editor_id)
                    .code_editor()
                    .font(editor_font(15.0))
                    .layouter(&mut layouter)
                    .desired_width(editor_width)
                    .desired_rows(desired_rows)
                    .background_color(ui_theme::surface())
                    .margin(egui::Margin::same(10))
                    .show(ui);
                if let Some((start, end)) = pending_selection {
                    use egui::text::{CCursor, CCursorRange};
                    output.state.cursor.set_char_range(Some(CCursorRange::two(
                        CCursor::new(start),
                        CCursor::new(end),
                    )));
                    output.state.store(ui.ctx(), editor_id);
                    output.response.request_focus();
                    let cursor_rect = output
                        .galley
                        .pos_from_cursor(CCursor::new(start))
                        .translate(output.galley_pos.to_vec2());
                    ui.scroll_to_rect(cursor_rect, Some(egui::Align::Center));
                } else if let Some(position) = self.pending_scroll_char.take() {
                    use egui::text::CCursor;
                    let cursor_rect = output
                        .galley
                        .pos_from_cursor(CCursor::new(position))
                        .translate(output.galley_pos.to_vec2());
                    ui.scroll_to_rect(cursor_rect, Some(egui::Align::Center));
                }
                if output.response.changed() {
                    self.search_from = 0;
                    self.error_line = None;
                }
            });
    }

    fn draw_json_search(&mut self, ui: &mut egui::Ui, search_id: egui::Id) {
        let mut search = false;
        egui::Frame::new()
            .fill(ui_theme::surface())
            .stroke(egui::Stroke::new(1.0, ui_theme::border()))
            .corner_radius(8)
            .inner_margin(egui::Margin::symmetric(10, 6))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    let search_width = (ui.available_width() - 105.0).max(180.0);
                    let response = ui.add(
                        egui::TextEdit::singleline(&mut self.search_query)
                            .id(search_id)
                            .hint_text("搜索中文或韩文…")
                            .font(editor_font(15.0))
                            .desired_width(search_width)
                            .frame(egui::Frame::NONE),
                    );
                    if response.changed() {
                        self.search_from = 0;
                    }
                    search |= ui
                        .add_sized([88.0, 34.0], egui::Button::new("查找下一个"))
                        .clicked();
                });
            });
        if search {
            self.find_next();
        }
    }

    fn draw_batch_import(&mut self, ui: &mut egui::Ui) -> bool {
        ui.painter()
            .rect_filled(ui.max_rect(), 0.0, ui_theme::canvas());
        let mut import = false;
        let mut close = false;
        if back_button(ui, "返回词库编辑").clicked() {
            return true;
        }
        ui.add_space(10.0);
        egui::Frame::new()
            .fill(ui_theme::surface())
            .stroke(egui::Stroke::new(1.0, ui_theme::border()))
            .corner_radius(8)
            .inner_margin(egui::Margin::same(18))
            .show(ui, |ui| {
                ui.label(
                    egui::RichText::new("BATCH IMPORT")
                        .monospace()
                        .size(11.0)
                        .color(ui_theme::muted_text()),
                );
                ui.label("粘贴 JSON 对象；导入内容会追加到现有表格，重复 key 在保存时去重。");
                ui.add_sized(
                    [
                        ui.available_width(),
                        (ui.available_height() - 70.0).max(180.0),
                    ],
                    egui::TextEdit::multiline(&mut self.batch_import_content)
                        .font(editor_font(15.0))
                        .desired_width(f32::INFINITY)
                        .desired_rows(15)
                        .background_color(ui_theme::surface()),
                );
                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    if action_button(
                        ui,
                        "导入",
                        egui::Color32::from_rgb(31, 31, 30),
                        egui::Color32::WHITE,
                    )
                    .clicked()
                    {
                        import = true;
                    }
                    if action_button(
                        ui,
                        "取消",
                        ui_theme::control_surface(),
                        ui_theme::primary_text(),
                    )
                    .clicked()
                    {
                        close = true;
                    }
                });
                if let Some((message, true)) = &self.message
                    && message.starts_with("批量导入失败")
                {
                    ui.colored_label(ui_theme::red_text(), message);
                }
            });
        if import {
            match parse_words(&self.batch_import_content) {
                Ok(words) => {
                    let count = words.len();
                    self.rows.extend(map_to_rows(&words));
                    self.message = Some((format!("已批量导入 {count} 个词条。"), false));
                    close = true;
                }
                Err(error) => {
                    self.message = Some((format!("批量导入失败，JSON 格式错误：{error}"), true));
                }
            }
        }
        close
    }

    fn draw_batch_export(&mut self, ui: &mut egui::Ui) -> bool {
        ui.painter()
            .rect_filled(ui.max_rect(), 0.0, ui_theme::canvas());
        let mut export = false;
        let mut close = false;
        if back_button(ui, "返回词库编辑").clicked() {
            return true;
        }
        ui.add_space(10.0);
        egui::Frame::new()
            .fill(ui_theme::surface())
            .stroke(egui::Stroke::new(1.0, ui_theme::border()))
            .corner_radius(8)
            .inner_margin(egui::Margin::same(20))
            .show(ui, |ui| {
                ui.label(
                    egui::RichText::new("BATCH EXPORT")
                        .monospace()
                        .size(11.0)
                        .color(ui_theme::muted_text()),
                );
                ui.label("请输入导出文件名，文件将保存到 word_libraries 词库目录：");
                ui.add_space(8.0);
                let response = ui.add_sized(
                    [ui.available_width(), 36.0],
                    egui::TextEdit::singleline(&mut self.batch_export_file_name)
                        .font(editor_font(15.0))
                        .hint_text("例如：my_words.json"),
                );
                export |=
                    response.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter));
                ui.add_space(14.0);
                ui.horizontal(|ui| {
                    export |= action_button(
                        ui,
                        "导出",
                        egui::Color32::from_rgb(31, 31, 30),
                        egui::Color32::WHITE,
                    )
                    .clicked();
                    if action_button(
                        ui,
                        "取消",
                        ui_theme::control_surface(),
                        ui_theme::primary_text(),
                    )
                    .clicked()
                    {
                        close = true;
                    }
                });
                if let Some((message, true)) = &self.message
                    && (message.starts_with("导出失败")
                        || message.starts_with("批量导出失败")
                        || message.starts_with("无法导出"))
                {
                    ui.colored_label(ui_theme::red_text(), message);
                }
            });

        if export && self.export_words() {
            close = true;
        }
        close
    }

    fn export_words(&mut self) -> bool {
        let file_name = match normalized_export_file_name(&self.batch_export_file_name) {
            Ok(file_name) => file_name,
            Err(error) => {
                self.message = Some((error, true));
                return false;
            }
        };
        let (words, _) = clean_rows(&self.rows);
        let content = match format_words(&words) {
            Ok(content) => content,
            Err(error) => {
                self.message = Some((format!("批量导出失败：{error}"), true));
                return false;
            }
        };
        let directory = word_library_directory();
        if let Err(error) = fs::create_dir_all(&directory) {
            self.message = Some((format!("无法创建词库目录：{error}"), true));
            return false;
        }
        let path = directory.join(&file_name);
        match fs::write(&path, content) {
            Ok(()) => {
                self.message = Some((
                    format!("已导出 {} 个词条到 {}。", words.len(), path.display()),
                    false,
                ));
                true
            }
            Err(error) => {
                self.message = Some((format!("无法导出到 {}：{error}", path.display()), true));
                false
            }
        }
    }

    fn find_next(&mut self) {
        let query = self.search_query.as_str();
        if query.is_empty() {
            self.message = Some(("请输入要搜索的中文或韩文。".to_string(), true));
            return;
        }
        let from = self.search_from.min(self.content.len());
        let found = self.content[from..]
            .find(query)
            .map(|offset| from + offset)
            .or_else(|| self.content[..from].find(query));
        let Some(start) = found else {
            self.search_from = 0;
            self.message = Some((format!("没有找到“{query}”。"), true));
            return;
        };
        let end = start + query.len();
        self.search_from = end;
        self.pending_selection = Some((
            self.content[..start].chars().count(),
            self.content[..end].chars().count(),
        ));
        self.message = Some((format!("已找到“{query}”，继续查找将定位下一处。"), false));
    }

    fn check_format(&mut self) {
        match parse_words(&self.content) {
            Ok(words) => {
                self.error_line = None;
                self.message = Some((format!("JSON 格式正确，共 {} 个词条。", words.len()), false));
            }
            Err(error) => self.show_json_error("JSON 格式错误", &error),
        }
    }

    fn save(&mut self) -> bool {
        let (words, removed) = match self.mode {
            EditorMode::Table => clean_rows(&self.rows),
            EditorMode::AdvancedJson => match parse_words(&self.content) {
                Ok(words) => clean_map(words),
                Err(error) => {
                    self.show_json_error("保存失败，JSON 格式错误", &error);
                    return false;
                }
            },
        };
        let formatted = match format_words(&words) {
            Ok(formatted) => formatted,
            Err(error) => {
                self.message = Some((format!("格式化失败：{error}"), true));
                return false;
            }
        };
        let path = match prepare_word_library_file(&self.file_name) {
            Ok(path) => path,
            Err(error) => {
                self.message = Some((format!("无法准备词库目录：{error:#}"), true));
                return false;
            }
        };
        if let Err(error) = fs::write(&path, &formatted) {
            self.message = Some((format!("保存 {} 失败：{error}", self.file_name), true));
            return false;
        }

        self.content = formatted;
        self.rows = map_to_rows(&words);
        self.error_line = None;
        self.reload_requested = true;
        self.message = Some((
            format!(
                "已保存 {} 个词条；剔除重复 key {} 项、空 key {} 项、空 value {} 项。",
                words.len(),
                removed.duplicates,
                removed.empty_keys,
                removed.empty_values
            ),
            false,
        ));
        true
    }

    fn show_json_error(&mut self, prefix: &str, error: &serde_json::Error) {
        let line = error.line().max(1);
        let column = error.column().max(1);
        self.error_line = Some(line);
        self.pending_scroll_char = Some(line_start_char_index(&self.content, line));
        self.message = Some((
            format!("{prefix}（第 {line} 行，第 {column} 列）：{error}"),
            true,
        ));
    }
}

#[derive(Default)]
struct RemovedCounts {
    duplicates: usize,
    empty_keys: usize,
    empty_values: usize,
}

fn clean_rows(rows: &[WordRow]) -> (BTreeMap<String, String>, RemovedCounts) {
    let mut words = BTreeMap::new();
    let mut removed = RemovedCounts::default();
    for row in rows {
        let key = row.key.trim();
        let value = row.value.trim();
        if key.is_empty() {
            removed.empty_keys += 1;
        } else if value.is_empty() {
            removed.empty_values += 1;
        } else if words.insert(key.to_owned(), value.to_owned()).is_some() {
            removed.duplicates += 1;
        }
    }
    (words, removed)
}

fn clean_map(words: BTreeMap<String, String>) -> (BTreeMap<String, String>, RemovedCounts) {
    let rows = map_to_rows(&words);
    clean_rows(&rows)
}

fn map_to_rows(words: &BTreeMap<String, String>) -> Vec<WordRow> {
    words
        .iter()
        .map(|(key, value)| WordRow {
            key: key.clone(),
            value: value.clone(),
        })
        .collect()
}

fn table_page_bounds(total: usize, requested_page: usize) -> (usize, usize, usize, usize) {
    let page_count = total.div_ceil(TABLE_PAGE_SIZE).max(1);
    let page = requested_page.min(page_count - 1);
    let start = page * TABLE_PAGE_SIZE;
    let end = (start + TABLE_PAGE_SIZE).min(total);
    (page, page_count, start, end)
}

fn normalized_export_file_name(input: &str) -> Result<String, String> {
    let input = input.trim();
    if input.is_empty() {
        return Err("导出失败：文件名不能为空。".to_string());
    }
    let path = Path::new(input);
    if path.components().count() != 1
        || path.file_name().and_then(|name| name.to_str()) != Some(input)
    {
        return Err("导出失败：请只输入文件名，不要包含目录路径。".to_string());
    }
    if input
        .chars()
        .any(|character| "<>:\"/\\|?*".contains(character))
    {
        return Err("导出失败：文件名包含 Windows 不允许的字符。".to_string());
    }
    if input.to_ascii_lowercase().ends_with(".json") {
        Ok(input.to_string())
    } else {
        Ok(format!("{input}.json"))
    }
}

fn format_words(words: &BTreeMap<String, String>) -> Result<String, serde_json::Error> {
    serde_json::to_string_pretty(words).map(|formatted| format!("{formatted}\n"))
}

fn line_byte_range(content: &str, line_number: usize) -> Option<std::ops::Range<usize>> {
    let mut start = 0;
    for (index, line) in content.split_inclusive('\n').enumerate() {
        let end = start + line.len();
        if index + 1 == line_number {
            return Some(start..end);
        }
        start = end;
    }
    (line_number == 1 && content.is_empty()).then_some(0..0)
}

fn line_start_char_index(content: &str, line_number: usize) -> usize {
    line_byte_range(content, line_number)
        .map(|range| content[..range.start].chars().count())
        .unwrap_or_else(|| content.chars().count())
}

fn parse_words(content: &str) -> Result<BTreeMap<String, String>, serde_json::Error> {
    serde_json::from_str(content)
}

fn editor_font(size: f32) -> egui::FontId {
    egui::FontId::new(size, egui::FontFamily::Name(WORD_EDITOR_FONT_FAMILY.into()))
}

fn editor_status(ui: &mut egui::Ui, message: &str, is_error: bool) {
    let (fill, color) = if is_error {
        (ui_theme::pale_red(), ui_theme::red_text())
    } else {
        (ui_theme::pale_green(), ui_theme::green_text())
    };
    egui::Frame::new()
        .fill(fill)
        .stroke(egui::Stroke::new(1.0, color.gamma_multiply(0.18)))
        .corner_radius(6)
        .inner_margin(egui::Margin::symmetric(12, 8))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(egui::RichText::new(message).size(13.0).color(color));
        });
}

fn action_button(
    ui: &mut egui::Ui,
    text: &str,
    fill: egui::Color32,
    text_color: egui::Color32,
) -> egui::Response {
    ui.add_sized(
        [112.0, 40.0],
        egui::Button::new(
            egui::RichText::new(text)
                .size(15.0)
                .strong()
                .color(text_color),
        )
        .fill(fill)
        .stroke(egui::Stroke::new(1.0, ui_theme::border()))
        .corner_radius(6),
    )
}

fn back_button(ui: &mut egui::Ui, text: &str) -> egui::Response {
    ui.add_sized(
        [128.0, 34.0],
        egui::Button::new(
            egui::RichText::new(text)
                .size(13.0)
                .strong()
                .color(ui_theme::muted_text()),
        )
        .fill(ui_theme::control_surface())
        .stroke(egui::Stroke::new(1.0, ui_theme::border()))
        .corner_radius(5),
    )
}

fn toolbar_button(ui: &mut egui::Ui, text: &str, selected: bool, width: f32) -> egui::Response {
    let (fill, color) = if selected {
        (egui::Color32::from_rgb(31, 31, 30), egui::Color32::WHITE)
    } else {
        (ui_theme::control_surface(), ui_theme::primary_text())
    };
    filled_toolbar_button(ui, text, width, fill, color)
}

fn filled_toolbar_button(
    ui: &mut egui::Ui,
    text: &str,
    width: f32,
    fill: egui::Color32,
    color: egui::Color32,
) -> egui::Response {
    ui.add_sized(
        [width, 36.0],
        egui::Button::new(egui::RichText::new(text).size(14.0).strong().color(color))
            .fill(fill)
            .stroke(egui::Stroke::new(1.0, ui_theme::border()))
            .corner_radius(6),
    )
}

fn page_button(ui: &mut egui::Ui, text: &str, enabled: bool) -> egui::Response {
    ui.add_enabled(
        enabled,
        egui::Button::new(
            egui::RichText::new(text)
                .size(12.5)
                .color(ui_theme::primary_text()),
        )
        .fill(ui_theme::control_surface())
        .stroke(egui::Stroke::new(1.0, ui_theme::border()))
        .corner_radius(5)
        .min_size(egui::vec2(72.0, 30.0)),
    )
}

fn row_action_button(
    ui: &mut egui::Ui,
    text: &str,
    fill: egui::Color32,
    color: egui::Color32,
) -> egui::Response {
    ui.add_sized(
        [52.0, 32.0],
        egui::Button::new(egui::RichText::new(text).size(13.0).strong().color(color))
            .fill(fill)
            .stroke(egui::Stroke::new(1.0, ui_theme::border()))
            .corner_radius(6),
    )
}

#[cfg(test)]
mod tests {
    use super::{
        EditorMode, WordEditorState, WordRow, clean_rows, normalized_export_file_name, parse_words,
        table_page_bounds,
    };

    #[test]
    fn accepts_string_to_string_word_map() {
        let words = parse_words(r#"{"안녕하세요":"你好"}"#).unwrap();
        assert_eq!(words.get("안녕하세요").map(String::as_str), Some("你好"));
    }

    #[test]
    fn rejects_non_string_values() {
        assert!(parse_words(r#"{"word":123}"#).is_err());
    }

    #[test]
    fn export_file_name_is_local_and_uses_json_extension() {
        assert_eq!(
            normalized_export_file_name("my_words").unwrap(),
            "my_words.json"
        );
        assert!(normalized_export_file_name("../my_words.json").is_err());
        assert!(normalized_export_file_name("bad:name.json").is_err());
    }

    #[test]
    fn table_pages_never_exceed_fifty_rows() {
        assert_eq!(table_page_bounds(121, 0), (0, 3, 0, 50));
        assert_eq!(table_page_bounds(121, 1), (1, 3, 50, 100));
        assert_eq!(table_page_bounds(121, 99), (2, 3, 100, 121));
        assert_eq!(table_page_bounds(0, 0), (0, 1, 0, 0));
    }

    #[test]
    fn cleans_duplicate_and_empty_rows() {
        let rows = vec![
            WordRow {
                key: "a".into(),
                value: "一".into(),
            },
            WordRow {
                key: "a".into(),
                value: "二".into(),
            },
            WordRow {
                key: "".into(),
                value: "空".into(),
            },
            WordRow {
                key: "b".into(),
                value: "".into(),
            },
        ];
        let (words, removed) = clean_rows(&rows);
        assert_eq!(words.get("a").map(String::as_str), Some("二"));
        assert_eq!(removed.duplicates, 1);
        assert_eq!(removed.empty_keys, 1);
        assert_eq!(removed.empty_values, 1);
    }

    #[test]
    fn save_error_marks_the_exact_json_line() {
        let mut editor = WordEditorState {
            mode: EditorMode::AdvancedJson,
            content: "{\n  \"正确\": \"内容\",\n  \"错误\":,\n}".to_string(),
            ..Default::default()
        };
        assert!(!editor.save());
        assert_eq!(editor.error_line, Some(3));
    }

    #[test]
    fn finds_chinese_and_korean_text_in_sequence() {
        let mut editor = WordEditorState {
            content: "你好 안녕 你好".to_string(),
            search_query: "你好".to_string(),
            ..Default::default()
        };
        editor.find_next();
        assert_eq!(editor.pending_selection, Some((0, 2)));
        editor.find_next();
        assert_eq!(editor.pending_selection, Some((6, 8)));
    }
}
