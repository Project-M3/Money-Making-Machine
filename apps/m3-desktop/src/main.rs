mod ui;

use anyhow::Result;
use gpui::{
    App, Application, Bounds, KeyBinding, Menu, MenuItem, OsAction, SharedString, SystemMenuType,
    TitlebarOptions, WindowBackgroundAppearance, WindowBounds, WindowOptions, actions, point,
    prelude::*, px, size,
};
use m3_core::app_config;
use m3_core::i18n::UiLang;
use m3_core::model::Bucket;
use m3_core::store::{self, WorkspacePaths};
use m3_core::update::{self, ReleaseChannel};
use std::{borrow::Cow, time::Duration};
use ui::{FontRoles, MungerWorkbench, NavView};

const M3_WEBSITE_URL: &str = "https://m3.thinksmart.systems";
const M3_GITHUB_URL: &str = "https://github.com/Project-M3/Money-Making-Machine";
const M3_RELEASES_URL: &str = "https://github.com/Project-M3/Money-Making-Machine/releases";

actions!(
    m3,
    [
        AboutM3,
        OpenSettings,
        CheckUpdate,
        ChannelStable,
        ChannelBeta,
        SetLangZh,
        SetLangEn,
        HideApp,
        HideOthers,
        ShowAll,
        Quit,
        OpenPalette,
        SyncData,
        CatchupEvents,
        SyncUniverse,
        SyncThemes,
        RevealWorkspace,
        ReloadWorkspace,
        CloseWindow,
        Undo,
        Redo,
        Cut,
        Copy,
        Paste,
        SelectAll,
        ViewInbox,
        ViewUniverse,
        ViewIndustry,
        ViewBaskets,
        ViewDossier,
        ViewAgent,
        ViewJournal,
        BackToBaskets,
        ToggleBasis,
        ToggleBasketsView,
        ToggleFullScreen,
        RunDeepResearch,
        ShowBudgetStatus,
        FilterBasketAll,
        FilterBasketCore,
        FilterBasketWatch,
        FilterBasketOut,
        FilterBasketTooHard,
        MinimizeWindow,
        ZoomWindow,
        OpenWebsite,
        OpenGitHub,
        OpenReleases
    ]
);

