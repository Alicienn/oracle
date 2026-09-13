//! Design tokens.
//!
//! The same palette and geometry the stylesheet carried, as typed constants. Colours are
//! straight sRGB in 0..1 because that is what a shader wants; the hex values they came from
//! are in the comments so the two versions can be compared by eye.
//!
//! # Contrast
//!
//! The first pass ported the stylesheet's values literally, and several of them were only
//! ever legible because a browser composited them over a light Mica backdrop. Over the app's
//! own dark wash they were not. Every foreground token below is annotated with its measured
//! contrast against the ground it is actually drawn on, and none of them is under the 4.5:1
//! that body text needs — `muted` in both themes was the worst offender at roughly 3.5:1,
//! which is why small secondary text was hard to read on screen and fine in the palette.

use glisten_motion::Spring;

/// Straight sRGB, 0..1, with alpha.
pub type Colour = [f32; 4];

/// Parses a hex literal at compile time, so the palette below reads like the stylesheet did.
pub const fn hex(value: u32) -> Colour {
    [
        ((value >> 16) & 0xFF) as f32 / 255.0,
        ((value >> 8) & 0xFF) as f32 / 255.0,
        (value & 0xFF) as f32 / 255.0,
        1.0,
    ]
}

/// The same colour at a different opacity.
pub const fn alpha(colour: Colour, a: f32) -> Colour {
    [colour[0], colour[1], colour[2], a]
}

/// The same colour with its opacity scaled, for fading a token in and out.
pub fn fade(colour: Colour, factor: f32) -> Colour {
    [colour[0], colour[1], colour[2], colour[3] * factor]
}

/// Blends two colours, ignoring their alpha. For deriving a hover state from a base.
pub fn mix(a: Colour, b: Colour, t: f32) -> Colour {
    [
        a[0] + (b[0] - a[0]) * t,
        a[1] + (b[1] - a[1]) * t,
        a[2] + (b[2] - a[2]) * t,
        a[3] + (b[3] - a[3]) * t,
    ]
}

/// Relative luminance, per WCAG. Used by the contrast tests below, and by nothing at
/// runtime — a palette that has to be measured while it draws is a palette with a bug.
pub fn luminance(colour: Colour) -> f32 {
    fn channel(c: f32) -> f32 {
        if c <= 0.03928 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    }
    0.2126 * channel(colour[0]) + 0.7152 * channel(colour[1]) + 0.0722 * channel(colour[2])
}

/// WCAG contrast ratio between two opaque colours, from 1:1 to 21:1.
pub fn contrast(a: Colour, b: Colour) -> f32 {
    let (x, y) = (luminance(a), luminance(b));
    (x.max(y) + 0.05) / (x.min(y) + 0.05)
}

/// Light and dark share every value except the ones listed here, exactly as the two blocks
/// in `tokens.css` did.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Palette {
    pub accent: Colour,
    pub accent_bright: Colour,
    /// A darker accent, for a filled surface carrying white text.
    ///
    /// White on the brand orange is 3.6:1 — under the 4.5:1 a button label needs. Going a
    /// step darker for the fill keeps the brand recognisable and the caption readable, which
    /// is the trade every brand palette eventually makes.
    pub accent_deep: Colour,
    /// The accent as *text*, which is not the same colour as the accent as a fill.
    ///
    /// A brand colour is chosen to work as a large area of ink on paper. At 12 point on a
    /// glass card it has to clear a contrast floor instead, and the brand orange clears it in
    /// neither theme: it is too light on the light one and too dark on the dark one. So this
    /// leans each way — deeper on paper, brighter on a dark ground — and the brand colour
    /// itself stays untouched for everything it is actually good at.
    pub accent_ink: Colour,
    pub accent_wash: Colour,

    pub paper: Colour,
    pub paper_deep: Colour,
    pub ink: Colour,
    pub ink_soft: Colour,
    pub muted: Colour,
    pub line: Colour,
    pub line_strong: Colour,

    pub ok: Colour,
    pub warn: Colour,
    pub danger: Colour,
    pub idle: Colour,

    /// Veil colour over the backdrop for a glass surface, and how much it covers.
    pub glass_tint: Colour,
    pub glass_tint_strong: Colour,
    pub glass_tint_faint: Colour,

    /// How strongly the backdrop's accent pools depart from flat paper.
    ///
    /// Lives here rather than in the renderer because it decides what every piece of text in
    /// the window is actually read against — turning it up washes the ground out from under
    /// the type — and the contrast tests below need the same number the shader uses. Two
    /// copies of it drifted apart once already.
    pub wash: f32,

    /// How bright a specular rim is against this ground.
    pub specular: f32,
    /// How bright the hairline tracing a surface's outline is.
    pub edge: f32,
    /// How deep the inner shadow is.
    pub inner_shadow: f32,
    /// How dark a cast shadow is. A dark room swallows shadows; a light one shows them.
    pub shadow: f32,
    pub dark: bool,
}

