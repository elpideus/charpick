//! The Wayland side: the picker as a layer-shell panel in the middle of the screen,
//! keyboard and pointer input, and typing the chosen characters through KWin's
//! fake input protocol (or the clipboard when that is missing).

use std::collections::HashSet;
use std::io::Write;
use std::time::{Duration, SystemTime};

use calloop::timer::{TimeoutAction, Timer};
use calloop::{EventLoop, LoopHandle, RegistrationToken};
use calloop_wayland_source::WaylandSource;
use smithay_client_toolkit::background_effect::{BackgroundEffectHandler, BackgroundEffectState};
use smithay_client_toolkit::compositor::{CompositorHandler, CompositorState, FrameCallbackData, Region};
use smithay_client_toolkit::dispatch2::Dispatch2;
use smithay_client_toolkit::output::{OutputHandler, OutputState};
use smithay_client_toolkit::registry::{ProvidesRegistryState, RegistryState};
use smithay_client_toolkit::seat::keyboard::{KeyEvent, KeyboardHandler, Keymap, Keysym, Modifiers, RawModifiers};
use smithay_client_toolkit::seat::pointer::cursor_shape::CursorShapeManager;
use smithay_client_toolkit::seat::pointer::{AxisScroll, BTN_LEFT, PointerEvent, PointerEventKind, PointerHandler};
use smithay_client_toolkit::seat::{Capability, SeatHandler, SeatState};
use smithay_client_toolkit::shell::WaylandSurface;
use smithay_client_toolkit::shell::wlr_layer::{
    KeyboardInteractivity, Layer, LayerShell, LayerShellHandler, LayerSurface, LayerSurfaceConfigure,
};
use smithay_client_toolkit::shm::slot::SlotPool;
use smithay_client_toolkit::shm::{Shm, ShmHandler};
use smithay_client_toolkit::{delegate_dispatch2, delegate_registry, registry_handlers};
use wayland_client::globals::registry_queue_init;
use wayland_client::protocol::wl_keyboard::WlKeyboard;
use wayland_client::protocol::wl_output::{self, WlOutput};
use wayland_client::protocol::wl_pointer::WlPointer;
use wayland_client::protocol::wl_seat::WlSeat;
use wayland_client::protocol::wl_shm;
use wayland_client::protocol::wl_surface::WlSurface;
use wayland_client::{Connection, QueueHandle};
use wayland_protocols::ext::background_effect::v1::client::ext_background_effect_manager_v1::Capability as Effects;
use wayland_protocols::ext::background_effect::v1::client::ext_background_effect_surface_v1::ExtBackgroundEffectSurfaceV1;
use wayland_protocols::ext::data_control::v1::client::ext_data_control_device_v1::{self, ExtDataControlDeviceV1};
use wayland_protocols::ext::data_control::v1::client::ext_data_control_manager_v1::ExtDataControlManagerV1;
use wayland_protocols::ext::data_control::v1::client::ext_data_control_offer_v1::{self, ExtDataControlOfferV1};
use wayland_protocols::ext::data_control::v1::client::ext_data_control_source_v1::{self, ExtDataControlSourceV1};
use wayland_protocols::wp::cursor_shape::v1::client::wp_cursor_shape_device_v1::{Shape, WpCursorShapeDeviceV1};
use wayland_protocols::wp::fractional_scale::v1::client::wp_fractional_scale_manager_v1::WpFractionalScaleManagerV1;
use wayland_protocols::wp::fractional_scale::v1::client::wp_fractional_scale_v1::{self, WpFractionalScaleV1};
use wayland_protocols::wp::viewporter::client::wp_viewport::WpViewport;
use wayland_protocols::wp::viewporter::client::wp_viewporter::WpViewporter;
use xkbcommon::xkb;

use super::dbus::Bus;
use super::paint::{self, MARGIN};
use super::protocols::fake_input::org_kde_kwin_fake_input::OrgKdeKwinFakeInput;
use super::tray::Tray;
use crate::Options;
use crate::data::Catalog;
use crate::picker::{Action, Nav, Picker, Recorded};
use crate::settings::{Hotkey, MOD_ALT, MOD_CTRL, MOD_SHIFT, MOD_WIN, Recent, Settings};
use crate::theme::{Backdrop, Theme};

const CARET_BLINK: Duration = Duration::from_millis(530);
/// How long to wait for the keyboard before taking it (some compositors do not hand
/// it to on-demand panels by themselves).
const FOCUS_WAIT: Duration = Duration::from_millis(300);
/// Longest wait for keys to be released before typing.
const RELEASE_WAIT: Duration = Duration::from_secs(2);
/// Time for the window underneath to get the keyboard back.
const REFOCUS_WAIT: Duration = Duration::from_millis(60);

/// The picker panel while it is on screen.
struct Window {
    layer: LayerSurface,
    viewport: Option<WpViewport>,
    fractional: Option<WpFractionalScaleV1>,
    effect: Option<ExtBackgroundEffectSurfaceV1>,
    /// Size in surface (logical) pixels.
    size: (i32, i32),
    /// Device pixels per logical pixel, times 120.
    scale120: u32,
    configured: bool,
    focused: bool,
    exclusive: bool,
    frame_pending: bool,
    dirty: bool,
    shadow: Option<paint::Shadow>,
    /// Text to type once the keys are released; the panel is already hidden.
    leaving: Option<String>,
}

