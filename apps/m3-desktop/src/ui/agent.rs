use super::{
    AgentMessageRole, AgentUiMessage, DossierTab, MungerWorkbench, NavView, components::*,
    format::*, theme::*,
};
use crate::{
    alan_interrogator::{
        AcmeEvidenceKind, InterrogatorEvent, M3InterrogatorHandle, TuringTapeEntry, TuringTapeKind,
        parse_acme_evidence_tokens,
    },
    cli,
    i18n::UiLang,
    lake::LakeDb,
    lake_l1::SimilarSearchFilter,
    lake_l2::ResearchRunStatus,
    model::{Bucket, Dossier, MungerReport},
    research::{
        self, DeepResearchEvent, DeepResearchPerspective, DeepResearchRequest, DeepResearchResult,
    },
    store::{self, WorkspacePaths},
};
use gpui::{Context, div, prelude::*, px, rgb, rgba};
use rust_decimal::Decimal;
use std::{collections::HashMap, sync::mpsc};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoSection {
    pub id: String,
    pub heading: String,
    pub body: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ParsedResearchMemo {
    pub title: String,
    pub metadata: Vec<(String, String)>,
    pub sections: Vec<MemoSection>,
    pub citations: Vec<String>,
    pub human_notes: Option<String>,
}

/// Parses a `workbench/memos/<TICKER>.md` Living Research Memo into structured sections & citations.
pub fn parse_research_memo(raw: &str) -> ParsedResearchMemo {
    let mut parsed = ParsedResearchMemo::default();
    let mut current_heading: Option<String> = None;
    let mut current_lines: Vec<String> = Vec::new();
    let mut in_preamble = true;

    let flush_section = |parsed: &mut ParsedResearchMemo, heading: String, lines: Vec<String>| {
        let body = lines.join("\n").trim().to_owned();
        let lower = heading.to_ascii_lowercase();
        if lower.contains("evidence & citations") || lower == "citations" {
            let mut cites = Vec::new();
            for line in body.lines() {
                let t = line.trim();
                if t.is_empty() || (t.starts_with('_') && t.ends_with('_')) {
                    continue;
                }
                cites.push(t.to_owned());
            }
            parsed.citations = cites;
        } else if lower.starts_with("human notes") {
            let cleaned: Vec<&str> = body
                .lines()
                .map(str::trim)
                .filter(|l| !l.is_empty() && !l.starts_with("<!--"))
                .collect();
            if !cleaned.is_empty() {
                parsed.human_notes = Some(cleaned.join("\n"));
            }
        }

        let id = if lower.contains("(moat)") {
            "moat".to_owned()
        } else if lower.contains("(accounting)") {
            "accounting".to_owned()
        } else if lower.contains("(latent_peers)") {
            "latent_peers".to_owned()
        } else if lower.contains("(premortem)") {
            "premortem".to_owned()
        } else if lower.contains("deterministic financial snapshot") {
            "snapshot".to_owned()
        } else if lower.contains("evidence & citations") {
            "citations".to_owned()
        } else if lower.starts_with("human notes") {
            "human_notes".to_owned()
        } else {
            lower.replace(' ', "_")
        };

        parsed.sections.push(MemoSection { id, heading, body });
    };

    for line in raw.lines() {
        let trimmed = line.trim_end();
        if in_preamble && parsed.title.is_empty() && trimmed.starts_with("# ") {
            parsed.title = trimmed.trim_start_matches("# ").trim().to_owned();
            continue;
        }
        if let Some(heading) = trimmed.strip_prefix("## ") {
            in_preamble = false;
            if let Some(prev) = current_heading.replace(heading.trim().to_owned()) {
                let lines = std::mem::take(&mut current_lines);
                flush_section(&mut parsed, prev, lines);
            }
            continue;
        }
        if in_preamble {
            if let Some(rest) = trimmed.trim().strip_prefix("- **")
                && let Some((k, v)) = rest.split_once("**:")
            {
                parsed
                    .metadata
                    .push((k.trim().to_owned(), v.trim().trim_matches('`').to_owned()));
            }
        } else {
            current_lines.push(trimmed.to_owned());
        }
    }

    if let Some(last) = current_heading {
        flush_section(&mut parsed, last, current_lines);
    }

    parsed
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PerspectiveUiStatus {
    Pending,
    Running,
    Completed,
    Failed,
}

impl PerspectiveUiStatus {
    pub fn label_for(self, lang: UiLang) -> &'static str {
        match self {
            Self::Pending => lang.t("待命", "Pending"),
            Self::Running => lang.t("分析中…", "Running…"),
            Self::Completed => lang.t("已完成", "Completed"),
            Self::Failed => lang.t("未完成", "Incomplete"),
        }
    }
}

pub(super) fn perspective_title_for(id: &str, fallback_cn: &str, lang: UiLang) -> String {
    match (lang, id) {
        (UiLang::En, "moat") => "Moat".to_owned(),
        (UiLang::En, "accounting") => "Accounting".to_owned(),
        (UiLang::En, "latent_peers") => "Latent Peers".to_owned(),
        (UiLang::En, "premortem") => "5Y Pre-Mortem".to_owned(),
        _ => fallback_cn.to_owned(),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeepResearchPerspectiveUi {
    pub id: String,
    pub title_cn: String,
    pub status: PerspectiveUiStatus,
    pub tools_used: Vec<String>,
    pub stream_text: String,
    pub summary_chars: usize,
    pub input_tokens: u64,
    pub output_tokens: u64,
}

pub fn default_deep_research_perspectives() -> Vec<DeepResearchPerspectiveUi> {
    DeepResearchPerspective::all()
        .into_iter()
        .map(|p| DeepResearchPerspectiveUi {
            id: p.id().to_owned(),
            title_cn: p.title_cn().to_owned(),
            status: PerspectiveUiStatus::Pending,
            tools_used: Vec::new(),
            stream_text: String::new(),
            summary_chars: 0,
            input_tokens: 0,
            output_tokens: 0,
        })
        .collect()
}

pub struct DeepResearchUiSlice<'a> {
    pub running: &'a mut bool,
    pub run_id: &'a mut Option<String>,
    pub status_note: &'a mut Option<String>,
    pub budget_blocked: &'a mut Option<String>,
    pub perspectives: &'a mut Vec<DeepResearchPerspectiveUi>,
    pub last_result: &'a mut Option<DeepResearchResult>,
    pub tape: &'a mut Vec<TuringTapeEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeepResearchEventEffect {
    pub reload_context: bool,
    pub banner: Option<(String, bool)>,
}

#[allow(dead_code)]
pub fn apply_deep_research_event(
    state: &mut DeepResearchUiSlice<'_>,
    ev: &DeepResearchEvent,
) -> DeepResearchEventEffect {
    apply_deep_research_event_for(state, ev, UiLang::Zh)
}

pub fn apply_deep_research_event_for(
    state: &mut DeepResearchUiSlice<'_>,
    ev: &DeepResearchEvent,
    lang: UiLang,
) -> DeepResearchEventEffect {
    match ev {
        DeepResearchEvent::Started { run_id, ticker } => {
            *state.running = true;
            *state.run_id = Some(run_id.clone());
            *state.budget_blocked = None;
            *state.status_note = Some(match lang {
                UiLang::Zh => format!("正在启动 4 视角并行深研 ({ticker})…"),
                UiLang::En => format!("Starting 4-perspective parallel Deep Research ({ticker})…"),
            });
            let seq = state.tape.len() + 1;
            state.tape.push(TuringTapeEntry {
                seq,
                call_id: Some(run_id.clone()),
                kind: TuringTapeKind::Input,
                summary: match lang {
                    UiLang::Zh => format!("Deep Research 启动 · {ticker} ({run_id})"),
                    UiLang::En => format!("Deep Research started · {ticker} ({run_id})"),
                },
                detail: Some(
                    lang.t(
                        "4 并行视角：护城河 / 会计排雷 / 隐空间同行 / 5年逆向验尸",
                        "4 parallel lenses: Moat / Accounting / Latent Peers / 5Y Pre-Mortem",
                    )
                    .to_owned(),
                ),
                status: "started".to_owned(),
            });
            DeepResearchEventEffect {
                reload_context: false,
                banner: None,
            }
        }
        DeepResearchEvent::BudgetBlocked {
            run_id,
            ticker,
            reason,
        } => {
            *state.running = false;
            *state.run_id = Some(run_id.clone());
            *state.budget_blocked = Some(reason.clone());
            *state.status_note = Some(match lang {
                UiLang::Zh => format!("预算闸门硬拦截：{reason}"),
                UiLang::En => format!("Budget gate blocked: {reason}"),
            });
            for p in state.perspectives.iter_mut() {
                if p.status == PerspectiveUiStatus::Pending {
                    p.status = PerspectiveUiStatus::Failed;
                }
            }
            let seq = state.tape.len() + 1;
            state.tape.push(TuringTapeEntry {
                seq,
                call_id: Some(run_id.clone()),
                kind: TuringTapeKind::Policy,
                summary: match lang {
                    UiLang::Zh => format!("预算硬拦截 · {ticker}（0 LLM Token）"),
                    UiLang::En => format!("Budget blocked · {ticker} (0 LLM tokens)"),
                },
                detail: Some(reason.clone()),
                status: "blocked_budget".to_owned(),
            });
            DeepResearchEventEffect {
                reload_context: true,
                banner: Some((
                    match lang {
                        UiLang::Zh => format!("{ticker} Deep Research 已被预算闸门拦截：{reason}"),
                        UiLang::En => {
                            format!("{ticker} Deep Research blocked by budget gate: {reason}")
                        }
                    },
                    true,
                )),
            }
        }
        DeepResearchEvent::PerspectiveStarted { perspective, label } => {
            let localized_label = perspective_title_for(perspective, label, lang);
            *state.status_note = Some(match lang {
                UiLang::Zh => format!("4 视角并行推进中 · {localized_label}…"),
                UiLang::En => format!("4-perspective run in progress · {localized_label}…"),
            });
            if let Some(p) = state
                .perspectives
                .iter_mut()
                .find(|item| item.id == *perspective)
            {
                p.status = PerspectiveUiStatus::Running;
            }
            DeepResearchEventEffect {
                reload_context: false,
                banner: None,
            }
        }
        DeepResearchEvent::ToolStarted {
            perspective,
            name,
            title,
        } => {
            if let Some(p) = state
                .perspectives
                .iter_mut()
                .find(|item| item.id == *perspective)
                && !p.tools_used.iter().any(|t| t == name)
            {
                p.tools_used.push(name.clone());
            }
            let seq = state.tape.len() + 1;
            state.tape.push(TuringTapeEntry {
                seq,
                call_id: Some(format!("dr-{perspective}-{name}-{seq}")),
                kind: TuringTapeKind::Tool,
                summary: if title.is_empty() || title == name {
                    format!("[{perspective}] {name}")
                } else {
                    format!("[{perspective}] {name} · {title}")
                },
                detail: None,
                status: "running".to_owned(),
            });
            DeepResearchEventEffect {
                reload_context: false,
                banner: None,
            }
        }
        DeepResearchEvent::ToolFinished {
            perspective,
            name,
            status,
        } => {
            let is_denied = status.contains("denied") || status.contains("blocked");
            let prefix = format!("[{perspective}] {name}");
            if let Some(row) = state
                .tape
                .iter_mut()
                .rev()
                .find(|r| r.summary.starts_with(&prefix) && r.status == "running")
            {
                row.status = status.clone();
                if is_denied {
                    row.kind = TuringTapeKind::Policy;
                }
            } else {
                let seq = state.tape.len() + 1;
                state.tape.push(TuringTapeEntry {
                    seq,
                    call_id: None,
                    kind: if is_denied {
                        TuringTapeKind::Policy
                    } else {
                        TuringTapeKind::Tool
                    },
                    summary: prefix,
                    detail: None,
                    status: status.clone(),
                });
            }
            DeepResearchEventEffect {
                reload_context: false,
                banner: None,
            }
        }
        DeepResearchEvent::PerspectiveDelta { perspective, chunk } => {
            if let Some(p) = state
                .perspectives
                .iter_mut()
                .find(|item| item.id == *perspective)
            {
                p.status = PerspectiveUiStatus::Running;
                p.stream_text.push_str(chunk);
            }
            DeepResearchEventEffect {
                reload_context: false,
                banner: None,
            }
        }
        DeepResearchEvent::PerspectiveFinished {
            perspective,
            summary_chars,
        } => {
            if let Some(p) = state
                .perspectives
                .iter_mut()
                .find(|item| item.id == *perspective)
            {
                p.status = PerspectiveUiStatus::Completed;
                p.summary_chars = *summary_chars;
            }
            DeepResearchEventEffect {
                reload_context: false,
                banner: None,
            }
        }
        DeepResearchEvent::Synthesizing { ticker } => {
            *state.status_note = Some(match lang {
                UiLang::Zh => {
                    format!("正在合成 {ticker} Living Research Memo（保留 ## Human Notes）…")
                }
                UiLang::En => {
                    format!(
                        "Synthesizing {ticker} Living Research Memo (preserving ## Human Notes)…"
                    )
                }
            });
            DeepResearchEventEffect {
                reload_context: false,
                banner: None,
            }
        }
        DeepResearchEvent::Completed { result } => {
            *state.running = false;
            *state.run_id = Some(result.run_id.clone());
            *state.last_result = Some((**result).clone());
            for outcome in &result.perspectives {
                if let Some(p) = state
                    .perspectives
                    .iter_mut()
                    .find(|item| item.id == outcome.perspective)
                {
                    p.status = PerspectiveUiStatus::Completed;
                    p.tools_used = outcome.tools_used.clone();
                    if p.stream_text.is_empty() {
                        p.stream_text = outcome.content.clone();
                    }
                    p.summary_chars = outcome.content.chars().count();
                    p.input_tokens = outcome.input_tokens;
                    p.output_tokens = outcome.output_tokens;
                }
            }
            let cost_str = result.cost_usd.round_dp(4).normalize();
            let memo_label = result.memo_path.as_deref().unwrap_or("workbench/memos");
            *state.status_note = Some(match lang {
                UiLang::Zh => format!(
                    "完成 · {} in / {} out · ${cost_str} · {memo_label}",
                    result.input_tokens, result.output_tokens
                ),
                UiLang::En => format!(
                    "Completed · {} in / {} out · ${cost_str} · {memo_label}",
                    result.input_tokens, result.output_tokens
                ),
            });
            let seq = state.tape.len() + 1;
            state.tape.push(TuringTapeEntry {
                seq,
                call_id: Some(result.run_id.clone()),
                kind: TuringTapeKind::Out,
                summary: match lang {
                    UiLang::Zh => format!(
                        "Deep Research 完成 · {} (${cost_str} · {} 引用)",
                        result.ticker,
                        result.citations.len()
                    ),
                    UiLang::En => format!(
                        "Deep Research completed · {} (${cost_str} · {} citations)",
                        result.ticker,
                        result.citations.len()
                    ),
                },
                detail: Some(memo_label.to_owned()),
                status: "completed".to_owned(),
            });
            DeepResearchEventEffect {
                reload_context: true,
                banner: Some((
                    match lang {
                        UiLang::Zh => format!(
                            "{} Deep Research 完成（{} in / {} out · ${cost_str}）· 已写入 {memo_label}",
                            result.ticker, result.input_tokens, result.output_tokens
                        ),
                        UiLang::En => format!(
                            "{} Deep Research complete ({} in / {} out · ${cost_str}) · wrote {memo_label}",
                            result.ticker, result.input_tokens, result.output_tokens
                        ),
                    },
                    false,
                )),
            }
        }
        DeepResearchEvent::Failed {
            run_id,
            ticker,
            error,
        } => {
            *state.running = false;
            if !run_id.is_empty() {
                *state.run_id = Some(run_id.clone());
            }
            *state.status_note = Some(match lang {
                UiLang::Zh => format!("Deep Research 未完成：{error}"),
                UiLang::En => format!("Deep Research incomplete: {error}"),
            });
            for p in state.perspectives.iter_mut() {
                if matches!(
                    p.status,
                    PerspectiveUiStatus::Pending | PerspectiveUiStatus::Running
                ) {
                    p.status = PerspectiveUiStatus::Failed;
                }
            }
            let seq = state.tape.len() + 1;
            state.tape.push(TuringTapeEntry {
                seq,
                call_id: (!run_id.is_empty()).then(|| run_id.clone()),
                kind: TuringTapeKind::Error,
                summary: match lang {
                    UiLang::Zh => format!("Deep Research 失败 · {ticker}"),
                    UiLang::En => format!("Deep Research failed · {ticker}"),
                },
                detail: Some(error.clone()),
                status: "failed".to_owned(),
            });
            DeepResearchEventEffect {
                reload_context: true,
                banner: Some((
                    match lang {
                        UiLang::Zh => format!("{ticker} Deep Research 失败：{error}"),
                        UiLang::En => format!("{ticker} Deep Research failed: {error}"),
                    },
                    true,
                )),
            }
        }
    }
}

impl MungerWorkbench {
    pub fn is_alan_streaming(&self) -> bool {
        self.agent_streaming || self.deep_research_running
    }

    pub(super) fn reload_budget_from_lake(&mut self) {
        self.budget_config = crate::budget::load_budget_config(&self.paths).unwrap_or_default();
        if self.universe_syncing {
            return;
        }
        if let Ok(report) = cli::load_budget_status_report(&self.paths) {
            self.budget_report = Some(report);
        }
    }

    pub(super) fn reload_research_desk_context(&mut self, ticker: &str) {
        let upper = ticker.trim().to_ascii_uppercase();
        self.selected_memo_raw = research::load_research_memo(&self.paths, &upper);
        self.selected_memo_parsed = self.selected_memo_raw.as_deref().map(parse_research_memo);

        if !self.universe_syncing
            && let Ok(Some(db)) = LakeDb::open_read_only_if_exists(&self.paths)
        {
            self.selected_research_runs =
                db.list_research_runs(Some(&upper), 8).unwrap_or_default();
            self.selected_peers = db.query_peers(&upper, 6).ok();
            self.selected_similar_hits = db
                .search_similar(
                    &upper,
                    true,
                    &SimilarSearchFilter {
                        exclude_financials: true,
                        limit: 5,
                        ..Default::default()
                    },
                )
                .unwrap_or_default();
        } else if self.universe_syncing {
            // Keep existing lake state while universe sync owns the DuckDB lock.
        } else {
            self.selected_research_runs.clear();
            self.selected_peers = None;
            self.selected_similar_hits.clear();
        }

        self.reload_budget_from_lake();
    }

    pub(super) fn start_deep_research(
        &mut self,
        ticker: &str,
        focus: Option<String>,
        trigger: &str,
        cx: &mut Context<Self>,
    ) {
        let upper = ticker.trim().to_ascii_uppercase();
        if upper.is_empty() {
            return;
        }
        if self.deep_research_running {
            self.banner = Some((
                self.lang
                    .t(
                        "Deep Research 任务正在执行中，请等待当前运行完成",
                        "Deep Research is already running; wait for completion.",
                    )
                    .into(),
                false,
            ));
            cx.notify();
            return;
        }
        if self.universe_syncing {
            self.banner = Some((
                self.lang
                    .t(
                        "全美股 L0 湖仓正在同步，请完成后再发起 Deep Research",
                        "US universe L0 lakehouse is syncing; start Deep Research after it finishes.",
                    )
                    .into(),
                false,
            ));
            cx.notify();
            return;
        }

        self.deep_research_running = true;
        self.deep_research_ticker = Some(upper.clone());
        self.deep_research_run_id = None;
        self.deep_research_budget_blocked = None;
        self.deep_research_perspectives = default_deep_research_perspectives();
        self.deep_research_status_note = Some(match self.lang {
            UiLang::Zh => format!("正在准备 {upper} · 4 视角并行深研…"),
            UiLang::En => format!("Preparing {upper} · 4-perspective Deep Research…"),
        });

        let root = self.paths.repo_root.clone();
        let req = DeepResearchRequest {
            ticker: upper.clone(),
            focus,
            trigger: trigger.to_owned(),
        };
        let (tx, rx) = mpsc::channel();
        self.deep_research_rx = Some(rx);

        std::thread::spawn(move || {
            let paths = WorkspacePaths::from_root(root);
            if let Err(err) = research::run_deep_research(&paths, &req, Some(tx.clone())) {
                let _ = tx.send(DeepResearchEvent::Failed {
                    run_id: String::new(),
                    ticker: req.ticker,
                    error: format!("{err:#}"),
                });
            }
        });

        cx.notify();
    }

    pub(super) fn poll_deep_research_events(&mut self) -> bool {
        let Some(rx) = self.deep_research_rx.take() else {
            return false;
        };
        let mut events = Vec::new();
        let mut disconnected = false;
        loop {
            match rx.try_recv() {
                Ok(ev) => events.push(ev),
                Err(mpsc::TryRecvError::Empty) => {
                    self.deep_research_rx = Some(rx);
                    break;
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    disconnected = true;
                    break;
                }
            }
        }

        if events.is_empty() {
            if disconnected && self.deep_research_running {
                self.deep_research_running = false;
                return true;
            }
            return false;
        }

        let mut reload_ctx = false;
        let mut latest_banner: Option<(String, bool)> = None;
        for ev in &events {
            let mut slice = DeepResearchUiSlice {
                running: &mut self.deep_research_running,
                run_id: &mut self.deep_research_run_id,
                status_note: &mut self.deep_research_status_note,
                budget_blocked: &mut self.deep_research_budget_blocked,
                perspectives: &mut self.deep_research_perspectives,
                last_result: &mut self.deep_research_last_result,
                tape: &mut self.agent_tape,
            };
            let effect = apply_deep_research_event_for(&mut slice, ev, self.lang);
            if effect.reload_context {
                reload_ctx = true;
            }
            if let Some(b) = effect.banner {
                latest_banner = Some(b);
            }
        }

        if let Some(b) = latest_banner {
            self.banner = Some(b);
        }
        if reload_ctx {
            let target = self
                .deep_research_ticker
                .clone()
                .unwrap_or_else(|| self.selected_ticker.clone());
            self.reload_research_desk_context(&target);
        }
        true
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
        self.agent_status_hint = Some(
            self.lang
                .t(
                    "正在连接 EmbeddedAlan V2…",
                    "Connecting to EmbeddedAlan V2…",
                )
                .to_string(),
        );

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
        let starter = match self.lang {
            UiLang::Zh => format!(
                "请使用 m3_report 与 m3_dossier 获取 {ticker}（{name}）的确定性财务报告与现有审讯档案，指出当前估值与护城河中最脆弱的核心假设，并向我发起一轮费曼审讯追问。"
            ),
            UiLang::En => format!(
                "Please use m3_report and m3_dossier to fetch the deterministic financial report and existing interrogation dossier for {ticker} ({name}), identify the most fragile assumptions in current valuation and moat, and initiate a round of Feynman interrogation."
            ),
        };
        self.submit_alan_question(ticker, name, bucket, starter);
    }

    pub(super) fn interrupt_alan(&mut self) {
        if let Some(handle) = &self.alan_handle {
            handle.interrupt();
        }
        self.agent_streaming = false;
        self.agent_status_hint = Some(
            self.lang
                .t("已中断当前审讯", "Interrogation interrupted")
                .to_string(),
        );
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
        let mut dirty = self.poll_deep_research_events();
        let Some(handle) = self.alan_handle.as_ref() else {
            return dirty;
        };
        let events = handle.drain_events();
        if events.is_empty() {
            return dirty;
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
            if apply_interrogator_event_for(
                &mut slice,
                &ev,
                &focus_ticker,
                &self.reports,
                &dossier_map,
                self.lang,
            ) {
                reload_disk = true;
            }
        }

        if reload_disk {
            self.reload_from_disk();
        }
        dirty = true;
        dirty
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

        canvas_el = canvas_el
            .child(self.render_deep_research_console(rep, cx))
            .child(self.render_cross_company_context_strip(&rep.ticker, cx))
            .child(self.render_living_research_memo_card(&rep.ticker, cx))
            .child(self.render_research_runs_history_card(&rep.ticker));

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
                                .child(match self.lang {
                                    UiLang::Zh => format!("AI 幕僚草稿 · {draft_source}"),
                                    UiLang::En => format!("AI Staff Draft · {draft_source}"),
                                }),
                        ),
                )
                .child(
                    div()
                        .font_family(self.fonts.mono_num.clone())
                        .text_xs()
                        .text_color(rgb(INK_MUTED))
                        .child(match self.lang {
                            UiLang::Zh => format!("更新于 {updated_at}"),
                            UiLang::En => format!("Updated {updated_at}"),
                        }),
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
                            .child(
                                self.lang
                                    .t("能力圈三句话过滤", "3-Sentence Business Filter"),
                            ),
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
                            .child(
                                self.lang
                                    .t("跨学科思维格栅", "Interdisciplinary Latticework"),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .border_b_1()
                            .border_color(rgb(BORDER_HAIRLINE))
                            .child(
                                quad(
                                    self.lang.t("微观经济与护城河", "Microeconomics & Moat"),
                                    &dossier.latticework.microeconomics,
                                )
                                .border_r_1()
                                .border_color(rgb(BORDER_HAIRLINE)),
                            )
                            .child(quad(
                                self.lang
                                    .t("生态位与技术演化", "Ecosystem & Tech Evolution"),
                                &dossier.latticework.ecology,
                            )),
                    )
                    .child(
                        div()
                            .flex()
                            .child(
                                quad(
                                    self.lang.t("心理学与管理层激励", "Psychology & Incentives"),
                                    &dossier.latticework.psychology_incentives,
                                )
                                .border_r_1()
                                .border_color(rgb(BORDER_HAIRLINE)),
                            )
                            .child(quad(
                                self.lang.t("资本配置历史", "Capital Allocation History"),
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
                                    .child(self.lang.t(
                                        "5 年逆向事前验尸（亏损 >50% 路径）",
                                        "5-Year Pre-Mortem (>50% Drawdown Paths)",
                                    )),
                            )
                            .when(dossier.premortem.is_empty(), |d| {
                                d.child(
                                    div()
                                        .pt_3()
                                        .font_family(self.fonts.ui.clone())
                                        .text_xs()
                                        .text_color(rgb(INK_MUTED))
                                        .child(self.lang.t(
                                            "（尚无事前验尸草稿）",
                                            "(No pre-mortem draft yet)",
                                        )),
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
                                            .child(self.lang.t(
                                                "费曼待答辩关键质疑",
                                                "Open Feynman Interrogation Questions",
                                            )),
                                    )
                                    .child(if pending {
                                        badge(
                                            self.lang.t("待作答", "Open").into(),
                                            GOLD_FG,
                                            GOLD_BG,
                                            GOLD_BORDER,
                                            &self.fonts,
                                        )
                                    } else {
                                        badge(
                                            self.lang.t("已通过", "Cleared").into(),
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
                    .child(match self.lang {
                        UiLang::Zh => format!(
                            "尚无 {} 审讯档案（{dossier_file}），可点击右上角「复制审讯 Prompt (p)」启动。",
                            rep.ticker
                        ),
                        UiLang::En => format!(
                            "No interrogation dossier for {} ({dossier_file}). Click \"Copy Prompt (p)\" in the top right to start.",
                            rep.ticker
                        ),
                    }),
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
                self.agent_status_hint.clone().unwrap_or_else(|| {
                    self.lang
                        .t("Alan 审讯运行中…", "Alan interrogation running…")
                        .into()
                }),
                VIOLET_FG,
                VIOLET_BG,
                VIOLET_BORDER,
                &self.fonts,
            )
        } else {
            badge(
                self.lang
                    .t(
                        "只读 /mnt/m3 · 确定性工具",
                        "Read-only /mnt/m3 · Deterministic Tools",
                    )
                    .into(),
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
                            .child(match self.lang {
                                UiLang::Zh => format!("Alan 费曼审讯台 · {}", rep.ticker),
                                UiLang::En => format!("Alan Feynman Desk · {}", rep.ticker),
                            }),
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
                    .child(self.lang.t("中断生成", "Interrupt")),
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
                    .child(self.lang.t("启动费曼审讯", "Start Feynman Session")),
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
                    .child(self.lang.t("清空", "Clear")),
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
                match self.lang {
                    UiLang::Zh => format!(
                        "点击输入对 {} 的费曼追问（按 Enter 发送，Esc 退出输入）…",
                        rep.ticker
                    ),
                    UiLang::En => format!(
                        "Click to ask {} a Feynman follow-up (Enter to send, Esc to blur)…",
                        rep.ticker
                    ),
                }
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
                    .child(self.lang.t("发送", "Send")),
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
                    .child(self.lang.t(
                        "点击「启动费曼审讯」或在上方输入追问，直接驱动进程内 EmbeddedAlan V2；工具受红线策略钩子硬保护（严禁触碰 human_judgment 与 computed/*），右上角「复制审讯 Prompt」仍可作为外部会话兜底。",
                        "Click \"Start Feynman Session\" or enter a follow-up above to drive in-process EmbeddedAlan V2 directly; tools are enforced by redline policy hooks (blocking human_judgment and computed/*), and \"Copy Prompt\" remains available as an external fallback.",
                    )),
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
                                .child(match self.lang {
                                    UiLang::Zh => format!("你  ·  {}", msg.text),
                                    UiLang::En => format!("You  ·  {}", msg.text),
                                }),
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
                            self.lang
                                .t(
                                    "Alan 正在读取 /mnt/m3 与确定性报告…",
                                    "Alan is reading /mnt/m3 and deterministic reports…",
                                )
                                .to_string()
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
                                    .child(match self.lang {
                                        UiLang::Zh => format!("Acme 证据核验 · {}", tok.label),
                                        UiLang::En => {
                                            format!("Acme Evidence Verification · {}", tok.label)
                                        }
                                    }),
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
                                                .child(self.lang.t(
                                                    "跳至 3 XBRL 底表 →",
                                                    "Jump to 3 XBRL Schedule →",
                                                )),
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
                                            .child(self.lang.t("收起", "Collapse")),
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

    pub(super) fn render_deep_research_console(
        &self,
        rep: &MungerReport,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        let dispatch_ticker = rep.ticker.clone();
        let active_for_ticker = self
            .deep_research_ticker
            .as_deref()
            .is_some_and(|t| t.eq_ignore_ascii_case(&rep.ticker));

        let (budget_txt, budget_fg, budget_bg, budget_br) = if let Some(b) = &self.budget_report {
            let used = b.usage.total_cost_usd.round_dp(2).normalize();
            let limit = b.config.monthly_budget_usd.round_dp(2).normalize();
            let exhausted =
                b.remaining_monthly_usd <= Decimal::ZERO || b.remaining_monthly_tokens == 0;
            let (fg, bg, br) = if exhausted {
                (CORAL_FG, CORAL_BG, CORAL_BORDER)
            } else if b.config.monthly_budget_usd > Decimal::ZERO
                && used * Decimal::from(10) >= b.config.monthly_budget_usd * Decimal::from(8)
            {
                (GOLD_FG, GOLD_BG, GOLD_BORDER)
            } else {
                (EMERALD_FG, EMERALD_BG, EMERALD_BORDER)
            };
            (
                match self.lang {
                    UiLang::Zh => format!("本月 AI 预算 ${used:.2} / ${limit:.2}"),
                    UiLang::En => format!("Monthly AI Budget ${used:.2} / ${limit:.2}"),
                },
                fg,
                bg,
                br,
            )
        } else {
            (
                self.lang
                    .t(
                        "本月 AI 预算 $0.00 / $10.00",
                        "Monthly AI Budget $0.00 / $10.00",
                    )
                    .to_owned(),
                EMERALD_FG,
                EMERALD_BG,
                EMERALD_BORDER,
            )
        };

        let status_badge = if self.deep_research_running {
            badge(
                self.lang
                    .t("4 视角并行分析中…", "4-perspective analysis running…")
                    .into(),
                VIOLET_FG,
                VIOLET_BG,
                VIOLET_BORDER,
                &self.fonts,
            )
        } else if active_for_ticker && self.deep_research_budget_blocked.is_some() {
            badge(
                self.lang.t("预算闸门已拦截", "Budget Gate Blocked").into(),
                CORAL_FG,
                CORAL_BG,
                CORAL_BORDER,
                &self.fonts,
            )
        } else if active_for_ticker
            && self
                .deep_research_last_result
                .as_ref()
                .is_some_and(|r| r.status == "completed")
        {
            badge(
                self.lang.t("最近运行已完成", "Latest Run Complete").into(),
                EMERALD_FG,
                EMERALD_BG,
                EMERALD_BORDER,
                &self.fonts,
            )
        } else {
            badge(
                self.lang
                    .t(
                        "受预算硬门控 · 只读湖仓工具",
                        "Budget Hard-Gated · Read-only Lakehouse Tools",
                    )
                    .into(),
                SKY_FG,
                SKY_BG,
                SKY_BORDER,
                &self.fonts,
            )
        };

        let mut card = card_shell().child(
            div()
                .px_4()
                .py_3()
                .bg(rgb(BG_WELL))
                .border_b_1()
                .border_color(rgb(BORDER_HAIRLINE))
                .flex()
                .items_center()
                .justify_between()
                .gap_3()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .flex_wrap()
                        .gap_2()
                        .child(div().w(px(6.)).h(px(6.)).rounded_full().bg(rgb(VIOLET_FG)))
                        .child(
                            div()
                                .font_family(self.fonts.ui.clone())
                                .text_xs()
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .text_color(rgb(INK_HERO))
                                .child(format!("Research Desk · Deep Research ({})", rep.ticker)),
                        )
                        .child(status_badge)
                        .child(badge(
                            budget_txt,
                            budget_fg,
                            budget_bg,
                            budget_br,
                            &self.fonts,
                        )),
                )
                .child(
                    div()
                        .id("dr-dispatch-btn")
                        .px_3()
                        .py_1p5()
                        .rounded_sm()
                        .cursor_pointer()
                        .border_1()
                        .when(self.deep_research_running, |d| {
                            d.bg(rgb(BG_SURFACE))
                                .border_color(rgb(BORDER_SUBTLE))
                                .text_color(rgb(INK_MUTED))
                        })
                        .when(!self.deep_research_running, |d| {
                            d.bg(rgb(CTA_BG))
                                .border_color(rgb(CTA_BG))
                                .hover(|s| s.bg(rgb(CTA_BG_HOVER)))
                                .active(|s| s.bg(rgb(CTA_BG_ACTIVE)))
                                .text_color(rgb(CTA_INK))
                        })
                        .font_family(self.fonts.ui.clone())
                        .text_xs()
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.start_deep_research(&dispatch_ticker, None, "manual", cx);
                        }))
                        .child(if self.deep_research_running {
                            self.lang
                                .t("4 视角并行运行中…", "4-Perspective Run Active…")
                        } else {
                            self.lang.t(
                                "Deep Research · 4 视角并行",
                                "Deep Research · 4 Perspectives",
                            )
                        }),
                ),
        );

        if active_for_ticker && let Some(reason) = &self.deep_research_budget_blocked {
            card = card.child(
                div()
                    .px_4()
                    .py_2p5()
                    .bg(rgba(CORAL_BG))
                    .border_b_1()
                    .border_color(rgba(CORAL_BORDER))
                    .font_family(self.fonts.ui.clone())
                    .text_xs()
                    .text_color(rgb(CORAL_FG))
                    .child(match self.lang {
                        UiLang::Zh => format!("预算硬闸门已拦截（0 LLM Token 消耗）：{reason}"),
                        UiLang::En => {
                            format!("Hard budget gate blocked (0 LLM tokens used): {reason}")
                        }
                    }),
            );
        } else if active_for_ticker && let Some(note) = &self.deep_research_status_note {
            card = card.child(
                div()
                    .px_4()
                    .py_2()
                    .bg(rgb(BG_ROW_ALT))
                    .border_b_1()
                    .border_color(rgb(BORDER_HAIRLINE))
                    .font_family(self.fonts.mono_term.clone())
                    .text_size(px(11.))
                    .text_color(rgb(INK_SECONDARY))
                    .child(note.clone()),
            );
        }

        let perspectives: Vec<DeepResearchPerspectiveUi> =
            if active_for_ticker && !self.deep_research_perspectives.is_empty() {
                self.deep_research_perspectives.clone()
            } else {
                default_deep_research_perspectives()
            };

        let left_pair: Vec<&DeepResearchPerspectiveUi> = perspectives.iter().take(2).collect();
        let right_pair: Vec<&DeepResearchPerspectiveUi> =
            perspectives.iter().skip(2).take(2).collect();

        let lang = self.lang;
        let render_p_card = |p: &DeepResearchPerspectiveUi| {
            let (st_fg, st_bg, st_br) = match p.status {
                PerspectiveUiStatus::Pending => (INK_MUTED, BG_WELL, BORDER_SUBTLE),
                PerspectiveUiStatus::Running => (VIOLET_FG, VIOLET_BG, VIOLET_BORDER),
                PerspectiveUiStatus::Completed => (EMERALD_FG, EMERALD_BG, EMERALD_BORDER),
                PerspectiveUiStatus::Failed => (CORAL_FG, CORAL_BG, CORAL_BORDER),
            };
            let excerpt: String = if p.stream_text.trim().is_empty() {
                match p.status {
                    PerspectiveUiStatus::Pending => lang
                        .t(
                            "点击「Deep Research · 4 视角并行」后，由独立只读子会话并行核验一手的财报/公告证据。",
                            "Click \"Deep Research · 4 Perspectives\" to launch parallel read-only sub-sessions verifying primary filings and disclosures.",
                        )
                        .into(),
                    PerspectiveUiStatus::Running => lang
                        .t(
                            "正在检索 Lakehouse 工具与生成分析结论…",
                            "Querying Lakehouse tools and synthesizing findings…",
                        )
                        .into(),
                    PerspectiveUiStatus::Completed => match lang {
                        UiLang::Zh => format!(
                            "已完成（{} 字符，已合并至 Living Research Memo）",
                            p.summary_chars
                        ),
                        UiLang::En => format!(
                            "Completed ({} chars, merged into Living Research Memo)",
                            p.summary_chars
                        ),
                    },
                    PerspectiveUiStatus::Failed => lang
                        .t(
                            "未产生输出（已保留既有工作区状态）",
                            "No output produced (existing workspace preserved)",
                        )
                        .into(),
                }
            } else {
                let chars: Vec<char> = p.stream_text.trim().chars().collect();
                if chars.len() > 220 {
                    format!("{}…", chars[..220].iter().collect::<String>())
                } else {
                    p.stream_text.trim().to_owned()
                }
            };
            let tools_line = if p.tools_used.is_empty() {
                "tools: —".to_owned()
            } else {
                format!("tools: {}", p.tools_used.join(", "))
            };
            let tok_line = if p.input_tokens > 0 || p.output_tokens > 0 {
                format!("{}in / {}out", p.input_tokens, p.output_tokens)
            } else if p.summary_chars > 0 {
                format!("{} chars", p.summary_chars)
            } else {
                p.id.clone()
            };

            div()
                .flex_1()
                .min_w_0()
                .p_3()
                .rounded_md()
                .bg(rgb(BG_VIEWPORT))
                .border_1()
                .border_color(rgb(BORDER_HAIRLINE))
                .flex()
                .flex_col()
                .gap_1p5()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .gap_2()
                        .child(
                            div()
                                .font_family(self.fonts.ui.clone())
                                .text_xs()
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .text_color(rgb(INK_HERO))
                                .child(perspective_title_for(&p.id, &p.title_cn, lang)),
                        )
                        .child(badge(
                            p.status.label_for(lang).into(),
                            st_fg,
                            st_bg,
                            st_br,
                            &self.fonts,
                        )),
                )
                .child(
                    div()
                        .font_family(self.fonts.ui.clone())
                        .text_xs()
                        .line_height(px(18.))
                        .text_color(if p.stream_text.trim().is_empty() {
                            rgb(INK_MUTED)
                        } else {
                            rgb(INK_PRIMARY)
                        })
                        .child(excerpt),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .gap_2()
                        .font_family(self.fonts.mono_term.clone())
                        .text_size(px(10.))
                        .text_color(rgb(INK_MUTED))
                        .child(div().truncate().child(tools_line))
                        .child(tok_line),
                )
        };

        card.child(
            div()
                .p_4()
                .flex()
                .flex_col()
                .gap_2p5()
                .child(
                    div()
                        .flex()
                        .gap_2p5()
                        .children(left_pair.into_iter().map(render_p_card)),
                )
                .child(
                    div()
                        .flex()
                        .gap_2p5()
                        .children(right_pair.into_iter().map(render_p_card)),
                ),
        )
    }

    pub(super) fn render_cross_company_context_strip(
        &self,
        ticker: &str,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        let has_peers = self
            .selected_peers
            .as_ref()
            .is_some_and(|p| p.target.ticker.eq_ignore_ascii_case(ticker) && p.peer_count > 1);
        let has_similar = !self.selected_similar_hits.is_empty();

        let mut card = card_shell()
            .p_4()
            .flex()
            .flex_col()
            .gap_3()
            .child(section_header(
                self.lang.t(
                    "跨公司对照 · 同业分位数 & 隐空间 Lookalike",
                    "Cross-Company Comparison · Peer Percentiles & Latent Lookalikes",
                ),
                if has_peers || has_similar {
                    "DUCKDB L0/L1 · 0 LLM TOKEN".into()
                } else {
                    self.lang
                        .t("等待 L0/L1 湖仓数据", "Awaiting L0/L1 Lakehouse Data")
                        .into()
                },
                &self.fonts,
            ));

        if !has_peers && !has_similar {
            return card.child(
                div()
                    .font_family(self.fonts.ui.clone())
                    .text_xs()
                    .text_color(rgb(INK_MUTED))
                    .child(self.lang.t(
                        "同步全美股 L0/L1 湖仓后，此处展示 SIC 同业 ROIC/毛利/FCF 分位数与语义 Lookalike 近邻公司。",
                        "After syncing US L0/L1 lakehouse, SIC peer ROIC/gross margin/strict FCF percentiles and semantic lookalikes appear here.",
                    )),
            );
        }

        if let Some(peers) = self
            .selected_peers
            .as_ref()
            .filter(|p| p.target.ticker.eq_ignore_ascii_case(ticker) && p.peer_count > 1)
        {
            let roic_p = peers
                .roic_percentile
                .map(|v| format!("P{v:.0}"))
                .unwrap_or_else(|| "—".into());
            let gm_p = peers
                .gross_margin_percentile
                .map(|v| format!("P{v:.0}"))
                .unwrap_or_else(|| "—".into());
            let fcf_p = peers
                .strict_fcf_yield_percentile
                .map(|v| format!("P{v:.0}"))
                .unwrap_or_else(|| "—".into());

            card = card.child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .flex_wrap()
                    .gap_2()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .flex_wrap()
                            .gap_2()
                            .child(badge(
                                match self.lang {
                                    UiLang::Zh => {
                                        format!("{} · {} 家同行", peers.sic_group, peers.peer_count)
                                    }
                                    UiLang::En => {
                                        format!("{} · {} peers", peers.sic_group, peers.peer_count)
                                    }
                                },
                                SKY_FG,
                                SKY_BG,
                                SKY_BORDER,
                                &self.fonts,
                            ))
                            .child(
                                div()
                                    .font_family(self.fonts.mono_num.clone())
                                    .text_xs()
                                    .text_color(rgb(INK_PRIMARY))
                                    .child(match self.lang {
                                        UiLang::Zh => format!(
                                            "ROIC {roic_p} · 毛利率 {gm_p} · 严苛 FCF {fcf_p}"
                                        ),
                                        UiLang::En => format!(
                                            "ROIC {roic_p} · Gross Margin {gm_p} · Strict FCF {fcf_p}"
                                        ),
                                    }),
                            ),
                    )
                    .child(div().flex().items_center().flex_wrap().gap_1p5().children(
                        peers.top_peers.iter().take(5).enumerate().map(|(i, p)| {
                            let jump = p.ticker.clone();
                            let roic_s = p
                                .roic_5y_avg_pct
                                .map(|v| format!("{v:.0}%"))
                                .unwrap_or_else(|| "—".into());
                            div()
                                .id(("peer-chip", i))
                                .px_2()
                                .py_0p5()
                                .rounded_xs()
                                .bg(rgb(BG_WELL))
                                .border_1()
                                .border_color(rgb(BORDER_SUBTLE))
                                .hover(|s| s.bg(rgb(BG_HOVER)))
                                .cursor_pointer()
                                .font_family(self.fonts.mono_num.clone())
                                .text_size(px(11.))
                                .text_color(rgb(INK_SECONDARY))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.open_theme_member(&jump, cx);
                                }))
                                .child(format!("{} {roic_s}", p.ticker))
                        }),
                    )),
            );
        }

        if has_similar {
            card = card.child(
                div()
                    .pt_2()
                    .border_t_1()
                    .border_color(rgb(BORDER_HAIRLINE))
                    .flex()
                    .items_center()
                    .flex_wrap()
                    .gap_1p5()
                    .child(
                        div()
                            .font_family(self.fonts.ui.clone())
                            .text_xs()
                            .text_color(rgb(INK_MUTED))
                            .child(self.lang.t("隐空间 Lookalike:", "Latent Lookalike:")),
                    )
                    .children(
                        self.selected_similar_hits
                            .iter()
                            .enumerate()
                            .map(|(i, hit)| {
                                let jump = hit.company.ticker.clone();
                                let sim_pct = (hit.score * 100.0).round() as i64;
                                let doc_type =
                                    hit.evidence.first().map_or("10-K", |e| e.doc_type.as_str());
                                div()
                                    .id(("sim-chip", i))
                                    .px_2()
                                    .py_0p5()
                                    .rounded_xs()
                                    .bg(rgba(VIOLET_BG))
                                    .border_1()
                                    .border_color(rgba(VIOLET_BORDER))
                                    .hover(|s| s.bg(rgb(BG_HOVER)))
                                    .cursor_pointer()
                                    .font_family(self.fonts.mono_num.clone())
                                    .text_size(px(11.))
                                    .text_color(rgb(VIOLET_FG))
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.open_theme_member(&jump, cx);
                                    }))
                                    .child(format!(
                                        "{} · {}% · {}",
                                        hit.company.ticker, sim_pct, doc_type
                                    ))
                            }),
                    ),
            );
        }

        card
    }

    pub(super) fn render_living_research_memo_card(
        &self,
        ticker: &str,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        let memo_rel = format!("workbench/memos/{ticker}.md");
        let refresh_ticker = ticker.to_owned();
        let copy_raw = self.selected_memo_raw.clone();

        let header = div()
            .px_4()
            .py_2p5()
            .bg(rgb(BG_WELL))
            .border_b_1()
            .border_color(rgb(BORDER_HAIRLINE))
            .flex()
            .items_center()
            .justify_between()
            .gap_3()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(div().w(px(6.)).h(px(6.)).rounded_full().bg(rgb(EMERALD_FG)))
                    .child(
                        div()
                            .font_family(self.fonts.ui.clone())
                            .text_xs()
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .text_color(rgb(INK_HERO))
                            .child(format!("Living Research Memo · {memo_rel}")),
                    ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .when_some(copy_raw, |d, md| {
                        let lbl = format!("Memo {ticker}");
                        d.child(
                            div()
                                .id("memo-copy-md")
                                .px_2()
                                .py_1()
                                .rounded_sm()
                                .bg(rgb(BG_SURFACE))
                                .border_1()
                                .border_color(rgb(BORDER_PILL))
                                .hover(|s| s.bg(rgb(BG_HOVER)))
                                .cursor_pointer()
                                .font_family(self.fonts.mono_term.clone())
                                .text_size(px(11.))
                                .text_color(rgb(INK_SECONDARY))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.copy_cli_command(md.clone(), &lbl, cx);
                                    cx.notify();
                                }))
                                .child(self.lang.t("复制 Markdown", "Copy Markdown")),
                        )
                    })
                    .child(
                        div()
                            .id("memo-refresh-disk")
                            .px_2()
                            .py_1()
                            .rounded_sm()
                            .bg(rgb(BG_SURFACE))
                            .border_1()
                            .border_color(rgb(BORDER_PILL))
                            .hover(|s| s.bg(rgb(BG_HOVER)))
                            .cursor_pointer()
                            .font_family(self.fonts.ui.clone())
                            .text_size(px(11.))
                            .text_color(rgb(INK_PRIMARY))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.reload_research_desk_context(&refresh_ticker);
                                this.banner = Some((
                                    match this.lang {
                                        UiLang::Zh => {
                                            format!("已从磁盘刷新 {refresh_ticker} Memo")
                                        }
                                        UiLang::En => {
                                            format!("Reloaded {refresh_ticker} Memo from disk")
                                        }
                                    },
                                    false,
                                ));
                                cx.notify();
                            }))
                            .child(self.lang.t("刷新", "Refresh")),
                    ),
            );

        let mut card = card_shell().child(header);

        let Some(memo) = &self.selected_memo_parsed else {
            return card.child(
                div()
                    .p_4()
                    .font_family(self.fonts.ui.clone())
                    .text_xs()
                    .line_height(px(19.))
                    .text_color(rgb(INK_MUTED))
                    .child(match self.lang {
                        UiLang::Zh => format!(
                            "尚未生成 {memo_rel}。点击上方「Deep Research · 4 视角并行」或运行 `m3 research run {ticker}` 后将在此展示结构化活研报与一手引用。"
                        ),
                        UiLang::En => format!(
                            "{memo_rel} not generated yet. Click \"Deep Research · 4 Perspectives\" above or run `m3 research run {ticker}` to render the structured living memo and primary citations."
                        ),
                    }),
            );
        };

        if !memo.metadata.is_empty() {
            card = card.child(
                div()
                    .px_4()
                    .py_2()
                    .bg(rgb(BG_ROW_ALT))
                    .border_b_1()
                    .border_color(rgb(BORDER_HAIRLINE))
                    .flex()
                    .items_center()
                    .flex_wrap()
                    .gap_3()
                    .children(memo.metadata.iter().map(|(k, v)| {
                        div()
                            .flex()
                            .items_center()
                            .gap_1()
                            .child(
                                div()
                                    .font_family(self.fonts.mono_term.clone())
                                    .text_size(px(10.))
                                    .text_color(rgb(INK_MUTED))
                                    .child(format!("{k}:")),
                            )
                            .child(
                                div()
                                    .font_family(self.fonts.mono_num.clone())
                                    .text_size(px(10.))
                                    .text_color(rgb(INK_PRIMARY))
                                    .child(v.clone()),
                            )
                    })),
            );
        }

        let perspective_sections: Vec<&MemoSection> = memo
            .sections
            .iter()
            .filter(|s| {
                matches!(
                    s.id.as_str(),
                    "moat" | "accounting" | "latent_peers" | "premortem"
                )
            })
            .collect();

        let mut body = div().p_4().flex().flex_col().gap_3();
        for sec in perspective_sections {
            body = body.child(
                div()
                    .p_3()
                    .rounded_sm()
                    .bg(rgb(BG_VIEWPORT))
                    .border_1()
                    .border_color(rgb(BORDER_HAIRLINE))
                    .flex()
                    .flex_col()
                    .gap_1p5()
                    .child(
                        div()
                            .font_family(self.fonts.ui.clone())
                            .text_xs()
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .text_color(rgb(INK_HERO))
                            .child(sec.heading.clone()),
                    )
                    .child(
                        div()
                            .font_family(self.fonts.ui.clone())
                            .text_xs()
                            .line_height(px(20.))
                            .text_color(rgb(INK_SECONDARY))
                            .child(sec.body.clone()),
                    ),
            );
        }

        if !memo.citations.is_empty() {
            body = body.child(
                div()
                    .p_3()
                    .rounded_sm()
                    .bg(rgb(BG_VIEWPORT))
                    .border_1()
                    .border_color(rgb(BORDER_HAIRLINE))
                    .flex()
                    .flex_col()
                    .gap_1p5()
                    .child(
                        div()
                            .font_family(self.fonts.mono_term.clone())
                            .text_size(px(10.))
                            .font_weight(gpui::FontWeight::BOLD)
                            .text_color(rgb(SKY_FG))
                            .child(format!("EVIDENCE & CITATIONS ({})", memo.citations.len())),
                    )
                    .children(memo.citations.iter().take(8).map(|cite| {
                        div()
                            .font_family(self.fonts.ui.clone())
                            .text_size(px(11.))
                            .line_height(px(17.))
                            .text_color(rgb(INK_SECONDARY))
                            .child(cite.clone())
                    })),
            );
        }

        if let Some(notes) = &memo.human_notes {
            body = body.child(
                div()
                    .p_3()
                    .rounded_sm()
                    .bg(rgba(GOLD_BG))
                    .border_1()
                    .border_color(rgba(GOLD_BORDER))
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .font_family(self.fonts.ui.clone())
                            .text_xs()
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .text_color(rgb(GOLD_FG))
                            .child(self.lang.t(
                                "## Human Notes（人类主权区 · Deep Research 永不覆盖）",
                                "## Human Notes (Human-Owned · Never Overwritten by Deep Research)",
                            )),
                    )
                    .child(
                        div()
                            .font_family(self.fonts.ui.clone())
                            .text_xs()
                            .line_height(px(19.))
                            .text_color(rgb(INK_PRIMARY))
                            .child(notes.clone()),
                    ),
            );
        }

        card.child(body)
    }

    pub(super) fn render_research_runs_history_card(&self, ticker: &str) -> gpui::Div {
        let mut card = card_shell()
            .p_4()
            .flex()
            .flex_col()
            .gap_2p5()
            .child(section_header(
                &match self.lang {
                    UiLang::Zh => format!("Deep Research 审计记录 · {ticker}"),
                    UiLang::En => format!("Deep Research Audit Log · {ticker}"),
                },
                match self.lang {
                    UiLang::Zh => format!(
                        "{} 条最近运行（m3.duckdb · research_runs）",
                        self.selected_research_runs.len()
                    ),
                    UiLang::En => format!(
                        "{} recent runs (m3.duckdb · research_runs)",
                        self.selected_research_runs.len()
                    ),
                },
                &self.fonts,
            ));

        if self.selected_research_runs.is_empty() {
            return card.child(
                div()
                    .font_family(self.fonts.ui.clone())
                    .text_xs()
                    .text_color(rgb(INK_MUTED))
                    .child(self.lang.t(
                        "暂无该标的的 Deep Research 运行记录。",
                        "No Deep Research runs recorded for this ticker yet.",
                    )),
            );
        }

        for (idx, run) in self.selected_research_runs.iter().enumerate() {
            let (st_label, st_fg, st_bg, st_br) = match run.status {
                ResearchRunStatus::Completed => {
                    ("completed", EMERALD_FG, EMERALD_BG, EMERALD_BORDER)
                }
                ResearchRunStatus::BlockedBudget => {
                    ("blocked_budget", CORAL_FG, CORAL_BG, CORAL_BORDER)
                }
                ResearchRunStatus::Failed => ("failed", GOLD_FG, GOLD_BG, GOLD_BORDER),
                ResearchRunStatus::Running => ("running", VIOLET_FG, VIOLET_BG, VIOLET_BORDER),
            };
            let detail_txt = run
                .error
                .as_deref()
                .or(run.memo_path.as_deref())
                .unwrap_or("—")
                .to_owned();
            card = card.child(
                div()
                    .py_2()
                    .when(idx + 1 < self.selected_research_runs.len(), |d| {
                        d.border_b_1().border_color(rgb(BORDER_HAIRLINE))
                    })
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_3()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .min_w_0()
                            .child(badge(st_label.into(), st_fg, st_bg, st_br, &self.fonts))
                            .child(
                                div()
                                    .font_family(self.fonts.mono_term.clone())
                                    .text_size(px(11.))
                                    .text_color(rgb(INK_PRIMARY))
                                    .child(run.run_id.clone()),
                            )
                            .child(
                                div()
                                    .font_family(self.fonts.mono_term.clone())
                                    .text_size(px(10.))
                                    .text_color(rgb(INK_MUTED))
                                    .child(format!("[{}]", run.trigger)),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .font_family(self.fonts.mono_term.clone())
                                    .text_size(px(10.))
                                    .text_color(rgb(INK_SECONDARY))
                                    .truncate()
                                    .child(detail_txt),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2p5()
                            .font_family(self.fonts.mono_num.clone())
                            .text_size(px(11.))
                            .text_color(rgb(INK_SECONDARY))
                            .child(format!("{}in/{}out", run.input_tokens, run.output_tokens))
                            .child(format!("${:.4}", run.cost_usd.round_dp(4).normalize()))
                            .child(fmt_time_cn_for(Some(run.started_at_ms), self.lang)),
                    ),
            );
        }

        card
    }
}

