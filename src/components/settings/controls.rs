//! 设置页共用交互控件：弹性数值滑杆、开关和自动收起的说明气泡。

use crate::i18n::tr;
use crate::ui_theme::{
    self, accent_fill, accent_text, border, control_surface, muted_text, primary_text,
};
use eframe::egui;
use std::ops::RangeInclusive;
use std::time::Duration;

const BUBBLE_SECONDS: f64 = 4.0;

pub(super) fn setting_switch(ui: &mut egui::Ui, value: &mut bool, text: &str) -> egui::Response {
    ui.horizontal(|ui| {
        let (rect, mut response) =
            ui.allocate_exact_size(egui::vec2(42.0, 25.0), egui::Sense::click());
        let label = ui
            .add(
                egui::Label::new(egui::RichText::new(tr(text)).size(13.0).color(
                    if ui.is_enabled() {
                        primary_text()
                    } else {
                        muted_text()
                    },
                ))
                .sense(egui::Sense::click()),
            )
            .on_hover_cursor(egui::CursorIcon::PointingHand);
        if response.clicked() || label.clicked() {
            *value = !*value;
            response.mark_changed();
        }

        let t = ui.ctx().animate_bool_with_time(response.id, *value, 0.2);
        let enabled = ui.is_enabled();
        let fill = if enabled {
            switch_color(t)
        } else {
            blend_color(switch_color(t), control_surface(), 0.45)
        };
        let white = if enabled {
            egui::Color32::WHITE
        } else {
            blend_color(egui::Color32::WHITE, control_surface(), 0.45)
        };
        let painter = ui.painter();
        painter.rect_filled(rect, 12.5, fill);
        let thumb_center = egui::pos2(rect.left() + 12.5 + t * 17.0, rect.center().y);
        if enabled {
            painter.circle_filled(
                thumb_center + egui::vec2(3.5, 1.5),
                9.3,
                egui::Color32::from_rgba_unmultiplied(8, 8, 8, 30),
            );
        }
        painter.circle_filled(thumb_center, 8.5, white);
        painter.circle_filled(thumb_center, 4.2, blend_color(fill, white, t));
        response.on_hover_cursor(egui::CursorIcon::PointingHand)
    })
    .inner
}

fn switch_color(t: f32) -> egui::Color32 {
    blend_color(
        egui::Color32::from_rgb(82, 82, 82),
        egui::Color32::from_rgb(148, 118, 255),
        t,
    )
}

fn blend_color(from: egui::Color32, to: egui::Color32, t: f32) -> egui::Color32 {
    let t = t.clamp(0.0, 1.0);
    let channel =
        |start: u8, end: u8| (start as f32 + (end as f32 - start as f32) * t).round() as u8;
    egui::Color32::from_rgb(
        channel(from.r(), to.r()),
        channel(from.g(), to.g()),
        channel(from.b(), to.b()),
    )
}

