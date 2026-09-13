//! The GPU side of a frame.
//!
//! Split in two because Oracle draws into two windows — the application and the tray panel.
//! [`Gpu`] owns everything that can be shared: the instance, the adapter, the device, the
//! queue, and the pipelines, which depend only on the surface format. [`Target`] owns what
//! cannot: one window's surface, its blur pyramid, and its glyph atlas.
//!
//! The split is not tidiness. A second device costs another 140 MB from the graphics driver
//! on this machine — measured — which is most of what the rewrite was for.

use glisten_glass::{Blur, GlassRenderer};

use super::paint::Frame;
use super::text::TextLayer;
use super::theme::Palette;

/// The ground the glass refracts against.
///
/// Two soft accent-coloured pools in opposite corners, which is what the stylesheet drew
/// with `radial-gradient`. Glass over a flat colour is invisible; it needs variation to bend.
const WASH: &str = r#"
struct Uniforms {
    resolution: vec2<f32>,
    time: f32,
    _pad: f32,
    paper: vec4<f32>,
    accent: vec4<f32>,
}

@group(0) @binding(0) var<uniform> u: Uniforms;

struct VertexOutput {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

@vertex
fn vs(@builtin(vertex_index) index: u32) -> VertexOutput {
    var out: VertexOutput;
    let uv = vec2<f32>(f32((index << 1u) & 2u), f32(index & 2u));
    out.uv = uv;
    out.clip = vec4<f32>(uv * vec2<f32>(2.0, -2.0) + vec2<f32>(-1.0, 1.0), 0.0, 1.0);
    return out;
}

fn pool(p: vec2<f32>, centre: vec2<f32>, radius: f32) -> f32 {
    let d = length(p - centre) / radius;
    return exp(-d * d * 1.6);
}

@fragment
fn fs(in: VertexOutput) -> @location(0) vec4<f32> {
    let aspect = u.resolution.x / max(u.resolution.y, 1.0);
    let p = vec2<f32>(in.uv.x * aspect, in.uv.y);

    // A very slow drift, so the glass has something alive behind it without anything on
    // screen appearing to move.
    let t = u.time * 0.04;

    var colour = u.paper.rgb;
    colour += u.accent.rgb * u.accent.a *
        pool(p, vec2<f32>(0.12 * aspect + sin(t) * 0.05, -0.08 + cos(t * 0.8) * 0.04), 0.75);
    colour += u.accent.rgb * u.accent.a * 0.85 *
        pool(p, vec2<f32>(1.02 * aspect + cos(t * 0.7) * 0.05, 1.06 + sin(t) * 0.04), 0.70);

    return vec4<f32>(colour, 1.0);
}
"#;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct WashUniforms {
    resolution: [f32; 2],
    time: f32,
    _pad: f32,
    paper: [f32; 4],
    accent: [f32; 4],
}

/// Everything both windows share.
pub struct Gpu {
    instance: wgpu::Instance,
    adapter: wgpu::Adapter,
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub format: wgpu::TextureFormat,

    wash: wgpu::RenderPipeline,
    wash_layout: wgpu::BindGroupLayout,
}

