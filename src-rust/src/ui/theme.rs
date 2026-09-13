//! Design tokens.
//!
//! The same palette and geometry the stylesheet carried, as typed constants. Colours are
//! linear-ish sRGB in 0..1 because that is what a shader wants; the hex values they came from
//! are in the comments so the two versions can be compared by eye.

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

/// Light and dark share every value except the ones listed here, exactly as the two blocks
/// in `tokens.css` did.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Palette {
    pub accent: Colour,
    pub accent_bright: Colour,
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

    /// How bright a specular rim is against this ground.
    pub specular: f32,
    /// How deep the inner shadow is.
    pub inner_shadow: f32,
    pub dark: bool,
}

impl Palette {
    pub const LIGHT: Self = Self {
        accent: hex(0xC15F3C),
        accent_bright: hex(0xD97449),
        accent_wash: alpha(hex(0xC15F3C), 0.12),

        paper: hex(0xF4F3EE),
        paper_deep: hex(0xEAE8E0),
        ink: hex(0x1A1815),
        ink_soft: hex(0x4A453D),
        muted: hex(0x8D887C),
        line: alpha(hex(0x1A1815), 0.10),
        line_strong: alpha(hex(0x1A1815), 0.18),

        ok: hex(0x5B8C6E),
        warn: hex(0xC99A2E),
        danger: hex(0xB3442E),
        idle: hex(0xB1ADA1),

        glass_tint: alpha(hex(0xFFFFFF), 0.55),
        glass_tint_strong: alpha(hex(0xFFFFFF), 0.72),
        glass_tint_faint: alpha(hex(0xFFFFFF), 0.34),

        specular: 0.70,
        inner_shadow: 0.08,
        dark: false,
    };

    pub const DARK: Self = Self {
        accent: hex(0xD1714C),
        accent_bright: hex(0xE08A63),
        accent_wash: alpha(hex(0xD1714C), 0.16),

        paper: hex(0x141310),
        paper_deep: hex(0x0D0C0A),
        ink: hex(0xF2EFE9),
        ink_soft: hex(0xC3BDB1),
        muted: hex(0x8A8377),
        line: alpha(hex(0xFFFFFF), 0.09),
        line_strong: alpha(hex(0xFFFFFF), 0.16),

        ok: hex(0x77AD8C),
        warn: hex(0xD8AE4A),
        danger: hex(0xCF5F47),
        idle: hex(0x6A6459),

        glass_tint: alpha(hex(0x282520), 0.58),
        glass_tint_strong: alpha(hex(0x302C26), 0.76),
        glass_tint_faint: alpha(hex(0x282520), 0.34),

        specular: 0.26,
        inner_shadow: 0.14,
        dark: true,
    };
}

/// Corner radii, in logical pixels.
pub mod radius {
    pub const XS: f32 = 6.0;
    pub const SM: f32 = 9.0;
    pub const MD: f32 = 13.0;
    pub const LG: f32 = 18.0;
    pub const XL: f32 = 24.0;
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

/// Motion, matching the durations the stylesheet used.
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
}

impl GlassStyle {
    /// A large surface: the shell, a detail pane. Gentle, because a wide panel with a hard
    /// rim reads as a sticker.
    pub fn panel(palette: &Palette) -> Self {
        Self {
            radius: radius::LG,
            bevel: 16.0,
            refraction: 14.0,
            tint: palette.glass_tint,
            specular: palette.specular * 0.8,
        }
    }

    /// A card or a row. Smaller, so it can afford a crisper edge.
    pub fn card(palette: &Palette) -> Self {
        Self {
            radius: radius::MD,
            bevel: 11.0,
            refraction: 16.0,
            tint: palette.glass_tint_strong,
            specular: palette.specular,
        }
    }

    /// Small floating chrome — a button, a pill. Thick relative to its size, which is what
    /// makes little elements read as glass rather than as flat tint.
    pub fn control(palette: &Palette) -> Self {
        Self {
            radius: radius::SM,
            bevel: 9.0,
            refraction: 20.0,
            tint: palette.glass_tint_strong,
            specular: palette.specular * 1.1,
        }
    }

    /// A recessed field. No refraction: an input should look like a hole, not a lens.
    pub fn sunken(palette: &Palette) -> Self {
        Self {
            radius: radius::SM,
            bevel: 6.0,
            refraction: 0.0,
            tint: palette.glass_tint_faint,
            specular: palette.specular * 0.3,
        }
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
        // Ink and paper invert; the accent stays recognisably the same hue.
        assert!(Palette::LIGHT.paper[0] > Palette::DARK.paper[0]);
        assert!(Palette::LIGHT.ink[0] < Palette::DARK.ink[0]);
        assert!(!Palette::LIGHT.dark && Palette::DARK.dark);

        let light_accent = Palette::LIGHT.accent;
        let dark_accent = Palette::DARK.accent;
        assert!(
            (light_accent[0] - dark_accent[0]).abs() < 0.1,
            "the accent must stay the same colour across themes"
        );
    }

    #[test]
    fn a_dark_ground_takes_a_dimmer_specular() {
        // A rim tuned for light paper blows out against a dark one.
        assert!(Palette::DARK.specular < Palette::LIGHT.specular);
    }

    #[test]
    fn an_input_does_not_refract() {
        // A field should read as a recess. Refraction would make it read as a lens.
        assert_eq!(GlassStyle::sunken(&Palette::DARK).refraction, 0.0);
        assert!(GlassStyle::control(&Palette::DARK).refraction > 0.0);
    }

    #[test]
    fn smaller_surfaces_refract_harder() {
        // A small control needs proportionally more bend to read as glass at all.
        let panel = GlassStyle::panel(&Palette::LIGHT);
        let control = GlassStyle::control(&Palette::LIGHT);
        assert!(control.refraction > panel.refraction);
    }
}
