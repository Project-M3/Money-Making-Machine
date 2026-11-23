use super::{
    AgentMessageRole, AgentUiMessage, DossierTab, MungerWorkbench, NavView, components::*, theme::*,
};
use crate::{
    alan_interrogator::{
        AcmeEvidenceKind, InterrogatorEvent, M3InterrogatorHandle, TuringTapeEntry, TuringTapeKind,
        parse_acme_evidence_tokens,
    },
    model::{Bucket, Dossier, MungerReport},
    store,
};
use gpui::{Context, div, prelude::*, px, rgb, rgba};
use std::collections::HashMap;

impl MungerWorkbench {
    pub fn is_alan_streaming(&self) -> bool {
        self.agent_streaming
    }

    pub(super) fn ensure_alan_handle(&mut self) -> M3InterrogatorHandle {
        if let Some(existing) = &self.alan_handle {
            existing.refresh_fs();
            return existing.clone();
        }
        let handle = M3InterrogatorHandle::start_shared_credentials(self.paths.clone());
        self.alan_handle = Some(handle.clone());
        handle
    }

    pub(super) fn next_agent_seq(&mut self) -> usize {
        let seq = self.agent_seq;
        self.agent_seq += 1;
        seq
    }

    pub(super) fn append_tape(
        &mut self,
        call_id: Option<String>,
        kind: TuringTapeKind,
        summary: String,
        detail: Option<String>,
        status: String,
    ) {
        let seq = self.agent_tape.len() + 1;
        self.agent_tape.push(TuringTapeEntry {
            seq,
            call_id,
            kind,
            summary,
            detail,
            status,
        });
    }

    pub(super) fn submit_alan_question(
        &mut self,
        ticker: &str,
        name: &str,
        bucket: Bucket,
        raw_question: String,
    ) {
        let question = raw_question.trim().to_string();
        if question.is_empty() {
            return;
        }
        self.agent_input_draft.clear();
        self.agent_input_focused = false;
        self.agent_session_ticker = Some(ticker.to_string());
        self.agent_streaming = true;
        self.agent_status_hint = Some("正在连接 EmbeddedAlan V2…".to_string());

        let short_q: String = question.chars().take(72).collect();
        self.append_tape(
            None,
            TuringTapeKind::Input,
            format!("{ticker} · {short_q}"),
            Some(question.clone()),
            "sent".to_string(),
        );

        let user_seq = self.next_agent_seq();
        let asst_seq = self.next_agent_seq();
        self.agent_messages.push(AgentUiMessage {
            id: format!("user-{user_seq}"),
            role: AgentMessageRole::User,
            raw_text: question.clone(),
            text: question.clone(),
            tokens: Vec::new(),
            streaming: false,
        });
        self.agent_messages.push(AgentUiMessage {
            id: format!("assistant-{asst_seq}"),
            role: AgentMessageRole::Assistant,
            raw_text: String::new(),
            text: String::new(),
            tokens: Vec::new(),
            streaming: true,
        });

        let handle = self.ensure_alan_handle();
        handle.submit(ticker, name, bucket, question);
    }

    pub(super) fn start_feynman_interrogation(&mut self, ticker: &str, name: &str, bucket: Bucket) {
        let starter = format!(
            "请使用 m3_report 与 m3_dossier 获取 {ticker}（{name}）的确定性财务报告与现有审讯档案，指出当前估值与护城河中最脆弱的核心假设，并向我发起一轮费曼审讯追问。"
        );
        self.submit_alan_question(ticker, name, bucket, starter);
    }

    pub(super) fn interrupt_alan(&mut self) {
        if let Some(handle) = &self.alan_handle {
            handle.interrupt();
        }
        self.agent_streaming = false;
        self.agent_status_hint = Some("已中断当前审讯".to_string());
        if let Some(last) = self
            .agent_messages
            .iter_mut()
            .rev()
            .find(|m| m.role == AgentMessageRole::Assistant)
        {
            last.streaming = false;
        }
    }

    pub(super) fn reset_alan_session(&mut self) {
        if let Some(handle) = &self.alan_handle {
            handle.reset();
        }
        self.agent_messages.clear();
        self.agent_tape.clear();
        self.agent_expanded_evidence = None;
        self.agent_expanded_tape = None;
        self.agent_streaming = false;
        self.agent_status_hint = None;
    }

