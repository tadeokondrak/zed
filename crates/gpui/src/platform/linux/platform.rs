#![allow(unused)]

use std::env;
use std::{
    path::{Path, PathBuf},
    rc::Rc,
    sync::Arc,
    time::Duration,
};

use ashpd::desktop::file_chooser::{OpenFileRequest, SaveFileRequest};
use async_task::Runnable;
use calloop::{EventLoop, LoopHandle, LoopSignal};
use futures::channel::oneshot;
use parking_lot::Mutex;
use time::UtcOffset;
use wayland_client::Connection;

use crate::platform::linux::wayland::WaylandClient;
use crate::platform::{X11Client, XcbAtoms};
use crate::{
    Action, AnyWindowHandle, BackgroundExecutor, ClipboardItem, CursorStyle, DisplayId,
    ForegroundExecutor, Keymap, LinuxDispatcher, LinuxTextSystem, Menu, PathPromptOptions,
    Platform, PlatformDisplay, PlatformInput, PlatformTextSystem, PlatformWindow, Result,
    SemanticVersion, Task, WindowOptions,
};
use calloop::channel::{Channel, Sender};

#[derive(Default)]
pub(crate) struct Callbacks {
    open_urls: Option<Box<dyn FnMut(Vec<String>)>>,
    become_active: Option<Box<dyn FnMut()>>,
    resign_active: Option<Box<dyn FnMut()>>,
    pub(crate) quit: Option<Box<dyn FnMut()>>,
    reopen: Option<Box<dyn FnMut()>>,
    event: Option<Box<dyn FnMut(PlatformInput) -> bool>>,
    app_menu_action: Option<Box<dyn FnMut(&dyn Action)>>,
    will_open_app_menu: Option<Box<dyn FnMut()>>,
    validate_app_menu_command: Option<Box<dyn FnMut(&dyn Action) -> bool>>,
}

pub(crate) struct LinuxPlatformInner {
    pub(crate) event_loop: Mutex<EventLoop<'static, ()>>,
    pub(crate) loop_handle: LoopHandle<'static, ()>,
    pub(crate) loop_signal: LoopSignal,
    pub(crate) background_executor: BackgroundExecutor,
    pub(crate) foreground_executor: ForegroundExecutor,
    pub(crate) text_system: Arc<LinuxTextSystem>,
    pub(crate) callbacks: Mutex<Callbacks>,
    pub(crate) state: Mutex<LinuxPlatformState>,
}

enum LinuxClient {
    Wayland(WaylandClient),
    X11(Rc<X11Client>),
}

pub(crate) struct LinuxPlatform {
    client: LinuxClient,
    inner: Rc<LinuxPlatformInner>,
}

pub(crate) struct LinuxPlatformState {
    pub(crate) quit_requested: bool,
}

impl Default for LinuxPlatform {
    fn default() -> Self {
        Self::new()
    }
}

impl LinuxPlatform {
    pub(crate) fn new() -> Self {
        let wayland_display = env::var_os("WAYLAND_DISPLAY");
        let use_wayland = wayland_display.is_some() && !wayland_display.unwrap().is_empty();

        let (main_sender, main_receiver) = calloop::channel::channel::<Runnable>();
        let text_system = Arc::new(LinuxTextSystem::new());
        let callbacks = Mutex::new(Callbacks::default());
        let state = Mutex::new(LinuxPlatformState {
            quit_requested: false,
        });

        let event_loop = EventLoop::try_new().unwrap();
        event_loop
            .handle()
            .insert_source(main_receiver, |event, _, _| match event {
                calloop::channel::Event::Msg(runnable) => {
                    runnable.run();
                }
                calloop::channel::Event::Closed => {}
            });

        let dispatcher = Arc::new(LinuxDispatcher::new(main_sender));
        let inner = Rc::new(LinuxPlatformInner {
            loop_handle: event_loop.handle(),
            loop_signal: event_loop.get_signal(),
            event_loop: Mutex::new(event_loop),
            background_executor: BackgroundExecutor::new(dispatcher.clone()),
            foreground_executor: ForegroundExecutor::new(dispatcher.clone()),
            text_system,
            callbacks,
            state,
        });

        if use_wayland {
            Self {
                client: LinuxClient::Wayland(WaylandClient::new(Rc::clone(&inner))),
                inner,
            }
        } else {
            Self {
                client: LinuxClient::X11(X11Client::new(Rc::clone(&inner))),
                inner,
            }
        }
    }
}

