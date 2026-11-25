//! Bilingual `⌘K` Command Palette backed by native IME-safe `nsh_core::prelude::input()`.

use crate::components::{kbd, styled_input};
use crate::format::{
    build_alan_interrogation_prompt_for, collect_circle_tags, filter_palette_items,
};
use crate::platform_desktop::{copy_text_with_notice, restart_after_update};
use crate::stores::{
    AppStore, BasketsViewMode, CompanyViewTab, PaletteAction, PaletteItem, UniverseSearchMode,
    WorkspaceTab,
};
use crate::theme::*;
use m3_core::i18n::UiLang;
use m3_core::model::{Bucket, primary_circle_tag};
use m3_core::update::{self, ReleaseChannel};
use nsh_core::prelude::*;

/// Builds the full list of Command Palette actions from current store state.
pub fn build_palette_items_from_store(store: AppStore) -> Vec<PaletteItem> {
    let l = store.lang.get();
    let reports = store.reports.get();
    let selected_ticker = store.selected_ticker.get();
    let baskets_mode = store.baskets_view_mode.get();
    let use_ttm = store.use_ttm_basis.get();
    let update_channel = store.update_channel.get();
    let update_applied = store.update_applied_version.get();
    let update_notice = store.update_notice.get();

    let mut items = Vec::with_capacity(reports.len() * 2 + 28);
    for rep in &reports {
        let (bc_fg, _, _) = bucket_colors(rep.bucket);
        items.push(PaletteItem {
            group: "DOSSIER".into(),
            title: format!("{}  {} [{}]", rep.ticker, rep.name, rep.bucket.code()),
            subtitle: format!(
                "#{} · {}",
                primary_circle_tag(&rep.circle),
                rep.status.label_for(l)
            ),
            shortcut: Some("↵"),
            dot_color: bc_fg,
            action: PaletteAction::OpenDossier(rep.ticker.clone()),
        });
    }
    for rep in &reports {
        items.push(PaletteItem {
            group: "ALAN".into(),
            title: format!(
                "{} · {} {}",
                l.t("Alan 审讯", "Alan Interrogation"),
                rep.ticker,
                rep.name
            ),
            subtitle: l
                .t(
                    "三句话过滤、跨学科格栅与 5 年事前验尸",
                    "Three-sentence filter, latticework, and 5Y pre-mortem",
                )
                .into(),
            shortcut: None,
            dot_color: VIOLET_FG,
            action: PaletteAction::OpenAgent(rep.ticker.clone()),
        });
    }
    items.push(PaletteItem {
        group: "VIEW".into(),
        title: l
            .t(
                "简报收件箱 · Event-Driven Inbox",
                "Briefing Inbox · Event-Driven L1",
            )
            .into(),
        subtitle: l
            .t(
                "新浮现、隐空间传导与击球红灯待审讯信号（0 Flagship Token）",
                "Discoveries, latent ripples, and strike/red-flag signals (0 Flagship Tokens)",
            )
            .into(),
        shortcut: Some("⌘1"),
        dot_color: EMERALD_FG,
        action: PaletteAction::SwitchTab(WorkspaceTab::Inbox),
    });
    items.push(PaletteItem {
        group: "VIEW".into(),
        title: l
            .t(
                "全市场雷达 · US Lakehouse L0/L1",
                "Universe Radar · US Lakehouse L0/L1",
            )
            .into(),
        subtitle: l
            .t(
                "DuckDB 全美股筛选、Lookalike 相似搜索与一键入观察池",
                "DuckDB full-US screening, lookalike search, and 1-click Watch",
            )
            .into(),
        shortcut: Some("⌘2"),
        dot_color: SKY_FG,
        action: PaletteAction::SwitchTab(WorkspaceTab::Universe),
    });
    items.push(PaletteItem {
        group: "VIEW".into(),
        title: l
            .t(
                "行业研究 · Industry Research Studio",
                "Industry Research · Alan Studio",
            )
            .into(),
        subtitle: l
            .t(
                "4 视角行业深研、10-K/Web 出处分离、费曼自测与能力圈毕业阈值",
                "4-lens industry study, separated 10-K/Web citations, Feynman quiz & Circle graduation",
            )
            .into(),
        shortcut: Some("⌘3"),
        dot_color: VIOLET_FG,
        action: PaletteAction::SwitchTab(WorkspaceTab::Industry),
    });
    items.push(PaletteItem {
        group: "VIEW".into(),
        title: l
            .t(
                "能力圈总览 (CORE / WATCH / OUT / TOO HARD)",
                "Circle Overview (CORE / WATCH / OUT / TOO HARD)",
            )
            .into(),
        subtitle: l
            .t(
                "今日信号与能力圈四层分拣筛选",
                "Today's signals and four-tier Circle triage",
            )
            .into(),
        shortcut: Some("⌘4"),
        dot_color: INK_PRIMARY,
        action: PaletteAction::FocusBasket(None),
    });
    items.push(PaletteItem {
        group: "VIEW".into(),
        title: l
            .t("切换 总览 列表 / 看板", "Toggle Circle List / Board View")
            .into(),
        subtitle: match baskets_mode {
            BasketsViewMode::List => l.t("切换为 4 列工作流看板", "Switch to 4-column board"),
            BasketsViewMode::Board => l.t("切换为分组指标列表", "Switch to grouped table list"),
        }
        .into(),
        shortcut: Some("v"),
        dot_color: SKY_FG,
        action: PaletteAction::ToggleBasketsView,
    });
    if !selected_ticker.is_empty() {
        items.push(PaletteItem {
            group: "VIEW".into(),
            title: format!(
                "{} · {}",
                l.t("财报与 K 线", "Financials & Chart"),
                selected_ticker
            ),
            subtitle: l
                .t(
                    "值锚定 K 线、查理沙盒、ROIC 与人类判词",
                    "Intrinsic-value chart, Charlie Sandbox, ROIC, and verdict",
                )
                .into(),
            shortcut: None,
            dot_color: EMERALD_FG,
            action: PaletteAction::OpenCompanyTab(CompanyViewTab::FinancialsMemo),
        });
        items.push(PaletteItem {
            group: "VIEW".into(),
            title: format!("Research Desk · Deep Research & Alan ({selected_ticker})"),
            subtitle: l
                .t(
                    "4 视角并行深研、Living Research Memo 与费曼审讯",
                    "4-perspective Deep Research, Living Memo, and Feynman chat",
                )
                .into(),
            shortcut: None,
            dot_color: VIOLET_FG,
            action: PaletteAction::OpenCompanyTab(CompanyViewTab::DeepResearchAlan),
        });
        items.push(PaletteItem {
            group: "RESEARCH".into(),
            title: format!(
                "{} ({selected_ticker})",
                l.t(
                    "Deep Research · 4 视角并行深研",
                    "Deep Research · 4 Parallel Perspectives"
                )
            ),
            subtitle: l
                .t(
                    "受预算硬门控：护城河 / 会计排雷 / 隐空间同行 / 5年逆向验尸",
                    "Budget-gated: Moat / Accounting / Latent Peers / 5Y Pre-Mortem",
                )
                .into(),
            shortcut: None,
            dot_color: VIOLET_FG,
            action: PaletteAction::RunDeepResearch(selected_ticker.clone()),
        });
        items.push(PaletteItem {
            group: "VIEW".into(),
            title: l
                .t(
                    "XBRL 底表 · 多年原账与公式审计",
                    "XBRL Ledger · Multi-Year Filings & Audit",
                )
                .into(),
            subtitle: match l {
                UiLang::Zh => format!("查看 {selected_ticker} 算术推导与 SHA-256 溯源"),
                UiLang::En => format!("Inspect {selected_ticker} arithmetic & SHA-256 provenance"),
            },
            shortcut: Some("i"),
            dot_color: SKY_FG,
            action: PaletteAction::ToggleInspector,
        });
    }
    items.push(PaletteItem {
        group: "BUDGET".into(),
        title: l
            .t(
                "Budget Status · 查看本月 AI 预算与配额",
                "Budget Status · Monthly AI Budget & Quota",
            )
            .into(),
        subtitle: l
            .t(
                "查看 workbench/budget.json 限额、本月 Token 消耗与剩余配额",
                "Inspect workbench/budget.json caps, monthly spend, and remaining tokens",
            )
            .into(),
        shortcut: None,
        dot_color: EMERALD_FG,
        action: PaletteAction::ShowBudgetStatus,
    });
    items.push(PaletteItem {
        group: "THEMES".into(),
        title: l
            .t(
                "Sync Themes · 同步跨公司商业原型主题图谱",
                "Sync Themes · Cross-Company Archetype Graph",
            )
            .into(),
        subtitle: l
            .t(
                "从 DuckDB 湖仓生成 workbench/themes.json 并保留人类批注",
                "Generate workbench/themes.json from DuckDB while preserving human notes",
            )
            .into(),
        shortcut: Some("s"),
        dot_color: SKY_FG,
        action: PaletteAction::SyncThemes,
    });
    items.push(PaletteItem {
        group: "VIEW".into(),
        title: l
            .t("决策账本 · Decision Journal", "Decision Journal")
            .into(),
        subtitle: l
            .t(
                "事前冻结决策账本 (100% 现金与 S&P 500 对比)",
                "Append-only decision journal vs 100% cash & S&P 500",
            )
            .into(),
        shortcut: Some("⌘5"),
        dot_color: GOLD_FG,
        action: PaletteAction::SwitchTab(WorkspaceTab::Journal),
    });
    items.push(PaletteItem {
        group: "SETTINGS".into(),
        title: l
            .t(
                "设置 · Preferences & Guardrails",
                "Settings · Preferences & Guardrails",
            )
            .into(),
        subtitle: l
            .t(
                "语言切换 (中文/English)、工作区目录、更新通道与 L2 预算上限",
                "Language (中文/English), Workspace root, Release channel, and AI budget caps",
            )
            .into(),
        shortcut: Some("⌘,"),
        dot_color: INK_HERO,
        action: PaletteAction::OpenSettings,
    });
    let target_lang = match l {
        UiLang::Zh => UiLang::En,
        UiLang::En => UiLang::Zh,
    };
    items.push(PaletteItem {
        group: "LANGUAGE".into(),
        title: match target_lang {
            UiLang::En => "Switch UI Language to English".into(),
            UiLang::Zh => "切换界面语言为中文".into(),
        },
        subtitle: l
            .t(
                "即时切换菜单栏与全部桌面视图至 English",
                "Switch all menus and desktop views immediately to 中文",
            )
            .into(),
        shortcut: None,
        dot_color: SKY_FG,
        action: PaletteAction::SetLanguage(target_lang),
    });
    items.push(PaletteItem {
        group: "SEARCH".into(),
        title: l
            .t(
                "混合语义相似搜索 · Lookalike Discovery",
                "Hybrid Semantic Search · Lookalike Discovery",
            )
            .into(),
        subtitle: l
            .t(
                "按商业模式描述或 Ticker 在全美股 L0/L1 中检索相似公司与出处",
                "Search full-US lakehouse by business description or ticker with citations",
            )
            .into(),
        shortcut: None,
        dot_color: VIOLET_FG,
        action: PaletteAction::FocusSimilarSearch,
    });
    for b in Bucket::display_order() {
        let (fg, _, _) = bucket_colors(b);
        items.push(PaletteItem {
            group: "CIRCLE".into(),
            title: format!(
                "{} · {} ({})",
                l.t("聚焦能力圈", "Focus Tier"),
                b.code(),
                b.short_label_for(l)
            ),
            subtitle: b.description_for(l).into(),
            shortcut: None,
            dot_color: fg,
            action: PaletteAction::FocusBasket(Some(b)),
        });
    }
    if !selected_ticker.is_empty() {
        for b in Bucket::display_order() {
            let (fg, _, _) = bucket_colors(b);
            items.push(PaletteItem {
                group: "MOVE".into(),
                title: match l {
                    UiLang::Zh => format!(
                        "将 {selected_ticker} 归入 [{} · {}]",
                        b.code(),
                        b.short_label_for(UiLang::Zh)
                    ),
                    UiLang::En => format!(
                        "Move {selected_ticker} to [{} · {}]",
                        b.code(),
                        b.short_label_for(UiLang::En)
                    ),
                },
                subtitle: l
                    .t(
                        "写入 workbench/watchlist.json 并重算状态",
                        "Save to workbench/watchlist.json and recompute",
                    )
                    .into(),
                shortcut: None,
                dot_color: fg,
                action: PaletteAction::MoveCompanyBucket(selected_ticker.clone(), b),
            });
        }
    }
    for (tag, cnt) in collect_circle_tags(&reports) {
        items.push(PaletteItem {
            group: "TAG".into(),
            title: format!("{} #{tag}", l.t("按标签筛选", "Filter by tag")),
            subtitle: match l {
                UiLang::Zh => format!("跨能力圈关联 {cnt} 家标的"),
                UiLang::En => format!("{cnt} linked holdings across Circle"),
            },
            shortcut: None,
            dot_color: SKY_FG,
            action: PaletteAction::FilterCircle(Some(tag)),
        });
    }
    items.push(PaletteItem {
        group: "DOSSIER".into(),
        title: l.t("切换 TTM / FY 口径", "Toggle TTM / FY Basis").into(),
        subtitle: if use_ttm {
            l.t("当前: TTM 滚动 12 个月", "Current: TTM trailing 12 months")
        } else {
            l.t("当前: FY 最近完整财年", "Current: Latest full fiscal year")
        }
        .into(),
        shortcut: Some("t"),
        dot_color: SKY_FG,
        action: PaletteAction::ToggleBasis,
    });
    if !selected_ticker.is_empty() {
        let selected_name = reports
            .iter()
            .find(|r| r.ticker == selected_ticker)
            .map(|r| r.name.as_str())
            .unwrap_or(&selected_ticker);
        let selected_bucket = reports
            .iter()
            .find(|r| r.ticker == selected_ticker)
            .map(|r| r.bucket)
            .unwrap_or(Bucket::Watch);
        items.push(PaletteItem {
            group: "PROMPT".into(),
            title: format!(
                "{} ({selected_ticker})",
                l.t(
                    "复制 Alan 费曼审讯 Prompt",
                    "Copy Alan Interrogation Prompt"
                )
            ),
            subtitle: l
                .t(
                    "按 docs/munger-workflow.md 生成只读审讯指令",
                    "Generate read-only interrogation prompt per workflow",
                )
                .into(),
            shortcut: Some("p"),
            dot_color: VIOLET_FG,
            action: PaletteAction::CopyCli(
                build_alan_interrogation_prompt_for(
                    &selected_ticker,
                    selected_name,
                    selected_bucket,
                    l,
                ),
                format!("Alan Prompt {selected_ticker}"),
            ),
        });
        items.push(PaletteItem {
            group: "CLI".into(),
            title: format!(
                "{}: m3 report {selected_ticker}",
                l.t("复制 CLI", "Copy CLI")
            ),
            subtitle: l.t("写入系统剪贴板", "Copy to system clipboard").into(),
            shortcut: Some("c"),
            dot_color: INK_SECONDARY,
            action: PaletteAction::CopyCli(
                format!("m3 report {selected_ticker}"),
                format!("report {selected_ticker}"),
            ),
        });
    }
    items.push(PaletteItem {
        group: "DATA".into(),
        title: l
            .t(
                "同步 SEC EDGAR 财报、股价与 K 线历史",
                "Sync SEC EDGAR Filings, Quotes & History",
            )
            .into(),
        subtitle: l
            .t(
                "更新一手 XBRL 与 5Y/1Y 行情",
                "Refresh primary XBRL and 5Y/1Y prices",
            )
            .into(),
        shortcut: Some("⌘R"),
        dot_color: EMERALD_FG,
        action: PaletteAction::SyncNow,
    });
    items.push(PaletteItem {
        group: "DATA".into(),
        title: l
            .t(
                "追平事件流 · L1 SEC Filings / News / 语义索引",
                "Catch Up L1 · SEC Filings / News / Semantic Index",
            )
            .into(),
        subtitle: l
            .t(
                "增量拉取 SEC 公告与新闻，生成新浮现/传导/击球信号（0 Flagship Token）",
                "Fetch incremental filings & news to generate signals (0 Flagship Tokens)",
            )
            .into(),
        shortcut: Some("r"),
        dot_color: EMERALD_FG,
        action: PaletteAction::CatchupEvents,
    });
    items.push(PaletteItem {
        group: "DATA".into(),
        title: l
            .t(
                "同步全美股 L0 湖仓 (SEC Bulk + 价格分层)",
                "Sync US L0 Lakehouse (SEC Bulk + Top Quotes)",
            )
            .into(),
        subtitle: l
            .t(
                "下载 companyfacts.zip 并计算 5,000+ 美股质量评分与 Top 400 报价",
                "Download companyfacts.zip and score 5,000+ US issuers",
            )
            .into(),
        shortcut: None,
        dot_color: SKY_FG,
        action: PaletteAction::SyncUniverseNow,
    });
    items.push(PaletteItem {
        group: "UPDATE".into(),
        title: match l {
            UiLang::Zh => format!(
                "检查 M3 更新（当前 v{} · {} 通道）",
                update::current_binary_version(),
                update_channel.as_str()
            ),
            UiLang::En => format!(
                "Check for M3 Updates (current v{} · {})",
                update::current_binary_version(),
                update_channel.as_str()
            ),
        },
        subtitle: l
            .t(
                "查询 GitHub Release 清单并比对 SHA-256",
                "Query GitHub release manifest and verify SHA-256",
            )
            .into(),
        shortcut: None,
        dot_color: SKY_FG,
        action: PaletteAction::CheckUpdate,
    });
    if let Some(ver) = &update_applied {
        items.push(PaletteItem {
            group: "UPDATE".into(),
            title: match l {
                UiLang::Zh => format!("立即重启应用已安装的 M3 v{ver}"),
                UiLang::En => format!("Restart to Apply Installed M3 v{ver}"),
            },
            subtitle: l
                .t(
                    "重启新版本 M3.app 使更新生效",
                    "Relaunch M3.app to finish update",
                )
                .into(),
            shortcut: None,
            dot_color: EMERALD_FG,
            action: PaletteAction::RestartApp,
        });
    } else if let Some(notice) = &update_notice {
        items.push(PaletteItem {
            group: "UPDATE".into(),
            title: format!(
                "{} · v{} ({})",
                l.t("安装更新", "Install Update"),
                notice.latest_version,
                notice.channel.as_str()
            ),
            subtitle: l
                .t(
                    "校验 SHA-256 并原子替换当前 M3.app 与 ~/.local/bin/m3",
                    "Verify SHA-256 and atomically swap M3.app & ~/.local/bin/m3",
                )
                .into(),
            shortcut: None,
            dot_color: EMERALD_FG,
            action: PaletteAction::ApplyUpdate,
        });
    }
    let target_channel = match update_channel {
        ReleaseChannel::Stable => ReleaseChannel::Beta,
        ReleaseChannel::Beta => ReleaseChannel::Stable,
    };
    items.push(PaletteItem {
        group: "UPDATE".into(),
        title: match l {
            UiLang::Zh => format!("切换更新通道至 {}", target_channel.label_cn()),
            UiLang::En => format!("Switch Release Channel to {}", target_channel.as_str()),
        },
        subtitle: match (target_channel, l) {
            (ReleaseChannel::Beta, UiLang::Zh) => {
                "跟随 main 分支每次合并自动发布的 rolling beta 构建"
            }
            (ReleaseChannel::Beta, UiLang::En) => {
                "Track rolling beta builds published on every merge to main"
            }
            (ReleaseChannel::Stable, UiLang::Zh) => "仅接收带 tag (vX.Y.Z) 的稳定发布版本",
            (ReleaseChannel::Stable, UiLang::En) => "Track tagged stable releases only (vX.Y.Z)",
        }
        .into(),
        shortcut: None,
        dot_color: GOLD_FG,
        action: PaletteAction::SwitchUpdateChannel(target_channel),
    });
    items.push(PaletteItem {
        group: "M³".into(),
        title: "M³ · Money Making Machine".into(),
        subtitle: l
            .t(
                "查看 Project M3 铭刻 · 机器读完 6,000 家财报，你只挥击甜区好球",
                "Inspect the Project M3 caseback engraving · Compound patiently",
            )
            .into(),
        shortcut: None,
        dot_color: EMERALD_FG,
        action: PaletteAction::ShowM3Engraving,
    });
    items
}

