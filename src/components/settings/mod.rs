//! 设置页面组件。
//!
//! 主要管理设置草稿、快捷键录制、语言选择、外观与行为选项，并负责设置首页与
//! 词库编辑器之间的导航。公开的 [`SettingsPanel`] 由应用层持有，
//! `shortcut_pressed` 用于判断 egui 收到的按键是否符合用户配置。

mod glass_footer;

use self::glass_footer::GlassFooter;
use crate::domain::shortcut::{MouseShortcut, Shortcut};
use crate::i18n::{self, tr};
use crate::platform::credentials;
use crate::platform::mouse_hotkey::MouseButtonEvent;
use crate::platform::ocr;
use crate::platform::voice;
use crate::platform::voice_identity::{self, EnrollmentEvent};
use crate::ui_theme::{
    self, accent_fill, accent_text, border, canvas, control_surface, green_text, muted_text,
    pale_yellow, primary_text, red_text, surface, yellow_text,
};
use crate::word_editor::WordEditor;
use crate::{UiConfig, translation_language};
use eframe::egui;
use std::time::{Duration, Instant};

const SAVED_MESSAGE_DURATION: Duration = Duration::from_millis(1200);

struct SettingsMessage {
    text: String,
    is_error: bool,
    expires_at: Option<Instant>,
}

impl SettingsMessage {
    fn saved() -> Self {
        Self {
            text: tr("配置已保存并立即生效。").to_string(),
            is_error: false,
            expires_at: Some(Instant::now() + SAVED_MESSAGE_DURATION),
        }
    }

