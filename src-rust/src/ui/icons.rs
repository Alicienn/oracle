//! Icons.
//!
//! The first native build drew its icons as characters from Segoe UI Symbol — ▶ for play, ■
//! for stop, ⚙ for settings. That was expedient and it looked it: those glyphs come from
//! three different eras of the same font, they are designed to sit in running text rather
//! than in a button, their optical weights do not match each other, and several of them are
//! simply the wrong shape for the job.
//!
//! These are [Lucide](https://lucide.dev) instead — one family, drawn on a 24-unit grid with
//! a uniform 2-unit stroke, so every icon in the interface shares a weight. They are vendored
//! into `assets/icons` under the ISC licence rather than pulled from a crate: the set needed
//! here is fifty files, and embedding them means the rendering path is ours and the binary
//! has no runtime dependency on anything.
//!
//! # How they reach the screen
//!
//! An SVG is a description, not an image, so something has to turn one into pixels. That
//! happens once per (icon, size) pair, on first use, into a shared alpha atlas; afterwards
//! drawing one is a quad and four texture coordinates. The atlas is single-channel because
//! an icon has no colour of its own — the tint comes from the draw call, which is what lets
//! the same rasterisation serve a muted icon, an accent one and a white one.

use std::collections::HashMap;

/// Every icon the interface can draw, embedded at compile time.
///
/// Named rather than indexed so a call site reads `icons::PLAY` and a missing icon is a
/// compile error rather than a blank square.
macro_rules! icons {
    ($($konst:ident => $file:literal),* $(,)?) => {
        $(pub const $konst: Icon = Icon { name: $file, svg: include_str!(concat!("../../assets/icons/", $file, ".svg")) };)*

        /// Every icon, for the test that checks they all rasterise.
        #[cfg(test)]
        const ALL: &[Icon] = &[$($konst),*];
    };
}

/// One icon: its name, and the SVG source it is drawn from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Icon {
    pub name: &'static str,
    svg: &'static str,
}

icons! {
    PLAY => "play",
    STOP => "square",
    SEARCH => "search",
    SETTINGS => "settings",
    PENCIL => "pencil",
    CLOSE => "x",
    PLUS => "plus",
    MINUS => "minus",
    MAXIMISE => "maximize",
    RESTORE => "minimize",
    POWER => "power",
    SCAN => "folder-search",
    REFRESH => "refresh-cw",
    EXTERNAL => "external-link",
    BRANCH => "git-branch",
    ACTIVITY => "activity",
    TERMINAL => "terminal",
    COPY => "copy",
    TRASH => "trash",
    CHEVRON_RIGHT => "chevron-right",
    CHEVRON_DOWN => "chevron-down",
    CHEVRON_LEFT => "chevron-left",
    SERVER => "server",
    GLOBE => "globe",
    BOX => "box",
    MOON => "moon",
    SUN => "sun",
    SUN_MOON => "sun-moon",
    SPINNER => "loader-circle",
    ALERT => "circle-alert",
    CHECK_CIRCLE => "circle-check",
    ERROR_CIRCLE => "circle-x",
    FOLDER => "folder",
    FOLDER_OPEN => "folder-open",
    ROCKET => "rocket",
    CPU => "cpu",
    MEMORY => "memory-stick",
    DISK => "hard-drive",
    CLOCK => "clock",
    LINK => "link-2",
    ARROW_OUT => "arrow-up-right",
    GRID => "layout-grid",
    LIST => "list",
    CHECK => "check",
    EYE => "eye",
    PANEL_CLOSE => "panel-right-close",
    PANEL_OPEN => "panel-right-open",
    SAVE => "save",
    ROTATE => "rotate-cw",
    ZAP => "zap",
    PACKAGE => "package",
}

/// Side of the atlas, in pixels.
///
/// Fifty icons at the three or four sizes an interface actually uses, rasterised at up to
/// 200% display scale, come to well under a quarter of this. It is single-channel, so the
/// whole thing is one megabyte.
const ATLAS: u32 = 1024;

/// A gap between packed icons, so bilinear filtering at the edge of one cannot pick up the
/// next.
const PAD: u32 = 2;

/// Where one rasterisation of one icon lives in the atlas.
#[derive(Debug, Clone, Copy)]
pub struct Placement {
    /// Texture coordinates: left, top, right, bottom, in 0..1.
    pub uv: [f32; 4],
    /// Size it was rasterised at, in physical pixels.
    pub size: f32,
}

/// The rasterised icons, and the texture holding them.
pub struct IconAtlas {
    texture: wgpu::Texture,
    pub view: wgpu::TextureView,

    placements: HashMap<(&'static str, bool, u32), Placement>,

    /// Shelf packing: icons go left to right along a row whose height is the tallest icon in
    /// it, and a full row starts a new one above. For a set this small and this uniform the
    /// waste is a few percent, and the alternative is a rectangle packer nobody needs to
    /// read.
    shelf_x: u32,
    shelf_y: u32,
    shelf_height: u32,
    full: bool,
}

impl IconAtlas {
    pub fn new(device: &wgpu::Device) -> Self {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("icon atlas"),
            size: wgpu::Extent3d {
                width: ATLAS,
                height: ATLAS,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            // Coverage only. The colour of an icon belongs to the place it is drawn, not to
            // the icon, so storing it would be three quarters waste and one quarter wrong.
            format: wgpu::TextureFormat::R8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());

        Self {
            texture,
            view,
            placements: HashMap::new(),
            shelf_x: PAD,
            shelf_y: PAD,
            shelf_height: 0,
            full: false,
        }
    }

