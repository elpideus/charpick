//! Drawing the picker with Cairo and Pango, into memory shared with the compositor.

use std::f64::consts::PI;

use cairo::{Antialias, Context, Format, HintMetrics, HintStyle, ImageSurface, LineCap, LineJoin};

use crate::gfx::{Align, Canvas, Color, Font, Icon, IconSize, Rect, Weight};
use crate::picker::Picker;
use crate::theme::Theme;

/// Room around the panel for its shadow, in device independent pixels.
pub const MARGIN: f32 = 24.0;
const SHADOW_BLUR: f64 = 12.0;
const SHADOW_DY: f64 = 4.0;

fn source(cr: &Context, c: Color) {
    cr.set_source_rgba(c.r as f64, c.g as f64, c.b as f64, c.a as f64);
}

fn rounded(cr: &Context, r: Rect, radius: f32) {
    let (l, t, w, h) = (r.left as f64, r.top as f64, r.width() as f64, r.height() as f64);
    let rad = (radius as f64).min(w / 2.0).min(h / 2.0);
    if rad <= 0.0 {
        cr.rectangle(l, t, w, h);
        return;
    }
    cr.new_sub_path();
    cr.arc(l + w - rad, t + rad, rad, -PI / 2.0, 0.0);
    cr.arc(l + w - rad, t + h - rad, rad, 0.0, PI / 2.0);
    cr.arc(l + rad, t + h - rad, rad, PI / 2.0, PI);
    cr.arc(l + rad, t + rad, rad, PI, 1.5 * PI);
    cr.close_path();
}

struct CairoCanvas<'a> {
    cr: &'a Context,
    theme: &'a Theme,
    layout: pango::Layout,
    /// One device pixel in device independent pixels.
    px: f64,
}

impl CairoCanvas<'_> {
    /// Prepare the shared layout for `s` in `font`, sized to `width` (None: no limit).
    fn set_layout(&self, s: &str, font: Font, width: Option<f32>) -> crate::gfx::FontSpec {
        let spec = self.theme.font(font);
        let mut fd = pango::FontDescription::new();
        fd.set_family(&spec.family);
        fd.set_absolute_size(spec.size as f64 * pango::SCALE as f64);
        fd.set_weight(match spec.weight {
            Weight::Normal => pango::Weight::Normal,
            Weight::SemiBold => pango::Weight::Semibold,
        });
        let l = &self.layout;
        l.set_font_description(Some(&fd));
        l.set_text(s);
        l.set_alignment(if spec.align == Align::Center { pango::Alignment::Center } else { pango::Alignment::Left });
        l.set_ellipsize(pango::EllipsizeMode::None);
        l.set_wrap(pango::WrapMode::WordChar);
        l.set_width(-1);
        if let Some(w) = width {
            if spec.wrap {
                l.set_width((w * pango::SCALE as f32) as i32);
            } else if spec.ellipsis {
                l.set_width((w * pango::SCALE as f32) as i32);
                l.set_ellipsize(pango::EllipsizeMode::End);
            }
        }
        spec
    }

    /// Logical size of the layout in device independent pixels.
    fn layout_size(&self) -> (f64, f64) {
        let (_, logical) = self.layout.extents();
        (logical.width() as f64 / pango::SCALE as f64, logical.height() as f64 / pango::SCALE as f64)
    }

    /// Stroke width for icons of size `s`: about one device pixel per 14 DIP, never thinner than a pixel.
    fn icon_line(&self, s: f64) -> f64 {
        (s / 14.0).max(self.px)
    }
}