    fn error(text: String) -> Self {
        Self {
            text,
            is_error: true,
            expires_at: None,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ShortcutTarget {
    Popup,
    Settings,
    Ocr,
    Voice,
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
    message: Option<SettingsMessage>,
    scroll_to_top: bool,
    active_shortcut: Option<ShortcutTarget>,
    recording_started_at: Option<Instant>,
    active_number: Option<egui::Id>,
    last_wheel_change: f64,
    word_editor: WordEditor,
    word_editor_review_requested: bool,
    ai_key_input: String,
    ai_key_present: bool,
    remove_ai_key: bool,
    active_source_language: String,
    ocr_status: String,
    voice_status: String,
    voice_enrollment: Option<std::sync::mpsc::Receiver<EnrollmentEvent>>,
    voice_enrollment_status: String,
    voice_registered: bool,
    voice_registration_playback_filter: Option<bool>,
    glass_footer: GlassFooter,
}

impl SettingsPanel {
    pub(crate) fn new(config: UiConfig) -> Self {
        Self {
            draft: config,
            message: None,
            scroll_to_top: false,
            active_shortcut: None,
            recording_started_at: None,
            active_number: None,
            last_wheel_change: 0.0,
            word_editor: WordEditor::default(),
            word_editor_review_requested: false,
            ai_key_input: String::new(),
            ai_key_present: false,
            remove_ai_key: false,
            active_source_language: "cn".to_string(),
            ocr_status: String::new(),
            voice_status: String::new(),
            voice_enrollment: None,
            voice_enrollment_status: String::new(),
            voice_registered: false,
            voice_registration_playback_filter: None,
            glass_footer: GlassFooter::default(),
        }
    }

    pub(crate) fn open(&mut self, config: &UiConfig, active_source_language: &str) {
        self.draft = config.clone();
        i18n::set_language(&self.draft.ui_language);
        self.active_source_language = active_source_language.to_string();
        self.ocr_status = ocr::engine_status();
        self.voice_status = voice::engine_status(&self.draft.voice_backend);
        self.voice_registered = voice_identity::is_registered();
        self.voice_registration_playback_filter =
            voice_identity::registration_uses_playback_filter();
        self.voice_enrollment_status = if self.voice_registered {
            tr("已注册本人声纹").to_string()
        } else {
            tr("尚未注册本人声纹").to_string()
        };
        self.message = None;
        self.scroll_to_top = false;
        self.active_shortcut = None;
        self.recording_started_at = None;
        self.active_number = None;
        self.word_editor.close();
        self.word_editor_review_requested = false;
        self.ai_key_input.clear();
        self.remove_ai_key = false;
        match credentials::load_key() {
            Ok(key) => self.ai_key_present = key.is_some(),
            Err(error) => {
                self.ai_key_present = false;
                self.message = Some(SettingsMessage::error(i18n::message(
                    "无法读取 DeepSeek 凭据：{error}",
                    &[("error", &error.to_string())],
                )));
            }
        }
    }

    pub(crate) fn is_recording_shortcut(&self) -> bool {
        self.active_shortcut.is_some()
    }

    pub(crate) fn capture_mouse_shortcut(&mut self, event: MouseButtonEvent) {
        let Some(target) = self.active_shortcut else {
            return;
        };
        if event.pressed
            && self
                .recording_started_at
                .is_some_and(|at| event.occurred_at >= at)
        {
            self.set_recorded_shortcut(
                target,
                Ok(MouseShortcut {
                    button: event.button,
                    modifiers: event.modifiers,
                }
                .to_string()),
            );
        }
    }

    pub(crate) fn has_open_dialog(&self) -> bool {
        self.word_editor.is_open()
    }

    pub(crate) fn take_word_reload_requested(&mut self) -> bool {
        self.word_editor.take_reload_requested()
    }

    pub(crate) fn take_word_editor_review_requested(&mut self) -> bool {
        std::mem::take(&mut self.word_editor_review_requested)
    }

    pub(crate) fn refresh_word_editor_after_external_save(
        &mut self,
        file_name: &str,
    ) -> anyhow::Result<()> {
        self.word_editor.refresh_after_external_save(file_name)
    }

    pub(crate) fn show_word_editor_review_error(&mut self, error: &str) {
        self.word_editor.show_review_error(error);
    }

    pub(crate) fn set_result(&mut self, result: Result<(), String>) {
        self.message = Some(match result {
            Ok(()) => SettingsMessage::saved(),
            Err(error) => SettingsMessage::error(error),
        });
    }

    fn expire_message(&mut self, context: &egui::Context, now: Instant) {
        if let Some(expires_at) = self.message.as_ref().and_then(|message| message.expires_at) {
            if now >= expires_at {
                self.message = None;
            } else {
                context.request_repaint_after(expires_at - now);
            }
        }
    }

    pub(crate) fn show(&mut self, ui: &mut egui::Ui) -> Option<(UiConfig, AiKeyUpdate)> {
        if self.voice_enrollment.is_some() {
            while let Some(event) = self
                .voice_enrollment
                .as_ref()
                .and_then(|receiver| receiver.try_recv().ok())
            {
                match event {
                    EnrollmentEvent::Captured(count) => {
                        self.voice_enrollment_status =
                            i18n::message("已录制 {count}/3 段", &[("count", &count.to_string())]);
                    }
                    EnrollmentEvent::Finished(result) => {
                        self.voice_enrollment_status = match result {
                            Ok(()) => {
                                self.voice_registered = true;
                                self.voice_registration_playback_filter =
                                    Some(self.draft.voice_playback_filter);
                                format!(
                                    "{} · {}",
                                    i18n::message("已录制 {count}/3 段", &[("count", "3")]),
                                    tr("本人声纹注册完成")
                                )
                            }
                            Err(error) => format!("{}: {error:#}", tr("声纹注册失败")),
                        };
                        self.voice_enrollment = None;
                        break;
                    }
                }
            }
            if self.voice_enrollment.is_some() {
                ui.ctx().request_repaint_after(Duration::from_millis(100));
            }
        }
        i18n::set_language(&self.draft.ui_language);
        self.expire_message(ui.ctx(), Instant::now());
        if ui.input(|input| input.pointer.any_pressed()) {
            self.active_number = None;
        }

        if self.word_editor.is_open() {
            self.active_shortcut = None;
            self.active_number = None;
            self.word_editor.show(ui);
            return None;
        }
        self.capture_shortcut(ui.ctx());

        ui_theme::apply(ui);
        ui.painter().rect_filled(ui.max_rect(), 0.0, canvas());
        ui.spacing_mut().item_spacing = egui::vec2(12.0, 12.0);

        let mut save = false;
        let allow_page_wheel = self.active_number.is_none();
        let footer_height: f32 = if self.message.is_some() { 94.0 } else { 68.0 };
        let mut footer_rect = egui::Rect::NOTHING;
        let mut scroll_offset = 0.0;
        egui::Frame::new()
            .inner_margin(egui::Margin::symmetric(28, 18))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new(tr("TO WORDS / PREFERENCES"))
                            .monospace()
                            .size(11.0)
                            .color(muted_text()),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let previous_language = self.draft.ui_language.clone();
                        egui::ComboBox::from_id_salt("ui_language")
                            .selected_text(match self.draft.ui_language.as_str() {
                                "en" => "English",
                                "ko" => "한국어",
                                "ja" => "日本語",
                                _ => "简体中文",
                            })
                            .width(170.0)
                            .show_ui(ui, |ui| {
                                for (code, name) in [
                                    ("zh-CN", "简体中文"),
                                    ("en", "English"),
                                    ("ko", "한국어"),
                                    ("ja", "日本語"),
                                ] {
                                    ui.selectable_value(&mut self.draft.ui_language, code.to_string(), name);
                                }
                            });
                        let language_label = if self.draft.ui_language == "zh-CN" {
                            "界面语言（language）"
                        } else {
                            tr("界面语言")
                        };
                        ui.label(egui::RichText::new(language_label).size(13.0).color(primary_text()));
                        if self.draft.ui_language != previous_language {
                            i18n::set_language(&self.draft.ui_language);
                            self.message = None;
                            self.ocr_status = ocr::engine_status();
                            self.voice_status = voice::engine_status(&self.draft.voice_backend);
                            ui.ctx().request_repaint();
                        }
                    });
                });
                ui.add_space(12.0);
                let body_rect = ui.available_rect_before_wrap();
                let visible_footer_height = footer_height.min(body_rect.height().max(0.0));
                footer_rect = egui::Rect::from_min_max(
                    egui::pos2(body_rect.left(), body_rect.bottom() - visible_footer_height),
                    body_rect.right_bottom(),
                );

                let mut scroll_area = egui::ScrollArea::vertical()
                    .id_salt("settings_scroll")
                    .auto_shrink([false, false])
                    .scroll_source(egui::scroll_area::ScrollSource {
                        mouse_wheel: allow_page_wheel,
                        ..Default::default()
                    });
                if std::mem::take(&mut self.scroll_to_top) {
                    scroll_area = scroll_area.vertical_scroll_offset(0.0);
                }
                let scroll = scroll_area.show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        settings_card(
                            ui,
                            "词库",
                            Some("选择原始语言与目标语言，并管理对应的用户词库。"),
                            |ui| {
                                self.draw_translation_selector(ui);
                                ui.add_space(12.0);
                                if secondary_button(ui, "编辑词库", 104.0).clicked() {
                                    self.open_word_editor(ui.ctx());
                                }
                            },
                        );
                        ui.add_space(14.0);

                        settings_card(
                            ui,
                            "AI 翻译 · DeepSeek",
                            Some("查询框可主动翻译；OCR 与语音识别可按设置自动翻译。"),
                            |ui| self.draw_ai_settings(ui),
                        );
                        ui.add_space(14.0);

