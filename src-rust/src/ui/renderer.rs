//! The GPU side of a frame.
//!
//! Split in two because Oracle draws into two windows — the application and the tray panel.
//! [`Gpu`] owns everything that can be shared: the instance, the adapter, the device, the
//! queue, the pipelines, and the icon atlas. [`Target`] owns what cannot: one window's
//! surface, its blur pyramid, and its glyph atlas.
//!
//! The split is not tidiness. A second device costs another 140 MB from the graphics driver
//! on this machine — measured — which is most of what the rewrite was for.
//!
//! A frame is four passes, in this order:
//!
//! 1. the **wash**, into the glass renderer's own target so it can be blurred
//! 2. the **glass**, refracting that blur, then the flat fills that sit on it
//! 3. the **icons**, from a shared coverage atlas
//! 4. the **text**

use glisten_glass::{Blur, GlassRenderer};

use super::icons::IconAtlas;
use super::paint::Frame;
use super::text::TextLayer;
use super::theme::Palette;

const WASH: &str = include_str!("shaders/wash.wgsl");
const ICON: &str = include_str!("shaders/icon.wgsl");

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct WashUniforms {
    resolution: [f32; 2],
    time: f32,
    intensity: f32,
    paper: [f32; 4],
    accent: [f32; 4],
    counter: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct IconGlobals {
    resolution: [f32; 2],
    _pad: [f32; 2],
}

/// One icon's instance data. Four `vec4`s, 64 bytes.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct IconInstance {
    placement: [f32; 4],
    uv: [f32; 4],
    colour: [f32; 4],
    bounds: [f32; 4],
}

impl IconInstance {
    const LAYOUT: wgpu::VertexBufferLayout<'static> = wgpu::VertexBufferLayout {
        array_stride: std::mem::size_of::<IconInstance>() as wgpu::BufferAddress,
        step_mode: wgpu::VertexStepMode::Instance,
        attributes: &wgpu::vertex_attr_array![
            0 => Float32x4,
            1 => Float32x4,
            2 => Float32x4,
            3 => Float32x4,
        ],
    };
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

    icon_pipeline: wgpu::RenderPipeline,
    icon_layout: wgpu::BindGroupLayout,
    icon_sampler: wgpu::Sampler,
    /// One atlas for both windows: the same twenty icons at the same sizes serve each, and a
    /// second copy would be a second megabyte for nothing.
    pub icons: IconAtlas,
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

        let (wash, wash_layout) = build_wash(&device, format);
        let (icon_pipeline, icon_layout) = build_icons(&device, format);

        let icon_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("icon sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        let icons = IconAtlas::new(&device);

        let gpu = Self {
            instance,
            adapter,
            device,
            queue,
            format,
            wash,
            wash_layout,
            icon_pipeline,
            icon_layout,
            icon_sampler,
            icons,
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

        let wash_uniforms = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("wash uniforms"),
            size: std::mem::size_of::<WashUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let wash_bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("wash bind group"),
            layout: &self.wash_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wash_uniforms.as_entire_binding(),
            }],
        });

        let icon_globals = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("icon globals"),
            size: std::mem::size_of::<IconGlobals>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let icon_bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("icon bind group"),
            layout: &self.icon_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: icon_globals.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&self.icons.view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&self.icon_sampler),
                },
            ],
        });

        Target {
            surface,
            config,
            wash_uniforms,
            wash_bind_group,
            icon_globals,
            icon_bind_group,
            icon_instances: self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("icon instances"),
                size: 64 * 64,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
            icon_capacity: 64,
            glass: GlassRenderer::new(&self.device, self.format, width, height),
            text: TextLayer::new(&self.device, &self.queue, self.format, scale),
        }
    }
}

/// One window's drawing surface.
pub struct Target {
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,

    wash_uniforms: wgpu::Buffer,
    wash_bind_group: wgpu::BindGroup,

    icon_globals: wgpu::Buffer,
    icon_bind_group: wgpu::BindGroup,
    icon_instances: wgpu::Buffer,
    icon_capacity: usize,

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
    ///
    /// `wash` scales how far the backdrop departs from flat paper. The tray panel passes
    /// less than the main window: the same pools spread across 380 points read as a stripe
    /// rather than as a wash.
    pub fn render(
        &mut self,
        gpu: &mut Gpu,
        frame: &Frame,
        palette: &Palette,
        time: f32,
        wash: f32,
    ) -> bool {
        let (width, height) = self.size();

        gpu.queue.write_buffer(
            &self.wash_uniforms,
            0,
            bytemuck::bytes_of(&WashUniforms {
                resolution: [width as f32, height as f32],
                time,
                intensity: wash,
                paper: palette.paper,
                accent: [
                    palette.accent[0],
                    palette.accent[1],
                    palette.accent[2],
                    palette.wash,
                ],
                // A cool violet, well off the brand hue. It is never identifiable as a colour
                // at this strength; what it does is give the rim something to disperse and
                // the warm side something to be warm against.
                counter: [0.34, 0.30, 0.52, palette.wash * 0.85],
            }),
        );

        // Icons are resolved here rather than at layout time because rasterising one needs
        // the queue, and a screen should not have to hold a GPU handle to draw a button.
        let instances = self.build_icons(gpu, frame);

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
            pass.set_bind_group(0, &self.wash_bind_group, &[]);
            pass.draw(0..3, 0..1);
        }

