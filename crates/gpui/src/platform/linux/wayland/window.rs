use std::any::Any;
use std::ffi::c_void;
use std::rc::Rc;
use std::sync::Arc;

use blade_graphics as gpu;
use blade_rwh::{HasRawDisplayHandle, HasRawWindowHandle, RawDisplayHandle, RawWindowHandle};
use futures::channel::oneshot::Receiver;
use parking_lot::Mutex;
use raw_window_handle::{
    DisplayHandle, HandleError, HasDisplayHandle, HasWindowHandle, WindowHandle,
};
use wayland_client::{protocol::wl_surface, Proxy};
use wayland_protocols::xdg::shell::client::xdg_toplevel;

use super::Callbacks;
use super::WaylandWindow;
use super::WaylandWindowInner;
use super::WaylandWindowState;
use crate::platform::blade::BladeRenderer;
use crate::platform::linux::wayland::WaylandDisplay;
use crate::platform::{PlatformAtlas, PlatformInputHandler, PlatformWindow};
use crate::scene::Scene;
use crate::{
    px, size, Bounds, Modifiers, Pixels, PlatformDisplay, PlatformInput, Point, PromptLevel, Size,
    WindowAppearance, WindowBounds, WindowOptions,
};
use wayland_protocols::xdg::shell::client::xdg_surface;

struct RawWindow {
    window: *mut c_void,
    display: *mut c_void,
}

unsafe impl HasRawWindowHandle for RawWindow {
    fn raw_window_handle(&self) -> RawWindowHandle {
        let mut wh = blade_rwh::WaylandWindowHandle::empty();
        wh.surface = self.window;
        wh.into()
    }
}

unsafe impl HasRawDisplayHandle for RawWindow {
    fn raw_display_handle(&self) -> RawDisplayHandle {
        let mut dh = blade_rwh::WaylandDisplayHandle::empty();
        dh.display = self.display;
        dh.into()
    }
}

impl WaylandWindowState {
    fn new(wl_surf: wl_surface::WlSurface, bounds: Bounds<i32>) -> Self {
        let raw = RawWindow {
            window: wl_surf.id().as_ptr().cast::<c_void>(),
            display: wl_surf
                .backend()
                .upgrade()
                .unwrap()
                .display_ptr()
                .cast::<c_void>(),
        };
        let gpu = Arc::new(
            unsafe {
                gpu::Context::init_windowed(
                    &raw,
                    gpu::ContextDesc {
                        validation: false,
                        capture: false,
                    },
                )
            }
            .unwrap(),
        );
        let extent = gpu::Extent {
            width: bounds.size.width as u32,
            height: bounds.size.height as u32,
            depth: 1,
        };
        Self {
            renderer: BladeRenderer::new(gpu, extent),
            bounds,
            input_handler: None,
        }
    }
}

impl WaylandWindowInner {
    pub fn new(
        wl_surf: wl_surface::WlSurface,
        xdg_surface: xdg_surface::XdgSurface,
        toplevel: xdg_toplevel::XdgToplevel,
        options: WindowOptions,
    ) -> Self {
        if options.bounds == WindowBounds::Maximized {
            toplevel.set_maximized();
        } else if options.bounds == WindowBounds::Fullscreen {
            toplevel.set_fullscreen(None);
        }

        let bounds: Bounds<i32> = match options.bounds {
            WindowBounds::Fullscreen | WindowBounds::Maximized => Bounds {
                origin: Point::default(),
                size: Size {
                    width: 500,
                    height: 500,
                }, //todo!(implement)
            },
            WindowBounds::Fixed(bounds) => bounds.map(|p| p.0 as i32),
        };

        Self {
            surface: wl_surf.clone(),
            xdg_surface,
            state: Mutex::new(WaylandWindowState::new(wl_surf, bounds)),
            callbacks: Mutex::new(Callbacks::default()),
            toplevel,
        }
    }

    pub fn update(&self) {
        let mut cb = self.callbacks.lock();
        if let Some(mut fun) = cb.request_frame.take() {
            drop(cb);
            fun();
            self.callbacks.lock().request_frame = Some(fun);
        }
    }

    pub fn resize(&self, width: i32, height: i32) {
        {
            let mut inner = self.state.lock();
            inner.bounds.size.width = width;
            inner.bounds.size.height = height;
            inner
                .renderer
                .update_drawable_size(size(width as f64, height as f64));
        }
        let mut callbacks = self.callbacks.lock();
        if let Some(ref mut fun) = callbacks.resize {
            fun(
                Size {
                    width: px(width as f32),
                    height: px(height as f32),
                },
                1.0,
            );
        }
        if let Some(ref mut fun) = callbacks.moved {
            fun()
        }
    }

    pub fn close(&self) {
        let mut callbacks = self.callbacks.lock();
        if let Some(fun) = callbacks.close.take() {
            fun()
        }
        self.toplevel.destroy();
    }

