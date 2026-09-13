// Icons, drawn from the coverage atlas.
//
// One instanced quad each. The atlas is single-channel, so the fragment shader has coverage
// and nothing else, and the colour comes from the instance — which is what lets one
// rasterisation of the play triangle serve the muted one in a list, the white one on an
// accent button and the accent one on hover, without three copies in the atlas.

struct Globals {
    resolution: vec2<f32>,
    _pad: vec2<f32>,
}

@group(0) @binding(0) var<uniform> globals: Globals;
@group(0) @binding(1) var atlas: texture_2d<f32>;
@group(0) @binding(2) var atlas_sampler: sampler;

struct VertexOutput {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) colour: vec4<f32>,
    // Clip rectangle in physical pixels: left, top, right, bottom.
    @location(2) bounds: vec4<f32>,
}

@vertex
fn vs(
    @builtin(vertex_index) vertex: u32,
    // Centre in physical pixels, box side, clockwise rotation in radians.
    @location(0) placement: vec4<f32>,
    // Atlas coordinates: left, top, right, bottom, in 0..1.
    @location(1) uv: vec4<f32>,
    @location(2) colour: vec4<f32>,
    @location(3) bounds: vec4<f32>,
) -> VertexOutput {
    let corner = vec2<f32>(f32(vertex & 1u), f32((vertex >> 1u) & 1u));

    // Half a pixel of bleed on every side. The rasteriser antialiases up to the edge of its
    // bitmap, and sampling exactly at the boundary loses the outermost half-covered row.
    let half = placement.z * 0.5;
    var offset = (corner * 2.0 - 1.0) * half;

    // Rotation, for the spinner. Around the centre, so an icon that is not square within its
    // box still turns about the point it was placed at.
    let angle = placement.w;
    if angle != 0.0 {
        let c = cos(angle);
        let s = sin(angle);
        offset = vec2<f32>(offset.x * c - offset.y * s, offset.x * s + offset.y * c);
    }

    let position = placement.xy + offset;

    var out: VertexOutput;
    out.uv = mix(uv.xy, uv.zw, corner);
    out.colour = colour;
    out.bounds = bounds;

    let ndc = position / globals.resolution * 2.0 - 1.0;
    out.clip = vec4<f32>(ndc.x, -ndc.y, 0.0, 1.0);
    return out;
}

@fragment
fn fs(in: VertexOutput) -> @location(0) vec4<f32> {
    // Clipping by discard rather than by a scissor rectangle: every icon in a frame is one
    // draw call, and a scissor is per-call. An icon scrolling out of a pane has to stop at
    // the pane's edge, and this is the only place that decision can be made per instance.
    let p = in.clip.xy;
    if p.x < in.bounds.x || p.y < in.bounds.y || p.x > in.bounds.z || p.y > in.bounds.w {
        discard;
    }

    let coverage = textureSample(atlas, atlas_sampler, in.uv).r;
    let alpha = coverage * in.colour.a;
    if alpha <= 0.002 {
        discard;
    }

    return vec4<f32>(in.colour.rgb, alpha);
}
