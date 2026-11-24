use super::{
    BasketsViewMode, DossierTab, MungerWorkbench, NavView, PaletteAction, PaletteItem,
    components::*, format::*, theme::*, universe::UniverseSearchMode,
};
use crate::i18n::UiLang;
use crate::model::{Bucket, primary_circle_tag};
use crate::update::{self, ReleaseChannel};
use gpui::{Context, MouseButton, Window, div, prelude::*, px, rgb, rgba};

impl MungerWorkbench {
    pub fn toggle_palette(&mut self, window: &mut Window) {
        self.palette_open = !self.palette_open;
        self.palette_query.clear();
        self.palette_cursor = 0;
        window.focus(&self.focus_handle);
    }

    pub(super) fn build_palette_items(&self) -> Vec<PaletteItem> {
        let l = self.lang;
        let mut items = Vec::with_capacity(self.reports.len() * 2 + 28);
        for rep in &self.reports {
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
        for rep in &self.reports {
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
            action: PaletteAction::SwitchView(NavView::Inbox),
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
            action: PaletteAction::SwitchView(NavView::Universe),
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
            action: PaletteAction::SwitchView(NavView::Industry),
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
            subtitle: match self.baskets_view_mode {
                BasketsViewMode::List => l.t("切换为 4 列工作流看板", "Switch to 4-column board"),
                BasketsViewMode::Board => l.t("切换为分组指标列表", "Switch to grouped table list"),
            }
            .into(),
            shortcut: Some("v"),
            dot_color: SKY_FG,
            action: PaletteAction::ToggleBasketsView,
        });
        if !self.selected_ticker.is_empty() {
            items.push(PaletteItem {
                group: "VIEW".into(),
                title: format!(
                    "{} · {}",
                    l.t("财报与 K 线", "Financials & Chart"),
                    self.selected_ticker
                ),
                subtitle: l
                    .t(
                        "值锚定 K 线、查理沙盒、ROIC 与人类判词",
                        "Intrinsic-value chart, Charlie Sandbox, ROIC, and verdict",
                    )
                    .into(),
                shortcut: None,
                dot_color: EMERALD_FG,
                action: PaletteAction::SwitchView(NavView::Dossier),
            });
            items.push(PaletteItem {
                group: "VIEW".into(),
                title: format!(
                    "Research Desk · Deep Research & Alan ({})",
                    self.selected_ticker
                ),
                subtitle: l
                    .t(
                        "4 视角并行深研、Living Research Memo 与费曼审讯",
                        "4-perspective Deep Research, Living Memo, and Feynman chat",
                    )
                    .into(),
                shortcut: None,
                dot_color: VIOLET_FG,
                action: PaletteAction::SwitchView(NavView::Agent),
            });
            items.push(PaletteItem {
                group: "RESEARCH".into(),
                title: format!(
                    "{} ({})",
                    l.t(
                        "Deep Research · 4 视角并行深研",
                        "Deep Research · 4 Parallel Perspectives"
                    ),
                    self.selected_ticker
                ),
                subtitle: l
                    .t(
                        "受预算硬门控：护城河 / 会计排雷 / 隐空间同行 / 5年逆向验尸",
                        "Budget-gated: Moat / Accounting / Latent Peers / 5Y Pre-Mortem",
                    )
                    .into(),
                shortcut: None,
                dot_color: VIOLET_FG,
                action: PaletteAction::RunDeepResearch(self.selected_ticker.clone()),
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
        if !self.selected_ticker.is_empty() {
            items.push(PaletteItem {
                group: "VIEW".into(),
                title: l
                    .t(
                        "XBRL 底表 · 多年原账与公式审计",
                        "XBRL Ledger · Multi-Year Filings & Audit",
                    )
                    .into(),
                subtitle: match l {
                    UiLang::Zh => {
                        format!("查看 {} 算术推导与 SHA-256 溯源", self.selected_ticker)
                    }
                    UiLang::En => {
                        format!(
                            "Inspect {} arithmetic & SHA-256 provenance",
                            self.selected_ticker
                        )
                    }
                },
                shortcut: Some("i"),
                dot_color: SKY_FG,
                action: PaletteAction::ToggleInspector,
            });
        }
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
            action: PaletteAction::SwitchView(NavView::Journal),
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
        if !self.selected_ticker.is_empty() {
            for b in Bucket::display_order() {
                let (fg, _, _) = bucket_colors(b);
                items.push(PaletteItem {
                    group: "MOVE".into(),
                    title: match l {
                        UiLang::Zh => format!(
                            "将 {} 归入 [{} · {}]",
                            self.selected_ticker,
                            b.code(),
                            b.short_label_for(UiLang::Zh)
                        ),
                        UiLang::En => format!(
                            "Move {} to [{} · {}]",
                            self.selected_ticker,
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
                    action: PaletteAction::MoveCompanyBucket(self.selected_ticker.clone(), b),
                });
            }
        }
        for (tag, cnt) in collect_circle_tags(&self.reports) {
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
            subtitle: if self.use_ttm_basis {
                l.t("当前: TTM 滚动 12 个月", "Current: TTM trailing 12 months")
            } else {
                l.t("当前: FY 最近完整财年", "Current: Latest full fiscal year")
            }
            .into(),
            shortcut: Some("t"),
            dot_color: SKY_FG,
            action: PaletteAction::ToggleBasis,
        });
        let selected_name = self
            .reports
            .iter()
            .find(|r| r.ticker == self.selected_ticker)
            .map(|r| r.name.as_str())
            .unwrap_or(&self.selected_ticker);
        let selected_bucket = self
            .reports
            .iter()
            .find(|r| r.ticker == self.selected_ticker)
            .map(|r| r.bucket)
            .unwrap_or(Bucket::Watch);
        if !self.selected_ticker.is_empty() {
            items.push(PaletteItem {
                group: "PROMPT".into(),
                title: format!(
                    "{} ({})",
                    l.t(
                        "复制 Alan 费曼审讯 Prompt",
                        "Copy Alan Interrogation Prompt"
                    ),
                    self.selected_ticker
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
                        &self.selected_ticker,
                        selected_name,
                        selected_bucket,
                        l,
                    ),
                    format!("Alan Prompt {}", self.selected_ticker),
                ),
            });
            items.push(PaletteItem {
                group: "CLI".into(),
                title: format!(
                    "{}: ./apps/m3-desktop/m3 report {}",
                    l.t("复制 CLI", "Copy CLI"),
                    self.selected_ticker
                ),
                subtitle: l.t("写入系统剪贴板", "Copy to system clipboard").into(),
                shortcut: Some("c"),
                dot_color: INK_SECONDARY,
                action: PaletteAction::CopyCli(
                    format!("./apps/m3-desktop/m3 report {}", self.selected_ticker),
                    format!("report {}", self.selected_ticker),
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
                    self.update_channel.as_str()
                ),
                UiLang::En => format!(
                    "Check for M3 Updates (current v{} · {})",
                    update::current_binary_version(),
                    self.update_channel.as_str()
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
        if let Some(ver) = &self.update_applied_version {
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
        } else if let Some(notice) = &self.update_notice {
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
        let target_channel = match self.update_channel {
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
                (ReleaseChannel::Stable, UiLang::En) => {
                    "Track tagged stable releases only (vX.Y.Z)"
                }
            }
            .into(),
            shortcut: None,
            dot_color: GOLD_FG,
            action: PaletteAction::SwitchUpdateChannel(target_channel),
        });
        items
    }

    pub(super) fn execute_palette_action(&mut self, action: PaletteAction, cx: &mut Context<Self>) {
        self.palette_open = false;
        self.palette_query.clear();
        self.palette_cursor = 0;
        match action {
            PaletteAction::OpenDossier(ticker) => {
                self.select_company(&ticker, Some(NavView::Dossier));
            }
            PaletteAction::OpenAgent(ticker) => {
                self.select_company(&ticker, Some(NavView::Agent));
            }
            PaletteAction::RunDeepResearch(ticker) => {
                self.select_company(&ticker, Some(NavView::Agent));
                self.start_deep_research(&ticker, None, "manual", cx);
                return;
            }
            PaletteAction::ShowBudgetStatus => {
                self.reload_budget_from_lake();
                if let Some(b) = &self.budget_report {
                    let used = b.usage.total_cost_usd.round_dp(4).normalize();
                    let limit = b.config.monthly_budget_usd.round_dp(2).normalize();
                    let rem = b.remaining_monthly_usd.round_dp(4).normalize();
                    let tok_used = b
                        .usage
                        .total_input_tokens
                        .saturating_add(b.usage.total_output_tokens);
                    self.banner = Some((
                        match self.lang {
                            UiLang::Zh => format!(
                                "本月 AI 预算 ({})：已用 ${used} / ${limit}（剩余 ${rem} · Token {tok_used} / {} · 完成 {} / 拦截 {}）",
                                b.usage.month_utc,
                                b.config.monthly_token_limit,
                                b.usage.completed_runs,
                                b.usage.blocked_runs
                            ),
                            UiLang::En => format!(
                                "AI Budget ({}): used ${used} / ${limit} (rem ${rem} · tokens {tok_used} / {} · ok {} / blocked {})",
                                b.usage.month_utc,
                                b.config.monthly_token_limit,
                                b.usage.completed_runs,
                                b.usage.blocked_runs
                            ),
                        },
                        false,
                    ));
                } else {
                    self.banner = Some((
                        self.lang
                            .t("无法读取 AI 预算状态", "Unable to read AI budget status")
                            .into(),
                        true,
                    ));
                }
            }
            PaletteAction::SyncThemes => {
                self.switch_view(NavView::Journal);
                self.start_themes_sync(cx);
                return;
            }
            PaletteAction::SwitchView(view) => {
                self.switch_view(view);
            }
            PaletteAction::FocusBasket(bucket) => {
                self.focus_basket(bucket);
            }
            PaletteAction::ToggleBasketsView => {
                self.baskets_view_mode = match self.baskets_view_mode {
                    BasketsViewMode::List => BasketsViewMode::Board,
                    BasketsViewMode::Board => BasketsViewMode::List,
                };
                self.active_view = NavView::Baskets;
            }
            PaletteAction::MoveCompanyBucket(ticker, target) => {
                self.move_company_bucket(&ticker, target, cx);
                return;
            }
            PaletteAction::FilterCircle(tag) => {
                self.circle_filter = tag;
                self.basket_filter = None;
                self.cursor_index = 0;
                self.active_view = NavView::Baskets;
            }
            PaletteAction::ToggleBasis => {
                self.use_ttm_basis = !self.use_ttm_basis;
                self.active_view = NavView::Dossier;
            }
            PaletteAction::ToggleInspector => {
                self.dossier_tab = if self.active_view == NavView::Dossier
                    && self.dossier_tab == DossierTab::Inspector
                {
                    DossierTab::Overview
                } else {
                    DossierTab::Inspector
                };
                self.active_view = NavView::Dossier;
            }
            PaletteAction::CopyCli(cmd, label) => {
                self.copy_cli_command(cmd, &label, cx);
            }
            PaletteAction::SyncNow => {
                self.start_sync(cx);
                return;
            }
            PaletteAction::SyncUniverseNow => {
                self.switch_view(NavView::Universe);
                self.start_universe_sync(false, cx);
                return;
            }
            PaletteAction::CatchupEvents => {
                self.switch_view(NavView::Inbox);
                self.start_l1_catchup(false, cx);
                return;
            }
            PaletteAction::FocusSimilarSearch => {
                self.universe_state
                    .set_search_mode(UniverseSearchMode::Similar);
                self.universe_state.similar_as_ticker = false;
                self.universe_state.search_focused = true;
                self.switch_view(NavView::Universe);
            }
            PaletteAction::CheckUpdate => {
                self.start_update_check(true, cx);
                return;
            }
            PaletteAction::ApplyUpdate => {
                self.start_update_apply(cx);
                return;
            }
            PaletteAction::SwitchUpdateChannel(channel) => {
                self.switch_update_channel(channel, cx);
                return;
            }
            PaletteAction::RestartApp => {
                self.restart_after_update(cx);
                return;
            }
            PaletteAction::OpenSettings => {
                self.switch_view(NavView::Settings);
            }
            PaletteAction::SetLanguage(lang) => {
                self.set_language(lang, cx);
                return;
            }
        }
        cx.notify();
    }

    pub(super) fn render_command_palette(
        &self,
        cx: &mut Context<Self>,
    ) -> gpui::Stateful<gpui::Div> {
        let all_items = self.build_palette_items();
        let filtered = filter_palette_items(&all_items, &self.palette_query);
        let active_idx = self.palette_cursor.min(filtered.len().saturating_sub(1));

        div()
            .id("palette-backdrop")
            .absolute()
            .inset_0()
            .bg(rgba(SCRIM_BACKDROP))
            .flex()
            .flex_col()
            .items_center()
            .pt(px(88.))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    this.palette_open = false;
                    this.palette_query.clear();
                    this.palette_cursor = 0;
                    cx.notify();
                }),
            )
            .child(
                div()
                    .id("palette-modal")
                    .w(px(600.))
                    .max_h(px(470.))
                    .rounded_xl()
                    .bg(rgb(BG_PALETTE))
                    .border_1()
                    .border_color(rgb(BORDER_PALETTE))
                    .shadow_2xl()
                    .overflow_hidden()
                    .flex()
                    .flex_col()
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|_, _, _, cx| {
                            cx.stop_propagation();
                        }),
                    )
                    .child(
                        div()
                            .px_4()
                            .py_3()
                            .border_b_1()
                            .border_color(rgb(BORDER_HAIRLINE))
                            .flex()
                            .items_center()
                            .justify_between()
                            .gap_2p5()
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_2()
                                    .flex_1()
                                    .min_w_0()
                                    .child(
                                        div()
                                            .font_family(self.fonts.mono_term.clone())
                                            .text_sm()
                                            .font_weight(gpui::FontWeight::BOLD)
                                            .text_color(rgb(EMERALD_FG))
                                            .child(">"),
                                    )
                                    .child(
                                        div()
                                            .font_family(self.fonts.mono_term.clone())
                                            .text_sm()
                                            .text_color(rgb(if self.palette_query.is_empty() {
                                                INK_MUTED
                                            } else {
                                                INK_HERO
                                            }))
                                            .truncate()
                                            .child(if self.palette_query.is_empty() {
                                                self.lang
                                                    .t(
                                                        "输入标的、能力圈 (CORE/WATCH/OUT/TOO HARD)、#标签 或命令…",
                                                        "Search ticker, Circle tier (CORE/WATCH/OUT/TOO HARD), #tag, or command…",
                                                    )
                                                    .to_owned()
                                            } else {
                                                format!("{}_", self.palette_query)
                                            }),
                                    ),
                            )
                            .child(kbd("esc", &self.fonts)),
                    )
                    .child(
                        div()
                            .id("palette-list")
                            .flex_1()
                            .min_h_0()
                            .overflow_y_scroll()
                            .p_2()
                            .when(filtered.is_empty(), |d| {
                                d.child(
                                    div()
                                        .p_4()
                                        .font_family(self.fonts.mono_term.clone())
                                        .text_xs()
                                        .text_color(rgb(INK_MUTED))
                                        .child(self.lang.t(
                                            "无匹配命令、标的或标签",
                                            "No matching commands, tickers, or tags",
                                        )),
                                )
                            })
                            .children(filtered.into_iter().enumerate().map(|(i, item)| {
                                let selected = i == active_idx;
                                let action = item.action.clone();
                                div()
                                    .id(gpui::SharedString::from(format!("pal-item-{i}")))
                                    .px_3()
                                    .py_2()
                                    .rounded_md()
                                    .cursor_pointer()
                                    .when(selected, |d| d.bg(rgb(BG_HOVER)))
                                    .hover(|s| s.bg(rgb(BG_SELECTED_ROW)))
                                    .flex()
                                    .items_center()
                                    .justify_between()
                                    .gap_3()
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.execute_palette_action(action.clone(), cx);
                                    }))
                                    .child(
                                        div()
                                            .flex()
                                            .items_center()
                                            .gap_2p5()
                                            .min_w_0()
                                            .flex_1()
                                            .child(
                                                div()
                                                    .w(px(6.))
                                                    .h(px(6.))
                                                    .rounded_full()
                                                    .flex_shrink_0()
                                                    .bg(rgb(item.dot_color)),
                                            )
                                            .child(
                                                div()
                                                    .w(px(76.))
                                                    .flex_shrink_0()
                                                    .font_family(self.fonts.mono_term.clone())
                                                    .text_size(px(10.))
                                                    .text_color(rgb(INK_MUTED))
                                                    .child(item.group.clone()),
                                            )
                                            .child(
                                                div()
                                                    .flex()
                                                    .flex_col()
                                                    .min_w_0()
                                                    .child(
                                                        div()
                                                            .font_family(
                                                                self.fonts.mono_term.clone(),
                                                            )
                                                            .text_xs()
                                                            .font_weight(gpui::FontWeight::SEMIBOLD)
                                                            .text_color(rgb(if selected {
                                                                INK_HERO
                                                            } else {
                                                                INK_PRIMARY
                                                            }))
                                                            .truncate()
                                                            .child(item.title.clone()),
                                                    )
                                                    .child(
                                                        div()
                                                            .font_family(self.fonts.ui.clone())
                                                            .text_size(px(11.))
                                                            .text_color(rgb(INK_MUTED))
                                                            .truncate()
                                                            .child(item.subtitle.clone()),
                                                    ),
                                            ),
                                    )
                                    .when_some(item.shortcut, |d, sc| d.child(kbd(sc, &self.fonts)))
                            })),
                    ),
            )
    }
}
