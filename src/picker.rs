//! The picker panel: layout, drawing, keyboard and mouse handling.
//!
//! Platform code feeds it input and renders it through a [`Canvas`]. On Windows the
//! window never takes focus: keys reach it through the low-level hook, so the
//! application being typed into keeps its caret and selection.

use std::time::Instant;

use crate::data::{Catalog, ViewSection};
use crate::gfx::{Canvas, Color, Font, Icon, IconSize, Rect, rect};
use crate::settings::{Hotkey, Settings, WIN_NAME, key_name};
use crate::theme::{Backdrop, Kind, Theme};

const SECTION_GAP: f32 = 6.0;
const NAV_ARROW_W: f32 = 18.0;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Hit {
    None,
    Cell(usize),
    Tab(usize),
    Close,
    NavPrev,
    NavNext,
    Other,
}

pub enum Action {
    None,
    Redraw,
    Insert {
        text: String,
        key: String,
        /// On Linux the picker always closes, since typing needs the keyboard back.
        #[cfg_attr(not(windows), allow(dead_code))]
        close: bool,
    },
    Close,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Nav {
    Left,
    Right,
    Up,
    Down,
    Home,
    End,
    PageUp,
    PageDown,
    NextSection,
    PrevSection,
}

/// A full-panel message shown instead of the characters, used while recording a
/// new shortcut.
#[derive(Clone, Debug, Default)]
pub struct Prompt {
    pub title: String,
    pub text: String,
    /// Key caps drawn in the middle, such as `["Ctrl", "Alt", "E"]`.
    pub keys: Vec<String>,
    /// Small line at the bottom.
    pub hint: String,
    /// Draw the hint as a warning.
    pub warn: bool,
}

impl Prompt {
    /// The shortcut recorder, showing the keys held so far.
    pub fn record_shortcut(current: &Hotkey, held: Hotkey) -> Prompt {
        Prompt {
            title: "Press the new shortcut".into(),
            text: format!("Hold Ctrl, Alt or {WIN_NAME} and press a key. Esc cancels."),
            keys: held.parts(),
            hint: if current.is_none() {
                "No shortcut is set".into()
            } else {
                format!("Current shortcut: {}", current.label())
            },
            warn: false,
        }
    }
}

/// What a key means to the shortcut recorder.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Recorded {
    /// Keep listening.
    Pending,
    Cancel,
    Done(Hotkey),
}

struct Item {
    sec: usize,
    idx: usize,
    entry: u32,
}

struct SecLayout {
    y: f32,
    start: usize,
    count: usize,
}

struct Anim {
    from: f32,
    to: f32,
    start: Instant,
}

const ANIM_SECS: f32 = 0.14;

pub struct Picker {
    pub catalog: Catalog,
    pub order: Vec<usize>,
    pub recent: Vec<u32>,
    pub theme: Theme,
    theme_gen: u32,
    pub gpu: bool,
    pub close_on_click: bool,
    /// The window system draws no background or border for the panel (Linux), so
    /// the picker paints them itself.
    pub own_frame: bool,
    dpi: u32,
    view: Vec<ViewSection>,
    secs: Vec<SecLayout>,
    items: Vec<Item>,
    content_h: f32,
    sel: usize,
    hover: Hit,
    pressed: Hit,
    scroll: f32,
    anim: Option<Anim>,
    nav_scroll: f32,
    pinned_tab: Option<usize>,
    pub query: String,
    pub upper: bool,
    pub caret_on: bool,
    /// Replaces the character description in the info bar.
    pub banner: Option<String>,
    /// Replaces the whole content while set.
    pub prompt: Option<Prompt>,
}

fn ease_out(t: f32) -> f32 {
    1.0 - (1.0 - t).powi(3)
}

impl Picker {
    pub fn new(catalog: Catalog, settings: &Settings, theme: Theme) -> Picker {
        let order = catalog.resolve_order(&settings.order);
        Picker {
            catalog,
            order,
            recent: Vec::new(),
            theme,
            theme_gen: 0,
            gpu: settings.gpu,
            close_on_click: settings.close_on_click,
            own_frame: false,
            dpi: 96,
            view: Vec::new(),
            secs: Vec::new(),
            items: Vec::new(),
            content_h: 0.0,
            sel: 0,
            hover: Hit::None,
            pressed: Hit::None,
            scroll: 0.0,
            anim: None,
            nav_scroll: 0.0,
            pinned_tab: None,
            query: String::new(),
            upper: false,
            caret_on: true,
            banner: None,
            prompt: None,
        }
    }

