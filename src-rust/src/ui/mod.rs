//! Drawing, input and theming.
//!
//! Parts of this are a design system rather than a call graph: a palette carries tokens no
//! screen has reached for yet, `Weight` names a button style the current screens do not use,
//! and `Key` enumerates the keys a text field will need before it can do selection. Pruning
//! those to whatever today's screens happen to call would make the next screen re-add them
//! one at a time, and the vocabulary is the point.
#![allow(dead_code)]

pub mod icons;
pub mod input;
pub mod paint;
pub mod renderer;
pub mod text;
pub mod widgets;
pub mod theme;
