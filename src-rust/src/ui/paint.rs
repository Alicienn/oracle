//! The draw list for one frame.
//!
//! Screens describe what they want in logical pixels; this collects it, scales it for the
//! display, and hands four ordered layers to the renderer:
//!
//! 1. **glass** — refractive surfaces, composited over the blurred backdrop
//! 2. **solid** — opaque fills that sit *on* the glass: icon tiles, status dots, dividers
//! 3. **icons**
//! 4. **text**
//!
//! Solids go through the same pipeline as glass, with the tint fully opaque and refraction
//! off. One pipeline for both is worth a redundant texture fetch: it means a button and the
//! panel behind it are antialiased by exactly the same code, so their edges match.

use glisten_glass::Surface;

use super::icons::Icon;
use super::text::{Align, Run};
use super::theme::{Colour, GlassStyle};

/// A rectangle in logical pixels: x, y, width, height.
pub type Rect = [f32; 4];

/// One icon to draw. Resolved against the atlas at render time, because that is where the
/// device and queue live.
#[derive(Debug, Clone, Copy)]
pub struct IconDraw {
    pub icon: Icon,
    /// Centre, in physical pixels.
    pub centre: [f32; 2],
    /// Box side, in physical pixels.
    pub size: f32,
    pub colour: Colour,
    /// Clockwise, in radians. Only the spinner uses it, and only it should.
    pub rotation: f32,
    pub filled: bool,
    /// Clip rectangle in physical pixels, or the whole frame.
    pub clip: Option<[f32; 4]>,
}

/// Everything to draw this frame.
pub struct Frame {
    pub glass: Vec<Surface>,
    pub solid: Vec<Surface>,
    pub icons: Vec<IconDraw>,
    pub text: Vec<Run>,
    scale: f32,
    clip: Option<[f32; 4]>,
}

impl Frame {
    pub fn new(scale: f32) -> Self {
        Self {
            glass: Vec::new(),
            solid: Vec::new(),
            icons: Vec::new(),
            text: Vec::new(),
            scale,
            clip: None,
        }
    }

    pub fn clear(&mut self) {
        self.glass.clear();
        self.solid.clear();
        self.icons.clear();
        self.text.clear();
        self.clip = None;
    }

    pub fn scale(&self) -> f32 {
        self.scale
    }

    pub fn set_scale(&mut self, scale: f32) {
        self.scale = scale;
    }

    /// Clips everything drawn until [`Self::clear_clip`] to this rectangle, in logical
    /// pixels. Text and icons honour it; surfaces do not, so a scrolling list draws its rows
    /// within bounds it has already checked.
    pub fn set_clip(&mut self, rect: Rect) {
        self.clip = Some([
            rect[0] * self.scale,
            rect[1] * self.scale,
            rect[2] * self.scale,
            rect[3] * self.scale,
        ]);
    }

    pub fn clear_clip(&mut self) {
        self.clip = None;
    }

    /// A refractive glass surface.
    pub fn panel(&mut self, rect: Rect, style: GlassStyle, opacity: f32) {
        if opacity <= 0.002 || rect[2] <= 0.0 || rect[3] <= 0.0 {
            return;
        }

        self.glass.push(Surface {
            position: [rect[0] * self.scale, rect[1] * self.scale],
            size: [rect[2] * self.scale, rect[3] * self.scale],
            radius: style.radius * self.scale,
            bevel: style.bevel * self.scale,
            refraction: style.refraction * self.scale,
            specular_power: style.specular_power,
            specular: style.specular,
            tint: style.tint,
            inner_shadow: style.inner_shadow,
            saturation: 1.7,
            opacity,
            // The shadow fades with the surface. A panel springing in at 20% opacity over a
            // full-strength shadow reads as a hole rather than as an arrival.
            shadow: style.shadow * opacity,
            shadow_blur: style.shadow_blur * self.scale,
            shadow_drop: style.shadow_drop * self.scale,
            edge: style.edge,
        });
    }

    /// A flat fill. Drawn above the glass, so it is legible on top of it.
    pub fn fill(&mut self, rect: Rect, colour: Colour, radius: f32) {
        self.fill_lifted(rect, colour, radius, 0.0);
    }

    /// A flat fill that casts a shadow, for a control that sits proud of its panel.
    ///
    /// `lift` scales a shadow sized for a small control — a button, a pill, a knob. Anything
    /// large enough to need its own geometry should be a [`Self::panel`] instead.
    pub fn fill_lifted(&mut self, rect: Rect, colour: Colour, radius: f32, lift: f32) {
        if colour[3] <= 0.002 || rect[2] <= 0.0 || rect[3] <= 0.0 {
            return;
        }

        self.solid.push(Surface {
            position: [rect[0] * self.scale, rect[1] * self.scale],
            size: [rect[2] * self.scale, rect[3] * self.scale],
            radius: radius * self.scale,
            // A hairline bevel still antialiases the edge but shows no thickness.
            bevel: 0.5,
            refraction: 0.0,
            specular_power: 40.0,
            specular: 0.0,
            // Fully opaque tint: the shader mixes entirely to this colour and the backdrop
            // sample falls away.
            tint: [colour[0], colour[1], colour[2], 1.0],
            inner_shadow: 0.0,
            saturation: 1.0,
            opacity: colour[3],
            shadow: 0.26 * lift * colour[3],
            shadow_blur: 14.0 * self.scale,
            shadow_drop: 4.0 * self.scale,
            edge: 0.0,
        });
    }