/// Executes a selected `PaletteAction` and closes the palette.
pub fn execute_palette_action(store: AppStore, action: PaletteAction) {
    store.close_palette();
    match action {
        PaletteAction::OpenDossier(ticker) => {
            store.open_company(&ticker, CompanyViewTab::FinancialsMemo);
        }
        PaletteAction::OpenAgent(ticker) => {
            store.open_company(&ticker, CompanyViewTab::DeepResearchAlan);
        }
        PaletteAction::RunDeepResearch(ticker) => {
            store.open_company(&ticker, CompanyViewTab::DeepResearchAlan);
            store.start_deep_research(&ticker, None, "manual");
        }
        PaletteAction::ShowBudgetStatus => {
            store.show_budget_status_notice();
        }
        PaletteAction::ShowM3Engraving => {
            store.show_m3_engraving_notice();
        }
        PaletteAction::SyncThemes => {
            store.trigger_themes_sync();
        }
        PaletteAction::SwitchTab(tab) => {
            store.open_tab(tab);
        }
        PaletteAction::OpenCompanyTab(tab) => {
            store.open_selected_company(tab);
        }
        PaletteAction::FocusBasket(bucket) => {
            store.focus_basket(bucket);
        }
        PaletteAction::ToggleBasketsView => {
            store.toggle_baskets_view();
        }
        PaletteAction::MoveCompanyBucket(ticker, target) => {
            store.move_company_bucket(&ticker, target);
        }
        PaletteAction::FilterCircle(tag) => {
            store.circle_filter.set(tag);
            store.basket_filter.set(None);
            store.baskets_cursor_index.set(0);
            store.open_tab(WorkspaceTab::Baskets);
        }
        PaletteAction::ToggleBasis => {
            store.toggle_basis();
            store.open_selected_company(CompanyViewTab::FinancialsMemo);
        }
        PaletteAction::ToggleInspector => {
            store.toggle_inspector();
        }
        PaletteAction::CopyCli(cmd, label) => {
            copy_text_with_notice(store, cmd, &label);
        }
        PaletteAction::SyncNow => {
            store.start_sync();
        }
        PaletteAction::SyncUniverseNow => {
            store.trigger_universe_sync();
        }
        PaletteAction::CatchupEvents => {
            store.trigger_l1_catchup();
        }
        PaletteAction::FocusSimilarSearch => {
            store.set_universe_search_mode(UniverseSearchMode::Similar);
            store.open_tab(WorkspaceTab::Universe);
        }
        PaletteAction::CheckUpdate => {
            store.start_update_check(true);
        }
        PaletteAction::ApplyUpdate => {
            store.start_update_apply();
        }
        PaletteAction::SwitchUpdateChannel(channel) => {
            store.switch_update_channel(channel);
        }
        PaletteAction::RestartApp => {
            restart_after_update(store);
        }
        PaletteAction::OpenSettings => {
            store.open_settings();
        }
        PaletteAction::SetLanguage(lang) => {
            store.set_language(lang);
        }
    }
}

