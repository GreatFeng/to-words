//! 设置页面组件。
//!
//! 主要管理设置草稿、快捷键录制、语言选择、外观与行为选项，并负责在设置首页、
//! 词库编辑器和词库合并界面之间导航。公开的 [`SettingsPanel`] 由应用层持有，
//! `shortcut_pressed` 用于判断 egui 收到的按键是否符合用户配置。

use crate::platform::credentials;
use crate::platform::ocr;
use crate::ui_theme::{
    self, accent_fill, accent_text, border, canvas, control_surface, green_text, muted_text,
    pale_green, pale_red, pale_yellow, primary_text, red_text, surface, yellow_text,
};
use crate::word_editor::WordEditor;
use crate::word_merge_panel::WordMergePanel;
use crate::{UiConfig, translation_language};
use eframe::egui;
use global_hotkey::hotkey::HotKey;

#[derive(Clone, Copy, PartialEq, Eq)]
enum ShortcutTarget {
    Popup,
    Settings,
    Ocr,
    ClearComposed,
}

#[derive(Clone)]
pub(crate) enum AiKeyUpdate {
    Unchanged,
    Set(String),
    Delete,
}

pub(crate) struct SettingsPanel {
    draft: UiConfig,
    message: Option<(String, bool)>,
    active_shortcut: Option<ShortcutTarget>,
    active_number: Option<egui::Id>,
    last_wheel_change: f64,
    word_editor: WordEditor,
    word_merge_panel: WordMergePanel,
    ai_key_input: String,
    ai_key_present: bool,
    remove_ai_key: bool,
    active_source_language: String,
    ocr_status: String,
}

impl SettingsPanel {
    pub(crate) fn new(config: UiConfig) -> Self {
        Self {
            draft: config,
            message: None,
            active_shortcut: None,
            active_number: None,
            last_wheel_change: 0.0,
            word_editor: WordEditor::default(),
            word_merge_panel: WordMergePanel::default(),
            ai_key_input: String::new(),
            ai_key_present: false,
            remove_ai_key: false,
            active_source_language: "cn".to_string(),
            ocr_status: String::new(),
        }
    }

    pub(crate) fn open(&mut self, config: &UiConfig, active_source_language: &str) {
        self.draft = config.clone();
        self.active_source_language = active_source_language.to_string();
        self.ocr_status = ocr::engine_status();
        self.message = None;
        self.active_shortcut = None;
        self.active_number = None;
        self.word_editor.close();
        self.word_merge_panel.close();
        self.ai_key_input.clear();
        self.remove_ai_key = false;
        match credentials::load_key() {
            Ok(key) => self.ai_key_present = key.is_some(),
            Err(error) => {
                self.ai_key_present = false;
                self.message = Some((format!("无法读取 DeepSeek 凭据：{error}"), true));
            }
        }
    }

    pub(crate) fn is_recording_shortcut(&self) -> bool {
        self.active_shortcut.is_some()
    }

    pub(crate) fn has_open_dialog(&self) -> bool {
        self.word_editor.is_open() || self.word_merge_panel.is_open()
    }

    pub(crate) fn take_word_reload_requested(&mut self) -> bool {
        let edited = self.word_editor.take_reload_requested();
        let selected_merge = self.word_merge_panel.take_reload_requested();
        edited || selected_merge
    }

    pub(crate) fn set_result(&mut self, result: Result<(), String>) {
        self.message = Some(match result {
            Ok(()) => ("配置已保存并立即生效。".to_string(), false),
            Err(error) => (error, true),
        });
    }