    /// Re-read everything that depends on the settings, with the theme resolved from them.
    pub fn apply_settings(&mut self, s: &Settings, theme: Theme) {
        self.order = self.catalog.resolve_order(&s.order);
        self.gpu = s.gpu;
        self.close_on_click = s.close_on_click;
        self.set_theme(theme);
    }

    pub fn set_theme(&mut self, theme: Theme) {
        self.theme = theme;
        // The window size, fonts and colors may have changed.
        self.theme_gen = self.theme_gen.wrapping_add(1);
        self.relayout();
    }

    /// Changes with every theme change, so renderers know to rebuild their fonts.
    #[cfg_attr(not(windows), allow(dead_code))]
    pub fn theme_generation(&self) -> u32 {
        self.theme_gen
    }

    #[cfg_attr(not(windows), allow(dead_code))]
    pub fn dpi(&self) -> u32 {
        self.dpi
    }

    #[cfg_attr(not(windows), allow(dead_code))]
    pub fn size_px(&self, dpi: u32) -> (i32, i32) {
        let s = dpi as f32 / 96.0;
        ((self.theme.m.width * s).round() as i32, (self.theme.m.height * s).round() as i32)
    }

    /// The renderer picks the new resolution up on its next frame.
    pub fn set_dpi(&mut self, dpi: u32) {
        self.dpi = dpi.max(1);
    }

    /// What the renderer clears the window to before drawing.
    #[cfg_attr(not(windows), allow(dead_code))]
    pub fn clear_color(&self) -> Color {
        if self.own_frame || self.theme.backdrop != Backdrop::Solid { Color::TRANSPARENT } else { self.theme.c.solid }
    }

    /// Reset to the default view; called every time the picker opens.
    pub fn reset(&mut self) {
        self.query.clear();
        self.banner = None;
        self.hover = Hit::None;
        self.pressed = Hit::None;
        self.rebuild_view();
    }

    fn rebuild_view(&mut self) {
        self.view = if self.query.trim().is_empty() {
            self.catalog.browse(&self.order, &self.recent)
        } else {
            self.catalog.search(&self.query, &self.order)
        };
        self.sel = 0;
        self.scroll = 0.0;
        self.anim = None;
        self.nav_scroll = 0.0;
        self.pinned_tab = None;
        self.hover = Hit::None;
        self.relayout();
    }

    fn relayout(&mut self) {
        let m = self.theme.m;
        self.secs.clear();
        self.items.clear();
        let mut y = 0.0;
        for (si, v) in self.view.iter().enumerate() {
            let start = self.items.len();
            for (idx, &entry) in v.entries.iter().enumerate() {
                self.items.push(Item { sec: si, idx, entry });
            }
            self.secs.push(SecLayout { y, start, count: v.entries.len() });
            let rows = v.entries.len().div_ceil(m.cols);
            y += m.section_h + rows as f32 * m.cell + SECTION_GAP;
        }
        self.content_h = y;
        self.sel = self.sel.min(self.items.len().saturating_sub(1));
        self.clamp_scroll();
    }

    // -----------------------------------------------------------------------
    // Geometry (DIPs, client coordinates)

    fn content_top(&self) -> f32 {
        self.theme.m.top_h
    }

    fn content_bottom(&self) -> f32 {
        let m = &self.theme.m;
        m.height - m.info_h - m.nav_h
    }

    fn view_h(&self) -> f32 {
        self.content_bottom() - self.content_top()
    }

    fn max_scroll(&self) -> f32 {
        (self.content_h - self.view_h()).max(0.0)
    }

    fn clamp_scroll(&mut self) {
        self.scroll = self.scroll.clamp(0.0, self.max_scroll());
    }

    fn scroll_target(&self) -> f32 {
        self.anim.as_ref().map_or(self.scroll, |a| a.to)
    }

    fn item_rect(&self, i: usize) -> Rect {
        let m = &self.theme.m;
        let it = &self.items[i];
        let s = &self.secs[it.sec];
        let (row, col) = (it.idx / m.cols, it.idx % m.cols);
        let x = m.pad + col as f32 * m.cell;
        let y = self.content_top() + s.y + m.section_h + row as f32 * m.cell - self.scroll;
        rect(x, y, x + m.cell, y + m.cell)
    }

    /// Left edge of the tab strip. When the tabs do not fit, both ends make room
    /// for a scroll arrow.
    fn nav_x0(&self) -> f32 {
        let base = if self.theme.kind == Kind::Win10 { 4.0 } else { 8.0 };
        if self.nav_total_w() > self.theme.m.width - 2.0 * base { NAV_ARROW_W } else { base }
    }

    fn nav_visible_w(&self) -> f32 {
        self.theme.m.width - 2.0 * self.nav_x0()
    }

