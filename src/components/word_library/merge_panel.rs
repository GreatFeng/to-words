//! 词库合并面板组件。
//!
//! 面板加载可合并的词库文件、保存用户勾选状态，并调用领域层完成去重和合并。
//! `WordMergePanel` 同时维护操作提示与“合并后重新加载词库”的请求状态。

use crate::ui_theme;
use crate::word_merge::{list_word_library_files, merge_selected_word_files};
use eframe::egui;

#[derive(Default)]
pub(crate) struct WordMergePanel {
    open: bool,
    target_file_name: String,
    candidates: Vec<MergeCandidate>,
    message: Option<(String, bool)>,
    reload_requested: bool,
}

struct MergeCandidate {
    file_name: String,
    selected: bool,
}

impl WordMergePanel {
    pub(crate) fn open(&mut self, target_file_name: String) {
        self.open = true;
        self.target_file_name = target_file_name;
        self.message = None;
        self.refresh_files();
    }

    pub(crate) fn close(&mut self) {
        self.open = false;
    }

    pub(crate) fn is_open(&self) -> bool {
        self.open
    }

    pub(crate) fn take_reload_requested(&mut self) -> bool {
        std::mem::take(&mut self.reload_requested)
    }

    pub(crate) fn show(&mut self, ui: &mut egui::Ui) {
        ui_theme::apply(ui);
        ui.painter()
            .rect_filled(ui.max_rect(), 0.0, ui_theme::canvas());
        if ui.input(|input| input.key_pressed(egui::Key::Escape)) {
            self.open = false;
            return;
        }

        egui::Frame::new()
            .inner_margin(egui::Margin::same(28))
            .show(ui, |ui| {
                if back_button(ui, "返回设置").clicked() {
                    self.open = false;
                    return;
                }
                ui.add_space(12.0);
                ui.label(
                    egui::RichText::new("TO WORDS / MERGE")
                        .monospace()
                        .size(11.0)
                        .color(ui_theme::muted_text()),
                );
                ui.label(
                    egui::RichText::new("选择要合并的词库")
                        .size(27.0)
                        .strong()
                        .color(ui_theme::primary_text()),
                );
                ui.label(
                    egui::RichText::new("勾选一个或多个来源文件，内容将写入当前语言词库。")
                        .size(13.0)
                        .color(ui_theme::muted_text()),
                );
                ui.add_space(16.0);

                target_card(ui, &self.target_file_name);
                ui.add_space(14.0);

                ui.horizontal_wrapped(|ui| {
                    if secondary_button(ui, "全部选择", 96.0).clicked() {
                        for candidate in &mut self.candidates {
                            candidate.selected = true;
                        }
                    }
                    if secondary_button(ui, "清空选择", 96.0).clicked() {
                        for candidate in &mut self.candidates {
                            candidate.selected = false;
                        }
                    }
                    if secondary_button(ui, "刷新文件", 96.0).clicked() {
                        self.refresh_files();
                    }
                });
                ui.add_space(10.0);

                let selected_count = self
                    .candidates
                    .iter()
                    .filter(|candidate| candidate.selected)
                    .count();
                ui.label(
                    egui::RichText::new(format!(
                        "词库目录中共有 {} 个可选文件，已选择 {} 个",
                        self.candidates.len(),
                        selected_count
                    ))
                    .size(12.0)
                    .color(ui_theme::muted_text()),
                );
                ui.add_space(6.0);

                let list_height = (ui.available_height() - 126.0).max(160.0);
                egui::Frame::new()
                    .fill(ui_theme::surface())
                    .stroke(egui::Stroke::new(1.0, ui_theme::border()))
                    .corner_radius(8)
                    .inner_margin(egui::Margin::same(8))
                    .show(ui, |ui| {
                        egui::ScrollArea::vertical()
                            .id_salt("word_merge_sources")
                            .max_height(list_height)
                            .auto_shrink([false, false])
                            .show(ui, |ui| {
                                ui.set_width(ui.available_width());
                                if self.candidates.is_empty() {
                                    ui.add_space(20.0);
                                    ui.centered_and_justified(|ui| {
                                        ui.label(
                                            egui::RichText::new("没有其他可合并的 JSON 词库")
                                                .color(ui_theme::muted_text()),
                                        );
                                    });
                                }
                                for candidate in &mut self.candidates {
                                    candidate_row(ui, candidate);
                                }
                            });
                    });

                ui.add_space(14.0);
                ui.horizontal(|ui| {
                    let enabled = selected_count > 0;
                    if ui
                        .add_enabled(
                            enabled,
                            egui::Button::new(
                                egui::RichText::new("合并所选词库")
                                    .size(14.0)
                                    .strong()
                                    .color(egui::Color32::WHITE),
                            )
                            .fill(egui::Color32::from_rgb(31, 31, 30))
                            .stroke(egui::Stroke::new(1.0, ui_theme::border()))
                            .corner_radius(6)
                            .min_size(egui::vec2(132.0, 40.0)),
                        )
                        .clicked()
                    {
                        self.merge_selected();
                    }
                });
                if let Some((message, is_error)) = &self.message {
                    ui.add_space(10.0);
                    status_message(ui, message, *is_error);
                }
            });
    }

