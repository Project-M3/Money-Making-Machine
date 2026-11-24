//! Circle Overview (CORE / WATCH / OUT / TOO HARD) view (Phase 1 skeleton wired to `AppStore`).

use super::skeleton_placeholder;
use crate::stores::AppStore;
use nsh_core::prelude::*;

pub fn baskets_view(store: AppStore) -> impl IntoElement {
    skeleton_placeholder(
        store,
        "CIRCLE OVERVIEW · FOUR TIERS",
        "能力圈总览 · Circle Overview",
        "Circle Overview · Four Tiers",
        "已接入 CORE / WATCH / OUT / TOO HARD 四层分桶状态、列表/看板切换与快速筛选。",
        "Phase 1 store wired: CORE / WATCH / OUT / TOO HARD tiers, list/board toggle & filters.",
        move || {
            let count = store.reports.with(|r| r.len());
            format!("{count} companies in Circle")
        },
        "切换列表 / 看板",
        "Toggle List / Board",
        move || store.toggle_baskets_view(),
    )
}