    /// A circle. Convenience for status dots, which are everywhere.
    pub fn dot(&mut self, centre: [f32; 2], diameter: f32, colour: Colour) {
        self.fill(
            [
                centre[0] - diameter * 0.5,
                centre[1] - diameter * 0.5,
                diameter,
                diameter,
            ],
            colour,
            diameter * 0.5,
        );
    }

    /// A soft coloured halo, for the pulse under a running project's status dot.
    pub fn glow(&mut self, centre: [f32; 2], diameter: f32, colour: Colour) {
        if colour[3] <= 0.002 {
            return;
        }
        let d = diameter * self.scale;
        self.solid.push(Surface {
            position: [
                (centre[0] - diameter * 0.5) * self.scale,
                (centre[1] - diameter * 0.5) * self.scale,
            ],
            size: [d, d],
            radius: d * 0.5,
            // The bevel is the whole radius, so the "rim" covers the entire disc and the
            // specular falls off smoothly from the middle. A glow is a shadow made of light.
            bevel: d * 0.5,
            refraction: 0.0,
            specular_power: 1.0,
            specular: 0.0,
            tint: [colour[0], colour[1], colour[2], 1.0],
            inner_shadow: 0.0,
            saturation: 1.0,
            opacity: colour[3],
            shadow: 0.0,
            shadow_blur: 1.0,
            shadow_drop: 0.0,
            edge: 0.0,
        });
    }

    /// A one-pixel rule. Uses the physical pixel grid so it stays crisp at any scale.
    pub fn rule(&mut self, x: f32, y: f32, length: f32, vertical: bool, colour: Colour) {
        let thin = 1.0 / self.scale;
        if vertical {
            self.fill([x, y, thin, length], colour, 0.0);
        } else {
            self.fill([x, y, length, thin], colour, 0.0);
        }
    }

    /// An icon, centred on a point and drawn at `size` logical pixels square.
    pub fn icon(&mut self, icon: Icon, centre: [f32; 2], size: f32, colour: Colour) {
        self.icon_full(icon, centre, size, colour, 0.0, false);
    }

    /// An icon with its paths flooded rather than stroked — play and stop, in practice.
    pub fn icon_solid(&mut self, icon: Icon, centre: [f32; 2], size: f32, colour: Colour) {
        self.icon_full(icon, centre, size, colour, 0.0, true);
    }

    pub fn icon_full(
        &mut self,
        icon: Icon,
        centre: [f32; 2],
        size: f32,
        colour: Colour,
        rotation: f32,
        filled: bool,
    ) {
        if colour[3] <= 0.002 || size <= 0.5 {
            return;
        }
        self.icons.push(IconDraw {
            icon,
            centre: [centre[0] * self.scale, centre[1] * self.scale],
            size: size * self.scale,
            colour,
            rotation,
            filled,
            clip: self.clip,
        });
    }

    pub fn text(&mut self, run: Run) {
        if run.colour[3] <= 0.002 || run.text.is_empty() {
            return;
        }
        let mut run = run;
        if run.clip.is_none() {
            // The frame's clip is already in physical pixels; a run's own is in logical
            // ones, so it is converted back rather than applied twice.
            run.clip = self.clip.map(|c| {
                [
                    c[0] / self.scale,
                    c[1] / self.scale,
                    c[2] / self.scale,
                    c[3] / self.scale,
                ]
            });
        }
        self.text.push(run);
    }

    /// A left-aligned line, the common case.
    pub fn label(&mut self, text: impl Into<String>, x: f32, y: f32, size: f32, colour: Colour) {
        self.text(Run::new(text, x, y, size, colour));
    }