impl Platform for LinuxPlatform {
    fn background_executor(&self) -> BackgroundExecutor {
        self.inner.background_executor.clone()
    }

    fn foreground_executor(&self) -> ForegroundExecutor {
        self.inner.foreground_executor.clone()
    }

    fn text_system(&self) -> Arc<dyn PlatformTextSystem> {
        self.inner.text_system.clone()
    }

    fn run(&self, on_finish_launching: Box<dyn FnOnce()>) {
        on_finish_launching();
        let mut event_loop = self.inner.event_loop.lock();
        let signal = event_loop.get_signal();
        event_loop
            .run(Duration::MAX, &mut (), |data| {
                if self.inner.state.lock().quit_requested {
                    signal.stop();
                }
            })
            .unwrap();
    }

    fn quit(&self) {
        self.inner.loop_signal.stop();
    }

    //todo!(linux)
    fn restart(&self) {}

    //todo!(linux)
    fn activate(&self, ignoring_other_apps: bool) {}

    //todo!(linux)
    fn hide(&self) {}

    //todo!(linux)
    fn hide_other_apps(&self) {}

    //todo!(linux)
    fn unhide_other_apps(&self) {}

    fn displays(&self) -> Vec<Rc<dyn PlatformDisplay>> {
        match &self.client {
            LinuxClient::Wayland(client) => client.displays(),
            LinuxClient::X11(client) => client.displays(),
        }
    }

    fn display(&self, id: DisplayId) -> Option<Rc<dyn PlatformDisplay>> {
        match &self.client {
            LinuxClient::Wayland(client) => client.display(id),
            LinuxClient::X11(client) => client.display(id),
        }
    }

    //todo!(linux)
    fn active_window(&self) -> Option<AnyWindowHandle> {
        None
    }

    fn open_window(
        &self,
        handle: AnyWindowHandle,
        options: WindowOptions,
    ) -> Box<dyn PlatformWindow> {
        match &self.client {
            LinuxClient::Wayland(client) => client.open_window(handle, options),
            LinuxClient::X11(client) => client.open_window(handle, options),
        }
    }

    fn open_url(&self, url: &str) {
        open::that(url);
    }

    fn on_open_urls(&self, callback: Box<dyn FnMut(Vec<String>)>) {
        self.inner.callbacks.lock().open_urls = Some(callback);
    }

    fn prompt_for_paths(
        &self,
        options: PathPromptOptions,
    ) -> oneshot::Receiver<Option<Vec<PathBuf>>> {
        let (done_tx, done_rx) = oneshot::channel();
        self.foreground_executor()
            .spawn(async move {
                let title = if options.multiple {
                    if !options.files {
                        "Open folders"
                    } else {
                        "Open files"
                    }
                } else {
                    if !options.files {
                        "Open folder"
                    } else {
                        "Open file"
                    }
                };

                let result = OpenFileRequest::default()
                    .modal(true)
                    .title(title)
                    .accept_label("Select")
                    .multiple(options.multiple)
                    .directory(options.directories)
                    .send()
                    .await
                    .ok()
                    .and_then(|request| request.response().ok())
                    .and_then(|response| {
                        response
                            .uris()
                            .iter()
                            .map(|uri| uri.to_file_path().ok())
                            .collect()
                    });

                done_tx.send(result);
            })
            .detach();
        done_rx
    }