/// Renders the `⌘K` Command Palette overlay when `store.palette_open` is `true`.
pub fn command_palette_overlay(store: AppStore) -> impl IntoElement {
    let filtered_items = computed(move || {
        let all = build_palette_items_from_store(store);
        let q = store.palette_query.get();
        filter_palette_items(&all, &q)
            .into_iter()
            .cloned()
            .collect::<Vec<_>>()
    });

    overlay()
        .absolute()
        .inset_0()
        .flex()
        .flex_col()
        .items_center()
        .pt(px(88.0))
        .child(
            view()
                .absolute()
                .inset_0()
                .bg(rgba(SCRIM_BACKDROP))
                .on_press(move || store.close_palette()),
        )
        .child(
            view()
                .w(px(600.0))
                .max_h(px(470.0))
                .rounded_xl()
                .bg(rgb(BG_PALETTE))
                .border_1()
                .border_color(rgb(BORDER_PALETTE))
                .overflow_hidden()
                .flex()
                .flex_col()
                .child(
                    view()
                        .px(px(16.0))
                        .py(px(10.0))
                        .border_b_1()
                        .border_color(rgb(BORDER_HAIRLINE))
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(10.0))
                        .child(
                            text(">")
                                .font_family(FONT_MONO_TERM)
                                .text_size(px(TEXT_MD))
                                .font_weight(FontWeight::BOLD)
                                .text_color(rgb(SKY_FG)),
                        )
                        .child(
                            styled_input(
                                move || store.palette_query.get(),
                                move || {
                                    store
                                        .lang
                                        .get()
                                        .t(
                                            "输入 Ticker、公司名、行业或命令…",
                                            "Type ticker, company, industry, or command…",
                                        )
                                        .to_owned()
                                },
                                false,
                            )
                            .auto_focus(true)
                            .border_none()
                            .bg(rgba(0x00000000))
                            .flex_1()
                            .on_change(move |val: &str| {
                                store.palette_query.set(val.to_owned());
                                store.palette_cursor.set(0);
                            })
                            .on_submit(move |_: &str| {
                                let list = filtered_items.get_untracked();
                                let cur = store.palette_cursor.get_untracked();
                                let idx = cur.min(list.len().saturating_sub(1));
                                if let Some(it) = list.get(idx) {
                                    execute_palette_action(store, it.action.clone());
                                }
                            }),
                        )
                        .child(kbd("ESC")),
                )
                .child(
                    scroll()
                        .flex_1()
                        .min_h(px(0.0))
                        .max_h(px(400.0))
                        .child(dynamic(move || {
                            let list = filtered_items.get();
                            let active_idx =
                                store.palette_cursor.get().min(list.len().saturating_sub(1));
                            if list.is_empty() {
                                return view()
                                    .p(px(24.0))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .child(
                                        text(
                                            store
                                                .lang
                                                .get()
                                                .t(
                                                    "未匹配到命令或标的",
                                                    "No matching command or ticker",
                                                )
                                                .to_owned(),
                                        )
                                        .font_family(FONT_UI)
                                        .text_size(px(TEXT_SM))
                                        .text_color(rgb(INK_MUTED)),
                                    );
                            }
                            view().flex().flex_col().py(px(6.0)).children(
                                list.into_iter().enumerate().map(|(idx, item)| {
                                    let is_active = idx == active_idx;
                                    let action = item.action.clone();
                                    let row_bg = if is_active {
                                        rgb(BG_SELECTED_ROW)
                                    } else {
                                        rgba(0x00000000)
                                    };
                                    let mut row = view()
                                        .px(px(14.0))
                                        .py(px(8.0))
                                        .mx(px(6.0))
                                        .rounded_md()
                                        .cursor_pointer()
                                        .bg(row_bg)
                                        .hover(|s| s.bg(rgb(BG_HOVER)))
                                        .flex()
                                        .flex_row()
                                        .items_center()
                                        .justify_between()
                                        .gap(px(12.0))
                                        .on_press(move || {
                                            execute_palette_action(store, action.clone());
                                        })
                                        .child(
                                            view()
                                                .flex_1()
                                                .min_w(px(0.0))
                                                .flex()
                                                .flex_row()
                                                .items_center()
                                                .gap(px(10.0))
                                                .child(
                                                    view()
                                                        .w(px(6.0))
                                                        .h(px(6.0))
                                                        .rounded_full()
                                                        .flex_shrink_0()
                                                        .bg(rgb(item.dot_color)),
                                                )
                                                .child(
                                                    view()
                                                        .flex_1()
                                                        .min_w(px(0.0))
                                                        .flex()
                                                        .flex_col()
                                                        .gap(px(2.0))
                                                        .child(
                                                            text(item.title)
                                                                .font_family(FONT_UI)
                                                                .text_size(px(TEXT_SM))
                                                                .font_weight(FontWeight::MEDIUM)
                                                                .text_color(rgb(if is_active {
                                                                    INK_HERO
                                                                } else {
                                                                    INK_PRIMARY
                                                                }))
                                                                .truncate()
                                                                .number_of_lines(1),
                                                        )
                                                        .child(
                                                            text(item.subtitle)
                                                                .font_family(FONT_UI)
                                                                .text_size(px(TEXT_XS))
                                                                .text_color(rgb(INK_MUTED))
                                                                .truncate()
                                                                .number_of_lines(1),
                                                        ),
                                                ),
                                        );
                                    let mut right =
                                        view().flex().flex_row().items_center().gap(px(6.0)).child(
                                            text(item.group)
                                                .font_family(FONT_MONO_TERM)
                                                .text_size(px(TEXT_2XS))
                                                .text_color(rgb(INK_MUTED)),
                                        );
                                    if let Some(sc) = item.shortcut {
                                        right = right.child(kbd(sc));
                                    }
                                    row = row.child(right);
                                    row
                                }),
                            )
                        })),
                ),
        )
}
