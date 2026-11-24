//! Obsidian Precision Instrument theme tokens and semantic color helpers.

use m3_core::industry::IndustryStatus;
use m3_core::model::{Bucket, JournalAction, StrikeZoneStatus};
use nsh_core::prelude::{Hsla, rgb, rgba};
use rust_decimal::Decimal;

pub const BG_SHELL: u32 = 0x0B0B0E;
pub const BG_VIEWPORT: u32 = 0x101015;
pub const BG_TOOLBAR: u32 = 0x121218;
pub const BG_SURFACE: u32 = 0x14141C;
pub const BG_WELL: u32 = 0x181822;
pub const BG_ROW_ALT: u32 = 0x121219;
pub const BG_HOVER: u32 = 0x1D1D28;
pub const BG_SELECTED_ROW: u32 = 0x1B1B27;
pub const BG_ACTIVE: u32 = 0x252534;
pub const BG_STATUSLINE: u32 = 0x0D0D13;
pub const BG_PALETTE: u32 = 0x13131C;

pub const RAIL_HOVER: u32 = 0xFFFFFF0D;
pub const RAIL_ACTIVE: u32 = 0xFFFFFF18;
pub const RAIL_SELECTED: u32 = 0xFFFFFF14;
pub const SCRIM_BACKDROP: u32 = 0x050508AA;

pub const BORDER_VIEWPORT: u32 = 0x1D1D26;
pub const BORDER_HAIRLINE: u32 = 0x1C1C26;
pub const BORDER_SUBTLE: u32 = 0x242432;
pub const BORDER_STRONG: u32 = 0x323244;
pub const BORDER_PILL: u32 = 0x2D2D3D;
pub const BORDER_PALETTE: u32 = 0x2E2E40;

pub const INK_HERO: u32 = 0xFFFFFF;
pub const INK_PRIMARY: u32 = 0xECEEF2;
pub const INK_SECONDARY: u32 = 0xA0A6B4;
pub const INK_MUTED: u32 = 0x6B7282;
pub const INK_RAIL_PRICE: u32 = 0x858C9C;

pub const CTA_BG: u32 = 0xF4F4F6;
pub const CTA_BG_HOVER: u32 = 0xFFFFFF;
pub const CTA_BG_ACTIVE: u32 = 0xD8D8DF;
pub const CTA_INK: u32 = 0x09090C;

pub const EMERALD_FG: u32 = 0x34D399;
pub const EMERALD_BG: u32 = 0x10B9811E;
pub const EMERALD_BORDER: u32 = 0x34D39945;
pub const EMERALD_BAND: u32 = 0x10B98114;
pub const EMERALD_LINE: u32 = 0x34D399CC;
pub const EMERALD_VOL: u32 = 0x34D39938;

pub const GOLD_FG: u32 = 0xFBBF24;
pub const GOLD_BG: u32 = 0xF59E0B1E;
pub const GOLD_BORDER: u32 = 0xFBBF2445;

pub const CORAL_FG: u32 = 0xF87171;
pub const CORAL_BG: u32 = 0xEF44441E;
pub const CORAL_BORDER: u32 = 0xF8717145;
pub const CORAL_VOL: u32 = 0xF8717138;

pub const ORANGE_FG: u32 = 0xFB923C;
pub const ORANGE_BG: u32 = 0xF973161E;
pub const ORANGE_BORDER: u32 = 0xFB923C45;

pub const SKY_FG: u32 = 0x60A5FA;
pub const SKY_BG: u32 = 0x3B82F61E;
pub const SKY_BORDER: u32 = 0x60A5FA45;

pub const VIOLET_FG: u32 = 0xA78BFA;
pub const VIOLET_BG: u32 = 0x8B5CF61E;
pub const VIOLET_BORDER: u32 = 0xA78BFA45;

pub const SLATE_FG: u32 = 0x94A3B8;
pub const SLATE_BG: u32 = 0x94A3B818;
pub const SLATE_BORDER: u32 = 0x94A3B836;

pub const PIXEL_M3: [&str; 10] = [
    "##....##..#####.",
    "###..###..######",
    "########......##",
    "##.##.##......##",
    "##.##.##....###.",
    "##....##....###.",
    "##....##......##",
    "##....##......##",
    "##....##..######",
    "##....##..#####.",
];

/// Canonical font families for UI copy, tabular numbers, and monospace labels.
/// Using NSH generic families (`system-ui` / `ui-monospace`) resolves directly to
/// `NSFont::systemFontOfSize_weight` and `NSFont::monospacedSystemFontOfSize_weight`
/// (SF Mono on macOS) with zero CoreText family-lookup warnings.
pub const FONT_UI: &str = "system-ui";
pub const FONT_MONO_NUM: &str = "ui-monospace";
pub const FONT_MONO_TERM: &str = "ui-monospace";

