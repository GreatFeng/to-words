//! OCR 屏幕快照和可反复调整的框选控件。
//!
//! 首次拖拽创建选区；松开后通知应用预览识别，拖动内部可移动，
//! 拖动右下角手柄可缩放；只有点击确认或取消才结束框选。

use std::sync::Arc;

use eframe::egui::{self, Color32, Pos2, Rect, TextureHandle, Vec2};

use crate::components::ocr_translation::paint_translation;
use crate::platform::ocr::{self, CaptureRegion, CapturedScreen};

pub(crate) enum SelectionAction {
    Changed(CaptureRegion),
    Confirm(CaptureRegion),
    Cancel,
}

enum DragMode {
    Create(Pos2),
    Move { offset: Vec2 },
    Resize(Pos2),
}

pub(crate) struct OcrSelection {
    pub(crate) screen: Arc<CapturedScreen>,
    texture: TextureHandle,
    selection: Option<Rect>,
    drag: Option<DragMode>,
}

impl OcrSelection {
    pub(crate) fn new(ctx: &egui::Context, screen: CapturedScreen) -> Self {
        let image = egui::ColorImage::from_rgb(
            [
                screen.monitor.width as usize,
                screen.monitor.height as usize,
            ],
            &screen.rgb,
        );
        let texture = ctx.load_texture("ocr-screen-snapshot", image, egui::TextureOptions::LINEAR);
        Self {
            screen: Arc::new(screen),
            texture,
            selection: None,
            drag: None,
        }
    }

    pub(crate) fn show(
        &mut self,
        ui: &mut egui::Ui,
        preview: Option<(&str, bool)>,
    ) -> Option<SelectionAction> {
        if ui.input(|input| input.key_pressed(egui::Key::Escape)) {
            return Some(SelectionAction::Cancel);
        }

        let area = ui.max_rect();
        let uv = Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0));
        ui.painter()
            .image(self.texture.id(), area, uv, Color32::WHITE);
        ui.painter()
            .rect_filled(area, 0.0, Color32::from_black_alpha(105));

        let mut button_area = None;
        if let Some(rect) = self.selection {
            // 从原始截图重新绘制选区，使选中部分保持明亮。
            ui.painter()
                .with_clip_rect(rect)
                .image(self.texture.id(), area, uv, Color32::WHITE);
            ui.painter().rect_stroke(
                rect,
                3.0,
                egui::Stroke::new(2.0, Color32::from_rgb(105, 194, 255)),
                egui::StrokeKind::Inside,
            );
            // 右下角保留可拖动的热区，但不绘制方块，避免看起来像复选框。
            for inset in [5.0, 9.0, 13.0] {
                ui.painter().line_segment(
                    [
                        rect.right_bottom() - Vec2::new(inset, 1.0),
                        rect.right_bottom() - Vec2::new(1.0, inset),
                    ],
                    egui::Stroke::new(1.5, Color32::from_rgb(105, 194, 255)),
                );
            }

            if rect.width() >= 8.0 && rect.height() >= 8.0 {
                let buttons = button_rect(rect, area);
                button_area = Some(buttons);
                ui.painter()
                    .rect_filled(buttons.expand(3.0), 8.0, Color32::from_rgb(34, 40, 49));
                let confirm = Rect::from_min_size(buttons.min, Vec2::new(30.0, 28.0));
                let cancel =
                    Rect::from_min_size(buttons.min + Vec2::new(38.0, 0.0), Vec2::new(30.0, 28.0));
                if ui
                    .put(
                        confirm,
                        egui::Button::new(
                            egui::RichText::new("√").size(18.0).color(Color32::WHITE),
                        )
                        .fill(Color32::from_rgb(38, 113, 91)),
                    )
                    .on_hover_text("复制识别原文并关闭")
                    .clicked()
                {
                    let region = region_from_rect(self.screen.monitor, rect, area);
                    if let Some(region) = region {
                        return Some(SelectionAction::Confirm(region));
                    }
                }
                if ui
                    .put(
                        cancel,
                        egui::Button::new(
                            egui::RichText::new("×").size(18.0).color(Color32::WHITE),
                        )
                        .fill(Color32::from_rgb(92, 51, 52)),
                    )
                    .on_hover_text("取消框选")
                    .clicked()
                {
                    return Some(SelectionAction::Cancel);
                }
            }
        }

        if self.drag.is_none()
            && let (Some(rect), Some(buttons), Some((text, is_error))) =
                (self.selection, button_area, preview)
        {
            paint_translation(ui, rect, buttons, text, is_error);
        }

        let (pressed, down, released, pointer) = ui.input(|input| {
            (
                input.pointer.primary_pressed(),
                input.pointer.primary_down(),
                input.pointer.primary_released(),
                input.pointer.interact_pos(),
            )
        });
        if pressed
            && let Some(point) = pointer.filter(|point| area.contains(*point))
            && !button_area.is_some_and(|buttons| buttons.expand(5.0).contains(point))
        {
            self.drag = Some(match self.selection {
                Some(rect) if resize_handle(rect).expand(8.0).contains(point) => {
                    DragMode::Resize(rect.min)
                }
                Some(rect) if rect.contains(point) => DragMode::Move {
                    offset: point - rect.min,
                },
                _ => {
                    self.selection = None;
                    DragMode::Create(point)
                }
            });
        }
        if down && let (Some(mode), Some(point)) = (&self.drag, pointer) {
            let point = point.clamp(area.min, area.max);
            self.selection = Some(match mode {
                DragMode::Create(start) => Rect::from_two_pos(*start, point),
                DragMode::Move { offset } => {
                    let rect = self.selection.expect("moving an existing selection");
                    moved_rect(rect, point - *offset, area)
                }
                DragMode::Resize(start) => {
                    Rect::from_min_max(*start, point.max(*start + Vec2::splat(8.0)))
                }
            });
        }
        if released
            && self.drag.take().is_some()
            && let Some(rect) = self.selection
            && let Some(region) = region_from_rect(self.screen.monitor, rect, area)
        {
            return Some(SelectionAction::Changed(region));
        }

        let hint = Rect::from_min_size(area.min + Vec2::new(22.0, 22.0), Vec2::new(410.0, 38.0));
        ui.painter()
            .rect_filled(hint, 7.0, Color32::from_black_alpha(195));
        ui.painter().text(
            hint.center(),
            egui::Align2::CENTER_CENTER,
            "松开自动识别 · 可移动或缩放选区 · √复制并关闭 · ×取消",
            egui::FontId::proportional(14.0),
            Color32::WHITE,
        );
        None
    }
}

