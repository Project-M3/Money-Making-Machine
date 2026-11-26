//! Desktop chrome, bilingual macOS application menu bar, context menus,
//! clipboard, file dialogs, and app restart helpers.
//!
//! Isolated here so `stores/*` and `views/*` remain iOS-ready via `os()` branching.

use crate::stores::{AppStore, CompanyViewTab, SignalTriageRecord, TriageVerdict, WorkspaceTab};
use crate::theme::{CORAL_FG, EMERALD_FG};
use m3_core::i18n::UiLang;
use m3_core::lake_l1::{SignalInboxRow, SignalStatus};
use m3_core::model::{Bucket, MungerReport, StrikeZoneStatus};
use m3_core::update::{self, ReleaseChannel};
use nsh_core::desktop::{
    Backdrop, MenuRole, TitleBarOptions, TitleBarStyle, TrayTitleRun, WindowAttributes,
};
use nsh_core::element::ImageSrc;
use nsh_core::element::desktop::{
    MenuItem, popup_menu_at, quit, restart, set_application_menu, set_tray, title_bar, tray,
};
use nsh_core::file_dialog::OpenDialog;
use nsh_core::platform::Os;
use nsh_core::prelude::*;
use std::collections::HashSet;
use std::path::Path;

pub const M3_WEBSITE_URL: &str = "https://m3.thinksmart.systems";
pub const M3_GITHUB_URL: &str = "https://github.com/Project-M3/Money-Making-Machine";
pub const M3_RELEASES_URL: &str = "https://github.com/Project-M3/Money-Making-Machine/releases";

/// Low-radius (`12pt`) behind-window Gaussian blur for the fog-free smoked-black glass shell.
pub const SMOKED_GLASS_BLUR_RADIUS: f32 = 12.0;

#[cfg(target_os = "macos")]
mod smoked_glass {
    use super::SMOKED_GLASS_BLUR_RADIUS;
    use nsh_platform_macos::objc2::{
        DefinedClass as _, MainThreadMarker, MainThreadOnly, define_class, msg_send,
        rc::Retained,
        runtime::{AnyObject, Bool},
    };
    use nsh_platform_macos::objc2_app_kit::{
        NSApplication, NSAutoresizingMaskOptions, NSNormalWindowLevel,
        NSUserInterfaceItemIdentification as _, NSView, NSVisualEffectBlendingMode,
        NSVisualEffectMaterial, NSVisualEffectState, NSVisualEffectView, NSWindowOrderingMode,
        NSWindowStyleMask,
    };
    use nsh_platform_macos::objc2_foundation::{
        NSArray, NSNumber, NSPoint, NSRect, NSString, ns_string,
    };
    use std::cell::Cell;

    const GLASS_VIEW_ID: &str = "m3.smoked_glass";

    define_class!(
        /// `NSVisualEffectView` stripped to a pure, crisp behind-window Gaussian blur (all AppKit
        /// milky tint/fill sublayers hidden) so our deep obsidian tint controls darkness & clarity.
        #[unsafe(super(NSVisualEffectView, NSView))]
        #[thread_kind = MainThreadOnly]
        #[ivars = Cell<f32>]
        #[name = "M3SmokedGlassView"]
        struct M3SmokedGlassView;

        impl M3SmokedGlassView {
            #[unsafe(method(layout))]
            fn layout(&self) {
                let _: () = unsafe { msg_send![super(self), layout] };
                self.strip();
            }

            #[unsafe(method(updateLayer))]
            fn update_layer(&self) {
                let _: () = unsafe { msg_send![super(self), updateLayer] };
                self.strip();
            }

            #[unsafe(method(viewDidChangeEffectiveAppearance))]
            fn appearance_changed(&self) {
                let _: () = unsafe { msg_send![super(self), viewDidChangeEffectiveAppearance] };
                self.strip();
            }

            #[unsafe(method(viewDidMoveToWindow))]
            fn moved_to_window(&self) {
                let _: () = unsafe { msg_send![super(self), viewDidMoveToWindow] };
                self.strip();
            }

            #[unsafe(method_id(hitTest:))]
            fn hit_test(&self, _point: NSPoint) -> Option<Retained<NSView>> {
                None
            }
        }
    );