pub(crate) fn build_app_menus(lang: UiLang) -> Vec<Menu> {
    vec![
        Menu {
            name: SharedString::from("M3"),
            items: vec![
                MenuItem::action(
                    format!(
                        "{} (v{})",
                        lang.t("关于 Project M3", "About Project M3"),
                        update::current_binary_version()
                    ),
                    AboutM3,
                ),
                MenuItem::separator(),
                MenuItem::action(lang.t("设置…", "Settings…"), OpenSettings),
                MenuItem::action(lang.t("检查更新…", "Check for Updates…"), CheckUpdate),
                MenuItem::submenu(Menu {
                    name: SharedString::from(lang.t("更新通道", "Release Channel")),
                    items: vec![
                        MenuItem::action(
                            lang.t("稳定版 (stable)", "Stable (stable)"),
                            ChannelStable,
                        ),
                        MenuItem::action(lang.t("测试版 (beta)", "Beta (beta)"), ChannelBeta),
                    ],
                }),
                MenuItem::submenu(Menu {
                    name: SharedString::from(lang.t("语言 / Language", "Language / 语言")),
                    items: vec![
                        MenuItem::action("中文", SetLangZh),
                        MenuItem::action("English", SetLangEn),
                    ],
                }),
                MenuItem::separator(),
                MenuItem::os_submenu(lang.t("服务", "Services"), SystemMenuType::Services),
                MenuItem::separator(),
                MenuItem::action(lang.t("隐藏 M3", "Hide M3"), HideApp),
                MenuItem::action(lang.t("隐藏其他", "Hide Others"), HideOthers),
                MenuItem::action(lang.t("全部显示", "Show All"), ShowAll),
                MenuItem::separator(),
                MenuItem::action(lang.t("退出 M3", "Quit M3"), Quit),
            ],
        },
        Menu {
            name: SharedString::from(lang.t("文件", "File")),
            items: vec![
                MenuItem::action(lang.t("命令面板…", "Command Palette…"), OpenPalette),
                MenuItem::separator(),
                MenuItem::action(
                    lang.t("同步核心池财报与价格", "Sync Circle Financials & Prices"),
                    SyncData,
                ),
                MenuItem::action(
                    lang.t("追平 L1 事件流", "Catch Up L1 Events"),
                    CatchupEvents,
                ),
                MenuItem::action(
                    lang.t("同步全美股 L0 湖仓", "Sync US L0 Lakehouse"),
                    SyncUniverse,
                ),
                MenuItem::action(lang.t("同步主题图谱", "Sync Theme Graph"), SyncThemes),
                MenuItem::separator(),
                MenuItem::action(
                    lang.t("在 Finder 中打开工作区", "Reveal Workspace in Finder"),
                    RevealWorkspace,
                ),
                MenuItem::action(
                    lang.t("重新加载本地工作区", "Reload Workspace from Disk"),
                    ReloadWorkspace,
                ),
                MenuItem::separator(),
                MenuItem::action(lang.t("关闭窗口", "Close Window"), CloseWindow),
            ],
        },
        Menu {
            name: SharedString::from(lang.t("编辑", "Edit")),
            items: vec![
                MenuItem::os_action(lang.t("撤销", "Undo"), Undo, OsAction::Undo),
                MenuItem::os_action(lang.t("重做", "Redo"), Redo, OsAction::Redo),
                MenuItem::separator(),
                MenuItem::os_action(lang.t("剪切", "Cut"), Cut, OsAction::Cut),
                MenuItem::os_action(lang.t("拷贝", "Copy"), Copy, OsAction::Copy),
                MenuItem::os_action(lang.t("粘贴", "Paste"), Paste, OsAction::Paste),
                MenuItem::os_action(lang.t("全选", "Select All"), SelectAll, OsAction::SelectAll),
            ],
        },
        Menu {
            name: SharedString::from(lang.t("视图", "View")),
            items: vec![
                MenuItem::action(lang.t("简报收件箱", "Briefing Inbox"), ViewInbox),
                MenuItem::action(lang.t("全市场雷达", "Universe Radar"), ViewUniverse),
                MenuItem::action(lang.t("行业研究", "Industry Research"), ViewIndustry),
                MenuItem::action(lang.t("能力圈总览", "Circle Overview"), ViewBaskets),
                MenuItem::action(lang.t("决策账本", "Decision Journal"), ViewJournal),
                MenuItem::separator(),
                MenuItem::action(lang.t("财报与活研报", "Financials & Memo"), ViewDossier),
                MenuItem::action("Research Desk · Alan", ViewAgent),
                MenuItem::action(
                    lang.t("返回能力圈总览", "Back to Circle Overview"),
                    BackToBaskets,
                ),
                MenuItem::separator(),
                MenuItem::action(
                    lang.t("切换 TTM / 年报口径", "Toggle TTM / FY Basis"),
                    ToggleBasis,
                ),
                MenuItem::action(
                    lang.t("切换列表与看板视图", "Toggle List / Board View"),
                    ToggleBasketsView,
                ),
                MenuItem::separator(),
                MenuItem::action(
                    lang.t("进入或退出全屏", "Toggle Full Screen"),
                    ToggleFullScreen,
                ),
            ],
        },
        Menu {
            name: SharedString::from(lang.t("研究", "Research")),
            items: vec![
                MenuItem::action(
                    lang.t(
                        "对当前标的运行 Deep Research（4 视角）",
                        "Run Deep Research on Selected (4 Perspectives)",
                    ),
                    RunDeepResearch,
                ),
                MenuItem::action(
                    lang.t("查看本月 AI 预算状态", "Show Monthly AI Budget Status"),
                    ShowBudgetStatus,
                ),
                MenuItem::separator(),
                MenuItem::submenu(Menu {
                    name: SharedString::from(lang.t("按能力圈筛选", "Filter by Circle Tier")),
                    items: vec![
                        MenuItem::action(lang.t("全部能力圈", "All Tiers"), FilterBasketAll),
                        MenuItem::action(
                            lang.t("CORE · 核心重仓候补", "CORE · Strike Candidates"),
                            FilterBasketCore,
                        ),
                        MenuItem::action(
                            lang.t("WATCH · 持续跟踪", "WATCH · Tracking"),
                            FilterBasketWatch,
                        ),
                        MenuItem::action(
                            lang.t("OUT · 超出能力圈", "OUT · Outside Circle"),
                            FilterBasketOut,
                        ),
                        MenuItem::action(
                            lang.t("TOO HARD · 暂不碰", "TOO HARD · Archived"),
                            FilterBasketTooHard,
                        ),
                    ],
                }),
            ],
        },
        Menu {
            name: SharedString::from(lang.t("窗口", "Window")),
            items: vec![
                MenuItem::action(lang.t("最小化", "Minimize"), MinimizeWindow),
                MenuItem::action(lang.t("缩放", "Zoom"), ZoomWindow),
                MenuItem::separator(),
                MenuItem::action(
                    lang.t("进入或退出全屏", "Toggle Full Screen"),
                    ToggleFullScreen,
                ),
            ],
        },
        Menu {
            name: SharedString::from(lang.t("帮助", "Help")),
            items: vec![
                MenuItem::action(lang.t("Project M3 官网", "Project M3 Website"), OpenWebsite),
                MenuItem::action(
                    lang.t("GitHub 开源仓库", "GitHub Open Source Repository"),
                    OpenGitHub,
                ),
                MenuItem::action(
                    lang.t("版本发布记录 (Releases)", "Release Notes (Releases)"),
                    OpenReleases,
                ),
            ],
        },
    ]
}

