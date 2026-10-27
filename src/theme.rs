//! Visual styles (Windows 10 and Windows 11 emoji panel look-alikes, and a KDE
//! Plasma / Breeze look) and colors.

use crate::gfx::{Align, Color, Font, FontSpec, IconSize, Weight};
use crate::settings::{BackdropChoice, Settings, StyleChoice, ThemeChoice};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Win10,
    Win11,
    /// KDE Plasma (Breeze), colored by the desktop's color scheme.
    Kde,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Backdrop {
    /// Windows 11 system backdrop (DWMSBT_TRANSIENTWINDOW).
    System,
    /// Acrylic blur through the accent policy (Windows 10 and Windows 11).
    Accent,
    /// Compositor blur behind a translucent background (Linux).
    Blur,
    Solid,
}

/// Layout metrics in device independent pixels.
#[derive(Clone, Copy, Debug)]
pub struct Metrics {
    pub width: f32,
    pub height: f32,
    /// Horizontal padding around the grid.
    pub pad: f32,
    pub cell: f32,
    pub cols: usize,
    /// Height of the top area holding the search field.
    pub top_h: f32,
    pub section_h: f32,
    pub info_h: f32,
    pub nav_h: f32,
    pub nav_tab_w: f32,
    pub cell_radius: f32,
    pub glyph_size: f32,
    /// Corner radius of the panel, when the picker draws its own frame.
    pub radius: f32,
}

#[derive(Clone, Copy, Debug)]
pub struct Palette {
    pub dark: bool,
    /// Opaque background, used without transparency.
    pub solid: Color,
    /// Tint painted over the blurred backdrop.
    pub tint: Color,
    pub text: Color,
    pub text2: Color,
    pub text3: Color,
    pub hover: Color,
    pub pressed: Color,
    pub selected: Color,
    pub focus: Color,
    pub accent: Color,
    pub divider: Color,
    pub field: Color,
    pub field_border: Color,
    pub border: Color,
}

/// Colors of a desktop color scheme (KDE's `kdeglobals`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DesktopColors {
    /// `[Colors:Window]` BackgroundNormal.
    pub window: Color,
    /// `[Colors:Window]` ForegroundNormal.
    pub text: Color,
    /// `[Colors:Window]` ForegroundInactive.
    pub inactive: Color,
    /// `[Colors:View]` BackgroundNormal.
    pub view: Color,
    /// `[Colors:Selection]` BackgroundNormal, or the accent color.
    pub highlight: Color,
}

impl DesktopColors {
    pub fn dark(&self) -> bool {
        self.window.luma() < 0.5
    }
}

pub const BREEZE_LIGHT: DesktopColors = DesktopColors {
    window: rgba(239, 240, 241, 255),
    text: rgba(35, 38, 41, 255),
    inactive: rgba(112, 125, 138, 255),
    view: rgba(255, 255, 255, 255),
    highlight: rgba(61, 174, 233, 255),
};

pub const BREEZE_DARK: DesktopColors = DesktopColors {
    window: rgba(32, 35, 38, 255),
    text: rgba(252, 252, 252, 255),
    inactive: rgba(161, 169, 177, 255),
    view: rgba(20, 22, 24, 255),
    highlight: rgba(61, 174, 233, 255),
};

/// What the platform knows about the desktop's look.
#[derive(Clone, Debug)]
pub struct SystemLook {
    /// The style used for `style = auto`.
    pub auto_kind: Kind,
    pub dark: bool,
    /// Accent colors: (variant for dark backgrounds, variant for light backgrounds).
    pub accent: Option<(Color, Color)>,
    /// Translucent windows are wanted and possible.
    pub transparency: bool,
    pub high_contrast: bool,
    /// Windows 11 22H2 or later: the system backdrop is available.
    pub system_backdrop: bool,
    /// The compositor blurs behind translucent windows (Linux).
    pub blur: bool,
    /// The desktop's color scheme (KDE), when known.
    pub desktop: Option<DesktopColors>,
    /// The desktop's UI font family.
    pub font: Option<String>,
}

