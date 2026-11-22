//! GPU port of @kawarp/core 1.3.1; shader attribution is in backdrop/SOURCE.md.

use std::cell::RefCell;
use std::sync::{Arc, Mutex, Weak};

use eframe::egui::{self, Color32, Rect, TextureHandle};
use eframe::egui_glow::{CallbackFn, Painter};
use eframe::glow::{self, HasContext};

type Slot = Arc<Mutex<Option<Result<Renderer, String>>>>;

/// Shared sidebar backdrop context stored per-frame so `GlassCard` surfaces can sample
/// pixel-aligned GPU backdrop blur directly from the dynamic Kawarp background.
#[derive(Clone)]
pub struct GlassBackdropContext {
    pub panel_rect: Rect,
    pub texture: TextureHandle,
    pub phase: f32,
    pub sidebar: bool,
}

/// Paint a raw, blurred cover with Kawarp's phase in accumulated animation seconds.
/// `sidebar` selects the NPv CSS filter; `alpha` is the layer's crossfade opacity.
pub fn paint(ui: &egui::Ui, rect: Rect, tex: &TextureHandle, phase: f32, alpha: f32, sidebar: bool, header_fade: Option<f32>) {
    paint_impl(ui, rect, rect, tex, phase, alpha, sidebar, header_fade, None, 0.0, 0.0);
}

/// Builds a `Shape::Callback` for a frosted glass backdrop blur strictly inside `card_rect`,
/// seamlessly aligned to `panel_rect` so callers can insert it into a reserved background painter slot.
pub fn glass_shape(
    ui: &egui::Ui,
    card_rect: Rect,
    ctx: &GlassBackdropContext,
    radius_pt: f32,
    frost_uv: f32,
    tint: f32,
) -> Option<egui::Shape> {
    build_shape(
        ui,
        card_rect,
        ctx.panel_rect,
        &ctx.texture,
        ctx.phase,
        1.0,
        ctx.sidebar,
        None,
        Some(radius_pt),
        frost_uv,
        tint,
    )
}

#[allow(clippy::too_many_arguments)]
fn paint_impl(
    ui: &egui::Ui,
    rect: Rect,
    panel_rect: Rect,
    tex: &TextureHandle,
    phase: f32,
    alpha: f32,
    sidebar: bool,
    header_fade: Option<f32>,
    radius_override_pt: Option<f32>,
    frost_uv: f32,
    tint: f32,
) -> bool {
    if let Some(shape) = build_shape(
        ui,
        rect,
        panel_rect,
        tex,
        phase,
        alpha,
        sidebar,
        header_fade,
        radius_override_pt,
        frost_uv,
        tint,
    ) {
        ui.painter().add(shape);
        true
    } else {
        if frost_uv <= 0.0001 && rect.is_positive() && rect.intersects(ui.clip_rect()) && alpha > 0.0 {
            ui.painter().image(
                tex.id(),
                rect,
                Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
                Color32::from_white_alpha((alpha.clamp(0.0, 1.0) * 255.0) as u8),
            );
        }
        false
    }
}

