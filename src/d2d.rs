//! Direct2D / DirectWrite renderer for the picker window (Windows).

use std::collections::HashMap;

use windows::Win32::Foundation::{D2DERR_RECREATE_TARGET, HWND};
use windows::Win32::Graphics::Direct2D::Common::{
    D2D_RECT_F, D2D_SIZE_U, D2D1_ALPHA_MODE_PREMULTIPLIED, D2D1_COLOR_F, D2D1_PIXEL_FORMAT,
};
use windows::Win32::Graphics::Direct2D::{
    D2D1_ANTIALIAS_MODE_PER_PRIMITIVE, D2D1_DRAW_TEXT_OPTIONS_NONE, D2D1_FACTORY_TYPE_SINGLE_THREADED,
    D2D1_FEATURE_LEVEL_DEFAULT, D2D1_HWND_RENDER_TARGET_PROPERTIES, D2D1_PRESENT_OPTIONS_NONE,
    D2D1_RENDER_TARGET_PROPERTIES, D2D1_RENDER_TARGET_TYPE_DEFAULT, D2D1_RENDER_TARGET_TYPE_SOFTWARE,
    D2D1_RENDER_TARGET_USAGE_NONE, D2D1_ROUNDED_RECT, D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE, D2D1CreateFactory,
    ID2D1Factory, ID2D1HwndRenderTarget, ID2D1RenderTarget, ID2D1SolidColorBrush,
};
use windows::Win32::Graphics::DirectWrite::{
    DWRITE_FACTORY_TYPE_SHARED, DWRITE_FONT_STRETCH_NORMAL, DWRITE_FONT_STYLE_NORMAL, DWRITE_FONT_WEIGHT_NORMAL,
    DWRITE_FONT_WEIGHT_SEMI_BOLD, DWRITE_MEASURING_MODE_NATURAL, DWRITE_PARAGRAPH_ALIGNMENT_CENTER,
    DWRITE_PARAGRAPH_ALIGNMENT_NEAR, DWRITE_TEXT_ALIGNMENT_CENTER, DWRITE_TEXT_ALIGNMENT_LEADING, DWRITE_TEXT_METRICS,
    DWRITE_TRIMMING, DWRITE_TRIMMING_GRANULARITY_CHARACTER, DWRITE_WORD_WRAPPING_NO_WRAP, DWRITE_WORD_WRAPPING_WRAP,
    DWriteCreateFactory, IDWriteFactory, IDWriteFontCollection, IDWriteTextFormat,
};
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
use windows_core::{BOOL, PCWSTR};

use crate::gfx::{Align, Canvas, Color, Font, FontSpec, Icon, IconSize, Rect, Weight};
use crate::picker::Picker;
use crate::theme::Theme;
use crate::util::wide;

const ICON_SIZES: [IconSize; 4] = [IconSize::Small, IconSize::Tab, IconSize::Normal, IconSize::Large];

struct Formats {
    text: HashMap<Font, IDWriteTextFormat>,
    icon: HashMap<IconSize, IDWriteTextFormat>,
}

pub struct Gfx {
    hwnd: HWND,
    d2d: ID2D1Factory,
    dw: IDWriteFactory,
    fonts: Option<IDWriteFontCollection>,
    rt: Option<ID2D1HwndRenderTarget>,
    brush: Option<ID2D1SolidColorBrush>,
    fmt: Option<Formats>,
    /// Theme generation the formats and the target were made for.
    theme_gen: u32,
    /// Resolution and rendering mode of the current target.
    dpi: u32,
    gpu: bool,
}

fn col(c: Color) -> D2D1_COLOR_F {
    D2D1_COLOR_F { r: c.r, g: c.g, b: c.b, a: c.a }
}

fn d2r(r: Rect) -> D2D_RECT_F {
    D2D_RECT_F { left: r.left, top: r.top, right: r.right, bottom: r.bottom }
}

fn utf16(s: &str) -> Vec<u16> {
    s.encode_utf16().collect()
}