    pub(crate) fn show(&mut self, ui: &mut egui::Ui) -> Option<(UiConfig, AiKeyUpdate)> {
        if ui.input(|input| input.pointer.any_pressed()) {
            self.active_shortcut = None;
            self.active_number = None;
        }

        if self.word_editor.is_open() {
            self.active_shortcut = None;
            self.active_number = None;
            self.word_editor.show(ui);
            return None;
        }
        if self.word_merge_panel.is_open() {
            self.active_shortcut = None;
            self.active_number = None;
            self.word_merge_panel.show(ui);
            return None;
        }

        self.capture_shortcut(ui.ctx());

        ui_theme::apply(ui);
        ui.painter().rect_filled(ui.max_rect(), 0.0, canvas());
        ui.spacing_mut().item_spacing = egui::vec2(12.0, 12.0);

        let mut save = false;
        let allow_page_wheel = self.active_number.is_none();
        egui::Frame::new()
            .inner_margin(egui::Margin::symmetric(28, 24))
            .show(ui, |ui| {
                ui.label(
                    egui::RichText::new("TO WORDS / PREFERENCES")
                        .monospace()
                        .size(11.0)
                        .color(muted_text()),
                );
                ui.label(
                    egui::RichText::new("设置")
                        .size(29.0)
                        .strong()
                        .color(primary_text()),
                );
                ui.label(
                    egui::RichText::new("管理词库、快捷键与查询窗口的显示方式。")
                        .size(14.0)
                        .color(muted_text()),
                );
                ui.add_space(18.0);

                egui::ScrollArea::vertical()
                    .id_salt("settings_scroll")
                    .auto_shrink([false, false])
                    .scroll_source(egui::scroll_area::ScrollSource {
                        mouse_wheel: allow_page_wheel,
                        ..Default::default()
                    })
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        settings_card(
                            ui,
                            "词库",
                            "选择原始语言与目标语言，并管理对应的用户词库。",
                            |ui| {
                                self.draw_translation_selector(ui);
                                ui.add_space(12.0);
                                ui.horizontal_wrapped(|ui| {
                                    if secondary_button(ui, "编辑词库", 104.0).clicked() {
                                        self.open_word_editor(ui.ctx());
                                    }
                                    if secondary_button(ui, "合并本地词库", 128.0)
                                        .on_hover_text("选择一个或多个词库并合并到当前语言词库")
                                        .clicked()
                                    {
                                        let file_name = self.current_word_file_name();
                                        self.word_merge_panel.open(file_name);
                                    }
                                });
                            },
                        );
                        ui.add_space(14.0);

                        settings_card(
                            ui,
                            "AI 翻译 · DeepSeek",
                            "可随时主动翻译当前输入文字；译文确认后加入组合。",
                            |ui| self.draw_ai_settings(ui),
                        );
                        ui.add_space(14.0);

                        settings_card(
                            ui,
                            "快捷键",
                            "点击按键框后，直接按下新的组合键。",
                            |ui| {
                                self.draw_shortcuts(ui);
                                ui.add_space(10.0);
                                ui.label(
                                    egui::RichText::new(&self.ocr_status)
                                        .size(12.0)
                                        .color(muted_text()),
                                );
                            },
                        );
                        ui.add_space(14.0);

                        if ui.available_width() >= 680.0 {
                            ui.columns(2, |columns| {
                                settings_card(
                                    &mut columns[0],
                                    "输入框",
                                    "调整查询输入区域的尺寸与外观。",
                                    |ui| {
                                        ui.set_min_height(390.0);
                                        self.draw_input_settings(ui);
                                    },
                                );
                                settings_card(
                                    &mut columns[1],
                                    "查询结果",
                                    "设置结果列表的范围、密度和分页。",
                                    |ui| {
                                        ui.set_min_height(390.0);
                                        self.draw_result_settings(ui);
                                    },
                                );
                            });
                        } else {
                            settings_card(
                                ui,
                                "输入框",
                                "调整查询输入区域的尺寸与外观。",
                                |ui| self.draw_input_settings(ui),
                            );
                            ui.add_space(14.0);
                            settings_card(
                                ui,
                                "查询结果",
                                "设置结果列表的范围、密度和分页。",
                                |ui| self.draw_result_settings(ui),
                            );
                        }
                        ui.add_space(14.0);
                        settings_card(
                            ui,
                            "通用",
                            "统一控制文字、选中状态和连续输入行为。",
                            |ui| self.draw_general_settings(ui),
                        );
                        ui.add_space(18.0);

                        if let Some((message, is_error)) = &self.message {
                            status_message(ui, message, *is_error);
                            ui.add_space(12.0);
                        }

                        ui.horizontal(|ui| {
                            if primary_button(ui, "保存并应用", 124.0).clicked() {
                                save = true;
                            }
                            if quiet_button(ui, "恢复默认值", 112.0).clicked() {
                                self.draft = UiConfig::default();
                                self.message = None;
                                self.active_shortcut = None;
                                self.active_number = None;
                                self.ai_key_input.clear();
                                self.remove_ai_key = false;
                            }
                        });
                        ui.add_space(8.0);
                    });
            });

        save.then(|| {
            let key_update = if !self.ai_key_input.trim().is_empty() {
                AiKeyUpdate::Set(self.ai_key_input.trim().to_owned())
            } else if self.remove_ai_key {
                AiKeyUpdate::Delete
            } else {
                AiKeyUpdate::Unchanged
            };
            (self.draft.clone(), key_update)
        })
    }

    pub(crate) fn mark_ai_key_saved(&mut self, change: &AiKeyUpdate) {
        match change {
            AiKeyUpdate::Unchanged => {}
            AiKeyUpdate::Set(_) => self.ai_key_present = true,
            AiKeyUpdate::Delete => self.ai_key_present = false,
        }
        self.ai_key_input.clear();
        self.remove_ai_key = false;
    }

    fn draw_ai_settings(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            contrast_checkbox(ui, &mut self.draft.ai_translation, "启用主动 AI 翻译");
            ui.label(
                egui::RichText::new("模型：deepseek-flash")
                    .size(12.0)
                    .color(muted_text()),
            );
        });
        ui.add_space(8.0);
        ui.add_enabled_ui(self.draft.ai_translation, |ui| {
            contrast_checkbox(
                ui,
                &mut self.draft.ai_auto_save,
                "确认 AI 译文后自动保存到当前语言词库",
            );
        });
        ui.add_space(8.0);
        ui.horizontal_wrapped(|ui| {
            ui.label(
                egui::RichText::new("API Key")
                    .size(13.0)
                    .color(muted_text()),
            );
            ui.add_sized(
                [300.0, 32.0],
                egui::TextEdit::singleline(&mut self.ai_key_input)
                    .password(true)
                    .hint_text(if self.ai_key_present && !self.remove_ai_key {
                        "已安全保存；留空则保持不变"
                    } else {
                        "输入你的 DeepSeek API Key"
                    }),
            );
            if self.ai_key_present && secondary_button(ui, "删除密钥", 94.0).clicked() {
                self.ai_key_input.clear();
                self.remove_ai_key = true;
            }
        });
        if self.remove_ai_key {
            ui.label(
                egui::RichText::new("保存并应用后将删除已保存的密钥。")
                    .size(12.0)
                    .color(red_text()),
            );
        } else {
            ui.label(egui::RichText::new("密钥保存在 Windows 凭据管理器，不写入 ui_config.json。未选择目标语言时无法请求翻译。")
                .size(12.0).color(muted_text()));
        }
    }

    fn draw_translation_selector(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            ui.vertical(|ui| {
                ui.label(
                    egui::RichText::new("原始语言")
                        .size(13.0)
                        .strong()
                        .color(primary_text()),
                );
                egui::ComboBox::from_id_salt("source_language")
                    .selected_text(if self.draft.source_language == "auto" {
                        "自动检测"
                    } else {
                        translation_language::language_label(&self.draft.source_language)
                    })
                    .width(220.0)
                    .height(320.0)
                    .show_ui(ui, |ui| {
                        ui.selectable_value(
                            &mut self.draft.source_language,
                            "auto".to_string(),
                            "自动检测",
                        );
                        ui.separator();
                        for language in translation_language::LANGUAGES {
                            ui.selectable_value(
                                &mut self.draft.source_language,
                                language.code.to_string(),
                                language.label,
                            );
                        }
                    });
            });
            ui.add_space(12.0);
            ui.vertical(|ui| {
                ui.label(
                    egui::RichText::new("目标语言")
                        .size(13.0)
                        .strong()
                        .color(primary_text()),
                );
                egui::ComboBox::from_id_salt("translation_language")
                    .selected_text(translation_language::language_label(
                        &self.draft.translation_language,
                    ))
                    .width(220.0)
                    .height(320.0)
                    .show_ui(ui, |ui| {
                        ui.selectable_value(
                            &mut self.draft.translation_language,
                            String::new(),
                            "未选择（默认词库）",
                        );
                        ui.separator();
                        for language in translation_language::LANGUAGES {
                            ui.selectable_value(
                                &mut self.draft.translation_language,
                                language.code.to_string(),
                                language.label,
                            );
                        }
                    });
            });
        });

        let file_name = self.current_word_file_name();
        ui.label(
            egui::RichText::new(format!("词库文件：{file_name}"))
                .monospace()
                .size(12.0)
                .color(muted_text()),
        );
        if self.draft.source_language == "auto" {
            ui.label(
                egui::RichText::new(format!(
                    "自动检测当前使用：{}；语言难以判断时沿用该词库。缺少的词库会自动创建。",
                    translation_language::language_label(&self.active_source_language)
                ))
                .size(12.0)
                .color(muted_text()),
            );
        } else {
            ui.label(
                egui::RichText::new("缺少的词库会自动创建，不会覆盖已有词条。")
                    .size(12.0)
                    .color(muted_text()),
            );
        }
    }

    fn current_word_file_name(&self) -> String {
        let source = if self.draft.source_language == "auto" {
            &self.active_source_language
        } else {
            &self.draft.source_language
        };
        translation_language::word_file_name(source, &self.draft.translation_language)
    }

    fn open_word_editor(&mut self, context: &egui::Context) {
        let file_name = self.current_word_file_name();
        self.word_editor.open(&file_name);

        // 只在空间不足时扩大窗口，保留完整表格视区和底部操作栏，也不缩小用户已
        // 经手动调整过的窗口。
        let current_size = context
            .input(|input| input.viewport().inner_rect.map(|rect| rect.size()))
            .unwrap_or(egui::vec2(800.0, 600.0));
        context.send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(
            current_size.x.max(800.0),
            current_size.y.max(700.0),
        )));
    }

    fn draw_shortcuts(&mut self, ui: &mut egui::Ui) {
        egui::Grid::new("hotkey_settings")
            .num_columns(2)
            .spacing([20.0, 12.0])
            .show(ui, |ui| {
                shortcut_row(
                    ui,
                    "显示查询框",
                    &mut self.draft.hotkeys.popup,
                    &mut self.active_shortcut,
                    ShortcutTarget::Popup,
                );
                shortcut_row(
                    ui,
                    "显示设置面板",
                    &mut self.draft.hotkeys.settings,
                    &mut self.active_shortcut,
                    ShortcutTarget::Settings,
                );
                shortcut_row(
                    ui,
                    "框选 OCR 识别",
                    &mut self.draft.hotkeys.ocr,
                    &mut self.active_shortcut,
                    ShortcutTarget::Ocr,
                );
                shortcut_row(
                    ui,
                    "清空组合内容",
                    &mut self.draft.hotkeys.clear_composed,
                    &mut self.active_shortcut,
                    ShortcutTarget::ClearComposed,
                );
            });
    }

    fn draw_input_settings(&mut self, ui: &mut egui::Ui) {
        egui::Grid::new("input_settings")
            .num_columns(2)
            .spacing([16.0, 11.0])
            .show(ui, |ui| {
                text_row(ui, "背景颜色", &mut self.draft.input.background_color);
                number_row(
                    ui,
                    "input_opacity",
                    "透明度",
                    &mut self.draft.input.opacity,
                    0.0..=1.0,
                    0.002,
                    &mut self.active_number,
                    &mut self.last_wheel_change,
                );
                number_row(
                    ui,
                    "input_width",
                    "宽度",
                    &mut self.draft.input.width,
                    80.0..=1200.0,
                    0.2,
                    &mut self.active_number,
                    &mut self.last_wheel_change,
                );
                number_row(
                    ui,
                    "input_height",
                    "高度",
                    &mut self.draft.input.height,
                    18.0..=200.0,
                    0.2,
                    &mut self.active_number,
                    &mut self.last_wheel_change,
                );
                number_row(
                    ui,
                    "input_font_size",
                    "字体大小",
                    &mut self.draft.input.font_size,
                    8.0..=72.0,
                    0.2,
                    &mut self.active_number,
                    &mut self.last_wheel_change,
                );
                number_row(
                    ui,
                    "input_corner_radius",
                    "圆角",
                    &mut self.draft.input.corner_radius,
                    0.0..=100.0,
                    0.2,
                    &mut self.active_number,
                    &mut self.last_wheel_change,
                );
            });
    }

    fn draw_result_settings(&mut self, ui: &mut egui::Ui) {
        egui::Grid::new("result_settings")
            .num_columns(2)
            .spacing([16.0, 11.0])
            .show(ui, |ui| {
                text_row(ui, "背景颜色", &mut self.draft.results.background_color);
                number_row(
                    ui,
                    "results_opacity",
                    "透明度",
                    &mut self.draft.results.opacity,
                    0.0..=1.0,
                    0.002,
                    &mut self.active_number,
                    &mut self.last_wheel_change,
                );
                number_row(
                    ui,
                    "results_min_width",
                    "最小宽度",
                    &mut self.draft.results.min_width,
                    80.0..=1200.0,
                    0.2,
                    &mut self.active_number,
                    &mut self.last_wheel_change,
                );
                number_row(
                    ui,
                    "results_max_width",
                    "最大宽度",
                    &mut self.draft.results.max_width,
                    80.0..=1600.0,
                    0.2,
                    &mut self.active_number,
                    &mut self.last_wheel_change,
                );
                number_row(
                    ui,
                    "results_min_row_height",
                    "最小行高",
                    &mut self.draft.results.min_row_height,
                    18.0..=200.0,
                    0.2,
                    &mut self.active_number,
                    &mut self.last_wheel_change,
                );
                number_row(
                    ui,
                    "results_max_height",
                    "最大高度",
                    &mut self.draft.results.max_height,
                    40.0..=1200.0,
                    0.2,
                    &mut self.active_number,
                    &mut self.last_wheel_change,
                );
                number_row(
                    ui,
                    "results_font_size",
                    "字体大小",
                    &mut self.draft.results.font_size,
                    8.0..=72.0,
                    0.2,
                    &mut self.active_number,
                    &mut self.last_wheel_change,
                );
                number_row(
                    ui,
                    "results_corner_radius",
                    "圆角",
                    &mut self.draft.results.corner_radius,
                    0.0..=100.0,
                    0.2,
                    &mut self.active_number,
                    &mut self.last_wheel_change,
                );
                usize_number_row(
                    ui,
                    "results_max_visible",
                    "每页最多结果",
                    &mut self.draft.results.max_visible_results,
                    1..=100,
                    &mut self.active_number,
                    &mut self.last_wheel_change,
                );
            });
    }

    fn draw_general_settings(&mut self, ui: &mut egui::Ui) {
        egui::Grid::new("general_settings")
            .num_columns(2)
            .spacing([20.0, 12.0])
            .show(ui, |ui| {
                number_row(
                    ui,
                    "general_gap",
                    "区域间距",
                    &mut self.draft.gap,
                    0.0..=40.0,
                    0.2,
                    &mut self.active_number,
                    &mut self.last_wheel_change,
                );
                text_row(ui, "文字颜色", &mut self.draft.text_color);
                text_row(ui, "选中背景色", &mut self.draft.selected_color);
                text_row(ui, "选中文字颜色", &mut self.draft.selected_text_color);
                number_row(
                    ui,
                    "selected_opacity",
                    "选中透明度",
                    &mut self.draft.selected_opacity,
                    0.0..=1.0,
                    0.002,
                    &mut self.active_number,
                    &mut self.last_wheel_change,
                );
                ui.label(
                    egui::RichText::new("背景模糊")
                        .size(13.0)
                        .color(muted_text()),
                );
                contrast_checkbox(ui, &mut self.draft.background_blur, "启用");
                ui.end_row();
                ui.label(
                    egui::RichText::new("保持打开")
                        .size(13.0)
                        .color(muted_text()),
                );
                contrast_checkbox(
                    ui,
                    &mut self.draft.continuous_input,
                    "Enter 复制后仍保持查询框打开",
                );
                ui.end_row();
                ui.label(
                    egui::RichText::new("返回后自动清空")
                        .size(13.0)
                        .color(muted_text()),
                );
                ui.add_enabled_ui(self.draft.continuous_input, |ui| {
                    contrast_checkbox(
                        ui,
                        &mut self.draft.clear_on_focus_return,
                        "复制后切出查询框，再次返回时清空全部输入",
                    );
                });
                ui.end_row();
            });
    }

    fn capture_shortcut(&mut self, context: &egui::Context) {
        let Some(target) = self.active_shortcut else {
            return;
        };
        let pressed = context.input(|input| {
            input.events.iter().find_map(|event| match event {
                egui::Event::Key {
                    key,
                    pressed: true,
                    repeat: false,
                    modifiers,
                    ..
                } if !is_modifier_key(*key) => Some((*key, *modifiers)),
                _ => None,
            })
        });
        let Some((key, modifiers)) = pressed else {
            return;
        };

        match shortcut_text(key, physical_modifiers(modifiers)) {
            Ok(shortcut) => {
                match target {
                    ShortcutTarget::Popup => self.draft.hotkeys.popup = shortcut,
                    ShortcutTarget::Settings => self.draft.hotkeys.settings = shortcut,
                    ShortcutTarget::Ocr => self.draft.hotkeys.ocr = shortcut,
                    ShortcutTarget::ClearComposed => self.draft.hotkeys.clear_composed = shortcut,
                }
                self.active_shortcut = None;
                self.message = None;
            }
            Err(error) => self.message = Some((error, true)),
        }
    }
}