impl Drop for Window {
    fn drop(&mut self) {
        if let Some(v) = self.viewport.take() {
            v.destroy();
        }
        if let Some(f) = self.fractional.take() {
            f.destroy();
        }
        if let Some(e) = self.effect.take() {
            e.destroy();
        }
    }
}

#[derive(Default)]
pub(super) struct Timers {
    caret: Option<RegistrationToken>,
    focus: Option<RegistrationToken>,
    release: Option<RegistrationToken>,
    pub(super) close: Option<RegistrationToken>,
}

pub struct State {
    conn: Connection,
    qh: QueueHandle<State>,
    pub(super) handle: LoopHandle<'static, State>,
    registry: RegistryState,
    outputs: OutputState,
    seats: SeatState,
    compositor: CompositorState,
    shm: Shm,
    layer_shell: LayerShell,
    effects: BackgroundEffectState,
    fractional: Option<WpFractionalScaleManagerV1>,
    viewporter: Option<WpViewporter>,
    cursor_shapes: Option<CursorShapeManager>,
    fake_input: Option<OrgKdeKwinFakeInput>,
    data_control: Option<ExtDataControlManagerV1>,
    seat: Option<WlSeat>,
    keyboard: Option<WlKeyboard>,
    pointer: Option<WlPointer>,
    cursor: Option<WpCursorShapeDeviceV1>,
    clip_device: Option<ExtDataControlDeviceV1>,
    keymap: Option<xkb::Keymap>,
    layout: u32,
    mods: Modifiers,
    /// Keys held down while the panel has the keyboard.
    held: HashSet<u32>,
    pool: Option<SlotPool>,
    win: Option<Window>,
    last_scale120: u32,
    fonts: HashSet<String>,
    pub(super) timers: Timers,

    pub(super) opts: Options,
    pub(super) picker: Picker,
    pub(super) settings: Settings,
    pub(super) settings_time: Option<SystemTime>,
    look_time: Option<SystemTime>,
    pub(super) recent: Recent,
    pub(super) bus: Option<Bus>,
    pub(super) tray: Option<ksni::blocking::Handle<Tray>>,
    /// The shortcut recorder is listening.
    pub(super) recording: bool,
    /// KDE's shortcut, when settings.ini cannot express it (such as Meta+Ò).
    pub(super) kde_label: Option<String>,
    /// The shortcut settings.ini and Plasma last agreed on.
    pub(super) synced: Option<Hotkey>,
    pub(super) exit: bool,
}

/// Wayland objects without events, or whose events Charpick ignores.
struct Plain;

macro_rules! no_events {
    ($($iface:ty),*) => {$(
        impl Dispatch2<$iface, State> for Plain {
            fn event(&self, _: &mut State, _: &$iface, _: <$iface as wayland_client::Proxy>::Event, _: &Connection, _: &QueueHandle<State>) {}
        }
    )*};
}

no_events!(OrgKdeKwinFakeInput, WpFractionalScaleManagerV1, WpViewporter, WpViewport, ExtDataControlManagerV1);

impl Dispatch2<WpFractionalScaleV1, State> for Plain {
    fn event(&self, st: &mut State, _: &WpFractionalScaleV1, ev: wp_fractional_scale_v1::Event, _: &Connection, _: &QueueHandle<State>) {
        if let wp_fractional_scale_v1::Event::PreferredScale { scale } = ev {
            st.set_scale(scale);
        }
    }
}

/// Clipboard contents offered while Charpick cannot type.
struct ClipSource(String);

impl Dispatch2<ExtDataControlSourceV1, State> for ClipSource {
    fn event(&self, _: &mut State, src: &ExtDataControlSourceV1, ev: ext_data_control_source_v1::Event, _: &Connection, _: &QueueHandle<State>) {
        match ev {
            ext_data_control_source_v1::Event::Send { fd, .. } => {
                let _ = std::fs::File::from(fd).write_all(self.0.as_bytes());
            }
            ext_data_control_source_v1::Event::Cancelled => src.destroy(),
            _ => {}
        }
    }
}

struct ClipDevice;

impl Dispatch2<ExtDataControlDeviceV1, State> for ClipDevice {
    fn event(&self, _: &mut State, dev: &ExtDataControlDeviceV1, ev: ext_data_control_device_v1::Event, _: &Connection, _: &QueueHandle<State>) {
        match ev {
            // Charpick only writes the clipboard.
            ext_data_control_device_v1::Event::Selection { id: Some(offer) }
            | ext_data_control_device_v1::Event::PrimarySelection { id: Some(offer) } => offer.destroy(),
            ext_data_control_device_v1::Event::Finished => dev.destroy(),
            _ => {}
        }
    }

    fn event_created_child(opcode: u16, qh: &QueueHandle<State>) -> std::sync::Arc<dyn wayland_client::backend::ObjectData> {
        assert_eq!(opcode, ext_data_control_device_v1::EVT_DATA_OFFER_OPCODE);
        qh.make_data::<ExtDataControlOfferV1, _>(Plain)
    }
}

impl Dispatch2<ExtDataControlOfferV1, State> for Plain {
    fn event(&self, _: &mut State, _: &ExtDataControlOfferV1, _: ext_data_control_offer_v1::Event, _: &Connection, _: &QueueHandle<State>) {}
}