fn bind_keybindings(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("cmd-,", OpenSettings, None),
        KeyBinding::new("cmd-q", Quit, None),
        KeyBinding::new("cmd-h", HideApp, None),
        KeyBinding::new("cmd-alt-h", HideOthers, None),
        KeyBinding::new("cmd-w", CloseWindow, None),
        KeyBinding::new("cmd-m", MinimizeWindow, None),
        KeyBinding::new("ctrl-cmd-f", ToggleFullScreen, None),
        KeyBinding::new("cmd-k", OpenPalette, None),
        KeyBinding::new("cmd-r", SyncData, None),
        KeyBinding::new("cmd-shift-r", CatchupEvents, None),
        KeyBinding::new("cmd-shift-o", RevealWorkspace, None),
        KeyBinding::new("cmd-z", Undo, None),
        KeyBinding::new("cmd-shift-z", Redo, None),
        KeyBinding::new("cmd-x", Cut, None),
        KeyBinding::new("cmd-c", Copy, None),
        KeyBinding::new("cmd-v", Paste, None),
        KeyBinding::new("cmd-a", SelectAll, None),
        KeyBinding::new("cmd-1", ViewInbox, None),
        KeyBinding::new("cmd-2", ViewUniverse, None),
        KeyBinding::new("cmd-3", ViewIndustry, None),
        KeyBinding::new("cmd-4", ViewBaskets, None),
        KeyBinding::new("cmd-5", ViewJournal, None),
        KeyBinding::new("cmd-6", ViewDossier, None),
        KeyBinding::new("cmd-7", ViewAgent, None),
        KeyBinding::new("cmd-[", BackToBaskets, None),
        KeyBinding::new("cmd-t", ToggleBasis, None),
        KeyBinding::new("cmd-shift-v", ToggleBasketsView, None),
        KeyBinding::new("cmd-shift-d", RunDeepResearch, None),
        KeyBinding::new("cmd-b", ShowBudgetStatus, None),
    ]);
}

