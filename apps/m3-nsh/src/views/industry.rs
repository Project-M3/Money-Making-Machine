//! Industry Research Studio view (Phase 1 skeleton wired to `AppStore`).

use super::skeleton_placeholder;
use crate::stores::AppStore;
use nsh_core::prelude::*;

pub fn industry_view(store: AppStore) -> impl IntoElement {
    skeleton_placeholder(
        store,
        "INDUSTRY STUDIO · ALAN",
        "行业研究工作台 · Industry Research",
        "Industry Research Studio · Alan",
        "已接入 IndustryProfile 列表、L0 同行矩阵、4 视角行业深研流、费曼自测与 ## Human Notes 保护。",
        "Phase 1 store wired: IndustryProfile, L0 peer matrix, 4-lens research, Feynman quiz & Human Notes.",
        move || {
            let studies = store.industries.with(|v| v.len());
            let clusters = store.themes_file.with(|t| t.clusters.len());
            format!("{studies} studies · {clusters} archetype clusters")
        },
        "同步主题图谱",
        "Sync Theme Graph",
        move || store.start_themes_sync(),
    )
}