    fn prompt_for_new_path(&self, directory: &Path) -> oneshot::Receiver<Option<PathBuf>> {
        let (done_tx, done_rx) = oneshot::channel();
        let directory = directory.to_owned();
        self.foreground_executor()
            .spawn(async move {
                let result = SaveFileRequest::default()
                    .modal(true)
                    .title("Select new path")
                    .accept_label("Accept")
                    .send()
                    .await
                    .ok()
                    .and_then(|request| request.response().ok())
                    .and_then(|response| {
                        response
                            .uris()
                            .first()
                            .and_then(|uri| uri.to_file_path().ok())
                    });

                done_tx.send(result);
            })
            .detach();
        done_rx
    }

    fn reveal_path(&self, path: &Path) {
        open::that(path);
    }

    fn on_become_active(&self, callback: Box<dyn FnMut()>) {
        self.inner.callbacks.lock().become_active = Some(callback);
    }

    fn on_resign_active(&self, callback: Box<dyn FnMut()>) {
        self.inner.callbacks.lock().resign_active = Some(callback);
    }

    fn on_quit(&self, callback: Box<dyn FnMut()>) {
        self.inner.callbacks.lock().quit = Some(callback);
    }

    fn on_reopen(&self, callback: Box<dyn FnMut()>) {
        self.inner.callbacks.lock().reopen = Some(callback);
    }

    fn on_event(&self, callback: Box<dyn FnMut(PlatformInput) -> bool>) {
        self.inner.callbacks.lock().event = Some(callback);
    }

    fn on_app_menu_action(&self, callback: Box<dyn FnMut(&dyn Action)>) {
        self.inner.callbacks.lock().app_menu_action = Some(callback);
    }

    fn on_will_open_app_menu(&self, callback: Box<dyn FnMut()>) {
        self.inner.callbacks.lock().will_open_app_menu = Some(callback);
    }

    fn on_validate_app_menu_command(&self, callback: Box<dyn FnMut(&dyn Action) -> bool>) {
        self.inner.callbacks.lock().validate_app_menu_command = Some(callback);
    }

    fn os_name(&self) -> &'static str {
        "Linux"
    }

    fn double_click_interval(&self) -> Duration {
        Duration::default()
    }

    fn os_version(&self) -> Result<SemanticVersion> {
        Ok(SemanticVersion {
            major: 1,
            minor: 0,
            patch: 0,
        })
    }

    fn app_version(&self) -> Result<SemanticVersion> {
        Ok(SemanticVersion {
            major: 1,
            minor: 0,
            patch: 0,
        })
    }

    fn app_path(&self) -> Result<PathBuf> {
        unimplemented!()
    }

    //todo!(linux)
    fn set_menus(&self, menus: Vec<Menu>, keymap: &Keymap) {}

    fn local_timezone(&self) -> UtcOffset {
        UtcOffset::UTC
    }

    fn path_for_auxiliary_executable(&self, name: &str) -> Result<PathBuf> {
        unimplemented!()
    }

    //todo!(linux)
    fn set_cursor_style(&self, style: CursorStyle) {}

    //todo!(linux)
    fn should_auto_hide_scrollbars(&self) -> bool {
        false
    }

    //todo!(linux)
    fn write_to_clipboard(&self, item: ClipboardItem) {}

    //todo!(linux)
    fn read_from_clipboard(&self) -> Option<ClipboardItem> {
        None
    }

    fn write_credentials(&self, url: &str, username: &str, password: &[u8]) -> Task<Result<()>> {
        unimplemented!()
    }

    fn read_credentials(&self, url: &str) -> Task<Result<Option<(String, Vec<u8>)>>> {
        unimplemented!()
    }

    fn delete_credentials(&self, url: &str) -> Task<Result<()>> {
        unimplemented!()
    }

    fn window_appearance(&self) -> crate::WindowAppearance {
        crate::WindowAppearance::Light
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn build_platform() -> LinuxPlatform {
        let platform = LinuxPlatform::new();
        platform
    }
}