fn contrast_checkbox(ui: &mut egui::Ui, value: &mut bool, text: &str) -> egui::Response {
    ui.scope(|ui| {
        if *value {
            let visuals = &mut ui.style_mut().visuals;
            visuals.widgets.inactive.bg_fill = accent_fill();
            visuals.widgets.inactive.bg_stroke = egui::Stroke::new(1.5, accent_text());
            visuals.widgets.inactive.fg_stroke =
                egui::Stroke::new(2.0, egui::Color32::from_rgb(24, 68, 94));
        }
        ui.checkbox(
            value,
            egui::RichText::new(text).size(13.0).color(primary_text()),
        )
    })
    .inner
}

fn settings_card<R>(
    ui: &mut egui::Ui,
    title: &str,
    description: &str,
    add_contents: impl FnOnce(&mut egui::Ui) -> R,
) -> R {
    egui::Frame::new()
        .fill(surface())
        .stroke(egui::Stroke::new(1.0, border()))
        .corner_radius(8)
        .inner_margin(egui::Margin::same(18))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(
                egui::RichText::new(title)
                    .size(17.0)
                    .strong()
                    .color(primary_text()),
            );
            ui.label(
                egui::RichText::new(description)
                    .size(12.5)
                    .color(muted_text()),
            );
            ui.add_space(12.0);
            add_contents(ui)
        })
        .inner
}