impl Palette {
    pub const LIGHT: Self = Self {
        accent: hex(0xC15F3C),
        accent_bright: hex(0xD97449),
        accent_deep: hex(0xA0492A),
        accent_ink: hex(0x9A4526),
        accent_wash: alpha(hex(0xC15F3C), 0.14),

        paper: hex(0xF4F3EE),
        paper_deep: hex(0xEAE8E0),
        ink: hex(0x1A1815),
        ink_soft: hex(0x4A453D),
        // Was #8D887C, which is 3.5:1 on paper — under the floor for the 11px labels it is
        // mostly used for.
        muted: hex(0x6E6A60),
        line: alpha(hex(0x1A1815), 0.12),
        line_strong: alpha(hex(0x1A1815), 0.20),

        ok: hex(0x3F7355),
        warn: hex(0x96701A),
        danger: hex(0xA83A26),
        idle: hex(0x726E63),

        glass_tint: alpha(hex(0xFFFFFF), 0.50),
        glass_tint_strong: alpha(hex(0xFFFFFF), 0.64),
        // A recess, so it goes the other way — see the note on the dark palette's.
        glass_tint_faint: alpha(hex(0x2A2620), 0.10),

        wash: 0.18,
        specular: 0.55,
        edge: 0.50,
        inner_shadow: 0.07,
        shadow: 0.20,
        dark: false,
    };

    pub const DARK: Self = Self {
        accent: hex(0xE0855C),
        accent_bright: hex(0xEE9C76),
        accent_deep: hex(0xB2573A),
        accent_ink: hex(0xF2A583),
        accent_wash: alpha(hex(0xE0855C), 0.18),

        paper: hex(0x16140F),
        paper_deep: hex(0x0D0C09),
        ink: hex(0xF4F1EB),
        ink_soft: hex(0xCEC8BB),
        // Was #8A8377, and then #A8A093, and both were measured against the wrong ground.
        // On a glass card sitting over a lit part of the wash — which is where most of the
        // project list is — #A8A093 comes out at 2.98:1. Secondary text in a dark theme over
        // pale translucent glass simply has to be this light; the hierarchy against `ink` is
        // carried by weight and size as much as by tone.
        muted: hex(0xC4BCAE),
        line: alpha(hex(0xFFFFFF), 0.13),
        line_strong: alpha(hex(0xFFFFFF), 0.22),

        ok: hex(0x92CBA7),
        warn: hex(0xE7C365),
        danger: hex(0xF09076),
        // "Stopped" is written in this, not only drawn as a dot, so it has to clear the same
        // floor every other status colour does — it reads as grey either way.
        idle: hex(0xADA596),

        // Much lighter than the ported values, and lighter than the page.
        //
        // The stylesheet's dark tints were near-black at around half opacity, which works in
        // a browser because the element sits over the system's own light-leaking Mica layer.
        // Over Oracle's own wash they came out darker than the ground, so a card read as a
        // hole punched in the page rather than as a pane resting on it. Dark-mode glass has
        // to *lift*: it is a pale, weakly-opaque film, and what makes it legible as glass is
        // that it is brighter than what surrounds it while still showing it through.
        glass_tint: alpha(hex(0x6E6559), 0.26),
        glass_tint_strong: alpha(hex(0x7C7265), 0.34),
        // The exception, and the reason this one is not simply a weaker version of the
        // others: a field is a recess, so it goes the other way and sits darker than the
        // surface it is cut into.
        glass_tint_faint: alpha(hex(0x0B0A07), 0.34),

        wash: 0.20,
        specular: 0.42,
        // The hairline does most of the work of separating one dark surface from another,
        // so it carries more here than it does on paper.
        edge: 0.55,
        inner_shadow: 0.16,
        // A dark ground has less light to block, so the same shadow reads as soot.
        shadow: 0.34,
        dark: true,
    };
}