#[derive(Clone, Debug)]
pub struct Theme {
    pub kind: Kind,
    pub m: Metrics,
    pub c: Palette,
    pub backdrop: Backdrop,
    pub text_font: String,
    pub title_font: String,
    pub glyph_font: String,
    #[cfg_attr(not(windows), allow(dead_code))]
    pub icon_font: String,
}

pub const fn rgba(r: u8, g: u8, b: u8, a: u8) -> Color {
    Color { r: r as f32 / 255.0, g: g as f32 / 255.0, b: b as f32 / 255.0, a: a as f32 / 255.0 }
}

pub const fn gray(v: u8, a: u8) -> Color {
    rgba(v, v, v, a)
}

fn font_or(families: &[&str], fallback: &Option<String>, available: &dyn Fn(&str) -> bool) -> String {
    match families.iter().find(|f| available(f)) {
        Some(f) => f.to_string(),
        None => fallback.clone().unwrap_or_else(|| families[families.len() - 1].to_string()),
    }
}

fn kde_palette(d: &DesktopColors, dark: bool) -> Palette {
    let (fg, hl) = (d.text, d.highlight);
    Palette {
        dark,
        solid: d.window,
        tint: d.window.with_alpha(0.86),
        text: fg,
        text2: d.inactive,
        text3: fg.with_alpha(0.42),
        hover: hl.with_alpha(0.16),
        pressed: hl.with_alpha(0.34),
        selected: hl.with_alpha(0.26),
        focus: hl,
        accent: hl,
        divider: fg.with_alpha(0.12),
        field: d.view.with_alpha(0.72),
        field_border: fg.with_alpha(0.2),
        border: fg.with_alpha(0.2),
    }
}