fn primary_button(ui: &mut egui::Ui, text: &str, width: f32) -> egui::Response {
    ui.add_sized(
        [width, 38.0],
        egui::Button::new(
            egui::RichText::new(text)
                .size(14.0)
                .strong()
                .color(egui::Color32::WHITE),
        )
        .fill(egui::Color32::from_rgb(31, 31, 30))
        .stroke(egui::Stroke::NONE)
        .corner_radius(6),
    )
}

fn secondary_button(ui: &mut egui::Ui, text: &str, width: f32) -> egui::Response {
    ui.add_sized(
        [width, 36.0],
        egui::Button::new(egui::RichText::new(text).size(13.5).color(primary_text()))
            .fill(control_surface())
            .stroke(egui::Stroke::new(1.0, border()))
            .corner_radius(6),
    )
}

fn quiet_button(ui: &mut egui::Ui, text: &str, width: f32) -> egui::Response {
    ui.add_sized(
        [width, 38.0],
        egui::Button::new(egui::RichText::new(text).size(13.5).color(muted_text()))
            .fill(surface())
            .stroke(egui::Stroke::new(1.0, border()))
            .corner_radius(6),
    )
}

fn status_message(ui: &mut egui::Ui, message: &str, is_error: bool) {
    let (fill, color) = if is_error {
        (pale_red(), red_text())
    } else {
        (pale_green(), green_text())
    };
    egui::Frame::new()
        .fill(fill)
        .stroke(egui::Stroke::new(1.0, color.gamma_multiply(0.16)))
        .corner_radius(6)
        .inner_margin(egui::Margin::symmetric(12, 9))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(egui::RichText::new(message).size(13.0).color(color));
        });
}