fn stop(handle: &LoopHandle<'static, State>, t: &mut Option<RegistrationToken>) {
    if let Some(t) = t.take() {
        handle.remove(t);
    }
}

/// The Windows virtual key for an unshifted keysym.
fn vk_from_keysym(ks: Keysym) -> Option<u8> {
    const SYMBOLS: &[(u8, u8)] = &[
        (b';', 0xBA),
        (b'=', 0xBB),
        (b',', 0xBC),
        (b'-', 0xBD),
        (b'.', 0xBE),
        (b'/', 0xBF),
        (b'`', 0xC0),
        (b'[', 0xDB),
        (b'\\', 0xDC),
        (b']', 0xDD),
        (b'\'', 0xDE),
    ];
    let raw = ks.raw();
    Some(match ks {
        _ if (0x61..=0x7A).contains(&raw) => (raw - 0x20) as u8,
        _ if (0x30..=0x39).contains(&raw) || (0x41..=0x5A).contains(&raw) => raw as u8,
        _ if (Keysym::F1.raw()..=Keysym::F24.raw()).contains(&raw) => 0x70 + (raw - Keysym::F1.raw()) as u8,
        Keysym::space => 0x20,
        Keysym::Return => 0x0D,
        Keysym::Tab => 0x09,
        Keysym::Insert => 0x2D,
        Keysym::Delete => 0x2E,
        Keysym::Home => 0x24,
        Keysym::End => 0x23,
        Keysym::Page_Up => 0x21,
        Keysym::Page_Down => 0x22,
        Keysym::Pause => 0x13,
        Keysym::Menu => 0x5D,
        _ => SYMBOLS.iter().find(|s| s.0 as u32 == raw)?.1,
    })
}

/// Whether Plasma would see `hk` as a different key: Shift turns `,` into `<`.
fn shift_changes_key(hk: Hotkey) -> bool {
    hk.mods & MOD_SHIFT != 0 && matches!(hk.vk, b'0'..=b'9' | 0xBA..=0xC0 | 0xDB..=0xDE)
}

/// Add a rounded rectangle to `region`, one row at a time along the corners.
fn add_rounded(region: &Region, x: i32, y: i32, w: i32, h: i32, radius: f32) {
    let r = (radius.round() as i32).clamp(0, w.min(h) / 2);
    for i in 0..r {
        let dy = (r - i) as f32 - 0.5;
        let dx = (r as f32 - (r as f32 * r as f32 - dy * dy).max(0.0).sqrt()).round() as i32;
        region.add(x + dx, y + i, w - 2 * dx, 1);
        region.add(x + dx, y + h - 1 - i, w - 2 * dx, 1);
    }
    region.add(x, y + r, w, h - 2 * r);
}

/// Connect to the compositor and set everything up, without showing anything yet.
pub fn connect(opts: Options) -> Result<(EventLoop<'static, State>, State), String> {
    let conn = Connection::connect_to_env()
        .map_err(|e| format!("Charpick needs a Wayland session such as KDE Plasma ({e})."))?;
    let (globals, mut queue) = registry_queue_init::<State>(&conn).map_err(|e| e.to_string())?;
    let qh = queue.handle();
    let event_loop = EventLoop::<State>::try_new().map_err(|e| e.to_string())?;

    let compositor = CompositorState::bind(&globals, &qh).map_err(|e| e.to_string())?;
    let shm = Shm::bind(&globals, &qh).map_err(|e| e.to_string())?;
    let layer_shell = LayerShell::bind(&globals, &qh)
        .map_err(|_| "Charpick needs a compositor with the layer-shell protocol, such as KDE Plasma.".to_string())?;
    // Version 6 types any character by its keysym.
    let fake_input: Option<OrgKdeKwinFakeInput> = globals.bind(&qh, 6..=6, Plain).ok();
    if let Some(f) = &fake_input {
        f.authenticate("Charpick".into(), "Type the characters picked in Charpick".into());
    }

    let mut settings = Settings::load();
    opts.apply(&mut settings);
    let fonts = paint::font_families();
    let look = super::kde::system_look(false);
    let theme = Theme::resolve(&settings, &look, &|f| fonts.contains(&f.to_lowercase()));
    let mut picker = Picker::new(Catalog::build(), &settings, theme);
    picker.own_frame = true;

    let mut state = State {
        conn: conn.clone(),
        qh: qh.clone(),
        handle: event_loop.handle(),
        registry: RegistryState::new(&globals),
        outputs: OutputState::new(&globals, &qh),
        seats: SeatState::new(&globals, &qh),
        effects: BackgroundEffectState::new(&globals, &qh),
        fractional: globals.bind(&qh, 1..=1, Plain).ok(),
        viewporter: globals.bind(&qh, 1..=1, Plain).ok(),
        cursor_shapes: CursorShapeManager::bind(&globals, &qh).ok(),
        data_control: globals.bind(&qh, 1..=1, Plain).ok(),
        compositor,
        shm,
        layer_shell,
        fake_input,
        seat: None,
        keyboard: None,
        pointer: None,
        cursor: None,
        clip_device: None,
        keymap: None,
        layout: 0,
        mods: Modifiers::default(),
        held: HashSet::new(),
        pool: None,
        win: None,
        last_scale120: 120,
        fonts,
        timers: Timers::default(),
        opts,
        picker,
        settings,
        settings_time: Settings::modified(),
        look_time: super::kde::modified(),
        recent: Recent::load(),
        bus: None,
        tray: None,
        recording: false,
        kde_label: None,
        synced: None,
        exit: false,
    };
    // Seats, outputs and the compositor's blur support.
    queue.roundtrip(&mut state).map_err(|e| e.to_string())?;
    state.last_scale120 = state.outputs.outputs().filter_map(|o| state.outputs.info(&o)).map(|i| i.scale_factor.max(1) as u32 * 120).max().unwrap_or(120);
    state.apply_settings();
    log!(
        "wl    fake input: {}, blur: {}, fractional scale: {}, clipboard: {}",
        state.fake_input.is_some(),
        state.blur_supported(),
        state.fractional.is_some(),
        state.data_control.is_some()
    );
    WaylandSource::new(conn, queue).insert(event_loop.handle()).map_err(|e| e.to_string())?;
    Ok((event_loop, state))
}

