use super::{
    AgentMessageRole, AlanReplyTarget, IndustryInputFocus, MungerWorkbench, NavView, components::*,
    theme::*,
};
use crate::{
    alan_interrogator::{compose_feynman_quiz_turn, compose_industry_tutor_turn},
    budget::ThemeCluster,
    events,
    i18n::UiLang,
    industry::{self, IndustryProfile, IndustryStatus},
    industry_research::{self, IndustryResearchEvent},
    lake::{LakeDb, UniverseRow},
    store::WorkspacePaths,
};
use gpui::{ClipboardItem, Context, div, prelude::*, px, rgb, rgba};
use rust_decimal::Decimal;
use std::sync::mpsc;

pub(super) fn industry_status_colors(status: IndustryStatus) -> (u32, u32, u32) {
    match status {
        IndustryStatus::InsideCircle => (EMERALD_FG, EMERALD_BG, EMERALD_BORDER),
        IndustryStatus::Studying => (SKY_FG, SKY_BG, SKY_BORDER),
        IndustryStatus::TooHard => (CORAL_FG, CORAL_BG, CORAL_BORDER),
    }
}

impl MungerWorkbench {
    pub(super) fn selected_industry(&self) -> Option<&IndustryProfile> {
        let slug = self.selected_industry_slug.as_deref()?;
        self.industries.iter().find(|p| p.slug == slug)
    }

    pub(super) fn reload_industries_from_disk(&mut self) {
        self.industries = industry::list_industries(&self.paths).unwrap_or_default();
        if let Some(cur) = self.selected_industry_slug.as_deref() {
            if !self.industries.iter().any(|p| p.slug == cur) {
                self.selected_industry_slug = self.industries.first().map(|p| p.slug.clone());
            }
        } else {
            self.selected_industry_slug = self.industries.first().map(|p| p.slug.clone());
        }
        self.reload_selected_industry_context();
    }

    pub(super) fn select_industry(&mut self, slug: &str) {
        self.selected_industry_slug = Some(slug.to_owned());
        self.industry_input_focus = IndustryInputFocus::None;
        self.industry_live_perspectives.clear();
        self.reload_selected_industry_context();
    }

    pub(super) fn reload_selected_industry_context(&mut self) {
        let Some(profile) = self.selected_industry().cloned() else {
            self.industry_peer_rows.clear();
            self.industry_primer_md = None;
            self.industry_human_note_draft.clear();
            return;
        };
        self.industry_human_note_draft = profile.human_notes.clone();
        self.industry_primer_md = industry::load_industry_primer_md(&self.paths, &profile.slug);

        if self.universe_syncing || self.inbox_catching_up {
            self.industry_peer_rows.clear();
            return;
        }
        self.industry_peer_rows =
            if let Ok(Some(db)) = LakeDb::open_read_only_if_exists(&self.paths) {
                db.load_universe_rows_by_tickers(&profile.bound_tickers)
                    .unwrap_or_default()
            } else {
                Vec::new()
            };
    }

    pub(super) fn open_or_create_industry_from_topic(
        &mut self,
        raw_topic: &str,
        cx: &mut Context<Self>,
    ) {
        let trimmed = raw_topic.trim();
        if trimmed.is_empty() {
            self.banner = Some((
                self.lang
                    .t(
                        "请输入行业名称或关键词再发起研究",
                        "Enter an industry topic first",
                    )
                    .into(),
                false,
            ));
            cx.notify();
            return;
        }
        match industry::create_or_open_from_topic(&self.paths, trimmed) {
            Ok(profile) => {
                self.industry_new_topic_input.clear();
                self.industry_input_focus = IndustryInputFocus::None;
                self.selected_industry_slug = Some(profile.slug.clone());
                self.reload_industries_from_disk();
                self.switch_view(NavView::Industry);
                let l = self.lang;
                let n = profile.bound_tickers.len();
                self.banner = Some((
                    match l {
                        UiLang::Zh => format!(
                            "已打开行业研究「{}」（自动匹配 {n} 家代表公司），可点击启动 Alan 4 视角深研",
                            profile.title
                        ),
                        UiLang::En => format!(
                            "Opened Industry Study \"{}\" (matched {n} peer tickers). Ready for Alan 4-Lens Research.",
                            profile.title
                        ),
                    },
                    false,
                ));
            }
            Err(err) => {
                self.banner = Some((
                    format!(
                        "{}: {err:#}",
                        self.lang.t("创建行业研究失败", "Failed to create study")
                    ),
                    true,
                ));
            }
        }
        cx.notify();
    }

    pub(super) fn open_or_create_industry_for_company(
        &mut self,
        ticker: &str,
        company_name: &str,
        sic: Option<u32>,
        sic_desc: Option<&str>,
        cx: &mut Context<Self>,
    ) {
        match industry::create_or_open_from_company(
            &self.paths,
            ticker,
            company_name,
            sic,
            sic_desc,
        ) {
            Ok(profile) => {
                self.selected_industry_slug = Some(profile.slug.clone());
                self.reload_industries_from_disk();
                self.switch_view(NavView::Industry);
                let l = self.lang;
                self.banner = Some((
                    match l {
                        UiLang::Zh => format!(
                            "已打开 {ticker} 所在行业「{}」（已绑定 {} 家代表公司）",
                            profile.title,
                            profile.bound_tickers.len()
                        ),
                        UiLang::En => format!(
                            "Opened industry \"{}\" for {ticker} ({} representative peers bound)",
                            profile.title,
                            profile.bound_tickers.len()
                        ),
                    },
                    false,
                ));
            }
            Err(err) => {
                self.banner = Some((
                    format!(
                        "{}: {err:#}",
                        self.lang
                            .t("打开行业研究失败", "Failed to open industry study")
                    ),
                    true,
                ));
            }
        }
        cx.notify();
    }

    pub(super) fn open_or_create_industry_from_theme(
        &mut self,
        theme: &ThemeCluster,
        cx: &mut Context<Self>,
    ) {
        match industry::create_or_open_from_theme(&self.paths, theme) {
            Ok(profile) => {
                self.selected_industry_slug = Some(profile.slug.clone());
                self.reload_industries_from_disk();
                self.switch_view(NavView::Industry);
                let l = self.lang;
                self.banner = Some((
                    match l {
                        UiLang::Zh => format!(
                            "已将主题「{}」升级为独立行业研究项目（{} 家代表公司）",
                            profile.title,
                            profile.bound_tickers.len()
                        ),
                        UiLang::En => format!(
                            "Promoted theme \"{}\" to Industry Study ({} bound companies)",
                            profile.title,
                            profile.bound_tickers.len()
                        ),
                    },
                    false,
                ));
            }
            Err(err) => {
                self.banner = Some((
                    format!(
                        "{}: {err:#}",
                        self.lang.t("转换主题失败", "Failed to promote theme")
                    ),
                    true,
                ));
            }
        }
        cx.notify();
    }

    pub(super) fn handle_industry_input_submit(&mut self, cx: &mut Context<Self>) {
        match self.industry_input_focus {
            IndustryInputFocus::NewTopic => {
                let topic = self.industry_new_topic_input.clone();
                self.open_or_create_industry_from_topic(&topic, cx);
            }
            IndustryInputFocus::AddTicker => {
                let ticker = self.industry_add_ticker_input.clone();
                self.add_ticker_to_selected_industry(&ticker, cx);
            }
            IndustryInputFocus::AlanQuestion => {
                let q = self.industry_alan_input.clone();
                self.submit_industry_alan_question(&q, cx);
            }
            IndustryInputFocus::HumanNote => {
                let note = self.industry_human_note_draft.clone();
                self.save_selected_industry_human_notes(&note, cx);
                self.industry_input_focus = IndustryInputFocus::None;
            }
            IndustryInputFocus::None => {}
        }
    }

    fn set_selected_industry_status(&mut self, status: IndustryStatus, cx: &mut Context<Self>) {
        let Some(slug) = self.selected_industry_slug.clone() else {
            return;
        };
        match industry::set_industry_status(&self.paths, &slug, status) {
            Ok(updated) => {
                self.reload_industries_from_disk();
                let l = self.lang;
                let msg = match (status, l) {
                    (IndustryStatus::InsideCircle, UiLang::Zh) => format!(
                        "「{}」已毕业进入【能力圈内】· 下次事件流追平起，该行业公司将按您的估值锚进入击球区雷达",
                        updated.title
                    ),
                    (IndustryStatus::InsideCircle, UiLang::En) => format!(
                        "\"{}\" graduated to Inside Circle · Bound companies now qualify for Circle-gated Strike signals",
                        updated.title
                    ),
                    (IndustryStatus::TooHard, UiLang::Zh) => format!(
                        "已将「{}」标为 Too Hard · 系统将屏蔽该行业个股的击球区与量化候选提醒",
                        updated.title
                    ),
                    (IndustryStatus::TooHard, UiLang::En) => format!(
                        "Marked \"{}\" as Too Hard · Strike and discovery signals suppressed for this industry",
                        updated.title
                    ),
                    (IndustryStatus::Studying, UiLang::Zh) => {
                        format!("已将「{}」恢复为「学习中」状态", updated.title)
                    }
                    (IndustryStatus::Studying, UiLang::En) => {
                        format!("Set \"{}\" back to Studying", updated.title)
                    }
                };
                self.banner = Some((msg, false));
            }
            Err(err) => {
                self.banner = Some((format!("{err:#}"), true));
            }
        }
        cx.notify();
    }