                        settings_card(
                            ui,
                            "语音识别",
                            Some("本地录音与识别；无词库匹配时使用已配置的 DeepSeek 翻译。"),
                            |ui| {
                                ui.label(
                                    egui::RichText::new(tr("识别引擎"))
                                        .size(13.0)
                                        .strong()
                                        .color(primary_text()),
                                );
                                let old_backend = self.draft.voice_backend.clone();
                                ui.horizontal(|ui| {
                                    egui::ComboBox::from_id_salt("voice_backend")
                                        .selected_text(match self.draft.voice_backend.as_str() {
                                            "sensevoice" => tr("SenseVoice（默认）"),
                                            "whisper" => "whisper.cpp",
                                            _ => tr("SenseVoice（默认）"),
                                        })
                                        .width(220.0)
                                        .show_ui(ui, |ui| {
                                            ui.selectable_value(&mut self.draft.voice_backend, "sensevoice".to_string(), tr("SenseVoice（默认）"));
                                            ui.selectable_value(&mut self.draft.voice_backend, "whisper".to_string(), "whisper.cpp");
                                        });
                                });
                                if old_backend != self.draft.voice_backend {
                                    self.voice_status = voice::engine_status(&self.draft.voice_backend);
                                }
                                ui.add_space(8.0);
                                ui.label(
                                    egui::RichText::new(tr("识别语言"))
                                        .size(13.0)
                                        .strong()
                                        .color(primary_text()),
                                );
                                ui.horizontal(|ui| {
                                    ui.add_enabled_ui(self.draft.voice_backend == "whisper", |ui| {
                                        egui::ComboBox::from_id_salt("voice_language")
                                            .selected_text(if self.draft.voice_language == "auto" {
                                                tr("自动识别")
                                            } else {
                                                i18n::language_name(&self.draft.voice_language)
                                            })
                                            .width(220.0)
                                            .height(320.0)
                                            .show_ui(ui, |ui| {
                                                ui.selectable_value(
                                                    &mut self.draft.voice_language,
                                                    "auto".to_string(),
                                                    tr("自动识别"),
                                                );
                                                ui.separator();
                                                for language in translation_language::LANGUAGES {
                                                    ui.selectable_value(
                                                        &mut self.draft.voice_language,
                                                        language.code.to_string(),
                                                        tr(language.label),
                                                    );
                                                }
                                            });
                                    });
                                    help_icon(
                                        ui,
                                        if self.draft.voice_backend == "whisper" {
                                            "经常说同一种语言时，手动指定可减少短句误判。"
                                        } else {
                                            "SenseVoice 自动识别语言，不能通过此选项限定；识别语言设置仅用于 Whisper。"
                                        },
                                    );
                                });
                                ui.add_space(8.0);
                                ui.horizontal(|ui| {
                                    contrast_checkbox(
                                        ui,
                                        &mut self.draft.voice_keep_input,
                                        "是否保持语音输入",
                                    );
                                    help_icon(ui, "默认说完一句后自动停止；勾选后持续监听下一句，再按快捷键结束。");
                                });
                                ui.add_space(8.0);
                                ui.horizontal(|ui| {
                                    contrast_checkbox(
                                        ui,
                                        &mut self.draft.voice_hold_to_talk,
                                        "按住语音快捷键说话，松开后停止",
                                    );
                                    help_icon(ui, "启用后按下快捷键开始录音，松开快捷键的主按键时停止并识别最后一句。按住期间可连续说多句；此模式优先于“是否保持语音输入”。");
                                });
                                ui.add_space(8.0);
                                ui.horizontal(|ui| {
                                    contrast_checkbox(ui, &mut self.draft.voice_owner_filter, "仅识别已注册的本人声音");
                                    help_icon(ui, "使用本地声纹模型判断说话人；需要先录制三段本人语音。多人同时说话、录音回放或环境变化可能误判，不能作为身份认证。");
                                });
                                ui.horizontal_wrapped(|ui| {
                                    if ui.add_enabled(self.voice_enrollment.is_none(), egui::Button::new(tr("注册本人声纹"))).clicked() {
                                        match voice_identity::start_enrollment(self.draft.voice_playback_filter) {
                                            Ok(receiver) => {
                                                self.voice_enrollment = Some(receiver);
                                                self.voice_enrollment_status = i18n::message("已录制 {count}/3 段", &[("count", "0")]);
                                            }
                                            Err(error) => self.voice_enrollment_status = format!("{}: {error:#}", tr("声纹注册失败")),
                                        }
                                    }
                                    if ui.add_enabled(self.voice_enrollment.is_none() && self.voice_registered, egui::Button::new(tr("删除声纹"))).clicked() {
                                        match voice_identity::clear_profile() {
                                            Ok(()) => {
                                                self.voice_registered = false;
                                                self.voice_registration_playback_filter = None;
                                                self.draft.voice_owner_filter = false;
                                                self.voice_enrollment_status = tr("声纹已删除，请保存并应用设置").to_string();
                                            }
                                            Err(error) => self.voice_enrollment_status = format!("{}: {error:#}", tr("删除声纹失败")),
                                        }
                                    }
                                    ui.label(egui::RichText::new(&self.voice_enrollment_status).size(12.0).color(muted_text()));
                                });
                                if self.voice_enrollment.is_some() {
                                    ui.add_space(10.0);
                                    voice_enrollment_guide(ui);
                                }
                                ui.add_space(8.0);
                                ui.horizontal(|ui| {
                                    contrast_checkbox(ui, &mut self.draft.voice_playback_filter, "过滤电脑播放声");
                                    help_icon(ui, "优先使用 Windows 通信麦克风的回声消除；设备不支持时改用系统播放声回采和软件消除。只处理电脑播放声，不能消除手机外放或旁人说话；耳机也有帮助。");
                                });
                                if self.voice_registered
                                    && self.voice_registration_playback_filter
                                        != Some(self.draft.voice_playback_filter)
                                {
                                    ui.colored_label(
                                        ui.visuals().warn_fg_color,
                                        tr("声纹与当前播放声过滤设置不一致，请重新注册"),
                                    );
                                }
                                ui.add_space(8.0);
                                egui::Grid::new("voice_silence_settings")
                                    .num_columns(2)
                                    .spacing([16.0, 8.0])
                                    .show(ui, |ui| {
                                        number_row(
                                            ui,
                                            "voice_silence_seconds",
                                            "语音停顿时间（秒）",
                                            &mut self.draft.voice_silence_seconds,
                                            0.2..=3.0,
                                            0.1,
                                            &mut self.active_number,
                                            &mut self.last_wheel_change,
                                        );
                                    });
                                ui.horizontal(|ui| {
                                    ui.label(egui::RichText::new(tr("点击数值后滚轮调整，每次 0.1 秒"))
                                        .size(12.0)
                                        .color(muted_text()));
                                    help_icon(ui, "范围 0.2–3.0 秒，默认 0.6 秒。按住说话模式会等到松开快捷键才提交，停顿时间对该模式不生效。");
                                });
                                ui.add_space(8.0);
                                ui.horizontal(|ui| {
                                    contrast_checkbox(
                                        ui,
                                        &mut self.draft.voice_auto_copy_first,
                                        "多条词库匹配时自动输入第一条",
                                    );
                                    help_icon(ui, "关闭后会打开查询框，供你自行选择匹配结果。");
                                });
                                ui.add_space(8.0);
                                ui.horizontal(|ui| {
                                    contrast_checkbox(
                                        ui,
                                        &mut self.draft.voice_aion2_manual_paste,
                                        "Aion2 兼容模式：复制语音结果，手动粘贴",
                                    );
                                    help_icon(ui, "默认关闭；仅在 Aion2 位于前台时生效，其他程序仍自动输入。");
                                });
                                ui.add_space(8.0);
                                ui.label(
                                    egui::RichText::new(&self.voice_status)
                                        .size(12.0)
                                        .color(muted_text()),
                                );
                            },
                        );
                        ui.add_space(14.0);