    fn nav_total_w(&self) -> f32 {
        self.view.len() as f32 * self.theme.m.nav_tab_w
    }

    fn close_rect(&self) -> Option<Rect> {
        let m = &self.theme.m;
        (self.theme.kind == Kind::Win10).then(|| rect(m.width - m.top_h, 0.0, m.width, m.top_h))
    }

    fn search_rect(&self) -> Rect {
        let m = &self.theme.m;
        match self.theme.kind {
            Kind::Win10 => rect(0.0, 0.0, m.width - m.top_h, m.top_h),
            Kind::Win11 | Kind::Kde => rect(12.0, 12.0, m.width - 12.0, 44.0),
        }
    }

    /// The section highlighted in the navigation bar.
    fn active_section(&self) -> usize {
        if let Some(p) = self.pinned_tab {
            return p.min(self.secs.len().saturating_sub(1));
        }
        let top = self.scroll_target() + 1.0;
        let mut active = 0;
        for (i, s) in self.secs.iter().enumerate() {
            if s.y <= top {
                active = i;
            }
        }
        active
    }

    pub fn hit_test(&self, x: f32, y: f32) -> Hit {
        let m = &self.theme.m;
        if let Some(r) = self.close_rect()
            && x >= r.left
            && x < r.right
            && y >= r.top
            && y < r.bottom
        {
            return Hit::Close;
        }
        if self.prompt.is_some() {
            return Hit::Other;
        }
        if y >= m.height - m.nav_h {
            let lx = x - self.nav_x0() + self.nav_scroll;
            if x >= self.nav_x0() && x < m.width - self.nav_x0() && lx >= 0.0 {
                let i = (lx / m.nav_tab_w) as usize;
                if i < self.view.len() {
                    return Hit::Tab(i);
                }
            }
            if self.nav_x0() == NAV_ARROW_W {
                if x < NAV_ARROW_W {
                    return Hit::NavPrev;
                }
                if x >= m.width - NAV_ARROW_W {
                    return Hit::NavNext;
                }
            }
            return Hit::Other;
        }
        if y >= self.content_top() && y < self.content_bottom() && x >= m.pad && x < m.pad + m.cols as f32 * m.cell {
            let cy = y - self.content_top() + self.scroll;
            for s in &self.secs {
                let grid_top = s.y + m.section_h;
                let rows = s.count.div_ceil(m.cols);
                if cy >= grid_top && cy < grid_top + rows as f32 * m.cell {
                    let row = ((cy - grid_top) / m.cell) as usize;
                    let col = ((x - m.pad) / m.cell) as usize;
                    let idx = row * m.cols + col;
                    if col < m.cols && idx < s.count {
                        return Hit::Cell(s.start + idx);
                    }
                }
            }
        }
        Hit::Other
    }

    // -----------------------------------------------------------------------
    // Scrolling

    pub fn scroll_to(&mut self, target: f32, animate: bool) {
        let target = target.clamp(0.0, self.max_scroll());
        if !animate || (target - self.scroll).abs() < 0.5 {
            self.scroll = target;
            self.anim = None;
        } else {
            self.anim = Some(Anim { from: self.scroll, to: target, start: Instant::now() });
        }
    }

    pub fn animating(&self) -> bool {
        self.anim.is_some()
    }

    /// Advance the scroll animation. Returns false when it has finished.
    pub fn tick(&mut self) -> bool {
        let Some(a) = &self.anim else { return false };
        let t = (a.start.elapsed().as_secs_f32() / ANIM_SECS).min(1.0);
        self.scroll = a.from + (a.to - a.from) * ease_out(t);
        if t >= 1.0 {
            self.anim = None;
        }
        self.anim.is_some()
    }

    pub fn wheel(&mut self, delta: i16, horizontal: bool, y: f32) -> Action {
        let m = self.theme.m;
        let steps = delta as f32 / 120.0;
        if horizontal || y >= m.height - m.nav_h {
            let max = (self.nav_total_w() - self.nav_visible_w()).max(0.0);
            let dir = if horizontal { steps } else { -steps };
            self.nav_scroll = (self.nav_scroll + dir * m.nav_tab_w * 2.0).clamp(0.0, max);
            return Action::Redraw;
        }
        self.pinned_tab = None;
        let target = self.scroll_target() - steps * m.cell * 2.5;
        self.scroll_to(target, true);
        Action::Redraw
    }

    fn ensure_visible(&mut self, i: usize) {
        if i >= self.items.len() {
            return;
        }
        let m = self.theme.m;
        let it = &self.items[i];
        let s = &self.secs[it.sec];
        let row = it.idx / m.cols;
        let top = if row == 0 { s.y } else { s.y + m.section_h + row as f32 * m.cell };
        let bottom = s.y + m.section_h + (row + 1) as f32 * m.cell;
        let target = self.scroll_target();
        if top < target {
            self.scroll_to(top, true);
        } else if bottom > target + self.view_h() {
            self.scroll_to(bottom - self.view_h() + SECTION_GAP, true);
        }
    }