/// Segoe MDL2 Assets / Segoe Fluent Icons code points.
fn icon_glyph(icon: Icon) -> &'static str {
    match icon {
        Icon::Search => "\u{E721}",
        Icon::Recent => "\u{E823}",
        Icon::Close => "\u{E711}",
        Icon::Prev => "\u{E76B}",
        Icon::Next => "\u{E76C}",
        Icon::Keyboard => "\u{E765}",
    }
}

impl Gfx {
    pub fn new(hwnd: HWND) -> windows_core::Result<Gfx> {
        let d2d: ID2D1Factory = unsafe { D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)? };
        let dw: IDWriteFactory = unsafe { DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)? };
        let mut fonts = None;
        unsafe {
            let _ = dw.GetSystemFontCollection(&mut fonts, false);
        }
        Ok(Gfx { hwnd, d2d, dw, fonts, rt: None, brush: None, fmt: None, theme_gen: u32::MAX, dpi: 96, gpu: false })
    }

    pub fn font_exists(&self, family: &str) -> bool {
        let Some(c) = &self.fonts else { return true };
        let (mut index, mut exists) = (0u32, BOOL(0));
        let name = wide(family);
        unsafe { c.FindFamilyName(PCWSTR(name.as_ptr()), &mut index, &mut exists).is_ok() && exists.as_bool() }
    }

    /// Drop the render target, e.g. after the display device was lost.
    pub fn reset_device(&mut self) {
        self.rt = None;
        self.brush = None;
    }

    fn make_format(&self, s: &FontSpec) -> windows_core::Result<IDWriteTextFormat> {
        let fam = wide(&s.family);
        let weight =
            if s.weight == Weight::SemiBold { DWRITE_FONT_WEIGHT_SEMI_BOLD } else { DWRITE_FONT_WEIGHT_NORMAL };
        unsafe {
            let f = self.dw.CreateTextFormat(
                PCWSTR(fam.as_ptr()),
                None,
                weight,
                DWRITE_FONT_STYLE_NORMAL,
                DWRITE_FONT_STRETCH_NORMAL,
                s.size,
                windows_core::w!(""),
            )?;
            f.SetTextAlignment(if s.align == Align::Center {
                DWRITE_TEXT_ALIGNMENT_CENTER
            } else {
                DWRITE_TEXT_ALIGNMENT_LEADING
            })?;
            if s.wrap {
                f.SetWordWrapping(DWRITE_WORD_WRAPPING_WRAP)?;
                f.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_NEAR)?;
            } else {
                f.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP)?;
                f.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER)?;
            }
            if s.ellipsis {
                let sign = self.dw.CreateEllipsisTrimmingSign(&f)?;
                let trim = DWRITE_TRIMMING {
                    granularity: DWRITE_TRIMMING_GRANULARITY_CHARACTER,
                    delimiter: 0,
                    delimiterCount: 0,
                };
                f.SetTrimming(&trim, &sign)?;
            }
            Ok(f)
        }
    }

    fn make_formats(&self, t: &Theme) -> windows_core::Result<Formats> {
        let mut text = HashMap::new();
        for f in Font::ALL {
            text.insert(f, self.make_format(&t.font(f))?);
        }
        let mut icon = HashMap::new();
        for size in ICON_SIZES {
            let spec = FontSpec {
                family: t.icon_font.clone(),
                size: t.icon_size(size),
                weight: Weight::Normal,
                align: Align::Center,
                wrap: false,
                ellipsis: false,
            };
            icon.insert(size, self.make_format(&spec)?);
        }
        Ok(Formats { text, icon })
    }

    /// Bring fonts and the render target up to date with the picker.
    fn ensure(&mut self, p: &Picker) -> windows_core::Result<()> {
        if self.theme_gen != p.theme_generation() {
            // Fonts, colors and the window size may all have changed.
            self.fmt = None;
            self.reset_device();
            self.theme_gen = p.theme_generation();
        }
        if self.gpu != p.gpu {
            self.reset_device();
            self.gpu = p.gpu;
        }
        if self.fmt.is_none() {
            self.fmt = Some(self.make_formats(&p.theme)?);
        }
        if let Some(rt) = &self.rt {
            if self.dpi != p.dpi() {
                self.dpi = p.dpi();
                let (w, h) = p.size_px(self.dpi);
                unsafe {
                    rt.SetDpi(self.dpi as f32, self.dpi as f32);
                    let _ = rt.Resize(&D2D_SIZE_U { width: w.max(1) as u32, height: h.max(1) as u32 });
                }
            }
            return Ok(());
        }
        self.dpi = p.dpi();
        let (w, h) = p.size_px(self.dpi);
        let props = D2D1_RENDER_TARGET_PROPERTIES {
            r#type: if self.gpu { D2D1_RENDER_TARGET_TYPE_DEFAULT } else { D2D1_RENDER_TARGET_TYPE_SOFTWARE },
            pixelFormat: D2D1_PIXEL_FORMAT {
                format: DXGI_FORMAT_B8G8R8A8_UNORM,
                alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
            },
            dpiX: 0.0,
            dpiY: 0.0,
            usage: D2D1_RENDER_TARGET_USAGE_NONE,
            minLevel: D2D1_FEATURE_LEVEL_DEFAULT,
        };
        let hprops = D2D1_HWND_RENDER_TARGET_PROPERTIES {
            hwnd: self.hwnd,
            pixelSize: D2D_SIZE_U { width: w.max(1) as u32, height: h.max(1) as u32 },
            presentOptions: D2D1_PRESENT_OPTIONS_NONE,
        };
        unsafe {
            let rt = self.d2d.CreateHwndRenderTarget(&props, &hprops)?;
            rt.SetDpi(self.dpi as f32, self.dpi as f32);
            rt.SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE);
            rt.SetAntialiasMode(D2D1_ANTIALIAS_MODE_PER_PRIMITIVE);
            self.brush = Some(rt.CreateSolidColorBrush(&col(p.theme.c.text), None)?);
            self.rt = Some(rt);
        }
        Ok(())
    }

    /// Make sure every device resource exists (used to warm up before the first show).
    pub fn prepare(&mut self, p: &Picker) {
        let _ = self.ensure(p);
    }

    pub fn render(&mut self, p: &Picker) {
        if self.ensure(p).is_err() {
            return;
        }
        let (Some(rt), Some(brush), Some(fmt)) = (&self.rt, &self.brush, &self.fmt) else { return };
        let result = unsafe {
            rt.BeginDraw();
            rt.Clear(Some(&col(p.clear_color())));
            p.draw(&mut D2dCanvas { rt, brush, fmt, dw: &self.dw });
            rt.EndDraw(None, None)
        };
        if let Err(e) = result
            && e.code() == D2DERR_RECREATE_TARGET
        {
            self.reset_device();
        }
    }

    /// Render the panel into a PNG file (development aid for checking the look
    /// without a visible desktop). The backdrop is approximated by its solid color.
    #[cfg(feature = "snapshot")]
    pub fn snapshot(&mut self, p: &Picker, path: &str) -> windows_core::Result<()> {
        use windows::Win32::Foundation::GENERIC_WRITE;
        use windows::Win32::Graphics::Imaging::{
            CLSID_WICImagingFactory, GUID_ContainerFormatPng, GUID_WICPixelFormat32bppPBGRA, IWICImagingFactory,
            WICBitmapCacheOnLoad, WICBitmapEncoderNoCache,
        };
        use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance};

        let fmt = self.make_formats(&p.theme)?;
        let (w, h) = p.size_px(p.dpi());
        unsafe {
            let wic: IWICImagingFactory = CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER)?;
            let bitmap = wic.CreateBitmap(w as u32, h as u32, &GUID_WICPixelFormat32bppPBGRA, WICBitmapCacheOnLoad)?;
            let props = D2D1_RENDER_TARGET_PROPERTIES {
                r#type: D2D1_RENDER_TARGET_TYPE_DEFAULT,
                pixelFormat: D2D1_PIXEL_FORMAT {
                    format: DXGI_FORMAT_B8G8R8A8_UNORM,
                    alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
                },
                dpiX: p.dpi() as f32,
                dpiY: p.dpi() as f32,
                usage: D2D1_RENDER_TARGET_USAGE_NONE,
                minLevel: D2D1_FEATURE_LEVEL_DEFAULT,
            };
            let rt = self.d2d.CreateWicBitmapRenderTarget(&bitmap, &props)?;
            rt.SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE);
            let brush = rt.CreateSolidColorBrush(&col(p.theme.c.text), None)?;
            rt.BeginDraw();
            rt.Clear(Some(&col(p.theme.c.solid)));
            p.draw(&mut D2dCanvas { rt: &rt, brush: &brush, fmt: &fmt, dw: &self.dw });
            rt.EndDraw(None, None)?;

            let stream = wic.CreateStream()?;
            let file = wide(path);
            stream.InitializeFromFilename(PCWSTR(file.as_ptr()), GENERIC_WRITE.0)?;
            let encoder = wic.CreateEncoder(&GUID_ContainerFormatPng, std::ptr::null())?;
            encoder.Initialize(&stream, WICBitmapEncoderNoCache)?;
            let mut frame = None;
            encoder.CreateNewFrame(&mut frame, std::ptr::null_mut())?;
            let frame = frame.ok_or_else(windows_core::Error::empty)?;
            frame.Initialize(None)?;
            frame.SetSize(w as u32, h as u32)?;
            let mut format = GUID_WICPixelFormat32bppPBGRA;
            frame.SetPixelFormat(&mut format)?;
            frame.WriteSource(&bitmap, std::ptr::null())?;
            frame.Commit()?;
            encoder.Commit()?;
        }
        Ok(())
    }
}