    pub fn handle_input(&self, input: PlatformInput) {
        if let Some(ref mut fun) = self.callbacks.lock().input {
            if fun(input.clone()) {
                return;
            }
        }
        if let PlatformInput::KeyDown(event) = input {
            let mut inner = self.state.lock();
            if let Some(ref mut input_handler) = inner.input_handler {
                if let Some(ime_key) = &event.keystroke.ime_key {
                    input_handler.replace_text_in_range(None, ime_key);
                }
            }
        }
    }
}

impl HasWindowHandle for WaylandWindow {
    fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
        unimplemented!()
    }
}

impl HasDisplayHandle for WaylandWindow {
    fn display_handle(&self) -> Result<DisplayHandle<'_>, HandleError> {
        unimplemented!()
    }
}

impl PlatformWindow for WaylandWindow {
    //todo!(linux)
    fn bounds(&self) -> WindowBounds {
        WindowBounds::Maximized
    }

    // todo!(linux)
    fn content_size(&self) -> Size<Pixels> {
        let inner = self.inner.state.lock();
        Size {
            width: Pixels(inner.bounds.size.width as f32),
            height: Pixels(inner.bounds.size.height as f32),
        }
    }

    // todo!(linux)
    fn scale_factor(&self) -> f32 {
        1f32
    }

    //todo!(linux)
    fn titlebar_height(&self) -> Pixels {
        unimplemented!()
    }

    // todo!(linux)
    fn appearance(&self) -> WindowAppearance {
        WindowAppearance::Light
    }

    // todo!(linux)
    fn display(&self) -> Rc<dyn PlatformDisplay> {
        Rc::new(WaylandDisplay {})
    }

    // todo!(linux)
    fn mouse_position(&self) -> Point<Pixels> {
        Point::default()
    }

    //todo!(linux)
    fn modifiers(&self) -> Modifiers {
        crate::Modifiers::default()
    }

    //todo!(linux)
    fn as_any_mut(&mut self) -> &mut dyn Any {
        unimplemented!()
    }

    fn set_input_handler(&mut self, input_handler: PlatformInputHandler) {
        self.inner.state.lock().input_handler = Some(input_handler);
    }

    //todo!(linux)
    fn take_input_handler(&mut self) -> Option<PlatformInputHandler> {
        self.inner.state.lock().input_handler.take()
    }

    //todo!(linux)
    fn prompt(
        &self,
        level: PromptLevel,
        msg: &str,
        detail: Option<&str>,
        answers: &[&str],
    ) -> Receiver<usize> {
        unimplemented!()
    }

    fn activate(&self) {
        //todo!(linux)
    }

    fn set_title(&mut self, title: &str) {
        self.inner.toplevel.set_title(title.to_string());
    }

    fn set_edited(&mut self, edited: bool) {
        //todo!(linux)
    }

    fn show_character_palette(&self) {
        //todo!(linux)
    }

    fn minimize(&self) {
        //todo!(linux)
    }

    fn zoom(&self) {
        //todo!(linux)
    }

    fn toggle_full_screen(&self) {
        //todo!(linux)
    }

    fn on_request_frame(&self, callback: Box<dyn FnMut()>) {
        self.inner.callbacks.lock().request_frame = Some(callback);
    }

    fn on_input(&self, callback: Box<dyn FnMut(PlatformInput) -> bool>) {
        self.inner.callbacks.lock().input = Some(callback);
    }

    fn on_active_status_change(&self, callback: Box<dyn FnMut(bool)>) {
        //todo!(linux)
    }

    fn on_resize(&self, callback: Box<dyn FnMut(Size<Pixels>, f32)>) {
        self.inner.callbacks.lock().resize = Some(callback);
    }

    fn on_fullscreen(&self, callback: Box<dyn FnMut(bool)>) {
        //todo!(linux)
    }

    fn on_moved(&self, callback: Box<dyn FnMut()>) {
        self.inner.callbacks.lock().moved = Some(callback);
    }

    fn on_should_close(&self, callback: Box<dyn FnMut() -> bool>) {
        self.inner.callbacks.lock().should_close = Some(callback);
    }

    fn on_close(&self, callback: Box<dyn FnOnce()>) {
        self.inner.callbacks.lock().close = Some(callback);
    }

    fn on_appearance_changed(&self, callback: Box<dyn FnMut()>) {
        //todo!(linux)
    }

    // todo!(linux)
    fn is_topmost_for_position(&self, position: Point<Pixels>) -> bool {
        false
    }

    fn draw(&self, scene: &Scene) {
        let mut inner = self.inner.state.lock();
        inner.renderer.draw(scene);
    }

    fn sprite_atlas(&self) -> Arc<dyn PlatformAtlas> {
        let inner = self.inner.state.lock();
        inner.renderer.sprite_atlas().clone()
    }

    fn set_graphics_profiler_enabled(&self, enabled: bool) {
        //todo!(linux)
    }
}