#[allow(clippy::too_many_arguments)]
fn build_shape(
    ui: &egui::Ui,
    rect: Rect,
    panel_rect: Rect,
    tex: &TextureHandle,
    phase: f32,
    alpha: f32,
    sidebar: bool,
    header_fade: Option<f32>,
    radius_override_pt: Option<f32>,
    frost_uv: f32,
    tint: f32,
) -> Option<egui::Shape> {
    if !rect.is_positive() || !panel_rect.is_positive() || !rect.intersects(ui.clip_rect()) || alpha <= 0.0 {
        return None;
    }
    let vp_id = ui.ctx().viewport_id();
    let slot: Slot = ui.ctx().data_mut(|data| {
        data.get_temp_mut_or_insert_with(egui::Id::NULL.with(("kawarp-gpu", vp_id)), Slot::default).clone()
    });
    if slot.lock().is_ok_and(|state| matches!(&*state, Some(Err(_)))) {
        return None;
    }
    let pw = panel_rect.width().max(1.0);
    let ph = panel_rect.height().max(1.0);
    let uv_rect = [
        (rect.left() - panel_rect.left()) / pw,
        1.0 - (rect.bottom() - panel_rect.top()) / ph,
        rect.width() / pw,
        rect.height() / ph,
    ];
    let tex = tex.clone();
    Some(egui::Shape::Callback(egui::PaintCallback {
        rect,
        callback: Arc::new(CallbackFn::new(move |info, painter| {
            let Some(texture) = painter.texture(tex.id()) else { return };
            let Ok(mut state) = slot.lock() else { return };
            let gl = painter.gl();
            // SAFETY: egui invokes this paint callback on the thread with this GL context current.
            unsafe {
                collect_retired(gl);
                let saved = GlState::save(gl);
                if state.as_ref().is_some_and(|s| s.as_ref().is_ok_and(|r| !Weak::ptr_eq(&r.gl, &Arc::downgrade(gl)))) {
                    *state = None;
                }
                let renderer = state.get_or_insert_with(|| {
                    Renderer::new(gl).inspect_err(|error| eprintln!("Kawarp GPU backdrop: {error}"))
                });
                if let Ok(renderer) = renderer {
                    let viewport = info.viewport_in_pixels();
                    let ppp = viewport.width_px as f32 / rect.width().max(1.0);
                    let radius = match radius_override_pt {
                        Some(r) => r * ppp,
                        None if rect.left() > 0.0 && rect.top() > 0.0 => 8.0 * ppp,
                        None => 0.0,
                    };
                    if let Err(error) = renderer.draw(
                        painter,
                        texture,
                        [viewport.width_px, viewport.height_px],
                        phase,
                        alpha.clamp(0.0, 1.0),
                        sidebar,
                        header_fade.map(|h| [(h - 16.0) / rect.height(), (h + 32.0) / rect.height()]),
                        radius,
                        uv_rect,
                        frost_uv,
                        tint,
                        &saved,
                    ) {
                        eprintln!("Kawarp GPU backdrop: {error}");
                        *state = Some(Err(error));
                    }
                }
                saved.restore(gl);
            }
        })),
    }))
}

type TargetSlot = (glow::Texture, glow::Framebuffer, [i32; 2], u64);

struct WarpLocs {
    texture: Option<glow::UniformLocation>,
    time: Option<glow::UniformLocation>,
    intensity: Option<glow::UniformLocation>,
}

struct OutputLocs {
    texture: Option<glow::UniformLocation>,
    time: Option<glow::UniformLocation>,
    saturation: Option<glow::UniformLocation>,
    dithering: Option<glow::UniformLocation>,
    scale: Option<glow::UniformLocation>,
    alpha: Option<glow::UniformLocation>,
    sidebar: Option<glow::UniformLocation>,
    frost: Option<glow::UniformLocation>,
    tint: Option<glow::UniformLocation>,
    uv_rect: Option<glow::UniformLocation>,
    radius: Option<glow::UniformLocation>,
    header_fade: Option<glow::UniformLocation>,
    resolution: Option<glow::UniformLocation>,
}

struct Renderer {
    gl: Weak<glow::Context>,
    warp: glow::Program,
    output: glow::Program,
    warp_locs: WarpLocs,
    output_locs: OutputLocs,
    vao: glow::VertexArray,
    targets: [Option<TargetSlot>; 4],
    last_warp: Option<(glow::Texture, u32)>,
    tick: u64,
}

// Deletion must also run with the owning context current, never from egui's data destructor.
thread_local! {
    static RETIRED: RefCell<Vec<Renderer>> = const { RefCell::new(Vec::new()) };
}

impl Drop for Renderer {
    fn drop(&mut self) {
        if self.gl.strong_count() > 0 {
            RETIRED.with(|queue| queue.borrow_mut().push(Self {
                gl: std::mem::replace(&mut self.gl, Weak::new()),
                warp: self.warp,
                output: self.output,
                warp_locs: WarpLocs { texture: None, time: None, intensity: None },
                output_locs: OutputLocs {
                    texture: None,
                    time: None,
                    saturation: None,
                    dithering: None,
                    scale: None,
                    alpha: None,
                    sidebar: None,
                    frost: None,
                    tint: None,
                    uv_rect: None,
                    radius: None,
                    header_fade: None,
                    resolution: None,
                },
                vao: self.vao,
                targets: std::mem::take(&mut self.targets),
                last_warp: None,
                tick: 0,
            }));
        }
    }
}