/// Typography scale tokens (points).
pub const TEXT_2XS: f32 = 10.0;
pub const TEXT_XS: f32 = 11.0;
pub const TEXT_SM: f32 = 12.0;
pub const TEXT_BASE: f32 = 13.0;
pub const TEXT_MD: f32 = 14.0;
pub const TEXT_LG: f32 = 16.0;
pub const TEXT_XL: f32 = 20.0;

/// Layout spacing tokens (points).
pub const SIDEBAR_WIDTH: f32 = 224.0;
pub const TOOLBAR_HEIGHT: f32 = 40.0;
pub const STATUSLINE_HEIGHT: f32 = 24.0;

/// Converts an opaque `0xRRGGBB` hex token to [`Hsla`].
pub fn hsla_rgb(hex: u32) -> Hsla {
    rgb(hex).into()
}

/// Converts a `0xRRGGBBAA` hex token to [`Hsla`].
pub fn hsla_rgba(hex: u32) -> Hsla {
    rgba(hex).into()
}

pub fn share_delta_color(val: Option<Decimal>, pol: i8) -> u32 {
    match val {
        Some(v) if v <= Decimal::new(-5, 1) => EMERALD_FG,
        Some(v) if v > Decimal::from(2) => GOLD_FG,
        Some(_) if pol == 0 => INK_MUTED,
        Some(_) => INK_SECONDARY,
        None => INK_MUTED,
    }
}

pub fn bucket_colors(bucket: Bucket) -> (u32, u32, u32) {
    match bucket {
        Bucket::Core => (EMERALD_FG, EMERALD_BG, EMERALD_BORDER),
        Bucket::Watch => (SKY_FG, SKY_BG, SKY_BORDER),
        Bucket::Out => (ORANGE_FG, ORANGE_BG, ORANGE_BORDER),
        Bucket::TooHard => (SLATE_FG, SLATE_BG, SLATE_BORDER),
    }
}

pub fn status_colors(status: StrikeZoneStatus) -> (u32, u32, u32) {
    match status {
        StrikeZoneStatus::InStrikeZone => (EMERALD_FG, EMERALD_BG, EMERALD_BORDER),
        StrikeZoneStatus::NearStrikeZone => (GOLD_FG, GOLD_BG, GOLD_BORDER),
        StrikeZoneStatus::WarningBlocked => (CORAL_FG, CORAL_BG, CORAL_BORDER),
        StrikeZoneStatus::Out => (ORANGE_FG, ORANGE_BG, ORANGE_BORDER),
        StrikeZoneStatus::Wait | StrikeZoneStatus::TooHard | StrikeZoneStatus::NoFinancials => {
            (SLATE_FG, SLATE_BG, SLATE_BORDER)
        }
    }
}

pub fn status_dot_color(status: StrikeZoneStatus) -> u32 {
    match status {
        StrikeZoneStatus::InStrikeZone => EMERALD_FG,
        StrikeZoneStatus::NearStrikeZone => GOLD_FG,
        StrikeZoneStatus::WarningBlocked => CORAL_FG,
        StrikeZoneStatus::Out => ORANGE_FG,
        StrikeZoneStatus::Wait | StrikeZoneStatus::TooHard | StrikeZoneStatus::NoFinancials => {
            INK_MUTED
        }
    }
}

pub fn journal_action_colors(action: JournalAction) -> (u32, u32, u32) {
    match action {
        JournalAction::Wait => (SKY_FG, SKY_BG, SKY_BORDER),
        JournalAction::Out => (ORANGE_FG, ORANGE_BG, ORANGE_BORDER),
        JournalAction::TooHard => (SLATE_FG, SLATE_BG, SLATE_BORDER),
        JournalAction::PaperBuy => (EMERALD_FG, EMERALD_BG, EMERALD_BORDER),
        JournalAction::PaperSell => (CORAL_FG, CORAL_BG, CORAL_BORDER),
    }
}

pub fn industry_status_colors(status: IndustryStatus) -> (u32, u32, u32) {
    match status {
        IndustryStatus::InsideCircle => (EMERALD_FG, EMERALD_BG, EMERALD_BORDER),
        IndustryStatus::Studying => (SKY_FG, SKY_BG, SKY_BORDER),
        IndustryStatus::TooHard => (CORAL_FG, CORAL_BG, CORAL_BORDER),
    }
}
