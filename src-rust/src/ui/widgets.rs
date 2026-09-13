//! Reusable controls.
//!
//! Each takes the frame and the input together and returns what happened, so a screen reads
//! as a list of statements rather than twenty lines of rectangle arithmetic per button. This
//! is the immediate-mode shape: laid out, drawn, and hit-tested in one pass, with only the
//! animation state persisting between frames.
//!
//! # Where the motion lives
//!
//! A control here owns no state, so anything that moves has to be a spring held by [`Input`]
//! and keyed by the widget's id. That is why a selection indicator is drawn once, at a
//! position the spring reports, rather than once per option: the indicator is a single object
//! that travels, and modelling it as one is what makes it read as one.

use super::icons::{self, Icon};
use super::input::{id, Id, Input};
use super::paint::{Frame, Rect};
use super::text::{Align, Run};
use super::theme::{fade, gap, motion, radius, text, weight, GlassStyle, Palette};

/// Resolves how wide a string will render, from the previous frame's measurements.
///
/// Only the text engine knows, and it is not reachable from a layout pass — so a width that
/// has not been asked for yet comes back as an estimate and is correct from the next frame.
/// One frame of lag on a tab strip is invisible; laying it out from a character count is
/// not, which is what the first version did and why its tabs had uneven padding.
pub struct Measure<'a> {
    pub cache: &'a mut std::collections::HashMap<(String, u32, u16), f32>,
    pub queue: &'a mut Vec<(String, f32, u16)>,
}

impl Measure<'_> {
    pub fn width(&mut self, value: &str, size: f32, weight: u16) -> f32 {
        let key = (value.to_string(), size.to_bits(), weight);
        if let Some(found) = self.cache.get(&key) {
            return *found;
        }
        if !self.queue.iter().any(|(q, s, w)| q == value && *s == size && *w == weight) {
            self.queue.push((value.to_string(), size, weight));
        }
        // Inter's average advance over lowercase Latin is about 0.52 em. Only ever used for
        // one frame, and only for strings the interface has never drawn before.
        value.chars().count() as f32 * size * 0.52
    }
}

/// Everything a control needs, bundled so call sites stay short.
pub struct Ui<'a> {
    pub frame: &'a mut Frame,
    pub input: &'a mut Input,
    pub measure: Measure<'a>,
    pub palette: Palette,
    pub dt: f32,
}

/// How a button is weighted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Weight {
    /// Filled with the accent. One per screen at most.
    Primary,
    /// Glass. The default.
    Secondary,
    /// No surface until hovered.
    Ghost,
    /// Secondary, but the label is the danger colour.
    Danger,
}

/// What a button shows beside its label, if anything.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Adornment {
    #[default]
    None,
    /// An icon, to the left of the label.
    Icon(Icon),
    /// A turning spinner in place of the icon, while the button's action is in flight.
    Busy,
}

