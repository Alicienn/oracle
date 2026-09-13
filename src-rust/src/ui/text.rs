//! Text rendering.
//!
//! Shaping, layout and rasterisation come from `cosmic-text` through `glyphon` — the one
//! part of a browser genuinely not worth rewriting. Everything here is the layer that turns
//! Oracle's idea of a line of text into what glyphon wants.

use glyphon::cosmic_text::{FeatureTag, FontFeatures};
use glyphon::{
    fontdb, Attrs, Buffer, Cache, Family, FontSystem, Metrics, Resolution, Shaping, SwashCache,
    TextArea, TextAtlas, TextBounds, TextRenderer, Viewport, Weight, Wrap,
};

use super::theme::Colour;

/// Where a run sits relative to the x it was given.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Align {
    Left,
    Centre,
    Right,
}

/// One piece of text to draw this frame.
#[derive(Debug, Clone)]
pub struct Run {
    pub text: String,
    pub x: f32,
    pub y: f32,
    pub size: f32,
    pub colour: Colour,
    pub weight: u16,
    /// Wrap and clip to this width. Without it the run is a single unwrapped line.
    pub width: Option<f32>,
    pub align: Align,
    /// Clip rectangle in physical pixels, so text inside a scrolling pane cannot escape it.
    pub clip: Option<[f32; 4]>,
    pub monospace: bool,
}

impl Run {
    pub fn new(text: impl Into<String>, x: f32, y: f32, size: f32, colour: Colour) -> Self {
        Self {
            text: text.into(),
            x,
            y,
            size,
            colour,
            weight: 400,
            width: None,
            align: Align::Left,
            clip: None,
            monospace: false,
        }
    }

    pub fn weight(mut self, weight: u16) -> Self {
        self.weight = weight;
        self
    }

    pub fn width(mut self, width: f32) -> Self {
        self.width = Some(width);
        self
    }

    pub fn align(mut self, align: Align) -> Self {
        self.align = align;
        self
    }

    pub fn clip(mut self, clip: [f32; 4]) -> Self {
        self.clip = Some(clip);
        self
    }

    pub fn monospace(mut self) -> Self {
        self.monospace = true;
        self
    }
}

/// Owns the font stack and draws a frame's worth of text.
pub struct TextLayer {
    fonts: FontSystem,
    swash: SwashCache,
    atlas: TextAtlas,
    viewport: Viewport,
    renderer: TextRenderer,

    /// Buffers are pooled rather than allocated per run: shaping is the expensive part and
    /// a frame typically reuses the same shapes.
    buffers: Vec<Buffer>,
    scale: f32,
}

