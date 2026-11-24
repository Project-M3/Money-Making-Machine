//! US Universe Radar L0/L1 view (Phase 1 skeleton wired to `AppStore`).

use super::skeleton_placeholder;
use crate::stores::AppStore;
use nsh_core::prelude::*;

pub fn universe_view(store: AppStore) -> impl IntoElement {
    skeleton_placeholder(
        store,
        "DUCKDB L0/L1 · VIRTUALIZED",
        "全市场雷达 · US Lakehouse L0/L1",
        "Universe Radar · US Lakehouse L0/L1",
        "已接入 6,000+ 美股虚拟列表底座、关键词 + 异步 Lookalike 检索与右侧检验抽屉状态。",
        "Phase 1 store wired: 6,000+ rows, screening presets, async Lookalike & inspection drawer.",
        move || {
            let loaded = store.universe_rows.with(|r| r.len());
            let total = store
                .universe_summary
                .with(|s| s.as_ref().map(|x| x.total_companies).unwrap_or(0));
            format!("{loaded} screened / {total} L0 issuers")
        },
        "同步全美股 L0",
        "Sync US L0 Lakehouse",
        move || store.start_universe_sync(false),
    )
}