    fn set_selected_industry_anchor(
        &mut self,
        low: Option<Decimal>,
        high: Option<Decimal>,
        cx: &mut Context<Self>,
    ) {
        let Some(slug) = self.selected_industry_slug.clone() else {
            return;
        };
        match industry::set_industry_valuation_anchor(&self.paths, &slug, low, high) {
            Ok(updated) => {
                self.reload_industries_from_disk();
                let lbl = updated
                    .valuation_anchor_label()
                    .unwrap_or_else(|| self.lang.t("已清除", "Cleared").to_owned());
                self.banner = Some((
                    match self.lang {
                        UiLang::Zh => format!("已保存「{}」人类估值锚：{lbl}", updated.title),
                        UiLang::En => {
                            format!("Saved valuation anchor for \"{}\": {lbl}", updated.title)
                        }
                    },
                    false,
                ));
            }
            Err(err) => {
                self.banner = Some((format!("{err:#}"), true));
            }
        }
        cx.notify();
    }

    fn add_ticker_to_selected_industry(&mut self, raw_ticker: &str, cx: &mut Context<Self>) {
        let Some(slug) = self.selected_industry_slug.clone() else {
            return;
        };
        let trimmed = raw_ticker.trim();
        if trimmed.is_empty() {
            return;
        }
        match industry::add_bound_ticker(&self.paths, &slug, trimmed) {
            Ok(_) => {
                self.industry_add_ticker_input.clear();
                self.industry_input_focus = IndustryInputFocus::None;
                self.reload_industries_from_disk();
            }
            Err(err) => {
                self.banner = Some((format!("{err:#}"), true));
            }
        }
        cx.notify();
    }

    fn remove_ticker_from_selected_industry(&mut self, ticker: &str, cx: &mut Context<Self>) {
        let Some(slug) = self.selected_industry_slug.clone() else {
            return;
        };
        if let Err(err) = industry::remove_bound_ticker(&self.paths, &slug, ticker) {
            self.banner = Some((format!("{err:#}"), true));
        } else {
            self.reload_industries_from_disk();
        }
        cx.notify();
    }

    fn toggle_selected_industry_feynman(&mut self, idx: usize, cx: &mut Context<Self>) {
        let Some(slug) = self.selected_industry_slug.clone() else {
            return;
        };
        if let Err(err) = industry::toggle_feynman_check(&self.paths, &slug, idx) {
            self.banner = Some((format!("{err:#}"), true));
        } else {
            self.reload_industries_from_disk();
        }
        cx.notify();
    }

    fn save_selected_industry_human_notes(&mut self, note: &str, cx: &mut Context<Self>) {
        let Some(slug) = self.selected_industry_slug.clone() else {
            return;
        };
        match industry::save_human_notes(&self.paths, &slug, note) {
            Ok(_) => {
                self.reload_industries_from_disk();
                self.banner = Some((
                    self.lang
                        .t(
                            "已保存人类行业笔记（同步受保护的 ## Human Notes）",
                            "Saved human industry notes (protected ## Human Notes synced)",
                        )
                        .into(),
                    false,
                ));
            }
            Err(err) => {
                self.banner = Some((format!("{err:#}"), true));
            }
        }
        cx.notify();
    }

    pub(super) fn start_industry_deep_research(&mut self, cx: &mut Context<Self>) {
        if self.industry_research_running || self.deep_research_running {
            self.banner = Some((
                self.lang
                    .t(
                        "已有 Alan 深研任务正在运行，请稍候",
                        "Another Alan research run is active",
                    )
                    .into(),
                false,
            ));
            cx.notify();
            return;
        }
        let Some(profile) = self.selected_industry().cloned() else {
            return;
        };
        if self.universe_syncing || self.inbox_catching_up {
            self.banner = Some((
                self.lang
                    .t(
                        "后台湖仓同步中，请稍候完成后再启动行业深研",
                        "Wait for lakehouse sync to finish before starting industry research",
                    )
                    .into(),
                false,
            ));
            cx.notify();
            return;
        }

        self.industry_research_running = true;
        self.industry_research_slug = Some(profile.slug.clone());
        self.industry_live_perspectives.clear();
        self.industry_research_progress = Some(match self.lang {
            UiLang::Zh => format!("正在准备「{}」4 视角行业深研…", profile.title),
            UiLang::En => format!("Preparing 4-lens research for \"{}\"…", profile.title),
        });

        let root = self.paths.repo_root.clone();
        let slug = profile.slug.clone();
        let (tx, rx) = mpsc::channel();
        self.industry_research_rx = Some(rx);

        std::thread::spawn(move || {
            let paths = WorkspacePaths::from_root(root);
            if let Err(err) =
                industry_research::run_industry_deep_research(&paths, &slug, None, Some(tx.clone()))
            {
                let _ = tx.send(IndustryResearchEvent::Failed {
                    run_id: String::new(),
                    slug,
                    error: format!("{err:#}"),
                });
            }
        });

        cx.notify();
    }

    pub(super) fn start_industry_batch_10k_fetch(&mut self, cx: &mut Context<Self>) {
        if self.industry_batch_10k_running || self.universe_syncing || self.inbox_catching_up {
            return;
        }
        let Some(_profile) = self.selected_industry().cloned() else {
            return;
        };
        let targets: Vec<(String, String)> = self
            .industry_peer_rows
            .iter()
            .take(5)
            .map(|r| (r.ticker.clone(), r.cik.clone()))
            .collect();
        if targets.is_empty() {
            self.banner = Some((
                self.lang
                    .t(
                        "代表公司尚未在本地 L0 湖仓找到 CIK，请先同步全美股 L0",
                        "Bound companies not yet in L0 lakehouse — sync L0 first",
                    )
                    .into(),
                false,
            ));
            cx.notify();
            return;
        }

        self.industry_batch_10k_running = true;
        let count = targets.len();
        self.banner = Some((
            match self.lang {
                UiLang::Zh => format!(
                    "正在后台拉取 {count} 家代表公司最近 SEC 10-K 原文（0 Flagship Token）…"
                ),
                UiLang::En => format!(
                    "Fetching latest SEC 10-K Item 1/1A for {count} peers (0 Flagship Tokens)…"
                ),
            },
            false,
        ));
        let root = self.paths.repo_root.clone();
        let (tx, rx) = mpsc::channel();
        self.industry_batch_10k_rx = Some(rx);

        std::thread::spawn(move || {
            let paths = WorkspacePaths::from_root(root);
            let mut ok_tickers = Vec::new();
            let mut total_chunks = 0usize;
            for (ticker, cik) in targets {
                if let Ok(rep) = events::fetch_company_10k_profile(&paths, &ticker, &cik) {
                    ok_tickers.push(rep.ticker);
                    total_chunks += rep.chunks_indexed;
                }
            }
            let _ = tx.send(Ok(format!("{}|{}", ok_tickers.join(", "), total_chunks)));
        });
        cx.notify();
    }

    pub(super) fn poll_industry_batch_10k(&mut self) -> bool {
        let Some(rx) = self.industry_batch_10k_rx.take() else {
            return false;
        };
        match rx.try_recv() {
            Ok(Ok(summary)) => {
                self.industry_batch_10k_running = false;
                let (tickers, chunks) = summary.split_once('|').unwrap_or((&summary, "0"));
                self.banner = Some((
                    match self.lang {
                        UiLang::Zh => {
                            format!("代表公司 10-K 原文同步完成：{tickers}（新增 {chunks} 条片段）")
                        }
                        UiLang::En => {
                            format!("Fetched 10-K profiles for {tickers} (+{chunks} chunks)")
                        }
                    },
                    false,
                ));
                self.reload_selected_industry_context();
                true
            }
            Ok(Err(err)) => {
                self.industry_batch_10k_running = false;
                self.banner = Some((err, true));
                true
            }
            Err(mpsc::TryRecvError::Empty) => {
                self.industry_batch_10k_rx = Some(rx);
                false
            }
            Err(mpsc::TryRecvError::Disconnected) => {
                self.industry_batch_10k_running = false;
                true
            }
        }
    }

