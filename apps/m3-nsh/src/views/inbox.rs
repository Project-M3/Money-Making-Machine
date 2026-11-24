//! Event-Driven L1 Briefing Inbox view (Phase 1 skeleton wired to `AppStore`).

use super::skeleton_placeholder;
use crate::stores::AppStore;
use nsh_core::prelude::*;

pub fn inbox_view(store: AppStore) -> impl IntoElement {
    skeleton_placeholder(
        store,
        "L1 INBOX · 0 FLAGSHIP TOKEN",
        "简报收件箱 · Event-Driven L1",
        "Briefing Inbox · Event-Driven L1",
        "等待 Wave 3 Inbox 视图实现：过滤标签、j/k 键盘分拣、10-K 业务简介拉取与点击直达。",
        "Phase 1 shell wired: signal filters, j/k/w/x/e/u/d triage, 10-K fetch & drilldown.",
        move || {
            let total = store.inbox_signals.with(|s| s.len());
            let unread = store
                .inbox_summary
                .with(|s| s.as_ref().map(|x| x.unread_signals).unwrap_or(0));
            format!("{unread} unread / {total} total signals")
        },
        "追平 L1 事件流",
        "Catch Up L1 Feed",
        move || store.start_l1_catchup(false),
    )
}