fn register_global_actions(cx: &mut App) {
    cx.on_action(|_: &Quit, cx| cx.quit());
    cx.on_action(|_: &HideApp, cx| cx.hide());
    cx.on_action(|_: &HideOthers, cx| cx.hide_other_apps());
    cx.on_action(|_: &ShowAll, cx| cx.unhide_other_apps());
    cx.on_action(|_: &OpenWebsite, cx| cx.open_url(M3_WEBSITE_URL));
    cx.on_action(|_: &OpenGitHub, cx| cx.open_url(M3_GITHUB_URL));
    cx.on_action(|_: &OpenReleases, cx| cx.open_url(M3_RELEASES_URL));
}

fn register_window_action<A: gpui::Action>(
    cx: &mut App,
    window: gpui::WindowHandle<MungerWorkbench>,
    handler: impl Fn(&mut MungerWorkbench, &mut gpui::Window, &mut gpui::Context<MungerWorkbench>)
    + std::marker::Copy
    + 'static,
) {
    cx.on_action(move |_: &A, cx| {
        // GPUI leases the active Window out of `cx.windows` while running
        // `dispatch_action_on_node`. Deferring onto the end of the current
        // effect cycle lets the Window return before `window.update` runs.
        cx.defer(move |cx| {
            let _ = window.update(cx, |v, win, cx| handler(v, win, cx));
        });
    });
}

fn register_window_actions(cx: &mut App, window: gpui::WindowHandle<MungerWorkbench>) {
    register_window_action::<AboutM3>(cx, window, |v, _, cx| v.show_about_notice(cx));
    register_window_action::<OpenSettings>(cx, window, |v, _, cx| {
        v.switch_view(NavView::Settings);
        cx.notify();
    });
    register_window_action::<SetLangZh>(cx, window, |v, _, cx| v.set_language(UiLang::Zh, cx));
    register_window_action::<SetLangEn>(cx, window, |v, _, cx| v.set_language(UiLang::En, cx));
    register_window_action::<OpenPalette>(cx, window, |v, win, cx| {
        v.toggle_palette(win);
        cx.notify();
    });
    register_window_action::<ViewInbox>(cx, window, |v, _, cx| {
        v.switch_view(NavView::Inbox);
        cx.notify();
    });
    register_window_action::<ViewUniverse>(cx, window, |v, _, cx| {
        v.switch_view(NavView::Universe);
        cx.notify();
    });
    register_window_action::<ViewIndustry>(cx, window, |v, _, cx| {
        v.switch_view(NavView::Industry);
        cx.notify();
    });
    register_window_action::<ViewBaskets>(cx, window, |v, _, cx| {
        v.focus_basket(None);
        cx.notify();
    });
    register_window_action::<ViewDossier>(cx, window, |v, _, cx| {
        v.switch_view(NavView::Dossier);
        cx.notify();
    });
    register_window_action::<ViewAgent>(cx, window, |v, _, cx| {
        v.switch_view(NavView::Agent);
        cx.notify();
    });
    register_window_action::<ViewJournal>(cx, window, |v, _, cx| {
        v.switch_view(NavView::Journal);
        cx.notify();
    });
    register_window_action::<BackToBaskets>(cx, window, |v, _, cx| {
        v.switch_view(NavView::Baskets);
        cx.notify();
    });
    register_window_action::<SyncData>(cx, window, |v, _, cx| v.start_sync(cx));
    register_window_action::<CatchupEvents>(cx, window, |v, _, cx| v.trigger_l1_catchup(cx));
    register_window_action::<SyncUniverse>(cx, window, |v, _, cx| v.trigger_universe_sync(cx));
    register_window_action::<SyncThemes>(cx, window, |v, _, cx| v.trigger_themes_sync(cx));
    register_window_action::<RevealWorkspace>(cx, window, |v, _, cx| {
        let root = v.workspace_root().to_path_buf();
        cx.reveal_path(&root);
    });
    register_window_action::<ReloadWorkspace>(cx, window, |v, _, cx| {
        v.reload_from_disk();
        cx.notify();
    });
    register_window_action::<CheckUpdate>(cx, window, |v, _, cx| v.start_update_check(true, cx));
    register_window_action::<ChannelStable>(cx, window, |v, _, cx| {
        v.switch_update_channel(ReleaseChannel::Stable, cx);
    });
    register_window_action::<ChannelBeta>(cx, window, |v, _, cx| {
        v.switch_update_channel(ReleaseChannel::Beta, cx);
    });
    register_window_action::<RunDeepResearch>(cx, window, |v, _, cx| {
        v.run_deep_research_selected(cx)
    });
    register_window_action::<ShowBudgetStatus>(cx, window, |v, _, cx| {
        v.show_budget_status_notice(cx)
    });
    register_window_action::<ToggleBasis>(cx, window, |v, _, cx| v.toggle_basis(cx));
    register_window_action::<ToggleBasketsView>(cx, window, |v, _, cx| v.toggle_baskets_view(cx));
    register_window_action::<FilterBasketAll>(cx, window, |v, _, cx| {
        v.focus_basket(None);
        cx.notify();
    });
    register_window_action::<FilterBasketCore>(cx, window, |v, _, cx| {
        v.focus_basket(Some(Bucket::Core));
        cx.notify();
    });
    register_window_action::<FilterBasketWatch>(cx, window, |v, _, cx| {
        v.focus_basket(Some(Bucket::Watch));
        cx.notify();
    });
    register_window_action::<FilterBasketOut>(cx, window, |v, _, cx| {
        v.focus_basket(Some(Bucket::Out));
        cx.notify();
    });
    register_window_action::<FilterBasketTooHard>(cx, window, |v, _, cx| {
        v.focus_basket(Some(Bucket::TooHard));
        cx.notify();
    });
    register_window_action::<Copy>(cx, window, |v, _, cx| v.handle_menu_copy(cx));
    register_window_action::<Paste>(cx, window, |v, _, cx| v.handle_menu_paste(cx));
    register_window_action::<Cut>(cx, window, |v, _, cx| v.handle_menu_cut(cx));
    register_window_action::<MinimizeWindow>(cx, window, |_, win, _| win.minimize_window());
    register_window_action::<ZoomWindow>(cx, window, |_, win, _| win.zoom_window());
    register_window_action::<ToggleFullScreen>(cx, window, |_, win, _| win.toggle_fullscreen());
    register_window_action::<CloseWindow>(cx, window, |_, win, _| win.remove_window());
}

