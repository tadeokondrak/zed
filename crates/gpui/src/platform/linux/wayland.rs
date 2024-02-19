#![allow(unused_variables)]

use crate::{
    platform::blade::BladeRenderer, Bounds, LinuxPlatformInner, Modifiers, MouseButton, Pixels,
    PlatformInputHandler, Point, Size,
};
use parking_lot::Mutex;
use slotmap::SlotMap;
use std::{rc::Rc, sync::Arc};
use wayland_client::{
    protocol::{wl_compositor, wl_surface},
    QueueHandle,
};
use wayland_protocols::xdg::shell::client::{xdg_surface, xdg_toplevel, xdg_wm_base};
use xkbcommon::xkb;

mod client;
mod display;
mod input;
mod window;

pub(super) struct WaylandClient {
    platform_inner: Rc<LinuxPlatformInner>,
    state: Arc<Mutex<WaylandClientState>>,
    qh: QueueHandle<WaylandClientState>,
}

struct WaylandClientState {
    platform_inner: Rc<LinuxPlatformInner>,
    compositor: wl_compositor::WlCompositor,
    wm_base: xdg_wm_base::XdgWmBase,
    windows: Vec<Rc<WaylandWindowInner>>,
    seats: SlotMap<WaylandSeatId, WaylandSeatState>,
    mouse_location: Option<Point<Pixels>>,
    button_pressed: Option<MouseButton>,
    mouse_focused_window: Option<Rc<WaylandWindowInner>>,
    keyboard_focused_window: Option<Rc<WaylandWindowInner>>,
}

#[derive(Clone)]
struct WaylandWindow {
    inner: Rc<WaylandWindowInner>,
}

struct WaylandWindowInner {
    state: Mutex<WaylandWindowState>,
    callbacks: Mutex<Callbacks>,
    surface: wl_surface::WlSurface,
    xdg_surface: xdg_surface::XdgSurface,
    toplevel: xdg_toplevel::XdgToplevel,
}

struct WaylandWindowState {
    renderer: BladeRenderer,
    bounds: Bounds<i32>,
    input_handler: Option<PlatformInputHandler>,
}

slotmap::new_key_type! {
    struct WaylandSeatId;
}

#[derive(Default)]
struct WaylandSeatState {
    xkb_state: Option<xkb::State>,
    modifiers: Modifiers,
    scroll_direction: f64,
}

#[derive(Default)]
struct Callbacks {
    request_frame: Option<Box<dyn FnMut()>>,
    input: Option<Box<dyn FnMut(crate::PlatformInput) -> bool>>,
    active_status_change: Option<Box<dyn FnMut(bool)>>,
    resize: Option<Box<dyn FnMut(Size<Pixels>, f32)>>,
    fullscreen: Option<Box<dyn FnMut(bool)>>,
    moved: Option<Box<dyn FnMut()>>,
    should_close: Option<Box<dyn FnMut() -> bool>>,
    close: Option<Box<dyn FnOnce()>>,
    appearance_changed: Option<Box<dyn FnMut()>>,
}

#[derive(Debug)]
struct WaylandDisplay {}
