//! GPU 2-pass separable Gaussian blur and layered active-lyric glow renderer for Nhịp.
//!
//! Uses `eframe::egui_glow` (`glow`, OpenGL 3.3 / ES 3.0) with two reusable ping-pong FBOs:
//!   1. Rasterize `egui`'s shaped glyph mesh from the font atlas (`TextureId::Managed(0)`) into FBO A.
//!   2. Run a mathematical 1D horizontal Gaussian convolution from FBO A -> FBO B.
//!   3. Run a mathematical 1D vertical Gaussian convolution from FBO B + vertical viewport mask -> screen,
//!      compositing crisp unblurred foreground glyphs (from FBO A) over the soft Gaussian glow on active lines.

use std::cell::RefCell;
use std::sync::{Arc, Mutex, Weak};

use eframe::egui::{self, Color32, Pos2, Rect};
use eframe::egui_glow::{CallbackFn, Painter};
use eframe::glow::{self, HasContext};

#[derive(Clone, Copy, Debug)]
pub struct LyricGpuParams {
    /// Logical-pixel Gaussian blur sigma for inactive lines (`0.0` = sharp).
    pub blur_sigma_pt: f32,
    /// Logical-pixel outer Gaussian glow sigma for active lines (`0.0` = no glow).
    pub glow_sigma_pt: f32,
    /// Outer glow intensity multiplier (`0.0..=1.2`).
    pub glow_strength: f32,
    /// Tight inner glow intensity multiplier (`0.0..=0.6`).
    pub inner_glow_strength: f32,
    /// Overall line opacity multiplier (`0.0..=1.0`).
    pub opacity: f32,
    /// Viewport bounds used for smooth vertical CSS `mask-image` fade.
    pub viewport: Rect,
    pub pad_h: f32,
    pub fade_h: f32,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct GlyphVertex {
    pos: [f32; 2],
    uv: [f32; 2],
    color: [f32; 4],
    /// Per-glyph atlas texel clamp `[u_min, v_min, u_max, v_max]` preventing adjacent-glyph bleed.
    uv_bounds: [f32; 4],
}

struct LineDrawData {
    vertices: Vec<GlyphVertex>,
    indices: Vec<u32>,
    texture_id: egui::TextureId,
    draw_rect: Rect,
    params: LyricGpuParams,
}

/// Computes safe per-glyph atlas texel clamp bounds `[u_min+0.5, v_min+0.5, u_max-0.5, v_max-0.5]`
/// so linear texture filtering on scaled glyph quads never bleeds texels from neighboring atlas slots.
fn glyph_quad_uv_bounds(quad: &[egui::epaint::Vertex]) -> [f32; 4] {
    let mut u_min = f32::INFINITY;
    let mut v_min = f32::INFINITY;
    let mut u_max = f32::NEG_INFINITY;
    let mut v_max = f32::NEG_INFINITY;
    for qv in quad {
        u_min = u_min.min(qv.uv.x);
        v_min = v_min.min(qv.uv.y);
        u_max = u_max.max(qv.uv.x);
        v_max = v_max.max(qv.uv.y);
    }
    let (cu_min, cu_max) = if u_max - u_min >= 1.0 {
        (u_min + 0.5, u_max - 0.5)
    } else {
        let mid = (u_min + u_max) * 0.5;
        (mid, mid)
    };
    let (cv_min, cv_max) = if v_max - v_min >= 1.0 {
        (v_min + 0.5, v_max - 0.5)
    } else {
        let mid = (v_min + v_max) * 0.5;
        (mid, mid)
    };
    [cu_min, cv_min, cu_max, cv_max]
}

type Slot = Arc<Mutex<Option<Result<Renderer, String>>>>;

/// Renders a laid-out lyric `Galley` with true GPU 2-pass separable Gaussian blur or layered active glow.
/// Returns `true` if scheduled on the GPU callback, or `false` if the caller should use the software fallback
/// (e.g. headless unit tests without an OpenGL context or empty mesh).
pub fn paint_line(
    ui: &egui::Ui,
    galley: &egui::Galley,
    origin: Pos2,
    tint_override: Option<Color32>,
    params: LyricGpuParams,
) -> bool {
    if params.opacity <= 0.004 || !params.viewport.is_positive() {
        return true;
    }
    let vp_id = ui.ctx().viewport_id();
    let slot: Slot = ui.ctx().data_mut(|data| {
        data.get_temp_mut_or_insert_with(egui::Id::NULL.with(("lyrics-gpu", vp_id)), Slot::default)
            .clone()
    });
    if slot
        .lock()
        .is_ok_and(|state| matches!(&*state, Some(Err(_))))
    {
        return false;
    }

    let mut vertices = Vec::new();
    let mut indices = Vec::new();
    let mut texture_id = egui::TextureId::Managed(0);
    let mut min_x = f32::INFINITY;
    let mut min_y = f32::INFINITY;
    let mut max_x = f32::NEG_INFINITY;
    let mut max_y = f32::NEG_INFINITY;

    for placed in &galley.rows {
        let mesh = &placed.row.visuals.mesh;
        if mesh.vertices.is_empty() || mesh.indices.is_empty() {
            continue;
        }
        texture_id = mesh.texture_id;
        let base_v = vertices.len() as u32;
        let row_ox = origin.x + placed.pos.x;
        let row_oy = origin.y + placed.pos.y;
        let glyph_range = &placed.row.visuals.glyph_vertex_range;
        let push_vertex = |vertices: &mut Vec<GlyphVertex>,
                           v: &egui::epaint::Vertex,
                           uv_bounds: [f32; 4],
                           min_x: &mut f32,
                           min_y: &mut f32,
                           max_x: &mut f32,
                           max_y: &mut f32| {
            let x = row_ox + v.pos.x;
            let y = row_oy + v.pos.y;
            *min_x = min_x.min(x);
            *min_y = min_y.min(y);
            *max_x = max_x.max(x);
            *max_y = max_y.max(y);
            let c = tint_override.unwrap_or(v.color);
            let fg_a = c.a() as f32 / 255.0;
            // For active lines, unsung portions (~0.34 alpha) stay crisp without glow while
            // sung portions (~0.42..0.96) smoothly ramp into soft GPU Gaussian glow emission.
            let glow_e = if params.glow_strength > 0.01 && params.glow_sigma_pt > 0.1 {
                let t = ((fg_a - 0.42) / 0.52).clamp(0.0, 1.0);
                t * t * (3.0 - 2.0 * t)
            } else {
                fg_a
            };
            vertices.push(GlyphVertex {
                pos: [x, y],
                uv: [v.uv.x, v.uv.y],
                color: [fg_a, glow_e, fg_a, fg_a],
                uv_bounds,
            });
        };

        let mut i = 0usize;
        while i < mesh.vertices.len() {
            let is_glyph_quad = glyph_range.contains(&i)
                && i + 4 <= mesh.vertices.len()
                && glyph_range.contains(&(i + 3));
            if is_glyph_quad {
                let quad = &mesh.vertices[i..i + 4];
                let bounds = glyph_quad_uv_bounds(quad);
                for qv in quad {
                    push_vertex(&mut vertices, qv, bounds, &mut min_x, &mut min_y, &mut max_x, &mut max_y);
                }
                i += 4;
            } else {
                let v = &mesh.vertices[i];
                let bounds = [0.0, 0.0, 16384.0, 16384.0];
                push_vertex(&mut vertices, v, bounds, &mut min_x, &mut min_y, &mut max_x, &mut max_y);
                i += 1;
            }
        }
        for &idx in &mesh.indices {
            indices.push(base_v + idx);
        }
    }

    if vertices.is_empty() || !min_x.is_finite() {
        return true;
    }

    // Pad the offscreen FBO rect by >= 3 sigma so Gaussian tails never clip at the line bounds,
    // and snap to physical pixel boundaries so `draw_rect` matches `viewport_in_pixels` 1:1.
    let active_glow = params.glow_strength > 0.01 && params.glow_sigma_pt > 0.1;
    let max_sigma = if active_glow {
        params.glow_sigma_pt
    } else {
        params.blur_sigma_pt
    };
    let pad = (max_sigma * 3.0 + 4.0).ceil().clamp(4.0, 48.0);
    let ppp = ui.ctx().pixels_per_point().max(1.0);
    let draw_rect = Rect::from_min_max(
        Pos2::new(
            ((min_x - pad) * ppp).floor() / ppp,
            ((min_y - pad) * ppp).floor() / ppp,
        ),
        Pos2::new(
            ((max_x + pad) * ppp).ceil() / ppp,
            ((max_y + pad) * ppp).ceil() / ppp,
        ),
    );
    if !draw_rect.intersects(params.viewport.expand(12.0)) {
        return true;
    }

    let data = LineDrawData {
        vertices,
        indices,
        texture_id,
        draw_rect,
        params,
    };

    ui.painter().add(egui::PaintCallback {
        rect: draw_rect,
        callback: Arc::new(CallbackFn::new(move |info, painter| {
            let Some(atlas_tex) = painter.texture(data.texture_id) else {
                return;
            };
            let Ok(mut state) = slot.lock() else { return };
            let gl = painter.gl();
            // SAFETY: egui invokes this paint callback on the thread with this GL context current.
            unsafe {
                collect_retired(gl);
                let saved = GlState::save(gl);
                if state.as_ref().is_some_and(|s| {
                    s.as_ref()
                        .is_ok_and(|r| !Weak::ptr_eq(&r.gl, &Arc::downgrade(gl)))
                }) {
                    *state = None;
                }
                let renderer = state.get_or_insert_with(|| {
                    Renderer::new(gl)
                        .inspect_err(|err| eprintln!("Lyrics GPU blur init error: {err}"))
                });
                if let Ok(renderer) = renderer {
                    let vp = info.viewport_in_pixels();
                    let clip = info.clip_rect_in_pixels();
                    if let Err(err) = renderer.draw(
                        painter,
                        atlas_tex,
                        [vp.left_px, vp.from_bottom_px, vp.width_px, vp.height_px],
                        [clip.left_px, clip.from_bottom_px, clip.width_px, clip.height_px],
                        info.pixels_per_point,
                        &data,
                        &saved,
                    ) {
                        eprintln!("Lyrics GPU blur draw error: {err}");
                        *state = Some(Err(err));
                    }
                }
                saved.restore(gl);
            }
        })),
    });
    true
}

struct FboTarget {
    tex: glow::Texture,
    fbo: glow::Framebuffer,
    size: [i32; 2],
}

struct TextLocs {
    atlas: Option<glow::UniformLocation>,
    rect: Option<glow::UniformLocation>,
}

struct BlurHLocs {
    src: Option<glow::UniformLocation>,
    tex_scale: Option<glow::UniformLocation>,
    uv_max: Option<glow::UniformLocation>,
    step_x: Option<glow::UniformLocation>,
    sigma: Option<glow::UniformLocation>,
}

struct CompLocs {
    blur_h: Option<glow::UniformLocation>,
    sharp: Option<glow::UniformLocation>,
    tex_scale: Option<glow::UniformLocation>,
    uv_max: Option<glow::UniformLocation>,
    step_y: Option<glow::UniformLocation>,
    sigma: Option<glow::UniformLocation>,
    mode: Option<glow::UniformLocation>,
    opacity: Option<glow::UniformLocation>,
    glow_strength: Option<glow::UniformLocation>,
    inner_glow: Option<glow::UniformLocation>,
    ppp: Option<glow::UniformLocation>,
    step_xy: Option<glow::UniformLocation>,
    mask: Option<glow::UniformLocation>,
    fade: Option<glow::UniformLocation>,
}

struct Renderer {
    gl: Weak<glow::Context>,
    text_prog: glow::Program,
    blur_h_prog: glow::Program,
    comp_prog: glow::Program,
    text_locs: TextLocs,
    blur_h_locs: BlurHLocs,
    comp_locs: CompLocs,
    text_vao: glow::VertexArray,
    text_vbo: glow::Buffer,
    text_ebo: glow::Buffer,
    quad_vao: glow::VertexArray,
    fbo_a: Option<FboTarget>,
    fbo_b: Option<FboTarget>,
}

thread_local! {
    static RETIRED: RefCell<Vec<Renderer>> = const { RefCell::new(Vec::new()) };
}

impl Drop for Renderer {
    fn drop(&mut self) {
        if self.gl.strong_count() > 0 {
            RETIRED.with(|q| {
                q.borrow_mut().push(Self {
                    gl: std::mem::replace(&mut self.gl, Weak::new()),
                    text_prog: self.text_prog,
                    blur_h_prog: self.blur_h_prog,
                    comp_prog: self.comp_prog,
                    text_locs: TextLocs { atlas: None, rect: None },
                    blur_h_locs: BlurHLocs {
                        src: None,
                        tex_scale: None,
                        uv_max: None,
                        step_x: None,
                        sigma: None,
                    },
                    comp_locs: CompLocs {
                        blur_h: None,
                        sharp: None,
                        tex_scale: None,
                        uv_max: None,
                        step_y: None,
                        sigma: None,
                        mode: None,
                        opacity: None,
                        glow_strength: None,
                        inner_glow: None,
                        ppp: None,
                        step_xy: None,
                        mask: None,
                        fade: None,
                    },
                    text_vao: self.text_vao,
                    text_vbo: self.text_vbo,
                    text_ebo: self.text_ebo,
                    quad_vao: self.quad_vao,
                    fbo_a: self.fbo_a.take(),
                    fbo_b: self.fbo_b.take(),
                })
            });
        }
    }
}

unsafe fn collect_retired(gl: &Arc<glow::Context>) {
    RETIRED.with(|q| {
        q.borrow_mut().retain_mut(|r| {
            if Weak::ptr_eq(&r.gl, &Arc::downgrade(gl)) {
                unsafe {
                    gl.delete_program(r.text_prog);
                    gl.delete_program(r.blur_h_prog);
                    gl.delete_program(r.comp_prog);
                    gl.delete_vertex_array(r.text_vao);
                    gl.delete_buffer(r.text_vbo);
                    gl.delete_buffer(r.text_ebo);
                    gl.delete_vertex_array(r.quad_vao);
                    if let Some(t) = r.fbo_a.take() {
                        gl.delete_framebuffer(t.fbo);
                        gl.delete_texture(t.tex);
                    }
                    if let Some(t) = r.fbo_b.take() {
                        gl.delete_framebuffer(t.fbo);
                        gl.delete_texture(t.tex);
                    }
                }
                r.gl = Weak::new();
                false
            } else if r.gl.strong_count() == 0 {
                r.gl = Weak::new();
                false
            } else {
                true
            }
        });
    });
}

const TEXT_VERT: &str = r#"
layout(location = 0) in vec2 a_pos;
layout(location = 1) in vec2 a_uv;
layout(location = 2) in vec4 a_color;
layout(location = 3) in vec4 a_uv_bounds;

uniform vec4 u_rect; // (min_x, min_y, inv_w, inv_h)

out vec2 v_uv;
out vec4 v_color;
flat out vec4 v_uv_bounds;

void main() {
    float nx = (a_pos.x - u_rect.x) * u_rect.z * 2.0 - 1.0;
    float ny = 1.0 - (a_pos.y - u_rect.y) * u_rect.w * 2.0;
    gl_Position = vec4(nx, ny, 0.0, 1.0);
    v_uv = a_uv;
    v_color = a_color;
    v_uv_bounds = a_uv_bounds;
}
"#;

const TEXT_FRAG: &str = r#"
in vec2 v_uv;
in vec4 v_color;
flat in vec4 v_uv_bounds;

uniform sampler2D u_atlas;

out vec4 frag_color;

void main() {
    // egui's RowVisuals::mesh stores glyph UVs in unnormalized font-atlas texel coordinates.
    // Clamp strictly inside the glyph's own texel rectangle to eliminate adjacent-glyph atlas bleed.
    vec2 atlas_size = max(vec2(textureSize(u_atlas, 0)), vec2(1.0));
    vec2 safe_uv = clamp(v_uv, v_uv_bounds.xy, v_uv_bounds.zw);
    float cov = texture(u_atlas, safe_uv / atlas_size).a;
    // Preserve exact premultiplied alpha coverage (R/B/A = fg coverage, G = glow emission).
    frag_color = v_color * cov;
}
"#;

const QUAD_VERT: &str = r#"
out vec2 v_uv;
void main() {
    vec2 pos = vec2((gl_VertexID == 1) ? 3.0 : -1.0, (gl_VertexID == 2) ? 3.0 : -1.0);
    v_uv = pos * 0.5 + 0.5;
    gl_Position = vec4(pos, 0.0, 1.0);
}
"#;

const BLUR_H_FRAG: &str = r#"
in vec2 v_uv;

uniform sampler2D u_src;
uniform vec2 u_tex_scale; // active region (w_px / tex_w, h_px / tex_h)
uniform vec2 u_uv_max;    // clamped max UV inside active sub-region
uniform float u_step_x;   // 1.0 / tex_w
uniform float u_sigma;    // Gaussian sigma in physical pixels

out vec4 frag_color;

void main() {
    vec2 uv = v_uv * u_tex_scale;
    if (u_sigma <= 0.15) {
        frag_color = texture(u_src, clamp(uv, vec2(0.0), u_uv_max));
        return;
    }
    int rad = clamp(int(ceil(u_sigma * 2.5)), 1, 28);
    float inv_2s2 = 0.5 / max(u_sigma * u_sigma, 0.0001);
    vec4 sum = texture(u_src, clamp(uv, vec2(0.0), u_uv_max));
    float wsum = 1.0;
    // Bilinear-paired Gaussian taps: combine adjacent pixels (i, i+1) into one hardware-filtered fetch.
    for (int i = 1; i <= 28; i += 2) {
        if (i > rad) break;
        float fi = float(i);
        float w1 = exp(-fi * fi * inv_2s2);
        if (i + 1 <= rad) {
            float fi2 = fi + 1.0;
            float w2 = exp(-fi2 * fi2 * inv_2s2);
            float w = w1 + w2;
            float off_px = (fi * w1 + fi2 * w2) / max(w, 1e-6);
            vec2 off = vec2(u_step_x * off_px, 0.0);
            sum += (texture(u_src, clamp(uv + off, vec2(0.0), u_uv_max))
                  + texture(u_src, clamp(uv - off, vec2(0.0), u_uv_max))) * w;
            wsum += 2.0 * w;
        } else {
            vec2 off = vec2(u_step_x * fi, 0.0);
            sum += (texture(u_src, clamp(uv + off, vec2(0.0), u_uv_max))
                  + texture(u_src, clamp(uv - off, vec2(0.0), u_uv_max))) * w1;
            wsum += 2.0 * w1;
        }
    }
    frag_color = sum / wsum;
}
"#;

const COMP_FRAG: &str = r#"
in vec2 v_uv;

uniform sampler2D u_blur_h;
uniform sampler2D u_sharp;
uniform vec2 u_tex_scale;
uniform vec2 u_uv_max;
uniform float u_step_y;
uniform float u_sigma;
uniform int u_mode; // 0 = inactive Gaussian blur, 1 = active sharp + layered glow
uniform float u_opacity;
uniform float u_glow_strength;
uniform float u_inner_glow;
uniform float u_ppp;
uniform vec2 u_step_xy;
uniform vec4 u_mask; // (draw_y_min, draw_y_max, vp_top, vp_bot)
uniform vec2 u_fade; // (pad_h, fade_h)

out vec4 frag_color;

float smooth01(float e0, float e1, float x) {
    float t = clamp((x - e0) / max(e1 - e0, 0.0001), 0.0, 1.0);
    return t * t * (3.0 - 2.0 * t);
}

void main() {
    // v_uv.y == 1.0 is top of draw_rect in OpenGL screen space, 0.0 is bottom.
    vec2 uv = v_uv * u_tex_scale;
    float y_pt = mix(u_mask.y, u_mask.x, v_uv.y);
    float mask_a = 1.0;
    if (u_fade.y > 0.5) {
        mask_a = smooth01(u_fade.x, u_fade.x + u_fade.y, y_pt - u_mask.z)
               * smooth01(u_fade.x, u_fade.x + u_fade.y, u_mask.w - y_pt);
    }

    vec4 blurred;
    if (u_sigma <= 0.15) {
        blurred = texture(u_blur_h, clamp(uv, vec2(0.0), u_uv_max));
    } else {
        int rad = clamp(int(ceil(u_sigma * 2.5)), 1, 28);
        float inv_2s2 = 0.5 / max(u_sigma * u_sigma, 0.0001);
        vec4 sum = texture(u_blur_h, clamp(uv, vec2(0.0), u_uv_max));
        float wsum = 1.0;
        for (int i = 1; i <= 28; i += 2) {
            if (i > rad) break;
            float fi = float(i);
            float w1 = exp(-fi * fi * inv_2s2);
            if (i + 1 <= rad) {
                float fi2 = fi + 1.0;
                float w2 = exp(-fi2 * fi2 * inv_2s2);
                float w = w1 + w2;
                float off_px = (fi * w1 + fi2 * w2) / max(w, 1e-6);
                vec2 off = vec2(0.0, u_step_y * off_px);
                sum += (texture(u_blur_h, clamp(uv + off, vec2(0.0), u_uv_max))
                      + texture(u_blur_h, clamp(uv - off, vec2(0.0), u_uv_max))) * w;
                wsum += 2.0 * w;
            } else {
                vec2 off = vec2(0.0, u_step_y * fi);
                sum += (texture(u_blur_h, clamp(uv + off, vec2(0.0), u_uv_max))
                      + texture(u_blur_h, clamp(uv - off, vec2(0.0), u_uv_max))) * w1;
                wsum += 2.0 * w1;
            }
        }
        blurred = sum / wsum;
    }

    if (u_mode == 0) {
        float a = clamp(blurred.a * u_opacity * mask_a, 0.0, 1.0);
        frag_color = vec4(a, a, a, a);
    } else {
        vec4 sharp = texture(u_sharp, clamp(uv, vec2(0.0), u_uv_max));
        float fg_a = sharp.a;
        // Tight inner glow halo (8-tap soft ring at ~1.8pt on sharp glow channel) + wide Gaussian outer glow.
        float r_in = 1.8 * u_ppp;
        float d_in = r_in * 0.7071;
        float inner_g = (
            texture(u_sharp, clamp(uv + vec2(u_step_xy.x * r_in, 0.0), vec2(0.0), u_uv_max)).g +
            texture(u_sharp, clamp(uv - vec2(u_step_xy.x * r_in, 0.0), vec2(0.0), u_uv_max)).g +
            texture(u_sharp, clamp(uv + vec2(0.0, u_step_xy.y * r_in), vec2(0.0), u_uv_max)).g +
            texture(u_sharp, clamp(uv - vec2(0.0, u_step_xy.y * r_in), vec2(0.0), u_uv_max)).g +
            texture(u_sharp, clamp(uv + vec2(u_step_xy.x * d_in, u_step_xy.y * d_in), vec2(0.0), u_uv_max)).g +
            texture(u_sharp, clamp(uv - vec2(u_step_xy.x * d_in, u_step_xy.y * d_in), vec2(0.0), u_uv_max)).g +
            texture(u_sharp, clamp(uv + vec2(u_step_xy.x * d_in, -u_step_xy.y * d_in), vec2(0.0), u_uv_max)).g +
            texture(u_sharp, clamp(uv - vec2(u_step_xy.x * d_in, -u_step_xy.y * d_in), vec2(0.0), u_uv_max)).g
        ) * 0.125;
        float glow_a = clamp(blurred.g * u_glow_strength + inner_g * u_inner_glow, 0.0, 0.88);
        // Premultiplied razor-sharp foreground OVER soft outer/inner glow so active text never blurs.
        float out_a = clamp((fg_a + glow_a * (1.0 - fg_a)) * u_opacity * mask_a, 0.0, 1.0);
        frag_color = vec4(out_a, out_a, out_a, out_a);
    }
}
"#;

impl Renderer {
    unsafe fn new(gl: &Arc<glow::Context>) -> Result<Self, String> {
        unsafe {
            let text_prog = link_program(gl, TEXT_VERT, TEXT_FRAG)?;
            let blur_h_prog = match link_program(gl, QUAD_VERT, BLUR_H_FRAG) {
                Ok(p) => p,
                Err(e) => {
                    gl.delete_program(text_prog);
                    return Err(e);
                }
            };
            let comp_prog = match link_program(gl, QUAD_VERT, COMP_FRAG) {
                Ok(p) => p,
                Err(e) => {
                    gl.delete_program(text_prog);
                    gl.delete_program(blur_h_prog);
                    return Err(e);
                }
            };

            let text_vao = gl.create_vertex_array()?;
            let text_vbo = gl.create_buffer()?;
            let text_ebo = gl.create_buffer()?;
            let quad_vao = gl.create_vertex_array()?;

            gl.bind_vertex_array(Some(text_vao));
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(text_vbo));
            gl.bind_buffer(glow::ELEMENT_ARRAY_BUFFER, Some(text_ebo));
            let stride = std::mem::size_of::<GlyphVertex>() as i32;
            gl.enable_vertex_attrib_array(0);
            gl.vertex_attrib_pointer_f32(0, 2, glow::FLOAT, false, stride, 0);
            gl.enable_vertex_attrib_array(1);
            gl.vertex_attrib_pointer_f32(1, 2, glow::FLOAT, false, stride, 8);
            gl.enable_vertex_attrib_array(2);
            gl.vertex_attrib_pointer_f32(2, 4, glow::FLOAT, false, stride, 16);
            gl.enable_vertex_attrib_array(3);
            gl.vertex_attrib_pointer_f32(3, 4, glow::FLOAT, false, stride, 32);

            let text_locs = TextLocs {
                atlas: gl.get_uniform_location(text_prog, "u_atlas"),
                rect: gl.get_uniform_location(text_prog, "u_rect"),
            };
            let blur_h_locs = BlurHLocs {
                src: gl.get_uniform_location(blur_h_prog, "u_src"),
                tex_scale: gl.get_uniform_location(blur_h_prog, "u_tex_scale"),
                uv_max: gl.get_uniform_location(blur_h_prog, "u_uv_max"),
                step_x: gl.get_uniform_location(blur_h_prog, "u_step_x"),
                sigma: gl.get_uniform_location(blur_h_prog, "u_sigma"),
            };
            let comp_locs = CompLocs {
                blur_h: gl.get_uniform_location(comp_prog, "u_blur_h"),
                sharp: gl.get_uniform_location(comp_prog, "u_sharp"),
                tex_scale: gl.get_uniform_location(comp_prog, "u_tex_scale"),
                uv_max: gl.get_uniform_location(comp_prog, "u_uv_max"),
                step_y: gl.get_uniform_location(comp_prog, "u_step_y"),
                sigma: gl.get_uniform_location(comp_prog, "u_sigma"),
                mode: gl.get_uniform_location(comp_prog, "u_mode"),
                opacity: gl.get_uniform_location(comp_prog, "u_opacity"),
                glow_strength: gl.get_uniform_location(comp_prog, "u_glow_strength"),
                inner_glow: gl.get_uniform_location(comp_prog, "u_inner_glow"),
                ppp: gl.get_uniform_location(comp_prog, "u_ppp"),
                step_xy: gl.get_uniform_location(comp_prog, "u_step_xy"),
                mask: gl.get_uniform_location(comp_prog, "u_mask"),
                fade: gl.get_uniform_location(comp_prog, "u_fade"),
            };

            Ok(Self {
                gl: Arc::downgrade(gl),
                text_prog,
                blur_h_prog,
                comp_prog,
                text_locs,
                blur_h_locs,
                comp_locs,
                text_vao,
                text_vbo,
                text_ebo,
                quad_vao,
                fbo_a: None,
                fbo_b: None,
            })
        }
    }