    pub fn poll_alan_events(&mut self) -> bool {
        let Some(handle) = self.alan_handle.as_ref() else {
            return false;
        };
        let events = handle.drain_events();
        if events.is_empty() {
            return false;
        }

        let mut reload_disk = false;
        let focus_ticker = self
            .agent_session_ticker
            .clone()
            .unwrap_or_else(|| self.selected_ticker.clone());
        let mut dossier_map = HashMap::new();
        for rep in &self.reports {
            if rep.ticker == self.selected_ticker {
                dossier_map.insert(rep.ticker.clone(), self.selected_dossier.clone());
            } else if let Ok(d) = store::load_dossier(&self.paths, &rep.ticker) {
                dossier_map.insert(rep.ticker.clone(), d);
            }
        }

        for ev in events {
            let mut slice = AgentPanelSlice {
                streaming: &mut self.agent_streaming,
                status_hint: &mut self.agent_status_hint,
                messages: &mut self.agent_messages,
                tape: &mut self.agent_tape,
                seq: &mut self.agent_seq,
            };
            if apply_interrogator_event(&mut slice, &ev, &focus_ticker, &self.reports, &dossier_map)
            {
                reload_disk = true;
            }
        }

        if reload_disk {
            self.reload_from_disk();
        }
        true
    }