                        settings_card(
                            ui,
                            "快捷键",
                            None,
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
                                    None,
                                    |ui| {
                                        ui.set_min_height(390.0);
                                        self.draw_input_settings(ui);
                                    },
                                );
                                settings_card(
                                    &mut columns[1],
                                    "查询结果",
                                    None,
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
                                None,
                                |ui| self.draw_input_settings(ui),
                            );
                            ui.add_space(14.0);
                            settings_card(
                                ui,
                                "查询结果",
                                None,
                                |ui| self.draw_result_settings(ui),
                            );
                        }
                        ui.add_space(14.0);
                        settings_card(
                            ui,
                            "通用",
                            None,
                            |ui| self.draw_general_settings(ui),
                        );
                        ui.add_space(footer_height + 16.0);
                    });
                scroll_offset = scroll.state.offset.y;
            });

        if footer_rect.is_positive() {
            self.draw_actions_footer(ui.ctx(), footer_rect, scroll_offset, &mut save);
        }

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

    fn draw_actions_footer(
        &mut self,
        context: &egui::Context,
        rect: egui::Rect,
        scroll_offset: f32,
        save: &mut bool,
    ) {
        let dark = self.glass_footer.background_is_dark();
        egui::Area::new(egui::Id::new("settings_actions_footer"))
            .order(egui::Order::Foreground)
            .fixed_pos(rect.min)
            .movable(false)
            .show(context, |ui| {
                ui.set_min_size(rect.size());
                ui.set_max_size(rect.size());
                let paint_rect = egui::Rect::from_min_size(ui.min_rect().min, rect.size());
                self.glass_footer.paint(ui, paint_rect, scroll_offset);
                egui::Frame::new()
                    .inner_margin(egui::Margin::symmetric(16, 10))
                    .show(ui, |ui| {
                        ui.set_width((rect.width() - 32.0).max(1.0));
                        if let Some(message) = &self.message {
                            status_message(ui, &message.text, message.is_error, dark);
                            ui.add_space(7.0);
                        }
                        ui.horizontal(|ui| {
                            let icon_width = 40.0;
                            let available = (ui.available_width()
                                - icon_width
                                - 2.0 * ui.spacing().item_spacing.x)
                                .max(0.0);
                            let save_width = (available * 0.52).min(142.0);
                            let reset_width = (available - save_width).min(130.0);
                            if glass_action_button(ui, "保存并应用", save_width, true, dark)
                                .clicked()
                            {
                                *save = true;
                            }
                            if glass_action_button(ui, "恢复默认值", reset_width, false, dark)
                                .clicked()
                            {
                                self.draft = UiConfig::default();
                                self.message = None;
                                self.active_shortcut = None;
                                self.active_number = None;
                                self.ai_key_input.clear();
                                self.remove_ai_key = false;
                            }
                            if scroll_to_top_visible(scroll_offset) {
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        if glass_scroll_to_top_button(ui, dark).clicked() {
                                            self.scroll_to_top = true;
                                            context.request_repaint();
                                        }
                                    },
                                );
                            }
                        });
                    });
            });
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
                egui::RichText::new(tr("模型：deepseek-flash"))
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
        ui.horizontal(|ui| {
            contrast_checkbox(
                ui,
                &mut self.draft.ai_polite_mode,
                "敬语模式",
            );
            help_icon(ui, "AI 正向或反向翻译时，按译文语言使用自然礼貌的表达；没有专门敬语形式的语言会使用礼貌语气。词库已有的固定句式不受影响。");
        });
        ui.add_space(8.0);
        self.draw_ocr_auto_translation(ui);
        ui.add_space(8.0);
        ui.horizontal_wrapped(|ui| {
            ui.label(
                egui::RichText::new(tr("API Key"))
                    .size(13.0)
                    .color(muted_text()),
            );
            ui.add_sized(
                [300.0, 32.0],
                egui::TextEdit::singleline(&mut self.ai_key_input)
                    .password(true)
                    .hint_text(if self.ai_key_present && !self.remove_ai_key {
                        tr("已安全保存；留空则保持不变")
                    } else {
                        tr("输入你的 DeepSeek API Key")
                    }),
            );
            if self.ai_key_present && secondary_button(ui, "删除密钥", 94.0).clicked() {
                self.ai_key_input.clear();
                self.remove_ai_key = true;
            }
            help_icon(
                ui,
                "密钥保存在 Windows 凭据管理器，不写入 to_words.db。未选择目标语言时无法请求翻译。",
            );
        });
        if self.remove_ai_key {
            ui.label(
                egui::RichText::new(tr("保存并应用后将删除已保存的密钥。"))
                    .size(12.0)
                    .color(red_text()),
            );
        }
    }

    fn draw_ocr_auto_translation(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            contrast_checkbox(
                ui,
                &mut self.draft.ocr_auto_translate,
                "OCR 识别后自动翻译成原始语言",
            );
            help_icon(ui, "松开截图选区后自动显示纯文字译文；点击 √ 才复制 OCR 原文并关闭。开启后仅向 DeepSeek 发送识别文字，不发送截图。原始语言为“自动检测”时译成简体中文。需要 API Key，可能产生费用。");
        });
    }

    fn draw_translation_selector(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            ui.vertical(|ui| {
                ui.label(
                    egui::RichText::new(tr("原始语言"))
                        .size(13.0)
                        .strong()
                        .color(primary_text()),
                );
                egui::ComboBox::from_id_salt("source_language")
                    .selected_text(if self.draft.source_language == "auto" {
                        tr("自动检测")
                    } else {
                        i18n::language_name(&self.draft.source_language)
                    })
                    .width(220.0)
                    .height(320.0)
                    .show_ui(ui, |ui| {
                        ui.selectable_value(
                            &mut self.draft.source_language,
                            "auto".to_string(),
                            tr("自动检测"),
                        );
                        ui.separator();
                        for language in translation_language::LANGUAGES {
                            ui.selectable_value(
                                &mut self.draft.source_language,
                                language.code.to_string(),
                                tr(language.label),
                            );
                        }
                    });
            });
            ui.add_space(12.0);
            ui.vertical(|ui| {
                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new(tr("目标语言"))
                            .size(13.0)
                            .strong()
                            .color(primary_text()),
                    );
                    if self.draft.source_language == "auto" {
                        let explanation = i18n::message(
                            "自动检测当前使用：{language}；语言难以判断时沿用该词库。缺少的词库会自动创建。",
                            &[(
                                "language",
                                i18n::language_name(&self.active_source_language),
                            )],
                        );
                        help_icon(ui, &explanation);
                    } else {
                        help_icon(ui, "缺少的词库会自动创建，不会覆盖已有词条。");
                    }
                });
                egui::ComboBox::from_id_salt("translation_language")
                    .selected_text(i18n::language_name(&self.draft.translation_language))
                    .width(220.0)
                    .height(320.0)
                    .show_ui(ui, |ui| {
                        ui.selectable_value(
                            &mut self.draft.translation_language,
                            String::new(),
                            tr("未选择（默认词库）"),
                        );
                        ui.separator();
                        for language in translation_language::LANGUAGES {
                            ui.selectable_value(
                                &mut self.draft.translation_language,
                                language.code.to_string(),
                                tr(language.label),
                            );
                        }
                    });
            });
        });
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
        self.word_editor_review_requested = true;

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
                    &mut self.recording_started_at,
                    ShortcutTarget::Popup,
                );
                shortcut_row(
                    ui,
                    "显示设置面板",
                    &mut self.draft.hotkeys.settings,
                    &mut self.active_shortcut,
                    &mut self.recording_started_at,
                    ShortcutTarget::Settings,
                );
                shortcut_row(
                    ui,
                    "框选 OCR 识别",
                    &mut self.draft.hotkeys.ocr,
                    &mut self.active_shortcut,
                    &mut self.recording_started_at,
                    ShortcutTarget::Ocr,
                );
                shortcut_row(
                    ui,
                    "开关语音录音",
                    &mut self.draft.hotkeys.voice,
                    &mut self.active_shortcut,
                    &mut self.recording_started_at,
                    ShortcutTarget::Voice,
                );
                shortcut_row(
                    ui,
                    "清空组合内容",
                    &mut self.draft.hotkeys.clear_composed,
                    &mut self.active_shortcut,
                    &mut self.recording_started_at,
                    ShortcutTarget::ClearComposed,
                );
            });
        ui.add_space(8.0);
        egui::CollapsingHeader::new(tr("G502 LIGHTSPEED 扩展键"))
            .default_open(false)
            .show(ui, |ui| {
                for line in [
                    "后退/前进键若录制为 MouseX1/MouseX2，可直接使用；其他按键只有在驱动提供独立鼠标事件时才能录制为 Mouse6～Mouse8。",
                    "DPI、G-Shift 等按键可在 G HUB 中分别映射为不同的键盘快捷键，再点击上方对应设置项录制；推荐使用不与其他程序冲突的 F13～F24（若 G HUB 提供）。",
                    "重新映射会替换该按键原来的 DPI 或 G-Shift 功能。",
                    "如切换到游戏后快捷键失效，请检查 G HUB 是否切换了配置文件；可将映射配置设为持久配置。",
                ] {
                    ui.label(
                        egui::RichText::new(tr(line))
                            .size(12.0)
                            .color(muted_text()),
                    );
                    ui.add_space(4.0);
                }
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
                    egui::RichText::new(tr("保持打开"))
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
                    egui::RichText::new(tr("返回后自动清空"))
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

        self.set_recorded_shortcut(target, shortcut_text(key, physical_modifiers(modifiers)));
    }

    fn set_recorded_shortcut(&mut self, target: ShortcutTarget, result: Result<String, String>) {
        match result {
            Ok(shortcut) => {
                match target {
                    ShortcutTarget::Popup => self.draft.hotkeys.popup = shortcut,
                    ShortcutTarget::Settings => self.draft.hotkeys.settings = shortcut,
                    ShortcutTarget::Ocr => self.draft.hotkeys.ocr = shortcut,
                    ShortcutTarget::Voice => self.draft.hotkeys.voice = shortcut,
                    ShortcutTarget::ClearComposed => self.draft.hotkeys.clear_composed = shortcut,
                }
                self.active_shortcut = None;
                self.recording_started_at = None;
                self.message = None;
            }
            Err(error) => self.message = Some(SettingsMessage::error(error)),
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
            egui::RichText::new(tr(text))
                .size(13.0)
                .color(primary_text()),
        )
    })
    .inner
}