fn text_row(ui: &mut egui::Ui, label: &str, value: &mut String) {
    ui.label(egui::RichText::new(label).size(13.0).color(muted_text()));
    let width = ui.available_width().clamp(132.0, 180.0);
    ui.add(
        egui::TextEdit::singleline(value)
            .desired_width(width)
            .margin(egui::Margin::symmetric(8, 5))
            .background_color(control_surface()),
    );
    ui.end_row();
}

fn shortcut_row(
    ui: &mut egui::Ui,
    label: &str,
    value: &mut String,
    active: &mut Option<ShortcutTarget>,
    target: ShortcutTarget,
) {
    ui.label(egui::RichText::new(label).size(13.0).color(muted_text()));
    let recording = *active == Some(target);
    let text = if recording || value.is_empty() {
        "请按下快捷键…"
    } else {
        value.as_str()
    };
    let width = ui.available_width().clamp(132.0, 280.0);
    let fill = if recording {
        pale_yellow()
    } else {
        control_surface()
    };
    if ui
        .add_sized(
            [width, 32.0],
            egui::Button::new(egui::RichText::new(text).monospace().size(12.5).color(
                if recording {
                    yellow_text()
                } else {
                    primary_text()
                },
            ))
            .fill(fill)
            .stroke(egui::Stroke::new(1.0, border()))
            .corner_radius(4),
        )
        .on_hover_text("点击后，通过键盘按下新的组合键")
        .clicked()
    {
        value.clear();
        *active = Some(target);
    }
    ui.end_row();
}