    fn refresh_files(&mut self) {
        match list_word_library_files(&self.target_file_name) {
            Ok(files) => {
                self.candidates = files
                    .into_iter()
                    .map(|file_name| MergeCandidate {
                        file_name,
                        selected: false,
                    })
                    .collect();
            }
            Err(error) => {
                self.candidates.clear();
                self.message = Some((format!("无法读取词库目录：{error:#}"), true));
            }
        }
    }

    fn merge_selected(&mut self) {
        let selected = self
            .candidates
            .iter()
            .filter(|candidate| candidate.selected)
            .map(|candidate| candidate.file_name.clone())
            .collect::<Vec<_>>();
        match merge_selected_word_files(&self.target_file_name, &selected) {
            Ok(report) => {
                self.reload_requested = true;
                self.message = Some((
                    format!(
                        "已合并 {} 个词库：新增 {} 项，跳过重复 {} 项，覆盖冲突 {} 项，当前共 {} 项。",
                        report.source_files,
                        report.added,
                        report.duplicates,
                        report.conflicts,
                        report.total
                    ),
                    false,
                ));
            }
            Err(error) => {
                self.message = Some((format!("词库合并失败：{error:#}"), true));
            }
        }
    }
}

fn target_card(ui: &mut egui::Ui, file_name: &str) {
    egui::Frame::new()
        .fill(ui_theme::control_surface())
        .stroke(egui::Stroke::new(1.0, ui_theme::border()))
        .corner_radius(8)
        .inner_margin(egui::Margin::symmetric(14, 10))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(
                egui::RichText::new("合并目标")
                    .size(11.0)
                    .color(ui_theme::muted_text()),
            );
            ui.label(
                egui::RichText::new(file_name)
                    .monospace()
                    .size(14.0)
                    .strong()
                    .color(ui_theme::primary_text()),
            );
        });
}

fn candidate_row(ui: &mut egui::Ui, candidate: &mut MergeCandidate) {
    egui::Frame::new()
        .fill(ui_theme::surface())
        .stroke(egui::Stroke::new(1.0, ui_theme::border()))
        .inner_margin(egui::Margin::symmetric(12, 8))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.checkbox(
                &mut candidate.selected,
                egui::RichText::new(&candidate.file_name)
                    .monospace()
                    .size(13.0)
                    .color(ui_theme::primary_text()),
            );
        });
}

fn back_button(ui: &mut egui::Ui, text: &str) -> egui::Response {
    secondary_button(ui, text, 112.0)
}

fn secondary_button(ui: &mut egui::Ui, text: &str, width: f32) -> egui::Response {
    ui.add_sized(
        [width, 34.0],
        egui::Button::new(
            egui::RichText::new(text)
                .size(13.0)
                .strong()
                .color(ui_theme::primary_text()),
        )
        .fill(ui_theme::control_surface())
        .stroke(egui::Stroke::new(1.0, ui_theme::border()))
        .corner_radius(5),
    )
}

fn status_message(ui: &mut egui::Ui, message: &str, is_error: bool) {
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