fn help_icon(ui: &mut egui::Ui, explanation: &str) {
    let (rect, response) = ui.allocate_exact_size(egui::vec2(18.0, 18.0), egui::Sense::click());
    let color = if response.hovered() {
        accent_text()
    } else {
        muted_text()
    };
    let center = rect.center();
    let painter = ui.painter();
    painter.circle_filled(
        center,
        7.0,
        if response.hovered() {
            accent_fill()
        } else {
            control_surface()
        },
    );
    painter.circle_stroke(center, 7.0, egui::Stroke::new(1.3, color));
    painter.circle_filled(egui::pos2(center.x, center.y - 2.5), 1.0, color);
    painter.rect_filled(
        egui::Rect::from_center_size(egui::pos2(center.x, center.y + 1.5), egui::vec2(1.6, 4.2)),
        0.8,
        color,
    );
    let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
    egui::Popup::from_toggle_button_response(&response)
        .width(360.0)
        .frame(ui_theme::tooltip_frame())
        .show(|ui| {
            ui.label(
                egui::RichText::new(tr(explanation))
                    .size(13.0)
                    .color(primary_text()),
            );
        });
}

fn voice_enrollment_guide(ui: &mut egui::Ui) {
    egui::Frame::new()
        .fill(control_surface())
        .corner_radius(8)
        .inner_margin(egui::Margin::same(12))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(
                egui::RichText::new(tr("请依次朗读以下三句话"))
                    .size(13.0)
                    .strong()
                    .color(primary_text()),
            );
            ui.label(egui::RichText::new(tr("请使用平时的麦克风，并保持当前“过滤电脑播放声”设置不变。"))
                .size(12.0).color(muted_text()));
            ui.add_space(6.0);
            for sentence in [
                "1. 今天我想试试语音输入，看看识别是否准确。",
                "2. 请把这句话记下来，等一下我还会继续说话。",
                "3. 现在我会用平时的声音，完成最后一段录音。",
            ] {
                ui.add(egui::Label::new(egui::RichText::new(tr(sentence)).color(primary_text())).wrap());
            }
            ui.add_space(8.0);
            ui.add(egui::Label::new(egui::RichText::new(tr("点击“注册本人声纹”后先等约 1 秒，再逐句朗读。每句说约 3–5 秒，每句后（包括最后一句）停顿约 1 秒，看界面是否依次显示 1/3、2/3、3/3。"))
                .size(12.0).color(muted_text())).wrap());
            ui.add_space(4.0);
            ui.add(egui::Label::new(egui::RichText::new(tr("当前程序要求三段各至少 1.5 秒，并靠停顿分段；使用不同句子也能提供更丰富的声音样本。"))
                .size(12.0).color(muted_text())).wrap());
        });
}

