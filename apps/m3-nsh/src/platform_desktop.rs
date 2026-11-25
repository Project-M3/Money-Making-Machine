//! Desktop chrome, bilingual macOS application menu bar, context menus,
//! clipboard, file dialogs, and app restart helpers.
//!
//! Isolated here so `stores/*` and `views/*` remain iOS-ready via `os()` branching.

use crate::stores::{AppStore, CompanyViewTab, WorkspaceTab};
use m3_core::i18n::UiLang;
use m3_core::model::Bucket;
use m3_core::update::{self, ReleaseChannel};
use nsh_core::desktop::{MenuRole, TitleBarOptions, TitleBarStyle, WindowAttributes};
use nsh_core::element::desktop::{
    MenuItem, popup_menu_at, restart, set_application_menu, title_bar,
};
use nsh_core::file_dialog::OpenDialog;
use nsh_core::platform::Os;
use nsh_core::prelude::*;
use std::path::Path;

pub const M3_WEBSITE_URL: &str = "https://m3.thinksmart.systems";
pub const M3_GITHUB_URL: &str = "https://github.com/Project-M3/Money-Making-Machine";
pub const M3_RELEASES_URL: &str = "https://github.com/Project-M3/Money-Making-Machine/releases";

/// Builds the main desktop window's attributes (`TitleBarStyle::Hidden`, 38pt bar, traffic
/// light offsets, and 1140x720 minimum size) applied atomically when NSH 0.7.0 opens `window()`.
pub fn main_window_attributes() -> WindowAttributes {
    let mut tb = TitleBarOptions::default();
    tb.style = TitleBarStyle::Hidden;
    tb.height = Some(px(38.0));
    tb.controls_offset = Some(point(px(16.0), px(13.0)));

    let mut attrs = WindowAttributes::default();
    attrs.title_bar = Some(tb);
    attrs.min_size = Some(Some(size(px(1140.0), px(720.0))));
    attrs
}

/// Returns the reactive left inset (traffic light reservation) for the sidebar header.
pub fn titlebar_left_inset() -> f32 {
    if os() == Os::MacOs {
        let tb = title_bar();
        if tb.left.0 > 0.0 {
            tb.left.0 + 16.0
        } else {
            14.0
        }
    } else {
        14.0
    }
}

/// Returns the reactive titlebar height (38pt on desktop, 0 in fullscreen/mobile).
pub fn titlebar_height() -> f32 {
    if os().is_desktop() {
        let tb = title_bar();
        if tb.top.0 > 0.0 { tb.top.0 } else { 38.0 }
    } else {
        0.0
    }
}

/// Installs a reactive effect that updates the macOS application menu bar whenever
/// `lang` or `update_channel` changes.
pub fn bind_live_application_menu(store: AppStore) {
    if !os().is_desktop() {
        return;
    }
    effect(move || {
        let lang = store.lang.get();
        let channel = store.update_channel.get();
        set_application_menu(Some(build_app_menus(lang, channel, store)));
    });
}