impl State {
    fn blur_supported(&self) -> bool {
        self.effects.capabilities().is_some_and(|c| c.contains(Effects::Blur))
    }

    pub(super) fn resolve_theme(&self) -> Theme {
        let look = super::kde::system_look(self.blur_supported());
        Theme::resolve(&self.settings, &look, &|f| self.fonts.contains(&f.to_lowercase()))
    }

    /// Re-read everything that depends on the settings or the desktop's look.
    pub(super) fn apply_settings(&mut self) {
        let theme = self.resolve_theme();
        self.picker.apply_settings(&self.settings, theme);
        if let Some(w) = &mut self.win {
            w.shadow = None;
        }
        self.redraw();
    }

    pub(super) fn load_settings(&mut self) {
        self.settings = Settings::load();
        self.opts.apply(&mut self.settings);
        self.settings_time = Settings::modified();
    }

    /// Pick up edits to settings.ini and a new Plasma color scheme. Returns true
    /// when settings.ini changed.
    pub(super) fn refresh(&mut self) -> bool {
        let edited = Settings::modified() != self.settings_time;
        if edited {
            self.load_settings();
        }
        let look = super::kde::modified();
        if edited || look != self.look_time {
            self.look_time = look;
            self.apply_settings();
        }
        edited
    }

    pub(super) fn visible(&self) -> bool {
        self.win.is_some()
    }

    fn upper(&self) -> bool {
        self.mods.shift ^ (self.mods.caps_lock && self.settings.caps_lock_uppercase)
    }

    fn held_mods(&self) -> u8 {
        let m = self.mods;
        (m.ctrl as u8 * MOD_CTRL) | (m.alt as u8 * MOD_ALT) | (m.shift as u8 * MOD_SHIFT) | (m.logo as u8 * MOD_WIN)
    }

    // --- Showing and hiding ----------------------------------------------------

    /// Open the picker, or the shortcut recorder.
    pub(super) fn show(&mut self, recorder: bool) {
        if self.win.is_some() {
            return;
        }
        self.refresh();
        let n = self.settings.recent_count;
        self.picker.recent = self.recent.items.iter().filter_map(|k| self.picker.catalog.entry_by_text(k)).take(n).collect();
        self.picker.reset();
        self.picker.upper = self.upper();
        self.picker.caret_on = true;
        self.picker.prompt = None;
        if recorder {
            self.picker.record_start(self.settings.hotkey);
            if let Some(label) = &self.kde_label {
                if let Some(p) = &mut self.picker.prompt {
                    p.hint = format!("Current shortcut: {label}");
                }
            }
        }

        let surface = self.compositor.create_surface(&self.qh);
        let (viewport, fractional) = match (&self.viewporter, &self.fractional) {
            (Some(vp), Some(fs)) => {
                (Some(vp.get_viewport(&surface, &self.qh, Plain)), Some(fs.get_fractional_scale(&surface, &self.qh, Plain)))
            }
            _ => (None, None),
        };
        let effect = if self.blur_supported() { self.effects.get_background_effect(&surface, &self.qh).ok() } else { None };
        let layer = self.layer_shell.create_layer_surface(&self.qh, surface, Layer::Overlay, Some("charpick"), None);
        let size = paint::logical_size(&self.picker);
        layer.set_size(size.0 as u32, size.1 as u32);
        let exclusive = recorder;
        layer.set_keyboard_interactivity(if exclusive { KeyboardInteractivity::Exclusive } else { KeyboardInteractivity::OnDemand });
        layer.commit();
        self.win = Some(Window {
            layer,
            viewport,
            fractional,
            effect,
            size,
            scale120: self.last_scale120,
            configured: false,
            focused: false,
            exclusive,
            frame_pending: false,
            dirty: false,
            shadow: None,
            leaving: None,
        });
        self.held.clear();

        if !recorder {
            self.timers.caret = self
                .handle
                .insert_source(Timer::from_duration(CARET_BLINK), |_, _, st| {
                    st.picker.caret_on = !st.picker.caret_on;
                    st.redraw();
                    TimeoutAction::ToDuration(CARET_BLINK)
                })
                .ok();
        }
        self.timers.focus = self
            .handle
            .insert_source(Timer::from_duration(FOCUS_WAIT), |_, _, st| {
                st.timers.focus = None;
                if let Some(w) = &mut st.win {
                    if !w.focused && !w.exclusive {
                        log!("wl    no keyboard focus yet, asking for it");
                        w.exclusive = true;
                        w.layer.set_keyboard_interactivity(KeyboardInteractivity::Exclusive);
                        w.layer.commit();
                    }
                }
                TimeoutAction::Drop
            })
            .ok();
        log!("wl    shown{}", if recorder { " (shortcut recorder)" } else { "" });
    }