fn settings_card<R>(
    ui: &mut egui::Ui,
    title: &str,
    description: Option<&str>,
    add_contents: impl FnOnce(&mut egui::Ui) -> R,
) -> R {
    egui::Frame::new()
        .fill(surface())
        .stroke(egui::Stroke::new(1.0, border()))
        .corner_radius(8)
        .inner_margin(egui::Margin::same(18))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new(tr(title))
                        .size(17.0)
                        .strong()
                        .color(primary_text()),
                );
                if let Some(description) = description {
                    help_icon(ui, description);
                }
            });
            ui.add_space(10.0);
            add_contents(ui)
        })
        .inner
}

fn secondary_button(ui: &mut egui::Ui, text: &str, width: f32) -> egui::Response {
    ui.add_sized(
        [width, 36.0],
        egui::Button::new(
            egui::RichText::new(tr(text))
                .size(13.5)
                .color(primary_text()),
        )
        .fill(control_surface())
        .stroke(egui::Stroke::new(1.0, border()))
        .corner_radius(6),
    )
}

fn glass_action_button(
    ui: &mut egui::Ui,
    text: &str,
    width: f32,
    primary: bool,
    dark_backdrop: bool,
) -> egui::Response {
    let (fill, text_color, stroke) = match (primary, dark_backdrop) {
        (true, false) => (
            egui::Color32::from_rgba_unmultiplied(31, 35, 38, 238),
            egui::Color32::WHITE,
            egui::Color32::from_rgba_unmultiplied(255, 255, 255, 85),
        ),
        (true, true) => (
            egui::Color32::from_rgba_unmultiplied(242, 246, 249, 244),
            egui::Color32::from_rgb(28, 33, 38),
            egui::Color32::from_rgba_unmultiplied(255, 255, 255, 140),
        ),
        (false, false) => (
            egui::Color32::from_rgba_unmultiplied(248, 250, 250, 152),
            egui::Color32::from_rgb(47, 53, 57),
            egui::Color32::from_rgba_unmultiplied(255, 255, 255, 168),
        ),
        (false, true) => (
            egui::Color32::from_rgba_unmultiplied(226, 236, 243, 50),
            egui::Color32::from_rgb(246, 248, 250),
            egui::Color32::from_rgba_unmultiplied(226, 236, 243, 114),
        ),
    };
    ui.add_sized(
        [width, 40.0],
        egui::Button::new(
            egui::RichText::new(tr(text))
                .size(13.5)
                .strong()
                .color(text_color),
        )
        .fill(fill)
        .stroke(egui::Stroke::new(1.0, stroke))
        .corner_radius(13),
    )
}