    unsafe fn ensure_fbo(
        gl: &glow::Context,
        slot: &mut Option<FboTarget>,
        need_w: i32,
        need_h: i32,
    ) -> Result<(glow::Texture, glow::Framebuffer, [i32; 2]), String> {
        if let Some(t) = slot.as_ref() {
            if t.size[0] >= need_w && t.size[1] >= need_h {
                return Ok((t.tex, t.fbo, t.size));
            }
        }
        // Grow in quantized 128px steps so resizing or slightly taller multiline lyrics never reallocate every frame.
        let alloc_w = ((need_w.max(256) + 127) & !127).min(2048);
        let alloc_h = ((need_h.max(128) + 127) & !127).min(1024);
        let tex = gl.create_texture()?;
        let fbo = match gl.create_framebuffer() {
            Ok(f) => f,
            Err(e) => {
                gl.delete_texture(tex);
                return Err(e);
            }
        };
        gl.bind_texture(glow::TEXTURE_2D, Some(tex));
        gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_MIN_FILTER, glow::LINEAR as i32);
        gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_MAG_FILTER, glow::LINEAR as i32);
        gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_WRAP_S, glow::CLAMP_TO_EDGE as i32);
        gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_WRAP_T, glow::CLAMP_TO_EDGE as i32);
        // Use RGBA8 (not RGBA16F): 8-bit alpha coverage + separable Gaussian blur matches the RGBA8
        // target framebuffer at half the bandwidth and avoids Intel RaptorLake CCS fast-clear ghosting
        // when reusing ping-pong FBOs across consecutive lyric lines within a single frame.
        gl.tex_image_2d(
            glow::TEXTURE_2D,
            0,
            glow::RGBA8 as i32,
            alloc_w,
            alloc_h,
            0,
            glow::RGBA,
            glow::UNSIGNED_BYTE,
            glow::PixelUnpackData::Slice(None),
        );
        gl.bind_framebuffer(glow::FRAMEBUFFER, Some(fbo));
        gl.framebuffer_texture_2d(
            glow::FRAMEBUFFER,
            glow::COLOR_ATTACHMENT0,
            glow::TEXTURE_2D,
            Some(tex),
            0,
        );
        if gl.check_framebuffer_status(glow::FRAMEBUFFER) != glow::FRAMEBUFFER_COMPLETE {
            gl.delete_framebuffer(fbo);
            gl.delete_texture(tex);
            return Err("Lyrics blur framebuffer incomplete".into());
        }
        if let Some(old) = slot.replace(FboTarget {
            tex,
            fbo,
            size: [alloc_w, alloc_h],
        }) {
            gl.delete_framebuffer(old.fbo);
            gl.delete_texture(old.tex);
        }
        Ok((tex, fbo, [alloc_w, alloc_h]))
    }

    #[allow(clippy::too_many_arguments)]
    unsafe fn draw(
        &mut self,
        painter: &Painter,
        atlas_tex: glow::Texture,
        vp: [i32; 4],
        clip: [i32; 4],
        ppp: f32,
        data: &LineDrawData,
        saved: &GlState,
    ) -> Result<(), String> {
        let gl = painter.gl();
        let w_px = vp[2].clamp(1, 2048);
        let h_px = vp[3].clamp(1, 1024);
        if vp[2] <= 0 || vp[3] <= 0 {
            return Ok(());
        }

        let p = &data.params;
        let is_active_glow = p.glow_strength > 0.01 && p.glow_sigma_pt > 0.1;
        let full_sigma_px = if is_active_glow {
            (p.glow_sigma_pt * ppp).clamp(0.0, 26.0)
        } else {
            (p.blur_sigma_pt * ppp).clamp(0.0, 26.0)
        };
        // Inactive blurred lines (`sigma >= 1.5px`) are band-limited: running their offscreen
        // Gaussian passes at half resolution cuts fragment + texture work by ~8x with identical output.
        let half_res = !is_active_glow && full_sigma_px >= 1.5 && w_px >= 8 && h_px >= 8;
        let (fbo_w, fbo_h, sigma_px) = if half_res {
            (((w_px + 1) / 2).max(1), ((h_px + 1) / 2).max(1), full_sigma_px * 0.5)
        } else {
            (w_px, h_px, full_sigma_px)
        };

        let (tex_a, fbo_a, size_a) = Self::ensure_fbo(gl, &mut self.fbo_a, fbo_w, fbo_h)?;
        let (tex_b, fbo_b, size_b) = Self::ensure_fbo(gl, &mut self.fbo_b, fbo_w, fbo_h)?;

        // Pass 1: Render glyph triangles into FBO A (`0..fbo_w, 0..fbo_h`) with scissor-bounded clear.
        gl.active_texture(glow::TEXTURE1);
        gl.bind_texture(glow::TEXTURE_2D, None);
        gl.active_texture(glow::TEXTURE0);
        gl.bind_texture(glow::TEXTURE_2D, None);
        gl.bind_framebuffer(glow::FRAMEBUFFER, Some(fbo_a));
        gl.viewport(0, 0, fbo_w, fbo_h);
        gl.enable(glow::SCISSOR_TEST);
        gl.scissor(0, 0, fbo_w, fbo_h);
        gl.color_mask(true, true, true, true);
        gl.enable(glow::BLEND);
        gl.blend_func_separate(
            glow::ONE,
            glow::ONE_MINUS_SRC_ALPHA,
            glow::ONE,
            glow::ONE_MINUS_SRC_ALPHA,
        );
        gl.clear_color(0.0, 0.0, 0.0, 0.0);
        gl.clear(glow::COLOR_BUFFER_BIT);

        gl.use_program(Some(self.text_prog));
        gl.active_texture(glow::TEXTURE0);
        gl.bind_texture(glow::TEXTURE_2D, Some(atlas_tex));
        gl.uniform_1_i32(self.text_locs.atlas.as_ref(), 0);
        gl.uniform_4_f32(
            self.text_locs.rect.as_ref(),
            data.draw_rect.min.x,
            data.draw_rect.min.y,
            1.0 / data.draw_rect.width().max(1e-4),
            1.0 / data.draw_rect.height().max(1e-4),
        );

        gl.bind_vertex_array(Some(self.text_vao));
        gl.bind_buffer(glow::ARRAY_BUFFER, Some(self.text_vbo));
        let v_bytes = std::slice::from_raw_parts(
            data.vertices.as_ptr() as *const u8,
            data.vertices.len() * std::mem::size_of::<GlyphVertex>(),
        );
        gl.buffer_data_u8_slice(glow::ARRAY_BUFFER, v_bytes, glow::STREAM_DRAW);

        gl.bind_buffer(glow::ELEMENT_ARRAY_BUFFER, Some(self.text_ebo));
        let i_bytes = std::slice::from_raw_parts(
            data.indices.as_ptr() as *const u8,
            data.indices.len() * std::mem::size_of::<u32>(),
        );
        gl.buffer_data_u8_slice(glow::ELEMENT_ARRAY_BUFFER, i_bytes, glow::STREAM_DRAW);
        gl.draw_elements(
            glow::TRIANGLES,
            data.indices.len() as i32,
            glow::UNSIGNED_INT,
            0,
        );

        // Pass 2: Horizontal 1D Gaussian blur (FBO A -> FBO B). Skip completely when sigma <= 0.15.
        // Unbind `TEXTURE0`/`TEXTURE1` before switching FBO target to avoid any texture-FBO feedback hazard.
        let need_blur = sigma_px > 0.15;
        if need_blur {
            gl.active_texture(glow::TEXTURE1);
            gl.bind_texture(glow::TEXTURE_2D, None);
            gl.active_texture(glow::TEXTURE0);
            gl.bind_texture(glow::TEXTURE_2D, None);
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(fbo_b));
            gl.viewport(0, 0, fbo_w, fbo_h);
            gl.scissor(0, 0, fbo_w, fbo_h);
            gl.disable(glow::BLEND);
            gl.use_program(Some(self.blur_h_prog));
            gl.bind_vertex_array(Some(self.quad_vao));
            gl.active_texture(glow::TEXTURE0);
            gl.bind_texture(glow::TEXTURE_2D, Some(tex_a));
            gl.uniform_1_i32(self.blur_h_locs.src.as_ref(), 0);
            let scale_ax = fbo_w as f32 / size_a[0] as f32;
            let scale_ay = fbo_h as f32 / size_a[1] as f32;
            let max_ax = (fbo_w as f32 - 0.5).max(0.5) / size_a[0] as f32;
            let max_ay = (fbo_h as f32 - 0.5).max(0.5) / size_a[1] as f32;
            gl.uniform_2_f32(self.blur_h_locs.tex_scale.as_ref(), scale_ax, scale_ay);
            gl.uniform_2_f32(self.blur_h_locs.uv_max.as_ref(), max_ax, max_ay);
            gl.uniform_1_f32(self.blur_h_locs.step_x.as_ref(), 1.0 / size_a[0] as f32);
            gl.uniform_1_f32(self.blur_h_locs.sigma.as_ref(), sigma_px);
            gl.draw_arrays(glow::TRIANGLES, 0, 3);
        }

        // Pass 3: Vertical 1D Gaussian blur + viewport fade + crisp foreground composite -> screen FBO.
        gl.bind_framebuffer(glow::FRAMEBUFFER, painter.intermediate_fbo());
        gl.viewport(vp[0], vp[1], vp[2], vp[3]);
        gl.scissor(clip[0], clip[1], clip[2], clip[3]);
        if !saved.scissor {
            gl.disable(glow::SCISSOR_TEST);
        }
        gl.enable(glow::BLEND);
        gl.blend_func_separate(
            glow::ONE,
            glow::ONE_MINUS_SRC_ALPHA,
            glow::ONE,
            glow::ONE_MINUS_SRC_ALPHA,
        );
        gl.use_program(Some(self.comp_prog));
        gl.bind_vertex_array(Some(self.quad_vao));

        let (blur_tex, blur_size) = if need_blur { (tex_b, size_b) } else { (tex_a, size_a) };
        gl.active_texture(glow::TEXTURE0);
        gl.bind_texture(glow::TEXTURE_2D, Some(blur_tex));
        gl.uniform_1_i32(self.comp_locs.blur_h.as_ref(), 0);
        gl.active_texture(glow::TEXTURE1);
        gl.bind_texture(glow::TEXTURE_2D, Some(tex_a));
        gl.uniform_1_i32(self.comp_locs.sharp.as_ref(), 1);

        let scale_bx = fbo_w as f32 / blur_size[0] as f32;
        let scale_by = fbo_h as f32 / blur_size[1] as f32;
        let max_bx = (fbo_w as f32 - 0.5).max(0.5) / blur_size[0] as f32;
        let max_by = (fbo_h as f32 - 0.5).max(0.5) / blur_size[1] as f32;
        gl.uniform_2_f32(self.comp_locs.tex_scale.as_ref(), scale_bx, scale_by);
        gl.uniform_2_f32(self.comp_locs.uv_max.as_ref(), max_bx, max_by);
        gl.uniform_1_f32(self.comp_locs.step_y.as_ref(), 1.0 / blur_size[1] as f32);
        gl.uniform_1_f32(self.comp_locs.sigma.as_ref(), sigma_px);
        gl.uniform_1_i32(self.comp_locs.mode.as_ref(), if is_active_glow { 1 } else { 0 });
        gl.uniform_1_f32(self.comp_locs.opacity.as_ref(), p.opacity);
        gl.uniform_1_f32(self.comp_locs.glow_strength.as_ref(), p.glow_strength);
        gl.uniform_1_f32(self.comp_locs.inner_glow.as_ref(), p.inner_glow_strength);
        gl.uniform_1_f32(self.comp_locs.ppp.as_ref(), ppp);
        gl.uniform_2_f32(
            self.comp_locs.step_xy.as_ref(),
            1.0 / size_a[0] as f32,
            1.0 / size_a[1] as f32,
        );
        gl.uniform_4_f32(
            self.comp_locs.mask.as_ref(),
            data.draw_rect.min.y,
            data.draw_rect.max.y,
            p.viewport.top(),
            p.viewport.bottom(),
        );
        gl.uniform_2_f32(self.comp_locs.fade.as_ref(), p.pad_h, p.fade_h);
        gl.draw_arrays(glow::TRIANGLES, 0, 3);
        Ok(())
    }
}