    pub(super) fn render_company_agent_body(
        &self,
        mut canvas_el: gpui::Stateful<gpui::Div>,
        rep: &MungerReport,
        cx: &mut Context<Self>,
    ) -> gpui::Stateful<gpui::Div> {
        let draft_source = self
            .selected_dossier
            .as_ref()
            .map(|d| d.draft_source.clone())
            .unwrap_or_else(|| "agent_draft".into());
        let updated_at = self
            .selected_dossier
            .as_ref()
            .map(|d| d.updated_at.clone())
            .unwrap_or_else(|| "—".into());
        let dossier_file = format!("workbench/dossiers/{}.json", rep.ticker);
        let dossier_cmd = format!("./apps/m3-desktop/m3 dossier {}", rep.ticker);
        let ticker_cli = rep.ticker.clone();

        canvas_el = self.render_embedded_alan_console(canvas_el, rep, cx);

        canvas_el = canvas_el.child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(div().w(px(6.)).h(px(6.)).rounded_full().bg(rgb(VIOLET_FG)))
                        .child(
                            div()
                                .font_family(self.fonts.ui.clone())
                                .text_xs()
                                .text_color(rgb(VIOLET_FG))
                                .child(format!("AI 幕僚草稿 · {draft_source}")),
                        ),
                )
                .child(
                    div()
                        .font_family(self.fonts.mono_num.clone())
                        .text_xs()
                        .text_color(rgb(INK_MUTED))
                        .child(format!("更新于 {updated_at}")),
                ),
        );

        if let Some(dossier) = &self.selected_dossier {
            let pending = dossier.human_judgment.pending_human_review;

            // 1. 3-Sentence Business Filter
            canvas_el = canvas_el.child(
                card_shell()
                    .p_4()
                    .child(
                        div()
                            .font_family(self.fonts.ui.clone())
                            .text_xs()
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .text_color(rgb(INK_SECONDARY))
                            .child("能力圈三句话过滤"),
                    )
                    .child(
                        div()
                            .mt_2()
                            .font_family(self.fonts.ui.clone())
                            .text_size(px(14.))
                            .line_height(px(23.))
                            .text_color(rgb(INK_PRIMARY))
                            .child(dossier.three_sentence_business.clone()),
                    ),
            );

            // 2. Interdisciplinary Latticework (Single flat 2x2 grid separated by hairlines)
            let quad = |title: &str, body: &str| {
                div()
                    .flex_1()
                    .min_w_0()
                    .p_4()
                    .child(
                        div()
                            .font_family(self.fonts.ui.clone())
                            .text_xs()
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .text_color(rgb(INK_HERO))
                            .child(title.to_owned()),
                    )
                    .child(
                        div()
                            .mt_1p5()
                            .font_family(self.fonts.ui.clone())
                            .text_size(px(13.))
                            .line_height(px(21.))
                            .text_color(rgb(INK_SECONDARY))
                            .child(body.to_owned()),
                    )
            };

            canvas_el = canvas_el.child(
                card_shell()
                    .child(
                        div()
                            .px_4()
                            .py_2p5()
                            .bg(rgb(BG_WELL))
                            .border_b_1()
                            .border_color(rgb(BORDER_HAIRLINE))
                            .font_family(self.fonts.ui.clone())
                            .text_xs()
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .text_color(rgb(INK_HERO))
                            .child("跨学科思维格栅"),
                    )
                    .child(
                        div()
                            .flex()
                            .border_b_1()
                            .border_color(rgb(BORDER_HAIRLINE))
                            .child(
                                quad("微观经济与护城河", &dossier.latticework.microeconomics)
                                    .border_r_1()
                                    .border_color(rgb(BORDER_HAIRLINE)),
                            )
                            .child(quad("生态位与技术演化", &dossier.latticework.ecology)),
                    )
                    .child(
                        div()
                            .flex()
                            .child(
                                quad(
                                    "心理学与管理层激励",
                                    &dossier.latticework.psychology_incentives,
                                )
                                .border_r_1()
                                .border_color(rgb(BORDER_HAIRLINE)),
                            )
                            .child(quad(
                                "资本配置历史",
                                &dossier.latticework.capital_allocation,
                            )),
                    ),
            );

            // 3. 5Y Pre-mortem + Feynman Interrogation Questions
            let pm_len = dossier.premortem.len();
            let q_len = dossier.open_questions.len();

            canvas_el = canvas_el.child(
                div()
                    .flex()
                    .gap_4()
                    .child(
                        card_shell()
                            .flex_1()
                            .min_w_0()
                            .p_4()
                            .flex()
                            .flex_col()
                            .child(
                                div()
                                    .pb_2p5()
                                    .border_b_1()
                                    .border_color(rgb(BORDER_HAIRLINE))
                                    .font_family(self.fonts.ui.clone())
                                    .text_sm()
                                    .font_weight(gpui::FontWeight::SEMIBOLD)
                                    .text_color(rgb(CORAL_FG))
                                    .child("5 年逆向事前验尸（亏损 >50% 路径）"),
                            )
                            .when(dossier.premortem.is_empty(), |d| {
                                d.child(
                                    div()
                                        .pt_3()
                                        .font_family(self.fonts.ui.clone())
                                        .text_xs()
                                        .text_color(rgb(INK_MUTED))
                                        .child("（尚无事前验尸草稿）"),
                                )
                            })
                            .children(dossier.premortem.iter().enumerate().map(|(i, item)| {
                                div()
                                    .py_2p5()
                                    .when(i + 1 < pm_len, |d| {
                                        d.border_b_1().border_color(rgb(BORDER_HAIRLINE))
                                    })
                                    .font_family(self.fonts.ui.clone())
                                    .text_size(px(13.))
                                    .line_height(px(21.))
                                    .text_color(rgb(INK_PRIMARY))
                                    .child(format!("{}. {item}", i + 1))
                            })),
                    )
                    .child(
                        card_shell()
                            .w(px(340.))
                            .flex_shrink_0()
                            .p_4()
                            .flex()
                            .flex_col()
                            .child(
                                div()
                                    .pb_2p5()
                                    .border_b_1()
                                    .border_color(rgb(BORDER_HAIRLINE))
                                    .flex()
                                    .items_center()
                                    .justify_between()
                                    .child(
                                        div()
                                            .font_family(self.fonts.ui.clone())
                                            .text_sm()
                                            .font_weight(gpui::FontWeight::SEMIBOLD)
                                            .text_color(rgb(INK_HERO))
                                            .child("费曼待答辩关键质疑"),
                                    )
                                    .child(if pending {
                                        badge(
                                            "待作答".into(),
                                            GOLD_FG,
                                            GOLD_BG,
                                            GOLD_BORDER,
                                            &self.fonts,
                                        )
                                    } else {
                                        badge(
                                            "已通过".into(),
                                            EMERALD_FG,
                                            EMERALD_BG,
                                            EMERALD_BORDER,
                                            &self.fonts,
                                        )
                                    }),
                            )
                            .children(dossier.open_questions.iter().enumerate().map(|(i, q)| {
                                div()
                                    .py_2p5()
                                    .when(i + 1 < q_len, |d| {
                                        d.border_b_1().border_color(rgb(BORDER_HAIRLINE))
                                    })
                                    .font_family(self.fonts.ui.clone())
                                    .text_size(px(13.))
                                    .line_height(px(20.))
                                    .text_color(rgb(INK_SECONDARY))
                                    .child(format!("? {q}"))
                            })),
                    ),
            );
        } else {
            canvas_el = canvas_el.child(
                card_shell()
                    .p_4()
                    .font_family(self.fonts.ui.clone())
                    .text_xs()
                    .text_color(rgb(INK_MUTED))
                    .child(format!(
                        "尚无 {} 审讯档案（{dossier_file}），可点击右上角「复制审讯 Prompt (p)」启动。",
                        rep.ticker
                    )),
            );
        }

        canvas_el.child(
            div()
                .pt_2()
                .border_t_1()
                .border_color(rgb(BORDER_HAIRLINE))
                .flex()
                .items_center()
                .justify_between()
                .child(
                    div()
                        .font_family(self.fonts.mono_term.clone())
                        .text_size(px(11.))
                        .text_color(rgb(INK_MUTED))
                        .child(dossier_file),
                )
                .child(
                    div()
                        .id("ag-copy-dossier")
                        .px_2p5()
                        .py_1()
                        .rounded_sm()
                        .bg(rgb(BG_SURFACE))
                        .border_1()
                        .border_color(rgb(BORDER_PILL))
                        .hover(|s| s.bg(rgb(BG_HOVER)))
                        .cursor_pointer()
                        .flex()
                        .items_center()
                        .gap_1p5()
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.copy_cli_command(
                                dossier_cmd.clone(),
                                &format!("dossier {ticker_cli}"),
                                cx,
                            );
                            cx.notify();
                        }))
                        .child(
                            div()
                                .font_family(self.fonts.mono_term.clone())
                                .text_xs()
                                .text_color(rgb(INK_SECONDARY))
                                .child("Copy dossier CLI"),
                        )
                        .child(kbd("c", &self.fonts)),
                ),
        )
    }

    pub(super) fn render_embedded_alan_console(
        &self,
        mut canvas_el: gpui::Stateful<gpui::Div>,
        rep: &MungerReport,
        cx: &mut Context<Self>,
    ) -> gpui::Stateful<gpui::Div> {
        let start_ticker = rep.ticker.clone();
        let start_name = rep.name.clone();
        let start_bucket = rep.bucket;

        let submit_ticker = rep.ticker.clone();
        let submit_name = rep.name.clone();
        let submit_bucket = rep.bucket;

        let status_pill = if self.agent_streaming {
            badge(
                self.agent_status_hint
                    .clone()
                    .unwrap_or_else(|| "Alan 审讯运行中…".into()),
                VIOLET_FG,
                VIOLET_BG,
                VIOLET_BORDER,
                &self.fonts,
            )
        } else {
            badge(
                "只读 /mnt/m3 · 确定性工具".into(),
                EMERALD_FG,
                EMERALD_BG,
                EMERALD_BORDER,
                &self.fonts,
            )
        };

        let mut header = div()
            .px_4()
            .py_2p5()
            .bg(rgb(BG_WELL))
            .border_b_1()
            .border_color(rgb(BORDER_HAIRLINE))
            .flex()
            .items_center()
            .justify_between()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(div().w(px(6.)).h(px(6.)).rounded_full().bg(rgb(VIOLET_FG)))
                    .child(
                        div()
                            .font_family(self.fonts.ui.clone())
                            .text_xs()
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .text_color(rgb(INK_HERO))
                            .child(format!("Alan 费曼审讯台 · {}", rep.ticker)),
                    )
                    .child(status_pill),
            );

        let mut actions = div().flex().items_center().gap_2();
        if self.agent_streaming {
            actions = actions.child(
                div()
                    .id("ag-interrupt")
                    .px_2p5()
                    .py_1()
                    .rounded_sm()
                    .bg(rgba(CORAL_BG))
                    .border_1()
                    .border_color(rgba(CORAL_BORDER))
                    .hover(|s| s.bg(rgb(BG_HOVER)))
                    .cursor_pointer()
                    .font_family(self.fonts.ui.clone())
                    .text_xs()
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .text_color(rgb(CORAL_FG))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.interrupt_alan();
                        cx.notify();
                    }))
                    .child("中断生成"),
            );
        } else {
            actions = actions.child(
                div()
                    .id("ag-start-feynman")
                    .px_2p5()
                    .py_1()
                    .rounded_sm()
                    .bg(rgb(CTA_BG))
                    .hover(|s| s.bg(rgb(CTA_BG_HOVER)))
                    .cursor_pointer()
                    .font_family(self.fonts.ui.clone())
                    .text_xs()
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .text_color(rgb(CTA_INK))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.start_feynman_interrogation(&start_ticker, &start_name, start_bucket);
                        cx.notify();
                    }))
                    .child("启动费曼审讯"),
            );
        }

        if !self.agent_messages.is_empty() || !self.agent_tape.is_empty() {
            actions = actions.child(
                div()
                    .id("ag-reset-session")
                    .px_2()
                    .py_1()
                    .rounded_sm()
                    .bg(rgb(BG_SURFACE))
                    .border_1()
                    .border_color(rgb(BORDER_PILL))
                    .hover(|s| s.bg(rgb(BG_HOVER)))
                    .cursor_pointer()
                    .font_family(self.fonts.ui.clone())
                    .text_xs()
                    .text_color(rgb(INK_SECONDARY))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.reset_alan_session();
                        cx.notify();
                    }))
                    .child("清空"),
            );
        }

        header = header.child(actions);

        let draft_empty = self.agent_input_draft.trim().is_empty();
        let input_border = if self.agent_input_focused {
            rgba(VIOLET_BORDER)
        } else {
            rgb(BORDER_PILL)
        };
        let input_display = if self.agent_input_draft.is_empty() {
            if self.agent_input_focused {
                "_".to_string()
            } else {
                format!(
                    "点击输入对 {} 的费曼追问（按 Enter 发送，Esc 退出输入）…",
                    rep.ticker
                )
            }
        } else if self.agent_input_focused {
            format!("{}_", self.agent_input_draft)
        } else {
            self.agent_input_draft.clone()
        };

        let composer_row = div()
            .p_3()
            .border_b_1()
            .border_color(rgb(BORDER_HAIRLINE))
            .flex()
            .items_center()
            .gap_2()
            .child(
                div()
                    .id("ag-question-input")
                    .flex_1()
                    .min_w_0()
                    .px_3()
                    .py_1p5()
                    .rounded_sm()
                    .bg(rgb(BG_WELL))
                    .border_1()
                    .border_color(input_border)
                    .cursor_pointer()
                    .font_family(self.fonts.ui.clone())
                    .text_xs()
                    .text_color(if self.agent_input_draft.is_empty() {
                        rgb(INK_MUTED)
                    } else {
                        rgb(INK_PRIMARY)
                    })
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.agent_input_focused = true;
                        cx.notify();
                    }))
                    .child(input_display),
            )
            .child(
                div()
                    .id("ag-send-question")
                    .px_3()
                    .py_1p5()
                    .rounded_sm()
                    .bg(if draft_empty {
                        rgb(BG_WELL)
                    } else {
                        rgb(CTA_BG)
                    })
                    .border_1()
                    .border_color(if draft_empty {
                        rgb(BORDER_PILL)
                    } else {
                        rgb(CTA_BG)
                    })
                    .cursor_pointer()
                    .font_family(self.fonts.ui.clone())
                    .text_xs()
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .text_color(if draft_empty {
                        rgb(INK_MUTED)
                    } else {
                        rgb(CTA_INK)
                    })
                    .on_click(cx.listener(move |this, _, _, cx| {
                        let draft = this.agent_input_draft.clone();
                        this.submit_alan_question(
                            &submit_ticker,
                            &submit_name,
                            submit_bucket,
                            draft,
                        );
                        cx.notify();
                    }))
                    .child("发送"),
            );

        // Flatten expanded evidence lookup across messages.
        let expanded_token = self.agent_expanded_evidence.as_ref().and_then(|eid| {
            self.agent_messages
                .iter()
                .flat_map(|m| m.tokens.iter())
                .find(|t| &t.id == eid)
                .cloned()
        });

        let mut stream_col = div().p_4().flex().flex_col().gap_3();
        if self.agent_messages.is_empty() {
            stream_col = stream_col.child(
                div()
                    .font_family(self.fonts.ui.clone())
                    .text_xs()
                    .line_height(px(20.))
                    .text_color(rgb(INK_MUTED))
                    .child("点击「启动费曼审讯」或在上方输入追问，直接驱动进程内 EmbeddedAlan V2；工具受红线策略钩子硬保护（严禁触碰 human_judgment 与 computed/*），右上角「复制审讯 Prompt」仍可作为外部会话兜底。"),
            );
        } else {
            for (msg_idx, msg) in self.agent_messages.iter().enumerate() {
                match msg.role {
                    AgentMessageRole::User => {
                        stream_col = stream_col.child(
                            div()
                                .px_3()
                                .py_2()
                                .rounded_sm()
                                .bg(rgb(BG_WELL))
                                .border_1()
                                .border_color(rgb(BORDER_HAIRLINE))
                                .font_family(self.fonts.ui.clone())
                                .text_xs()
                                .line_height(px(19.))
                                .text_color(rgb(INK_HERO))
                                .child(format!("你  ·  {}", msg.text)),
                        );
                    }
                    AgentMessageRole::SystemNote => {
                        stream_col = stream_col.child(
                            div()
                                .px_3()
                                .py_2()
                                .rounded_sm()
                                .bg(rgba(GOLD_BG))
                                .border_1()
                                .border_color(rgba(GOLD_BORDER))
                                .font_family(self.fonts.mono_term.clone())
                                .text_xs()
                                .line_height(px(19.))
                                .text_color(rgb(GOLD_FG))
                                .child(msg.text.clone()),
                        );
                    }
                    AgentMessageRole::Assistant => {
                        let body = if msg.text.is_empty() && msg.streaming {
                            "Alan 正在读取 /mnt/m3 与确定性报告…".to_string()
                        } else {
                            msg.text.clone()
                        };
                        let mut box_el = div().flex().flex_col().gap_2().child(
                            div()
                                .font_family(self.fonts.ui.clone())
                                .text_size(px(13.))
                                .line_height(px(22.))
                                .text_color(if msg.text.is_empty() && msg.streaming {
                                    rgb(INK_MUTED)
                                } else {
                                    rgb(INK_PRIMARY)
                                })
                                .child(body),
                        );
                        if !msg.tokens.is_empty() {
                            let mut row = div().flex().flex_wrap().gap_1p5();
                            for (tok_idx, tok) in msg.tokens.iter().enumerate() {
                                let tok_id = tok.id.clone();
                                let selected =
                                    self.agent_expanded_evidence.as_deref() == Some(&tok_id);
                                let (fg, bg, border) = match tok.kind {
                                    AcmeEvidenceKind::Report => {
                                        (EMERALD_FG, EMERALD_BG, EMERALD_BORDER)
                                    }
                                    AcmeEvidenceKind::Xbrl => (SKY_FG, SKY_BG, SKY_BORDER),
                                    AcmeEvidenceKind::Dossier => (GOLD_FG, GOLD_BG, GOLD_BORDER),
                                };
                                row = row.child(
                                    div()
                                        .id(("ag-acme-tok", msg_idx * 32 + tok_idx))
                                        .px_2()
                                        .py_0p5()
                                        .rounded_sm()
                                        .bg(if selected { rgb(BG_ACTIVE) } else { rgba(bg) })
                                        .border_1()
                                        .border_color(rgba(border))
                                        .hover(|s| s.bg(rgb(BG_HOVER)))
                                        .cursor_pointer()
                                        .font_family(self.fonts.mono_term.clone())
                                        .text_xs()
                                        .text_color(rgb(fg))
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            if this.agent_expanded_evidence.as_deref()
                                                == Some(tok_id.as_str())
                                            {
                                                this.agent_expanded_evidence = None;
                                            } else {
                                                this.agent_expanded_evidence = Some(tok_id.clone());
                                            }
                                            cx.notify();
                                        }))
                                        .child(tok.label.clone()),
                                );
                            }
                            box_el = box_el.child(row);
                        }
                        stream_col = stream_col.child(box_el);
                    }
                }
            }
        }

        if let Some(tok) = expanded_token {
            let jump_ticker = tok.ticker.clone();
            let is_xbrl = tok.kind == AcmeEvidenceKind::Xbrl;
            stream_col = stream_col.child(
                div()
                    .p_3()
                    .rounded_sm()
                    .bg(rgb(BG_WELL))
                    .border_1()
                    .border_color(rgb(BORDER_PILL))
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .child(
                                div()
                                    .font_family(self.fonts.mono_term.clone())
                                    .text_xs()
                                    .font_weight(gpui::FontWeight::SEMIBOLD)
                                    .text_color(rgb(INK_HERO))
                                    .child(format!("Acme 证据核验 · {}", tok.label)),
                            )
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_2()
                                    .when(is_xbrl, |d| {
                                        d.child(
                                            div()
                                                .id("ag-jump-xbrl")
                                                .px_2()
                                                .py_0p5()
                                                .rounded_sm()
                                                .bg(rgba(SKY_BG))
                                                .border_1()
                                                .border_color(rgba(SKY_BORDER))
                                                .cursor_pointer()
                                                .font_family(self.fonts.ui.clone())
                                                .text_xs()
                                                .text_color(rgb(SKY_FG))
                                                .on_click(cx.listener(move |this, _, _, cx| {
                                                    if this.reports.iter().any(|r| {
                                                        r.ticker.eq_ignore_ascii_case(&jump_ticker)
                                                    }) {
                                                        this.load_selection(&jump_ticker);
                                                    }
                                                    this.dossier_tab = DossierTab::Inspector;
                                                    this.active_view = NavView::Dossier;
                                                    cx.notify();
                                                }))
                                                .child("跳至 3 XBRL 底表 →"),
                                        )
                                    })
                                    .child(
                                        div()
                                            .id("ag-close-evidence")
                                            .px_2()
                                            .py_0p5()
                                            .cursor_pointer()
                                            .font_family(self.fonts.ui.clone())
                                            .text_xs()
                                            .text_color(rgb(INK_MUTED))
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                this.agent_expanded_evidence = None;
                                                cx.notify();
                                            }))
                                            .child("收起"),
                                    ),
                            ),
                    )
                    .child(
                        div()
                            .font_family(self.fonts.ui.clone())
                            .text_xs()
                            .line_height(px(19.))
                            .text_color(rgb(INK_SECONDARY))
                            .child(tok.summary),
                    ),
            );
        }

        let mut console_card = card_shell()
            .child(header)
            .child(composer_row)
            .child(stream_col);

        if !self.agent_tape.is_empty() {
            let tape_len = self.agent_tape.len();
            let mut tape_box = div()
                .border_t_1()
                .border_color(rgb(BORDER_HAIRLINE))
                .bg(rgb(BG_WELL))
                .px_4()
                .py_2p5()
                .flex()
                .flex_col()
                .child(
                    div()
                        .pb_1p5()
                        .font_family(self.fonts.mono_term.clone())
                        .text_size(px(10.))
                        .text_color(rgb(INK_MUTED))
                        .child("TURING TAPE · DETERMINISTIC TRACE"),
                );

            for (idx, row) in self.agent_tape.iter().enumerate() {
                let row_seq = row.seq;
                let expanded = self.agent_expanded_tape == Some(row_seq);
                let tag_color = match row.kind {
                    TuringTapeKind::Input => INK_MUTED,
                    TuringTapeKind::Tool => SKY_FG,
                    TuringTapeKind::Out => EMERALD_FG,
                    TuringTapeKind::Policy => CORAL_FG,
                    TuringTapeKind::Error => GOLD_FG,
                };
                tape_box = tape_box.child(
                    div()
                        .id(("ag-tape-row", idx))
                        .py_1()
                        .when(idx + 1 < tape_len, |d| {
                            d.border_b_1().border_color(rgb(BORDER_HAIRLINE))
                        })
                        .cursor_pointer()
                        .hover(|s| s.bg(rgb(BG_HOVER)))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if this.agent_expanded_tape == Some(row_seq) {
                                this.agent_expanded_tape = None;
                            } else {
                                this.agent_expanded_tape = Some(row_seq);
                            }
                            cx.notify();
                        }))
                        .flex()
                        .flex_col()
                        .gap_1()
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .justify_between()
                                .gap_2()
                                .child(
                                    div()
                                        .flex()
                                        .items_center()
                                        .gap_2()
                                        .min_w_0()
                                        .child(
                                            div()
                                                .font_family(self.fonts.mono_term.clone())
                                                .text_size(px(11.))
                                                .text_color(rgb(INK_MUTED))
                                                .child(format!("{:04}", row.seq)),
                                        )
                                        .child(
                                            div()
                                                .font_family(self.fonts.mono_term.clone())
                                                .text_size(px(11.))
                                                .text_color(rgb(tag_color))
                                                .child(format!("[{}]", row.kind.tag())),
                                        )
                                        .child(
                                            div()
                                                .font_family(self.fonts.mono_term.clone())
                                                .text_size(px(11.))
                                                .text_color(rgb(INK_PRIMARY))
                                                .truncate()
                                                .child(row.summary.clone()),
                                        ),
                                )
                                .child(
                                    div()
                                        .font_family(self.fonts.mono_term.clone())
                                        .text_size(px(10.))
                                        .text_color(rgb(INK_MUTED))
                                        .child(row.status.clone()),
                                ),
                        )
                        .when(expanded && row.detail.is_some(), |d| {
                            d.child(
                                div()
                                    .pl_6()
                                    .font_family(self.fonts.mono_term.clone())
                                    .text_size(px(11.))
                                    .line_height(px(17.))
                                    .text_color(rgb(INK_SECONDARY))
                                    .child(row.detail.clone().unwrap_or_default()),
                            )
                        }),
                );
            }
            console_card = console_card.child(tape_box);
        }

        canvas_el = canvas_el.child(console_card);
        canvas_el
    }
}