unsafe fn collect_retired(gl: &Arc<glow::Context>) {
    RETIRED.with(|queue| queue.borrow_mut().retain_mut(|renderer| {
        if Weak::ptr_eq(&renderer.gl, &Arc::downgrade(gl)) {
            // SAFETY: this queue entry belongs to the callback's current context.
            unsafe {
                gl.delete_program(renderer.warp);
                gl.delete_program(renderer.output);
                gl.delete_vertex_array(renderer.vao);
                for target in &mut renderer.targets {
                    if let Some((texture, framebuffer, _, _)) = target.take() {
                        gl.delete_framebuffer(framebuffer);
                        gl.delete_texture(texture);
                    }
                }
            }
            renderer.gl = Weak::new();
            false
        } else if renderer.gl.strong_count() == 0 {
            // Destroying a native context already releases its remaining driver resources.
            renderer.gl = Weak::new();
            false
        } else {
            true
        }
    }));
}

impl Renderer {
    unsafe fn new(gl: &Arc<glow::Context>) -> Result<Self, String> {
        // SAFETY: called only by the current-context paint callback.
        unsafe {
            let warp = program(gl, include_str!("backdrop/warp.frag"))?;
            let output = match program(gl, include_str!("backdrop/output.frag")) {
                Ok(program) => program,
                Err(error) => { gl.delete_program(warp); return Err(error); }
            };
            let vao = match gl.create_vertex_array() {
                Ok(vao) => vao,
                Err(error) => { gl.delete_program(warp); gl.delete_program(output); return Err(error); }
            };
            let warp_locs = WarpLocs {
                texture: gl.get_uniform_location(warp, "u_texture"),
                time: gl.get_uniform_location(warp, "u_time"),
                intensity: gl.get_uniform_location(warp, "u_intensity"),
            };
            let output_locs = OutputLocs {
                texture: gl.get_uniform_location(output, "u_texture"),
                time: gl.get_uniform_location(output, "u_time"),
                saturation: gl.get_uniform_location(output, "u_saturation"),
                dithering: gl.get_uniform_location(output, "u_dithering"),
                scale: gl.get_uniform_location(output, "u_scale"),
                alpha: gl.get_uniform_location(output, "u_alpha"),
                sidebar: gl.get_uniform_location(output, "u_sidebar"),
                frost: gl.get_uniform_location(output, "u_frost"),
                tint: gl.get_uniform_location(output, "u_tint"),
                uv_rect: gl.get_uniform_location(output, "u_uv_rect"),
                radius: gl.get_uniform_location(output, "u_radius"),
                header_fade: gl.get_uniform_location(output, "u_header_fade"),
                resolution: gl.get_uniform_location(output, "u_resolution"),
            };
            Ok(Self {
                gl: Arc::downgrade(gl),
                warp,
                output,
                warp_locs,
                output_locs,
                vao,
                targets: [None, None, None, None],
                last_warp: None,
                tick: 0,
            })
        }
    }