unsafe fn link_program(gl: &glow::Context, vert: &str, frag: &str) -> Result<glow::Program, String> {
    let program = gl.create_program()?;
    let version = if gl.version().is_embedded {
        "#version 300 es\nprecision highp float;\n"
    } else {
        "#version 330 core\n#define highp\n"
    };
    for (kind, src) in [(glow::VERTEX_SHADER, vert), (glow::FRAGMENT_SHADER, frag)] {
        let shader = match gl.create_shader(kind) {
            Ok(s) => s,
            Err(e) => {
                gl.delete_program(program);
                return Err(e);
            }
        };
        gl.shader_source(shader, &format!("{version}{src}"));
        gl.compile_shader(shader);
        if !gl.get_shader_compile_status(shader) {
            let log = gl.get_shader_info_log(shader);
            gl.delete_shader(shader);
            gl.delete_program(program);
            return Err(log);
        }
        gl.attach_shader(program, shader);
        gl.delete_shader(shader);
    }
    gl.link_program(program);
    if !gl.get_program_link_status(program) {
        let log = gl.get_program_info_log(program);
        gl.delete_program(program);
        return Err(log);
    }
    Ok(program)
}

struct GlState {
    viewport: [i32; 4],
    framebuffer: Option<glow::Framebuffer>,
    read_framebuffer: Option<glow::Framebuffer>,
    program: Option<glow::Program>,
    vao: Option<glow::VertexArray>,
    array_buffer: Option<glow::Buffer>,
    active_texture: u32,
    texture0: Option<glow::Texture>,
    texture1: Option<glow::Texture>,
    blend: bool,
    blend_src_rgb: u32,
    blend_dst_rgb: u32,
    blend_src_alpha: u32,
    blend_dst_alpha: u32,
    scissor: bool,
    clear_color: [f32; 4],
}