/// Builds the full 7-menu macOS application bar.
pub fn build_app_menus(lang: UiLang, channel: ReleaseChannel, store: AppStore) -> Vec<MenuItem> {
    vec![
        MenuItem::submenu(
            "M3",
            [
                MenuItem::new(format!(
                    "{} (v{})",
                    lang.t("关于 Project M3", "About Project M3"),
                    update::current_binary_version()
                ))
                .on_select(move || store.show_about_notice()),
                MenuItem::separator(),
                MenuItem::new(lang.t("设置…", "Settings…"))
                    .shortcut("mod+,")
                    .on_select(move || store.open_settings()),
                MenuItem::new(lang.t("检查更新…", "Check for Updates…"))
                    .on_select(move || store.start_update_check(true)),
                MenuItem::submenu(
                    lang.t("更新通道", "Release Channel"),
                    [
                        MenuItem::new(lang.t("稳定版 (stable)", "Stable (stable)"))
                            .checked(channel == ReleaseChannel::Stable)
                            .on_select(move || store.switch_update_channel(ReleaseChannel::Stable)),
                        MenuItem::new(lang.t("测试版 (beta)", "Beta (beta)"))
                            .checked(channel == ReleaseChannel::Beta)
                            .on_select(move || store.switch_update_channel(ReleaseChannel::Beta)),
                    ],
                ),
                MenuItem::submenu(
                    lang.t("语言 / Language", "Language / 语言"),
                    [
                        MenuItem::new("中文")
                            .checked(lang == UiLang::Zh)
                            .on_select(move || store.set_language(UiLang::Zh)),
                        MenuItem::new("English")
                            .checked(lang == UiLang::En)
                            .on_select(move || store.set_language(UiLang::En)),
                    ],
                ),
                MenuItem::separator(),
                MenuItem::role(MenuRole::Hide).label(lang.t("隐藏 M3", "Hide M3")),
                MenuItem::role(MenuRole::HideOthers).label(lang.t("隐藏其他", "Hide Others")),
                MenuItem::role(MenuRole::ShowAll).label(lang.t("全部显示", "Show All")),
                MenuItem::separator(),
                MenuItem::role(MenuRole::Quit).label(lang.t("退出 M3", "Quit M3")),
            ],
        ),
        MenuItem::submenu(
            lang.t("文件", "File"),
            [
                MenuItem::new(lang.t("命令面板…", "Command Palette…"))
                    .shortcut("mod+k")
                    .on_select(move || store.toggle_palette()),
                MenuItem::separator(),
                MenuItem::new(lang.t("同步核心池财报与价格", "Sync Circle Financials & Prices"))
                    .shortcut("mod+r")
                    .on_select(move || store.start_sync()),
                MenuItem::new(lang.t("追平 L1 事件流", "Catch Up L1 Events"))
                    .shortcut("mod+shift+r")
                    .on_select(move || store.trigger_l1_catchup()),
                MenuItem::new(lang.t("同步全美股 L0 湖仓", "Sync US L0 Lakehouse"))
                    .on_select(move || store.trigger_universe_sync()),
                MenuItem::new(lang.t("同步主题图谱", "Sync Theme Graph"))
                    .on_select(move || store.trigger_themes_sync()),
                MenuItem::separator(),
                MenuItem::new(lang.t("在 Finder 中打开工作区", "Reveal Workspace in Finder"))
                    .shortcut("mod+shift+o")
                    .on_select(move || {
                        let root = store.paths.get_untracked().repo_root;
                        reveal_workspace_dir(&root);
                    }),
                MenuItem::new(lang.t("重新加载本地工作区", "Reload Workspace from Disk"))
                    .on_select(move || store.reload_all()),
                MenuItem::separator(),
                MenuItem::role(MenuRole::Close).label(lang.t("关闭窗口", "Close Window")),
            ],
        ),
        MenuItem::submenu(
            lang.t("编辑", "Edit"),
            [
                MenuItem::role(MenuRole::Undo).label(lang.t("撤销", "Undo")),
                MenuItem::role(MenuRole::Redo).label(lang.t("重做", "Redo")),
                MenuItem::separator(),
                MenuItem::role(MenuRole::Cut).label(lang.t("剪切", "Cut")),
                MenuItem::role(MenuRole::Copy).label(lang.t("拷贝", "Copy")),
                MenuItem::role(MenuRole::Paste).label(lang.t("粘贴", "Paste")),
                MenuItem::role(MenuRole::SelectAll).label(lang.t("全选", "Select All")),
            ],
        ),
        MenuItem::submenu(
            lang.t("视图", "View"),
            [
                MenuItem::new(lang.t("简报收件箱", "Briefing Inbox"))
                    .shortcut("mod+1")
                    .on_select(move || store.open_tab(WorkspaceTab::Inbox)),
                MenuItem::new(lang.t("全市场雷达", "Universe Radar"))
                    .shortcut("mod+2")
                    .on_select(move || store.open_tab(WorkspaceTab::Universe)),
                MenuItem::new(lang.t("行业研究", "Industry Research"))
                    .shortcut("mod+3")
                    .on_select(move || store.open_tab(WorkspaceTab::Industry)),
                MenuItem::new(lang.t("能力圈总览", "Circle Overview"))
                    .shortcut("mod+4")
                    .on_select(move || store.focus_basket(None)),
                MenuItem::new(lang.t("决策账本", "Decision Journal"))
                    .shortcut("mod+5")
                    .on_select(move || store.open_tab(WorkspaceTab::Journal)),
                MenuItem::separator(),
                MenuItem::new(lang.t("财报与活研报", "Financials & Memo"))
                    .shortcut("mod+6")
                    .on_select(move || store.open_selected_company(CompanyViewTab::FinancialsMemo)),
                MenuItem::new("Research Desk · Alan")
                    .shortcut("mod+7")
                    .on_select(move || {
                        store.open_selected_company(CompanyViewTab::DeepResearchAlan);
                    }),
                MenuItem::new(lang.t("返回能力圈总览", "Back to Circle Overview"))
                    .shortcut("mod+[")
                    .on_select(move || store.open_tab(WorkspaceTab::Baskets)),
                MenuItem::separator(),
                MenuItem::new(lang.t("切换 TTM / 年报口径", "Toggle TTM / FY Basis"))
                    .shortcut("mod+t")
                    .on_select(move || store.toggle_basis()),
                MenuItem::new(lang.t("切换列表与看板视图", "Toggle List / Board View"))
                    .shortcut("mod+shift+v")
                    .on_select(move || store.toggle_baskets_view()),
                MenuItem::separator(),
                MenuItem::role(MenuRole::ToggleFullScreen)
                    .label(lang.t("进入或退出全屏", "Toggle Full Screen")),
            ],
        ),
        MenuItem::submenu(
            lang.t("研究", "Research"),
            [
                MenuItem::new(lang.t(
                    "对当前标的运行 Deep Research（4 视角）",
                    "Run Deep Research on Selected (4 Perspectives)",
                ))
                .shortcut("mod+shift+d")
                .on_select(move || store.run_deep_research_selected()),
                MenuItem::new(lang.t("查看本月 AI 预算状态", "Show Monthly AI Budget Status"))
                    .shortcut("mod+b")
                    .on_select(move || store.show_budget_status_notice()),
                MenuItem::separator(),
                MenuItem::submenu(
                    lang.t("按能力圈筛选", "Filter by Circle Tier"),
                    [
                        MenuItem::new(lang.t("全部能力圈", "All Tiers"))
                            .on_select(move || store.focus_basket(None)),
                        MenuItem::new(lang.t("CORE · 核心重仓候补", "CORE · Strike Candidates"))
                            .on_select(move || store.focus_basket(Some(Bucket::Core))),
                        MenuItem::new(lang.t("WATCH · 持续跟踪", "WATCH · Tracking"))
                            .on_select(move || store.focus_basket(Some(Bucket::Watch))),
                        MenuItem::new(lang.t("OUT · 超出能力圈", "OUT · Outside Circle"))
                            .on_select(move || store.focus_basket(Some(Bucket::Out))),
                        MenuItem::new(lang.t("TOO HARD · 暂不碰", "TOO HARD · Archived"))
                            .on_select(move || store.focus_basket(Some(Bucket::TooHard))),
                    ],
                ),
            ],
        ),
        MenuItem::submenu(
            lang.t("窗口", "Window"),
            [
                MenuItem::role(MenuRole::Minimize).label(lang.t("最小化", "Minimize")),
                MenuItem::role(MenuRole::Zoom).label(lang.t("缩放", "Zoom")),
                MenuItem::separator(),
                MenuItem::role(MenuRole::ToggleFullScreen)
                    .label(lang.t("进入或退出全屏", "Toggle Full Screen")),
            ],
        ),
        MenuItem::submenu(
            lang.t("帮助", "Help"),
            [
                MenuItem::new(lang.t("Project M3 官网", "Project M3 Website"))
                    .on_select(|| open_external_url(M3_WEBSITE_URL)),
                MenuItem::new(lang.t("GitHub 开源仓库", "GitHub Open Source Repository"))
                    .on_select(|| open_external_url(M3_GITHUB_URL)),
                MenuItem::new(lang.t("版本发布记录 (Releases)", "Release Notes (Releases)"))
                    .on_select(|| open_external_url(M3_RELEASES_URL)),
            ],
        ),
    ]
}