    impl M3SmokedGlassView {
        fn new(frame: NSRect, radius: f32, mtm: MainThreadMarker) -> Retained<Self> {
            let this = Self::alloc(mtm).set_ivars(Cell::new(radius));
            let view: Retained<Self> = unsafe { msg_send![super(this), initWithFrame: frame] };
            view.setWantsLayer(true);
            view.setAutoresizingMask(
                NSAutoresizingMaskOptions::ViewWidthSizable
                    | NSAutoresizingMaskOptions::ViewHeightSizable,
            );
            view.setBlendingMode(NSVisualEffectBlendingMode::BehindWindow);
            view.setState(NSVisualEffectState::Active);
            view.setMaterial(NSVisualEffectMaterial::UnderWindowBackground);
            view.setIdentifier(Some(ns_string!(GLASS_VIEW_ID)));
            view
        }

        fn strip(&self) {
            let layer: Option<Retained<AnyObject>> = unsafe { msg_send![self, layer] };
            if let Some(layer) = layer {
                strip_material_layer(&layer, self.ivars().get());
            }
        }
    }

    fn strip_material_layer(layer: &AnyObject, radius: f32) {
        let sublayers: Option<Retained<NSArray<AnyObject>>> =
            unsafe { msg_send![layer, sublayers] };
        let Some(subs) = sublayers else {
            return;
        };
        let gaussian = ns_string!("gaussianBlur");
        for sub in subs.iter() {
            let is_backdrop = sub
                .class()
                .name()
                .to_str()
                .is_ok_and(|n| n.contains("Backdrop"));
            if is_backdrop {
                let filters: Option<Retained<NSArray<AnyObject>>> =
                    unsafe { msg_send![&*sub, filters] };
                let kept: Vec<Retained<AnyObject>> = filters
                    .map(|f| f.to_vec())
                    .unwrap_or_default()
                    .into_iter()
                    .filter(|f| {
                        let name: Option<Retained<NSString>> = unsafe { msg_send![&**f, name] };
                        name.is_some_and(|n| &*n == gaussian)
                    })
                    .collect();
                if kept.is_empty() {
                    continue;
                }
                unsafe {
                    let _: () = msg_send![&*sub, setFilters: &*NSArray::from_retained_slice(&kept)];
                    let value = NSNumber::new_f32(radius);
                    let _: () = msg_send![
                        &*sub,
                        setValue: &*value,
                        forKeyPath: ns_string!("filters.gaussianBlur.inputRadius")
                    ];
                }
            } else {
                let child_subs: Option<Retained<NSArray<AnyObject>>> =
                    unsafe { msg_send![&*sub, sublayers] };
                if child_subs.is_some_and(|l| l.count() > 0) {
                    strip_material_layer(&sub, radius);
                } else {
                    unsafe {
                        let _: () = msg_send![&*sub, setHidden: Bool::YES];
                    }
                }
            }
        }
    }

    pub fn install() {
        let Some(mtm) = MainThreadMarker::new() else {
            return;
        };
        let app = NSApplication::sharedApplication(mtm);
        for win in app.windows().iter() {
            let mask = win.styleMask();
            if win.level() != NSNormalWindowLevel
                || !mask.contains(NSWindowStyleMask::Titled)
                || mask.contains(NSWindowStyleMask::NonactivatingPanel)
            {
                continue;
            }
            let Some(content_view) = win.contentView() else {
                continue;
            };
            let already = content_view.subviews().iter().any(|v| {
                v.identifier()
                    .is_some_and(|id| &*id == ns_string!(GLASS_VIEW_ID))
            });
            if already {
                continue;
            }
            let glass =
                M3SmokedGlassView::new(content_view.bounds(), SMOKED_GLASS_BLUR_RADIUS, mtm);
            content_view.addSubview_positioned_relativeTo(
                &glass,
                NSWindowOrderingMode::Below,
                None,
            );
            glass.strip();
        }
    }
}

/// Installs the fog-free, low-blur smoked-black behind-window glass layer on desktop macOS windows.
pub fn install_smoked_black_glass_backdrop() {
    #[cfg(target_os = "macos")]
    {
        drop(run_on_main_thread(smoked_glass::install));
    }
}

