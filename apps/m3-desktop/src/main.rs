mod alan_interrogator;
mod app_config;
mod budget;
mod calculator;
mod cli;
mod edgar;
mod events;
mod lake;
mod lake_l1;
mod lake_l2;
mod latent;
mod market;
mod model;
mod research;
mod store;
mod ui;
mod universe;
mod update;

use anyhow::Result;
use gpui::{
    App, Application, Bounds, KeyBinding, Menu, MenuItem, OsAction, SharedString, SystemMenuType,
    TitlebarOptions, WindowBackgroundAppearance, WindowBounds, WindowOptions, actions, point,
    prelude::*, px, size,
};
use model::Bucket;
use std::{borrow::Cow, time::Duration};
use store::WorkspacePaths;
use ui::{FontRoles, MungerWorkbench, NavView};
use update::ReleaseChannel;

const M3_WEBSITE_URL: &str = "https://m3.thinksmart.systems";
const M3_GITHUB_URL: &str = "https://github.com/Project-M3/Money-Making-Machine";
const M3_RELEASES_URL: &str = "https://github.com/Project-M3/Money-Making-Machine/releases";

actions!(
    m3,
    [
        AboutM3,
        CheckUpdate,
        ChannelStable,
        ChannelBeta,
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

fn build_app_menus() -> Vec<Menu> {
    vec![
        Menu {
            name: SharedString::from("M3"),
            items: vec![
                MenuItem::action(
                    format!("关于 Project M3 (v{})", update::current_binary_version()),
                    AboutM3,
                ),
                MenuItem::separator(),
                MenuItem::action("检查更新…", CheckUpdate),
                MenuItem::submenu(Menu {
                    name: SharedString::from("更新通道"),
                    items: vec![
                        MenuItem::action("稳定版 (stable)", ChannelStable),
                        MenuItem::action("测试版 (beta)", ChannelBeta),
                    ],
                }),
                MenuItem::separator(),
                MenuItem::os_submenu("服务", SystemMenuType::Services),
                MenuItem::separator(),
                MenuItem::action("隐藏 M3", HideApp),
                MenuItem::action("隐藏其他", HideOthers),
                MenuItem::action("全部显示", ShowAll),
                MenuItem::separator(),
                MenuItem::action("退出 M3", Quit),
            ],
        },
        Menu {
            name: SharedString::from("文件"),
            items: vec![
                MenuItem::action("命令面板…", OpenPalette),
                MenuItem::separator(),
                MenuItem::action("同步核心池财报与价格", SyncData),
                MenuItem::action("追平 L1 事件流", CatchupEvents),
                MenuItem::action("同步全美股 L0 湖仓", SyncUniverse),
                MenuItem::action("同步主题图谱", SyncThemes),
                MenuItem::separator(),
                MenuItem::action("在 Finder 中打开工作区", RevealWorkspace),
                MenuItem::action("重新加载本地工作区", ReloadWorkspace),
                MenuItem::separator(),
                MenuItem::action("关闭窗口", CloseWindow),
            ],
        },
        Menu {
            name: SharedString::from("编辑"),
            items: vec![
                MenuItem::os_action("撤销", Undo, OsAction::Undo),
                MenuItem::os_action("重做", Redo, OsAction::Redo),
                MenuItem::separator(),
                MenuItem::os_action("剪切", Cut, OsAction::Cut),
                MenuItem::os_action("拷贝", Copy, OsAction::Copy),
                MenuItem::os_action("粘贴", Paste, OsAction::Paste),
                MenuItem::os_action("全选", SelectAll, OsAction::SelectAll),
            ],
        },
        Menu {
            name: SharedString::from("视图"),
            items: vec![
                MenuItem::action("简报收件箱", ViewInbox),
                MenuItem::action("全市场雷达", ViewUniverse),
                MenuItem::action("篮子总览", ViewBaskets),
                MenuItem::action("主题与账本", ViewJournal),
                MenuItem::separator(),
                MenuItem::action("财报与活研报", ViewDossier),
                MenuItem::action("Research Desk · Alan", ViewAgent),
                MenuItem::action("返回篮子总览", BackToBaskets),
                MenuItem::separator(),
                MenuItem::action("切换 TTM / 年报口径", ToggleBasis),
                MenuItem::action("切换列表与看板视图", ToggleBasketsView),
                MenuItem::separator(),
                MenuItem::action("进入或退出全屏", ToggleFullScreen),
            ],
        },
        Menu {
            name: SharedString::from("研究"),
            items: vec![
                MenuItem::action("对当前标的运行 Deep Research（4 视角）", RunDeepResearch),
                MenuItem::action("查看本月 AI 预算状态", ShowBudgetStatus),
                MenuItem::separator(),
                MenuItem::submenu(Menu {
                    name: SharedString::from("按芒格篮子筛选"),
                    items: vec![
                        MenuItem::action("全部篮子", FilterBasketAll),
                        MenuItem::action("CORE · 核心重仓候补", FilterBasketCore),
                        MenuItem::action("WATCH · 持续跟踪", FilterBasketWatch),
                        MenuItem::action("OUT · 超出能力圈", FilterBasketOut),
                        MenuItem::action("TOO HARD · 暂不碰", FilterBasketTooHard),
                    ],
                }),
            ],
        },
        Menu {
            name: SharedString::from("窗口"),
            items: vec![
                MenuItem::action("最小化", MinimizeWindow),
                MenuItem::action("缩放", ZoomWindow),
                MenuItem::separator(),
                MenuItem::action("进入或退出全屏", ToggleFullScreen),
            ],
        },
        Menu {
            name: SharedString::from("帮助"),
            items: vec![
                MenuItem::action("Project M3 官网", OpenWebsite),
                MenuItem::action("GitHub 开源仓库", OpenGitHub),
                MenuItem::action("版本发布记录 (Releases)", OpenReleases),
            ],
        },
    ]
}

fn bind_keybindings(cx: &mut App) {
    cx.bind_keys([
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
        KeyBinding::new("cmd-3", ViewBaskets, None),
        KeyBinding::new("cmd-4", ViewJournal, None),
        KeyBinding::new("cmd-5", ViewDossier, None),
        KeyBinding::new("cmd-6", ViewAgent, None),
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
    if args.as_slice() == ["-V"] {
        println!("m3 {}", update::current_binary_version());
        return Ok(());
    }
    if args.as_slice() == ["--version"] {
        println!("m3 {}", update::current_long_version());
        return Ok(());
    }
    if !args.is_empty()
        && let Some(standalone) = cli::run_standalone(&args)
    {
        let (value, code) = match standalone {
            Ok(val) => {
                let code = if val["ok"] == false { 1 } else { 0 };
                (val, code)
            }
            Err(err) => (
                serde_json::json!({
                    "schema_version": 1,
                    "ok": false,
                    "error": format!("{err:#}")
                }),
                1,
            ),
        };
        println!("{value}");
        std::process::exit(code);
    }

    let repo_root = store::resolve_repo_root()?;
    let paths = WorkspacePaths::from_root(repo_root);

    if !args.is_empty() {
        let (value, code) = match cli::run(&paths, &args) {
            Ok(val) => {
                let code = if val["ok"] == false { 1 } else { 0 };
                (val, code)
            }
            Err(err) => (
                serde_json::json!({
                    "schema_version": 1,
                    "ok": false,
                    "error": format!("{err:#}")
                }),
                1,
            ),
        };
        println!("{value}");
        std::process::exit(code);
    }

    Application::new().run(move |cx: &mut App| {
        let fonts = resolve_font_roles(cx);

        register_global_actions(cx);
        bind_keybindings(cx);
        cx.set_menus(build_app_menus());
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

    #[test]
    fn builds_full_pro_macos_menu_bar() {
        let menus = build_app_menus();
        let names: Vec<&str> = menus.iter().map(|m| m.name.as_ref()).collect();
        assert_eq!(
            names,
            vec!["M3", "文件", "编辑", "视图", "研究", "窗口", "帮助"]
        );
        for menu in &menus {
            assert!(
                !menu.items.is_empty(),
                "menu `{}` should not be empty",
                menu.name
            );
        }
    }
}