    /// A line centred on `x`, for button captions and anything in a column.
    pub fn centred(
        &mut self,
        text: impl Into<String>,
        x: f32,
        y: f32,
        size: f32,
        colour: Colour,
        weight: u16,
    ) {
        self.text(
            Run::new(text, x, y, size, colour)
                .align(Align::Centre)
                .weight(weight),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::icons;
    use crate::ui::theme::Palette;

    fn frame() -> Frame {
        Frame::new(2.0)
    }

    #[test]
    fn logical_coordinates_are_scaled_to_physical_pixels() {
        let mut f = frame();
        f.fill([10.0, 20.0, 100.0, 50.0], [1.0, 0.0, 0.0, 1.0], 8.0);

        let s = &f.solid[0];
        assert_eq!(s.position, [20.0, 40.0]);
        assert_eq!(s.size, [200.0, 100.0]);
        assert_eq!(s.radius, 16.0, "radius must scale too, or corners drift");
    }

    #[test]
    fn a_solid_fill_does_not_refract() {
        let mut f = frame();
        f.fill([0.0, 0.0, 10.0, 10.0], [1.0; 4], 2.0);

        let s = &f.solid[0];
        assert_eq!(s.refraction, 0.0);
        assert_eq!(s.tint[3], 1.0, "the tint must fully replace the backdrop");
        assert_eq!(s.specular, 0.0);
        assert_eq!(s.shadow, 0.0, "a plain fill lies flat on what is under it");
    }

    #[test]
    fn fill_opacity_rides_on_the_surface_not_the_tint() {
        // The shader mixes to the tint colour first and applies opacity at the end. Putting
        // alpha in the tint instead would blend towards the blurred backdrop and tint the
        // fill with whatever happened to be behind it.
        let mut f = frame();
        f.fill([0.0, 0.0, 10.0, 10.0], [1.0, 0.0, 0.0, 0.4], 0.0);

        assert_eq!(f.solid[0].tint[3], 1.0);
        assert_eq!(f.solid[0].opacity, 0.4);
    }

    #[test]
    fn a_panel_fading_in_takes_its_shadow_with_it() {
        // A full-strength shadow under a barely-there panel reads as a hole in the page.
        let mut f = frame();
        let style = GlassStyle::panel(&Palette::DARK);
        f.panel([0.0, 0.0, 100.0, 100.0], style, 0.25);

        assert!(f.glass[0].shadow < style.shadow);
        assert!(f.glass[0].shadow > 0.0);
    }

    #[test]
    fn invisible_things_are_dropped_rather_than_drawn() {
        let mut f = frame();
        f.fill([0.0, 0.0, 10.0, 10.0], [1.0, 1.0, 1.0, 0.0], 0.0);
        f.fill([0.0, 0.0, 0.0, 10.0], [1.0; 4], 0.0);
        f.panel([0.0, 0.0, 10.0, 10.0], GlassStyle::card(&Palette::DARK), 0.0);
        f.label("", 0.0, 0.0, 13.0, [1.0; 4]);
        f.icon(icons::PLAY, [0.0, 0.0], 16.0, [1.0, 1.0, 1.0, 0.0]);

        assert!(f.solid.is_empty());
        assert!(f.glass.is_empty());
        assert!(f.text.is_empty());
        assert!(f.icons.is_empty());
    }

    #[test]
    fn a_dot_is_a_circle_centred_where_it_was_asked_for() {
        let mut f = Frame::new(1.0);
        f.dot([50.0, 50.0], 8.0, [1.0; 4]);

        let s = &f.solid[0];
        assert_eq!(s.position, [46.0, 46.0]);
        assert_eq!(s.size, [8.0, 8.0]);
        assert_eq!(s.radius, 4.0, "radius must be half the diameter or it is a square");
    }

    #[test]
    fn a_rule_is_one_physical_pixel_at_any_scale() {
        let mut f = Frame::new(2.0);
        f.rule(0.0, 10.0, 100.0, false, [1.0; 4]);
        assert_eq!(f.solid[0].size[1], 1.0, "a hairline must not thicken with DPI");

        let mut f = Frame::new(1.0);
        f.rule(0.0, 10.0, 100.0, false, [1.0; 4]);
        assert_eq!(f.solid[0].size[1], 1.0);
    }

    #[test]
    fn a_clip_reaches_icons_and_text_drawn_under_it() {
        let mut f = Frame::new(2.0);
        f.set_clip([10.0, 20.0, 100.0, 50.0]);
        f.icon(icons::PLAY, [50.0, 50.0], 16.0, [1.0; 4]);
        f.label("row", 20.0, 30.0, 13.0, [1.0; 4]);
        f.clear_clip();
        f.label("free", 0.0, 0.0, 13.0, [1.0; 4]);

        assert_eq!(f.icons[0].clip, Some([20.0, 40.0, 200.0, 100.0]));
        assert_eq!(f.text[0].clip, Some([10.0, 20.0, 100.0, 50.0]));
        assert!(f.text[1].clip.is_none());
    }

    #[test]
    fn an_explicit_run_clip_survives_the_frame_clip() {
        // A field clips its own text to the field. Being inside a scrolling pane must not
        // silently widen that.
        let mut f = Frame::new(1.0);
        f.set_clip([0.0, 0.0, 500.0, 500.0]);
        f.text(Run::new("x", 0.0, 0.0, 13.0, [1.0; 4]).clip([1.0, 2.0, 3.0, 4.0]));

        assert_eq!(f.text[0].clip, Some([1.0, 2.0, 3.0, 4.0]));
    }

    #[test]
    fn clearing_keeps_the_scale() {
        let mut f = frame();
        f.fill([0.0, 0.0, 1.0, 1.0], [1.0; 4], 0.0);
        f.clear();

        assert!(f.solid.is_empty());
        assert_eq!(f.scale(), 2.0);
    }
}