    pub(super) fn poll_industry_research(&mut self) -> bool {
        let Some(rx) = self.industry_research_rx.take() else {
            return false;
        };
        let mut events = Vec::new();
        let mut disconnected = false;
        loop {
            match rx.try_recv() {
                Ok(ev) => events.push(ev),
                Err(mpsc::TryRecvError::Empty) => {
                    self.industry_research_rx = Some(rx);
                    break;
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    disconnected = true;
                    break;
                }
            }
        }
        if events.is_empty() {
            if disconnected && self.industry_research_running {
                self.industry_research_running = false;
                return true;
            }
            return false;
        }

        let mut reload = false;
        for ev in events {
            match ev {
                IndustryResearchEvent::Started { slug, .. } => {
                    self.industry_research_progress = Some(match self.lang {
                        UiLang::Zh => {
                            format!("正在检查/拉取「{slug}」代表公司 10-K 与启动 4 视角 Alan…")
                        }
                        UiLang::En => {
                            format!("Bootstrapping 10-K filings & Alan 4-lens for \"{slug}\"…")
                        }
                    });
                }
                IndustryResearchEvent::Fetching10K { ticker, idx, total } => {
                    self.industry_research_progress = Some(match self.lang {
                        UiLang::Zh => {
                            format!("正在拉取代表公司 10-K（{idx}/{total}）：{ticker} (0 LLM)…")
                        }
                        UiLang::En => {
                            format!("Fetching peer 10-K ({idx}/{total}): {ticker} (0 LLM)…")
                        }
                    });
                }
                IndustryResearchEvent::PerspectiveStarted { perspective, label } => {
                    self.industry_live_perspectives
                        .entry(perspective)
                        .or_default();
                    self.industry_research_progress = Some(match self.lang {
                        UiLang::Zh => format!("Alan 视角运行中：{label}…"),
                        UiLang::En => format!("Alan lens running: {label}…"),
                    });
                }
                IndustryResearchEvent::PerspectiveDelta { perspective, chunk } => {
                    self.industry_live_perspectives
                        .entry(perspective)
                        .or_default()
                        .push_str(&chunk);
                }
                IndustryResearchEvent::ToolStarted {
                    perspective: _,
                    name,
                    title,
                } => {
                    self.industry_research_progress = Some(format!("{name}: {title}"));
                }
                IndustryResearchEvent::PerspectiveFinished { .. } => {}
                IndustryResearchEvent::Completed(outcome) => {
                    self.industry_research_running = false;
                    self.industry_research_progress = None;
                    reload = true;
                    let cost = outcome.cost_usd_str;
                    self.banner = Some((
                        match self.lang {
                            UiLang::Zh => format!(
                                "行业深研完成（拉取 {} 家 10-K · {} in / {} out · ${cost}）· 已写入活手册",
                                outcome.sec_10k_fetched_count,
                                outcome.input_tokens,
                                outcome.output_tokens,
                            ),
                            UiLang::En => format!(
                                "Industry research complete ({} 10-Ks fetched · {} in / {} out · ${cost})",
                                outcome.sec_10k_fetched_count,
                                outcome.input_tokens,
                                outcome.output_tokens,
                            ),
                        },
                        false,
                    ));
                }
                IndustryResearchEvent::Failed { error, .. } => {
                    self.industry_research_running = false;
                    self.industry_research_progress = None;
                    self.banner = Some((error, true));
                }
            }
        }
        if reload {
            self.reload_industries_from_disk();
            self.reload_budget_from_lake();
        }
        true
    }

    pub(super) fn submit_industry_alan_question(
        &mut self,
        raw_question: &str,
        cx: &mut Context<Self>,
    ) {
        let question = raw_question.trim().to_owned();
        if question.is_empty() {
            return;
        }
        let Some(profile) = self.selected_industry().cloned() else {
            return;
        };
        let primer = self.industry_primer_md.as_deref().unwrap_or("");
        let full_prompt = compose_industry_tutor_turn(&profile, primer, &question);

        self.industry_alan_input.clear();
        self.industry_input_focus = IndustryInputFocus::None;
        self.alan_reply_target = AlanReplyTarget::Industry(profile.slug.clone());
        self.industry_alan_streaming = true;
        self.industry_alan_status_hint = Some(
            self.lang
                .t("Alan 行业导师思考中…", "Alan industry tutor thinking…")
                .to_owned(),
        );

        let user_seq = self.next_agent_seq();
        let asst_seq = self.next_agent_seq();
        self.industry_alan_messages.push(super::AgentUiMessage {
            id: format!("ind-u-{user_seq}"),
            role: AgentMessageRole::User,
            raw_text: question.clone(),
            text: question.clone(),
            tokens: Vec::new(),
            streaming: false,
        });
        self.industry_alan_messages.push(super::AgentUiMessage {
            id: format!("ind-a-{asst_seq}"),
            role: AgentMessageRole::Assistant,
            raw_text: String::new(),
            text: String::new(),
            tokens: Vec::new(),
            streaming: true,
        });

        let handle = self.ensure_alan_handle();
        handle.submit_prompt(question, full_prompt);
        cx.notify();
    }

    pub(super) fn trigger_industry_feynman_quiz(&mut self, cx: &mut Context<Self>) {
        let Some(profile) = self.selected_industry().cloned() else {
            return;
        };
        let primer = self.industry_primer_md.as_deref().unwrap_or("");
        let full_prompt = compose_feynman_quiz_turn(&profile, primer);
        let display = match self.lang {
            UiLang::Zh => format!("对「{}」发起费曼通关自测（4 道尖锐追问）", profile.title),
            UiLang::En => format!(
                "Start Feynman Mastery Quiz for \"{}\" (4 sharp questions)",
                profile.title
            ),
        };

        self.alan_reply_target = AlanReplyTarget::Industry(profile.slug.clone());
        self.industry_alan_streaming = true;
        self.industry_alan_status_hint = Some(
            self.lang
                .t("正在生成费曼通关考题…", "Generating Feynman quiz…")
                .to_owned(),
        );

        let user_seq = self.next_agent_seq();
        let asst_seq = self.next_agent_seq();
        self.industry_alan_messages.push(super::AgentUiMessage {
            id: format!("ind-fq-u-{user_seq}"),
            role: AgentMessageRole::User,
            raw_text: display.clone(),
            text: display.clone(),
            tokens: Vec::new(),
            streaming: false,
        });
        self.industry_alan_messages.push(super::AgentUiMessage {
            id: format!("ind-fq-a-{asst_seq}"),
            role: AgentMessageRole::Assistant,
            raw_text: String::new(),
            text: String::new(),
            tokens: Vec::new(),
            streaming: true,
        });

        let handle = self.ensure_alan_handle();
        handle.submit_prompt(display, full_prompt);
        cx.notify();
    }

    pub(super) fn render_industry_view(&self, cx: &mut Context<Self>) -> gpui::Div {
        let l = self.lang;
        div()
            .flex_1()
            .min_h_0()
            .w_full()
            .flex()
            .flex_row()
            .child(self.render_industry_left_rail(cx))
            .child(
                div()
                    .id("industry-right-pane")
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .overflow_y_scroll()
                    .p_5()
                    .flex()
                    .flex_col()
                    .gap_4()
                    .when_some(self.selected_industry().cloned(), |d, profile| {
                        d.child(self.render_industry_workspace(&profile, cx))
                    })
                    .when(self.selected_industry().is_none(), |d| {
                        d.child(self.render_industry_empty_guide(l))
                    }),
            )
    }