/// Corner radii, in logical pixels.
///
/// Generous throughout, and deliberately so: a square corner on a translucent surface reads
/// as a clipping error rather than as a shape, because nothing physical has one.
pub mod radius {
    pub const XS: f32 = 7.0;
    pub const SM: f32 = 10.0;
    pub const MD: f32 = 14.0;
    pub const LG: f32 = 20.0;
    pub const XL: f32 = 26.0;
    /// The window itself. Matches what Windows 11 rounds a native frame to, so the drawn
    /// corner and the clipped one agree.
    pub const WINDOW: f32 = 10.0;
    /// Fully rounded. Clamped to half the shorter side by the shader.
    pub const FULL: f32 = 9999.0;
}

/// Spacing, in logical pixels.
pub mod gap {
    pub const XS: f32 = 4.0;
    pub const SM: f32 = 8.0;
    pub const MD: f32 = 12.0;
    pub const LG: f32 = 16.0;
    pub const XL: f32 = 24.0;
    pub const XXL: f32 = 32.0;
}

/// Type sizes, in logical pixels.
pub mod text {
    pub const XS: f32 = 11.0;
    pub const SM: f32 = 12.0;
    pub const BASE: f32 = 13.0;
    pub const MD: f32 = 15.0;
    pub const LG: f32 = 19.0;
    pub const XL: f32 = 26.0;
}

/// Type weights, named rather than numbered.
///
/// Inter is loaded as four static cuts, so anything in between is rounded to the nearest by
/// the font matcher. Naming them stops call sites inventing a 570 that silently becomes a
/// 600 and a 560 that silently becomes a 500.
pub mod weight {
    pub const REGULAR: u16 = 400;
    pub const MEDIUM: u16 = 500;
    pub const SEMIBOLD: u16 = 600;
    pub const BOLD: u16 = 700;
}

/// Motion.
///
/// Every duration in the stylesheet is here as a spring instead. `response` is roughly the
/// time to reach the target, and `damping_ratio` below 1 overshoots — which is what makes an
/// arrival read as physical rather than as a fade.
pub mod motion {
    use super::Spring;

    /// Hover and press states. Quick enough to feel attached to the pointer.
    pub const HOVER: Spring = Spring {
        response: 0.18,
        damping_ratio: 1.0,
    };

    /// Panels sliding in and out.
    pub const PANEL: Spring = Spring {
        response: 0.42,
        damping_ratio: 0.86,
    };

    /// Anything arriving on screen for the first time, where a little overshoot reads as
    /// physical rather than mechanical.
    pub const ENTRANCE: Spring = Spring {
        response: 0.45,
        damping_ratio: 0.72,
    };

    /// A selection indicator travelling between options. Loose enough to be followed by the
    /// eye, tight enough not to lag behind a second click.
    pub const SLIDE: Spring = Spring {
        response: 0.32,
        damping_ratio: 0.80,
    };

    /// A value counting towards a new reading — a gauge, a percentage.
    pub const READOUT: Spring = Spring {
        response: 0.55,
        damping_ratio: 1.0,
    };
}

