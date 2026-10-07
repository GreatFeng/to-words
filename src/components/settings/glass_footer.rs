//! 设置页底部的局部毛玻璃绘制。
//!
//! 使用 glow 回调读取已绘制的滚动内容，再在页脚范围内作小半径模糊、轻微
//! 边缘折射和随滚动移动的高光。着色器不可用时，egui 的半透明底色仍可用。

use eframe::egui;
use eframe::egui_glow::CallbackFn;
use eframe::glow::{self, HasContext};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

const VERTEX_SHADER: &str = r#"#version 330 core
out vec2 uv;
void main() {
    vec2 points[3] = vec2[3](vec2(-1.0, -1.0), vec2(3.0, -1.0), vec2(-1.0, 3.0));
    vec2 point = points[gl_VertexID];
    uv = point * 0.5 + 0.5;
    gl_Position = vec4(point, 0.0, 1.0);
}"#;

const FRAGMENT_SHADER: &str = r#"#version 330 core
in vec2 uv;
out vec4 color;
uniform sampler2D backdrop;
uniform vec2 size_px;
uniform float scroll_px;

vec3 sample_backdrop(vec2 at) {
    return texture(backdrop, clamp(at, vec2(0.003), vec2(0.997))).rgb;
}

void main() {
    vec2 point = (uv - 0.5) * size_px;
    float radius = 17.0;
    vec2 extent = size_px * 0.5 - vec2(radius);
    vec2 delta = abs(point) - extent;
    float distance_to_edge = length(max(delta, 0.0)) + min(max(delta.x, delta.y), 0.0) - radius;
    float coverage = 1.0 - smoothstep(-1.0, 1.0, distance_to_edge);

    // Refract only near the rim; keep text behind the center legible.
    float rim = exp(-max(-distance_to_edge, 0.0) / 12.0);
    vec2 normal = point / max(length(point), 1.0);
    vec2 displaced = uv + normal * rim * 1.6 / size_px;
    vec2 dx = vec2(2.4 / size_px.x, 0.0);
    vec2 dy = vec2(0.0, 2.4 / size_px.y);
    vec3 blurred = sample_backdrop(displaced) * 0.16;
    blurred += (sample_backdrop(displaced + dx) + sample_backdrop(displaced - dx)
             + sample_backdrop(displaced + dy) + sample_backdrop(displaced - dy)) * 0.11;
    blurred += (sample_backdrop(displaced + 2.0 * dx) + sample_backdrop(displaced - 2.0 * dx)
             + sample_backdrop(displaced + 2.0 * dy) + sample_backdrop(displaced - 2.0 * dy)) * 0.065;
    blurred += (sample_backdrop(displaced + 3.0 * dx) + sample_backdrop(displaced - 3.0 * dx)
             + sample_backdrop(displaced + 3.0 * dy) + sample_backdrop(displaced - 3.0 * dy)) * 0.035;

    float luminance = dot(blurred, vec3(0.2126, 0.7152, 0.0722));
    float light = smoothstep(0.32, 0.64, luminance);
    vec3 tint = mix(vec3(0.12, 0.16, 0.19), vec3(0.96, 0.97, 0.97), light);
    vec3 glass = mix(blurred, tint, 0.57);

    // The highlight shifts with scroll position, but never flashes or obscures controls.
    float sweep = 0.5 + 0.5 * sin((uv.x * size_px.x - scroll_px * 0.42) / 105.0);
    float upper_rim = exp(-abs(distance_to_edge) / 1.8) * smoothstep(0.65, 0.95, uv.y);
    glass += vec3(0.045 + 0.11 * sweep) * upper_rim;
    float lower_rim = exp(-abs(distance_to_edge) / 1.8) * (1.0 - smoothstep(0.10, 0.35, uv.y));
    glass -= vec3(0.025) * lower_rim;
    color = vec4(clamp(glass, 0.0, 1.0) * coverage, coverage);
}"#;