    fn ensure_tab_visible(&mut self, t: usize) {
        let w = self.theme.m.nav_tab_w;
        let (l, r) = (t as f32 * w, (t + 1) as f32 * w);
        let vis = self.nav_visible_w();
        if l < self.nav_scroll + w * 0.5 {
            self.nav_scroll = l - w;
        } else if r > self.nav_scroll + vis - w * 0.5 {
            self.nav_scroll = r - vis + w;
        }
        self.nav_scroll = self.nav_scroll.clamp(0.0, (self.nav_total_w() - vis).max(0.0));
    }

    // -----------------------------------------------------------------------
    // Keyboard

    pub fn navigate(&mut self, nav: Nav) -> Action {
        let n = self.items.len();
        if n == 0 {
            return Action::None;
        }
        let cols = self.theme.m.cols;
        let it = &self.items[self.sel];
        let (sec, idx) = (it.sec, it.idx);
        let s = &self.secs[sec];
        let down = |sel: usize, items: &Vec<Item>, secs: &Vec<SecLayout>| -> usize {
            let it = &items[sel];
            let s = &secs[it.sec];
            if it.idx + cols < s.count {
                sel + cols
            } else if it.idx / cols + 1 < s.count.div_ceil(cols) {
                s.start + s.count - 1
            } else if let Some(ns) = secs[it.sec + 1..].iter().find(|x| x.count > 0) {
                ns.start + (it.idx % cols).min(ns.count - 1)
            } else {
                sel
            }
        };
        let up = |sel: usize, items: &Vec<Item>, secs: &Vec<SecLayout>| -> usize {
            let it = &items[sel];
            if it.idx >= cols {
                sel - cols
            } else if let Some(ps) = secs[..it.sec].iter().rev().find(|x| x.count > 0) {
                let last_row = (ps.count - 1) / cols;
                ps.start + (last_row * cols + it.idx % cols).min(ps.count - 1)
            } else {
                sel
            }
        };
        let page = ((self.view_h() / self.theme.m.cell) as usize).saturating_sub(1).max(1);
        self.sel = match nav {
            Nav::Left => self.sel.saturating_sub(1),
            Nav::Right => (self.sel + 1).min(n - 1),
            Nav::Down => down(self.sel, &self.items, &self.secs),
            Nav::Up => up(self.sel, &self.items, &self.secs),
            Nav::Home => 0,
            Nav::End => n - 1,
            Nav::PageDown => (0..page).fold(self.sel, |s, _| down(s, &self.items, &self.secs)),
            Nav::PageUp => (0..page).fold(self.sel, |s, _| up(s, &self.items, &self.secs)),
            Nav::NextSection => self.secs[sec + 1..].iter().find(|x| x.count > 0).map_or(self.sel, |x| x.start),
            Nav::PrevSection => {
                if idx > 0 {
                    s.start
                } else {
                    self.secs[..sec].iter().rev().find(|x| x.count > 0).map_or(self.sel, |x| x.start)
                }
            }
        };
        let new_sec = self.items[self.sel].sec;
        self.pinned_tab = Some(new_sec);
        self.ensure_tab_visible(new_sec);
        self.ensure_visible(self.sel);
        Action::Redraw
    }

    pub fn type_char(&mut self, c: char) -> Action {
        if c == ' ' && (self.query.is_empty() || self.query.ends_with(' ')) {
            return Action::None;
        }
        if self.query.chars().count() >= 40 {
            return Action::None;
        }
        self.query.push(c);
        self.rebuild_view();
        Action::Redraw
    }

    pub fn backspace(&mut self, word: bool) -> Action {
        if self.query.is_empty() {
            return Action::None;
        }
        if word {
            let trimmed = self.query.trim_end();
            let cut = trimmed.rfind(' ').map_or(0, |i| i + 1);
            self.query.truncate(cut);
        } else {
            self.query.pop();
        }
        self.rebuild_view();
        Action::Redraw
    }

    /// Escape clears the search first, then closes.
    pub fn escape(&mut self) -> Action {
        if self.query.is_empty() {
            Action::Close
        } else {
            self.query.clear();
            self.rebuild_view();
            Action::Redraw
        }
    }

    /// Delete clears the whole search.
    pub fn clear_query(&mut self) -> Action {
        if self.query.is_empty() {
            return Action::None;
        }
        self.query.clear();
        self.rebuild_view();
        Action::Redraw
    }