pub(super) fn elastic_slider(
    ui: &mut egui::Ui,
    value: &mut f32,
    range: RangeInclusive<f32>,
    step: f32,
    decimals: usize,
) -> egui::Response {
    let width = ui.available_width().clamp(150.0, 215.0);
    let (id, rect) = ui.allocate_space(egui::vec2(width, 36.0));
    let track_left = rect.left() + 12.0;
    let track_right = rect.right() - 64.0;
    let hit_rect = egui::Rect::from_min_max(
        egui::pos2(track_left - 7.0, rect.center().y - 13.0),
        egui::pos2(track_right + 7.0, rect.center().y + 13.0),
    );
    let mut response = ui.interact(hit_rect, id, egui::Sense::click_and_drag());
    let down = response.is_pointer_button_down_on();
    let pressed = ui
        .ctx()
        .animate_bool_with_time(response.id.with("pressed_edge"), down, 0.12);
    let hover = ui.ctx().animate_bool_with_time(
        response.id.with("hover_halo"),
        response.hovered() || down,
        0.3,
    );
    let half_height = 2.0 + pressed * 1.3;
    let track = egui::Rect::from_min_max(
        egui::pos2(track_left, rect.center().y - half_height),
        egui::pos2(track_right, rect.center().y + half_height),
    );
    let min = *range.start();
    let max = *range.end();
    let pointer_x = response.interact_pointer_pos().map(|pos| pos.x);
    if down && let Some(x) = pointer_x {
        let fraction = ((x - track.left()) / track.width()).clamp(0.0, 1.0);
        let next = quantize(min + fraction * (max - min), &range, step);
        if (*value - next).abs() > f32::EPSILON {
            *value = next;
            response.mark_changed();
        }
        response.request_focus();
    }
    if response.has_focus() {
        let direction = ui.input(|input| {
            (input.key_pressed(egui::Key::ArrowRight) as i32)
                - (input.key_pressed(egui::Key::ArrowLeft) as i32)
        });
        if direction != 0 {
            let next = quantize(*value + direction as f32 * step, &range, step);
            if (*value - next).abs() > f32::EPSILON {
                *value = next;
                response.mark_changed();
            }
        }
    }

    let fraction = ((*value - min) / (max - min)).clamp(0.0, 1.0);
    let thumb_x = track.left() + track.width() * fraction;
    let overshoot = if down {
        pointer_x
            .map(|x| {
                let outside = if x < track.left() {
                    x - track.left()
                } else if x > track.right() {
                    x - track.right()
                } else {
                    0.0
                };
                (outside / 16.0).tanh() * 12.0
            })
            .unwrap_or(0.0)
    } else {
        0.0
    };
    let stretch = ui.ctx().animate_value_with_time(
        response.id.with("elastic_edge"),
        overshoot,
        if down { 0.0 } else { 0.24 },
    );
    let painter = ui.painter();
    painter.rect_filled(
        rect.translate(egui::vec2(0.0, 1.0)),
        6.0,
        egui::Color32::from_rgba_unmultiplied(31, 30, 36, 20),
    );
    painter.rect_filled(rect, 6.0, ui_theme::surface());
    painter.rect_stroke(
        rect,
        6.0,
        egui::Stroke::new(1.0, egui::Color32::from_rgb(198, 202, 206)),
        egui::StrokeKind::Inside,
    );
    let track_color = blend_color(
        ui_theme::surface(),
        egui::Color32::from_rgb(213, 219, 225),
        0.7 + 0.3 * hover,
    );
    let active_color = blend_color(
        ui_theme::surface(),
        egui::Color32::from_rgb(24, 24, 24),
        0.7 + 0.3 * hover,
    );
    painter.rect_filled(track, half_height, track_color);
    if thumb_x > track.left() {
        painter.rect_filled(
            egui::Rect::from_min_max(track.min, egui::pos2(thumb_x, track.bottom())),
            half_height,
            active_color,
        );
    }
    if hover > 0.0 {
        painter.circle_filled(
            egui::pos2(thumb_x + stretch * 0.5, track.center().y),
            7.5 + 8.0 * hover,
            egui::Color32::from_rgba_unmultiplied(0, 0, 0, (40.0 * hover) as u8),
        );
    }
    let thumb = egui::Rect::from_min_max(
        egui::pos2(thumb_x.min(thumb_x + stretch) - 7.5, track.center().y - 7.5),
        egui::pos2(thumb_x.max(thumb_x + stretch) + 7.5, track.center().y + 7.5),
    );
    painter.rect_filled(thumb, 7.5, active_color);
    painter.text(
        egui::pos2(rect.right() - 10.0, rect.center().y),
        egui::Align2::RIGHT_CENTER,
        format!("{value:.decimals$}"),
        egui::FontId::proportional(12.5),
        primary_text(),
    );
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

pub(super) fn quantize(value: f32, range: &RangeInclusive<f32>, step: f32) -> f32 {
    let min = *range.start();
    let max = *range.end();
    if !value.is_finite() || !step.is_finite() || step <= 0.0 || max <= min {
        return min;
    }
    (min + ((value - min) / step).round() * step).clamp(min, max)
}

pub(super) fn help_bubble(ui: &mut egui::Ui, explanation: &str) {
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
    let id = response.id.with("bubble_expiry");
    let now = ui.input(|input| input.time);
    if response.clicked() {
        ui.ctx()
            .data_mut(|data| data.insert_temp(id, now + BUBBLE_SECONDS));
    }
    let deadline = ui
        .ctx()
        .data(|data| data.get_temp::<f64>(id))
        .unwrap_or(0.0);
    if now >= deadline {
        return;
    }
    show_bubble(ui, &response, explanation, deadline);
}

pub(super) fn hover_bubble(
    ui: &mut egui::Ui,
    response: egui::Response,
    explanation: &str,
) -> egui::Response {
    let id = response.id.with("hover_bubble_expiry");
    let now = ui.input(|input| input.time);
    let mut deadline = ui.ctx().data(|data| data.get_temp::<f64>(id));
    if !response.hovered() {
        ui.ctx().data_mut(|data| data.remove::<f64>(id));
    } else {
        if deadline.is_none() {
            let expires_at = now + BUBBLE_SECONDS;
            ui.ctx().data_mut(|data| data.insert_temp(id, expires_at));
            deadline = Some(expires_at);
        }
        if let Some(expires_at) = deadline.filter(|expires_at| now < *expires_at) {
            show_bubble(ui, &response, explanation, expires_at);
        }
    }
    response
}

fn show_bubble(ui: &mut egui::Ui, response: &egui::Response, explanation: &str, deadline: f64) {
    let rect = response.rect;
    let center = rect.center();
    let now = ui.input(|input| input.time);
    ui.ctx()
        .request_repaint_after(Duration::from_secs_f64(deadline - now));

    let above = ui.ctx().content_rect().bottom() - rect.bottom() < 165.0;
    let popup = egui::Popup::from_response(response)
        .open(true)
        .align(if above {
            egui::RectAlign::TOP_START
        } else {
            egui::RectAlign::BOTTOM_START
        })
        .align_alternatives(&[])
        .gap(4.0)
        .width(360.0)
        .frame(egui::Frame::NONE);
    popup.show(|ui| {
        ui.set_width(360.0);
        ui.spacing_mut().item_spacing.y = 0.0;
        if !above {
            bubble_arrow(ui, center.x, true);
        }
        ui_theme::tooltip_frame().show(ui, |ui| {
            ui.set_width(334.0);
            ui.add(
                egui::Label::new(
                    egui::RichText::new(tr(explanation))
                        .size(13.0)
                        .color(primary_text()),
                )
                .wrap(),
            );
        });
        if above {
            bubble_arrow(ui, center.x, false);
        }
    });
}

fn bubble_arrow(ui: &mut egui::Ui, anchor_x: f32, points_up: bool) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(360.0, 7.0), egui::Sense::hover());
    let x = anchor_x.clamp(rect.left() + 12.0, rect.right() - 12.0);
    let (tip_y, base_y) = if points_up {
        (rect.top(), rect.bottom())
    } else {
        (rect.bottom(), rect.top())
    };
    let points = vec![
        egui::pos2(x, tip_y),
        egui::pos2(x - 7.0, base_y),
        egui::pos2(x + 7.0, base_y),
    ];
    ui.painter().add(egui::Shape::convex_polygon(
        points,
        control_surface(),
        egui::Stroke::new(1.0, border()),
    ));
}

#[cfg(test)]
mod tests {
    use super::{quantize, switch_color};
    use eframe::egui;

    #[test]
    fn switch_uses_the_requested_off_and_on_colors() {
        assert_eq!(switch_color(0.0), egui::Color32::from_rgb(82, 82, 82));
        assert_eq!(switch_color(1.0), egui::Color32::from_rgb(148, 118, 255));
    }

    #[test]
    fn slider_snaps_and_clamps_without_persisting_elastic_overshoot() {
        assert!((quantize(0.64, &(0.2..=3.0), 0.1) - 0.6).abs() < 0.0001);
        assert!((quantize(0.66, &(0.2..=3.0), 0.1) - 0.7).abs() < 0.0001);
        assert_eq!(quantize(-5.0, &(0.2..=3.0), 0.1), 0.2);
        assert_eq!(quantize(5.0, &(0.2..=3.0), 0.1), 3.0);
    }
}