#[allow(clippy::too_many_arguments)]
fn number_row(
    ui: &mut egui::Ui,
    id_source: &str,
    label: &str,
    value: &mut f32,
    range: std::ops::RangeInclusive<f32>,
    step: f32,
    active: &mut Option<egui::Id>,
    last_wheel_change: &mut f64,
) {
    ui.label(egui::RichText::new(label).size(13.0).color(muted_text()));
    let id = ui.make_persistent_id(id_source);
    let selected = *active == Some(id);
    let text = if step < 1.0 {
        format!("{value:.2}")
    } else {
        format!("{value:.0}")
    };
    let width = ui.available_width().clamp(132.0, 180.0);
    let response = ui
        .add_sized(
            [width, 32.0],
            egui::Button::new(egui::RichText::new(text).monospace().size(12.5).color(
                if selected {
                    accent_text()
                } else {
                    primary_text()
                },
            ))
            .fill(if selected {
                accent_fill()
            } else {
                control_surface()
            })
            .stroke(egui::Stroke::new(1.0, border()))
            .corner_radius(4),
        )
        .on_hover_text("点击选中，然后滚动鼠标滚轮调整");
    if response.clicked() {
        *active = Some(id);
        response.request_focus();
    }
    if *active == Some(id) {
        let (wheel, now) = ui.input(|input| (input.smooth_scroll_delta().y, input.time));

        // 每隔 0.15 秒最多调整一次
        if wheel != 0.0 && now - *last_wheel_change >= 0.03 {
            let direction = if wheel > 0.0 { 1.0 } else { -1.0 };

            *value = (*value + direction * step).clamp(*range.start(), *range.end());

            *last_wheel_change = now;
        }
    }
    ui.end_row();
}

