//! EmbeddedAlan & 4-Perspective Deep Research view (Phase 1 skeleton wired to `AppStore`).

use super::skeleton_placeholder;
use crate::stores::AppStore;
use nsh_core::prelude::*;

pub fn agent_view(store: AppStore) -> impl IntoElement {
    skeleton_placeholder(
        store,
        "RESEARCH DESK · EMBEDDED ALAN",
        "4 视角并行深研 & EmbeddedAlan 费曼审讯",
        "4-Perspective Deep Research & EmbeddedAlan",
        "已接入 4 视角实时流、Living Research Memo、Turing Tape 与 Event-Driven Alan 对话通道。",
        "Phase 1 store wired: 4-lens Deep Research stream, Living Memo, Turing Tape & Alan chat.",
        move || {
            let ticker = store.selected_ticker.get();
            let msgs = store.agent_messages.with(|m| m.len());
            let tape = store.agent_tape.with(|t| t.len());
            format!("{ticker} · {msgs} messages · {tape} tape entries")
        },
        "启动 4 视角深研",
        "Run 4-Lens Deep Research",
        move || store.run_deep_research_selected(),
    )
}