impl Theme {
    pub fn resolve(settings: &Settings, look: &SystemLook, font_available: &dyn Fn(&str) -> bool) -> Theme {
        let kind = match settings.style {
            StyleChoice::Win10 => Kind::Win10,
            StyleChoice::Win11 => Kind::Win11,
            StyleChoice::Kde => Kind::Kde,
            StyleChoice::Auto => look.auto_kind,
        };
        let dark = match settings.theme {
            ThemeChoice::Dark => true,
            ThemeChoice::Light => false,
            ThemeChoice::Auto => look.dark,
        };
        let transparent = !look.high_contrast
            && match settings.backdrop {
                BackdropChoice::Solid => false,
                BackdropChoice::Acrylic => true,
                BackdropChoice::Auto => look.transparency,
            };
        let backdrop = if !transparent {
            Backdrop::Solid
        } else if look.blur {
            Backdrop::Blur
        } else if kind != Kind::Win10 && look.system_backdrop {
            Backdrop::System
        } else {
            Backdrop::Accent
        };
        // Without a compositor blur (or Windows' own backdrops) a translucent panel
        // would show the windows behind it unblurred.
        let backdrop = if backdrop != Backdrop::Blur && cfg!(not(windows)) { Backdrop::Solid } else { backdrop };

        let (accent_on_dark, accent_on_light) =
            look.accent.unwrap_or((rgba(0x60, 0xCD, 0xFF, 255), rgba(0x00, 0x5F, 0xB8, 255)));
        let accent = if dark { accent_on_dark } else { accent_on_light };
        let ui_font = &look.font;

        match kind {
            Kind::Win10 => Theme {
                kind,
                backdrop,
                m: Metrics {
                    width: 344.0,
                    height: 424.0,
                    pad: 12.0,
                    cell: 40.0,
                    cols: 8,
                    top_h: 44.0,
                    section_h: 32.0,
                    info_h: 28.0,
                    nav_h: 40.0,
                    nav_tab_w: 40.0,
                    cell_radius: 0.0,
                    glyph_size: 20.0,
                    radius: 0.0,
                },
                c: if dark {
                    Palette {
                        dark,
                        solid: gray(0x2B, 255),
                        tint: gray(0x1F, 0xCC),
                        text: gray(0xFF, 255),
                        text2: gray(0xFF, 0x99),
                        text3: gray(0xFF, 0x66),
                        hover: gray(0xFF, 0x1A),
                        pressed: gray(0xFF, 0x33),
                        selected: gray(0xFF, 0x1A),
                        focus: accent,
                        accent,
                        divider: gray(0xFF, 0x26),
                        field: gray(0x00, 0x00),
                        field_border: gray(0xFF, 0x00),
                        border: gray(0x55, 255),
                    }
                } else {
                    Palette {
                        dark,
                        solid: gray(0xF2, 255),
                        tint: gray(0xE6, 0xCC),
                        text: gray(0x00, 255),
                        text2: gray(0x00, 0x99),
                        text3: gray(0x00, 0x66),
                        hover: gray(0x00, 0x1A),
                        pressed: gray(0x00, 0x33),
                        selected: gray(0x00, 0x1A),
                        focus: accent,
                        accent,
                        divider: gray(0x00, 0x26),
                        field: gray(0xFF, 0x00),
                        field_border: gray(0x00, 0x00),
                        border: gray(0xBB, 255),
                    }
                },
                text_font: font_or(&["Segoe UI"], ui_font, font_available),
                title_font: font_or(&["Segoe UI"], ui_font, font_available),
                glyph_font: font_or(&["Segoe UI"], ui_font, font_available),
                icon_font: font_or(&["Segoe MDL2 Assets", "Segoe Fluent Icons", "Segoe UI Symbol"], &None, font_available),
            },
            Kind::Win11 => Theme {
                kind,
                backdrop,
                m: Metrics {
                    width: 360.0,
                    height: 452.0,
                    pad: 20.0,
                    cell: 40.0,
                    cols: 8,
                    top_h: 56.0,
                    section_h: 34.0,
                    info_h: 30.0,
                    nav_h: 44.0,
                    nav_tab_w: 40.0,
                    cell_radius: 4.0,
                    glyph_size: 20.0,
                    radius: 8.0,
                },
                c: if dark {
                    Palette {
                        dark,
                        solid: gray(0x2C, 255),
                        tint: gray(0x20, 0xD8),
                        text: gray(0xFF, 255),
                        text2: gray(0xFF, 0xC5),
                        text3: gray(0xFF, 0x87),
                        hover: gray(0xFF, 0x0F),
                        pressed: gray(0xFF, 0x0A),
                        selected: gray(0xFF, 0x15),
                        focus: gray(0xFF, 255),
                        accent,
                        divider: gray(0xFF, 0x15),
                        field: gray(0xFF, 0x0F),
                        field_border: gray(0xFF, 0x12),
                        border: gray(0x75, 0x66),
                    }
                } else {
                    Palette {
                        dark,
                        solid: gray(0xF9, 255),
                        tint: gray(0xFC, 0xD8),
                        text: gray(0x00, 0xE4),
                        text2: gray(0x00, 0x9E),
                        text3: gray(0x00, 0x72),
                        hover: gray(0x00, 0x09),
                        pressed: gray(0x00, 0x06),
                        selected: gray(0x00, 0x0F),
                        focus: gray(0x00, 0xE4),
                        accent,
                        divider: gray(0x00, 0x14),
                        field: gray(0xFF, 0xB3),
                        field_border: gray(0x00, 0x0F),
                        border: gray(0x75, 0x66),
                    }
                },
                text_font: font_or(&["Segoe UI Variable Text", "Segoe UI"], ui_font, font_available),
                title_font: font_or(&["Segoe UI Variable Display", "Segoe UI"], ui_font, font_available),
                glyph_font: font_or(&["Segoe UI"], ui_font, font_available),
                icon_font: font_or(&["Segoe Fluent Icons", "Segoe MDL2 Assets", "Segoe UI Symbol"], &None, font_available),
            },
            Kind::Kde => {
                // Follow the desktop's color scheme unless a forced light or dark
                // theme disagrees with it; then use stock Breeze with the accent.
                let colors = match look.desktop {
                    Some(d) if settings.theme == ThemeChoice::Auto || d.dark() == dark => d,
                    _ => {
                        let base = if dark { BREEZE_DARK } else { BREEZE_LIGHT };
                        let highlight = look.desktop.map_or(base.highlight, |d| d.highlight);
                        DesktopColors { highlight, ..base }
                    }
                };
                let dark = colors.dark();
                let font = ui_font.clone().unwrap_or_else(|| "Noto Sans".to_string());
                Theme {
                    kind,
                    backdrop,
                    m: Metrics {
                        width: 360.0,
                        height: 452.0,
                        pad: 20.0,
                        cell: 40.0,
                        cols: 8,
                        top_h: 56.0,
                        section_h: 34.0,
                        info_h: 30.0,
                        nav_h: 44.0,
                        nav_tab_w: 40.0,
                        cell_radius: 5.0,
                        glyph_size: 20.0,
                        radius: 8.0,
                    },
                    c: kde_palette(&colors, dark),
                    text_font: font.clone(),
                    title_font: font.clone(),
                    glyph_font: font,
                    // Linux draws the icons itself; Windows takes them from its icon fonts.
                    icon_font: font_or(&["Segoe Fluent Icons", "Segoe MDL2 Assets", "Segoe UI Symbol"], &None, font_available),
                }
            }
        }
    }