/// Copies `text` to the system clipboard and shows a toast notice.
pub fn copy_text_with_notice(store: AppStore, text: String, label: &str) {
    drop(set_clipboard(text.clone()));
    let l = store.lang.get_untracked();
    store.set_info_notice(format!("{} ({label}): {text}", l.t("已复制", "Copied")));
}

/// Reads the system clipboard asynchronously and delivers the text to `on_read`.
pub fn read_clipboard_async(on_read: impl FnOnce(String) + 'static) {
    spawn(async move {
        let s = clipboard_string().await;
        on_read(s);
    });
}

/// Prompts the user to select a workspace directory using the native OS dialog.
pub fn prompt_choose_workspace(store: AppStore) {
    if !os().is_desktop() {
        return;
    }
    let l = store.lang.get_untracked();
    let title = l
        .t(
            "选择或新建 M3 工作区目录",
            "Select or Create M3 Workspace Directory",
        )
        .to_owned();
    let button = l.t("选择工作区", "Select Workspace").to_owned();
    spawn(async move {
        if let Ok(Some(paths)) =
            pick_paths(OpenDialog::folders().title(title).button_label(button)).await
            && let Some(dir) = paths.into_iter().next()
        {
            store.switch_workspace_to(&dir);
        }
    });
}

/// Reveals a local path in Finder (desktop only).
pub fn reveal_workspace_dir(path: &Path) {
    if os().is_desktop() {
        drop(reveal_path(path.to_path_buf()));
    }
}

