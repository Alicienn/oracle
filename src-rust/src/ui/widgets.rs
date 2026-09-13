//! Reusable controls.
//!
//! Each takes the frame and the input together and returns what happened, so a screen reads
//! as a list of statements rather than twenty lines of rectangle arithmetic per button. This
//! is the immediate-mode shape: laid out, drawn, and hit-tested in one pass, with only the
//! animation state persisting between frames.

use super::input::{id, Input};
use super::paint::{Frame, Rect};
use super::text::{Align, Run};
use super::theme::{fade, gap, radius, text, GlassStyle, Palette};

/// Everything a control needs, bundled so call sites stay short.
pub struct Ui<'a> {
    pub frame: &'a mut Frame,
    pub input: &'a mut Input,
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

impl Ui<'_> {
    /// A button with a label. Returns true on the frame it is clicked.
    pub fn button(&mut self, key: &str, rect: Rect, label: &str, weight: Weight) -> bool {
        let response = self.input.interact(id("button", key), rect, self.dt);
        let palette = self.palette;

        // Pressing compresses the surface slightly. Small enough to feel physical rather
        // than cartoonish — the same 1.5% the stylesheet used.
        let squeeze = response.press * 0.015;
        let inset = rect[2].min(rect[3]) * squeeze * 0.5;
        let shrunk: Rect = [
            rect[0] + inset,
            rect[1] + inset,
            rect[2] - inset * 2.0,
            rect[3] - inset * 2.0,
        ];

        match weight {
            Weight::Primary => {
                let lift = response.hover * 0.10;
                self.frame.fill(
                    shrunk,
                    [
                        palette.accent[0] + lift,
                        palette.accent[1] + lift,
                        palette.accent[2] + lift,
                        1.0,
                    ],
                    radius::SM,
                );
            }
            Weight::Ghost => {
                if response.hover > 0.01 {
                    self.frame
                        .fill(shrunk, fade(palette.line, response.hover * 0.9), radius::SM);
                }
            }
            _ => {
                self.frame
                    .panel(shrunk, GlassStyle::control(&palette), 1.0);
                if response.hover > 0.01 {
                    self.frame
                        .fill(shrunk, fade(palette.line, response.hover * 0.5), radius::SM);
                }
            }
        }

        let ink = match weight {
            Weight::Primary => [1.0, 1.0, 1.0, 1.0],
            Weight::Danger => palette.danger,
            _ => palette.ink,
        };

        self.frame.centred(
            label,
            rect[0] + rect[2] * 0.5,
            rect[1] + (rect[3] - text::BASE * 1.35) * 0.5,
            text::BASE,
            ink,
            570,
        );

        response.clicked
    }

    /// A square button holding a single glyph.
    pub fn icon_button(&mut self, key: &str, rect: Rect, glyph: &str, danger: bool) -> bool {
        let response = self.input.interact(id("icon", key), rect, self.dt);
        let palette = self.palette;

        if response.hover > 0.01 {
            let tint = if danger {
                fade(palette.danger, response.hover)
            } else {
                fade(palette.line, response.hover)
            };
            self.frame.fill(rect, tint, radius::SM);
        }

        let ink = if danger && response.hover > 0.5 {
            [1.0, 1.0, 1.0, 1.0]
        } else {
            palette.muted
        };

        self.frame.centred(
            glyph,
            rect[0] + rect[2] * 0.5,
            rect[1] + (rect[3] - text::MD * 1.35) * 0.5,
            text::MD,
            ink,
            500,
        );

        response.clicked
    }

    /// A row of mutually exclusive options. Returns the one clicked, if any.
    pub fn segmented(&mut self, key: &str, rect: Rect, options: &[&str], selected: usize) -> Option<usize> {
        let palette = self.palette;
        self.frame.panel(rect, GlassStyle::sunken(&palette), 1.0);

        let width = rect[2] / options.len() as f32;
        let mut clicked = None;

        for (index, label) in options.iter().enumerate() {
            let cell: Rect = [
                rect[0] + width * index as f32 + 3.0,
                rect[1] + 3.0,
                width - 6.0,
                rect[3] - 6.0,
            ];
            let response = self
                .input
                .interact(id(key, label), cell, self.dt);

            if index == selected {
                self.frame
                    .fill(cell, palette.glass_tint_strong, radius::SM);
            } else if response.hover > 0.01 {
                self.frame
                    .fill(cell, fade(palette.line, response.hover * 0.7), radius::SM);
            }

            self.frame.centred(
                *label,
                cell[0] + cell[2] * 0.5,
                cell[1] + (cell[3] - text::SM * 1.35) * 0.5,
                text::SM,
                if index == selected {
                    palette.ink
                } else {
                    palette.muted
                },
                560,
            );

            if response.clicked {
                clicked = Some(index);
            }
        }

        clicked
    }

    /// A labelled switch. Returns the new value when it is flipped.
    pub fn toggle(
        &mut self,
        key: &str,
        x: f32,
        y: f32,
        width: f32,
        title: &str,
        description: &str,
        value: bool,
    ) -> Option<bool> {
        let palette = self.palette;
        let rect: Rect = [x, y, width, 46.0];
        let response = self.input.interact(id("toggle", key), rect, self.dt);

        if response.hover > 0.01 {
            self.frame.fill(
                [x - gap::SM, y, width + gap::SM * 2.0, rect[3]],
                fade(palette.line, response.hover * 0.5),
                radius::SM,
            );
        }

        self.frame.text(
            Run::new(title, x, y + 6.0, text::BASE, palette.ink).weight(560),
        );
        self.frame.text(
            Run::new(description, x, y + 24.0, text::SM, palette.muted)
                .width(width - 60.0),
        );

        // The track, then the knob. The knob position is the value, so a spring on it would
        // be the natural next step; a fill is enough while the rest of the screen lands.
        let track: Rect = [x + width - 40.0, y + 11.0, 40.0, 23.0];
        self.frame.fill(
            track,
            if value { palette.accent } else { palette.line_strong },
            radius::FULL,
        );
        self.frame.dot(
            [
                track[0] + if value { 28.5 } else { 11.5 },
                track[1] + 11.5,
            ],
            19.0,
            [1.0, 1.0, 1.0, 1.0],
        );

        self.frame
            .rule(x, y + rect[3], width, false, palette.line);

        response.clicked.then_some(!value)
    }

    /// A tab strip. Returns the tab clicked, if any.
    pub fn tabs(&mut self, key: &str, x: f32, y: f32, labels: &[&str], selected: usize) -> Option<usize> {
        let palette = self.palette;
        let mut cursor = x;
        let mut clicked = None;

        for (index, label) in labels.iter().enumerate() {
            // Roughly eight pixels per character plus padding; close enough for a tab strip
            // and far cheaper than shaping the text twice.
            let width = label.len() as f32 * 7.2 + 20.0;
            let rect: Rect = [cursor, y, width, 27.0];
            let response = self.input.interact(id(key, label), rect, self.dt);

            if index == selected {
                self.frame.fill(rect, palette.accent_wash, radius::SM);
            } else if response.hover > 0.01 {
                self.frame
                    .fill(rect, fade(palette.line, response.hover * 0.8), radius::SM);
            }

            self.frame.centred(
                *label,
                rect[0] + width * 0.5,
                rect[1] + 5.0,
                text::SM,
                if index == selected {
                    palette.accent
                } else {
                    palette.muted
                },
                560,
            );

            if response.clicked {
                clicked = Some(index);
            }
            cursor += width + 4.0;
        }

        clicked
    }

    /// A label and a value on one line, as the overview and git panes use throughout.
    pub fn row(&mut self, x: f32, y: f32, width: f32, label: &str, value: &str) {
        let palette = self.palette;
        self.frame.label(label, x, y, text::SM, palette.muted);
        self.frame.text(
            Run::new(value, x + 104.0, y, text::BASE, palette.ink).width(width - 104.0),
        );
    }

    /// A section heading, in the small uppercase style the stylesheet used.
    pub fn heading(&mut self, x: f32, y: f32, label: &str) {
        let palette = self.palette;
        self.frame.text(
            Run::new(label.to_uppercase(), x, y, text::XS, palette.muted).weight(700),
        );
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
        if focused {
            // A ring rather than a fill, so the text stays as legible as it was.
            self.frame
                .fill(rect, fade(palette.accent, 0.10), radius::SM);
        } else if response.hover > 0.01 {
            self.frame
                .fill(rect, fade(palette.line, response.hover * 0.5), radius::SM);
        }

        let changed = if focused {
            self.input.edit(value)
        } else {
            false
        };

        let showing_placeholder = value.is_empty() && !focused;
        let shown = if showing_placeholder { placeholder } else { value.as_str() };
        let ink = if showing_placeholder {
            palette.muted
        } else {
            palette.ink
        };

        let text_y = rect[1] + (rect[3] - text::BASE * 1.35) * 0.5;
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
    pub fn labelled_field(
        &mut self,
        key: &str,
        x: f32,
        y: f32,
        width: f32,
        label: &str,
        value: &mut String,
        placeholder: &str,
        text_width: f32,
    ) -> bool {
        let palette = self.palette;
        self.frame.text(
            Run::new(label, x, y, text::SM, palette.ink_soft).weight(570),
        );
        self.field(key, [x, y + 20.0, width, 32.0], value, placeholder, text_width)
    }
}
