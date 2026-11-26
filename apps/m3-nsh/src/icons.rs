//! Original outline SVG icons fetched from IconMCP (`https://iconmcp.com/mcp`, MIT license)
//! and rendered natively by NSH 0.7.0's `svg(ImageSrc::svg_markup(...))` (`resvg` backend).
//!
//! All SVGs preserve `stroke="currentColor"` (and `fill="currentColor"` where needed) so their
//! foreground tint follows `.text_color(...)` cleanly on the element itself or a parent container.

use nsh_core::element::ImageSrc;
use nsh_core::prelude::*;

pub const ICON_SIDEBAR_LEFT: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.25" stroke-linecap="round" stroke-linejoin="round"><g><rect x="2.25" y="2.75" width="11.5" height="10.5" rx="2"/><line x1="6" y1="2.75" x2="6" y2="13.25"/></g></svg>"#;

pub const ICON_ARROW_LEFT: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.25" stroke-linecap="round" stroke-linejoin="round"><g transform="rotate(180 8 8)"><line x1="2.75" y1="8" x2="13.25" y2="8"/><path d="M8.5 3.25 L13.25 8 L8.5 12.75"/></g></svg>"#;

pub const ICON_ARROW_RIGHT: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.25" stroke-linecap="round" stroke-linejoin="round"><g><line x1="2.75" y1="8" x2="13.25" y2="8"/><path d="M8.5 3.25 L13.25 8 L8.5 12.75"/></g></svg>"#;

pub const ICON_PLUS: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.25" stroke-linecap="round" stroke-linejoin="round"><g><line x1="8" y1="3.25" x2="8" y2="12.75"/><line x1="3.25" y1="8" x2="12.75" y2="8"/></g></svg>"#;

pub const ICON_SEARCH: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.25" stroke-linecap="round" stroke-linejoin="round"><g><circle cx="7" cy="7" r="4.75"/><line x1="10.375" y1="10.375" x2="13.75" y2="13.75"/></g></svg>"#;

pub const ICON_BELL: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.25" stroke-linecap="round" stroke-linejoin="round"><g><path d="M3.75 10.75 V6.5 A4.25 4.25 0 0 1 12.25 6.5 V10.75 L13.25 11.75 H2.75 Z"/><line x1="6.5" y1="13.75" x2="9.5" y2="13.75"/></g></svg>"#;

pub const ICON_REFRESH: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.25" stroke-linecap="round" stroke-linejoin="round"><g><path d="M13.75 8 A5.75 5.75 0 1 1 8 2.25 C9.6 2.25 11.1 2.9 12.25 4 L13.75 5.5"/><path d="M13.75 2.25 L13.75 5.5 L10.5 5.5"/></g></svg>"#;

pub const ICON_SETTINGS: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.25" stroke-linecap="round" stroke-linejoin="round"><g><path d="M6.663 3.269 L6.631 2.646 C6.611 2.246 6.93 1.91 7.33 1.91 L8.67 1.91 C9.07 1.91 9.389 2.246 9.369 2.646 L9.337 3.269 C9.324 3.532 9.459 3.779 9.686 3.911 L10.698 4.495 C10.926 4.626 11.208 4.619 11.428 4.476 L11.953 4.137 C12.289 3.92 12.739 4.028 12.939 4.375 L13.609 5.535 C13.809 5.882 13.678 6.326 13.322 6.508 L12.766 6.793 C12.532 6.912 12.385 7.153 12.385 7.416 L12.385 8.584 C12.385 8.847 12.532 9.088 12.766 9.207 L13.322 9.492 C13.678 9.674 13.809 10.118 13.609 10.465 L12.939 11.625 C12.739 11.972 12.289 12.08 11.953 11.863 L11.428 11.524 C11.208 11.381 10.926 11.374 10.698 11.505 L9.686 12.089 C9.459 12.221 9.324 12.468 9.337 12.731 L9.369 13.354 C9.389 13.754 9.07 14.09 8.67 14.09 L7.33 14.09 C6.93 14.09 6.611 13.754 6.631 13.354 L6.663 12.731 C6.676 12.468 6.541 12.221 6.314 12.089 L5.302 11.505 C5.074 11.374 4.792 11.381 4.572 11.524 L4.047 11.863 C3.711 12.08 3.261 11.972 3.061 11.625 L2.391 10.465 C2.191 10.118 2.322 9.674 2.678 9.492 L3.234 9.207 C3.468 9.088 3.615 8.847 3.615 8.584 L3.615 7.416 C3.615 7.153 3.468 6.912 3.234 6.793 L2.678 6.508 C2.322 6.326 2.191 5.882 2.391 5.535 L3.061 4.375 C3.261 4.028 3.711 3.92 4.047 4.137 L4.572 4.476 C4.792 4.619 5.074 4.626 5.302 4.495 L6.314 3.911 C6.541 3.779 6.676 3.532 6.663 3.269 Z"/><circle cx="8" cy="8" r="2"/></g></svg>"#;

