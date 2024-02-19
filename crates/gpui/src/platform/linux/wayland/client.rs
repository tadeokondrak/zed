use crate::platform::linux::wayland::{
    WaylandClient, WaylandClientState, WaylandWindow, WaylandWindowInner,
};
use crate::platform::{LinuxPlatformInner, PlatformWindow};
use crate::{AnyWindowHandle, DisplayId, Modifiers, PlatformDisplay, WindowOptions};
use calloop_wayland_source::WaylandSource;
use parking_lot::Mutex;
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
        let state = Arc::new(Mutex::new(WaylandClientState {
            platform_inner: Rc::clone(&linux_platform_inner),
            compositor: globals.bind(&qh, 1..=1, ()).unwrap(),
            wm_base: globals.bind(&qh, 1..=1, ()).unwrap(),
            windows: Vec::new(),
            modifiers: Modifiers::default(),
            scroll_direction: -1.0,
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

        wl_surface.frame(&self.qh, wl_surface.clone());
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
        if let wl_registry::Event::Global {
            name, interface, ..
        } = event
        {
            match interface.as_str() {
                "wl_seat" => {
                    let seat = registry.bind::<wl_seat::WlSeat, _, _>(name, 1, qh, ());
                }
                _ => {}
            };
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
        if let wl_callback::Event::Done { .. } = event {
            for window in &state.windows {
                if window.surface.id() == surf.id() {
                    window.surface.frame(qh, surf.clone());
                    window.update();
                    window.surface.commit();
                }
            }
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
        _: &QueueHandle<Self>,
    ) {
        if let xdg_surface::Event::Configure { serial, .. } = event {
            xdg_surface.ack_configure(serial);
            for window in &state.windows {
                if &window.xdg_surface == xdg_surface {
                    window.update();
                    window.surface.commit();
                    return;
                }
            }
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
        if let xdg_toplevel::Event::Configure {
            width,
            height,
            states: _,
        } = event
        {
            if width == 0 || height == 0 {
                return;
            }
            for window in &state.windows {
                if window.toplevel.id() == xdg_toplevel.id() {
                    window.resize(width, height);
                    window.surface.commit();
                    return;
                }
            }
        } else if let xdg_toplevel::Event::Close = event {
            state.windows.retain(|window| {
                if window.toplevel.id() == xdg_toplevel.id() {
                    window.toplevel.destroy();
                    false
                } else {
                    true
                }
            });
            state.platform_inner.state.lock().quit_requested |= state.windows.is_empty();
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