pub(super) struct AgentPanelSlice<'a> {
    pub(super) streaming: &'a mut bool,
    pub(super) status_hint: &'a mut Option<String>,
    pub(super) messages: &'a mut Vec<AgentUiMessage>,
    pub(super) tape: &'a mut Vec<TuringTapeEntry>,
    pub(super) seq: &'a mut usize,
}

pub(super) fn apply_interrogator_event(
    state: &mut AgentPanelSlice<'_>,
    ev: &InterrogatorEvent,
    focus_ticker: &str,
    reports: &[MungerReport],
    dossiers: &HashMap<String, Option<Dossier>>,
) -> bool {
    match ev {
        InterrogatorEvent::Ready | InterrogatorEvent::UserSubmitted { .. } => {
            if state
                .status_hint
                .as_deref()
                .is_some_and(|h| h.contains("正在连接"))
            {
                *state.status_hint = None;
            }
            false
        }
        InterrogatorEvent::ToolStarted {
            call_id,
            name,
            title,
        } => {
            *state.status_hint = Some(format!("正在执行 {name}…"));
            let seq = state.tape.len() + 1;
            state.tape.push(TuringTapeEntry {
                seq,
                call_id: Some(call_id.clone()),
                kind: TuringTapeKind::Tool,
                summary: if title.is_empty() || title == name {
                    format!("{name} {focus_ticker}")
                } else {
                    format!("{name} · {title}")
                },
                detail: None,
                status: "running".to_string(),
            });
            false
        }
        InterrogatorEvent::ToolProgress { call_id, preview } => {
            if let Some(row) = state
                .tape
                .iter_mut()
                .rev()
                .find(|r| r.call_id.as_deref() == Some(call_id.as_str()))
            {
                row.detail = Some(preview.clone());
            }
            false
        }
        InterrogatorEvent::ToolFinished {
            call_id,
            name,
            status,
            preview,
        } => {
            let blocked_by_redline = preview.contains("红线拦截");
            if let Some(row) = state
                .tape
                .iter_mut()
                .rev()
                .find(|r| r.call_id.as_deref() == Some(call_id.as_str()))
            {
                row.status = if blocked_by_redline {
                    "denied".to_string()
                } else {
                    status.clone()
                };
                if blocked_by_redline {
                    row.kind = TuringTapeKind::Policy;
                }
                if !preview.is_empty() {
                    row.detail = Some(preview.clone());
                }
            } else {
                let seq = state.tape.len() + 1;
                state.tape.push(TuringTapeEntry {
                    seq,
                    call_id: Some(call_id.clone()),
                    kind: if blocked_by_redline {
                        TuringTapeKind::Policy
                    } else {
                        TuringTapeKind::Tool
                    },
                    summary: format!("{name} {focus_ticker}"),
                    detail: (!preview.is_empty()).then(|| preview.clone()),
                    status: if blocked_by_redline {
                        "denied".to_string()
                    } else {
                        status.clone()
                    },
                });
            }
            name == "m3_update_draft" && status == "completed" && !blocked_by_redline
        }
        InterrogatorEvent::TextDelta {
            chunk, is_final, ..
        } => {
            *state.status_hint = None;
            if let Some(last) = state
                .messages
                .iter_mut()
                .rev()
                .find(|m| m.role == AgentMessageRole::Assistant && m.streaming)
            {
                last.raw_text.push_str(chunk);
                let (cleaned, tokens) =
                    parse_acme_evidence_tokens(&last.raw_text, focus_ticker, reports, dossiers);
                last.text = cleaned;
                last.tokens = tokens;
                if *is_final {
                    last.streaming = false;
                }
            } else {
                let id = *state.seq;
                *state.seq += 1;
                let (cleaned, tokens) =
                    parse_acme_evidence_tokens(chunk, focus_ticker, reports, dossiers);
                state.messages.push(AgentUiMessage {
                    id: format!("assistant-{id}"),
                    role: AgentMessageRole::Assistant,
                    raw_text: chunk.clone(),
                    text: cleaned,
                    tokens,
                    streaming: !is_final,
                });
            }
            false
        }
        InterrogatorEvent::TurnDone { error, .. } => {
            *state.streaming = false;
            *state.status_hint = None;
            let mut token_count = 0usize;
            if let Some(last) = state
                .messages
                .iter_mut()
                .rev()
                .find(|m| m.role == AgentMessageRole::Assistant)
            {
                last.streaming = false;
                token_count = last.tokens.len();
            }
            if let Some(err) = error {
                if state
                    .messages
                    .last()
                    .is_some_and(|m| m.role == AgentMessageRole::Assistant && m.text.is_empty())
                {
                    state.messages.pop();
                }
                let seq = state.tape.len() + 1;
                state.tape.push(TuringTapeEntry {
                    seq,
                    call_id: None,
                    kind: TuringTapeKind::Error,
                    summary: "审讯未能完成".to_string(),
                    detail: Some(err.clone()),
                    status: "error".to_string(),
                });
                let id = *state.seq;
                *state.seq += 1;
                state.messages.push(AgentUiMessage {
                    id: format!("note-{id}"),
                    role: AgentMessageRole::SystemNote,
                    raw_text: err.clone(),
                    text: err.clone(),
                    tokens: Vec::new(),
                    streaming: false,
                });
                false
            } else {
                if state
                    .messages
                    .last()
                    .is_some_and(|m| m.role == AgentMessageRole::Assistant && m.text.is_empty())
                {
                    state.messages.pop();
                }
                let seq = state.tape.len() + 1;
                state.tape.push(TuringTapeEntry {
                    seq,
                    call_id: None,
                    kind: TuringTapeKind::Out,
                    summary: format!("回答完成 · 引用 Acme 证据 {token_count} 项"),
                    detail: None,
                    status: "done".to_string(),
                });
                true
            }
        }
        InterrogatorEvent::ResetDone => {
            *state.streaming = false;
            *state.status_hint = None;
            state.messages.clear();
            state.tape.clear();
            false
        }
        InterrogatorEvent::Error { message } => {
            *state.streaming = false;
            *state.status_hint = None;
            if state
                .messages
                .last()
                .is_some_and(|m| m.role == AgentMessageRole::Assistant && m.text.is_empty())
            {
                state.messages.pop();
            }
            let seq = state.tape.len() + 1;
            state.tape.push(TuringTapeEntry {
                seq,
                call_id: None,
                kind: TuringTapeKind::Error,
                summary: "EmbeddedAlan 状态提示".to_string(),
                detail: Some(message.clone()),
                status: "error".to_string(),
            });
            let id = *state.seq;
            *state.seq += 1;
            state.messages.push(AgentUiMessage {
                id: format!("err-{id}"),
                role: AgentMessageRole::SystemNote,
                raw_text: message.clone(),
                text: message.clone(),
                tokens: Vec::new(),
                streaming: false,
            });
            false
        }
    }
}
