//! NSH 0.5.1 cross-platform UI crate for Project M3.

pub mod app;
pub mod components;
pub mod format;
pub mod palette;
pub mod platform_desktop;
pub mod stores;
pub mod theme;
pub mod views;

#[cfg(test)]
mod tests;

use nsh_core::prelude::*;

/// Registers `App` for NSH runners and app bundling (`nsh.json`).
#[native_module("m3/native")]
fn native(registry: &mut Registry) {
    registry.register::<App>();
}

/// Application root mounted by NSH across desktop and future iOS targets.
#[component]
pub fn App() -> impl IntoElement {
    app::render_app_root()
}
