use super::{
    BasketsViewMode, DossierTab, MungerWorkbench, NavView, PaletteAction, PaletteItem,
    components::*, format::*, theme::*, universe::UniverseSearchMode,
};
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
        let mut items = Vec::with_capacity(self.reports.len() * 2 + 24);
        for rep in &self.reports {
            let (bc_fg, _, _) = bucket_colors(rep.bucket);
            items.push(PaletteItem {
                group: "DOSSIER".into(),
                title: format!("{}  {} [{}]", rep.ticker, rep.name, rep.bucket.code()),
                subtitle: format!(
                    "#{} · {}",
                    primary_circle_tag(&rep.circle),
                    rep.status.label()
                ),
                shortcut: Some("↵"),
                dot_color: bc_fg,
                action: PaletteAction::OpenDossier(rep.ticker.clone()),
            });
        }
        for rep in &self.reports {
            items.push(PaletteItem {
                group: "ALAN".into(),
                title: format!("Alan 审讯 · {} {}", rep.ticker, rep.name),
                subtitle: "三句话过滤、跨学科格栅与 5 年事前验尸".into(),
                shortcut: None,
                dot_color: VIOLET_FG,
                action: PaletteAction::OpenAgent(rep.ticker.clone()),
            });
        }
        items.push(PaletteItem {
            group: "VIEW".into(),
            title: "简报收件箱 · Event-Driven Inbox".into(),
            subtitle: "新浮现、隐空间传导与击球红灯待审讯信号（0 Flagship Token）".into(),
            shortcut: Some("⌘1"),
            dot_color: EMERALD_FG,
            action: PaletteAction::SwitchView(NavView::Inbox),
        });
        items.push(PaletteItem {
            group: "VIEW".into(),
            title: "全市场雷达 · US Lakehouse L0/L1".into(),
            subtitle: "DuckDB 全美股筛选、Lookalike 相似搜索与一键入观察池".into(),
            shortcut: Some("⌘2"),
            dot_color: SKY_FG,
            action: PaletteAction::SwitchView(NavView::Universe),
        });
        items.push(PaletteItem {
            group: "VIEW".into(),
            title: "篮子总览 (IN / WATCH / OUT / TOO HARD)".into(),
            subtitle: "今日信号与芒格四篮子筛选".into(),
            shortcut: Some("⌘3"),
            dot_color: INK_PRIMARY,
            action: PaletteAction::FocusBasket(None),
        });
        items.push(PaletteItem {
            group: "VIEW".into(),
            title: "切换 总览 列表 / 看板".into(),
            subtitle: match self.baskets_view_mode {
                BasketsViewMode::List => "切换为 4 列工作流看板",
                BasketsViewMode::Board => "切换为分组指标列表",
            }
            .into(),
            shortcut: Some("v"),
            dot_color: SKY_FG,
            action: PaletteAction::ToggleBasketsView,
        });
        items.push(PaletteItem {
            group: "VIEW".into(),
            title: format!("财报与 K 线 · {}", self.selected_ticker),
            subtitle: "值锚定 K 线、查理沙盒、ROIC 与人类判词".into(),
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
            subtitle: "4 视角并行深研、Living Research Memo 与费曼审讯".into(),
            shortcut: None,
            dot_color: VIOLET_FG,
            action: PaletteAction::SwitchView(NavView::Agent),
        });
        items.push(PaletteItem {
            group: "RESEARCH".into(),
            title: format!("Deep Research · 4 视角并行深研 ({})", self.selected_ticker),
            subtitle: "受预算硬门控：护城河 / 会计排雷 / 隐空间同行 / 5年逆向验尸".into(),
            shortcut: None,
            dot_color: VIOLET_FG,
            action: PaletteAction::RunDeepResearch(self.selected_ticker.clone()),
        });
        items.push(PaletteItem {
            group: "BUDGET".into(),
            title: "Budget Status · 查看本月 AI 预算与配额".into(),
            subtitle: "查看 workbench/budget.json 限额、本月 Token 消耗与剩余配额".into(),
            shortcut: None,
            dot_color: EMERALD_FG,
            action: PaletteAction::ShowBudgetStatus,
        });
        items.push(PaletteItem {
            group: "THEMES".into(),
            title: "Sync Themes · 同步跨公司商业原型主题图谱".into(),
            subtitle: "从 DuckDB 湖仓生成 workbench/themes.json 并保留人类批注".into(),
            shortcut: Some("s"),
            dot_color: SKY_FG,
            action: PaletteAction::SyncThemes,
        });
        items.push(PaletteItem {
            group: "VIEW".into(),
            title: "XBRL 底表 · 多年原账与公式审计".into(),
            subtitle: format!("查看 {} 算术推导与 SHA-256 溯源", self.selected_ticker),
            shortcut: Some("i"),
            dot_color: SKY_FG,
            action: PaletteAction::ToggleInspector,
        });
        items.push(PaletteItem {
            group: "VIEW".into(),
            title: "主题与账本 · Themes & Journal".into(),
            subtitle: "跨公司商业原型图谱 + 事前冻结决策账本 (100% 现金与 S&P 500)".into(),
            shortcut: Some("⌘4"),
            dot_color: GOLD_FG,
            action: PaletteAction::SwitchView(NavView::Journal),
        });
        items.push(PaletteItem {
            group: "SEARCH".into(),
            title: "混合语义相似搜索 · Lookalike Discovery".into(),
            subtitle: "按商业模式描述或 Ticker 在全美股 L0/L1 中检索相似公司与出处".into(),
            shortcut: None,
            dot_color: VIOLET_FG,
            action: PaletteAction::FocusSimilarSearch,
        });
        for b in Bucket::display_order() {
            let (fg, _, _) = bucket_colors(b);
            items.push(PaletteItem {
                group: "BASKET".into(),
                title: format!("聚焦篮子 · {} ({})", b.code(), b.short_label()),
                subtitle: b.description().into(),
                shortcut: None,
                dot_color: fg,
                action: PaletteAction::FocusBasket(Some(b)),
            });
        }
        for b in Bucket::display_order() {
            let (fg, _, _) = bucket_colors(b);
            items.push(PaletteItem {
                group: "MOVE".into(),
                title: format!(
                    "将 {} 归入 [{} · {}]",
                    self.selected_ticker,
                    b.code(),
                    b.short_label()
                ),
                subtitle: "写入 workbench/watchlist.json 并重算状态".into(),
                shortcut: None,
                dot_color: fg,
                action: PaletteAction::MoveCompanyBucket(self.selected_ticker.clone(), b),
            });
        }
        for (tag, cnt) in collect_circle_tags(&self.reports) {
            items.push(PaletteItem {
                group: "TAG".into(),
                title: format!("按标签筛选 #{tag}"),
                subtitle: format!("跨篮子关联 {cnt} 家标的"),
                shortcut: None,
                dot_color: SKY_FG,
                action: PaletteAction::FilterCircle(Some(tag)),
            });
        }
        items.push(PaletteItem {
            group: "DOSSIER".into(),
            title: "切换 TTM / FY 口径".into(),
            subtitle: if self.use_ttm_basis {
                "当前: TTM 滚动 12 个月"
            } else {
                "当前: FY 最近完整财年"
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
        items.push(PaletteItem {
            group: "PROMPT".into(),
            title: format!("复制 Alan 费曼审讯 Prompt ({})", self.selected_ticker),
            subtitle: "按 docs/munger-workflow.md 生成只读审讯指令".into(),
            shortcut: Some("p"),
            dot_color: VIOLET_FG,
            action: PaletteAction::CopyCli(
                build_alan_interrogation_prompt(
                    &self.selected_ticker,
                    selected_name,
                    selected_bucket,
                ),
                format!("Alan Prompt {}", self.selected_ticker),
            ),
        });
        items.push(PaletteItem {
            group: "CLI".into(),
            title: format!(
                "复制 CLI: ./apps/m3-desktop/m3 report {}",
                self.selected_ticker
            ),
            subtitle: "写入系统剪贴板".into(),
            shortcut: Some("c"),
            dot_color: INK_SECONDARY,
            action: PaletteAction::CopyCli(
                format!("./apps/m3-desktop/m3 report {}", self.selected_ticker),
                format!("report {}", self.selected_ticker),
            ),
        });
        items.push(PaletteItem {
            group: "DATA".into(),
            title: "同步 SEC EDGAR 财报、股价与 K 线历史".into(),
            subtitle: "更新一手 XBRL 与 5Y/1Y 行情".into(),
            shortcut: Some("⌘R"),
            dot_color: EMERALD_FG,
            action: PaletteAction::SyncNow,
        });
        items.push(PaletteItem {
            group: "DATA".into(),
            title: "追平事件流 · L1 SEC Filings / News / 语义索引".into(),
            subtitle: "增量拉取 SEC 公告与新闻，生成新浮现/传导/击球信号（0 Flagship Token）"
                .into(),
            shortcut: Some("r"),
            dot_color: EMERALD_FG,
            action: PaletteAction::CatchupEvents,
        });
        items.push(PaletteItem {
            group: "DATA".into(),
            title: "同步全美股 L0 湖仓 (SEC Bulk + 价格分层)".into(),
            subtitle: "下载 companyfacts.zip 并计算 5,000+ 美股质量评分与 Top 400 报价".into(),
            shortcut: None,
            dot_color: SKY_FG,
            action: PaletteAction::SyncUniverseNow,
        });
        items.push(PaletteItem {
            group: "UPDATE".into(),
            title: format!(
                "检查 M3 更新（当前 v{} · {} 通道）",
                update::current_binary_version(),
                self.update_channel.as_str()
            ),
            subtitle: "查询 GitHub Release 清单并比对 SHA-256".into(),
            shortcut: None,
            dot_color: SKY_FG,
            action: PaletteAction::CheckUpdate,
        });
        if let Some(ver) = &self.update_applied_version {
            items.push(PaletteItem {
                group: "UPDATE".into(),
                title: format!("立即重启应用已安装的 M3 v{ver}"),
                subtitle: "重启新版本 M3.app 使更新生效".into(),
                shortcut: None,
                dot_color: EMERALD_FG,
                action: PaletteAction::RestartApp,
            });
        } else if let Some(notice) = &self.update_notice {
            items.push(PaletteItem {
                group: "UPDATE".into(),
                title: format!(
                    "安装更新 · v{} ({})",
                    notice.latest_version,
                    notice.channel.as_str()
                ),
                subtitle: "校验 SHA-256 并原子替换当前 M3.app 与 ~/.local/bin/m3".into(),
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
            title: format!("切换更新通道至 {}", target_channel.label_cn()),
            subtitle: match target_channel {
                ReleaseChannel::Beta => "跟随 main 分支每次合并自动发布的 rolling beta 构建",
                ReleaseChannel::Stable => "仅接收带 tag (vX.Y.Z) 的稳定发布版本",
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
                        format!(
                            "本月 AI 预算 ({})：已用 ${used} / ${limit}（剩余 ${rem} · Token {tok_used} / {} · 完成 {} / 拦截 {}）",
                            b.usage.month_utc,
                            b.config.monthly_token_limit,
                            b.usage.completed_runs,
                            b.usage.blocked_runs
                        ),
                        false,
                    ));
                } else {
                    self.banner = Some(("无法读取 AI 预算状态".into(), true));
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
                                                "输入标的、篮子 (IN/OUT/TOO HARD)、#标签 或命令…"
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
                                        .child("无匹配命令、标的或标签"),
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
