//! Decision Journal view (Phase 1 skeleton wired to `AppStore`).

use super::skeleton_placeholder;
use crate::stores::AppStore;
use nsh_core::prelude::*;

pub fn journal_view(store: AppStore) -> impl IntoElement {
    skeleton_placeholder(
        store,
        "DECISION JOURNAL · APPEND-ONLY",
        "决策账本 · Decision Journal",
        "Decision Journal · Append-Only",
        "已接入不可变决策账本、持仓与 100% 现金 / S&P 500 基准对比状态。",
        "Phase 1 store wired: append-only decision journal, positions & S&P 500 benchmark.",
        move || {
            let entries = store.journal.with(|j| j.entries.len());
            let positions = store.journal.with(|j| j.positions.len());
            format!("{entries} entries · {positions} positions")
        },
        "刷新账本",
        "Reload Journal",
        move || store.reload_from_disk(),
    )
}