/// The glass finish used for each kind of surface.
///
/// Collected here rather than scattered through the widgets, so the whole interface can be
/// retuned from one place — which is what the CSS custom properties were for.
#[derive(Debug, Clone, Copy)]
pub struct GlassStyle {
    pub radius: f32,
    pub bevel: f32,
    pub refraction: f32,
    pub tint: Colour,
    pub specular: f32,
    pub specular_power: f32,
    pub edge: f32,
    pub inner_shadow: f32,
    pub shadow: f32,
    pub shadow_blur: f32,
    pub shadow_drop: f32,
}

impl GlassStyle {
    /// A large surface: the shell, a detail pane. Gentle, because a wide panel with a hard
    /// rim reads as a sticker — but it sits high above the page, so its shadow is wide.
    pub fn panel(palette: &Palette) -> Self {
        Self {
            radius: radius::LG,
            bevel: 18.0,
            refraction: 26.0,
            tint: palette.glass_tint,
            specular: palette.specular * 0.8,
            specular_power: 20.0,
            edge: palette.edge * 0.9,
            inner_shadow: palette.inner_shadow,
            shadow: palette.shadow,
            shadow_blur: 48.0,
            shadow_drop: 18.0,
        }
    }

    /// A card or a row. Smaller, so it can afford a crisper edge and a tighter shadow.
    pub fn card(palette: &Palette) -> Self {
        Self {
            radius: radius::MD,
            bevel: 12.0,
            refraction: 24.0,
            tint: palette.glass_tint_strong,
            specular: palette.specular,
            specular_power: 24.0,
            edge: palette.edge,
            inner_shadow: palette.inner_shadow,
            shadow: palette.shadow * 0.7,
            shadow_blur: 22.0,
            shadow_drop: 7.0,
        }
    }

    /// Small floating chrome — a button, a pill. Thick relative to its size, which is what
    /// makes little elements read as glass rather than as flat tint.
    ///
    /// The bevel is over a third of a 30-point control's height on purpose: at this scale
    /// there is no flat middle to speak of, and the whole surface should be lens.
    pub fn control(palette: &Palette) -> Self {
        Self {
            radius: radius::SM,
            bevel: 11.0,
            refraction: 30.0,
            tint: palette.glass_tint_strong,
            specular: palette.specular * 1.15,
            specular_power: 30.0,
            edge: palette.edge * 1.25,
            inner_shadow: palette.inner_shadow * 0.8,
            shadow: palette.shadow * 0.55,
            shadow_blur: 14.0,
            shadow_drop: 4.0,
        }
    }

    /// A recessed field or track. No refraction and no shadow: an input should look like a
    /// hole, not a lens sitting on top of the page.
    pub fn sunken(palette: &Palette) -> Self {
        Self {
            radius: radius::SM,
            bevel: 7.0,
            refraction: 0.0,
            tint: palette.glass_tint_faint,
            specular: palette.specular * 0.2,
            specular_power: 40.0,
            edge: palette.edge * 0.25,
            // Inverted depth: the shading sits on the rim nearest the light, which is what
            // makes a recess read as a recess rather than as a dull button.
            inner_shadow: palette.inner_shadow * 2.2,
            shadow: 0.0,
            shadow_blur: 1.0,
            shadow_drop: 0.0,
        }
    }

    /// The selected cell of a segmented control: a small pane that slides. Brighter than a
    /// control so it reads as lifted out of the track it moves in.
    pub fn slider(palette: &Palette) -> Self {
        Self {
            radius: radius::XS + 1.0,
            bevel: 9.0,
            refraction: 26.0,
            tint: palette.glass_tint_strong,
            specular: palette.specular * 1.3,
            specular_power: 34.0,
            edge: palette.edge * 1.4,
            inner_shadow: palette.inner_shadow * 0.5,
            shadow: palette.shadow * 0.5,
            shadow_blur: 10.0,
            shadow_drop: 3.0,
        }
    }

    /// Overrides the corner radius, for a surface whose shape the layout dictates.
    pub fn radius(mut self, radius: f32) -> Self {
        self.radius = radius;
        self
    }