struct D2dCanvas<'a> {
    rt: &'a ID2D1RenderTarget,
    brush: &'a ID2D1SolidColorBrush,
    fmt: &'a Formats,
    dw: &'a IDWriteFactory,
}

impl D2dCanvas<'_> {
    fn draw_text(&self, s: &str, fmt: &IDWriteTextFormat, r: Rect, c: Color) {
        unsafe {
            self.brush.SetColor(&col(c));
            self.rt.DrawText(
                &utf16(s),
                fmt,
                &d2r(r),
                self.brush,
                D2D1_DRAW_TEXT_OPTIONS_NONE,
                DWRITE_MEASURING_MODE_NATURAL,
            );
        }
    }
}

impl Canvas for D2dCanvas<'_> {
    fn fill(&mut self, r: Rect, radius: f32, c: Color) {
        unsafe {
            self.brush.SetColor(&col(c));
            if radius > 0.0 {
                self.rt.FillRoundedRectangle(
                    &D2D1_ROUNDED_RECT { rect: d2r(r), radiusX: radius, radiusY: radius },
                    self.brush,
                );
            } else {
                self.rt.FillRectangle(&d2r(r), self.brush);
            }
        }
    }

    fn stroke(&mut self, r: Rect, radius: f32, width: f32, c: Color) {
        unsafe {
            self.brush.SetColor(&col(c));
            if radius > 0.0 {
                let rr = D2D1_ROUNDED_RECT { rect: d2r(r), radiusX: radius, radiusY: radius };
                self.rt.DrawRoundedRectangle(&rr, self.brush, width, None);
            } else {
                self.rt.DrawRectangle(&d2r(r), self.brush, width, None);
            }
        }
    }

    fn text(&mut self, s: &str, font: Font, r: Rect, c: Color) {
        self.draw_text(s, &self.fmt.text[&font], r, c);
    }

    fn icon(&mut self, icon: Icon, size: IconSize, r: Rect, c: Color) {
        self.draw_text(icon_glyph(icon), &self.fmt.icon[&size], r, c);
    }

    fn measure(&mut self, s: &str, font: Font) -> f32 {
        unsafe {
            let Ok(layout) = self.dw.CreateTextLayout(&utf16(s), &self.fmt.text[&font], 4000.0, 100.0) else {
                return 0.0;
            };
            let mut m = DWRITE_TEXT_METRICS::default();
            if layout.GetMetrics(&mut m).is_ok() { m.widthIncludingTrailingWhitespace } else { 0.0 }
        }
    }

    fn push_clip(&mut self, r: Rect) {
        unsafe { self.rt.PushAxisAlignedClip(&d2r(r), D2D1_ANTIALIAS_MODE_PER_PRIMITIVE) }
    }

    fn pop_clip(&mut self) {
        unsafe { self.rt.PopAxisAlignedClip() }
    }
}
