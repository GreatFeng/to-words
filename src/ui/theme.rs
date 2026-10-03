//! 应用主题与颜色令牌。
//!
//! 文件前半部分提供画布、表面、边框、正文、状态色等统一颜色函数；`apply`
//! 将这些令牌写入当前 egui `Ui` 的视觉样式，使设置页和词库页面保持一致。

use eframe::egui;

pub(crate) fn canvas() -> egui::Color32 {
    egui::Color32::from_rgb(232, 230, 225)
}

pub(crate) fn surface() -> egui::Color32 {
    egui::Color32::from_rgb(247, 246, 243)
}

pub(crate) fn control_surface() -> egui::Color32 {
    egui::Color32::from_rgb(236, 234, 229)
}

pub(crate) fn border() -> egui::Color32 {
    egui::Color32::from_rgb(191, 188, 181)
}

pub(crate) fn primary_text() -> egui::Color32 {
    egui::Color32::from_rgb(38, 40, 42)
}

pub(crate) fn muted_text() -> egui::Color32 {
    egui::Color32::from_rgb(91, 91, 88)
}

pub(crate) fn accent_fill() -> egui::Color32 {
    egui::Color32::from_rgb(180, 210, 227)
}

pub(crate) fn accent_text() -> egui::Color32 {
    egui::Color32::from_rgb(36, 95, 130)
}

pub(crate) fn pale_red() -> egui::Color32 {
    egui::Color32::from_rgb(253, 235, 236)
}

pub(crate) fn red_text() -> egui::Color32 {
    egui::Color32::from_rgb(159, 47, 45)
}

pub(crate) fn pale_green() -> egui::Color32 {
    egui::Color32::from_rgb(237, 243, 236)
}

pub(crate) fn green_text() -> egui::Color32 {
    egui::Color32::from_rgb(52, 101, 56)
}

pub(crate) fn pale_yellow() -> egui::Color32 {
    egui::Color32::from_rgb(251, 243, 219)
}

pub(crate) fn yellow_text() -> egui::Color32 {
    egui::Color32::from_rgb(149, 100, 0)
}

pub(crate) fn apply(ui: &mut egui::Ui) {
    ui.spacing_mut().icon_width = 18.0;
    ui.spacing_mut().icon_width_inner = 11.0;
    let visuals = &mut ui.style_mut().visuals;
    visuals.override_text_color = Some(primary_text());
    visuals.selection.bg_fill = accent_fill();
    visuals.selection.stroke = egui::Stroke::new(1.5, accent_text());
    visuals.widgets.noninteractive.fg_stroke = egui::Stroke::new(1.2, primary_text());
    visuals.widgets.inactive.bg_fill = control_surface();
    visuals.widgets.inactive.weak_bg_fill = control_surface();
    visuals.widgets.inactive.bg_stroke = egui::Stroke::new(1.2, border());
    visuals.widgets.inactive.fg_stroke = egui::Stroke::new(1.5, primary_text());
    visuals.widgets.hovered.bg_fill = egui::Color32::from_rgb(225, 222, 216);
    visuals.widgets.hovered.weak_bg_fill = egui::Color32::from_rgb(225, 222, 216);
    visuals.widgets.hovered.bg_stroke =
        egui::Stroke::new(1.4, egui::Color32::from_rgb(128, 125, 118));
    visuals.widgets.hovered.fg_stroke = egui::Stroke::new(1.7, primary_text());
    visuals.widgets.active.bg_fill = accent_fill();
    visuals.widgets.active.weak_bg_fill = accent_fill();
    visuals.widgets.active.bg_stroke = egui::Stroke::new(1.5, accent_text());
    visuals.widgets.active.fg_stroke = egui::Stroke::new(1.8, egui::Color32::from_rgb(24, 68, 94));
}