    pub(super) fn hide(&mut self) {
        if self.recording {
            self.recording = false;
            if let Some(b) = &self.bus {
                b.block_shortcuts(false);
            }
        }
        for t in [&mut self.timers.caret, &mut self.timers.focus, &mut self.timers.release, &mut self.timers.close] {
            stop(&self.handle, t);
        }
        if self.win.take().is_some() {
            log!("wl    hidden");
        }
        self.picker.prompt = None;
        self.held.clear();
        self.recent.flush();
        let _ = self.conn.flush();
    }

    pub(super) fn toggle(&mut self) {
        if self.win.is_some() {
            if !self.recording {
                self.hide();
            }
        } else {
            self.show(false);
        }
    }

    /// Show the shortcut recorder, closing the picker first.
    pub(super) fn record_shortcut(&mut self) {
        if self.recording {
            return;
        }
        self.hide();
        self.show(true);
        self.recording = true;
        // Let the recorder see combinations that are global shortcuts already.
        if let Some(b) = &self.bus {
            b.block_shortcuts(true);
        }
    }

    fn set_scale(&mut self, scale120: u32) {
        self.last_scale120 = scale120;
        self.picker.set_dpi(scale120 * 96 / 120);
        if let Some(w) = &mut self.win {
            if w.scale120 != scale120 {
                w.scale120 = scale120;
                w.shadow = None;
                self.redraw();
            }
        }
    }

    // --- Drawing ---------------------------------------------------------------

    pub(super) fn redraw(&mut self) {
        match &mut self.win {
            Some(w) if w.frame_pending => w.dirty = true,
            Some(_) => self.draw(),
            None => {}
        }
    }

    fn draw(&mut self) {
        let Some(w) = &mut self.win else { return };
        if !w.configured {
            return;
        }
        w.dirty = false;
        let size = paint::logical_size(&self.picker);
        if size != w.size {
            w.size = size;
            w.layer.set_size(size.0 as u32, size.1 as u32);
        }
        let (scale, bw, bh) = if w.viewport.is_some() {
            let s = w.scale120 as f64 / 120.0;
            (s, (size.0 as f64 * s).round() as i32, (size.1 as f64 * s).round() as i32)
        } else {
            let s = w.scale120.div_ceil(120).max(1) as i32;
            (s as f64, size.0 * s, size.1 * s)
        };
        // While leaving the panel stays mapped (to keep the keyboard) but shows nothing.
        let blank = w.leaving.is_some();
        let image = if blank { None } else { paint::render(&self.picker, bw, bh, scale, &mut w.shadow) };
        if !blank && image.is_none() {
            return;
        }
        let pool = match &mut self.pool {
            Some(p) => p,
            None => match SlotPool::new((bw * bh * 4) as usize, &self.shm) {
                Ok(p) => self.pool.insert(p),
                Err(e) => {
                    log!("wl    no shared memory: {e}");
                    return;
                }
            },
        };
        let stride = bw * 4;
        let Ok((buffer, canvas)) = pool.create_buffer(bw, bh, stride, wl_shm::Format::Argb8888) else { return };
        match image {
            Some(mut img) => paint::copy_to(&mut img, canvas, stride as usize),
            None => canvas.fill(0),
        }

        let surface = w.layer.wl_surface();
        let m = &self.picker.theme.m;
        let (px, py, pw, ph) = (MARGIN as i32, MARGIN as i32, m.width.round() as i32, m.height.round() as i32);
        // Clicks on the shadow go to the window underneath (and close the picker).
        if let Ok(r) = Region::new(&self.compositor) {
            if !blank {
                r.add(px, py, pw, ph);
            }
            surface.set_input_region(Some(r.wl_region()));
        }
        if let Some(e) = &w.effect {
            if blank || self.picker.theme.backdrop != Backdrop::Blur {
                e.set_blur_region(None);
            } else if let Ok(r) = Region::new(&self.compositor) {
                add_rounded(&r, px, py, pw, ph, m.radius);
                e.set_blur_region(Some(r.wl_region()));
            }
        }
        match &w.viewport {
            Some(v) => v.set_destination(size.0, size.1),
            None => surface.set_buffer_scale(scale as i32),
        }
        surface.damage_buffer(0, 0, bw, bh);
        surface.frame(&self.qh, FrameCallbackData(surface.clone()));
        w.frame_pending = true;
        if buffer.attach_to(surface).is_err() {
            return;
        }
        w.layer.commit();
    }

    // --- Actions ---------------------------------------------------------------

