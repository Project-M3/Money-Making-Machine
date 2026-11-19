mod calculator;
mod cli;
mod edgar;
mod market;
mod model;
mod store;
mod ui;

use anyhow::Result;
use gpui::{
    App, Application, Bounds, Menu, MenuItem, SharedString, TitlebarOptions,
    WindowBackgroundAppearance, WindowBounds, WindowOptions, actions, point, prelude::*, px,
    size,
};
use std::{borrow::Cow, time::Duration};
use store::WorkspacePaths;
use ui::{FontRoles, MungerWorkbench, NavView};

actions!(
    m3,
    [
        Quit,
        OpenPalette,
        ViewBaskets,
        ViewDossier,
        ViewAgent,
        ViewJournal,
        BackToBaskets,
        SyncData
    ]
);

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

    let repo_root = store::resolve_repo_root()?;
    let paths = WorkspacePaths::from_root(repo_root);

    let args: Vec<String> = std::env::args().skip(1).collect();
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

        cx.on_action(|_: &Quit, cx| cx.quit());
        cx.bind_keys([
            gpui::KeyBinding::new("cmd-q", Quit, None),
            gpui::KeyBinding::new("cmd-k", OpenPalette, None),
            gpui::KeyBinding::new("cmd-1", ViewBaskets, None),
            gpui::KeyBinding::new("cmd-2", ViewDossier, None),
            gpui::KeyBinding::new("cmd-3", ViewAgent, None),
            gpui::KeyBinding::new("cmd-4", ViewJournal, None),
            gpui::KeyBinding::new("cmd-[", BackToBaskets, None),
            gpui::KeyBinding::new("cmd-r", SyncData, None),
        ]);
        cx.set_menus(vec![Menu {
            name: SharedString::from("M3"),
            items: vec![
                MenuItem::action("命令面板", OpenPalette),
                MenuItem::separator(),
                MenuItem::action("篮子总览", ViewBaskets),
                MenuItem::action("财报与 K 线", ViewDossier),
                MenuItem::action("Alan 审讯台", ViewAgent),
                MenuItem::action("决策账本", ViewJournal),
                MenuItem::action("返回篮子总览", BackToBaskets),
                MenuItem::separator(),
                MenuItem::action("同步财报与价格", SyncData),
                MenuItem::separator(),
                MenuItem::action("退出 M3", Quit),
            ],
        }]);
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
                            let alive = this
                                .update(cx, |this: &mut MungerWorkbench, cx| {
                                    if this.poll_background_sync() {
                                        cx.notify();
                                    }
                                })
                                .is_ok();
                            if !alive {
                                break;
                            }
                            cx.background_executor()
                                .timer(Duration::from_millis(250))
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
            Ok(window) => {
                let wp = window;
                cx.on_action(move |_: &OpenPalette, cx| {
                    let _ = wp.update(cx, |v, window, cx| {
                        v.toggle_palette(window);
                        cx.notify();
                    });
                });
                let w1 = window;
                cx.on_action(move |_: &ViewBaskets, cx| {
                    let _ = w1.update(cx, |v, _, cx| {
                        v.focus_basket(None);
                        cx.notify();
                    });
                });
                let w2 = window;
                cx.on_action(move |_: &ViewDossier, cx| {
                    let _ = w2.update(cx, |v, _, cx| {
                        v.switch_view(NavView::Dossier);
                        cx.notify();
                    });
                });
                let w3 = window;
                cx.on_action(move |_: &ViewAgent, cx| {
                    let _ = w3.update(cx, |v, _, cx| {
                        v.switch_view(NavView::Agent);
                        cx.notify();
                    });
                });
                let w4 = window;
                cx.on_action(move |_: &ViewJournal, cx| {
                    let _ = w4.update(cx, |v, _, cx| {
                        v.switch_view(NavView::Journal);
                        cx.notify();
                    });
                });
                let wb = window;
                cx.on_action(move |_: &BackToBaskets, cx| {
                    let _ = wb.update(cx, |v, _, cx| {
                        v.switch_view(NavView::Baskets);
                        cx.notify();
                    });
                });
                let ws = window;
                cx.on_action(move |_: &SyncData, cx| {
                    let _ = ws.update(cx, |v, _, cx| {
                        v.start_sync(cx);
                    });
                });
            }
            Err(error) => {
                eprintln!("无法打开 M3：{error:#}");
                cx.quit();
            }
        }
        cx.activate(true);
    });

    Ok(())
}