/// Builds the main desktop window's attributes (`TitleBarStyle::Hidden`, 44pt bar, traffic
/// light offsets, genuine transparent window backing for smoked black glass, and 1140x720 min size).
pub fn main_window_attributes() -> WindowAttributes {
    let mut tb = TitleBarOptions::default();
    tb.style = TitleBarStyle::Hidden;
    tb.height = Some(px(44.0));
    tb.controls_offset = Some(point(px(16.0), px(15.0)));

    let mut attrs = WindowAttributes::default();
    attrs.title_bar = Some(tb);
    attrs.transparent = Some(true);
    attrs.backdrop = Some(Backdrop::None);
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

/// Returns the reactive titlebar height (44pt on desktop, 0 in fullscreen/mobile).
pub fn titlebar_height() -> f32 {
    if os().is_desktop() {
        let tb = title_bar();
        if tb.top.0 > 0.0 { tb.top.0 } else { 44.0 }
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

/// Counts active Circle `WATCH + IN` `FalsificationAlert`s and Strike Zone companies.
pub fn count_circle_tray_badges(
    reports: &[MungerReport],
    inbox_signals: &[SignalInboxRow],
    signal_triages: &[SignalTriageRecord],
) -> (usize, usize) {
    let mut circle_tickers = HashSet::new();
    let mut strike_hits = 0usize;

    for rep in reports {
        if !matches!(rep.bucket, Bucket::Watch | Bucket::Core) {
            continue;
        }
        circle_tickers.insert(rep.ticker.trim().to_ascii_uppercase());
        let latest_price = rep.price.as_ref().map(|p| p.price);
        let strike_price = rep
            .ttm_valuation
            .as_ref()
            .or(rep.fy_valuation.as_ref())
            .and_then(|v| v.strike_price);
        let price_in_zone = matches!((latest_price, strike_price), (Some(p), Some(s)) if p <= s);
        if rep.status == StrikeZoneStatus::InStrikeZone || price_in_zone {
            strike_hits += 1;
        }
    }

    let unread_signal_ids: Option<HashSet<&str>> = (!inbox_signals.is_empty()).then(|| {
        inbox_signals
            .iter()
            .filter(|s| s.status == SignalStatus::Unread)
            .map(|s| s.signal_id.as_str())
            .collect()
    });

    let active_alerts = signal_triages
        .iter()
        .filter(|t| t.verdict == TriageVerdict::FalsificationAlert)
        .filter(|t| circle_tickers.contains(&t.ticker.trim().to_ascii_uppercase()))
        .filter(|t| match &unread_signal_ids {
            Some(unread) => unread.contains(t.signal_id.as_str()),
            None => true,
        })
        .count();

    (active_alerts, strike_hits)
}

/// Shows (`Regular`) or hides (`Accessory`) the macOS Dock icon while keeping the tray-resident app alive.
pub fn set_macos_dock_visible(visible: bool) {
    #[cfg(target_os = "macos")]
    {
        drop(run_on_main_thread(move || {
            use nsh_platform_macos::objc2::MainThreadMarker;
            use nsh_platform_macos::objc2_app_kit::{NSApplication, NSApplicationActivationPolicy};

            let Some(mtm) = MainThreadMarker::new() else {
                return;
            };
            let app = NSApplication::sharedApplication(mtm);
            let target = if visible {
                NSApplicationActivationPolicy::Regular
            } else {
                NSApplicationActivationPolicy::Accessory
            };
            if app.activationPolicy() != target {
                app.setActivationPolicy(target);
            }
            if visible {
                #[allow(deprecated)]
                app.activateIgnoringOtherApps(true);
            }
        }));
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = visible;
    }
}

/// Builds minimal Menu Bar status dots shown beside the monochrome M3 tray icon.
/// Returns empty when quiet so the tray displays only the crisp template icon.
pub fn tray_status_runs(alerts: usize, strike_hits: usize) -> Vec<TrayTitleRun> {
    let mut runs = Vec::new();
    if alerts > 0 {
        runs.push(TrayTitleRun::new("●").color(rgb(CORAL_FG)));
    }
    if strike_hits > 0 {
        if !runs.is_empty() {
            runs.push(TrayTitleRun::new(" "));
        }
        runs.push(TrayTitleRun::new("●").color(rgb(EMERALD_FG)));
    }
    runs
}

/// Installs a reactive macOS Menu Bar tray (`set_tray`) with a monochrome template M3 SVG icon
/// that stays quiet by default, hides the Dock icon on close-to-tray, and recreates the status
/// item cleanly across `Regular ↔ Accessory` activation policy transitions.
pub fn bind_live_system_tray(store: AppStore, main_open: Signal<bool>) {
    if !os().is_desktop() {
        return;
    }
    let tray_epoch = signal(0u64);
    let rebind_seq = signal(0u64);
    let last_open = signal(main_open.get_untracked());

    effect(move || {
        let open = main_open.get();
        set_macos_dock_visible(open);
        if open != last_open.get_untracked() {
            last_open.set(open);
            let seq = rebind_seq.get_untracked().wrapping_add(1);
            rebind_seq.set(seq);
            spawn(async move {
                timer(std::time::Duration::from_millis(80)).await;
                if rebind_seq.get_untracked() != seq {
                    return;
                }
                set_tray(None);
                timer(std::time::Duration::from_millis(40)).await;
                if rebind_seq.get_untracked() != seq {
                    return;
                }
                tray_epoch.update(|n| *n = n.wrapping_add(1));
            });
        }
    });

    effect(move || {
        let _epoch = tray_epoch.get();
        let lang = store.lang.get();
        let proactive_running = store.proactive_running.get();
        let syncing = store.sync_running.get()
            || store.universe_syncing.get()
            || store.inbox_catching_up.get();

        let (alerts, strike_hits) = store.reports.with(|reports| {
            store.inbox_signals.with(|signals| {
                store
                    .signal_triages
                    .with(|triages| count_circle_tray_badges(reports, signals, triages))
            })
        });

        let latest_headline = store
            .briefings
            .with(|list| list.first().map(|b| b.headline.clone()));

        let runs = tray_status_runs(alerts, strike_hits);

        let tooltip = if alerts > 0 || strike_hits > 0 {
            match lang {
                UiLang::Zh => format!("M3 · 证伪预警 {alerts} · 能力圈击球区 {strike_hits}"),
                UiLang::En => {
                    format!("M3 · {alerts} Falsification Alerts · {strike_hits} Strike Zone")
                }
            }
        } else {
            lang.t("Project M3 · 静默巡检中", "Project M3 · Calm Watch")
                .to_owned()
        };

        let status_summary = if proactive_running || syncing {
            lang.t("状态：正在巡检能力圈…", "Status: Catching up Circle…")
                .to_owned()
        } else if alerts > 0 || strike_hits > 0 {
            match lang {
                UiLang::Zh => format!("预警 {alerts} 条 · 击球区 {strike_hits} 家"),
                UiLang::En => format!("{alerts} alerts · {strike_hits} in Strike Zone"),
            }
        } else if let Some(headline) = latest_headline {
            let clipped: String = headline.chars().take(42).collect();
            if headline.chars().count() > 42 {
                format!("{clipped}…")
            } else {
                clipped
            }
        } else {
            lang.t(
                "静默巡检中 · 暂无待处理异动",
                "Quiet · No pending Circle alerts",
            )
            .to_owned()
        };

        set_tray(
            tray()
                .icon(ImageSrc::svg_markup(crate::icons::ICON_TRAY_M3))
                .tooltip(tooltip)
                .title(runs)
                .menu([
                    MenuItem::new(lang.t("打开 M3", "Open M3")).on_select(move || {
                        set_macos_dock_visible(true);
                        main_open.set(true);
                    }),
                    MenuItem::new(status_summary).disabled(true),
                    MenuItem::new(lang.t("立即巡检能力圈", "Catch Up Now"))
                        .disabled(proactive_running)
                        .on_select(move || store.run_proactive_catchup()),
                    MenuItem::separator(),
                    MenuItem::new(lang.t("退出 M3", "Quit M3")).on_select(quit),
                ]),
        );
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
                        MenuItem::new(lang.t("WATCH · 持续跟踪", "WATCH · Tracking"))
                            .on_select(move || store.focus_basket(Some(Bucket::Watch))),
                        MenuItem::new(lang.t("CORE · 核心重仓候补", "CORE · Strike Candidates"))
                            .on_select(move || store.focus_basket(Some(Bucket::Core))),
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
    crate::stores::async_bridge::spawn_detached(async move {
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
    crate::stores::async_bridge::spawn_detached(async move {
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
    let t_watch = ticker.clone();
    let t_core = ticker.clone();
    let t_out = ticker.clone();
    let t_hard = ticker.clone();
    let t_remove = ticker;

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
                MenuItem::new("WATCH")
                    .on_select(move || store.request_bucket_change(&t_watch, Bucket::Watch)),
                MenuItem::new("CORE")
                    .on_select(move || store.request_bucket_change(&t_core, Bucket::Core)),
                MenuItem::new("OUT")
                    .on_select(move || store.request_bucket_change(&t_out, Bucket::Out)),
                MenuItem::new("TOO HARD")
                    .on_select(move || store.request_bucket_change(&t_hard, Bucket::TooHard)),
            ],
        ),
        MenuItem::new(l.t("移出能力圈", "Remove from Circle"))
            .on_select(move || store.remove_company_from_circle(&t_remove)),
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