    fn act(&mut self, action: Action) {
        match action {
            Action::None => {}
            Action::Redraw => self.redraw(),
            Action::Close => self.hide(),
            // Typing needs the keyboard back in the other window, so the picker
            // always closes.
            Action::Insert { text, key, .. } => {
                if self.settings.recent_count > 0 {
                    self.recent.push(&key, self.settings.recent_count);
                }
                self.begin_insert(text);
            }
        }
    }

    fn begin_insert(&mut self, text: String) {
        let Some(w) = &mut self.win else { return };
        w.leaving = Some(text);
        let wait = w.focused && !self.held.is_empty();
        stop(&self.handle, &mut self.timers.caret);
        if !wait {
            self.finish_insert();
            return;
        }
        // KWin types with the modifiers that are held down, so keep the keyboard
        // (with nothing on screen) until Enter, Shift and the rest are released.
        if let Some(w) = &mut self.win {
            w.frame_pending = false;
        }
        self.draw();
        self.timers.release = self
            .handle
            .insert_source(Timer::from_duration(RELEASE_WAIT), |_, _, st| {
                st.timers.release = None;
                st.finish_insert();
                TimeoutAction::Drop
            })
            .ok();
    }

    fn finish_insert(&mut self) {
        let Some(text) = self.win.as_mut().and_then(|w| w.leaving.take()) else { return };
        self.hide();
        self.update_tray();
        let _ = self.handle.insert_source(Timer::from_duration(REFOCUS_WAIT), move |_, _, st| {
            st.type_text(&text);
            TimeoutAction::Drop
        });
    }

    fn type_text(&mut self, text: &str) {
        let Some(f) = &self.fake_input else {
            self.copy_to_clipboard(text);
            return;
        };
        log!("wl    typing {text:?}");
        for c in text.chars() {
            let ks = xkb::utf32_to_keysym(c as u32).raw();
            if ks != 0 {
                f.keyboard_keysym(ks, 1);
                f.keyboard_keysym(ks, 0);
            }
        }
        let _ = self.conn.flush();
    }

    fn copy_to_clipboard(&mut self, text: &str) {
        let (Some(mgr), Some(seat)) = (&self.data_control, &self.seat) else {
            self.notify("Charpick cannot type here", "The compositor offers no way to type or copy characters.");
            return;
        };
        let device = self.clip_device.get_or_insert_with(|| mgr.get_data_device(seat, &self.qh, ClipDevice));
        let source = mgr.create_data_source(&self.qh, ClipSource(text.to_string()));
        for mime in ["text/plain;charset=utf-8", "UTF8_STRING", "text/plain", "TEXT", "STRING"] {
            source.offer(mime.into());
        }
        device.set_selection(Some(&source));
        let _ = self.conn.flush();
        log!("wl    copied {text:?} to the clipboard");
        self.notify(&format!("Copied {text}"), "Press Ctrl+V to paste it.");
    }

    pub(super) fn notify(&self, summary: &str, body: &str) {
        match &self.bus {
            Some(b) => b.notify(summary, body),
            None => eprintln!("{summary}: {body}"),
        }
    }

    // --- Keyboard --------------------------------------------------------------

    fn on_key(&mut self, ev: KeyEvent, repeat: bool) {
        let Some(w) = &self.win else { return };
        if w.leaving.is_some() {
            return;
        }
        if self.recording {
            if !repeat {
                self.record_key(&ev);
            }
            return;
        }
        let ctrl = self.mods.ctrl;
        let p = &mut self.picker;
        if p.prompt.is_some() {
            // "Shortcut saved": any key closes it.
            if !repeat && !ev.keysym.is_modifier_key() {
                self.hide();
            }
            return;
        }
        let action = match ev.keysym {
            Keysym::Escape => p.escape(),
            Keysym::Return | Keysym::KP_Enter => p.enter(),
            Keysym::Left | Keysym::KP_Left => p.navigate(Nav::Left),
            Keysym::Right | Keysym::KP_Right => p.navigate(Nav::Right),
            Keysym::Up | Keysym::KP_Up => p.navigate(Nav::Up),
            Keysym::Down | Keysym::KP_Down => p.navigate(Nav::Down),
            Keysym::Home | Keysym::KP_Home => p.navigate(Nav::Home),
            Keysym::End | Keysym::KP_End => p.navigate(Nav::End),
            Keysym::Page_Up | Keysym::KP_Page_Up => p.navigate(Nav::PageUp),
            Keysym::Page_Down | Keysym::KP_Page_Down => p.navigate(Nav::PageDown),
            Keysym::Tab if self.mods.shift => p.navigate(Nav::PrevSection),
            Keysym::Tab => p.navigate(Nav::NextSection),
            Keysym::ISO_Left_Tab => p.navigate(Nav::PrevSection),
            Keysym::BackSpace => p.backspace(ctrl),
            Keysym::Delete | Keysym::KP_Delete => p.clear_query(),
            _ if ctrl || self.mods.alt || self.mods.logo => Action::None,
            _ => {
                let mut action = Action::None;
                for c in ev.utf8.iter().flat_map(|s| s.chars()).filter(|c| !c.is_control()) {
                    if let Action::Redraw = p.type_char(c) {
                        action = Action::Redraw;
                    }
                }
                action
            }
        };
        self.act(action);
    }