fn resolve_font_roles(cx: &App) -> FontRoles {
    const SF_MONO_BUNDLE_PATHS: [&str; 4] = [
        "/System/Applications/Utilities/Terminal.app/Contents/Resources/Fonts/SF-Mono-Regular.otf",
        "/System/Applications/Utilities/Terminal.app/Contents/Resources/Fonts/SF-Mono-Medium.otf",
        "/System/Applications/Utilities/Terminal.app/Contents/Resources/Fonts/SF-Mono-Semibold.otf",
        "/System/Applications/Utilities/Terminal.app/Contents/Resources/Fonts/SF-Mono-Bold.otf",
    ];
    let loaded: Vec<Cow<'static, [u8]>> = SF_MONO_BUNDLE_PATHS
        .iter()
        .filter_map(|p| std::fs::read(p).ok().map(Cow::Owned))
        .collect();
    if !loaded.is_empty() {
        let _ = cx.text_system().add_fonts(loaded);
    }

    let available = cx.text_system().all_font_names();
    let has_family = |name: &str| available.iter().any(|n| n == name);

    let mono_num: SharedString = if has_family("SF Mono") {
        "SF Mono".into()
    } else {
        "Menlo".into()
    };
    let mono_term: SharedString = if has_family("Maple Mono NF CN") {
        "Maple Mono NF CN".into()
    } else {
        mono_num.clone()
    };

    FontRoles {
        ui: ".SystemUIFont".into(),
        mono_num,
        mono_term,
    }
}