/// Opens a local file with the system default handler.
pub fn open_local_file(path: &Path) {
    drop(open_path(path.to_path_buf()));
}

/// Opens an external HTTP/HTTPS URL in the system browser.
pub fn open_external_url(url: &str) {
    drop(open_url(url.to_owned()));
}

/// Restarts the application after an update (using NSH's `restart()` or M3 bundle relaunch fallback).
pub fn restart_after_update(store: AppStore) {
    if !os().is_desktop() {
        return;
    }
    let env_app_dir = std::env::var("M3_APP_DIR").ok();
    let current_exe = std::env::current_exe().ok();
    if let Ok(bundle) = update::install::resolve_target_app_bundle(
        None,
        env_app_dir.as_deref(),
        current_exe.as_deref(),
    ) && bundle.exists()
        && update::install::relaunch_app_bundle(&bundle).is_ok()
    {
        nsh_core::element::desktop::quit();
        return;
    }
    let _ = store;
    restart();
}

/// Shows a context menu for a company row/card at `at`.
pub fn show_company_context_menu(store: AppStore, ticker: String, at: Point<Points>) {
    if !os().is_desktop() {
        return;
    }
    let l = store.lang.get_untracked();
    let t_dossier = ticker.clone();
    let t_alan = ticker.clone();
    let t_dr = ticker.clone();
    let t_cli = ticker.clone();
    let t_core = ticker.clone();
    let t_watch = ticker.clone();
    let t_out = ticker.clone();
    let t_hard = ticker;

    let items = vec![
        MenuItem::new(l.t("打开财报与活研报", "Open Financials & Memo"))
            .on_select(move || store.open_company(&t_dossier, CompanyViewTab::FinancialsMemo)),
        MenuItem::new("Open Research Desk · Alan")
            .on_select(move || store.open_company(&t_alan, CompanyViewTab::DeepResearchAlan)),
        MenuItem::new(l.t("运行 Deep Research（4 视角）", "Run Deep Research (4-Lens)")).on_select(
            move || {
                store.open_company(&t_dr, CompanyViewTab::DeepResearchAlan);
                store.start_deep_research(&t_dr, None, "context_menu");
            },
        ),
        MenuItem::separator(),
        MenuItem::submenu(
            l.t("调整能力圈分桶", "Move Circle Tier"),
            [
                MenuItem::new("CORE")
                    .on_select(move || store.move_company_bucket(&t_core, Bucket::Core)),
                MenuItem::new("WATCH")
                    .on_select(move || store.move_company_bucket(&t_watch, Bucket::Watch)),
                MenuItem::new("OUT")
                    .on_select(move || store.move_company_bucket(&t_out, Bucket::Out)),
                MenuItem::new("TOO HARD")
                    .on_select(move || store.move_company_bucket(&t_hard, Bucket::TooHard)),
            ],
        ),
        MenuItem::separator(),
        MenuItem::new(l.t("复制 `m3 report` CLI", "Copy `m3 report` CLI")).on_select(move || {
            copy_text_with_notice(
                store,
                format!("m3 report {t_cli}"),
                &format!("report {t_cli}"),
            );
        }),
    ];
    drop(popup_menu_at(items, at));
}
