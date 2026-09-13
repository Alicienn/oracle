// The ground the glass refracts against.
//
// Glass over a flat colour is invisible. Whatever the shader in `glisten-glass` does — bend,
// disperse, compress — it can only reveal detail that was already there, and a constant has
// none. The first native build drew two faint accent pools over the paper colour and the
// result was very nearly flat, which is most of why its glass read as tinted rectangles.
//
// So this is deliberately busy, at a scale large enough that nothing on screen appears to
// move and nothing competes with the interface for attention:
//
//   - four pools of accent light, at different sizes, drifting on incommensurable periods so
//     the pattern never visibly repeats
//   - a cool counter-light, because a ground made only of the accent leaves the glass nothing
//     to disperse and the rim fringes stay invisible
//   - a soft vignette, so the corners sit back and the middle carries the content
//   - a diagonal sheen that sweeps very slowly, which is what gives a still window the
//     impression that a light source exists somewhere off screen
//   - ordered dither, without which a dark gradient bands into visible steps at 8 bits

struct Uniforms {
    resolution: vec2<f32>,
    time: f32,
    // How strongly the wash departs from flat paper. A small window wants less.
    intensity: f32,
    paper: vec4<f32>,
    accent: vec4<f32>,
    // The cool counter-light. Alpha is its strength.
    counter: vec4<f32>,
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

// A soft circular pool of light. Gaussian rather than a smoothstep, because a smoothstep has
// a discontinuous second derivative and the seam shows up as a faint ring once the glass
// magnifies it.
fn pool(p: vec2<f32>, centre: vec2<f32>, radius: f32) -> f32 {
    let d = length(p - centre) / radius;
    return exp(-d * d * 1.55);
}

// A cheap value-noise hash. Used only for dither, where the quality bar is "not correlated
// with anything".
fn hash(p: vec2<f32>) -> f32 {
    let h = dot(p, vec2<f32>(127.1, 311.7));
    return fract(sin(h) * 43758.5453123);
}

@fragment
fn fs(in: VertexOutput) -> @location(0) vec4<f32> {
    let aspect = u.resolution.x / max(u.resolution.y, 1.0);
    let p = vec2<f32>(in.uv.x * aspect, in.uv.y);

    // Slow enough that a still screenshot and the live window look identical, fast enough
    // that a specular highlight crawls along a rim over about a minute.
    let t = u.time * 0.035;

    var light = 0.0;
    // Periods chosen to share no common multiple, so the arrangement never returns to a
    // state the eye could recognise as a loop.
    light += 1.00 * pool(p, vec2<f32>(0.10 * aspect + sin(t) * 0.06, -0.10 + cos(t * 0.83) * 0.05), 0.78);
    light += 0.80 * pool(p, vec2<f32>(1.04 * aspect + cos(t * 0.71) * 0.06, 1.08 + sin(t * 1.11) * 0.05), 0.72);
    light += 0.45 * pool(p, vec2<f32>(0.72 * aspect + sin(t * 0.47) * 0.09, 0.28 + cos(t * 0.61) * 0.07), 0.42);
    light += 0.30 * pool(p, vec2<f32>(0.24 * aspect + cos(t * 0.29) * 0.08, 0.86 + sin(t * 0.37) * 0.06), 0.36);

    // A diagonal sheen crossing the whole window. This is the part the rim of a large panel
    // picks up as it refracts, and the reason a static-looking window still has life in it.
    let sweep = sin((p.x * 0.6 + p.y * 0.9) * 2.2 - u.time * 0.10) * 0.5 + 0.5;

    var colour = u.paper.rgb;
    colour += u.accent.rgb * u.accent.a * light * u.intensity;
    colour += u.accent.rgb * u.accent.a * 0.22 * sweep * sweep * u.intensity;

    // The counter-light. One hue across a whole backdrop gives dispersion nothing to separate
    // and a tinted panel nothing to sit against, so the far corner runs cool.
    let cool = pool(p, vec2<f32>(0.92 * aspect + sin(t * 0.53) * 0.05, -0.02 + cos(t * 0.67) * 0.05), 0.62);
    colour += u.counter.rgb * u.counter.a * cool * u.intensity;

    // Vignette. Measured from the centre in aspect-corrected space so it stays circular on a
    // wide window instead of pinching the sides.
    let centred = (in.uv - vec2<f32>(0.5)) * vec2<f32>(aspect, 1.0);
    let vignette = 1.0 - smoothstep(0.35, 0.95, length(centred)) * 0.14;
    colour *= vignette;

    // Ordered dither, one 256th of a step, keyed on pixel position and time. A gradient this
    // shallow across a dark ground quantises into visible bands at 8 bits per channel, and
    // the glass blur widens each band rather than hiding it. Noise below the quantisation
    // step converts the banding into grain the eye reads as texture.
    let grain = (hash(in.clip.xy + vec2<f32>(u.time)) - 0.5) / 255.0;

    return vec4<f32>(colour + vec3<f32>(grain), 1.0);
}