    /// The key as Charpick names it: the unshifted symbol on the current layout.
    fn base_keysym(&self, raw: u32) -> Option<Keysym> {
        let km = self.keymap.as_ref()?;
        km.key_get_syms_by_level(xkb::Keycode::new(raw + 8), self.layout, 0).first().copied()
    }

    fn record_key(&mut self, ev: &KeyEvent) {
        if ev.keysym.is_modifier_key() {
            return;
        }
        let mods = self.held_mods();
        let base = self.base_keysym(ev.raw_code).unwrap_or(ev.keysym);
        if base == Keysym::Escape {
            if mods == 0 {
                self.hide();
            }
            return;
        }
        let Some(vk) = vk_from_keysym(base) else {
            let mut keys = Hotkey { vk: 0, mods }.parts();
            let name = base.key_char().filter(|c| !c.is_control()).map_or_else(
                || xkb::keysym_get_name(base),
                |c| c.to_uppercase().collect(),
            );
            keys.push(name);
            self.picker.record_warn(keys, "Use a letter, a digit, an F key or one of , . ; ' [ ] / \\ - = `");
            self.redraw();
            return;
        };
        match self.picker.record(Hotkey { vk, mods }) {
            Recorded::Cancel => self.hide(),
            Recorded::Pending => self.redraw(),
            Recorded::Done(hk) if shift_changes_key(hk) => {
                self.picker.record_warn(hk.parts(), "Shift changes this key; use it without Shift");
                self.redraw();
            }
            Recorded::Done(hk) => {
                self.recorded(hk);
                self.redraw();
            }
        }
    }
}

// --- Wayland handlers ----------------------------------------------------------

impl CompositorHandler for State {
    fn scale_factor_changed(&mut self, _: &Connection, _: &QueueHandle<Self>, surface: &WlSurface, factor: i32) {
        // Only used without fractional scaling.
        if let Some(w) = &self.win {
            if w.viewport.is_none() && w.layer.wl_surface() == surface {
                self.set_scale(factor.max(1) as u32 * 120);
            }
        }
    }

    fn transform_changed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlSurface, _: wl_output::Transform) {}

    fn frame(&mut self, _: &Connection, _: &QueueHandle<Self>, surface: &WlSurface, _: u32) {
        let Some(w) = &mut self.win else { return };
        if w.layer.wl_surface() != surface {
            return;
        }
        w.frame_pending = false;
        if self.picker.animating() {
            self.picker.tick();
            w.dirty = true;
        }
        if w.dirty {
            self.draw();
        }
    }

    fn surface_enter(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlSurface, _: &WlOutput) {}

    fn surface_leave(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlSurface, _: &WlOutput) {}
}

impl OutputHandler for State {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.outputs
    }

    fn new_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: WlOutput) {}

    fn update_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: WlOutput) {}

    fn output_destroyed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: WlOutput) {}
}

impl LayerShellHandler for State {
    fn closed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &LayerSurface) {
        self.hide();
    }

    fn configure(&mut self, _: &Connection, _: &QueueHandle<Self>, layer: &LayerSurface, _: LayerSurfaceConfigure, _: u32) {
        let Some(w) = &mut self.win else { return };
        if &w.layer != layer {
            return;
        }
        let first = !w.configured;
        w.configured = true;
        if first || !w.frame_pending {
            w.frame_pending = false;
            self.draw();
            if let Some(w) = self.win.as_ref().filter(|_| first) {
                log!("wl    drawn at {}×{}, scale {:.2}", w.size.0, w.size.1, w.scale120 as f64 / 120.0);
            }
        } else {
            w.dirty = true;
        }
    }
}

impl SeatHandler for State {
    fn seat_state(&mut self) -> &mut SeatState {
        &mut self.seats
    }

    fn new_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: WlSeat) {}

    fn new_capability(&mut self, _: &Connection, qh: &QueueHandle<Self>, seat: WlSeat, cap: Capability) {
        self.seat.get_or_insert_with(|| seat.clone());
        if cap == Capability::Keyboard && self.keyboard.is_none() {
            let repeat = Box::new(|st: &mut State, _: &WlKeyboard, ev: KeyEvent| st.on_key(ev, true));
            self.keyboard = self.seats.get_keyboard_with_repeat(qh, &seat, None, self.handle.clone(), repeat).ok();
        }
        if cap == Capability::Pointer && self.pointer.is_none() {
            if let Ok(p) = self.seats.get_pointer(qh, &seat) {
                self.cursor = self.cursor_shapes.as_ref().map(|c| c.get_shape_device(&p, qh));
                self.pointer = Some(p);
            }
        }
    }

    fn remove_capability(&mut self, _: &Connection, _: &QueueHandle<Self>, _: WlSeat, cap: Capability) {
        if cap == Capability::Keyboard {
            if let Some(k) = self.keyboard.take() {
                k.release();
            }
        }
        if cap == Capability::Pointer {
            if let Some(c) = self.cursor.take() {
                c.destroy();
            }
            if let Some(p) = self.pointer.take() {
                p.release();
            }
        }
    }

    fn remove_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: WlSeat) {}
}

impl KeyboardHandler for State {
    fn enter(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlKeyboard, surface: &WlSurface, _: u32, raw: &[u32], _: &[Keysym]) {
        let Some(w) = &mut self.win else { return };
        if w.layer.wl_surface() == surface {
            w.focused = true;
            log!("wl    keyboard focus");
            self.held = raw.iter().copied().collect();
            stop(&self.handle, &mut self.timers.focus);
        }
    }

