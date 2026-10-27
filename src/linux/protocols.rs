//! Client bindings for KWin's fake input protocol (version 6, which added
//! `keyboard_keysym`; the copy bundled with wayland-protocols-plasma stops at 5).

#![allow(dead_code, non_camel_case_types, non_upper_case_globals, unused_imports, clippy::all)]

pub mod fake_input {
    use wayland_client;
    use wayland_client::protocol::*;

    pub mod __interfaces {
        use wayland_client::backend as wayland_backend;
        use wayland_client::protocol::__interfaces::*;
        wayland_scanner::generate_interfaces!("src/linux/protocols/fake-input.xml");
    }
    use self::__interfaces::*;

    wayland_scanner::generate_client_code!("src/linux/protocols/fake-input.xml");
}