pub(super) struct AgentPanelSlice<'a> {
    pub(super) streaming: &'a mut bool,
    pub(super) status_hint: &'a mut Option<String>,
    pub(super) messages: &'a mut Vec<AgentUiMessage>,
    pub(super) tape: &'a mut Vec<TuringTapeEntry>,
    pub(super) seq: &'a mut usize,
}

#[allow(dead_code)]
pub(super) fn apply_interrogator_event(
    state: &mut AgentPanelSlice<'_>,
    ev: &InterrogatorEvent,
    focus_ticker: &str,
    reports: &[MungerReport],
    dossiers: &HashMap<String, Option<Dossier>>,
) -> bool {
    apply_interrogator_event_for(state, ev, focus_ticker, reports, dossiers, UiLang::Zh)
}

pub(super) fn apply_interrogator_event_for(
    state: &mut AgentPanelSlice<'_>,
    ev: &InterrogatorEvent,
    focus_ticker: &str,
    reports: &[MungerReport],
    dossiers: &HashMap<String, Option<Dossier>>,
    lang: UiLang,
) -> bool {
    match ev {
        InterrogatorEvent::Ready | InterrogatorEvent::UserSubmitted { .. } => {
            if state
                .status_hint
                .as_deref()
                .is_some_and(|h| h.contains("正在连接") || h.contains("Connecting"))
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
            *state.status_hint = Some(match lang {
                UiLang::Zh => format!("正在执行 {name}…"),
                UiLang::En => format!("Running {name}…"),
            });
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
            let blocked_by_redline =
                preview.contains("红线拦截") || preview.to_ascii_lowercase().contains("redline");
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
                    summary: lang
                        .t("审讯未能完成", "Interrogation did not complete")
                        .to_string(),
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
                    summary: match lang {
                        UiLang::Zh => format!("回答完成 · 引用 Acme 证据 {token_count} 项"),
                        UiLang::En => {
                            format!("Answer complete · {token_count} Acme evidence citations")
                        }
                    },
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
                summary: lang
                    .t("EmbeddedAlan 状态提示", "EmbeddedAlan status notice")
                    .to_string(),
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
