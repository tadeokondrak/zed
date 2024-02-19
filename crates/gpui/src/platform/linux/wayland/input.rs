use crate::platform::linux::wayland::WaylandClientState;
use crate::{
    KeyDownEvent, KeyUpEvent, Keystroke, Modifiers, MouseButton, MouseDownEvent, MouseMoveEvent,
    MouseUpEvent, Pixels, PlatformInput, Point, ScrollDelta, ScrollWheelEvent, TouchPhase,
};
use std::rc::Rc;
use wayland_client::{
    protocol::{
        wl_keyboard,
        wl_pointer::{self, AxisRelativeDirection},
        wl_seat,
    },
    Connection, Dispatch, Proxy, QueueHandle, WEnum,
};
use xkbcommon::xkb::{self, ffi::XKB_KEYMAP_FORMAT_TEXT_V1, KEYMAP_COMPILE_NO_FLAGS};


impl Dispatch<wl_seat::WlSeat, ()> for WaylandClientState {
    fn event(
        state: &mut Self,
        seat: &wl_seat::WlSeat,
        event: wl_seat::Event,
        data: &(),
        conn: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let wl_seat::Event::Capabilities {
            capabilities: WEnum::Value(capabilities),
        } = event
        {
            if capabilities.contains(wl_seat::Capability::Keyboard) {
                seat.get_keyboard(qh, ());
            }
            if capabilities.contains(wl_seat::Capability::Pointer) {
                seat.get_pointer(qh, ());
            }
        }
    }
}

impl Dispatch<wl_keyboard::WlKeyboard, ()> for WaylandClientState {
    fn event(
        state: &mut Self,
        keyboard: &wl_keyboard::WlKeyboard,
        event: wl_keyboard::Event,
        data: &(),
        conn: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        match event {
            wl_keyboard::Event::Keymap {
                format: WEnum::Value(format),
                fd,
                size,
                ..
            } => {
                assert_eq!(
                    format,
                    wl_keyboard::KeymapFormat::XkbV1,
                    "Unsupported keymap format"
                );
                let keymap = unsafe {
                    xkb::Keymap::new_from_fd(
                        &xkb::Context::new(xkb::CONTEXT_NO_FLAGS),
                        fd,
                        size as usize,
                        XKB_KEYMAP_FORMAT_TEXT_V1,
                        KEYMAP_COMPILE_NO_FLAGS,
                    )
                    .unwrap()
                }
                .unwrap();
                state.keymap_state = Some(xkb::State::new(&keymap));
            }
            wl_keyboard::Event::Enter { surface, .. } => {
                for window in &state.windows {
                    if window.surface.id() == surface.id() {
                        state.keyboard_focused_window = Some(Rc::clone(&window));
                    }
                }
            }
            wl_keyboard::Event::Modifiers {
                mods_depressed,
                mods_latched,
                mods_locked,
                group,
                ..
            } => {
                let keymap_state = state.keymap_state.as_mut().unwrap();
                keymap_state.update_mask(mods_depressed, mods_latched, mods_locked, 0, 0, group);
                state.modifiers.shift =
                    keymap_state.mod_name_is_active(xkb::MOD_NAME_SHIFT, xkb::STATE_MODS_EFFECTIVE);
                state.modifiers.alt =
                    keymap_state.mod_name_is_active(xkb::MOD_NAME_ALT, xkb::STATE_MODS_EFFECTIVE);
                state.modifiers.control =
                    keymap_state.mod_name_is_active(xkb::MOD_NAME_CTRL, xkb::STATE_MODS_EFFECTIVE);
                state.modifiers.command =
                    keymap_state.mod_name_is_active(xkb::MOD_NAME_LOGO, xkb::STATE_MODS_EFFECTIVE);
            }
            wl_keyboard::Event::Key {
                key,
                state: WEnum::Value(key_state),
                ..
            } => {
                const MIN_KEYCODE: u32 = 8; // used to convert evdev scancode to xkb scancode

                let keymap_state = state.keymap_state.as_ref().unwrap();
                let keycode = xkb::Keycode::from(key + MIN_KEYCODE);
                let key_utf32 = keymap_state.key_get_utf32(keycode);
                let key_utf8 = keymap_state.key_get_utf8(keycode);
                let key_sym = keymap_state.key_get_one_sym(keycode);
                let key = xkb::keysym_get_name(key_sym).to_lowercase();

                // Ignore control characters (and DEL) for the purposes of ime_key,
                // but if key_utf32 is 0 then assume it isn't one
                let ime_key =
                    (key_utf32 == 0 || (key_utf32 >= 32 && key_utf32 != 127)).then_some(key_utf8);

                let focused_window = &state.keyboard_focused_window;
                if let Some(focused_window) = focused_window {
                    match key_state {
                        wl_keyboard::KeyState::Pressed => {
                            focused_window.handle_input(PlatformInput::KeyDown(KeyDownEvent {
                                keystroke: Keystroke {
                                    modifiers: state.modifiers,
                                    key,
                                    ime_key,
                                },
                                is_held: false, // todo!(linux)
                            }));
                        }
                        wl_keyboard::KeyState::Released => {
                            focused_window.handle_input(PlatformInput::KeyUp(KeyUpEvent {
                                keystroke: Keystroke {
                                    modifiers: state.modifiers,
                                    key,
                                    ime_key,
                                },
                            }));
                        }
                        _ => {}
                    }
                }
            }
            wl_keyboard::Event::Leave { .. } => {}
            _ => {}
        }
    }
}