struct DiagnosticLog;
impl log::Log for DiagnosticLog {
    fn enabled(&self, metadata: &log::Metadata) -> bool {
        metadata.level() <= log::Level::Warn
    }
    fn log(&self, record: &log::Record) {
        if self.enabled(record.metadata()) {
            eprintln!("{}: {}", record.level(), record.args());
        }
    }
    fn flush(&self) {}
}

fn main() -> Result<()> {
    static LOGGER: DiagnosticLog = DiagnosticLog;
    let _ = log::set_logger(&LOGGER);
    log::set_max_level(log::LevelFilter::Warn);

    let args: Vec<String> = std::env::args().skip(1).collect();
    if !args.is_empty() {
        let code = m3_cli::run_cli_args(&args)?;
        std::process::exit(code);
    }

    let repo_root = store::resolve_repo_root()?;
    let paths = WorkspacePaths::from_root(repo_root);

    Application::new().run(move |cx: &mut App| {
        let fonts = resolve_font_roles(cx);
        let initial_lang = app_config::read_ui_language();

        register_global_actions(cx);
        bind_keybindings(cx);
        cx.set_menus(build_app_menus(initial_lang));
        cx.on_window_closed(|cx| {
            if cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();

        let bounds = Bounds::centered(None, size(px(1380.), px(880.)), cx);
        let result = cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                window_background: WindowBackgroundAppearance::Opaque,
                titlebar: Some(TitlebarOptions {
                    title: Some("M3".into()),
                    appears_transparent: true,
                    traffic_light_position: Some(point(px(16.), px(13.))),
                }),
                window_min_size: Some(size(px(1140.), px(720.))),
                ..Default::default()
            },
            move |window, cx| {
                let fonts = fonts.clone();
                let view = cx.new(|cx| {
                    cx.spawn(async move |this, cx| {
                        loop {
                            let state = this.update(cx, |this: &mut MungerWorkbench, cx| {
                                let sync_dirty = this.poll_background_sync();
                                let alan_dirty = this.poll_alan_events();
                                if sync_dirty || alan_dirty {
                                    cx.notify();
                                }
                                this.is_alan_streaming()
                            });
                            let Ok(streaming) = state else {
                                break;
                            };
                            let interval_ms = if streaming { 60 } else { 250 };
                            cx.background_executor()
                                .timer(Duration::from_millis(interval_ms))
                                .await;
                        }
                    })
                    .detach();
                    MungerWorkbench::new(paths, fonts, cx)
                });
                let fh = view.read(cx).focus_handle().clone();
                window.focus(&fh);
                view
            },
        );

        match result {
            Ok(window) => register_window_actions(cx, window),
            Err(error) => {
                eprintln!("无法打开 M3：{error:#}");
                cx.quit();
            }
        }
        cx.activate(true);
    });

    Ok(())
}

#[cfg(test)]
mod menu_tests {
    use super::build_app_menus;
    use m3_core::i18n::UiLang;

    #[test]
    fn builds_full_pro_macos_menu_bar() {
        let zh_menus = build_app_menus(UiLang::Zh);
        let zh_names: Vec<&str> = zh_menus.iter().map(|m| m.name.as_ref()).collect();
        assert_eq!(
            zh_names,
            vec!["M3", "文件", "编辑", "视图", "研究", "窗口", "帮助"]
        );
        for menu in &zh_menus {
            assert!(
                !menu.items.is_empty(),
                "menu `{}` should not be empty",
                menu.name
            );
        }

        let en_menus = build_app_menus(UiLang::En);
        let en_names: Vec<&str> = en_menus.iter().map(|m| m.name.as_ref()).collect();
        assert_eq!(
            en_names,
            vec!["M3", "File", "Edit", "View", "Research", "Window", "Help"]
        );
        for menu in &en_menus {
            assert!(
                !menu.items.is_empty(),
                "menu `{}` should not be empty",
                menu.name
            );
        }
    }
}