    pub fn enter(&mut self) -> Action {
        self.insert(self.sel, true)
    }

    fn insert(&self, i: usize, close: bool) -> Action {
        let Some(it) = self.items.get(i) else { return Action::None };
        let e = &self.catalog.entries[it.entry as usize];
        Action::Insert { text: e.text(self.upper).to_string(), key: e.lower.clone(), close }
    }

    // -----------------------------------------------------------------------
    // Mouse

    pub fn mouse_move(&mut self, x: f32, y: f32) -> Action {
        let h = self.hit_test(x, y);
        if h != self.hover {
            self.hover = h;
            return Action::Redraw;
        }
        Action::None
    }

    pub fn mouse_leave(&mut self) -> Action {
        self.pressed = Hit::None;
        if self.hover != Hit::None {
            self.hover = Hit::None;
            return Action::Redraw;
        }
        Action::None
    }

    pub fn mouse_down(&mut self, x: f32, y: f32) -> Action {
        self.pressed = self.hit_test(x, y);
        Action::Redraw
    }

    pub fn mouse_up(&mut self, x: f32, y: f32) -> Action {
        let h = self.hit_test(x, y);
        let pressed = std::mem::replace(&mut self.pressed, Hit::None);
        if h != pressed {
            return Action::Redraw;
        }
        match h {
            Hit::Cell(i) => {
                self.sel = i;
                self.pinned_tab = Some(self.items[i].sec);
                self.insert(i, self.close_on_click)
            }
            Hit::Tab(t) => {
                self.jump_to_section(t);
                Action::Redraw
            }
            Hit::Close => Action::Close,
            Hit::NavPrev | Hit::NavNext => {
                let step = if h == Hit::NavPrev { -1.0 } else { 1.0 } * self.nav_visible_w() * 0.75;
                let max = (self.nav_total_w() - self.nav_visible_w()).max(0.0);
                self.nav_scroll = (self.nav_scroll + step).clamp(0.0, max);
                Action::Redraw
            }
            _ => Action::Redraw,
        }
    }

    fn jump_to_section(&mut self, t: usize) {
        let Some(s) = self.secs.get(t) else { return };
        if s.count > 0 {
            self.sel = s.start;
        }
        let y = s.y;
        self.pinned_tab = Some(t);
        self.ensure_tab_visible(t);
        self.scroll_to(y, true);
    }

    /// Show the shortcut recorder instead of the characters.
    pub fn record_start(&mut self, current: Hotkey) {
        self.prompt = Some(Prompt::record_shortcut(&current, Hotkey::NONE));
        self.hover = Hit::None;
        self.pressed = Hit::None;
    }

    /// Feed the recorder: the modifiers held and the key pressed (`vk` 0 when only
    /// the modifiers changed).
    pub fn record(&mut self, hk: Hotkey) -> Recorded {
        let Some(p) = &mut self.prompt else { return Recorded::Pending };
        if hk.vk == 0x1B && hk.mods == 0 {
            return Recorded::Cancel;
        }
        if hk.vk == 0 {
            // Keep a rejected combination on screen until a new one is started.
            if !p.warn || hk.mods != 0 {
                p.keys = hk.parts();
            }
            return Recorded::Pending;
        }
        p.keys = hk.parts();
        if hk.valid() {
            return Recorded::Done(hk);
        }
        p.hint = format!("{} needs Ctrl, Alt or {WIN_NAME} with it", key_name(hk.vk));
        p.warn = true;
        Recorded::Pending
    }

    /// The recorded shortcut is in place.
    pub fn record_saved(&mut self, hk: Hotkey) {
        if let Some(p) = &mut self.prompt {
            p.title = "Shortcut saved".into();
            p.text = format!("Press {} in any text field to open Charpick.", hk.label());
            p.keys = hk.parts();
            p.hint = String::new();
            p.warn = false;
        }
    }

    /// Show a problem with the shortcut just pressed (or just saved).
    #[cfg_attr(windows, allow(dead_code))]
    pub fn record_warn(&mut self, keys: Vec<String>, hint: &str) {
        if let Some(p) = &mut self.prompt {
            p.keys = keys;
            p.hint = hint.into();
            p.warn = true;
        }
    }

    // -----------------------------------------------------------------------
    // Drawing

    /// Simulate the pointer resting on a cell (snapshots only).
    #[cfg(feature = "snapshot")]
    pub fn set_hover_cell(&mut self, i: usize) {
        if i < self.items.len() {
            self.hover = Hit::Cell(i);
        }
    }