    #[allow(clippy::too_many_arguments)]
    unsafe fn draw(
        &mut self,
        painter: &Painter,
        source: glow::Texture,
        size: [i32; 2],
        phase: f32,
        alpha: f32,
        sidebar: bool,
        header_fade: Option<[f32; 2]>,
        radius: f32,
        uv_rect: [f32; 4],
        frost_uv: f32,
        tint: f32,
        saved: &GlState,
    ) -> Result<(), String> {
        let gl = painter.gl();
        if size[0] <= 0 || size[1] <= 0 { return Ok(()); }
        // SAFETY: all handles belong to the callback's current context; the outer callback restores state on errors too.
        unsafe {
            gl.active_texture(glow::TEXTURE0);
            self.tick = self.tick.wrapping_add(1);
            let tick = self.tick;
            // The input cover bokeh texture is 64x64 low-frequency color clouds, so running the
            // 4-octave simplex warp at 256x256 RGBA8 (instead of full-window RGBA16F) produces
            // identical creamy gradients at 16x-36x lower GPU fill cost AND lets the sidebar
            // backdrop + all GlassCards share ONE cached warp pass per (source, phase).
            let warp_size = [256, 256];
            let target = &mut self.targets[0];
            let mut allocated = false;
            if target.as_ref().is_none_or(|target| target.2 != warp_size) {
                let texture = gl.create_texture()?;
                let framebuffer = match gl.create_framebuffer() {
                    Ok(framebuffer) => framebuffer,
                    Err(error) => { gl.delete_texture(texture); return Err(error); }
                };
                gl.bind_texture(glow::TEXTURE_2D, Some(texture));
                gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_MIN_FILTER, glow::LINEAR as i32);
                gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_MAG_FILTER, glow::LINEAR as i32);
                gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_WRAP_S, glow::CLAMP_TO_EDGE as i32);
                gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_WRAP_T, glow::CLAMP_TO_EDGE as i32);
                gl.tex_image_2d(glow::TEXTURE_2D, 0, glow::RGBA8 as i32, warp_size[0], warp_size[1], 0, glow::RGBA, glow::UNSIGNED_BYTE, glow::PixelUnpackData::Slice(None));
                gl.bind_framebuffer(glow::FRAMEBUFFER, Some(framebuffer));
                gl.framebuffer_texture_2d(glow::FRAMEBUFFER, glow::COLOR_ATTACHMENT0, glow::TEXTURE_2D, Some(texture), 0);
                if gl.check_framebuffer_status(glow::FRAMEBUFFER) != glow::FRAMEBUFFER_COMPLETE {
                    gl.delete_framebuffer(framebuffer);
                    gl.delete_texture(texture);
                    return Err("RGBA8 warp framebuffer is incomplete".into());
                }
                if let Some((old_texture, old_framebuffer, _, _)) = target.replace((texture, framebuffer, warp_size, tick)) {
                    gl.delete_framebuffer(old_framebuffer);
                    gl.delete_texture(old_texture);
                }
                allocated = true;
            } else if let Some(entry) = target.as_mut() {
                entry.3 = tick;
            }
            let Some((texture, framebuffer, _, _)) = *target else { return Ok(()) };
            gl.bind_vertex_array(Some(self.vao));
            let warp_key = (source, phase.to_bits());
            if allocated || self.last_warp != Some(warp_key) {
                gl.disable(glow::SCISSOR_TEST);
                gl.disable(glow::BLEND);
                gl.bind_framebuffer(glow::FRAMEBUFFER, Some(framebuffer));
                gl.viewport(0, 0, warp_size[0], warp_size[1]);
                gl.use_program(Some(self.warp));
                gl.bind_texture(glow::TEXTURE_2D, Some(source));
                gl.uniform_1_i32(self.warp_locs.texture.as_ref(), 0);
                gl.uniform_1_f32(self.warp_locs.time.as_ref(), phase);
                gl.uniform_1_f32(self.warp_locs.intensity.as_ref(), 1.0);
                gl.draw_arrays(glow::TRIANGLES, 0, 3);
                self.last_warp = Some(warp_key);
            }

            gl.bind_framebuffer(glow::FRAMEBUFFER, painter.intermediate_fbo());
            gl.viewport(saved.viewport[0], saved.viewport[1], size[0], size[1]);
            if saved.scissor { gl.enable(glow::SCISSOR_TEST); } else { gl.disable(glow::SCISSOR_TEST); }
            if saved.blend { gl.enable(glow::BLEND); } else { gl.disable(glow::BLEND); }
            gl.use_program(Some(self.output));
            gl.bind_texture(glow::TEXTURE_2D, Some(texture));
            gl.uniform_1_i32(self.output_locs.texture.as_ref(), 0);
            gl.uniform_1_f32(self.output_locs.time.as_ref(), phase);
            gl.uniform_1_f32(self.output_locs.saturation.as_ref(), 1.5);
            gl.uniform_1_f32(self.output_locs.dithering.as_ref(), 0.008);
            gl.uniform_1_f32(self.output_locs.scale.as_ref(), 1.0);
            gl.uniform_1_f32(self.output_locs.alpha.as_ref(), alpha);
            gl.uniform_1_f32(self.output_locs.sidebar.as_ref(), if sidebar { 1.0 } else { 0.0 });
            gl.uniform_1_f32(self.output_locs.frost.as_ref(), frost_uv);
            gl.uniform_1_f32(self.output_locs.tint.as_ref(), tint);
            gl.uniform_4_f32(
                self.output_locs.uv_rect.as_ref(),
                uv_rect[0],
                uv_rect[1],
                uv_rect[2],
                uv_rect[3],
            );
            let fade = header_fade.unwrap_or([-2.0, -1.0]);
            gl.uniform_1_f32(self.output_locs.radius.as_ref(), radius);
            gl.uniform_2_f32(self.output_locs.header_fade.as_ref(), fade[0], fade[1]);
            gl.uniform_2_f32(self.output_locs.resolution.as_ref(), size[0] as f32, size[1] as f32);
            gl.draw_arrays(glow::TRIANGLES, 0, 3);
        }
        Ok(())
    }
}