    pub fn font(&self, f: Font) -> FontSpec {
        let windows10 = self.kind == Kind::Win10;
        let spec = |family: &str, size: f32, weight: Weight, align: Align| FontSpec {
            family: family.to_string(),
            size,
            weight,
            align,
            wrap: false,
            ellipsis: false,
        };
        match f {
            Font::Glyph => spec(&self.glyph_font, self.m.glyph_size, Weight::Normal, Align::Center),
            Font::GlyphSmall => spec(&self.glyph_font, 15.0, Weight::Normal, Align::Center),
            Font::Title => spec(&self.title_font, if windows10 { 15.0 } else { 14.0 }, Weight::SemiBold, Align::Leading),
            Font::Subtitle => spec(&self.text_font, 12.0, Weight::Normal, Align::Leading),
            Font::Search => spec(&self.text_font, 14.0, Weight::Normal, Align::Leading),
            Font::Info => FontSpec { ellipsis: true, ..spec(&self.text_font, 12.0, Weight::Normal, Align::Leading) },
            Font::InfoGlyph => spec(&self.glyph_font, 18.0, Weight::Normal, Align::Center),
            Font::Tab => spec(&self.text_font, 12.0, Weight::SemiBold, Align::Center),
            Font::Message => FontSpec { wrap: true, ..spec(&self.text_font, 13.0, Weight::Normal, Align::Center) },
            Font::Heading => spec(&self.title_font, 18.0, Weight::SemiBold, Align::Center),
            Font::Key => spec(&self.text_font, 14.0, Weight::SemiBold, Align::Center),
        }
    }

    /// Icon size in device independent pixels.
    pub fn icon_size(&self, size: IconSize) -> f32 {
        match size {
            IconSize::Small => 10.0,
            IconSize::Tab => 14.0,
            IconSize::Normal => {
                if self.kind == Kind::Win10 {
                    15.0
                } else {
                    14.0
                }
            }
            IconSize::Large => 32.0,
        }
    }

    /// Tint for the accent policy, as 0xAABBGGRR.
    #[cfg(windows)]
    pub fn accent_gradient(&self) -> u32 {
        let c = self.c.tint;
        let b = |v: f32| (v * 255.0).round() as u32;
        // Windows 10's acrylic needs a fairly opaque tint to stay legible.
        let a = b(c.a).max(0x99);
        (a << 24) | (b(c.b) << 16) | (b(c.g) << 8) | b(c.r)
    }
}