    fn render_industry_left_rail(&self, cx: &mut Context<Self>) -> gpui::Stateful<gpui::Div> {
        let l = self.lang;
        let filtered: Vec<&IndustryProfile> = self
            .industries
            .iter()
            .filter(|p| self.industry_status_filter.is_none_or(|st| p.status == st))
            .collect();

        let status_filters: &[(Option<IndustryStatus>, &str)] = &[
            (None, l.t("全部", "All")),
            (Some(IndustryStatus::Studying), l.t("学习中", "Studying")),
            (
                Some(IndustryStatus::InsideCircle),
                l.t("能力圈内", "Inside Circle"),
            ),
            (Some(IndustryStatus::TooHard), "Too Hard"),
        ];

        let new_focused = self.industry_input_focus == IndustryInputFocus::NewTopic;
        let new_text = if self.industry_new_topic_input.is_empty() {
            l.t(
                "输入新行业/主题（如 EDA软件、连锁诊所）…",
                "New industry topic (e.g. EDA, Dialysis)…",
            )
            .to_owned()
        } else {
            self.industry_new_topic_input.clone()
        };

        div()
            .id("industry-left-rail")
            .w(px(320.))
            .flex_shrink_0()
            .h_full()
            .border_r_1()
            .border_color(rgb(BORDER_HAIRLINE))
            .bg(rgb(BG_SURFACE))
            .overflow_y_scroll()
            .p_3()
            .flex()
            .flex_col()
            .gap_3()
            .child(
                card_shell()
                    .w_full()
                    .p_3()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(
                        div()
                            .font_family(self.fonts.ui.clone())
                            .text_xs()
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .text_color(rgb(INK_HERO))
                            .child(l.t("+ 新建行业研究（自由主题）", "+ New Industry Study (Topic)")),
                    )
                    .child(
                        div()
                            .id("ind-new-topic-input")
                            .w_full()
                            .px_2p5()
                            .py_1p5()
                            .rounded_sm()
                            .cursor_text()
                            .bg(rgb(BG_WELL))
                            .border_1()
                            .border_color(rgb(if new_focused {
                                VIOLET_FG
                            } else {
                                BORDER_SUBTLE
                            }))
                            .font_family(self.fonts.ui.clone())
                            .text_size(px(12.))
                            .text_color(rgb(if self.industry_new_topic_input.is_empty() {
                                INK_MUTED
                            } else {
                                INK_PRIMARY
                            }))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.industry_input_focus = IndustryInputFocus::NewTopic;
                                cx.notify();
                            }))
                            .child(new_text),
                    )
                    .child(
                        div()
                            .id("ind-new-topic-btn")
                            .w_full()
                            .px_3()
                            .py_1p5()
                            .rounded_sm()
                            .cursor_pointer()
                            .bg(rgb(CTA_BG))
                            .hover(|s| s.opacity(0.9))
                            .font_family(self.fonts.ui.clone())
                            .text_xs()
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .text_color(rgb(CTA_INK))
                            .flex()
                            .items_center()
                            .justify_center()
                            .on_click(cx.listener(|this, _, _, cx| {
                                let t = this.industry_new_topic_input.clone();
                                this.open_or_create_industry_from_topic(&t, cx);
                            }))
                            .child(l.t("自动匹配代表公司并创建", "Match Peers & Start Study")),
                    ),
            )
            .child(
                div().flex().items_center().flex_wrap().gap_1p5().children(
                    status_filters.iter().enumerate().map(|(idx, (st_opt, label))| {
                        let active = self.industry_status_filter == *st_opt;
                        let st_copy = *st_opt;
                        div()
                            .id(gpui::SharedString::from(format!("ind-flt-{idx}")))
                            .px_2()
                            .py_1()
                            .rounded_sm()
                            .cursor_pointer()
                            .bg(rgb(if active { BG_HOVER } else { BG_WELL }))
                            .border_1()
                            .border_color(rgb(if active {
                                BORDER_PILL
                            } else {
                                BORDER_HAIRLINE
                            }))
                            .font_family(self.fonts.ui.clone())
                            .text_size(px(11.))
                            .text_color(rgb(if active {
                                INK_HERO
                            } else {
                                INK_SECONDARY
                            }))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.industry_status_filter = st_copy;
                                cx.notify();
                            }))
                            .child(*label)
                    }),
                ),
            )
            .child(
                div().flex().flex_col().gap_1p5().children(if filtered.is_empty() {
                    vec![
                        div()
                            .p_3()
                            .rounded_md()
                            .bg(rgb(BG_WELL))
                            .border_1()
                            .border_color(rgb(BORDER_HAIRLINE))
                            .font_family(self.fonts.ui.clone())
                            .text_xs()
                            .text_color(rgb(INK_MUTED))
                            .child(l.t(
                                "暂无行业研究项目。输入上方主题，或在收件箱/雷达中点击「研究所在行业」发起。",
                                "No industry studies yet. Enter a topic above or click 'Research Industry' from Inbox/Radar.",
                            ))
                            .into_any_element(),
                    ]
                } else {
                    filtered
                        .into_iter()
                        .map(|ind| {
                            let selected =
                                self.selected_industry_slug.as_deref() == Some(ind.slug.as_str());
                            let (fg, bg, br) = industry_status_colors(ind.status);
                            let slug = ind.slug.clone();
                            let anchor = ind.valuation_anchor_label();
                            div()
                                .id(gpui::SharedString::from(format!("ind-row-{}", ind.slug)))
                                .w_full()
                                .p_2p5()
                                .rounded_md()
                                .cursor_pointer()
                                .bg(rgb(if selected { BG_HOVER } else { BG_VIEWPORT }))
                                .border_1()
                                .border_color(rgb(if selected {
                                    BORDER_PILL
                                } else {
                                    BORDER_HAIRLINE
                                }))
                                .hover(|s| s.bg(rgb(BG_HOVER)))
                                .flex()
                                .flex_col()
                                .gap_1p5()
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.select_industry(&slug);
                                    cx.notify();
                                }))
                                .child(
                                    div()
                                        .w_full()
                                        .flex()
                                        .items_center()
                                        .justify_between()
                                        .gap_2()
                                        .child(
                                            div()
                                                .flex_1()
                                                .min_w_0()
                                                .font_family(self.fonts.ui.clone())
                                                .text_xs()
                                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                                .text_color(rgb(INK_HERO))
                                                .truncate()
                                                .child(ind.title.clone()),
                                        )
                                        .child(badge(
                                            ind.status.label_for(l).into(),
                                            fg,
                                            bg,
                                            br,
                                            &self.fonts,
                                        )),
                                )
                                .child(
                                    div()
                                        .w_full()
                                        .flex()
                                        .items_center()
                                        .justify_between()
                                        .gap_2()
                                        .child(
                                            div()
                                                .font_family(self.fonts.mono_num.clone())
                                                .text_size(px(10.))
                                                .text_color(rgb(INK_MUTED))
                                                .child(match l {
                                                    UiLang::Zh => format!(
                                                        "{} 家代表公司",
                                                        ind.bound_tickers.len()
                                                    ),
                                                    UiLang::En => {
                                                        format!("{} peers", ind.bound_tickers.len())
                                                    }
                                                }),
                                        )
                                        .when_some(anchor, |d, a| {
                                            d.child(
                                                div()
                                                    .font_family(self.fonts.mono_num.clone())
                                                    .text_size(px(10.))
                                                    .text_color(rgb(EMERALD_FG))
                                                    .child(a),
                                            )
                                        }),
                                )
                                .into_any_element()
                        })
                        .collect()
                }),
            )
            .child(self.render_themes_promotion_rail(cx))
    }

    fn render_themes_promotion_rail(&self, cx: &mut Context<Self>) -> gpui::Div {
        let l = self.lang;
        div()
            .pt_2()
            .border_t_1()
            .border_color(rgb(BORDER_HAIRLINE))
            .flex()
            .flex_col()
            .gap_2()
            .child(
                div()
                    .w_full()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_2()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .font_family(self.fonts.ui.clone())
                            .text_size(px(11.))
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .text_color(rgb(INK_SECONDARY))
                            .child(l.t("SIC / 主题聚类快速发起", "Promote SIC / Theme Clusters")),
                    )
                    .child(
                        div()
                            .id("ind-sync-themes-btn")
                            .flex_shrink_0()
                            .px_2()
                            .py_0p5()
                            .rounded_xs()
                            .cursor_pointer()
                            .bg(rgb(BG_WELL))
                            .border_1()
                            .border_color(rgb(BORDER_SUBTLE))
                            .hover(|s| s.bg(rgb(BG_HOVER)))
                            .font_family(self.fonts.ui.clone())
                            .text_size(px(10.))
                            .text_color(rgb(SKY_FG))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.start_themes_sync(cx);
                            }))
                            .child(if self.themes_syncing {
                                l.t("同步中…", "Syncing…")
                            } else {
                                l.t("同步聚类 s", "Sync s")
                            }),
                    ),
            )
            .children(self.themes_file.clusters.iter().take(6).map(|cluster| {
                let cluster_clone = cluster.clone();
                div()
                    .id(gpui::SharedString::from(format!(
                        "ind-promote-theme-{}",
                        cluster.theme_id
                    )))
                    .w_full()
                    .px_2p5()
                    .py_1p5()
                    .rounded_sm()
                    .cursor_pointer()
                    .bg(rgb(BG_WELL))
                    .border_1()
                    .border_color(rgb(BORDER_HAIRLINE))
                    .hover(|s| s.bg(rgb(BG_HOVER)))
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_2()
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.open_or_create_industry_from_theme(&cluster_clone, cx);
                    }))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .font_family(self.fonts.ui.clone())
                            .text_size(px(11.))
                            .text_color(rgb(INK_PRIMARY))
                            .truncate()
                            .child(cluster.title.clone()),
                    )
                    .child(
                        div()
                            .flex_shrink_0()
                            .font_family(self.fonts.ui.clone())
                            .text_size(px(10.))
                            .text_color(rgb(SKY_FG))
                            .child(l.t("研究 →", "Study →")),
                    )
            }))
    }

    fn render_industry_empty_guide(&self, l: UiLang) -> gpui::Div {
        card_shell()
            .w_full()
            .p_6()
            .flex()
            .flex_col()
            .gap_3()
            .child(
                div()
                    .font_family(self.fonts.ui.clone())
                    .text_base()
                    .font_weight(gpui::FontWeight::BOLD)
                    .text_color(rgb(INK_HERO))
                    .child(l.t(
                        "行业研究工作台 · 从 Too Hard 扩展你的能力圈",
                        "Industry Research Studio · Expand Your Circle Beyond Too Hard",
                    )),
            )
            .child(
                div()
                    .font_family(self.fonts.ui.clone())
                    .text_size(px(13.))
                    .line_height(px(21.))
                    .text_color(rgb(INK_SECONDARY))
                    .child(l.t(
                        "未研究过的行业不应盲目给出机械击球价。你可以通过三种方式启动行业研究：(1) 在左侧输入任意主题；(2) 在「简报收件箱」或「全市场雷达」选中任意公司点击「研究所在行业」；(3) 从左下角的 SIC/主题聚类一键发起。\n\nAlan 会自动结合代表公司 10-K 原文、DuckDB 5 年财务矩阵与公开网页资料完成 4 视角研究并生成费曼自测题。唯有当你真正读懂该行业、手动设好估值锚并点击「毕业进能力圈」后，该行业标的才会被允许触发击球区雷达信号。",
                        "Unknown industries never receive blind mechanical Strike alerts. Start an Industry Study in 3 ways: (1) enter a free-form topic on the left, (2) click 'Research Industry' from any company in Inbox or Radar, or (3) promote a SIC cluster below.\n\nAlan combines representative SEC 10-Ks, the Zero-LLM DuckDB 5Y Peer Matrix, and bounded public web search with strict source separation. Only after you pass the Feynman check, set your human valuation anchor, and graduate an industry to Inside Circle will its companies qualify for Strike signals.",
                    )),
            )
    }

    fn render_industry_workspace(
        &self,
        profile: &IndustryProfile,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        div()
            .w_full()
            .flex()
            .flex_col()
            .gap_4()
            .child(self.render_industry_header_card(profile, cx))
            .child(self.render_industry_sovereignty_card(profile, cx))
            .child(self.render_industry_peer_matrix_card(profile, cx))
            .child(self.render_industry_primer_card(profile, cx))
            .child(self.render_industry_alan_tutor_card(profile, cx))
    }

    fn render_industry_header_card(
        &self,
        profile: &IndustryProfile,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        let l = self.lang;
        let (st_fg, st_bg, st_br) = industry_status_colors(profile.status);
        let sic_text = if profile.sic_codes.is_empty() {
            l.t("自定义主题池", "Custom Archetype Pool").to_owned()
        } else {
            format!(
                "SIC {}",
                profile
                    .sic_codes
                    .iter()
                    .map(u32::to_string)
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        };
        let is_running_this = self.industry_research_running
            && self.industry_research_slug.as_deref() == Some(profile.slug.as_str());

        card_shell()
            .w_full()
            .p_5()
            .flex()
            .flex_col()
            .gap_3()
            .child(
                div()
                    .w_full()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_3()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .items_center()
                            .flex_wrap()
                            .gap_2()
                            .child(badge(
                                profile.status.label_for(l).into(),
                                st_fg,
                                st_bg,
                                st_br,
                                &self.fonts,
                            ))
                            .child(
                                div()
                                    .font_family(self.fonts.ui.clone())
                                    .text_lg()
                                    .font_weight(gpui::FontWeight::BOLD)
                                    .text_color(rgb(INK_HERO))
                                    .child(profile.title.clone()),
                            )
                            .child(
                                div()
                                    .font_family(self.fonts.mono_num.clone())
                                    .text_xs()
                                    .text_color(rgb(INK_MUTED))
                                    .child(sic_text),
                            ),
                    )
                    .child(
                        div()
                            .id("ind-run-alan-btn")
                            .flex_shrink_0()
                            .whitespace_nowrap()
                            .px_3p5()
                            .py_2()
                            .rounded_sm()
                            .cursor_pointer()
                            .bg(rgba(VIOLET_BG))
                            .border_1()
                            .border_color(rgba(VIOLET_BORDER))
                            .hover(|s| s.bg(rgb(BG_HOVER)))
                            .font_family(self.fonts.ui.clone())
                            .text_xs()
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .text_color(rgb(VIOLET_FG))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.start_industry_deep_research(cx);
                            }))
                            .child(if is_running_this {
                                l.t("Alan 4 视角深研中…", "Alan 4-Lens Running…")
                            } else {
                                l.t(
                                    "启动 Alan 4 视角行业深研 (含 10-K + Web)",
                                    "Run Alan 4-Lens Industry Research (10-K + Web)",
                                )
                            }),
                    ),
            )
            .when_some(
                is_running_this
                    .then_some(self.industry_research_progress.as_ref())
                    .flatten()
                    .cloned(),
                |d, prog| {
                    d.child(
                        div()
                            .w_full()
                            .px_3()
                            .py_2()
                            .rounded_sm()
                            .bg(rgba(VIOLET_BG))
                            .border_1()
                            .border_color(rgba(VIOLET_BORDER))
                            .font_family(self.fonts.mono_term.clone())
                            .text_xs()
                            .text_color(rgb(VIOLET_FG))
                            .child(prog),
                    )
                },
            )
            .child(
                div()
                    .w_full()
                    .pt_2()
                    .border_t_1()
                    .border_color(rgb(BORDER_HAIRLINE))
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_2()
                    .child(
                        div()
                            .font_family(self.fonts.ui.clone())
                            .text_xs()
                            .text_color(rgb(INK_SECONDARY))
                            .child(l.t(
                                "人类毕业裁决（仅你本人可改变行业能力圈状态）：",
                                "Human Graduation Decision (Sovereign Control):",
                            )),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(self.status_button(
                                "ind-st-studying",
                                l.t("学习中", "Studying"),
                                IndustryStatus::Studying,
                                profile.status == IndustryStatus::Studying,
                                cx,
                            ))
                            .child(self.status_button(
                                "ind-st-circle",
                                l.t("毕业进能力圈 ✓", "Graduate to Circle ✓"),
                                IndustryStatus::InsideCircle,
                                profile.status == IndustryStatus::InsideCircle,
                                cx,
                            ))
                            .child(self.status_button(
                                "ind-st-toohard",
                                l.t("标为 Too Hard", "Mark Too Hard"),
                                IndustryStatus::TooHard,
                                profile.status == IndustryStatus::TooHard,
                                cx,
                            )),
                    ),
            )
    }

    fn status_button(
        &self,
        id: &'static str,
        label: &'static str,
        status: IndustryStatus,
        active: bool,
        cx: &mut Context<Self>,
    ) -> gpui::Stateful<gpui::Div> {
        let (fg, bg, br) = industry_status_colors(status);
        div()
            .id(id)
            .flex_shrink_0()
            .whitespace_nowrap()
            .px_2p5()
            .py_1()
            .rounded_sm()
            .cursor_pointer()
            .bg(if active { rgba(bg) } else { rgb(BG_WELL) })
            .border_1()
            .border_color(if active { rgba(br) } else { rgb(BORDER_SUBTLE) })
            .hover(|s| s.bg(rgb(BG_HOVER)))
            .font_family(self.fonts.ui.clone())
            .text_xs()
            .font_weight(if active {
                gpui::FontWeight::SEMIBOLD
            } else {
                gpui::FontWeight::NORMAL
            })
            .text_color(rgb(if active { fg } else { INK_SECONDARY }))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.set_selected_industry_status(status, cx);
            }))
            .child(label)
    }

    fn render_industry_sovereignty_card(
        &self,
        profile: &IndustryProfile,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        let l = self.lang;
        let presets: &[(i64, i64, &str)] = &[
            (
                8,
                12,
                l.t("8–12x 重资产/强周期", "8–12x Cyclical/Asset-Heavy"),
            ),
            (12, 16, l.t("12–16x 稳健制造", "12–16x Steady Industrial")),
            (15, 18, l.t("15–18x 经典复利", "15–18x Classic Compounder")),
            (18, 24, l.t("18–24x 软件/特许", "18–24x Software/Franchise")),
            (22, 28, l.t("22–28x 顶级垄断", "22–28x Dominant Moat")),
        ];

        let add_focused = self.industry_input_focus == IndustryInputFocus::AddTicker;
        let note_focused = self.industry_input_focus == IndustryInputFocus::HumanNote;
        let note_display = if self.industry_human_note_draft.is_empty() {
            l.t(
                "点击填写人类行业备忘与护城河边界（Alan 永不覆盖此区）…",
                "Click to type your Human Notes & moat boundaries (Alan never overwrites)…",
            )
            .to_owned()
        } else {
            self.industry_human_note_draft.clone()
        };

        card_shell()
            .w_full()
            .p_5()
            .flex()
            .flex_col()
            .gap_3p5()
            .child(
                div()
                    .w_full()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_2()
                    .child(
                        div()
                            .font_family(self.fonts.ui.clone())
                            .text_sm()
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .text_color(rgb(INK_HERO))
                            .child(l.t(
                                "人类主权区：行业估值锚、代表公司池与费曼通关清单",
                                "Human Sovereignty: Valuation Anchor, Peer Pool & Feynman Check",
                            )),
                    )
                    .child(badge(
                        l.t("仅人类可编辑", "HUMAN ONLY").into(),
                        GOLD_FG,
                        GOLD_BG,
                        GOLD_BORDER,
                        &self.fonts,
                    )),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1p5()
                    .child(
                        div()
                            .font_family(self.fonts.ui.clone())
                            .text_xs()
                            .text_color(rgb(INK_SECONDARY))
                            .child(match (l, profile.valuation_anchor_label()) {
                                (UiLang::Zh, Some(lbl)) => format!(
                                    "行业合理股东盈余倍数锚：{lbl}（毕业进能力圈后，将按此倍数下沿计算该行业标的击球价）"
                                ),
                                (UiLang::En, Some(lbl)) => format!(
                                    "Industry OE Multiple Anchor: {lbl} (used to compute Strike Price when Inside Circle)"
                                ),
                                (UiLang::Zh, None) => {
                                    "行业合理股东盈余倍数锚：尚未设定（默认 18x，请选择符合该行业商业本质的区间）："
                                        .to_owned()
                                }
                                (UiLang::En, None) => {
                                    "Industry OE Multiple Anchor: Unset (defaults to 18x; select a band matching unit economics):"
                                        .to_owned()
                                }
                            }),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .flex_wrap()
                            .gap_1p5()
                            .children(presets.iter().enumerate().map(|(i, (lo, hi, label))| {
                                let lo_d = Decimal::from(*lo);
                                let hi_d = Decimal::from(*hi);
                                let active = profile.multiple_low == Some(lo_d)
                                    && profile.multiple_high == Some(hi_d);
                                div()
                                    .id(gpui::SharedString::from(format!("ind-mult-{i}")))
                                    .px_2p5()
                                    .py_1()
                                    .rounded_sm()
                                    .cursor_pointer()
                                    .bg(if active {
                                        rgba(EMERALD_BG)
                                    } else {
                                        rgb(BG_WELL)
                                    })
                                    .border_1()
                                    .border_color(if active {
                                        rgba(EMERALD_BORDER)
                                    } else {
                                        rgb(BORDER_SUBTLE)
                                    })
                                    .hover(|s| s.bg(rgb(BG_HOVER)))
                                    .font_family(self.fonts.ui.clone())
                                    .text_xs()
                                    .text_color(rgb(if active {
                                        EMERALD_FG
                                    } else {
                                        INK_PRIMARY
                                    }))
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.set_selected_industry_anchor(
                                            Some(lo_d),
                                            Some(hi_d),
                                            cx,
                                        );
                                    }))
                                    .child(*label)
                            })),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1p5()
                    .child(
                        div()
                            .font_family(self.fonts.ui.clone())
                            .text_xs()
                            .text_color(rgb(INK_SECONDARY))
                            .child(l.t("绑定代表公司池（点击可删除或新增 Ticker）：", "Bound Peer Pool (click × to remove or add ticker):")),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .flex_wrap()
                            .gap_1p5()
                            .children(profile.bound_tickers.iter().map(|t| {
                                let tk = t.clone();
                                div()
                                    .id(gpui::SharedString::from(format!("ind-chip-{tk}")))
                                    .px_2()
                                    .py_1()
                                    .rounded_xs()
                                    .bg(rgb(BG_WELL))
                                    .border_1()
                                    .border_color(rgb(BORDER_SUBTLE))
                                    .flex()
                                    .items_center()
                                    .gap_1p5()
                                    .child(
                                        div()
                                            .font_family(self.fonts.mono_num.clone())
                                            .text_xs()
                                            .font_weight(gpui::FontWeight::SEMIBOLD)
                                            .text_color(rgb(INK_HERO))
                                            .child(tk.clone()),
                                    )
                                    .child(
                                        div()
                                            .id(gpui::SharedString::from(format!(
                                                "ind-chip-rm-{tk}"
                                            )))
                                            .cursor_pointer()
                                            .font_family(self.fonts.mono_term.clone())
                                            .text_xs()
                                            .text_color(rgb(INK_MUTED))
                                            .hover(|s| s.text_color(rgb(CORAL_FG)))
                                            .on_click(cx.listener(move |this, _, _, cx| {
                                                this.remove_ticker_from_selected_industry(&tk, cx);
                                            }))
                                            .child("×"),
                                    )
                            }))
                            .child(
                                div()
                                    .id("ind-add-ticker-input")
                                    .w(px(130.))
                                    .px_2()
                                    .py_1()
                                    .rounded_xs()
                                    .cursor_text()
                                    .bg(rgb(BG_WELL))
                                    .border_1()
                                    .border_color(rgb(if add_focused {
                                        SKY_FG
                                    } else {
                                        BORDER_SUBTLE
                                    }))
                                    .font_family(self.fonts.mono_num.clone())
                                    .text_xs()
                                    .text_color(rgb(if self.industry_add_ticker_input.is_empty() {
                                        INK_MUTED
                                    } else {
                                        INK_PRIMARY
                                    }))
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.industry_input_focus = IndustryInputFocus::AddTicker;
                                        cx.notify();
                                    }))
                                    .child(if self.industry_add_ticker_input.is_empty() {
                                        l.t("+ 输入代码回车", "+ Add Ticker ↵").to_owned()
                                    } else {
                                        self.industry_add_ticker_input.clone()
                                    }),
                            ),
                    ),
            )
            .when(!profile.feynman_questions.is_empty(), |d| {
                d.child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_1p5()
                        .child(
                            div()
                                .font_family(self.fonts.ui.clone())
                                .text_xs()
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .text_color(rgb(INK_SECONDARY))
                                .child(l.t(
                                    "费曼通关自检表（确认真正理解后再毕业进能力圈）：",
                                    "Feynman Mastery Checklist (check before graduating to Circle):",
                                )),
                        )
                        .children(profile.feynman_questions.iter().enumerate().map(|(i, q)| {
                            let checked = profile.feynman_checked.get(i).copied().unwrap_or(false);
                            div()
                                .id(gpui::SharedString::from(format!("ind-fq-{i}")))
                                .w_full()
                                .px_2p5()
                                .py_1p5()
                                .rounded_sm()
                                .cursor_pointer()
                                .bg(rgb(BG_WELL))
                                .border_1()
                                .border_color(if checked {
                                    rgba(EMERALD_BORDER)
                                } else {
                                    rgb(BORDER_HAIRLINE)
                                })
                                .hover(|s| s.bg(rgb(BG_HOVER)))
                                .flex()
                                .items_center()
                                .gap_2()
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.toggle_selected_industry_feynman(i, cx);
                                }))
                                .child(
                                    div()
                                        .font_family(self.fonts.mono_num.clone())
                                        .text_xs()
                                        .text_color(rgb(if checked {
                                            EMERALD_FG
                                        } else {
                                            INK_MUTED
                                        }))
                                        .child(if checked { "[✓]" } else { "[ ]" }),
                                )
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .font_family(self.fonts.ui.clone())
                                        .text_xs()
                                        .text_color(rgb(INK_PRIMARY))
                                        .child(q.clone()),
                                )
                        })),
                )
            })
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1p5()
                    .child(
                        div()
                            .w_full()
                            .flex()
                            .items_center()
                            .justify_between()
                            .gap_2()
                            .child(
                                div()
                                    .font_family(self.fonts.ui.clone())
                                    .text_xs()
                                    .text_color(rgb(INK_SECONDARY))
                                    .child(l.t("人类行业研判笔记（## Human Notes）：", "Human Industry Notes (## Human Notes):")),
                            )
                            .child(
                                div()
                                    .id("ind-save-note-btn")
                                    .flex_shrink_0()
                                    .px_2p5()
                                    .py_1()
                                    .rounded_xs()
                                    .cursor_pointer()
                                    .bg(rgb(CTA_BG))
                                    .font_family(self.fonts.ui.clone())
                                    .text_size(px(11.))
                                    .font_weight(gpui::FontWeight::SEMIBOLD)
                                    .text_color(rgb(CTA_INK))
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        let note = this.industry_human_note_draft.clone();
                                        this.save_selected_industry_human_notes(&note, cx);
                                        this.industry_input_focus = IndustryInputFocus::None;
                                    }))
                                    .child(l.t("保存笔记", "Save Note")),
                            ),
                    )
                    .child(
                        div()
                            .id("ind-human-note-box")
                            .w_full()
                            .p_2p5()
                            .rounded_sm()
                            .cursor_text()
                            .bg(rgb(BG_WELL))
                            .border_1()
                            .border_color(rgb(if note_focused {
                                GOLD_FG
                            } else {
                                BORDER_SUBTLE
                            }))
                            .font_family(self.fonts.ui.clone())
                            .text_size(px(12.))
                            .line_height(px(19.))
                            .text_color(rgb(if self.industry_human_note_draft.is_empty() {
                                INK_MUTED
                            } else {
                                INK_PRIMARY
                            }))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.industry_input_focus = IndustryInputFocus::HumanNote;
                                cx.notify();
                            }))
                            .child(note_display),
                    ),
            )
    }

    fn render_industry_peer_matrix_card(
        &self,
        profile: &IndustryProfile,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        let l = self.lang;
        let anchor_mult_lbl = profile
            .multiple_low
            .map(|m| format!("{}x", m.round_dp(1).normalize()))
            .unwrap_or_else(|| "18x".to_owned());

        card_shell()
            .w_full()
            .p_5()
            .flex()
            .flex_col()
            .gap_3()
            .child(
                div()
                    .w_full()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_2()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(
                                div()
                                    .font_family(self.fonts.ui.clone())
                                    .text_sm()
                                    .font_weight(gpui::FontWeight::SEMIBOLD)
                                    .text_color(rgb(INK_HERO))
                                    .child(l.t(
                                        "代表公司 L0 确定性财务矩阵",
                                        "Representative Peer L0 Financial Matrix",
                                    )),
                            )
                            .child(badge(
                                "0 LLM".into(),
                                EMERALD_FG,
                                EMERALD_BG,
                                EMERALD_BORDER,
                                &self.fonts,
                            )),
                    )
                    .child(
                        div()
                            .id("ind-fetch-10k-btn")
                            .flex_shrink_0()
                            .px_2p5()
                            .py_1()
                            .rounded_sm()
                            .cursor_pointer()
                            .bg(rgba(SKY_BG))
                            .border_1()
                            .border_color(rgba(SKY_BORDER))
                            .hover(|s| s.bg(rgb(BG_HOVER)))
                            .font_family(self.fonts.ui.clone())
                            .text_xs()
                            .text_color(rgb(SKY_FG))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.start_industry_batch_10k_fetch(cx);
                            }))
                            .child(if self.industry_batch_10k_running {
                                l.t("拉取 10-K 中…", "Fetching 10-Ks…")
                            } else {
                                l.t("批量拉取 10-K 原文 (0 LLM)", "Fetch Peer 10-Ks (0 LLM)")
                            }),
                    ),
            )
            .when(self.industry_peer_rows.is_empty(), |d| {
                d.child(
                    div()
                        .font_family(self.fonts.ui.clone())
                        .text_xs()
                        .text_color(rgb(INK_MUTED))
                        .child(l.t(
                            "暂无 DuckDB L0 同行财务快照（请添加美股代码或先同步全美股 L0 湖仓）。",
                            "No DuckDB L0 financial snapshot yet (add US tickers or sync US L0 lakehouse).",
                        )),
                )
            })
            .when(!self.industry_peer_rows.is_empty(), |d| {
                d.child(
                    div()
                        .w_full()
                        .rounded_sm()
                        .border_1()
                        .border_color(rgb(BORDER_HAIRLINE))
                        .overflow_hidden()
                        .flex()
                        .flex_col()
                        .child(self.render_peer_matrix_header(l, &anchor_mult_lbl))
                        .children(self.industry_peer_rows.iter().enumerate().map(|(idx, row)| {
                            self.render_peer_matrix_row(idx, row, profile.multiple_low, cx)
                        })),
                )
            })
    }

    fn render_peer_matrix_header(&self, l: UiLang, anchor_mult_lbl: &str) -> gpui::Div {
        let h = |txt: &str, w: f32, right: bool| {
            div()
                .w(px(w))
                .flex()
                .when(right, gpui::Styled::justify_end)
                .font_family(self.fonts.ui.clone())
                .text_size(px(10.))
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(rgb(INK_MUTED))
                .child(txt.to_owned())
        };
        div()
            .w_full()
            .px_3()
            .py_2()
            .bg(rgb(BG_WELL))
            .border_b_1()
            .border_color(rgb(BORDER_HAIRLINE))
            .flex()
            .items_center()
            .gap_2()
            .child(h("Ticker", 72., false))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .font_family(self.fonts.ui.clone())
                    .text_size(px(10.))
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .text_color(rgb(INK_MUTED))
                    .child(l.t("公司", "Company")),
            )
            .child(h("5Y ROIC", 72., true))
            .child(h(l.t("5Y毛利", "5Y GM"), 68., true))
            .child(h("FCF%", 64., true))
            .child(h("SBC/OCF", 72., true))
            .child(h(l.t("现价", "Price"), 74., true))
            .child(h(
                &format!("{} ({anchor_mult_lbl})", l.t("锚价", "Ref")),
                92.,
                true,
            ))
            .child(h(l.t("操作", "Act"), 78., true))
    }

    fn render_peer_matrix_row(
        &self,
        idx: usize,
        row: &UniverseRow,
        multiple_low: Option<Decimal>,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        let l = self.lang;
        let (adj_strike, _dist) = industry::compute_industry_strike_price(row, multiple_low);
        let ticker_open = row.ticker.clone();
        let row_watch = row.clone();
        let in_watch = self
            .reports
            .iter()
            .any(|r| r.ticker.eq_ignore_ascii_case(&row.ticker));

        let cell_num = |val: Option<Decimal>, suf: &str, w: f32, col: u32| {
            let txt = val
                .map(|d| format!("{}{suf}", d.round_dp(1).normalize()))
                .unwrap_or_else(|| "—".to_owned());
            div()
                .w(px(w))
                .flex()
                .justify_end()
                .font_family(self.fonts.mono_num.clone())
                .text_xs()
                .text_color(rgb(col))
                .child(txt)
        };

        div()
            .w_full()
            .px_3()
            .py_2()
            .border_t_1()
            .border_color(rgb(BORDER_HAIRLINE))
            .flex()
            .items_center()
            .gap_2()
            .child(
                div()
                    .w(px(72.))
                    .font_family(self.fonts.mono_num.clone())
                    .text_xs()
                    .font_weight(gpui::FontWeight::BOLD)
                    .text_color(rgb(INK_HERO))
                    .child(row.ticker.clone()),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .font_family(self.fonts.ui.clone())
                    .text_xs()
                    .text_color(rgb(INK_SECONDARY))
                    .truncate()
                    .child(row.name.clone()),
            )
            .child(cell_num(row.roic_5y_avg_pct, "%", 72., EMERALD_FG))
            .child(cell_num(row.gross_margin_5y_avg_pct, "%", 68., INK_PRIMARY))
            .child(cell_num(row.strict_fcf_yield_pct, "%", 64., INK_PRIMARY))
            .child(cell_num(row.sbc_to_ocf_pct, "%", 72., INK_SECONDARY))
            .child(cell_num(row.latest_price, "", 74., INK_HERO))
            .child(cell_num(adj_strike, "", 92., SKY_FG))
            .child(div().w(px(78.)).flex().justify_end().child(if in_watch {
                div()
                    .id(gpui::SharedString::from(format!("ind-peer-open-{idx}")))
                    .px_2()
                    .py_0p5()
                    .rounded_xs()
                    .cursor_pointer()
                    .bg(rgb(BG_WELL))
                    .border_1()
                    .border_color(rgb(BORDER_SUBTLE))
                    .font_family(self.fonts.ui.clone())
                    .text_size(px(10.))
                    .text_color(rgb(EMERALD_FG))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.select_company(&ticker_open, Some(NavView::Dossier));
                        cx.notify();
                    }))
                    .child(l.t("打开", "Open"))
            } else {
                div()
                    .id(gpui::SharedString::from(format!("ind-peer-watch-{idx}")))
                    .px_2()
                    .py_0p5()
                    .rounded_xs()
                    .cursor_pointer()
                    .bg(rgba(EMERALD_BG))
                    .border_1()
                    .border_color(rgba(EMERALD_BORDER))
                    .font_family(self.fonts.ui.clone())
                    .text_size(px(10.))
                    .text_color(rgb(EMERALD_FG))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.add_universe_company_to_watch(&row_watch, cx);
                    }))
                    .child("+WATCH")
            }))
    }

    fn render_industry_primer_card(
        &self,
        profile: &IndustryProfile,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        let l = self.lang;
        let primer_path = self.paths.industry_primer_path(&profile.slug);
        let live_text = if self.industry_research_running
            && self.industry_research_slug.as_deref() == Some(profile.slug.as_str())
            && !self.industry_live_perspectives.is_empty()
        {
            Some(
                self.industry_live_perspectives
                    .iter()
                    .map(|(k, v)| format!("## {k}\n{v}"))
                    .collect::<Vec<_>>()
                    .join("\n\n"),
            )
        } else {
            self.industry_primer_md.clone()
        };

        card_shell()
            .w_full()
            .p_5()
            .flex()
            .flex_col()
            .gap_3()
            .child(
                div()
                    .w_full()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_2()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(
                                div()
                                    .font_family(self.fonts.ui.clone())
                                    .text_sm()
                                    .font_weight(gpui::FontWeight::SEMIBOLD)
                                    .text_color(rgb(INK_HERO))
                                    .child(l.t(
                                        "行业活手册 · Living Industry Primer",
                                        "Living Industry Primer",
                                    )),
                            )
                            .child(badge(
                                "SEC 10-K vs Web".into(),
                                SKY_FG,
                                SKY_BG,
                                SKY_BORDER,
                                &self.fonts,
                            )),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .when_some(live_text.clone(), |d, md_copy| {
                                d.child(
                                    div()
                                        .id("ind-copy-primer-btn")
                                        .px_2p5()
                                        .py_1()
                                        .rounded_xs()
                                        .cursor_pointer()
                                        .bg(rgb(BG_WELL))
                                        .border_1()
                                        .border_color(rgb(BORDER_SUBTLE))
                                        .font_family(self.fonts.ui.clone())
                                        .text_xs()
                                        .text_color(rgb(INK_SECONDARY))
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            cx.write_to_clipboard(ClipboardItem::new_string(
                                                md_copy.clone(),
                                            ));
                                            this.banner = Some((
                                                this.lang
                                                    .t("已复制行业活手册 Markdown", "Copied Industry Primer Markdown")
                                                    .into(),
                                                false,
                                            ));
                                            cx.notify();
                                        }))
                                        .child(l.t("复制 Markdown", "Copy Markdown")),
                                )
                            })
                            .child(
                                div()
                                    .id("ind-reveal-primer-btn")
                                    .px_2p5()
                                    .py_1()
                                    .rounded_xs()
                                    .cursor_pointer()
                                    .bg(rgb(BG_WELL))
                                    .border_1()
                                    .border_color(rgb(BORDER_SUBTLE))
                                    .font_family(self.fonts.ui.clone())
                                    .text_xs()
                                    .text_color(rgb(INK_SECONDARY))
                                    .on_click(cx.listener(move |_, _, _, cx| {
                                        if primer_path.exists() {
                                            cx.reveal_path(&primer_path);
                                        }
                                    }))
                                    .child(l.t("在 Finder 显示", "Reveal File")),
                            ),
                    ),
            )
            .child(match live_text {
                Some(md) if !md.trim().is_empty() => div()
                    .w_full()
                    .p_3()
                    .rounded_sm()
                    .bg(rgb(BG_WELL))
                    .border_1()
                    .border_color(rgb(BORDER_HAIRLINE))
                    .font_family(self.fonts.mono_term.clone())
                    .text_size(px(12.))
                    .line_height(px(20.))
                    .text_color(rgb(INK_PRIMARY))
                    .child(md),
                _ => div()
                    .w_full()
                    .p_3()
                    .rounded_sm()
                    .bg(rgb(BG_WELL))
                    .border_1()
                    .border_color(rgb(BORDER_HAIRLINE))
                    .font_family(self.fonts.ui.clone())
                    .text_xs()
                    .text_color(rgb(INK_MUTED))
                    .child(l.t(
                        "尚未生成行业活手册。点击右上角「启动 Alan 4 视角行业深研」，系统将自动拉取代表公司 SEC 10-K、结合 DuckDB 财务矩阵与公开网页搜索，写出产业链、护城河、历史死法与关键 KPI 活手册。",
                        "No primer yet. Click 'Run Alan 4-Lens Industry Research' above to synthesize Value Chain, Moats, Failure Modes & Key KPIs from SEC 10-K, DuckDB & bounded Web Search.",
                    )),
            })
    }

    fn render_industry_alan_tutor_card(
        &self,
        _profile: &IndustryProfile,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        let l = self.lang;
        let q_focused = self.industry_input_focus == IndustryInputFocus::AlanQuestion;
        let q_display = if self.industry_alan_input.is_empty() {
            l.t(
                "向 Alan 行业导师提问（例如：这个行业最赚现金流的环节为什么不会被颠覆？）…",
                "Ask Alan about this industry (e.g. why does the highest-ROIC choke point endure?)…",
            )
            .to_owned()
        } else {
            self.industry_alan_input.clone()
        };

        card_shell()
            .w_full()
            .p_5()
            .flex()
            .flex_col()
            .gap_3()
            .child(
                div()
                    .w_full()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_2()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(
                                div()
                                    .font_family(self.fonts.ui.clone())
                                    .text_sm()
                                    .font_weight(gpui::FontWeight::SEMIBOLD)
                                    .text_color(rgb(INK_HERO))
                                    .child(l.t(
                                        "Embedded Alan 行业学习与费曼自测台",
                                        "Embedded Alan Industry Learning & Feynman Desk",
                                    )),
                            )
                            .when_some(self.industry_alan_status_hint.clone(), |d, hint| {
                                d.child(
                                    div()
                                        .font_family(self.fonts.mono_term.clone())
                                        .text_xs()
                                        .text_color(rgb(VIOLET_FG))
                                        .child(hint),
                                )
                            }),
                    )
                    .child(
                        div()
                            .id("ind-feynman-quiz-btn")
                            .flex_shrink_0()
                            .px_3()
                            .py_1()
                            .rounded_sm()
                            .cursor_pointer()
                            .bg(rgba(GOLD_BG))
                            .border_1()
                            .border_color(rgba(GOLD_BORDER))
                            .hover(|s| s.bg(rgb(BG_HOVER)))
                            .font_family(self.fonts.ui.clone())
                            .text_xs()
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .text_color(rgb(GOLD_FG))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.trigger_industry_feynman_quiz(cx);
                            }))
                            .child(l.t("发起费曼通关自测 (4问)", "Start Feynman Quiz (4 Qs)")),
                    ),
            )
            .when(!self.industry_alan_messages.is_empty(), |d| {
                d.child(div().flex().flex_col().gap_2().children(
                    self.industry_alan_messages.iter().map(|msg| {
                        let is_user = msg.role == AgentMessageRole::User;
                        div()
                            .w_full()
                            .p_3()
                            .rounded_sm()
                            .bg(rgb(if is_user { BG_SURFACE } else { BG_WELL }))
                            .border_1()
                            .border_color(if is_user {
                                rgb(BORDER_SUBTLE)
                            } else {
                                rgba(VIOLET_BORDER)
                            })
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(
                                div()
                                    .font_family(self.fonts.mono_term.clone())
                                    .text_size(px(10.))
                                    .font_weight(gpui::FontWeight::BOLD)
                                    .text_color(rgb(if is_user { INK_MUTED } else { VIOLET_FG }))
                                    .child(if is_user {
                                        "YOU"
                                    } else {
                                        "ALAN · INDUSTRY TUTOR"
                                    }),
                            )
                            .child(
                                div()
                                    .font_family(self.fonts.ui.clone())
                                    .text_size(px(13.))
                                    .line_height(px(21.))
                                    .text_color(rgb(INK_PRIMARY))
                                    .child(if msg.text.is_empty() && msg.streaming {
                                        l.t("思考与检索中…", "Thinking & querying tools…")
                                            .to_owned()
                                    } else {
                                        msg.text.clone()
                                    }),
                            )
                    }),
                ))
            })
            .child(
                div()
                    .w_full()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .id("ind-alan-q-input")
                            .flex_1()
                            .min_w_0()
                            .px_3()
                            .py_2()
                            .rounded_sm()
                            .cursor_text()
                            .bg(rgb(BG_WELL))
                            .border_1()
                            .border_color(rgb(if q_focused { VIOLET_FG } else { BORDER_SUBTLE }))
                            .font_family(self.fonts.ui.clone())
                            .text_size(px(12.))
                            .text_color(rgb(if self.industry_alan_input.is_empty() {
                                INK_MUTED
                            } else {
                                INK_PRIMARY
                            }))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.industry_input_focus = IndustryInputFocus::AlanQuestion;
                                cx.notify();
                            }))
                            .child(q_display),
                    )
                    .child(
                        div()
                            .id("ind-alan-q-send")
                            .flex_shrink_0()
                            .px_3p5()
                            .py_2()
                            .rounded_sm()
                            .cursor_pointer()
                            .bg(rgb(CTA_BG))
                            .font_family(self.fonts.ui.clone())
                            .text_xs()
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .text_color(rgb(CTA_INK))
                            .on_click(cx.listener(|this, _, _, cx| {
                                let q = this.industry_alan_input.clone();
                                this.submit_industry_alan_question(&q, cx);
                            }))
                            .child(l.t("提问 ↵", "Ask ↵")),
                    ),
            )
    }
}