pub(super) struct GlassFooter {
    resources: Arc<Mutex<Option<GlResources>>>,
    brightness: Arc<AtomicU32>,
    warned: Arc<AtomicBool>,
}

impl Default for GlassFooter {
    fn default() -> Self {
        Self {
            resources: Arc::new(Mutex::new(None)),
            brightness: Arc::new(AtomicU32::new(255)),
            warned: Arc::new(AtomicBool::new(false)),
        }
    }
}

impl GlassFooter {
    pub(super) fn background_is_dark(&self) -> bool {
        self.brightness.load(Ordering::Relaxed) < 128
    }

    pub(super) fn paint(&self, ui: &egui::Ui, rect: egui::Rect, scroll_points: f32) {
        // Only draw the fallback after shader initialization has failed. Painting it
        // before the callback would contaminate the backdrop that the shader samples.
        let dark = self.background_is_dark();
        let fallback = if dark {
            egui::Color32::from_rgba_unmultiplied(33, 39, 47, 224)
        } else {
            egui::Color32::from_rgba_unmultiplied(240, 243, 245, 226)
        };
        if self.warned.load(Ordering::Relaxed) {
            ui.painter().rect_filled(rect, 17.0, fallback);
        }

        let resources = Arc::clone(&self.resources);
        let brightness = Arc::clone(&self.brightness);
        let warned = Arc::clone(&self.warned);
        let context = ui.ctx().clone();
        ui.painter().add(egui::PaintCallback {
            rect,
            callback: Arc::new(CallbackFn::new(move |info, painter| {
                if warned.load(Ordering::Relaxed) {
                    return;
                }
                let pixels = info.viewport_in_pixels();
                if pixels.width_px < 2 || pixels.height_px < 2 {
                    return;
                }
                let gl = painter.gl();
                let mut resources = resources.lock().unwrap_or_else(|error| error.into_inner());
                if resources.is_none() {
                    match GlResources::new(gl) {
                        Ok(created) => *resources = Some(created),
                        Err(error) => {
                            if !warned.swap(true, Ordering::Relaxed) {
                                eprintln!("Settings glass fallback: {error}");
                                context.request_repaint();
                            }
                            return;
                        }
                    }
                }
                if let Some(resources) = resources.as_mut() {
                    resources.draw(
                        gl,
                        pixels,
                        scroll_points * info.pixels_per_point,
                        &brightness,
                        &context,
                    );
                }
            })),
        });

        let edge = if dark {
            egui::Color32::from_rgba_unmultiplied(230, 238, 245, 112)
        } else {
            egui::Color32::from_rgba_unmultiplied(255, 255, 255, 172)
        };
        ui.painter().rect_stroke(
            rect.shrink(0.5),
            17.0,
            egui::Stroke::new(1.0, edge),
            egui::StrokeKind::Inside,
        );
    }
}

struct GlResources {
    program: glow::NativeProgram,
    vertex_array: glow::NativeVertexArray,
    texture: glow::NativeTexture,
    size: [i32; 2],
}