fn glass_scroll_to_top_button(ui: &mut egui::Ui, dark_backdrop: bool) -> egui::Response {
    let (fill, text_color, stroke) = if dark_backdrop {
        (
            egui::Color32::from_rgba_unmultiplied(226, 236, 243, 50),
            egui::Color32::from_rgb(246, 248, 250),
            egui::Color32::from_rgba_unmultiplied(226, 236, 243, 114),
        )
    } else {
        (
            egui::Color32::from_rgba_unmultiplied(248, 250, 250, 152),
            egui::Color32::from_rgb(47, 53, 57),
            egui::Color32::from_rgba_unmultiplied(255, 255, 255, 168),
        )
    };
    let response = ui.add_sized(
        [40.0, 40.0],
        egui::Button::new("")
            .fill(fill)
            .stroke(egui::Stroke::new(1.0, stroke))
            .corner_radius(13),
    );
    let center = response.rect.center();
    ui.painter().add(egui::Shape::line(
        vec![
            egui::pos2(center.x - 7.0, center.y + 3.0),
            egui::pos2(center.x, center.y - 4.0),
            egui::pos2(center.x + 7.0, center.y + 3.0),
        ],
        egui::Stroke::new(2.0, text_color),
    ));
    response.on_hover_text(tr("返回顶部"))
}

fn scroll_to_top_visible(offset: f32) -> bool {
    offset > 1.0
}

fn status_message(ui: &mut egui::Ui, message: &str, is_error: bool, dark_backdrop: bool) {
    let color = match (is_error, dark_backdrop) {
        (true, false) => red_text(),
        (false, false) => green_text(),
        (true, true) => egui::Color32::from_rgb(255, 184, 185),
        (false, true) => egui::Color32::from_rgb(182, 233, 187),
    };
    ui.add(egui::Label::new(egui::RichText::new(message).size(13.0).color(color)).truncate())
        .on_hover_text(message);
}