impl Gpu {
    /// Opens a device compatible with `window`, and builds the first target for it.
    pub async fn new(
        window: std::sync::Arc<winit::window::Window>,
        scale: f32,
    ) -> Result<(Self, Target), String> {
        let size = window.inner_size();
        let (width, height) = (size.width.max(1), size.height.max(1));

        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let surface = instance
            .create_surface(window)
            .map_err(|e| format!("could not create a drawing surface: {e}"))?;

        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::LowPower,
                compatible_surface: Some(&surface),
                force_fallback_adapter: false,
                ..Default::default()
            })
            .await
            .map_err(|e| format!("no usable graphics adapter: {e}"))?;

        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("oracle"),
                // Measured on an AMD 860M with ORACLE_DIAG=1: the default Performance hint
                // costs 307 MB at device creation and this costs 127 MB. An interface
                // allocates a handful of small textures, so the large pools the default
                // reserves buy nothing. Sizing the suballocation blocks by hand saved a
                // further 6 MB, which is not worth the magic numbers.
                memory_hints: wgpu::MemoryHints::MemoryUsage,
                // The smallest capability set that still runs the glass shader, so the
                // driver has no reason to size its heaps for a game.
                required_limits: wgpu::Limits::downlevel_defaults(),
                ..Default::default()
            })
            .await
            .map_err(|e| format!("could not open the graphics device: {e}"))?;
        crate::app::probe("after wgpu device");

        let caps = surface.get_capabilities(&adapter);
        let format = caps
            .formats
            .iter()
            .copied()
            .find(|f| !f.is_srgb())
            .unwrap_or(caps.formats[0]);

        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("wash"),
            source: wgpu::ShaderSource::Wgsl(WASH.into()),
        });

        let wash_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("wash layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("wash pipeline layout"),
            bind_group_layouts: &[Some(&wash_layout)],
            immediate_size: 0,
        });

        let wash = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("wash"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vs"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fs"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        let gpu = Self {
            instance,
            adapter,
            device,
            queue,
            format,
            wash,
            wash_layout,
        };

        let target = gpu.build_target(surface, width, height, scale);
        crate::app::probe("after first target");

        Ok((gpu, target))
    }

    /// Adds a second window to the same device.
    pub fn attach(
        &self,
        window: std::sync::Arc<winit::window::Window>,
        scale: f32,
    ) -> Result<Target, String> {
        let size = window.inner_size();
        let (width, height) = (size.width.max(1), size.height.max(1));

        let surface = self
            .instance
            .create_surface(window)
            .map_err(|e| format!("could not create a second surface: {e}"))?;

        Ok(self.build_target(surface, width, height, scale))
    }

    fn build_target(
        &self,
        surface: wgpu::Surface<'static>,
        width: u32,
        height: u32,
        scale: f32,
    ) -> Target {
        let mut config = surface
            .get_default_config(&self.adapter, width, height)
            .expect("the adapter cannot draw to this window");
        config.format = self.format;
        config.usage = wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_DST;
        // Vsync. An interface has no reason to render faster than the display refreshes,
        // and uncapped rendering on a laptop is just heat.
        config.present_mode = wgpu::PresentMode::AutoVsync;
        surface.configure(&self.device, &config);

        let uniforms = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("wash uniforms"),
            size: std::mem::size_of::<WashUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("wash bind group"),
            layout: &self.wash_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniforms.as_entire_binding(),
            }],
        });

        Target {
            surface,
            config,
            uniforms,
            bind_group,
            glass: GlassRenderer::new(&self.device, self.format, width, height),
            text: TextLayer::new(&self.device, &self.queue, self.format, scale),
        }
    }
}

/// One window's drawing surface.
pub struct Target {
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    uniforms: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    glass: GlassRenderer,
    pub text: TextLayer,
}

impl Target {
    pub fn size(&self) -> (u32, u32) {
        (self.config.width, self.config.height)
    }

    pub fn resize(&mut self, gpu: &Gpu, width: u32, height: u32) {
        if width == 0 || height == 0 || (width, height) == self.size() {
            return;
        }
        self.config.width = width;
        self.config.height = height;
        self.surface.configure(&gpu.device, &self.config);
        self.glass.resize(&gpu.device, width, height);
    }

    pub fn set_blur(&mut self, gpu: &Gpu, blur: Blur) {
        self.glass.set_blur(&gpu.device, blur);
    }

    /// Draws one frame. Returns false if the surface was lost and the frame was skipped.
    pub fn render(&mut self, gpu: &Gpu, frame: &Frame, palette: &Palette, time: f32) -> bool {
        let (width, height) = self.size();

        gpu.queue.write_buffer(
            &self.uniforms,
            0,
            bytemuck::bytes_of(&WashUniforms {
                resolution: [width as f32, height as f32],
                time,
                _pad: 0.0,
                paper: palette.paper,
                accent: [
                    palette.accent[0],
                    palette.accent[1],
                    palette.accent[2],
                    if palette.dark { 0.20 } else { 0.16 },
                ],
            }),
        );

        let surface_texture = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(t)
            | wgpu::CurrentSurfaceTexture::Suboptimal(t) => t,
            _ => {
                self.surface.configure(&gpu.device, &self.config);
                return false;
            }
        };
        let view = surface_texture
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());

        let mut encoder = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("frame") });

        // 1. The wash, into the glass renderer's own target so it can be blurred.
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("wash pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: self.glass.scene_view(),
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&gpu.wash);
            pass.set_bind_group(0, &self.bind_group, &[]);
            pass.draw(0..3, 0..1);
        }

        // 2. Blur it, so the glass has a backdrop to read.
        self.glass.prepare(&gpu.device, &gpu.queue, &mut encoder);

        // 3. The sharp wash to the screen, then glass, then flat fills, then text.
        encoder.copy_texture_to_texture(
            self.glass.scene_texture().as_image_copy(),
            surface_texture.texture.as_image_copy(),
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );

        self.glass
            .draw(&gpu.device, &gpu.queue, &mut encoder, &view, &frame.glass);
        self.glass
            .draw(&gpu.device, &gpu.queue, &mut encoder, &view, &frame.solid);

        self.text
            .prepare(&gpu.device, &gpu.queue, width, height, &frame.text);

        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("text pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            self.text.render(&mut pass);
        }

        gpu.queue.submit(Some(encoder.finish()));
        gpu.queue.present(surface_texture);
        self.text.trim();

        true
    }
}
