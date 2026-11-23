use crate::model::{Bucket, JournalAction, StrikeZoneStatus};
use rust_decimal::Decimal;

// M3 Obsidian Precision Instrument Theme (unified with #0B0B0E / #FFFFFF pixel-art AppIcon)
pub(super) const BG_SHELL: u32 = 0x0B0B0E;
pub(super) const BG_VIEWPORT: u32 = 0x101015;
pub(super) const BG_TOOLBAR: u32 = 0x121218;
pub(super) const BG_SURFACE: u32 = 0x14141C;
pub(super) const BG_WELL: u32 = 0x181822;
pub(super) const BG_ROW_ALT: u32 = 0x121219;
pub(super) const BG_HOVER: u32 = 0x1D1D28;
pub(super) const BG_SELECTED_ROW: u32 = 0x1B1B27;
pub(super) const BG_ACTIVE: u32 = 0x252534;
pub(super) const BG_STATUSLINE: u32 = 0x0D0D13;
pub(super) const BG_PALETTE: u32 = 0x13131C;

pub(super) const RAIL_HOVER: u32 = 0xFFFFFF0D;
pub(super) const RAIL_ACTIVE: u32 = 0xFFFFFF18;
pub(super) const RAIL_SELECTED: u32 = 0xFFFFFF14;
pub(super) const SCRIM_BACKDROP: u32 = 0x050508AA;

pub(super) const BORDER_VIEWPORT: u32 = 0x1D1D26;
pub(super) const BORDER_HAIRLINE: u32 = 0x1C1C26;
pub(super) const BORDER_SUBTLE: u32 = 0x242432;
pub(super) const BORDER_STRONG: u32 = 0x323244;
pub(super) const BORDER_PILL: u32 = 0x2D2D3D;
pub(super) const BORDER_PALETTE: u32 = 0x2E2E40;

pub(super) const INK_HERO: u32 = 0xFFFFFF;
pub(super) const INK_PRIMARY: u32 = 0xECEEF2;
pub(super) const INK_SECONDARY: u32 = 0xA0A6B4;
pub(super) const INK_MUTED: u32 = 0x6B7282;
pub(super) const INK_RAIL_PRICE: u32 = 0x858C9C;

pub(super) const CTA_BG: u32 = 0xF4F4F6;
pub(super) const CTA_BG_HOVER: u32 = 0xFFFFFF;
pub(super) const CTA_BG_ACTIVE: u32 = 0xD8D8DF;
pub(super) const CTA_INK: u32 = 0x09090C;

pub(super) const EMERALD_FG: u32 = 0x34D399;
pub(super) const EMERALD_BG: u32 = 0x10B9811E;
pub(super) const EMERALD_BORDER: u32 = 0x34D39945;
pub(super) const EMERALD_BAND: u32 = 0x10B98114;
pub(super) const EMERALD_LINE: u32 = 0x34D399CC;
pub(super) const EMERALD_VOL: u32 = 0x34D39938;

pub(super) const GOLD_FG: u32 = 0xFBBF24;
pub(super) const GOLD_BG: u32 = 0xF59E0B1E;
pub(super) const GOLD_BORDER: u32 = 0xFBBF2445;

pub(super) const CORAL_FG: u32 = 0xF87171;
pub(super) const CORAL_BG: u32 = 0xEF44441E;
pub(super) const CORAL_BORDER: u32 = 0xF8717145;
pub(super) const CORAL_VOL: u32 = 0xF8717138;

pub(super) const ORANGE_FG: u32 = 0xFB923C;
pub(super) const ORANGE_BG: u32 = 0xF973161E;
pub(super) const ORANGE_BORDER: u32 = 0xFB923C45;

pub(super) const SKY_FG: u32 = 0x60A5FA;
pub(super) const SKY_BG: u32 = 0x3B82F61E;
pub(super) const SKY_BORDER: u32 = 0x60A5FA45;

pub(super) const VIOLET_FG: u32 = 0xA78BFA;
pub(super) const VIOLET_BG: u32 = 0x8B5CF61E;
pub(super) const VIOLET_BORDER: u32 = 0xA78BFA45;

pub(super) const SLATE_FG: u32 = 0x94A3B8;
pub(super) const SLATE_BG: u32 = 0x94A3B818;
pub(super) const SLATE_BORDER: u32 = 0x94A3B836;

pub(super) const PIXEL_M3: [&str; 10] = [
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

pub(super) fn share_delta_color(val: Option<Decimal>, pol: i8) -> u32 {
    match val {
        Some(v) if v <= Decimal::new(-5, 1) => EMERALD_FG,
        Some(v) if v > Decimal::from(2) => GOLD_FG,
        Some(_) if pol == 0 => INK_MUTED,
        Some(_) => INK_SECONDARY,
        None => INK_MUTED,
    }
}

pub(super) fn bucket_colors(bucket: Bucket) -> (u32, u32, u32) {
    match bucket {
        Bucket::Core => (EMERALD_FG, EMERALD_BG, EMERALD_BORDER),
        Bucket::Watch => (SKY_FG, SKY_BG, SKY_BORDER),
        Bucket::Out => (ORANGE_FG, ORANGE_BG, ORANGE_BORDER),
        Bucket::TooHard => (SLATE_FG, SLATE_BG, SLATE_BORDER),
    }
}

pub(super) fn status_colors(status: StrikeZoneStatus) -> (u32, u32, u32) {
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

pub(super) fn status_dot_color(status: StrikeZoneStatus) -> u32 {
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

pub(super) fn journal_action_colors(action: JournalAction) -> (u32, u32, u32) {
    match action {
        JournalAction::Wait => (SKY_FG, SKY_BG, SKY_BORDER),
        JournalAction::Out => (ORANGE_FG, ORANGE_BG, ORANGE_BORDER),
        JournalAction::TooHard => (SLATE_FG, SLATE_BG, SLATE_BORDER),
        JournalAction::PaperBuy => (EMERALD_FG, EMERALD_BG, EMERALD_BORDER),
        JournalAction::PaperSell => (CORAL_FG, CORAL_BG, CORAL_BORDER),
    }
}