impl Canvas for CairoCanvas<'_> {
    fn fill(&mut self, r: Rect, radius: f32, c: Color) {
        if c.a <= 0.0 {
            return;
        }
        rounded(self.cr, r, radius);
        source(self.cr, c);
        let _ = self.cr.fill();
    }

    fn stroke(&mut self, r: Rect, radius: f32, width: f32, c: Color) {
        if c.a <= 0.0 {
            return;
        }
        rounded(self.cr, r, radius);
        source(self.cr, c);
        self.cr.set_line_width(width as f64);
        let _ = self.cr.stroke();
    }

    fn text(&mut self, s: &str, font: Font, r: Rect, c: Color) {
        if s.is_empty() {
            return;
        }
        let spec = self.set_layout(s, font, Some(r.width()));
        let (w, h) = self.layout_size();
        let x = if self.layout.width() < 0 && spec.align == Align::Center {
            r.left as f64 + (r.width() as f64 - w) / 2.0
        } else {
            r.left as f64
        };
        let y = if spec.wrap { r.top as f64 } else { r.top as f64 + (r.height() as f64 - h) / 2.0 };
        source(self.cr, c);
        self.cr.move_to(x, y);
        pangocairo::functions::show_layout(self.cr, &self.layout);
    }

    fn icon(&mut self, icon: Icon, size: IconSize, r: Rect, c: Color) {
        let cr = self.cr;
        let s = self.theme.icon_size(size) as f64;
        let (cx, cy) = (((r.left + r.right) / 2.0) as f64, ((r.top + r.bottom) / 2.0) as f64);
        cr.save().ok();
        source(cr, c);
        cr.set_line_width(self.icon_line(s));
        cr.set_line_cap(LineCap::Round);
        cr.set_line_join(LineJoin::Round);
        match icon {
            Icon::Search => {
                let rad = s * 0.32;
                let (ox, oy) = (cx - s * 0.08, cy - s * 0.08);
                cr.new_sub_path();
                cr.arc(ox, oy, rad, 0.0, 2.0 * PI);
                let d = rad / std::f64::consts::SQRT_2;
                cr.move_to(ox + d, oy + d);
                cr.line_to(cx + s * 0.42, cy + s * 0.42);
            }
            Icon::Recent => {
                cr.new_sub_path();
                cr.arc(cx, cy, s * 0.43, 0.0, 2.0 * PI);
                cr.move_to(cx, cy - s * 0.26);
                cr.line_to(cx, cy);
                cr.line_to(cx + s * 0.18, cy + s * 0.1);
            }
            Icon::Close => {
                let d = s * 0.32;
                cr.move_to(cx - d, cy - d);
                cr.line_to(cx + d, cy + d);
                cr.move_to(cx + d, cy - d);
                cr.line_to(cx - d, cy + d);
            }
            Icon::Prev | Icon::Next => {
                let dir = if icon == Icon::Prev { 1.0 } else { -1.0 };
                cr.move_to(cx + dir * s * 0.2, cy - s * 0.4);
                cr.line_to(cx - dir * s * 0.2, cy);
                cr.line_to(cx + dir * s * 0.2, cy + s * 0.4);
            }
            Icon::Keyboard => {
                let (w, h) = (s * 0.96, s * 0.6);
                let body = crate::gfx::rect((cx - w / 2.0) as f32, (cy - h / 2.0) as f32, (cx + w / 2.0) as f32, (cy + h / 2.0) as f32);
                rounded(cr, body, (s * 0.08) as f32);
                let _ = cr.stroke();
                // Three rows of keys, the last one with a space bar.
                let key = s * 0.065;
                let (gx, gy) = (w / 8.0, h / 4.0);
                for row in 0..2 {
                    let y = cy - h / 2.0 + gy * (row as f64 + 1.0);
                    for col in 0..7 {
                        let x = cx - w / 2.0 + gx * (col as f64 + 1.0);
                        cr.rectangle(x - key, y - key, 2.0 * key, 2.0 * key);
                    }
                }
                let _ = cr.fill();
                let y = cy + h / 4.0;
                cr.move_to(cx - w * 0.22, y);
                cr.line_to(cx + w * 0.22, y);
            }
        }
        let _ = cr.stroke();
        cr.restore().ok();
    }

    fn measure(&mut self, s: &str, font: Font) -> f32 {
        self.set_layout(s, font, None);
        self.layout_size().0 as f32
    }

    fn push_clip(&mut self, r: Rect) {
        self.cr.save().ok();
        self.cr.rectangle(r.left as f64, r.top as f64, r.width() as f64, r.height() as f64);
        self.cr.clip();
    }

    fn pop_clip(&mut self) {
        self.cr.restore().ok();
    }
}

/// Blur an A8 image in place: three box blurs approximate a Gaussian.
fn blur_alpha(data: &mut [u8], w: usize, h: usize, stride: usize, radius: usize) {
    if radius == 0 || w == 0 || h == 0 {
        return;
    }
    let mut line = vec![0u8; w.max(h)];
    let pass = |src: &mut [u8], len: usize, at: &dyn Fn(usize) -> usize, line: &mut [u8]| {
        let win = 2 * radius + 1;
        let mut sum: usize = 0;
        for i in 0..len.min(radius + 1) {
            sum += src[at(i)] as usize;
        }
        for i in 0..len {
            line[i] = (sum / win) as u8;
            if i + radius + 1 < len {
                sum += src[at(i + radius + 1)] as usize;
            }
            if i >= radius {
                sum -= src[at(i - radius)] as usize;
            }
        }
        for i in 0..len {
            src[at(i)] = line[i];
        }
    };
    for _ in 0..3 {
        for y in 0..h {
            pass(data, w, &|i| y * stride + i, &mut line);
        }
        for x in 0..w {
            pass(data, h, &|i| i * stride + x, &mut line);
        }
    }
}

/// The panel's shadow as an alpha mask the size of the whole surface.
pub struct Shadow {
    key: (i32, i32, u32, u32),
    mask: ImageSurface,
}