fn region_from_rect(monitor: ocr::MonitorBounds, rect: Rect, area: Rect) -> Option<CaptureRegion> {
    ocr::selected_region(
        monitor,
        [area.width(), area.height()],
        [rect.left() - area.left(), rect.top() - area.top()],
        [rect.right() - area.left(), rect.bottom() - area.top()],
    )
}

fn resize_handle(rect: Rect) -> Rect {
    Rect::from_center_size(rect.right_bottom(), Vec2::splat(14.0))
}

fn moved_rect(rect: Rect, requested_min: Pos2, area: Rect) -> Rect {
    let max_min = area.max - rect.size();
    Rect::from_min_size(requested_min.clamp(area.min, max_min), rect.size())
}

fn button_rect(selection: Rect, area: Rect) -> Rect {
    let size = Vec2::new(68.0, 28.0);
    // 按钮贴在选区右侧的下沿，不占用译文所在的下方区域。
    let outside_right = selection.right() + 10.0;
    let x = if outside_right + size.x <= area.right() - 6.0 {
        outside_right
    } else {
        (selection.right() - size.x).clamp(area.left(), (area.right() - size.x).max(area.left()))
    };
    let y =
        (selection.bottom() - size.y).clamp(area.top(), (area.bottom() - size.y).max(area.top()));
    Rect::from_min_size(Pos2::new(x, y), size)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn moving_selection_preserves_size_and_stays_on_screen() {
        let area = Rect::from_min_size(Pos2::ZERO, Vec2::new(100.0, 100.0));
        let rect = Rect::from_min_size(Pos2::new(10.0, 10.0), Vec2::new(30.0, 20.0));
        let moved = moved_rect(rect, Pos2::new(90.0, 95.0), area);
        assert_eq!(moved.min, Pos2::new(70.0, 80.0));
        assert_eq!(moved.size(), rect.size());
    }

    #[test]
    fn buttons_sit_at_selection_lower_right_without_occupying_translation_row() {
        let area = Rect::from_min_size(Pos2::ZERO, Vec2::new(1920.0, 1080.0));
        let selection = Rect::from_min_size(Pos2::new(100.0, 100.0), Vec2::new(200.0, 80.0));
        let buttons = button_rect(selection, area);
        assert!(buttons.left() > selection.right());
        assert_eq!(buttons.bottom(), selection.bottom());
        assert_eq!(buttons.size(), Vec2::new(68.0, 28.0));
    }

    #[test]
    fn buttons_remain_visible_at_right_edge() {
        let area = Rect::from_min_size(Pos2::ZERO, Vec2::new(400.0, 300.0));
        let selection = Rect::from_min_size(Pos2::new(320.0, 150.0), Vec2::new(80.0, 60.0));
        let buttons = button_rect(selection, area);
        assert!(buttons.right() <= area.right());
        assert_eq!(buttons.bottom(), selection.bottom());
    }
}