fn usize_number_row(
    ui: &mut egui::Ui,
    id_source: &str,
    label: &str,
    value: &mut usize,
    range: std::ops::RangeInclusive<usize>,
    active: &mut Option<egui::Id>,
    last_wheel_change: &mut f64,
) {
    ui.label(egui::RichText::new(label).size(13.0).color(muted_text()));
    let id = ui.make_persistent_id(id_source);
    let selected = *active == Some(id);
    let width = ui.available_width().clamp(132.0, 180.0);
    let response = ui
        .add_sized(
            [width, 32.0],
            egui::Button::new(
                egui::RichText::new(value.to_string())
                    .monospace()
                    .size(12.5)
                    .color(if selected {
                        accent_text()
                    } else {
                        primary_text()
                    }),
            )
            .fill(if selected {
                accent_fill()
            } else {
                control_surface()
            })
            .stroke(egui::Stroke::new(1.0, border()))
            .corner_radius(4),
        )
        .on_hover_text("点击选中，然后滚动鼠标滚轮调整");
    if response.clicked() {
        *active = Some(id);
        response.request_focus();
    }
    if *active == Some(id) {
        let (wheel, now) = ui.input(|input| (input.smooth_scroll_delta().y, input.time));
        if wheel != 0.0 && now - *last_wheel_change >= 0.03 {
            if wheel > 0.0 {
                *value = value.saturating_add(1).min(*range.end());
            } else {
                *value = value.saturating_sub(1).max(*range.start());
            }
            *last_wheel_change = now;
        }
    }
    ui.end_row();
}