        // 2. Blur it, so the glass has a backdrop to read.
        self.glass.prepare(&gpu.device, &gpu.queue, &mut encoder);

        // 3. The sharp wash to the screen, then glass, then flat fills.
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

        // 4. Icons, then text. Both read the target and neither writes anything the other
        // needs, so they share one load-preserving pass each.
        if !instances.is_empty() {
            gpu.queue.write_buffer(
                &self.icon_globals,
                0,
                bytemuck::bytes_of(&IconGlobals {
                    resolution: [width as f32, height as f32],
                    _pad: [0.0; 2],
                }),
            );
            gpu.queue
                .write_buffer(&self.icon_instances, 0, bytemuck::cast_slice(&instances));

            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("icon pass"),
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
            pass.set_pipeline(&gpu.icon_pipeline);
            pass.set_bind_group(0, &self.icon_bind_group, &[]);
            pass.set_vertex_buffer(0, self.icon_instances.slice(..));
            pass.draw(0..4, 0..instances.len() as u32);
        }

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

    /// Turns this frame's icon requests into instance data, rasterising anything new.
    ///
    /// Growing the instance buffer here rather than per frame: a screen's icon count is
    /// stable, so after the first frame of a screen this reallocates nothing.
    fn build_icons(&mut self, gpu: &mut Gpu, frame: &Frame) -> Vec<IconInstance> {
        let (width, height) = self.size();
        let mut instances = Vec::with_capacity(frame.icons.len());

        for draw in &frame.icons {
            let Some(placement) = gpu
                .icons
                .place(&gpu.queue, draw.icon, draw.size, draw.filled)
            else {
                continue;
            };

            let bounds = draw.clip.unwrap_or([0.0, 0.0, width as f32, height as f32]);

            instances.push(IconInstance {
                // The rasterised size, not the requested one: asking for 15.3 pixels gives a
                // 15-pixel bitmap, and drawing it at 15.3 resamples a crisp edge into a soft
                // one for a third of a pixel of accuracy nobody can see.
                placement: [draw.centre[0], draw.centre[1], placement.size, draw.rotation],
                uv: placement.uv,
                colour: draw.colour,
                bounds: [
                    bounds[0],
                    bounds[1],
                    bounds[0] + bounds[2],
                    bounds[1] + bounds[3],
                ],
            });
        }

        if instances.len() > self.icon_capacity {
            self.icon_capacity = instances.len().next_power_of_two();
            self.icon_instances = gpu.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("icon instances"),
                size: (self.icon_capacity * std::mem::size_of::<IconInstance>()) as u64,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            self.icon_bind_group = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("icon bind group"),
                layout: &gpu.icon_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: self.icon_globals.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(&gpu.icons.view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::Sampler(&gpu.icon_sampler),
                    },
                ],
            });
        }

        instances
    }
}

fn build_wash(
    device: &wgpu::Device,
    format: wgpu::TextureFormat,
) -> (wgpu::RenderPipeline, wgpu::BindGroupLayout) {
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("wash"),
        source: wgpu::ShaderSource::Wgsl(WASH.into()),
    });

    let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
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
        bind_group_layouts: &[Some(&layout)],
        immediate_size: 0,
    });

    let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
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

    (pipeline, layout)
}

fn build_icons(
    device: &wgpu::Device,
    format: wgpu::TextureFormat,
) -> (wgpu::RenderPipeline, wgpu::BindGroupLayout) {
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("icons"),
        source: wgpu::ShaderSource::Wgsl(ICON.into()),
    });

    let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("icon layout"),
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 2,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                count: None,
            },
        ],
    });

    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("icon pipeline layout"),
        bind_group_layouts: &[Some(&layout)],
        immediate_size: 0,
    });

    let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("icons"),
        layout: Some(&pipeline_layout),
        vertex: wgpu::VertexState {
            module: &module,
            entry_point: Some("vs"),
            compilation_options: Default::default(),
            buffers: &[Some(IconInstance::LAYOUT)],
        },
        fragment: Some(wgpu::FragmentState {
            module: &module,
            entry_point: Some("fs"),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format,
                blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleStrip,
            ..Default::default()
        },
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
        cache: None,
    });

    (pipeline, layout)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_icon_instance_layout_matches_the_shader() {
        // Four vec4<f32>. A mismatch reads as icons in the wrong place rather than as an
        // error, and the wrong place is usually off screen.
        assert_eq!(std::mem::size_of::<IconInstance>(), 64);
        assert_eq!(IconInstance::LAYOUT.array_stride, 64);
        assert_eq!(IconInstance::LAYOUT.attributes.len(), 4);
    }

    #[test]
    fn the_wash_uniforms_are_a_shape_the_shader_can_read() {
        // WGSL aligns a uniform struct to 16 bytes. A size that is not a multiple of that is
        // rejected at pipeline creation with a message about binding sizes rather than about
        // the field that caused it.
        assert_eq!(std::mem::size_of::<WashUniforms>() % 16, 0);
        assert_eq!(std::mem::size_of::<IconGlobals>() % 16, 0);
    }
}