impl Ui<'_> {
    /// A button with a label.
    pub fn button(&mut self, key: &str, rect: Rect, label: &str, weight: Weight) -> bool {
        self.adorned_button(key, rect, label, weight, Adornment::None)
    }

    /// A button with an icon, a spinner, or neither, beside its label.
    ///
    /// The icon and the label are centred as a pair, so a button does not visibly shift its
    /// caption when an icon is swapped for a spinner mid-action.
    pub fn adorned_button(
        &mut self,
        key: &str,
        rect: Rect,
        label: &str,
        weight: Weight,
        adornment: Adornment,
    ) -> bool {
        let widget = id("button", key);
        let response = self.input.interact(widget, rect, self.dt);
        let palette = self.palette;

        // Pressing compresses the surface and settles it towards the page; hovering lifts it
        // off. Both are the same gesture read in opposite directions, which is why they are
        // one number: a button cannot be lifted and pressed at once.
        let lift = response.hover * (1.0 - response.press);
        let squeeze = response.press * 0.02;
        let inset = rect[3] * squeeze * 0.5;
        let shrunk: Rect = [
            rect[0] + inset,
            rect[1] + inset,
            rect[2] - inset * 2.0,
            rect[3] - inset * 2.0,
        ];

        match weight {
            Weight::Primary => {
                // `accent_deep` rather than `accent`: white on the brand orange is 3.6:1,
                // which is under what a caption needs. Hover brightens back towards the
                // brand colour, so the control is at its most recognisable under the pointer.
                let base = crate::ui::theme::mix(palette.accent_deep, palette.accent, lift);
                self.frame
                    .fill_lifted(shrunk, base, radius::SM, 0.5 + lift * 0.9);
            }
            Weight::Ghost => {
                if response.hover > 0.01 {
                    self.frame
                        .fill(shrunk, fade(palette.line, response.hover * 1.1), radius::SM);
                }
            }
            _ => {
                let mut style = GlassStyle::control(&palette);
                // Hover thickens the glass rather than tinting it. A button that gets paler
                // under the pointer reads as a state change; one that gets deeper and casts
                // further reads as movement, which is what it is.
                style.shadow *= 1.0 + lift * 1.1;
                style.shadow_blur *= 1.0 + lift * 0.5;
                style.edge *= 1.0 + lift * 0.35;
                self.frame.panel(shrunk, style, 1.0);
            }
        }

        let ink = match weight {
            Weight::Primary => [1.0, 1.0, 1.0, 1.0],
            Weight::Danger => palette.danger,
            _ => palette.ink,
        };

        let glyph = 15.0;
        let label_width = self.measure.width(label, text::BASE, weight::SEMIBOLD);
        let adorned = adornment != Adornment::None;
        let total = label_width + if adorned { glyph + gap::SM } else { 0.0 };

        let centre = rect[0] + rect[2] * 0.5;
        let left = centre - total * 0.5;
        let middle = rect[1] + rect[3] * 0.5;

        match adornment {
            Adornment::None => {}
            Adornment::Icon(icon) => {
                self.frame
                    .icon(icon, [left + glyph * 0.5, middle], glyph, ink);
            }
            Adornment::Busy => self.spinner([left + glyph * 0.5, middle], glyph, ink),
        }

        let text_left = if adorned { left + glyph + gap::SM } else { left };
        self.frame.text(
            Run::new(
                label,
                text_left,
                middle - text::BASE * 0.72,
                text::BASE,
                ink,
            )
            .weight(weight::SEMIBOLD),
        );

        response.clicked
    }

    /// A square button holding a single icon.
    pub fn icon_button(&mut self, key: &str, rect: Rect, icon: Icon, danger: bool) -> bool {
        let response = self.input.interact(id("icon", key), rect, self.dt);
        let palette = self.palette;

        if response.hover > 0.01 {
            let tint = if danger {
                fade(palette.danger, response.hover)
            } else {
                fade(palette.line, response.hover * 1.2)
            };
            self.frame.fill(rect, tint, radius::SM);
        }

        let ink = if danger && response.hover > 0.5 {
            [1.0, 1.0, 1.0, 1.0]
        } else {
            // The icon lightens towards full ink as the pointer arrives. A control that
            // stays muted under the pointer reads as disabled.
            crate::ui::theme::mix(palette.muted, palette.ink, response.hover)
        };

        // Sized against the button rather than fixed, so the same call serves a 26-point
        // window control and a 34-point toolbar button without either overflowing — which is
        // exactly what went wrong when these were font glyphs inheriting a size.
        let glyph = (rect[2].min(rect[3]) * 0.52).clamp(12.0, 22.0);
        self.frame.icon(
            icon,
            [rect[0] + rect[2] * 0.5, rect[1] + rect[3] * 0.5],
            glyph,
            ink,
        );

        response.clicked
    }

    /// A turning spinner, for an action that is in flight.
    ///
    /// Driven by the wall clock rather than by a spring: it has no target to reach, and a
    /// spring that settles is the one thing a busy indicator must never do.
    pub fn spinner(&mut self, centre: [f32; 2], size: f32, colour: super::theme::Colour) {
        self.frame.icon_full(
            icons::SPINNER,
            centre,
            size,
            colour,
            // Four fifths of a second per turn. Faster reads as frantic, slower as stuck.
            self.input.clock * (std::f32::consts::TAU / 0.8),
            false,
        );
    }

    /// A row of mutually exclusive options.
    ///
    /// The selected cell is a single pane of glass that *travels* between options rather than
    /// a highlight that switches off in one place and on in another. That distinction is the
    /// entire difference between this reading as a physical control and as a list of states,
    /// and it is why the indicator is drawn once, outside the loop, at a sprung rectangle.
    pub fn segmented(
        &mut self,
        key: &str,
        rect: Rect,
        options: &[&str],
        selected: usize,
    ) -> Option<usize> {
        let palette = self.palette;
        self.frame
            .panel(rect, GlassStyle::sunken(&palette).radius(radius::SM), 1.0);

        if options.is_empty() {
            return None;
        }

        let pad = 3.0;
        let width = rect[2] / options.len() as f32;
        let cell = |index: usize| -> Rect {
            [
                rect[0] + width * index as f32 + pad,
                rect[1] + pad,
                width - pad * 2.0,
                rect[3] - pad * 2.0,
            ]
        };

        // The indicator, before the labels, so text sits on top of its own glass.
        let target = cell(selected.min(options.len() - 1));
        let slid = self
            .input
            .animate_rect(id(key, "indicator"), target, motion::SLIDE, self.dt);
        self.frame.panel(slid, GlassStyle::slider(&palette), 1.0);

        let mut clicked = None;
        for (index, label) in options.iter().enumerate() {
            let cell = cell(index);
            let response = self.input.interact(id(key, label), cell, self.dt);

            if index != selected && response.hover > 0.01 {
                self.frame
                    .fill(cell, fade(palette.line, response.hover * 0.7), radius::XS);
            }

            self.frame.centred(
                *label,
                cell[0] + cell[2] * 0.5,
                cell[1] + (cell[3] - text::SM * 1.4) * 0.5,
                text::SM,
                if index == selected {
                    palette.ink
                } else {
                    // Towards full ink on hover, so an unselected option is legible the
                    // moment it is a candidate.
                    crate::ui::theme::mix(palette.muted, palette.ink, response.hover * 0.7)
                },
                if index == selected { weight::SEMIBOLD } else { weight::MEDIUM },
            );

            if response.clicked {
                clicked = Some(index);
            }
        }

        clicked
    }

    /// A labelled switch.
    pub fn toggle(
        &mut self,
        key: &str,
        rect: Rect,
        title: &str,
        description: &str,
        value: bool,
    ) -> Option<bool> {
        let palette = self.palette;
        let (x, y, width) = (rect[0], rect[1], rect[2]);
        let widget = id("toggle", key);
        let response = self.input.interact(widget, rect, self.dt);

        if response.hover > 0.01 {
            self.frame.fill(
                [x - gap::SM, y, width + gap::SM * 2.0, rect[3]],
                fade(palette.line, response.hover * 0.5),
                radius::SM,
            );
        }

        self.frame
            .text(Run::new(title, x, y + 6.0, text::BASE, palette.ink).weight(weight::MEDIUM));
        self.frame.text(
            Run::new(description, x, y + 24.0, text::SM, palette.muted).width(width - 64.0),
        );

        let track: Rect = [x + width - 42.0, y + 10.0, 42.0, 24.0];

        // One spring drives the whole switch: the knob's travel, the track's colour, and how
        // far the knob stretches as it goes. Deriving them from a single number is what keeps
        // them in step — three springs would arrive at three different times.
        let on = self
            .input
            .animate(widget ^ 0xFF, if value { 1.0 } else { 0.0 }, motion::PANEL, self.dt);

        self.frame.fill(
            track,
            crate::ui::theme::mix(palette.line_strong, palette.accent, on),
            radius::FULL,
        );

        // The knob elongates towards the direction of travel at the midpoint and rounds off
        // again at either end — the squash a physical switch would show. `on * (1 - on)`
        // peaks at the halfway point and is zero at both rest states, so it needs no
        // special-casing at the ends.
        let stretch = on * (1.0 - on) * 10.0;
        let knob = 18.0;
        let travel = track[2] - knob - 6.0;
        let knob_x = track[0] + 3.0 + travel * on - stretch * 0.5;

        self.frame.fill_lifted(
            [knob_x, track[1] + 3.0, knob + stretch, knob],
            [1.0, 1.0, 1.0, 1.0],
            knob * 0.5,
            0.8,
        );

        self.frame.rule(x, y + rect[3], width, false, palette.line);

        response.clicked.then_some(!value)
    }

    /// A tab strip, with an indicator that slides between tabs.
    ///
    /// Returns the tab clicked and the width the strip took, so a caller can lay out beside
    /// it without guessing.
    pub fn tabs(
        &mut self,
        key: &str,
        x: f32,
        y: f32,
        labels: &[&str],
        selected: usize,
    ) -> (Option<usize>, f32) {
        let palette = self.palette;

        // Measured, not estimated. The first version multiplied the character count by a
        // constant, which gave "Git" and "Metrics" visibly different padding.
        let widths: Vec<f32> = labels
            .iter()
            .map(|label| self.measure.width(label, text::SM, weight::SEMIBOLD) + gap::LG * 1.5)
            .collect();

        let mut cursor = x;
        let mut rects = Vec::with_capacity(labels.len());
        for width in &widths {
            rects.push([cursor, y, *width, 28.0] as Rect);
            cursor += width + gap::XS;
        }

        if let Some(target) = rects.get(selected.min(rects.len().saturating_sub(1))) {
            let slid = self
                .input
                .animate_rect(id(key, "indicator"), *target, motion::SLIDE, self.dt);
            self.frame.fill(slid, palette.accent_wash, radius::SM);
        }

        let mut clicked = None;
        for (index, label) in labels.iter().enumerate() {
            let rect = rects[index];
            let response = self.input.interact(id(key, label), rect, self.dt);

            if index != selected && response.hover > 0.01 {
                self.frame
                    .fill(rect, fade(palette.line, response.hover * 0.8), radius::SM);
            }

            self.frame.centred(
                *label,
                rect[0] + rect[2] * 0.5,
                rect[1] + (rect[3] - text::SM * 1.4) * 0.5,
                text::SM,
                if index == selected {
                    palette.accent_ink
                } else {
                    crate::ui::theme::mix(palette.muted, palette.ink, response.hover * 0.7)
                },
                weight::SEMIBOLD,
            );

            if response.clicked {
                clicked = Some(index);
            }
        }

        (clicked, cursor - x)
    }

    /// A label and a value on one line, as the overview and git panes use throughout.
    pub fn row(&mut self, x: f32, y: f32, width: f32, label: &str, value: &str) {
        let palette = self.palette;
        self.frame.label(label, x, y, text::SM, palette.muted);
        self.frame.text(
            Run::new(value, x + 108.0, y, text::BASE, palette.ink).width(width - 108.0),
        );
    }

    /// A section heading, in the small uppercase style the stylesheet used.
    pub fn heading(&mut self, x: f32, y: f32, label: &str) {
        let palette = self.palette;
        self.frame
            .text(Run::new(label.to_uppercase(), x, y, text::XS, palette.muted).weight(weight::BOLD));
    }

    /// Centred placeholder text, for a pane with nothing to show yet.
    pub fn placeholder(&mut self, x: f32, y: f32, width: f32, message: &str) {
        let palette = self.palette;
        self.frame.text(
            Run::new(message, x + width * 0.5, y, text::BASE, palette.muted)
                .align(Align::Centre)
                .width(width - gap::XL),
        );
    }

    /// A pill carrying a status word, and a dot that pulses while that status is live.
    ///
    /// Returns the width it drew, so a caller can place something after it.
    pub fn badge(
        &mut self,
        label: &str,
        x: f32,
        y: f32,
        colour: super::theme::Colour,
        pulsing: bool,
    ) -> f32 {
        let width = self.measure.width(label, text::XS, weight::SEMIBOLD) + 30.0;
        let rect: Rect = [x, y, width, 20.0];

        self.frame.fill(rect, fade(colour, 0.16), radius::FULL);

        let dot = [x + 11.0, y + 10.0];
        if pulsing {
            // A halo breathing under the dot. Sine rather than a spring: this is a heartbeat,
            // not a journey, and it should never arrive anywhere.
            let beat = (self.input.clock * 2.2).sin() * 0.5 + 0.5;
            self.frame
                .glow(dot, 8.0 + beat * 9.0, fade(colour, 0.30 - beat * 0.18));
        }
        self.frame.dot(dot, 6.0, colour);

        self.frame.text(
            Run::new(label, x + 20.0, y + 4.0, text::XS, colour).weight(weight::SEMIBOLD),
        );

        width
    }

    /// A horizontal meter, for a percentage that changes while it is being watched.
    ///
    /// The bar springs to its reading rather than jumping. A CPU figure sampled twice a
    /// second and drawn raw flickers; one that is chased reads as a measurement.
    pub fn meter(&mut self, key: Id, rect: Rect, fraction: f32, colour: super::theme::Colour) {
        let palette = self.palette;
        self.frame
            .fill(rect, fade(palette.line_strong, 0.7), radius::FULL);

        let value = self
            .input
            .animate(key, fraction.clamp(0.0, 1.0), motion::READOUT, self.dt);

        if value > 0.001 {
            // Never narrower than its own height, so a reading of half a percent is still a
            // dot rather than a sliver with square ends.
            let filled = (rect[2] * value).max(rect[3]);
            self.frame
                .fill([rect[0], rect[1], filled, rect[3]], colour, radius::FULL);
        }
    }
}

