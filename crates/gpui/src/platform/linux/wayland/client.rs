use crate::platform::linux::wayland::{
    WaylandClient, WaylandClientState, WaylandWindow, WaylandWindowInner,
};
use crate::platform::{LinuxPlatformInner, PlatformWindow};
use crate::{AnyWindowHandle, DisplayId, PlatformDisplay, WindowOptions};
use calloop_wayland_source::WaylandSource;
use parking_lot::Mutex;
use slotmap::SlotMap;
use std::rc::Rc;
use std::sync::Arc;
use wayland_client::globals::{registry_queue_init, GlobalListContents};
use wayland_client::protocol::wl_callback::WlCallback;
use wayland_client::{
    delegate_noop,
    protocol::{
        wl_buffer, wl_callback, wl_compositor, wl_registry, wl_seat, wl_shm, wl_shm_pool,
        wl_surface::{self, WlSurface},
    },
    Connection, Dispatch, Proxy, QueueHandle,
};
use wayland_protocols::xdg::shell::client::{xdg_surface, xdg_toplevel, xdg_wm_base};

impl WaylandClient {
    pub fn new(linux_platform_inner: Rc<LinuxPlatformInner>) -> Self {
        let conn = Connection::connect_to_env().unwrap();
        let (globals, mut event_queue) = registry_queue_init::<WaylandClientState>(&conn).unwrap();
        let qh = event_queue.handle();
        let mut seats = SlotMap::with_key();
        globals.contents().with_list(|list| {
            for global in list {
                if global.interface == "wl_seat" {
                    let seat_id = seats.insert(super::WaylandSeatState::default());
                    globals
                        .registry()
                        .bind::<wl_seat::WlSeat, _, _>(global.name, 1, &qh, seat_id);
                }
            }
        });
        let state = Arc::new(Mutex::new(WaylandClientState {
            platform_inner: Rc::clone(&linux_platform_inner),
            compositor: globals.bind(&qh, 1..=1, ()).unwrap(),
            wm_base: globals.bind(&qh, 1..=1, ()).unwrap(),
            windows: Vec::new(),
            seats,
            mouse_location: None,
            button_pressed: None,
            mouse_focused_window: None,
            keyboard_focused_window: None,
        }));
        let source = WaylandSource::new(conn, event_queue);
        {
            let state = Arc::clone(&state);
            linux_platform_inner
                .loop_handle
                .insert_source(source, move |_, queue, _| {
                    queue.dispatch_pending(&mut *state.lock())
                })
                .unwrap();
        }
        Self {
            platform_inner: Rc::clone(&linux_platform_inner),
            state,
            qh,
        }
    }
}

impl WaylandClient {
    pub fn displays(&self) -> Vec<Rc<dyn PlatformDisplay>> {
        Vec::new()
    }

    pub fn display(&self, id: DisplayId) -> Option<Rc<dyn PlatformDisplay>> {
        unimplemented!()
    }

    pub fn open_window(
        &self,
        handle: AnyWindowHandle,
        options: WindowOptions,
    ) -> Box<dyn PlatformWindow> {
        let mut state = self.state.lock();

        let wl_surface = state.compositor.create_surface(&self.qh, ());
        let xdg_surface = state.wm_base.get_xdg_surface(&wl_surface, &self.qh, ());
        let toplevel = xdg_surface.get_toplevel(&self.qh, ());

        wl_surface.commit();

        let window = Rc::new(WaylandWindowInner::new(
            wl_surface.clone(),
            xdg_surface,
            toplevel,
            options,
        ));

        state.windows.push(Rc::clone(&window));
        Box::new(WaylandWindow { inner: window })
    }
}

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for WaylandClientState {
    fn event(
        state: &mut Self,
        registry: &wl_registry::WlRegistry,
        event: wl_registry::Event,
        _: &GlobalListContents,
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        match event {
            wl_registry::Event::Global {
                name,
                interface,
                version: _,
            } => match interface.as_str() {
                "wl_seat" => {
                    let seat_id = state.seats.insert(super::WaylandSeatState::default());
                    registry.bind::<wl_seat::WlSeat, _, _>(name, 1, qh, seat_id);
                }
                _ => {}
            },
            wl_registry::Event::GlobalRemove { name: _ } => {}
            _ => {}
        }
    }
}

delegate_noop!(WaylandClientState: ignore wl_compositor::WlCompositor);
delegate_noop!(WaylandClientState: ignore wl_surface::WlSurface);
delegate_noop!(WaylandClientState: ignore wl_shm::WlShm);
delegate_noop!(WaylandClientState: ignore wl_shm_pool::WlShmPool);
delegate_noop!(WaylandClientState: ignore wl_buffer::WlBuffer);

impl Dispatch<WlCallback, WlSurface> for WaylandClientState {
    fn event(
        state: &mut Self,
        _: &WlCallback,
        event: wl_callback::Event,
        surf: &WlSurface,
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        match event {
            wl_callback::Event::Done { callback_data: _ } => {
                for window in &state.windows {
                    if window.surface.id() == surf.id() {
                        window.surface.frame(qh, window.surface.clone());
                        window.update();
                        window.surface.commit();
                    }
                }
            }
            _ => (),
        }
    }
}

impl Dispatch<xdg_surface::XdgSurface, ()> for WaylandClientState {
    fn event(
        state: &mut Self,
        xdg_surface: &xdg_surface::XdgSurface,
        event: xdg_surface::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        match event {
            xdg_surface::Event::Configure { serial } => {
                xdg_surface.ack_configure(serial);
                for window in &state.windows {
                    if &window.xdg_surface == xdg_surface {
                        let mut state = window.state.lock();
                        let frame_callback_already_requested = state.frame_callback_requested;
                        state.frame_callback_requested = true;
                        drop(state);

                        if !frame_callback_already_requested {
                            window.surface.frame(qh, window.surface.clone());
                        }
                        window.update();
                        window.surface.commit();
                        return;
                    }
                }
            }
            _ => todo!(),
        }
    }
}

impl Dispatch<xdg_toplevel::XdgToplevel, ()> for WaylandClientState {
    fn event(
        state: &mut Self,
        xdg_toplevel: &xdg_toplevel::XdgToplevel,
        event: <xdg_toplevel::XdgToplevel as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            xdg_toplevel::Event::Configure {
                mut width,
                mut height,
                states,
            } => {
                if width == 0 || height == 0 {
                    width = 1000;
                    height = 1000;
                }
                for window in &state.windows {
                    if window.toplevel.id() == xdg_toplevel.id() {
                        window.resize(width, height);
                        return;
                    }
                }
            }
            xdg_toplevel::Event::Close => {
                xdg_toplevel.destroy();
                let index = state
                    .windows
                    .iter()
                    .position(|window| window.toplevel.id() == xdg_toplevel.id())
                    .unwrap();
                state.windows.swap_remove(index);
                state.platform_inner.state.lock().quit_requested |= state.windows.is_empty();
            }
            xdg_toplevel::Event::ConfigureBounds {
                width: _,
                height: _,
            } => {}
            xdg_toplevel::Event::WmCapabilities { capabilities: _ } => {}
            _ => {}
        }
    }
}

impl Dispatch<xdg_wm_base::XdgWmBase, ()> for WaylandClientState {
    fn event(
        _: &mut Self,
        wm_base: &xdg_wm_base::XdgWmBase,
        event: <xdg_wm_base::XdgWmBase as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            xdg_wm_base::Event::Ping { serial } => wm_base.pong(serial),
            _ => {}
        }
    }
}