    /// Where this icon sits at this size, rasterising it first if it has not been seen.
    ///
    /// `size` is in physical pixels and is rounded to whole pixels, so a control that grows
    /// by a fraction during a spring does not rasterise a new copy every frame.
    pub fn place(
        &mut self,
        queue: &wgpu::Queue,
        icon: Icon,
        size: f32,
        filled: bool,
    ) -> Option<Placement> {
        let px = (size.round().max(4.0) as u32).min(256);
        let key = (icon.name, filled, px);

        if let Some(found) = self.placements.get(&key) {
            return Some(*found);
        }
        if self.full {
            return None;
        }

        let pixels = rasterise(icon, px, filled)?;

        // Find a shelf. A new row starts above the tallest icon in the current one.
        if self.shelf_x + px + PAD > ATLAS {
            self.shelf_x = PAD;
            self.shelf_y += self.shelf_height + PAD;
            self.shelf_height = 0;
        }
        if self.shelf_y + px + PAD > ATLAS {
            // Out of room. Drawing nothing is a worse outcome than a blank icon, but
            // corrupting the atlas is worse than both, so the door closes here.
            self.full = true;
            return None;
        }

        let (x, y) = (self.shelf_x, self.shelf_y);
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &self.texture,
                mip_level: 0,
                origin: wgpu::Origin3d { x, y, z: 0 },
                aspect: wgpu::TextureAspect::All,
            },
            &pixels,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(px),
                rows_per_image: Some(px),
            },
            wgpu::Extent3d {
                width: px,
                height: px,
                depth_or_array_layers: 1,
            },
        );

        self.shelf_x += px + PAD;
        self.shelf_height = self.shelf_height.max(px);

        let scale = 1.0 / ATLAS as f32;
        let placement = Placement {
            uv: [
                x as f32 * scale,
                y as f32 * scale,
                (x + px) as f32 * scale,
                (y + px) as f32 * scale,
            ],
            size: px as f32,
        };
        self.placements.insert(key, placement);
        Some(placement)
    }
}

/// Renders one icon to a square of coverage bytes.
///
/// Lucide draws with `stroke="currentColor"` and no fill, which means nothing at all outside
/// a browser. Both are substituted here for black, and only the alpha channel is kept — so
/// what comes back is exactly the shape, with the antialiasing the rasteriser produced and
/// no colour to fight the tint applied later.
///
/// `filled` additionally floods the paths. Lucide's play and square are outlines; a transport
/// control wants them solid, and drawing a filled triangle from the same source keeps its
/// proportions identical to the outlined one beside it.
fn rasterise(icon: Icon, size: u32, filled: bool) -> Option<Vec<u8>> {
    let fill = if filled { "black" } else { "none" };
    let svg = icon
        .svg
        .replace("stroke=\"currentColor\"", "stroke=\"black\"")
        .replace("fill=\"none\"", &format!("fill=\"{fill}\""));

    let tree = resvg::usvg::Tree::from_str(&svg, &resvg::usvg::Options::default()).ok()?;

    let mut pixmap = resvg::tiny_skia::Pixmap::new(size, size)?;
    let scale = size as f32 / tree.size().width().max(1.0);
    resvg::render(
        &tree,
        resvg::tiny_skia::Transform::from_scale(scale, scale),
        &mut pixmap.as_mut(),
    );

    // Every pixel is black at some opacity, so the alpha channel alone is the coverage.
    Some(pixmap.pixels().iter().map(|p| p.alpha()).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_icon_rasterises_to_something_visible() {
        // A typo in a filename is a compile error, but a valid file that renders blank is
        // not — and a blank icon is invisible rather than obviously broken.
        for icon in ALL {
            let pixels = rasterise(*icon, 32, false)
                .unwrap_or_else(|| panic!("{} did not rasterise", icon.name));

            assert_eq!(pixels.len(), 32 * 32);
            let covered = pixels.iter().filter(|p| **p > 8).count();
            assert!(
                covered > 20,
                "{} rasterised to {covered} covered pixels, which is blank",
                icon.name
            );
        }
    }

    #[test]
    fn filling_an_outline_covers_more_than_stroking_it() {
        let outline = rasterise(PLAY, 48, false).unwrap();
        let solid = rasterise(PLAY, 48, true).unwrap();

        let count = |p: &Vec<u8>| p.iter().filter(|b| **b > 128).count();
        assert!(
            count(&solid) > count(&outline) * 2,
            "a filled play triangle should be substantially more ink than its outline"
        );
    }

    #[test]
    fn an_icon_fills_the_box_it_was_asked_for() {
        // Lucide's 24-unit grid has a 2-unit margin, so a glyph should reach close to the
        // edge but not touch it. A scale error shows up here as either overflow or a
        // postage stamp in the corner.
        let pixels = rasterise(SETTINGS, 64, false).unwrap();

        let mut min = 64usize;
        let mut max = 0usize;
        for (index, byte) in pixels.iter().enumerate() {
            if *byte > 16 {
                let x = index % 64;
                min = min.min(x);
                max = max.max(x);
            }
        }

        assert!(min < 12, "left edge at {min} means the icon is too small");
        assert!(max > 52, "right edge at {max} means the icon is too small");
        assert!(max < 64, "the icon overflows its box");
    }

    #[test]
    fn names_are_unique() {
        // Two constants pointing at the same file would share atlas entries silently.
        let mut names: Vec<&str> = ALL.iter().map(|i| i.name).collect();
        names.sort_unstable();
        let before = names.len();
        names.dedup();
        assert_eq!(before, names.len(), "two icons share a source file");
    }
}