impl GlState {
    unsafe fn save(gl: &glow::Context) -> Self {
        let mut viewport = [0; 4];
        gl.get_parameter_i32_slice(glow::VIEWPORT, &mut viewport);
        let mut clear_color = [0.0; 4];
        gl.get_parameter_f32_slice(glow::COLOR_CLEAR_VALUE, &mut clear_color);
        let active_texture = gl.get_parameter_i32(glow::ACTIVE_TEXTURE) as u32;
        gl.active_texture(glow::TEXTURE0);
        let texture0 = gl.get_parameter_texture(glow::TEXTURE_BINDING_2D);
        gl.active_texture(glow::TEXTURE1);
        let texture1 = gl.get_parameter_texture(glow::TEXTURE_BINDING_2D);
        Self {
            viewport,
            framebuffer: gl.get_parameter_framebuffer(glow::DRAW_FRAMEBUFFER_BINDING),
            read_framebuffer: gl.get_parameter_framebuffer(glow::READ_FRAMEBUFFER_BINDING),
            program: gl.get_parameter_program(glow::CURRENT_PROGRAM),
            vao: gl.get_parameter_vertex_array(glow::VERTEX_ARRAY_BINDING),
            array_buffer: gl.get_parameter_buffer(glow::ARRAY_BUFFER_BINDING),
            active_texture,
            texture0,
            texture1,
            blend: gl.is_enabled(glow::BLEND),
            blend_src_rgb: gl.get_parameter_i32(glow::BLEND_SRC_RGB) as u32,
            blend_dst_rgb: gl.get_parameter_i32(glow::BLEND_DST_RGB) as u32,
            blend_src_alpha: gl.get_parameter_i32(glow::BLEND_SRC_ALPHA) as u32,
            blend_dst_alpha: gl.get_parameter_i32(glow::BLEND_DST_ALPHA) as u32,
            scissor: gl.is_enabled(glow::SCISSOR_TEST),
            clear_color,
        }
    }