    fn leave(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlKeyboard, surface: &WlSurface, _: u32) {
        let Some(w) = &mut self.win else { return };
        if w.layer.wl_surface() != surface {
            return;
        }
        w.focused = false;
        self.held.clear();
        if w.leaving.is_some() {
            self.finish_insert();
        } else {
            // Clicked somewhere else.
            self.hide();
        }
    }

    fn press_key(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlKeyboard, _: u32, ev: KeyEvent) {
        self.held.insert(ev.raw_code);
        self.on_key(ev, false);
    }

    fn repeat_key(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlKeyboard, _: u32, ev: KeyEvent) {
        self.on_key(ev, true);
    }

    fn release_key(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlKeyboard, _: u32, ev: KeyEvent) {
        self.held.remove(&ev.raw_code);
        if self.held.is_empty() && self.win.as_ref().is_some_and(|w| w.leaving.is_some()) {
            self.finish_insert();
        }
    }

    fn update_modifiers(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlKeyboard, _: u32, mods: Modifiers, _: RawModifiers, layout: u32) {
        self.mods = mods;
        self.layout = layout;
        if self.win.as_ref().is_none_or(|w| w.leaving.is_some()) {
            return;
        }
        if self.recording {
            let held = Hotkey { vk: 0, mods: self.held_mods() };
            self.picker.record(held);
            self.redraw();
        } else if self.picker.upper != self.upper() {
            self.picker.upper = self.upper();
            self.redraw();
        }
    }

    fn update_keymap(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlKeyboard, keymap: Keymap<'_>) {
        let ctx = xkb::Context::new(xkb::CONTEXT_NO_FLAGS);
        self.keymap =
            xkb::Keymap::new_from_string(&ctx, keymap.as_string(), xkb::KEYMAP_FORMAT_TEXT_V1, xkb::KEYMAP_COMPILE_NO_FLAGS);
    }
}

/// Pointer scrolling in the picker's units: 120 per wheel step, positive = up or right.
fn wheel_delta(a: &AxisScroll) -> f64 {
    if a.value120 != 0 {
        a.value120 as f64
    } else if a.discrete != 0 {
        a.discrete as f64 * 120.0
    } else {
        // Touchpads: move about as far as the fingers did.
        a.absolute * 1.2
    }
}

impl PointerHandler for State {
    fn pointer_frame(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlPointer, events: &[PointerEvent]) {
        for ev in events {
            let Some(w) = &self.win else { return };
            if &ev.surface != w.layer.wl_surface() || w.leaving.is_some() {
                continue;
            }
            let (x, y) = (ev.position.0 as f32 - MARGIN, ev.position.1 as f32 - MARGIN);
            let p = &mut self.picker;
            let action = match ev.kind {
                PointerEventKind::Enter { serial } => {
                    if let Some(c) = &self.cursor {
                        c.set_shape(serial, Shape::Default);
                    }
                    p.mouse_move(x, y)
                }
                PointerEventKind::Leave { .. } => p.mouse_leave(),
                PointerEventKind::Motion { .. } => p.mouse_move(x, y),
                PointerEventKind::Press { button: BTN_LEFT, .. } => p.mouse_down(x, y),
                PointerEventKind::Release { button: BTN_LEFT, .. } => p.mouse_up(x, y),
                PointerEventKind::Axis { horizontal, vertical, .. } => {
                    let clamp = |d: f64| d.round().clamp(i16::MIN as f64, i16::MAX as f64) as i16;
                    let (v, h) = (wheel_delta(&vertical), wheel_delta(&horizontal));
                    if v != 0.0 {
                        p.wheel(clamp(-v), false, y)
                    } else if h != 0.0 {
                        p.wheel(clamp(h), true, y)
                    } else {
                        Action::None
                    }
                }
                _ => Action::None,
            };
            self.act(action);
        }
    }
}

impl ShmHandler for State {
    fn shm_state(&mut self) -> &mut Shm {
        &mut self.shm
    }
}

impl BackgroundEffectHandler for State {
    fn background_effect_state(&mut self) -> &mut BackgroundEffectState {
        &mut self.effects
    }

    fn update_capabilities(&mut self) {}
}

impl ProvidesRegistryState for State {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry
    }
    registry_handlers![OutputState, SeatState];
}

delegate_registry!(State);
delegate_dispatch2!(State);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keysyms_to_keys() {
        assert_eq!(vk_from_keysym(Keysym::comma), Some(0xBC));
        assert_eq!(vk_from_keysym(Keysym::k), Some(b'K'));
        assert_eq!(vk_from_keysym(Keysym::_7), Some(b'7'));
        assert_eq!(vk_from_keysym(Keysym::F12), Some(0x7B));
        assert_eq!(vk_from_keysym(Keysym::space), Some(0x20));
        assert_eq!(vk_from_keysym(Keysym::ograve), None);
        assert!(shift_changes_key(Hotkey { vk: 0xBC, mods: MOD_WIN | MOD_SHIFT }));
        assert!(!shift_changes_key(Hotkey { vk: b'K', mods: MOD_WIN | MOD_SHIFT }));
    }
}
