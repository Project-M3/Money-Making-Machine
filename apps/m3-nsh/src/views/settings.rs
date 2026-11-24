//! Settings & Guardrails view (Phase 1 skeleton wired to `AppStore`).

use super::skeleton_placeholder;
use crate::platform_desktop::prompt_choose_workspace;
use crate::stores::AppStore;
use m3_core::i18n::UiLang;
use nsh_core::prelude::*;

pub fn settings_view(store: AppStore) -> impl IntoElement {
    skeleton_placeholder(
        store,
        "SETTINGS · GUARDRAILS",
        "设置 · 语言、工作区、更新与 AI 预算护栏",
        "Settings · Language, Workspace, Updates & Budget",
        "已接入中/英双语实时切换、工作区切换、Release 通道与 L2 月度预算护栏。",
        "Phase 1 store wired: live Zh/En toggle, workspace switcher, release channel & budget caps.",
        move || {
            let lang = store.lang.get();
            let ch = store.update_channel.get();
            let root = store.paths.with(|p| p.repo_root.display().to_string());
            let lang_code = match lang {
                UiLang::Zh => "zh",
                UiLang::En => "en",
            };
            format!("{lang_code} · {} · {root}", ch.as_str())
        },
        "切换工作区目录…",
        "Choose Workspace Folder…",
        move || prompt_choose_workspace(store),
    )
}

#[allow(dead_code)]
fn _lang_unused(_: UiLang) {}
