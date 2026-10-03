//! 在 OCR 截图选区旁绘制无背景的译文，不创建额外窗口。

use eframe::egui::{self, Color32, Pos2, Rect, Vec2};

pub(crate) fn paint_translation(
    ui: &egui::Ui,
    selection: Rect,
    controls: Rect,
    text: &str,
    is_error: bool,
) {
    let area = ui.max_rect();
    let wrap_width = selection
        .width()
        .clamp(280.0, 620.0)
        .min((area.width() - 32.0).max(100.0));
    let color = if is_error {
        Color32::from_rgb(255, 180, 180)
    } else {
        Color32::WHITE
    };
    let font = egui::FontId::proportional(17.0);
    let galley = ui
        .painter()
        .layout(text.to_owned(), font.clone(), color, wrap_width);
    let position = translation_position(selection, controls, area, galley.size());
    let shadow = ui.painter().layout(
        text.to_owned(),
        font,
        Color32::from_black_alpha(215),
        wrap_width,
    );
    ui.painter()
        .galley(position + Vec2::new(1.0, 1.0), shadow, Color32::BLACK);
    ui.painter().galley(position, galley, color);
}

fn translation_position(selection: Rect, controls: Rect, area: Rect, size: Vec2) -> Pos2 {
    let left = area.left() + 16.0;
    let right = (area.right() - size.x - 16.0).max(left);
    let x = selection.left().clamp(left, right);
    let below = selection.bottom() + 10.0;
    let above = selection.top().min(controls.top()) - size.y - 10.0;
    let y = if below + size.y <= area.bottom() - 12.0 {
        below
    } else if above >= area.top() + 12.0 {
        above
    } else {
        (area.bottom() - size.y - 12.0).max(area.top() + 12.0)
    };
    Pos2::new(x, y)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn translation_starts_directly_below_selection_and_buttons() {
        let area = Rect::from_min_size(Pos2::ZERO, Vec2::new(1920.0, 1080.0));
        let selection = Rect::from_min_size(Pos2::new(100.0, 100.0), Vec2::new(400.0, 50.0));
        let controls = Rect::from_min_size(Pos2::new(510.0, 122.0), Vec2::new(68.0, 28.0));
        let position = translation_position(selection, controls, area, Vec2::new(300.0, 80.0));
        assert_eq!(position.y, selection.bottom() + 10.0);
        assert!(position.y > controls.bottom());
    }

    #[test]
    fn translation_moves_above_selection_at_screen_bottom() {
        let area = Rect::from_min_size(Pos2::ZERO, Vec2::new(1920.0, 1080.0));
        let selection = Rect::from_min_size(Pos2::new(100.0, 900.0), Vec2::new(400.0, 80.0));
        let controls = Rect::from_min_size(Pos2::new(420.0, 856.0), Vec2::new(82.0, 32.0));
        let position = translation_position(selection, controls, area, Vec2::new(360.0, 100.0));
        assert!(position.y + 100.0 < controls.top());
    }
}