impl TextLayer {
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        format: wgpu::TextureFormat,
        scale: f32,
    ) -> Self {
        let cache = Cache::new(device);
        let mut atlas = TextAtlas::new(device, queue, &cache, format);
        crate::app::probe("after atlas");
        let renderer =
            TextRenderer::new(&mut atlas, device, wgpu::MultisampleState::default(), None);

        crate::app::probe("before fonts");
        let fonts = load_fonts();
        crate::app::probe("after fonts");

        Self {
            fonts,
            swash: SwashCache::new(),
            atlas,
            viewport: Viewport::new(device, &cache),
            renderer,
            buffers: Vec::new(),
            scale,
        }
    }

    pub fn set_scale(&mut self, scale: f32) {
        self.scale = scale;
    }

    /// Measures a line without drawing it, for layout that has to fit around text.
    pub fn measure(&mut self, text: &str, size: f32, weight: u16, monospace: bool) -> f32 {
        let mut buffer = Buffer::new(&mut self.fonts, Metrics::new(size, size * 1.4));
        buffer.set_wrap(Wrap::None);
        buffer.set_text(text, &attrs(weight, monospace), Shaping::Advanced, None);
        buffer.shape_until_scroll(&mut self.fonts, false);

        buffer
            .layout_runs()
            .fold(0.0f32, |widest, run| widest.max(run.line_w))
    }

    /// Shapes and uploads a frame's text. Call once per frame, before [`Self::render`].
    pub fn prepare(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        width: u32,
        height: u32,
        runs: &[Run],
    ) {
        self.viewport.update(queue, Resolution { width, height });

        // Grow the pool, never shrink it: a list that scrolls back and forth would otherwise
        // reallocate every frame.
        while self.buffers.len() < runs.len() {
            self.buffers
                .push(Buffer::new(&mut self.fonts, Metrics::new(14.0, 20.0)));
        }

        for (buffer, run) in self.buffers.iter_mut().zip(runs) {
            let size = run.size * self.scale;
            buffer.set_metrics(Metrics::new(size, size * 1.45));
            buffer.set_wrap(if run.width.is_some() {
                Wrap::WordOrGlyph
            } else {
                Wrap::None
            });
            buffer.set_size(run.width.map(|w| w * self.scale), Some(height as f32));
            buffer.set_text(
                &run.text,
                &attrs(run.weight, run.monospace),
                Shaping::Advanced,
                None,
            );
            buffer.shape_until_scroll(&mut self.fonts, false);
        }

        // Alignment needs the shaped width, so the offset is computed after shaping.
        let areas: Vec<TextArea> = self
            .buffers
            .iter()
            .zip(runs)
            .map(|(buffer, run)| {
                let measured = buffer
                    .layout_runs()
                    .fold(0.0f32, |widest, line| widest.max(line.line_w));

                let left = match run.align {
                    Align::Left => run.x * self.scale,
                    Align::Centre => run.x * self.scale - measured * 0.5,
                    Align::Right => run.x * self.scale - measured,
                };

                let bounds = run
                    .clip
                    .map(|c| TextBounds {
                        left: (c[0] * self.scale) as i32,
                        top: (c[1] * self.scale) as i32,
                        right: ((c[0] + c[2]) * self.scale) as i32,
                        bottom: ((c[1] + c[3]) * self.scale) as i32,
                    })
                    .unwrap_or(TextBounds {
                        left: 0,
                        top: 0,
                        right: width as i32,
                        bottom: height as i32,
                    });

                TextArea {
                    buffer,
                    left,
                    top: run.y * self.scale,
                    scale: 1.0,
                    bounds,
                    default_color: colour(run.colour),
                    custom_glyphs: &[],
                }
            })
            .collect();

        // A failure here means the glyph atlas is full, which is recoverable: the frame
        // simply draws without the text that did not fit rather than taking the app down.
        let _ = self.renderer.prepare(
            device,
            queue,
            &mut self.fonts,
            &mut self.atlas,
            &self.viewport,
            areas,
            &mut self.swash,
        );
    }

    pub fn render(&self, pass: &mut wgpu::RenderPass<'_>) {
        let _ = self.renderer.render(&self.atlas, &self.viewport, pass);
    }

    /// Releases atlas pages that nothing referenced this frame.
    pub fn trim(&mut self) {
        self.atlas.trim();
    }
}

/// The typefaces Oracle draws with, embedded in the executable.
///
/// The first native build asked Windows for Segoe UI Variable and fell back through whatever
/// else happened to be installed. That is two problems. The obvious one is that the app looks
/// different on different machines, and on a machine missing the Windows 11 face it falls
/// back to Segoe UI, whose smaller x-height and looser spacing make the dense metric readouts
/// noticeably harder to read. The subtler one is that Segoe UI was drawn for reading prose in
/// Windows chrome — it has no tabular figures on by default, so a CPU percentage updating
/// four times a second jitters sideways as its digits change width.
///
/// So the fonts ship with the binary instead:
///
/// - **Inter** (SIL Open Font Licence) for the interface. Designed for screens at small
///   sizes, with a tall x-height and — switched on here — tabular figures, which is what
///   stops a live number dancing.
/// - **JetBrains Mono** (SIL Open Font Licence) for logs and paths.
///
/// Both are subset to Latin, Latin Extended-A, punctuation and arrows, which takes the four
/// Inter cuts from 1.6 MB to 247 KB and JetBrains Mono from 270 KB to 21 KB. Licences are
/// beside the files in `assets/fonts`.
///
/// # Why not the variable font
///
/// `InterVariable.ttf` is one 880 KB file covering every weight, which sounds strictly
/// better. It is not, here: `fontdb` registers a variable face at its default instance, so
/// asking for weight 600 gets weight 400 drawn and the interface loses its whole typographic
/// hierarchy silently. Four static cuts are 247 KB and actually have the weights.
const UI_FONTS: &[(&str, &[u8])] = &[
    ("Inter Regular", include_bytes!("../../assets/fonts/Inter-Regular.ttf")),
    ("Inter Medium", include_bytes!("../../assets/fonts/Inter-Medium.ttf")),
    ("Inter SemiBold", include_bytes!("../../assets/fonts/Inter-SemiBold.ttf")),
    ("Inter Bold", include_bytes!("../../assets/fonts/Inter-Bold.ttf")),
];