pub const ICON_INBOX: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.25" stroke-linecap="round" stroke-linejoin="round"><g><path d="M2.31 8.334 L4.011 3.658 C4.154 3.263 4.53 3 4.95 3 L11.05 3 C11.47 3 11.846 3.263 11.989 3.658 L13.69 8.334 C13.73 8.444 13.75 8.56 13.75 8.676 L13.75 12 C13.75 12.552 13.302 13 12.75 13 L3.25 13 C2.698 13 2.25 12.552 2.25 12 L2.25 8.676 C2.25 8.56 2.27 8.444 2.31 8.334 Z"/><path d="M2.25 8.5 L5.152 8.5 C5.367 8.5 5.566 8.615 5.673 8.802 L6.327 9.948 C6.434 10.135 6.633 10.25 6.848 10.25 L9.152 10.25 C9.367 10.25 9.566 10.135 9.673 9.948 L10.327 8.802 C10.434 8.615 10.633 8.5 10.848 8.5 L13.75 8.5"/></g></svg>"#;

pub const ICON_RADAR: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.25" stroke-linecap="round" stroke-linejoin="round"><g><circle cx="8" cy="8" r="6"/><circle cx="8" cy="8" r="2.75"/><line x1="8" y1="8" x2="12.25" y2="3.75"/></g></svg>"#;

pub const ICON_LAYERS: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.25" stroke-linecap="round" stroke-linejoin="round"><g><path d="M8.381 2.206 L12.701 4.547 C13.26 4.849 13.26 5.651 12.701 5.953 L8.381 8.294 C8.143 8.422 7.857 8.422 7.619 8.294 L3.299 5.953 C2.74 5.651 2.74 4.849 3.299 4.547 L7.619 2.206 C7.857 2.078 8.143 2.078 8.381 2.206 Z"/><path d="M2 8 L8 11.25 L14 8"/><path d="M2 10.75 L8 14 L14 10.75"/></g></svg>"#;

pub const ICON_TARGET: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.25" stroke-linecap="round" stroke-linejoin="round"><g><circle cx="8" cy="8" r="6"/><circle cx="8" cy="8" r="3.25"/><circle cx="8" cy="8" r="0.825" fill="currentColor" stroke="none"/></g></svg>"#;

pub const ICON_JOURNAL: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.25" stroke-linecap="round" stroke-linejoin="round"><g><path d="M3.25 12.5 V3.5 A1.5 1.5 0 0 1 4.75 2 H12.75 V14 H4.75 A1.5 1.5 0 0 1 3.25 12.5 Z"/><path d="M3.25 12.5 A1.5 1.5 0 0 1 4.75 11 H12.75"/><path d="M5.25 5.25 H11 M5.25 8 H11 M8 3.75 V9.5"/></g></svg>"#;