fn linux_button_to_gpui(button: u32) -> MouseButton {
    match button {
        0x110 => MouseButton::Left,
        0x111 => MouseButton::Right,
        0x112 => MouseButton::Middle,
        _ => unimplemented!(), // todo!(linux)
    }
}

impl Dispatch<wl_pointer::WlPointer, ()> for WaylandClientState {
    fn event(
        state: &mut Self,
        wl_pointer: &wl_pointer::WlPointer,
        event: wl_pointer::Event,
        data: &(),
        conn: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        match event {
            wl_pointer::Event::Enter {
                surface,
                surface_x,
                surface_y,
                ..
            } => {
                for window in &state.windows {
                    if window.surface.id() == surface.id() {
                        state.mouse_focused_window = Some(Rc::clone(&window));
                    }
                }
                state.mouse_location = Some(Point {
                    x: Pixels::from(surface_x),
                    y: Pixels::from(surface_y),
                });
            }
            wl_pointer::Event::Motion {
                time,
                surface_x,
                surface_y,
                ..
            } => {
                let focused_window = &state.mouse_focused_window;
                if let Some(focused_window) = focused_window {
                    state.mouse_location = Some(Point {
                        x: Pixels::from(surface_x),
                        y: Pixels::from(surface_y),
                    });
                    focused_window.handle_input(PlatformInput::MouseMove(MouseMoveEvent {
                        position: state.mouse_location.unwrap(),
                        pressed_button: state.button_pressed,
                        modifiers: state.modifiers,
                    }))
                }
            }
            wl_pointer::Event::Button {
                button,
                state: WEnum::Value(button_state),
                ..
            } => {
                let focused_window = &state.mouse_focused_window;
                let mouse_location = &state.mouse_location;
                if let (Some(focused_window), Some(mouse_location)) =
                    (focused_window, mouse_location)
                {
                    match button_state {
                        wl_pointer::ButtonState::Pressed => {
                            state.button_pressed = Some(linux_button_to_gpui(button));
                            focused_window.handle_input(PlatformInput::MouseDown(MouseDownEvent {
                                button: linux_button_to_gpui(button),
                                position: *mouse_location,
                                modifiers: state.modifiers,
                                click_count: 1,
                            }));
                        }
                        wl_pointer::ButtonState::Released => {
                            state.button_pressed = None;
                            focused_window.handle_input(PlatformInput::MouseUp(MouseUpEvent {
                                button: linux_button_to_gpui(button),
                                position: *mouse_location,
                                modifiers: Modifiers::default(),
                                click_count: 1,
                            }));
                        }
                        _ => {}
                    }
                }
            }
            wl_pointer::Event::AxisRelativeDirection {
                direction: WEnum::Value(direction),
                ..
            } => {
                state.scroll_direction = match direction {
                    AxisRelativeDirection::Identical => -1.0,
                    AxisRelativeDirection::Inverted => 1.0,
                    _ => -1.0,
                }
            }
            wl_pointer::Event::Axis {
                time,
                axis: WEnum::Value(axis),
                value,
                ..
            } => {
                let focused_window = &state.mouse_focused_window;
                let mouse_location = &state.mouse_location;
                if let (Some(focused_window), Some(mouse_location)) =
                    (focused_window, mouse_location)
                {
                    let value = value * state.scroll_direction;
                    focused_window.handle_input(PlatformInput::ScrollWheel(ScrollWheelEvent {
                        position: *mouse_location,
                        delta: match axis {
                            wl_pointer::Axis::VerticalScroll => {
                                ScrollDelta::Pixels(Point::new(Pixels(0.0), Pixels(value as f32)))
                            }
                            wl_pointer::Axis::HorizontalScroll => {
                                ScrollDelta::Pixels(Point::new(Pixels(value as f32), Pixels(0.0)))
                            }
                            _ => unimplemented!(),
                        },
                        modifiers: state.modifiers,
                        touch_phase: TouchPhase::Started,
                    }))
                }
            }
            wl_pointer::Event::Leave { surface, .. } => {
                let focused_window = &state.mouse_focused_window;
                if let Some(focused_window) = focused_window {
                    focused_window.handle_input(PlatformInput::MouseMove(MouseMoveEvent {
                        position: Point::<Pixels>::default(),
                        pressed_button: None,
                        modifiers: Modifiers::default(),
                    }));
                }
                state.mouse_focused_window = None;
                state.mouse_location = None;
            }
            _ => {}
        }
    }
}