    pub fn draw(&self, cv: &mut dyn Canvas) {
        let t = &self.theme;
        let (m, c) = (&t.m, &t.c);
        let px = 96.0 / self.dpi as f32;
        let full = rect(0.0, 0.0, m.width, m.height);
        if self.own_frame {
            cv.fill(full, m.radius, if t.backdrop == Backdrop::Solid { c.solid } else { c.tint });
        }
        if let Some(p) = &self.prompt {
            self.draw_prompt(cv, p);
        } else {
            self.draw_panel(cv);
        }

        // --- Window border (Windows 10 style; Windows 11 gets one from DWM).
        if self.own_frame || t.kind == Kind::Win10 {
            let h = px / 2.0;
            cv.stroke(rect(h, h, m.width - h, m.height - h), m.radius, px, c.border);
        }
    }

    fn draw_panel(&self, cv: &mut dyn Canvas) {
        let t = &self.theme;
        let (m, c) = (&t.m, &t.c);
        let px = 96.0 / self.dpi as f32;
        let win11 = t.kind != Kind::Win10;
        let kde = t.kind == Kind::Kde;
        let stroke = |cv: &mut dyn Canvas, r: Rect, radius: f32, width: f32, col: Color| {
            let h = width / 2.0;
            cv.stroke(rect(r.left + h, r.top + h, r.right - h, r.bottom - h), radius, width, col);
        };

        // --- Search field
        let sr = self.search_rect();
        let caret_x;
        if win11 {
            let radius = if kde { m.cell_radius } else { 4.0 };
            cv.fill(sr, radius, c.field);
            if kde {
                // Breeze outlines the focused field in the highlight color.
                stroke(cv, sr, radius, px.max(1.0), c.focus);
            } else {
                stroke(cv, sr, radius, px, c.field_border);
                // Focused text fields carry an accent underline.
                cv.fill(rect(sr.left + 1.0, sr.bottom - 2.0, sr.right - 1.0, sr.bottom), 0.0, c.accent);
            }
            cv.icon(Icon::Search, IconSize::Normal, rect(sr.left + 4.0, sr.top, sr.left + 32.0, sr.bottom), c.text2);
            let tr = rect(sr.left + 34.0, sr.top, sr.right - 10.0, sr.bottom - 1.0);
            caret_x = self.draw_query(cv, tr);
        } else {
            cv.icon(Icon::Search, IconSize::Normal, rect(4.0, 0.0, 40.0, m.top_h), c.text2);
            let tr = rect(40.0, 0.0, sr.right - 4.0, m.top_h);
            caret_x = self.draw_query(cv, tr);
            if let Some(cr) = self.close_rect() {
                if self.hover == Hit::Close {
                    cv.fill(cr, 0.0, if self.pressed == Hit::Close { c.pressed } else { c.hover });
                }
                cv.icon(Icon::Close, IconSize::Normal, cr, c.text);
            }
            cv.fill(rect(0.0, m.top_h - px, m.width, m.top_h), 0.0, c.divider);
        }
        if self.caret_on {
            let (ct, cb) =
                if win11 { (sr.top + 8.0, sr.bottom - 8.0) } else { (m.top_h / 2.0 - 9.0, m.top_h / 2.0 + 9.0) };
            cv.fill(rect(caret_x, ct, caret_x + px.max(1.0), cb), 0.0, c.text);
        }

        // --- Content
        let (top, bottom) = (self.content_top(), self.content_bottom());
        cv.push_clip(rect(0.0, top, m.width, bottom));
        if self.items.is_empty() {
            let msg = if self.query.trim().is_empty() {
                "Nothing to show.".to_string()
            } else {
                format!("No characters or languages match “{}”.", self.query.trim())
            };
            cv.text(&msg, Font::Message, rect(24.0, top + 40.0, m.width - 24.0, bottom), c.text2);
        }
        for (si, s) in self.secs.iter().enumerate() {
            let y = top + s.y - self.scroll;
            let rows = s.count.div_ceil(m.cols);
            let end = y + m.section_h + rows as f32 * m.cell;
            if end < top || y > bottom {
                continue;
            }
            let v = &self.view[si];
            let hx = m.pad + if win11 { 4.0 } else { 6.0 };
            let hr = rect(hx, y, m.width - m.pad, y + m.section_h);
            cv.text(&v.title, Font::Title, hr, c.text);
            if !v.subtitle.is_empty() {
                let sx = hx + cv.measure(&v.title, Font::Title) + 8.0;
                cv.text(&v.subtitle, Font::Subtitle, rect(sx, y + 1.0, m.width - m.pad, y + m.section_h), c.text2);
            }
            for i in s.start..s.start + s.count {
                let r = self.item_rect(i);
                if r.bottom < top || r.top > bottom {
                    continue;
                }
                let cell = rect(r.left + 2.0, r.top + 2.0, r.right - 2.0, r.bottom - 2.0);
                let selected = i == self.sel;
                let hovered = self.hover == Hit::Cell(i);
                if hovered && self.pressed == Hit::Cell(i) {
                    cv.fill(cell, m.cell_radius, c.pressed);
                } else if selected && win11 {
                    cv.fill(cell, m.cell_radius, c.selected);
                } else if hovered {
                    cv.fill(cell, m.cell_radius, c.hover);
                }
                if selected {
                    if kde {
                        stroke(cv, cell, m.cell_radius, px.max(1.0), c.focus);
                    } else if win11 {
                        let outer = rect(cell.left - 1.0, cell.top - 1.0, cell.right + 1.0, cell.bottom + 1.0);
                        stroke(cv, outer, m.cell_radius + 1.0, 2.0, c.focus);
                    } else {
                        stroke(cv, cell, 0.0, 2.0, c.focus);
                    }
                }
                let e = &self.catalog.entries[self.items[i].entry as usize];
                let g = e.text(self.upper);
                let font = if g.chars().count() > 1 { Font::GlyphSmall } else { Font::Glyph };
                cv.text(g, font, rect(r.left, r.top, r.right, r.bottom - 1.0), c.text);
            }
        }
        // Scroll indicator.
        let max = self.max_scroll();
        if max > 0.0 {
            let vh = bottom - top;
            let thumb = (vh * vh / self.content_h).max(24.0);
            let ty = top + (vh - thumb) * (self.scroll / max);
            let x = m.width - if win11 { 5.0 } else { 4.0 };
            cv.fill(rect(x, ty + 2.0, x + 2.0, ty + thumb - 2.0), 1.0, c.text3);
        }
        cv.pop_clip();

        // --- Info bar
        let iy = bottom;
        let ib = rect(m.pad, iy, m.width - m.pad, iy + m.info_h);
        if let Some(b) = &self.banner {
            cv.text(b, Font::Info, ib, c.accent);
        } else {
            let shown = match self.hover {
                Hit::Cell(i) => Some(i),
                _ => (!self.items.is_empty()).then_some(self.sel),
            };
            if let Some(i) = shown {
                let id = self.items[i].entry;
                let e = &self.catalog.entries[id as usize];
                let g = e.text(self.upper);
                cv.text(g, Font::InfoGlyph, rect(ib.left, iy, ib.left + 28.0, iy + m.info_h), c.text);
                let langs = self.catalog.languages_of(id);
                let line = if langs.is_empty() {
                    e.name(self.upper).to_string()
                } else {
                    format!("{}  ·  {}", e.name(self.upper), langs)
                };
                cv.text(&line, Font::Info, rect(ib.left + 34.0, iy, ib.right, iy + m.info_h), c.text2);
            }
        }

        // --- Navigation bar
        let ny = m.height - m.nav_h;
        if !win11 {
            cv.fill(rect(0.0, ny, m.width, ny + px), 0.0, c.divider);
        }
        let x0 = self.nav_x0();
        cv.push_clip(rect(x0, ny, m.width - x0, m.height));
        let active = self.active_section();
        for (i, v) in self.view.iter().enumerate() {
            let x = x0 + i as f32 * m.nav_tab_w - self.nav_scroll;
            if x + m.nav_tab_w < x0 || x > m.width - x0 {
                continue;
            }
            let tr = rect(x, ny, x + m.nav_tab_w, m.height);
            let is_active = i == active;
            if win11 {
                let pill = rect(tr.left + 3.0, ny + 6.0, tr.right - 3.0, m.height - 6.0);
                if self.hover == Hit::Tab(i) || is_active {
                    cv.fill(pill, if kde { m.cell_radius } else { 4.0 }, if is_active { c.selected } else { c.hover });
                }
                if is_active {
                    let cx = (tr.left + tr.right) / 2.0;
                    cv.fill(rect(cx - 8.0, m.height - 9.0, cx + 8.0, m.height - 6.0), 1.5, c.accent);
                }
            } else {
                if self.hover == Hit::Tab(i) {
                    cv.fill(tr, 0.0, c.hover);
                }
                if is_active {
                    cv.fill(rect(tr.left + 6.0, m.height - 3.0, tr.right - 6.0, m.height), 0.0, c.accent);
                }
            }
            let col = if is_active { if win11 { c.text } else { c.accent } } else { c.text2 };
            let label_bottom = if win11 { m.height - 4.0 } else { m.height };
            let lr = rect(tr.left, ny, tr.right, label_bottom);
            match v.tab_icon {
                Some(icon) => cv.icon(icon, IconSize::Tab, lr, col),
                None => cv.text(&v.tab, Font::Tab, lr, col),
            }
        }
        cv.pop_clip();
        // Fade hints that more tabs are hidden on either side.
        let overflow_l = self.nav_scroll > 0.5;
        let overflow_r = self.nav_scroll < self.nav_total_w() - self.nav_visible_w() - 0.5;
        let arrow = |cv: &mut dyn Canvas, hit: Hit, on: bool, r: Rect, icon: Icon| {
            if self.hover == hit && on {
                cv.fill(rect(r.left + 1.0, ny + 8.0, r.right - 1.0, m.height - 8.0), 3.0, c.hover);
            }
            cv.icon(icon, IconSize::Small, r, if on { c.text2 } else { c.text3 });
        };
        if x0 == NAV_ARROW_W {
            arrow(cv, Hit::NavPrev, overflow_l, rect(0.0, ny, x0, m.height), Icon::Prev);
            arrow(cv, Hit::NavNext, overflow_r, rect(m.width - x0, ny, m.width, m.height), Icon::Next);
        }
    }