impl GlResources {
    fn new(gl: &glow::Context) -> Result<Self, String> {
        unsafe {
            let vertex = gl.create_shader(glow::VERTEX_SHADER)?;
            gl.shader_source(vertex, VERTEX_SHADER);
            gl.compile_shader(vertex);
            if !gl.get_shader_compile_status(vertex) {
                let error = gl.get_shader_info_log(vertex);
                gl.delete_shader(vertex);
                return Err(error);
            }
            let fragment = gl.create_shader(glow::FRAGMENT_SHADER)?;
            gl.shader_source(fragment, FRAGMENT_SHADER);
            gl.compile_shader(fragment);
            if !gl.get_shader_compile_status(fragment) {
                let error = gl.get_shader_info_log(fragment);
                gl.delete_shader(vertex);
                gl.delete_shader(fragment);
                return Err(error);
            }
            let program = gl.create_program()?;
            gl.attach_shader(program, vertex);
            gl.attach_shader(program, fragment);
            gl.link_program(program);
            gl.delete_shader(vertex);
            gl.delete_shader(fragment);
            if !gl.get_program_link_status(program) {
                let error = gl.get_program_info_log(program);
                gl.delete_program(program);
                return Err(error);
            }
            let vertex_array = gl.create_vertex_array()?;
            let texture = gl.create_texture()?;
            gl.bind_texture(glow::TEXTURE_2D, Some(texture));
            gl.tex_parameter_i32(
                glow::TEXTURE_2D,
                glow::TEXTURE_MIN_FILTER,
                glow::LINEAR as i32,
            );
            gl.tex_parameter_i32(
                glow::TEXTURE_2D,
                glow::TEXTURE_MAG_FILTER,
                glow::LINEAR as i32,
            );
            gl.tex_parameter_i32(
                glow::TEXTURE_2D,
                glow::TEXTURE_WRAP_S,
                glow::CLAMP_TO_EDGE as i32,
            );
            gl.tex_parameter_i32(
                glow::TEXTURE_2D,
                glow::TEXTURE_WRAP_T,
                glow::CLAMP_TO_EDGE as i32,
            );
            Ok(Self {
                program,
                vertex_array,
                texture,
                size: [0, 0],
            })
        }
    }

    fn draw(
        &mut self,
        gl: &glow::Context,
        pixels: egui::epaint::ViewportInPixels,
        scroll_px: f32,
        brightness: &AtomicU32,
        context: &egui::Context,
    ) {
        unsafe {
            gl.active_texture(glow::TEXTURE0);
            gl.bind_texture(glow::TEXTURE_2D, Some(self.texture));
            if self.size != [pixels.width_px, pixels.height_px] {
                gl.tex_image_2d(
                    glow::TEXTURE_2D,
                    0,
                    glow::RGBA8 as i32,
                    pixels.width_px,
                    pixels.height_px,
                    0,
                    glow::RGBA,
                    glow::UNSIGNED_BYTE,
                    glow::PixelUnpackData::Slice(None),
                );
                self.size = [pixels.width_px, pixels.height_px];
            }
            gl.copy_tex_sub_image_2d(
                glow::TEXTURE_2D,
                0,
                0,
                0,
                pixels.left_px,
                pixels.from_bottom_px,
                pixels.width_px,
                pixels.height_px,
            );

            // One narrow row is enough for stable light/dark button contrast.
            let mut row = vec![0_u8; pixels.width_px as usize * 4];
            gl.read_pixels(
                pixels.left_px,
                pixels.from_bottom_px + pixels.height_px / 2,
                pixels.width_px,
                1,
                glow::RGBA,
                glow::UNSIGNED_BYTE,
                glow::PixelPackData::Slice(Some(&mut row)),
            );
            let samples = row.chunks_exact(4).step_by(12);
            let (total, count) = samples.fold((0_u32, 0_u32), |(sum, count), rgba| {
                (
                    sum + (rgba[0] as u32 * 2 + rgba[1] as u32 * 7 + rgba[2] as u32) / 10,
                    count + 1,
                )
            });
            if count > 0 {
                let measured = total / count;
                let previous = brightness.swap(measured, Ordering::Relaxed);
                if (previous < 128) != (measured < 128) {
                    context.request_repaint();
                }
            }

            gl.use_program(Some(self.program));
            gl.uniform_1_i32(
                gl.get_uniform_location(self.program, "backdrop").as_ref(),
                0,
            );
            gl.uniform_2_f32(
                gl.get_uniform_location(self.program, "size_px").as_ref(),
                pixels.width_px as f32,
                pixels.height_px as f32,
            );
            gl.uniform_1_f32(
                gl.get_uniform_location(self.program, "scroll_px").as_ref(),
                scroll_px,
            );
            gl.bind_vertex_array(Some(self.vertex_array));
            gl.enable(glow::BLEND);
            gl.blend_func(glow::ONE, glow::ONE_MINUS_SRC_ALPHA);
            gl.draw_arrays(glow::TRIANGLES, 0, 3);
        }
    }
}