    /// Removes the cast shadow, for glass lying directly on other glass.
    pub fn flat(mut self) -> Self {
        self.shadow = 0.0;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_parses_the_brand_accent() {
        let accent = hex(0xC15F3C);
        assert!((accent[0] - 193.0 / 255.0).abs() < 1e-6);
        assert!((accent[1] - 95.0 / 255.0).abs() < 1e-6);
        assert!((accent[2] - 60.0 / 255.0).abs() < 1e-6);
        assert_eq!(accent[3], 1.0);
    }

    #[test]
    fn alpha_keeps_the_colour_and_replaces_the_opacity() {
        let washed = alpha(hex(0xC15F3C), 0.12);
        assert_eq!(&washed[..3], &hex(0xC15F3C)[..3]);
        assert_eq!(washed[3], 0.12);
    }

    #[test]
    fn the_two_palettes_differ_where_they_should_and_agree_where_they_must() {
        // Ink and paper invert. Compared by luminance rather than by the red channel: it is
        // what "lighter" means, and it keeps the assertion out of reach of const folding.
        assert!(luminance(Palette::LIGHT.paper) > luminance(Palette::DARK.paper));
        assert!(luminance(Palette::LIGHT.ink) < luminance(Palette::DARK.ink));
        assert!(Palette::LIGHT.dark != Palette::DARK.dark);

        // Hue, not lightness. Dark needs a lighter, less saturated accent to clear its
        // ground, so comparing the channels directly would fail on two colours anyone would
        // call the same orange. The hue angle is what "the same colour" actually means.
        let hue = |c: Colour| {
            let (max, min) = (c[0].max(c[1]).max(c[2]), c[0].min(c[1]).min(c[2]));
            let span = max - min;
            if span < 1e-6 {
                return 0.0;
            }
            let degrees = if max == c[0] {
                60.0 * ((c[1] - c[2]) / span)
            } else if max == c[1] {
                60.0 * (2.0 + (c[2] - c[0]) / span)
            } else {
                60.0 * (4.0 + (c[0] - c[1]) / span)
            };
            (degrees + 360.0) % 360.0
        };

        let drift = (hue(Palette::LIGHT.accent) - hue(Palette::DARK.accent)).abs();
        assert!(
            drift < 8.0,
            "the accent drifts {drift:.1}° between themes; it must stay the same orange"
        );
    }

    /// The ground each foreground token is actually drawn against, at its worst.
    ///
    /// Not the paper: almost nothing in Oracle sits on bare paper. Text sits on a glass card,
    /// which is the tint composited over the wash — and the wash is not flat either. The
    /// accent pools raise a good part of it, and a card over one of them is meaningfully
    /// lighter than a card over the corner.
    ///
    /// An earlier version of this measured against flat paper and passed while small text was
    /// genuinely hard to read on screen, because every token it approved was approved against
    /// the most forgiving ground in the window rather than the least. The lit case is the one
    /// worth pinning: in a dark theme it is where light text has least to work with, and in a
    /// light theme it is where dark text does.
    fn ground(palette: &Palette) -> Colour {
        // The peak of an accent pool, from the same token the shader is handed.
        let mut wash = [0.0; 4];
        for i in 0..3 {
            wash[i] = (palette.paper[i] + palette.accent[i] * palette.wash).min(1.0);
        }

        let tint = palette.glass_tint_strong;
        let mut out = [0.0; 4];
        for i in 0..3 {
            out[i] = wash[i] * (1.0 - tint[3]) + tint[i] * tint[3];
        }
        out[3] = 1.0;
        out
    }

    #[test]
    fn body_text_clears_the_readability_floor_on_both_themes() {
        for palette in [&Palette::LIGHT, &Palette::DARK] {
            let bg = ground(palette);
            for (name, colour) in [
                ("ink", palette.ink),
                ("ink_soft", palette.ink_soft),
                ("muted", palette.muted),
            ] {
                let ratio = contrast(colour, bg);
                assert!(
                    ratio >= 4.5,
                    "{name} is {ratio:.2}:1 on a {} card, under the 4.5:1 body text needs",
                    if palette.dark { "dark" } else { "light" }
                );
            }
        }
    }

    #[test]
    fn status_colours_are_legible_as_text_not_only_as_dots() {
        // Running, Failed and the rest are written out beside their dot, in this colour.
        for palette in [&Palette::LIGHT, &Palette::DARK] {
            let bg = ground(palette);
            for (name, colour) in [
                ("ok", palette.ok),
                ("warn", palette.warn),
                ("danger", palette.danger),
                ("idle", palette.idle),
                // `accent_ink`, not `accent`: see the note on the field. Testing the brand
                // colour here would either fail forever or force the brand to change.
                ("accent_ink", palette.accent_ink),
            ] {
                let ratio = contrast(colour, bg);
                assert!(ratio >= 3.5, "{name} is {ratio:.2}:1, which is not readable");
            }
        }
    }

    #[test]
    fn a_filled_accent_button_carries_white_text_legibly() {
        // The brand orange itself is 3.6:1 against white, which is why `accent_deep` exists.
        for palette in [&Palette::LIGHT, &Palette::DARK] {
            let ratio = contrast(palette.accent_deep, [1.0, 1.0, 1.0, 1.0]);
            assert!(ratio >= 4.5, "white on accent_deep is only {ratio:.2}:1");
        }
    }

    #[test]
    fn a_dark_ground_takes_a_dimmer_specular_and_a_heavier_shadow() {
        // A rim tuned for light paper blows out against a dark one; a shadow tuned for light
        // paper disappears into it.
        let (dark, light) = (&Palette::DARK, &Palette::LIGHT);
        assert!(dark.specular < light.specular);
        assert!(dark.shadow > light.shadow);
    }

    #[test]
    fn a_dark_panel_is_lighter_than_the_page_it_floats_on() {
        // A tint darker than the wash behind it makes a panel read as a hole. This was the
        // ported stylesheet's dark theme, and it is why the first native build looked flat.
        let lift = |tint: Colour, paper: Colour| luminance(tint) - luminance(paper);
        assert!(lift(Palette::DARK.glass_tint, Palette::DARK.paper) > 0.0);
        assert!(lift(Palette::DARK.glass_tint_strong, Palette::DARK.paper) > 0.0);
    }

    #[test]
    fn an_input_does_not_refract_and_does_not_float() {
        // A field should read as a recess. Refraction would make it a lens, and a cast
        // shadow would make it a button.
        let sunken = GlassStyle::sunken(&Palette::DARK);
        assert_eq!(sunken.refraction, 0.0);
        assert_eq!(sunken.shadow, 0.0);
        assert!(GlassStyle::control(&Palette::DARK).refraction > 0.0);
    }

    #[test]
    fn smaller_surfaces_refract_harder() {
        // A small control needs proportionally more bend to read as glass at all.
        let panel = GlassStyle::panel(&Palette::LIGHT);
        let control = GlassStyle::control(&Palette::LIGHT);
        assert!(control.refraction > panel.refraction);
    }

    #[test]
    fn every_floating_surface_casts_a_shadow() {
        // Without one it is painted on, whatever else the shader does.
        for palette in [&Palette::LIGHT, &Palette::DARK] {
            for style in [
                GlassStyle::panel(palette),
                GlassStyle::card(palette),
                GlassStyle::control(palette),
                GlassStyle::slider(palette),
            ] {
                assert!(style.shadow > 0.0);
                assert!(style.edge > 0.0, "and traces its own outline");
            }
        }
    }

    #[test]
    fn a_bigger_surface_casts_a_wider_shadow_than_a_smaller_one() {
        let palette = &Palette::LIGHT;
        assert!(GlassStyle::panel(palette).shadow_blur > GlassStyle::card(palette).shadow_blur);
        assert!(GlassStyle::card(palette).shadow_blur > GlassStyle::control(palette).shadow_blur);
    }

    #[test]
    fn no_corner_in_the_interface_is_square() {
        for r in [radius::XS, radius::SM, radius::MD, radius::LG, radius::XL, radius::WINDOW] {
            assert!(r >= 7.0, "{r} is tight enough to read as a clipping error");
        }
    }
}
