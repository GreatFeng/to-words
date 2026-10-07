//! 屏幕底部的语音识别结果悬浮层。只绘制纯文字，不绘制文本框或描边，
//! 并允许鼠标穿透，避免干扰当前正在使用的程序。

use eframe::egui;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

pub(crate) struct VoiceOverlay {
    monitor_work_area: egui::Rect,
    pub(crate) original: String,
    pub(crate) result: String,
    pub(crate) status: String,
    pub(crate) original_utterance_id: u64,
    drawn_original_id: Arc<AtomicU64>,
    border_suppressed: Arc<AtomicBool>,
    hide_at: Option<Instant>,
}

impl VoiceOverlay {
    pub(crate) fn new(monitor_work_area: egui::Rect) -> Self {
        Self {
            monitor_work_area,
            original: String::new(),
            result: String::new(),
            status: "请说话；停顿约 0.6 秒后自动识别".to_string(),
            original_utterance_id: 0,
            drawn_original_id: Arc::new(AtomicU64::new(0)),
            border_suppressed: Arc::new(AtomicBool::new(false)),
            hide_at: None,
        }
    }

    pub(crate) fn finish_after(&mut self, delay: Duration) {
        self.hide_at = Some(Instant::now() + delay);
    }

    pub(crate) fn keep_visible(&mut self) {
        self.hide_at = None;
    }

    pub(crate) fn begin_utterance(&mut self) {
        self.original.clear();
        self.result.clear();
        self.original_utterance_id = 0;
        self.status = "正在听下一句…".to_string();
        self.keep_visible();
    }

    pub(crate) fn expired(&self) -> bool {
        self.hide_at
            .is_some_and(|deadline| Instant::now() >= deadline)
    }

    pub(crate) fn drawn_original_id(&self) -> u64 {
        self.drawn_original_id.load(Ordering::Acquire)
    }

    pub(crate) fn show(&self, context: &egui::Context) {
        let width = self.monitor_work_area.width().clamp(1.0, 720.0);
        let content_width = (width - 32.0).max(100.0);
        let estimated_lines = |text: &str, font_size: f32| {
            ((text.chars().count() as f32 * font_size * 0.8) / content_width)
                .ceil()
                .max(1.0)
        };
        let show_status = should_show_status(&self.result, &self.status);
        let height = (estimated_lines(&self.original, 20.0) * 25.0
            + if self.result.is_empty() {
                0.0
            } else {
                estimated_lines(&self.result, 18.0) * 23.0 + 7.0
            }
            + if show_status {
                estimated_lines(&self.status, 12.0) * 18.0 + 7.0
            } else {
                0.0
            }
            + 16.0)
            .clamp(48.0, 320.0);
        let position = overlay_position(self.monitor_work_area, egui::vec2(width, height));
        let builder = egui::ViewportBuilder::default()
            .with_title("to_words 语音识别")
            .with_position(position)
            .with_inner_size([width, height])
            .with_decorations(false)
            .with_resizable(false)
            .with_transparent(true)
            .with_active(false)
            .with_taskbar(false)
            .with_mouse_passthrough(true)
            .with_window_level(egui::WindowLevel::AlwaysOnTop);
        let original = if self.original.is_empty() {
            "等待语音…".to_string()
        } else {
            self.original.clone()
        };
        let result = self.result.clone();
        let status = self.status.clone();
        let original_utterance_id = self.original_utterance_id;
        let drawn_original_id = Arc::clone(&self.drawn_original_id);
        let border_suppressed = Arc::clone(&self.border_suppressed);
        let viewport_id = egui::ViewportId::from_hash_of("to_words_voice_overlay");
        context.show_viewport_deferred(viewport_id, builder, move |ui, _| {
            if !border_suppressed.load(Ordering::Acquire)
                && crate::platform::voice_overlay_window::suppress_border()
            {
                border_suppressed.store(true, Ordering::Release);
            }
            let painter = ui.painter();
            let max_width = (ui.available_width() - 32.0).max(100.0);
            let mut y = 8.0;
            paint_text(painter, &original, 20.0, max_width, &mut y);
            if original_utterance_id != 0 {
                drawn_original_id.fetch_max(original_utterance_id, Ordering::Release);
            }
            if !result.is_empty() {
                y += 7.0;
                paint_text(painter, &result, 18.0, max_width, &mut y);
            }
            if show_status {
                y += 7.0;
                paint_text(painter, &status, 12.0, max_width, &mut y);
            }
        });
        // 该悬浮层是独立的延迟视口：主窗口重绘不会自动刷新它。
        // 识别文字先于 AI 译文到达时，必须主动唤醒子视口才能立即显示原文。
        context.request_repaint_of(viewport_id);
    }
}

fn should_show_status(result: &str, status: &str) -> bool {
    result.is_empty() && !status.is_empty()
}

fn overlay_position(work_area: egui::Rect, size: egui::Vec2) -> egui::Pos2 {
    let x = work_area.center().x - size.x / 2.0;
    let y = work_area.bottom() - size.y - 24.0;
    egui::pos2(
        x.clamp(
            work_area.left(),
            (work_area.right() - size.x).max(work_area.left()),
        ),
        y.clamp(
            work_area.top(),
            (work_area.bottom() - size.y).max(work_area.top()),
        ),
    )
}

fn paint_text(painter: &egui::Painter, text: &str, font_size: f32, max_width: f32, y: &mut f32) {
    let galley = painter.layout(
        text.to_string(),
        egui::FontId::proportional(font_size),
        egui::Color32::PLACEHOLDER,
        max_width,
    );
    let pos = egui::pos2(16.0, *y);
    let height = galley.size().y;
    painter.galley(pos, galley, egui::Color32::WHITE);
    *y += height;
}

#[cfg(test)]
mod tests {
    use super::{VoiceOverlay, overlay_position, should_show_status};
    use eframe::egui;

    #[test]
    fn recognized_original_is_drawn_before_any_result() {
        let context = egui::Context::default();
        context.set_embed_viewports(true);
        let mut overlay = VoiceOverlay::new(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(1280.0, 720.0),
        ));
        overlay.original = "识别原文".to_string();
        overlay.original_utterance_id = 1;
        assert!(overlay.result.is_empty());
        let mut output = context.run_ui(egui::RawInput::default(), |ui| overlay.show(ui.ctx()));
        output.textures_delta.clear();
        assert_eq!(overlay.drawn_original_id(), 1);
    }

    #[test]
    fn placement_stays_at_monitor_bottom() {
        let area = egui::Rect::from_min_size(egui::pos2(100.0, 200.0), egui::vec2(600.0, 400.0));
        let size = egui::vec2(300.0, 120.0);
        assert_eq!(overlay_position(area, size), egui::pos2(250.0, 456.0));
    }

    #[test]
    fn completed_result_hides_status_line() {
        assert!(should_show_status("", "正在识别"));
        assert!(!should_show_status("译文", "已发送到当前窗口"));
    }

    #[test]
    fn next_utterance_clears_previous_result_and_cancels_hide() {
        let mut overlay = VoiceOverlay::new(egui::Rect::EVERYTHING);
        overlay.original = "上一句".to_string();
        overlay.result = "上一句结果".to_string();
        overlay.original_utterance_id = 1;
        overlay.finish_after(std::time::Duration::ZERO);
        assert!(overlay.expired());
        overlay.begin_utterance();
        assert!(overlay.original.is_empty());
        assert!(overlay.result.is_empty());
        assert_eq!(overlay.original_utterance_id, 0);
        assert!(!overlay.expired());
    }
}