    /// Draw the search text (or placeholder) and return the caret x position.
    fn draw_query(&self, cv: &mut dyn Canvas, r: Rect) -> f32 {
        let c = &self.theme.c;
        if self.query.is_empty() {
            cv.push_clip(r);
            cv.text("Search characters or languages", Font::Search, r, c.text3);
            cv.pop_clip();
            r.left
        } else {
            let w = cv.measure(&self.query, Font::Search);
            let avail = r.right - r.left;
            // Keep the end of a long query visible.
            let shift = (w - avail + 2.0).max(0.0);
            cv.push_clip(r);
            let tr = rect(r.left - shift, r.top, r.left - shift + w + 100.0, r.bottom);
            cv.text(&self.query, Font::Search, tr, c.text);
            cv.pop_clip();
            r.left - shift + w + 1.0
        }
    }

    fn draw_prompt(&self, cv: &mut dyn Canvas, p: &Prompt) {
        let t = &self.theme;
        let (m, c) = (&t.m, &t.c);
        let px = 96.0 / self.dpi as f32;
        let cx = m.width / 2.0;
        if let Some(cr) = self.close_rect() {
            if self.hover == Hit::Close {
                cv.fill(cr, 0.0, if self.pressed == Hit::Close { c.pressed } else { c.hover });
            }
            cv.icon(Icon::Close, IconSize::Normal, cr, c.text);
        }

        let mut y = m.height * 0.17;
        cv.icon(Icon::Keyboard, IconSize::Large, rect(cx - 30.0, y, cx + 30.0, y + 48.0), c.text2);
        y += 62.0;
        cv.text(&p.title, Font::Heading, rect(16.0, y, m.width - 16.0, y + 30.0), c.text);
        y += 38.0;
        cv.text(&p.text, Font::Message, rect(28.0, y, m.width - 28.0, y + 60.0), c.text2);

        // Key caps, centered as a row.
        let ky = m.height * 0.6;
        let (cap_h, gap, pad) = (36.0, 22.0, 13.0);
        let caps: Vec<f32> = p.keys.iter().map(|k| (cv.measure(k, Font::Key) + 2.0 * pad).max(cap_h)).collect();
        let total = caps.iter().sum::<f32>() + gap * caps.len().saturating_sub(1) as f32;
        let mut x = cx - total / 2.0;
        for (i, (k, w)) in p.keys.iter().zip(&caps).enumerate() {
            if i > 0 {
                cv.text("+", Font::Key, rect(x - gap, ky, x, ky + cap_h), c.text3);
            }
            let r = rect(x, ky, x + w, ky + cap_h);
            cv.fill(r, 6.0, c.field);
            let h = px.max(1.0) / 2.0;
            cv.stroke(rect(r.left + h, r.top + h, r.right - h, r.bottom - h), 6.0, px.max(1.0), c.field_border);
            // A thicker bottom edge makes the rectangle read as a key.
            cv.fill(
                rect(r.left + 3.0, r.bottom - 2.0 * px.max(1.0), r.right - 3.0, r.bottom - px.max(1.0)),
                0.0,
                c.field_border,
            );
            cv.text(k, Font::Key, rect(r.left, r.top, r.right, r.bottom - 1.0), c.text);
            x += w + gap;
        }
        if p.keys.is_empty() {
            cv.text("…", Font::Key, rect(0.0, ky, m.width, ky + cap_h), c.text3);
        }

        let hy = m.height - 64.0;
        cv.text(
            &p.hint,
            Font::Message,
            rect(20.0, hy, m.width - 20.0, m.height - 12.0),
            if p.warn { c.accent } else { c.text3 },
        );
    }
}
