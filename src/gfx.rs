//! Drawing primitives shared by the platform renderers (Direct2D on Windows,
//! Cairo and Pango on Linux). The picker draws only through [`Canvas`].

#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct Color {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

impl Color {
    #[cfg_attr(not(windows), allow(dead_code))]
    pub const TRANSPARENT: Color = Color { r: 0.0, g: 0.0, b: 0.0, a: 0.0 };

    pub const fn with_alpha(self, a: f32) -> Color {
        Color { a, ..self }
    }

    /// Relative luminance, for telling light from dark backgrounds.
    pub fn luma(self) -> f32 {
        0.2126 * self.r + 0.7152 * self.g + 0.0722 * self.b
    }
}

/// Rectangle in device independent pixels.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct Rect {
    pub left: f32,
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
}

pub const fn rect(left: f32, top: f32, right: f32, bottom: f32) -> Rect {
    Rect { left, top, right, bottom }
}

// Used by the Cairo renderer.
#[cfg_attr(windows, allow(dead_code))]
impl Rect {
    pub fn width(&self) -> f32 {
        self.right - self.left
    }

    pub fn height(&self) -> f32 {
        self.bottom - self.top
    }
}

/// Text styles of the picker. The theme decides family, size and weight.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum Font {
    /// A character in the grid.
    Glyph,
    /// A grid entry made of several characters.
    GlyphSmall,
    /// Section title.
    Title,
    Subtitle,
    Search,
    /// Character description, trimmed with an ellipsis.
    Info,
    /// The character in the info bar.
    InfoGlyph,
    /// Text label of a navigation tab.
    Tab,
    /// Centered, wrapping message (nothing found).
    Message,
    /// Centered heading of a full-panel prompt.
    Heading,
    /// Label of a drawn key cap.
    Key,
}

impl Font {
    #[cfg_attr(not(windows), allow(dead_code))]
    pub const ALL: [Font; 11] = [
        Font::Glyph,
        Font::GlyphSmall,
        Font::Title,
        Font::Subtitle,
        Font::Search,
        Font::Info,
        Font::InfoGlyph,
        Font::Tab,
        Font::Message,
        Font::Heading,
        Font::Key,
    ];
}

/// Icons of the picker. Windows draws them from the Segoe icon fonts, Linux as paths.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum Icon {
    Search,
    Recent,
    Close,
    Prev,
    Next,
    Keyboard,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum IconSize {
    /// Navigation bar scroll arrows.
    Small,
    /// Navigation tabs.
    Tab,
    /// Search and close buttons.
    Normal,
    /// Large icon of a full-panel message.
    Large,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Weight {
    Normal,
    SemiBold,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Align {
    Leading,
    Center,
}

/// How a [`Font`] looks; built by the theme.
#[derive(Clone, Debug, PartialEq)]
pub struct FontSpec {
    pub family: String,
    /// Size in device independent pixels.
    pub size: f32,
    pub weight: Weight,
    pub align: Align,
    /// Wrap onto several lines, starting at the top of the rectangle. Otherwise a
    /// single line is centered vertically.
    pub wrap: bool,
    /// End lines that do not fit with an ellipsis.
    pub ellipsis: bool,
}

pub trait Canvas {
    /// Fill `r`, with rounded corners when `radius` > 0.
    fn fill(&mut self, r: Rect, radius: f32, c: Color);
    /// Stroke the outline of `r`, centered on its edges.
    fn stroke(&mut self, r: Rect, radius: f32, width: f32, c: Color);
    fn text(&mut self, s: &str, font: Font, r: Rect, c: Color);
    /// Draw `icon` centered in `r`.
    fn icon(&mut self, icon: Icon, size: IconSize, r: Rect, c: Color);
    /// Advance width of `s`, trailing spaces included.
    fn measure(&mut self, s: &str, font: Font) -> f32;
    fn push_clip(&mut self, r: Rect);
    fn pop_clip(&mut self);
}