impl Ui<'_> {
    /// An editable text field.
    ///
    /// Returns true when the value changed this frame. The caret is drawn at the end of the
    /// text because that is the only place editing can happen — see `Input::edit`. Showing a
    /// caret the user could not move would be a lie, so it only appears while focused.
    pub fn field(
        &mut self,
        key: &str,
        rect: Rect,
        value: &mut String,
        placeholder: &str,
        text_width: f32,
    ) -> bool {
        let widget = id("field", key);
        let response = self.input.interact(widget, rect, self.dt);
        let palette = self.palette;

        if response.clicked {
            self.input.focus_on(widget);
        }
        let focused = self.input.has_focus(widget);

        self.frame.panel(rect, GlassStyle::sunken(&palette), 1.0);

        // The focus ring grows rather than appearing. It is the only thing on screen telling
        // the user where their typing will go, so it is worth the two frames.
        let ring = self.input.animate(
            widget ^ 0xAB,
            if focused { 1.0 } else { 0.0 },
            motion::HOVER,
            self.dt,
        );
        if ring > 0.01 {
            self.frame
                .fill(rect, fade(palette.accent, ring * 0.12), radius::SM);
        } else if response.hover > 0.01 {
            self.frame
                .fill(rect, fade(palette.line, response.hover * 0.5), radius::SM);
        }

        let changed = if focused { self.input.edit(value) } else { false };

        let showing_placeholder = value.is_empty() && !focused;
        let shown = if showing_placeholder {
            placeholder
        } else {
            value.as_str()
        };
        let ink = if showing_placeholder {
            palette.muted
        } else {
            palette.ink
        };

        let text_y = rect[1] + (rect[3] - text::BASE * 1.4) * 0.5;
        self.frame.text(
            Run::new(shown, rect[0] + gap::MD, text_y, text::BASE, ink)
                .clip([rect[0], rect[1], rect[2], rect[3]]),
        );

        if focused && self.input.caret_visible(self.dt) {
            // Positioned from a measured width rather than a character count, so it does not
            // drift on proportional type.
            let caret_x = rect[0] + gap::MD + text_width.min(rect[2] - gap::MD * 2.0);
            self.frame.fill(
                [caret_x + 1.0, rect[1] + 8.0, 1.5, rect[3] - 16.0],
                palette.accent,
                0.75,
            );
        }

        changed
    }

    /// A field with a label above it, as the project form uses throughout.
    ///
    /// `rect` covers the pair; the label takes the first 20 points and the field the rest.
    pub fn labelled_field(
        &mut self,
        key: &str,
        rect: Rect,
        label: &str,
        value: &mut String,
        placeholder: &str,
        text_width: f32,
    ) -> bool {
        let palette = self.palette;
        self.frame.text(
            Run::new(label, rect[0], rect[1], text::SM, palette.ink_soft).weight(weight::MEDIUM),
        );
        self.field(
            key,
            [rect[0], rect[1] + 20.0, rect[2], rect[3] - 20.0],
            value,
            placeholder,
            text_width,
        )
    }
}