fn text_row(ui: &mut egui::Ui, label: &str, value: &mut String) {
    ui.label(
        egui::RichText::new(tr(label))
            .size(13.0)
            .color(muted_text()),
    );
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
    recording_started_at: &mut Option<Instant>,
    target: ShortcutTarget,
) {
    ui.label(
        egui::RichText::new(tr(label))
            .size(13.0)
            .color(muted_text()),
    );
    let recording = *active == Some(target);
    let text = if recording || value.is_empty() {
        tr("请按键或鼠标按键…")
    } else {
        value.as_str()
    };
    let width = ui.available_width().clamp(132.0, 280.0);
    let fill = if recording {
        pale_yellow()
    } else {
        control_surface()
    };
    let response =
        ui.add_sized(
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
        );
    if response.clicked() {
        value.clear();
        *active = Some(target);
        *recording_started_at = Some(Instant::now());
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
    let voice_pause = id_source == "voice_silence_seconds";
    ui.label(
        egui::RichText::new(tr(label))
            .size(13.0)
            .color(muted_text()),
    );
    let id = ui.make_persistent_id(id_source);
    let selected = *active == Some(id);
    let text = if voice_pause {
        format!("{value:.1}")
    } else if step < 1.0 {
        format!("{value:.2}")
    } else {
        format!("{value:.0}")
    };
    let width = ui.available_width().clamp(132.0, 180.0);
    let response =
        ui.add_sized(
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
        );
    if response.clicked() {
        *active = Some(id);
        response.request_focus();
    }
    if *active == Some(id) {
        let (wheel, now) = ui.input(|input| {
            let wheel = if voice_pause {
                // 平滑滚动带有惯性，同一次滚轮动作可能跨多帧造成连续跳值。
                input
                    .events
                    .iter()
                    .filter_map(|event| match event {
                        egui::Event::MouseWheel { delta, .. } => Some(delta.y),
                        _ => None,
                    })
                    .sum()
            } else {
                input.smooth_scroll_delta().y
            };
            (wheel, input.time)
        });
        let cooldown = if voice_pause { 0.2 } else { 0.03 };
        if wheel != 0.0 && now - *last_wheel_change >= cooldown {
            if voice_pause {
                *value = step_voice_pause(*value, wheel, &range);
            } else {
                let direction = if wheel > 0.0 { 1.0 } else { -1.0 };
                *value = (*value + direction * step).clamp(*range.start(), *range.end());
            }

            *last_wheel_change = now;
        }
    }
    ui.end_row();
}

fn step_voice_pause(current: f32, wheel: f32, range: &std::ops::RangeInclusive<f32>) -> f32 {
    let current_tenths = (current * 10.0).round() as i32;
    let min_tenths = (*range.start() * 10.0).round() as i32;
    let max_tenths = (*range.end() * 10.0).round() as i32;
    let direction = if wheel > 0.0 { 1 } else { -1 };
    (current_tenths + direction).clamp(min_tenths, max_tenths) as f32 / 10.0
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
    ui.label(
        egui::RichText::new(tr(label))
            .size(13.0)
            .color(muted_text()),
    );
    let id = ui.make_persistent_id(id_source);
    let selected = *active == Some(id);
    let width = ui.available_width().clamp(132.0, 180.0);
    let response = ui.add_sized(
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
    );
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

pub(crate) fn shortcut_released(context: &egui::Context, configured: &str) -> bool {
    let key_name = configured.rsplit('+').next().unwrap_or(configured);
    context.input(|input| {
        input.events.iter().any(|event| {
            matches!(event, egui::Event::Key { key, pressed: false, .. }
                if key.name().eq_ignore_ascii_case(key_name))
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
        .parse::<Shortcut>()
        .map(|_| shortcut)
        .map_err(|_| i18n::message("不支持这个按键：{key}", &[("key", key.name())]))
}

#[cfg(test)]
mod tests {
    use super::{
        SettingsPanel, ShortcutTarget, scroll_to_top_visible, shortcut_text, step_voice_pause,
    };
    use crate::UiConfig;
    use crate::domain::shortcut::{MouseButton, MouseModifiers};
    use crate::platform::mouse_hotkey::MouseButtonEvent;
    use eframe::egui;
    use std::time::{Duration, Instant};

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

    #[test]
    fn records_g_hub_keyboard_mappings() {
        assert_eq!(
            shortcut_text(egui::Key::F13, egui::Modifiers::NONE).unwrap(),
            "F13"
        );
        assert_eq!(
            shortcut_text(egui::Key::F24, egui::Modifiers::NONE).unwrap(),
            "F24"
        );
    }

    #[test]
    fn mouse_recording_ignores_the_click_that_started_it() {
        let mut panel = SettingsPanel::new(UiConfig::default());
        let started = Instant::now();
        panel.active_shortcut = Some(ShortcutTarget::Popup);
        panel.recording_started_at = Some(started);
        panel.capture_mouse_shortcut(MouseButtonEvent {
            button: MouseButton::Left,
            pressed: true,
            modifiers: MouseModifiers::default(),
            occurred_at: started - Duration::from_millis(1),
        });
        assert!(panel.is_recording_shortcut());
        panel.capture_mouse_shortcut(MouseButtonEvent {
            button: MouseButton::X1,
            pressed: true,
            modifiers: MouseModifiers::new(true, false, false, false),
            occurred_at: started + Duration::from_millis(1),
        });
        assert_eq!(panel.draft.hotkeys.popup, "ctrl+MouseX1");
        assert!(!panel.is_recording_shortcut());
    }

    #[test]
    fn voice_pause_wheel_changes_one_tenth_per_event() {
        let range = 0.2..=3.0;
        assert_eq!(step_voice_pause(0.6, 120.0, &range), 0.7);
        assert_eq!(step_voice_pause(0.6, -120.0, &range), 0.5);
        assert_eq!(step_voice_pause(3.0, 120.0, &range), 3.0);
        assert_eq!(step_voice_pause(0.2, -120.0, &range), 0.2);
    }

    #[test]
    fn scroll_to_top_icon_is_hidden_at_the_top() {
        assert!(!scroll_to_top_visible(0.0));
        assert!(!scroll_to_top_visible(1.0));
        assert!(scroll_to_top_visible(2.0));
    }

    #[test]
    fn saved_message_expires_after_1_2_seconds_but_error_remains() {
        let context = egui::Context::default();
        let mut panel = SettingsPanel::new(UiConfig::default());
        let before_save = std::time::Instant::now();
        panel.set_result(Ok(()));
        let after_save = std::time::Instant::now();
        let first_deadline = panel.message.as_ref().unwrap().expires_at.unwrap();
        assert!(first_deadline >= before_save + Duration::from_millis(1200));
        assert!(first_deadline <= after_save + Duration::from_millis(1200));
        panel.expire_message(&context, first_deadline - Duration::from_millis(1));
        assert!(panel.message.is_some());

        panel.set_result(Ok(()));
        let renewed_deadline = panel.message.as_ref().unwrap().expires_at.unwrap();
        assert!(renewed_deadline >= first_deadline);
        panel.expire_message(&context, renewed_deadline);
        assert!(panel.message.is_none());

        panel.set_result(Err("save failed".to_string()));
        panel.expire_message(&context, renewed_deadline + Duration::from_secs(10));
        assert_eq!(panel.message.as_ref().unwrap().text, "save failed");
    }

    #[test]
    fn actions_footer_stays_near_viewport_bottom() {
        let context = egui::Context::default();
        let mut panel = SettingsPanel::new(UiConfig::default());
        let mut output = context.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(800.0, 600.0),
                )),
                ..Default::default()
            },
            |ui| {
                panel.show(ui);
            },
        );
        output.textures_delta.clear();
        let footer = context
            .memory(|memory| memory.area_rect(egui::Id::new("settings_actions_footer")))
            .expect("settings footer area");
        assert!(
            footer.top() > 480.0,
            "footer should be fixed below scroll content"
        );
        assert!(footer.bottom() <= 600.0);

        let mut output = context.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(800.0, 600.0),
                )),
                events: vec![
                    egui::Event::PointerMoved(egui::pos2(400.0, 300.0)),
                    egui::Event::MouseWheel {
                        unit: egui::MouseWheelUnit::Point,
                        delta: egui::vec2(0.0, -320.0),
                        phase: egui::TouchPhase::Move,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
                ..Default::default()
            },
            |ui| {
                panel.show(ui);
            },
        );
        output.textures_delta.clear();
        let after_scroll = context
            .memory(|memory| memory.area_rect(egui::Id::new("settings_actions_footer")))
            .expect("settings footer after scroll");
        assert_eq!(footer.min, after_scroll.min);
        assert_eq!(footer.max, after_scroll.max);
    }
}