fn make_shadow(p: &Picker, w: i32, h: i32, scale: f64) -> Option<ImageSurface> {
    let mut mask = ImageSurface::create(Format::A8, w, h).ok()?;
    {
        let cr = Context::new(&mask).ok()?;
        cr.scale(scale, scale);
        let m = &p.theme.m;
        let r = Rect { left: MARGIN, top: MARGIN + SHADOW_DY as f32, right: MARGIN + m.width, bottom: MARGIN + m.height + SHADOW_DY as f32 };
        rounded(&cr, r, m.radius);
        cr.set_source_rgba(0.0, 0.0, 0.0, 1.0);
        cr.fill().ok()?;
    }
    let stride = mask.stride() as usize;
    let radius = (SHADOW_BLUR * scale / 3.0).round() as usize;
    blur_alpha(&mut mask.data().ok()?, w as usize, h as usize, stride, radius);
    Some(mask)
}

/// Draw the picker, its frame and shadow into a new image of `w`×`h` device pixels
/// at `scale` device pixels per DIP.
pub fn render(p: &Picker, w: i32, h: i32, scale: f64, shadow: &mut Option<Shadow>) -> Option<ImageSurface> {
    let surface = ImageSurface::create(Format::ARgb32, w, h).ok()?;
    let cr = Context::new(&surface).ok()?;

    let key = (w, h, (scale * 1000.0) as u32, (p.theme.m.radius * 10.0) as u32);
    if shadow.as_ref().is_none_or(|s| s.key != key) {
        *shadow = make_shadow(p, w, h, scale).map(|mask| Shadow { key, mask });
    }
    if let Some(s) = shadow {
        cr.set_source_rgba(0.0, 0.0, 0.0, if p.theme.c.dark { 0.5 } else { 0.24 });
        cr.mask_surface(&s.mask, 0.0, 0.0).ok()?;
    }
    // The shadow must not show through a translucent panel.
    let m = &p.theme.m;
    cr.save().ok()?;
    cr.scale(scale, scale);
    rounded(&cr, Rect { left: MARGIN, top: MARGIN, right: MARGIN + m.width, bottom: MARGIN + m.height }, m.radius);
    cr.set_operator(cairo::Operator::Clear);
    cr.fill().ok()?;
    cr.restore().ok()?;

    cr.scale(scale, scale);
    cr.translate(MARGIN as f64, MARGIN as f64);
    let layout = pangocairo::functions::create_layout(&cr);
    let mut fo = cairo::FontOptions::new().ok()?;
    fo.set_antialias(Antialias::Gray);
    fo.set_hint_style(HintStyle::Slight);
    fo.set_hint_metrics(HintMetrics::Off);
    pangocairo::functions::context_set_font_options(&layout.context(), Some(&fo));
    layout.context_changed();
    let mut cv = CairoCanvas { cr: &cr, theme: &p.theme, layout, px: 1.0 / scale };
    p.draw(&mut cv);
    drop(cv);
    drop(cr);
    surface.flush();
    Some(surface)
}

/// The installed font families, lowercased.
pub fn font_families() -> std::collections::HashSet<String> {
    use pango::prelude::*;
    pangocairo::FontMap::default().list_families().iter().map(|f| f.name().to_lowercase()).collect()
}

/// Size of the whole surface (panel and shadow) in DIP.
pub fn logical_size(p: &Picker) -> (i32, i32) {
    let m = &p.theme.m;
    ((m.width + 2.0 * MARGIN).round() as i32, (m.height + 2.0 * MARGIN).round() as i32)
}

/// Copy `img` into a shared memory buffer laid out as `wl_shm` ARGB8888.
pub fn copy_to(img: &mut ImageSurface, dst: &mut [u8], dst_stride: usize) {
    let h = img.height() as usize;
    let row = img.width() as usize * 4;
    let stride = img.stride() as usize;
    let Ok(src) = img.data() else { return };
    for y in 0..h {
        dst[y * dst_stride..y * dst_stride + row].copy_from_slice(&src[y * stride..y * stride + row]);
    }
}

/// `--snapshot`: render the picker the way the compositor gets it.
#[cfg(feature = "snapshot")]
pub fn snapshot(p: &Picker, path: &str) -> Result<(), String> {
    let scale = p.dpi() as f64 / 96.0;
    let (lw, lh) = logical_size(p);
    let (w, h) = ((lw as f64 * scale).round() as i32, (lh as f64 * scale).round() as i32);
    let img = render(p, w, h, scale, &mut None).ok_or("rendering failed")?;
    let mut f = std::fs::File::create(path).map_err(|e| e.to_string())?;
    img.write_to_png(&mut f).map_err(|e| e.to_string())
}