    unsafe fn restore(&self, gl: &glow::Context) {
        gl.bind_framebuffer(glow::DRAW_FRAMEBUFFER, self.framebuffer);
        gl.bind_framebuffer(glow::READ_FRAMEBUFFER, self.read_framebuffer);
        gl.viewport(
            self.viewport[0],
            self.viewport[1],
            self.viewport[2],
            self.viewport[3],
        );
        gl.clear_color(
            self.clear_color[0],
            self.clear_color[1],
            self.clear_color[2],
            self.clear_color[3],
        );
        gl.use_program(self.program);
        gl.bind_vertex_array(self.vao);
        gl.bind_buffer(glow::ARRAY_BUFFER, self.array_buffer);
        gl.active_texture(glow::TEXTURE0);
        gl.bind_texture(glow::TEXTURE_2D, self.texture0);
        gl.active_texture(glow::TEXTURE1);
        gl.bind_texture(glow::TEXTURE_2D, self.texture1);
        gl.active_texture(self.active_texture);
        gl.blend_func_separate(
            self.blend_src_rgb,
            self.blend_dst_rgb,
            self.blend_src_alpha,
            self.blend_dst_alpha,
        );
        for (cap, enabled) in [(glow::BLEND, self.blend), (glow::SCISSOR_TEST, self.scissor)] {
            if enabled {
                gl.enable(cap);
            } else {
                gl.disable(cap);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn glyph_quad_uv_bounds_inset_half_texel_and_handle_subtexel_quads() {
        let mk = |u: f32, v: f32| egui::epaint::Vertex {
            pos: Pos2::ZERO,
            uv: Pos2::new(u, v),
            color: Color32::WHITE,
        };
        let normal = [mk(100.0, 50.0), mk(124.0, 50.0), mk(100.0, 82.0), mk(124.0, 82.0)];
        assert_eq!(glyph_quad_uv_bounds(&normal), [100.5, 50.5, 123.5, 81.5]);

        // Degenerate < 1px quad collapses cleanly to midpoint without inverting min > max.
        let thin = [mk(12.0, 4.0), mk(12.4, 4.0), mk(12.0, 4.6), mk(12.4, 4.6)];
        let b = glyph_quad_uv_bounds(&thin);
        assert!(b[0] <= b[2] && b[1] <= b[3]);
        assert!((b[0] - 12.2).abs() < 1e-5 && (b[1] - 4.3).abs() < 1e-5);
    }

    #[test]
    fn glyph_vertex_layout_matches_gl_attribute_offsets() {
        assert_eq!(std::mem::size_of::<GlyphVertex>(), 48);
        assert_eq!(std::mem::offset_of!(GlyphVertex, pos), 0);
        assert_eq!(std::mem::offset_of!(GlyphVertex, uv), 8);
        assert_eq!(std::mem::offset_of!(GlyphVertex, color), 16);
        assert_eq!(std::mem::offset_of!(GlyphVertex, uv_bounds), 32);
    }
}