unsafe fn program(gl: &glow::Context, fragment: &str) -> Result<glow::Program, String> {
    // SAFETY: shader compilation and deletion use the current callback context.
    unsafe {
        let program = gl.create_program()?;
        let version = if gl.version().is_embedded { "#version 300 es\nprecision highp float;\n" } else { "#version 330 core\n#define highp\n" };
        for (kind, source) in [(glow::VERTEX_SHADER, include_str!("backdrop/fullscreen.vert")), (glow::FRAGMENT_SHADER, fragment)] {
            let shader = match gl.create_shader(kind) {
                Ok(shader) => shader,
                Err(error) => { gl.delete_program(program); return Err(error); }
            };
            gl.shader_source(shader, &format!("{version}{source}"));
            gl.compile_shader(shader);
            if !gl.get_shader_compile_status(shader) {
                let error = gl.get_shader_info_log(shader);
                gl.delete_shader(shader);
                gl.delete_program(program);
                return Err(error);
            }
            gl.attach_shader(program, shader);
            gl.delete_shader(shader);
        }
        gl.link_program(program);
        if !gl.get_program_link_status(program) {
            let error = gl.get_program_info_log(program);
            gl.delete_program(program);
            return Err(error);
        }
        Ok(program)
    }
}

struct GlState {
    viewport: [i32; 4],
    framebuffer: Option<glow::Framebuffer>,
    read_framebuffer: Option<glow::Framebuffer>,
    program: Option<glow::Program>,
    vao: Option<glow::VertexArray>,
    active_texture: u32,
    texture0: Option<glow::Texture>,
    blend: bool,
    scissor: bool,
}

impl GlState {
    unsafe fn save(gl: &glow::Context) -> Self {
        // SAFETY: queried on the callback's current context, before any offscreen rendering.
        unsafe {
            let mut viewport = [0; 4];
            gl.get_parameter_i32_slice(glow::VIEWPORT, &mut viewport);
            let active_texture = gl.get_parameter_i32(glow::ACTIVE_TEXTURE) as u32;
            gl.active_texture(glow::TEXTURE0);
            Self { viewport, active_texture,
                framebuffer: gl.get_parameter_framebuffer(glow::DRAW_FRAMEBUFFER_BINDING),
                read_framebuffer: gl.get_parameter_framebuffer(glow::READ_FRAMEBUFFER_BINDING),
                program: gl.get_parameter_program(glow::CURRENT_PROGRAM),
                vao: gl.get_parameter_vertex_array(glow::VERTEX_ARRAY_BINDING),
                texture0: gl.get_parameter_texture(glow::TEXTURE_BINDING_2D),
                blend: gl.is_enabled(glow::BLEND), scissor: gl.is_enabled(glow::SCISSOR_TEST) }
        }
    }

    unsafe fn restore(&self, gl: &glow::Context) {
        // SAFETY: saved bindings belong to this same, still-current callback context.
        unsafe {
            gl.bind_framebuffer(glow::DRAW_FRAMEBUFFER, self.framebuffer);
            gl.bind_framebuffer(glow::READ_FRAMEBUFFER, self.read_framebuffer);
            gl.viewport(self.viewport[0], self.viewport[1], self.viewport[2], self.viewport[3]);
            gl.use_program(self.program);
            gl.bind_vertex_array(self.vao);
            gl.active_texture(glow::TEXTURE0);
            gl.bind_texture(glow::TEXTURE_2D, self.texture0);
            gl.active_texture(self.active_texture);
            for (cap, enabled) in [(glow::BLEND, self.blend), (glow::SCISSOR_TEST, self.scissor)] {
                if enabled { gl.enable(cap); } else { gl.disable(cap); }
            }
        }
    }
}