pub const ICON_CLOSE: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.25" stroke-linecap="round" stroke-linejoin="round"><g><line x1="3.75" y1="3.75" x2="12.25" y2="12.25"/><line x1="12.25" y1="3.75" x2="3.75" y2="12.25"/></g></svg>"#;

pub const ICON_ARROW_CIRCLE_UP: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.25" stroke-linecap="round" stroke-linejoin="round"><g transform="rotate(-90 8 8)"><circle cx="8" cy="8" r="6"/><line x1="5" y1="8" x2="11" y2="8"/><path d="M8.75 5.75 L11 8 L8.75 10.25"/></g></svg>"#;

pub const ICON_CHECK: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.25" stroke-linecap="round" stroke-linejoin="round"><g><path d="M2.75 8.5 L6.25 12 L13.25 4.5"/></g></svg>"#;

pub const ICON_CIRCLE_CHECK: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.25" stroke-linecap="round" stroke-linejoin="round"><g><circle cx="8" cy="8" r="6"/><path d="M5.25 8.25 L7.25 10.25 L10.75 6.25"/></g></svg>"#;

pub const ICON_ALERT: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.25" stroke-linecap="round" stroke-linejoin="round"><g><path d="M9.311 4.36 L13.012 11.022 C13.567 12.021 12.844 13.25 11.701 13.25 L4.299 13.25 C3.156 13.25 2.433 12.021 2.988 11.022 L6.689 4.36 C7.26 3.332 8.74 3.332 9.311 4.36 Z"/><line x1="8" y1="6.25" x2="8" y2="8.75"/><circle cx="8" cy="10.9" r="0.825" fill="currentColor" stroke="none"/></g></svg>"#;

pub const ICON_ENTER: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.25" stroke-linecap="round" stroke-linejoin="round"><g><rect x="2.25" y="2.25" width="11.5" height="11.5" rx="2"/><path d="M10.75 4.75 L10.75 8 C10.75 8.552 10.302 9 9.75 9 L5.25 9"/><path d="M7.25 7 L5.25 9 L7.25 11"/></g></svg>"#;

pub const ICON_CHEVRON_DOWN: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.25" stroke-linecap="round" stroke-linejoin="round"><g transform="rotate(90 8 8)"><path d="M5.875 3.75 L10.125 8 L5.875 12.25"/></g></svg>"#;

pub const ICON_CHEVRON_RIGHT: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.25" stroke-linecap="round" stroke-linejoin="round"><g><path d="M5.875 3.75 L10.125 8 L5.875 12.25"/></g></svg>"#;

pub const ICON_CHEVRON_LEFT: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.25" stroke-linecap="round" stroke-linejoin="round"><g transform="rotate(180 8 8)"><path d="M5.875 3.75 L10.125 8 L5.875 12.25"/></g></svg>"#;

/// Crisp monochrome `18x18` macOS Menu Bar template icon matching `PIXEL_M3` (transparent background).
pub const ICON_TRAY_M3: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="18" height="18" viewBox="0 0 18 18" shape-rendering="crispEdges"><g fill="#000000"><rect x="1" y="4" width="2" height="10"/><rect x="7" y="4" width="2" height="10"/><rect x="3" y="5" width="1" height="2"/><rect x="6" y="5" width="1" height="2"/><rect x="4" y="6" width="2" height="3"/><rect x="11" y="4" width="5" height="1"/><rect x="11" y="5" width="6" height="1"/><rect x="15" y="6" width="2" height="2"/><rect x="13" y="8" width="3" height="2"/><rect x="15" y="10" width="2" height="2"/><rect x="11" y="12" width="6" height="1"/><rect x="11" y="13" width="5" height="1"/></g></svg>"##;

/// Renders a crisp outline SVG icon from IconMCP, inheriting `currentColor` from `.text_color(...)`.
pub fn icon(markup: &'static str, size_px: f32) -> Svg {
    svg(ImageSrc::svg_markup(markup))
        .w(px(size_px))
        .h(px(size_px))
        .flex_shrink_0()
}