pub(crate) fn shortcut_pressed(context: &egui::Context, configured: &str) -> bool {
    context.input(|input| {
        input.events.iter().any(|event| match event {
            egui::Event::Key {
                key,
                pressed: true,
                repeat: false,
                modifiers,
                ..
            } if !is_modifier_key(*key) => shortcut_text(*key, physical_modifiers(*modifiers))
                .is_ok_and(|pressed| pressed.eq_ignore_ascii_case(configured)),
            _ => false,
        })
    })
}

fn is_modifier_key(key: egui::Key) -> bool {
    matches!(
        key,
        egui::Key::ShiftLeft
            | egui::Key::ShiftRight
            | egui::Key::ControlLeft
            | egui::Key::ControlRight
            | egui::Key::AltLeft
            | egui::Key::AltRight
            | egui::Key::SuperLeft
            | egui::Key::SuperRight
    )
}

#[cfg(target_os = "windows")]
fn physical_modifiers(mut modifiers: egui::Modifiers) -> egui::Modifiers {
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
        GetAsyncKeyState, VK_CONTROL, VK_MENU, VK_SHIFT,
    };

    let is_pressed = |key| unsafe { (GetAsyncKeyState(key) as u16 & 0x8000) != 0 };
    modifiers.ctrl |= is_pressed(VK_CONTROL as i32);
    modifiers.alt |= is_pressed(VK_MENU as i32);
    modifiers.shift |= is_pressed(VK_SHIFT as i32);
    modifiers
}

#[cfg(not(target_os = "windows"))]
fn physical_modifiers(modifiers: egui::Modifiers) -> egui::Modifiers {
    modifiers
}

fn shortcut_text(key: egui::Key, modifiers: egui::Modifiers) -> Result<String, String> {
    let mut parts = Vec::new();
    if modifiers.ctrl {
        parts.push("ctrl");
    }
    if modifiers.alt {
        parts.push("alt");
    }
    if modifiers.shift {
        parts.push("shift");
    }
    if modifiers.mac_cmd {
        parts.push("super");
    }
    parts.push(key.name());
    let shortcut = parts.join("+");
    shortcut
        .parse::<HotKey>()
        .map(|_| shortcut)
        .map_err(|_| format!("不支持这个按键：{}", key.name()))
}

#[cfg(test)]
mod tests {
    use super::shortcut_text;
    use eframe::egui;

    #[test]
    fn records_ctrl_alt_enter() {
        let modifiers = egui::Modifiers {
            ctrl: true,
            alt: true,
            ..egui::Modifiers::NONE
        };
        assert_eq!(
            shortcut_text(egui::Key::Enter, modifiers).unwrap(),
            "ctrl+alt+Enter"
        );
    }
}