const MONO_FONT: (&str, &[u8]) = (
    "JetBrains Mono",
    include_bytes!("../../assets/fonts/JetBrainsMono-Regular.ttf"),
);

pub const UI_FAMILY: &str = "Inter";
pub const MONO_FAMILY: &str = "JetBrains Mono";

fn load_fonts() -> FontSystem {
    // An empty database, filled only from what is embedded. `FontSystem::new()` loads every
    // font installed on the machine, which measured 343 MB of resident memory here — more
    // than the rest of the application put together, for four hundred families nothing draws.
    let mut db = fontdb::Database::new();

    for (_, bytes) in UI_FONTS {
        db.load_font_data(bytes.to_vec());
    }
    db.load_font_data(MONO_FONT.1.to_vec());

    db.set_sans_serif_family(UI_FAMILY);
    db.set_monospace_family(MONO_FAMILY);

    eprintln!("[diag] loaded {} embedded font faces", db.len());

    FontSystem::new_with_locale_and_db("en-US".to_string(), db)
}

fn attrs(weight: u16, monospace: bool) -> Attrs<'static> {
    let family = if monospace {
        Family::Name(MONO_FAMILY)
    } else {
        Family::Name(UI_FAMILY)
    };

    // Tabular figures. Without them a CPU percentage ticking from 9.9 to 10.1 shifts every
    // character after it, and a column of memory readings does not line up — Inter's
    // proportional digits are narrower for 1 than for 0. This is the whole reason the subset
    // keeps the `tnum` feature table.
    let mut features = FontFeatures::new();
    features.enable(FeatureTag::new(b"tnum"));

    Attrs::new()
        .family(family)
        .weight(Weight(weight))
        .font_features(features)
}

fn colour(c: Colour) -> glyphon::Color {
    glyphon::Color::rgba(
        (c[0].clamp(0.0, 1.0) * 255.0) as u8,
        (c[1].clamp(0.0, 1.0) * 255.0) as u8,
        (c[2].clamp(0.0, 1.0) * 255.0) as u8,
        (c[3].clamp(0.0, 1.0) * 255.0) as u8,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_run_defaults_to_a_single_unwrapped_left_aligned_line() {
        let run = Run::new("hello", 10.0, 20.0, 13.0, [1.0, 1.0, 1.0, 1.0]);
        assert_eq!(run.align, Align::Left);
        assert!(run.width.is_none());
        assert!(run.clip.is_none());
        assert_eq!(run.weight, 400);
    }

    #[test]
    fn the_builders_compose() {
        let run = Run::new("x", 0.0, 0.0, 13.0, [0.0; 4])
            .weight(620)
            .width(200.0)
            .align(Align::Right)
            .monospace();

        assert_eq!(run.weight, 620);
        assert_eq!(run.width, Some(200.0));
        assert_eq!(run.align, Align::Right);
        assert!(run.monospace);
    }

    #[test]
    fn colours_survive_the_trip_to_eight_bit() {
        let c = colour([1.0, 0.5, 0.0, 1.0]);
        assert_eq!(c.r(), 255);
        assert_eq!(c.b(), 0);
        assert_eq!(c.a(), 255);
    }

    #[test]
    fn out_of_range_colours_are_clamped_rather_than_wrapping() {
        // A spring overshooting an opacity target would otherwise wrap 1.02 round to near 5.
        let c = colour([1.4, -0.3, 0.5, 1.2]);
        assert_eq!(c.r(), 255);
        assert_eq!(c.g(), 0);
        assert_eq!(c.a(), 255);
    }
}
