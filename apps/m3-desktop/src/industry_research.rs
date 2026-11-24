//! Multi-perspective Industry Deep Research orchestrator built on `EmbeddedAlan` + Zero-LLM DuckDB/10-K + bounded Web Search.

use std::collections::{BTreeMap, HashSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::Sender;
use std::time::Duration;

use alan_os_host::embed::{
    EmbedEvent, EmbeddedAlan, LlmSource, SessionOptions, SystemPromptMode, UiInputStatus,
};
use anyhow::{Context, Result};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use crate::alan_interrogator::{M3WorkbenchFs, boot_m3_embedded, friendly_m3_credentials_error};
use crate::budget;
use crate::events;
use crate::industry::{self, IndustryProfile};
use crate::lake::{LakeDb, UniverseRow};
use crate::lake_l2::{ResearchRunRow, ResearchRunStatus};
use crate::research::PerspectiveOutcome;
use crate::store::{self, WorkspacePaths};

const DEFAULT_SESSION_TIMEOUT_SECS: u64 = 180;
static IND_RUN_SEQ: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IndustryPerspective {
    ValueChain,
    BusinessModelMoat,
    FailureModes,
    ValuationKpis,
}

impl IndustryPerspective {
    pub fn all() -> [Self; 4] {
        [
            Self::ValueChain,
            Self::BusinessModelMoat,
            Self::FailureModes,
            Self::ValuationKpis,
        ]
    }

    pub fn id(self) -> &'static str {
        match self {
            Self::ValueChain => "value_chain",
            Self::BusinessModelMoat => "business_model_moat",
            Self::FailureModes => "failure_modes",
            Self::ValuationKpis => "valuation_kpis",
        }
    }

    pub fn title_cn(self) -> &'static str {
        match self {
            Self::ValueChain => "产业链与利润池",
            Self::BusinessModelMoat => "商业模式与护城河",
            Self::FailureModes => "历史死法与周期陷阱",
            Self::ValuationKpis => "估值锚与核心 KPI",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IndustryResearchEvent {
    Started {
        run_id: String,
        slug: String,
    },
    Fetching10K {
        ticker: String,
        idx: usize,
        total: usize,
    },
    PerspectiveStarted {
        perspective: String,
        label: String,
    },
    PerspectiveDelta {
        perspective: String,
        chunk: String,
    },
    ToolStarted {
        perspective: String,
        name: String,
        title: String,
    },
    PerspectiveFinished {
        perspective: String,
        summary_chars: usize,
    },
    Completed(IndustryResearchOutcome),
    Failed {
        run_id: String,
        slug: String,
        error: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IndustryResearchOutcome {
    pub run_id: String,
    pub slug: String,
    pub primer_path: Option<String>,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cost_usd_str: String,
    pub sec_10k_fetched_count: usize,
    pub perspectives: Vec<PerspectiveOutcome>,
    pub feynman_questions: Vec<String>,
    pub error: Option<String>,
}

pub fn run_industry_deep_research(
    paths: &WorkspacePaths,
    slug: &str,
    focus: Option<&str>,
    event_tx: Option<Sender<IndustryResearchEvent>>,
) -> Result<IndustryResearchOutcome> {
    let profile = industry::load_industry(paths, slug)?;
    let ticker_key = format!("IND:{}", profile.slug.to_ascii_uppercase());
    let started_at_ms = store::now_ms();
    let seq = IND_RUN_SEQ.fetch_add(1, Ordering::Relaxed);
    let run_id = format!("ind-{}-{started_at_ms}-{seq}", profile.slug);

    // 1. Pre-flight budget gate before any LLM boot.
    let budget_cfg = budget::load_budget_config(paths)?;
    let db = LakeDb::open(paths)?;
    let monthly_usage = db.monthly_budget_usage(started_at_ms)?;
    let monthly_tokens = monthly_usage
        .total_input_tokens
        .saturating_add(monthly_usage.total_output_tokens);
    let decision = budget::evaluate_budget_gate(
        &budget_cfg,
        monthly_tokens,
        monthly_usage.total_cost_usd,
        monthly_usage.weekly_auto_runs,
        false,
    );

    if !decision.allowed {
        let reason = decision
            .reason
            .unwrap_or_else(|| "AI 预算上限已达，行业深研已拦截".to_owned());
        let finished_at_ms = store::now_ms();
        db.insert_research_run(&ResearchRunRow {
            run_id: run_id.clone(),
            ticker: ticker_key,
            trigger: "industry".to_owned(),
            focus: focus.map(str::to_owned),
            model: "budget-gate".to_owned(),
            input_tokens: 0,
            output_tokens: 0,
            cost_usd: Decimal::ZERO,
            started_at_ms,
            finished_at_ms: Some(finished_at_ms),
            status: ResearchRunStatus::BlockedBudget,
            error: Some(reason.clone()),
            memo_path: None,
            perspectives_json: "[]".to_owned(),
            citations_json: "[]".to_owned(),
        })?;
        drop(db);
        emit_event(
            &event_tx,
            IndustryResearchEvent::Failed {
                run_id: run_id.clone(),
                slug: profile.slug.clone(),
                error: reason.clone(),
            },
        );
        return Ok(IndustryResearchOutcome {
            run_id,
            slug: profile.slug,
            primer_path: None,
            input_tokens: 0,
            output_tokens: 0,
            cost_usd_str: "0".to_owned(),
            sec_10k_fetched_count: 0,
            perspectives: Vec::new(),
            feynman_questions: profile.feynman_questions,
            error: Some(reason),
        });
    }

    db.insert_research_run(&ResearchRunRow {
        run_id: run_id.clone(),
        ticker: ticker_key.clone(),
        trigger: "industry".to_owned(),
        focus: focus.map(str::to_owned),
        model: "alan-connection".to_owned(),
        input_tokens: 0,
        output_tokens: 0,
        cost_usd: Decimal::ZERO,
        started_at_ms,
        finished_at_ms: None,
        status: ResearchRunStatus::Running,
        error: None,
        memo_path: None,
        perspectives_json: "[]".to_owned(),
        citations_json: "[]".to_owned(),
    })?;
    drop(db);

    emit_event(
        &event_tx,
        IndustryResearchEvent::Started {
            run_id: run_id.clone(),
            slug: profile.slug.clone(),
        },
    );

    // 2. Zero-LLM SEC 10-K bootstrap for up to 5 representative bound companies lacking 10-K chunks.
    let sec_10k_fetched_count = bootstrap_missing_10k_profiles(paths, &profile, &event_tx);

    // Gather deterministic peer matrix snapshot for primer synthesis & drop RO handle before EmbeddedAlan boots.
    let peer_rows: Vec<UniverseRow> = if let Ok(Some(ro)) = LakeDb::open_read_only_if_exists(paths)
    {
        ro.load_universe_rows_by_tickers(&profile.bound_tickers)
            .unwrap_or_default()
    } else {
        Vec::new()
    };

    // 3. Boot EmbeddedAlan & run 4 parallel industry analyst sessions.
    let focus_owned = focus
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned);
    let batch = run_parallel_industry_sessions(
        paths.clone(),
        profile.clone(),
        focus_owned.clone(),
        LlmSource::SharedProductCredentials,
        event_tx.clone(),
    );

    match batch {
        Err(boot_err) => {
            let friendly = friendly_m3_credentials_error(&format!("{boot_err:#}"));
            let finished_at_ms = store::now_ms();
            if let Ok(db) = LakeDb::open(paths) {
                let _ = db.finish_research_run(
                    &run_id,
                    ResearchRunStatus::Failed,
                    0,
                    0,
                    Decimal::ZERO,
                    finished_at_ms,
                    Some(&friendly),
                    None,
                    "[]",
                    "[]",
                );
            }
            emit_event(
                &event_tx,
                IndustryResearchEvent::Failed {
                    run_id: run_id.clone(),
                    slug: profile.slug.clone(),
                    error: friendly.clone(),
                },
            );
            Ok(IndustryResearchOutcome {
                run_id,
                slug: profile.slug,
                primer_path: None,
                input_tokens: 0,
                output_tokens: 0,
                cost_usd_str: "0".to_owned(),
                sec_10k_fetched_count,
                perspectives: Vec::new(),
                feynman_questions: profile.feynman_questions,
                error: Some(friendly),
            })
        }
        Ok((outcomes, session_errors)) => {
            let total_in: u64 = outcomes
                .iter()
                .fold(0u64, |acc, o| acc.saturating_add(o.input_tokens));
            let total_out: u64 = outcomes
                .iter()
                .fold(0u64, |acc, o| acc.saturating_add(o.output_tokens));
            let total_tokens = total_in.saturating_add(total_out);
            let cost_usd = budget::estimate_cost_usd(&budget_cfg, total_in, total_out);
            let cost_str = cost_usd.round_dp(4).normalize().to_string();
            let finished_at_ms = store::now_ms();
            let perspectives_json =
                serde_json::to_string(&outcomes).unwrap_or_else(|_| "[]".to_owned());

            if total_tokens > budget_cfg.per_run_token_limit
                || cost_usd > budget_cfg.per_run_budget_usd
                || session_errors.len() == outcomes.len()
            {
                let msg = if session_errors.len() == outcomes.len() && !session_errors.is_empty() {
                    session_errors.join(" | ")
                } else {
                    format!("行业深研超出单次预算上限（{total_tokens} tokens / ${cost_str}）")
                };
                if let Ok(db) = LakeDb::open(paths) {
                    let _ = db.finish_research_run(
                        &run_id,
                        ResearchRunStatus::Failed,
                        total_in,
                        total_out,
                        cost_usd,
                        finished_at_ms,
                        Some(&msg),
                        None,
                        &perspectives_json,
                        "[]",
                    );
                }
                emit_event(
                    &event_tx,
                    IndustryResearchEvent::Failed {
                        run_id: run_id.clone(),
                        slug: profile.slug.clone(),
                        error: msg.clone(),
                    },
                );
                return Ok(IndustryResearchOutcome {
                    run_id,
                    slug: profile.slug,
                    primer_path: None,
                    input_tokens: total_in,
                    output_tokens: total_out,
                    cost_usd_str: cost_str,
                    sec_10k_fetched_count,
                    perspectives: outcomes,
                    feynman_questions: profile.feynman_questions,
                    error: Some(msg),
                });
            }

            let primer_body = render_industry_primer(
                &profile,
                &run_id,
                finished_at_ms,
                total_in,
                total_out,
                &cost_str,
                &peer_rows,
                &outcomes,
                &session_errors,
            );
            let primer_path = industry::write_industry_primer_preserving_human_notes(
                paths,
                &profile.slug,
                &primer_body,
                Some(&profile.human_notes),
            )?;
            let primer_path_str = primer_path.display().to_string();

            let extracted_questions = extract_feynman_questions(&outcomes, &profile.title);
            let mut updated = industry::load_industry(paths, &profile.slug).unwrap_or(profile);
            updated.last_research_run_id = Some(run_id.clone());
            updated.updated_at_ms = finished_at_ms;
            if updated.feynman_questions.is_empty() && !extracted_questions.is_empty() {
                updated.feynman_questions = extracted_questions;
                industry::sync_feynman_checked_len(&mut updated);
            }
            industry::save_industry(paths, &updated)?;

            if let Ok(db) = LakeDb::open(paths) {
                let _ = db.finish_research_run(
                    &run_id,
                    ResearchRunStatus::Completed,
                    total_in,
                    total_out,
                    cost_usd,
                    finished_at_ms,
                    None,
                    Some(&primer_path_str),
                    &perspectives_json,
                    "[]",
                );
            }

            let outcome = IndustryResearchOutcome {
                run_id,
                slug: updated.slug,
                primer_path: Some(primer_path_str),
                input_tokens: total_in,
                output_tokens: total_out,
                cost_usd_str: cost_str,
                sec_10k_fetched_count,
                perspectives: outcomes,
                feynman_questions: updated.feynman_questions,
                error: None,
            };
            emit_event(&event_tx, IndustryResearchEvent::Completed(outcome.clone()));
            Ok(outcome)
        }
    }
}

fn bootstrap_missing_10k_profiles(
    paths: &WorkspacePaths,
    profile: &IndustryProfile,
    event_tx: &Option<Sender<IndustryResearchEvent>>,
) -> usize {
    let target_tickers: Vec<String> = profile.bound_tickers.iter().take(5).cloned().collect();
    if target_tickers.is_empty() {
        return 0;
    }

    let (rows, sec_counts) = match LakeDb::open_read_only_if_exists(paths) {
        Ok(Some(db)) => {
            let rows = db
                .load_universe_rows_by_tickers(&target_tickers)
                .unwrap_or_default();
            let counts = db
                .count_sec_10k_chunks_by_ticker(&target_tickers)
                .unwrap_or_default();
            (rows, counts)
        }
        _ => return 0,
    };

    let missing: Vec<(String, String)> = rows
        .into_iter()
        .filter(|r| {
            sec_counts
                .get(&r.ticker.to_ascii_uppercase())
                .copied()
                .unwrap_or(0)
                == 0
        })
        .map(|r| (r.ticker, r.cik))
        .take(5)
        .collect();

    let total = missing.len();
    let mut fetched = 0usize;
    for (idx, (ticker, cik)) in missing.into_iter().enumerate() {
        emit_event(
            event_tx,
            IndustryResearchEvent::Fetching10K {
                ticker: ticker.clone(),
                idx: idx + 1,
                total,
            },
        );
        match events::fetch_company_10k_profile(paths, &ticker, &cik) {
            Ok(rep) if rep.chunks_indexed > 0 => fetched += 1,
            Ok(_) => {}
            Err(err) => {
                log::warn!("Industry 10-K bootstrap skipped {ticker}: {err:#}");
            }
        }
    }
    fetched
}

fn run_parallel_industry_sessions(
    paths: WorkspacePaths,
    profile: IndustryProfile,
    focus: Option<String>,
    llm: LlmSource,
    event_tx: Option<Sender<IndustryResearchEvent>>,
) -> Result<(Vec<PerspectiveOutcome>, Vec<String>)> {
    let handle = std::thread::Builder::new()
        .name(format!("m3-industry-research-{}", profile.slug))
        .stack_size(8 * 1024 * 1024)
        .spawn(move || -> Result<(Vec<PerspectiveOutcome>, Vec<String>)> {
            let rt = tokio::runtime::Builder::new_multi_thread()
                .worker_threads(4)
                .enable_all()
                .build()
                .context("无法创建行业深研 Tokio 运行时")?;

            rt.block_on(async move {
                let fs = Arc::new(M3WorkbenchFs::new(paths.clone()));
                let app = boot_m3_embedded(paths.clone(), fs, llm).await?;

                let (r_vc, r_moat, r_fail, r_val) = tokio::join!(
                    run_single_industry_perspective(
                        &app,
                        IndustryPerspective::ValueChain,
                        &profile,
                        focus.as_deref(),
                        &event_tx,
                    ),
                    run_single_industry_perspective(
                        &app,
                        IndustryPerspective::BusinessModelMoat,
                        &profile,
                        focus.as_deref(),
                        &event_tx,
                    ),
                    run_single_industry_perspective(
                        &app,
                        IndustryPerspective::FailureModes,
                        &profile,
                        focus.as_deref(),
                        &event_tx,
                    ),
                    run_single_industry_perspective(
                        &app,
                        IndustryPerspective::ValuationKpis,
                        &profile,
                        focus.as_deref(),
                        &event_tx,
                    ),
                );

                if let Err(err) = app.shutdown().await {
                    log::warn!("EmbeddedAlan shutdown warning during industry research: {err:#}");
                }

                let mut outcomes = Vec::with_capacity(4);
                let mut errors = Vec::new();
                for (perspective, res) in [
                    (IndustryPerspective::ValueChain, r_vc),
                    (IndustryPerspective::BusinessModelMoat, r_moat),
                    (IndustryPerspective::FailureModes, r_fail),
                    (IndustryPerspective::ValuationKpis, r_val),
                ] {
                    match res {
                        Ok(o) => outcomes.push(o),
                        Err(err) => {
                            let msg = friendly_m3_credentials_error(&format!("{err:#}"));
                            errors.push(format!(
                                "{} ({}): {msg}",
                                perspective.title_cn(),
                                perspective.id()
                            ));
                            outcomes.push(PerspectiveOutcome {
                                perspective: perspective.id().to_owned(),
                                title: perspective.title_cn().to_owned(),
                                content: format!("_视角执行失败：{msg}_"),
                                tools_used: Vec::new(),
                                input_tokens: 0,
                                output_tokens: 0,
                            });
                        }
                    }
                }
                Ok((outcomes, errors))
            })
        })
        .context("无法启动行业深研线程")?;

    match handle.join() {
        Ok(res) => res,
        Err(_) => anyhow::bail!("行业深研线程意外中止"),
    }
}

async fn run_single_industry_perspective(
    app: &EmbeddedAlan,
    perspective: IndustryPerspective,
    profile: &IndustryProfile,
    focus: Option<&str>,
    event_tx: &Option<Sender<IndustryResearchEvent>>,
) -> Result<PerspectiveOutcome> {
    let pid = perspective.id().to_owned();
    let label = perspective.title_cn().to_owned();
    emit_event(
        event_tx,
        IndustryResearchEvent::PerspectiveStarted {
            perspective: pid.clone(),
            label: label.clone(),
        },
    );

    let opts = SessionOptions::default().with_system_prompt_mode(SystemPromptMode::Replace {
        prompt: industry_perspective_system_prompt(perspective),
    });
    let session = app
        .spawn_child_session(app.root_pid(), opts)
        .await
        .with_context(|| format!("创建行业子会话 {pid} 失败"))?;

    let mut stream = session
        .client()
        .event_stream()
        .await
        .with_context(|| format!("打开行业子会话 {pid} 事件流失败"))?;

    let prompt = compose_industry_perspective_prompt(perspective, profile, focus);
    let submission_id = session
        .client()
        .submit_text(prompt)
        .await
        .with_context(|| format!("向行业子会话 {pid} 提交任务失败"))?;

    let mut content = String::new();
    let mut tools_used = Vec::new();
    let mut seen_tools = HashSet::new();
    let mut input_tokens = 0u64;
    let mut output_tokens = 0u64;

    let deadline = tokio::time::Instant::now() + Duration::from_secs(DEFAULT_SESSION_TIMEOUT_SECS);
    loop {
        let ev = tokio::time::timeout_at(deadline, stream.next_event())
            .await
            .with_context(|| format!("行业子会话 {pid} 超时"))??;
        match ev {
            EmbedEvent::TextDelta { chunk, .. } => {
                if !chunk.is_empty() {
                    content.push_str(&chunk);
                    emit_event(
                        event_tx,
                        IndustryResearchEvent::PerspectiveDelta {
                            perspective: pid.clone(),
                            chunk,
                        },
                    );
                }
            }
            EmbedEvent::ToolStarted { name, title, .. } => {
                if seen_tools.insert(name.clone()) {
                    tools_used.push(name.clone());
                }
                emit_event(
                    event_tx,
                    IndustryResearchEvent::ToolStarted {
                        perspective: pid.clone(),
                        name,
                        title,
                    },
                );
            }
            EmbedEvent::ToolFinished { name, .. } => {
                if seen_tools.insert(name.clone()) {
                    tools_used.push(name);
                }
            }
            EmbedEvent::Usage { usage, .. } => {
                input_tokens = input_tokens.max(usage.session_input_tokens.max(usage.input_tokens));
                output_tokens =
                    output_tokens.max(usage.session_output_tokens.max(usage.output_tokens));
            }
            EmbedEvent::PendingRequest { request_id, .. } => {
                let _ = session
                    .client()
                    .respond_request(&request_id, "denied: non-interactive industry research")
                    .await;
            }
            EmbedEvent::Error {
                message,
                recoverable,
            } => {
                if !recoverable {
                    let _ = session.close().await;
                    anyhow::bail!("{message}");
                }
            }
            EmbedEvent::TurnFinished {
                submission_id: fid,
                status,
                error,
            } if fid == submission_id => {
                if status != UiInputStatus::Completed {
                    let _ = session.close().await;
                    let msg = error.unwrap_or_else(|| format!("状态 {status:?}"));
                    anyhow::bail!("{msg}");
                }
                break;
            }
            _ => {}
        }
    }

    let _ = session.close().await;
    let trimmed = content.trim().to_owned();
    let chars = trimmed.chars().count();
    emit_event(
        event_tx,
        IndustryResearchEvent::PerspectiveFinished {
            perspective: pid.clone(),
            summary_chars: chars,
        },
    );

    Ok(PerspectiveOutcome {
        perspective: pid,
        title: label,
        content: trimmed,
        tools_used,
        input_tokens,
        output_tokens,
    })
}

fn industry_perspective_system_prompt(perspective: IndustryPerspective) -> String {
    let focus = match perspective {
        IndustryPerspective::ValueChain => {
            "你的专责视角是【产业链与利润池 (Value Chain & Profit Pool)】。\
             对比行业上中下游谁真正留住超额 ROIC 与现金流，结合 `m3_industry_matrix`、代表公司 SEC 10-K 与 `m3_web_search` 梳理市场份额演变与定价权所在环节。"
        }
        IndustryPerspective::BusinessModelMoat => {
            "你的专责视角是【商业模式与护城河 (Business Model & Moats)】。\
             剖析真实护城河来源（转换成本、网络效应、规模经济、专利/牌照），并说明行业内盈利能力最强的龙头与二线玩家的本质区别。"
        }
        IndustryPerspective::FailureModes => {
            "你的专责视角是【历史死法与周期陷阱 (Failure Modes & Cyclical Traps)】。\
             利用 `m3_web_search` 和 SEC 10-K 风险章节，找出该行业过去 10-20 年摧毁股东价值的典型死法：资本开支陷阱、技术代际颠覆、价格战、资产负债表错配与政策逆转。"
        }
        IndustryPerspective::ValuationKpis => {
            "你的专责视角是【估值锚与核心 KPI (Valuation Anchor & Key KPIs)】。\
             结合 `m3_industry_matrix` 展示代表公司的 5Y ROIC、毛利与现金转化，提炼投资者在读懂该行业时必须盯紧的 3-5 个领先运营 KPI 与合理估值倍数区间思考框架（仅供人类自主设定，禁止给出买卖指令），并在末尾用 `### 费曼通关问题` 列出 3-4 条检验人类是否真正理解该行业的问题。"
        }
    };
    format!(
        "你是 Project-M3 行业研究工作台的资深芒格派行业分析师。\n{focus}\n\
         铁律：\n\
         1. 财务数字必须直接引用 `m3_industry_matrix` / `m3_events_for_ticker` 的 SEC 确定性结果，严禁编造。\n\
         2. 引用出处时严格区分两类标签：`[SEC 10-K: TICKER]`（一手官方文件）与 `[Web: 标题 (URL)]`（公开网络来源）。\n\
         3. 严禁修改任何 `workbench/industries/` 或 `human_judgment` 文件，严禁输出买卖指令。\n\
         4. 输出清晰精炼的中文结构化分析（350-650 字）。"
    )
}

fn compose_industry_perspective_prompt(
    perspective: IndustryPerspective,
    profile: &IndustryProfile,
    focus: Option<&str>,
) -> String {
    let tickers = if profile.bound_tickers.is_empty() {
        "尚未绑定具体代码（可先用 m3_web_search 与 m3_universe_query 寻找代表公司）".to_owned()
    } else {
        profile.bound_tickers.join(", ")
    };
    let sic = if profile.sic_codes.is_empty() {
        "未指定".to_owned()
    } else {
        profile
            .sic_codes
            .iter()
            .map(u32::to_string)
            .collect::<Vec<_>>()
            .join(", ")
    };
    let focus_line = focus
        .map(|f| format!("\n人类研究补充聚焦：{f}"))
        .unwrap_or_default();
    format!(
        "[行业深研视角：{} ({})]\n\
         行业名称：{}\n\
         Slug：`{}`\n\
         SIC：{sic}\n\
         代表公司池：{tickers}{focus_line}\n\n\
         请先调用 `m3_industry_matrix` (传 `industry_slug: \"{}\"` )，按需结合 `m3_events_for_ticker` 查看 SEC 10-K 原文片段，并调用 `m3_web_search` 检索公开市场格局与历史演变，严格分离 `[SEC 10-K]` 与 `[Web]` 引用后给出结论。",
        perspective.title_cn(),
        perspective.id(),
        profile.title,
        profile.slug,
        profile.slug,
    )
}

#[allow(clippy::too_many_arguments)]
fn render_industry_primer(
    profile: &IndustryProfile,
    run_id: &str,
    finished_at_ms: u64,
    input_tokens: u64,
    output_tokens: u64,
    cost_str: &str,
    peer_rows: &[UniverseRow],
    outcomes: &[PerspectiveOutcome],
    session_errors: &[String],
) -> String {
    let mut md = String::new();
    md.push_str(&format!(
        "# {} — 行业活手册 (Living Industry Primer)\n\n",
        profile.title
    ));
    md.push_str(&format!("- **Run ID**: `{run_id}`\n"));
    md.push_str(&format!(
        "- **Updated Date**: {}\n",
        store::civil_today_iso()
    ));
    md.push_str(&format!("- **Updated At (ms)**: `{finished_at_ms}`\n"));
    md.push_str(&format!(
        "- **Bound Tickers**: {}\n",
        if profile.bound_tickers.is_empty() {
            "—".to_owned()
        } else {
            profile.bound_tickers.join(", ")
        }
    ));
    md.push_str(&format!(
        "- **Usage**: `{input_tokens}` in / `{output_tokens}` out · `${cost_str}` USD\n\n"
    ));

    md.push_str("## L0 代表公司确定性对照表 (Zero-LLM DuckDB)\n\n");
    if peer_rows.is_empty() {
        md.push_str("_当前绑定公司尚未进入本地 L0 DuckDB 湖仓。_\n\n");
    } else {
        md.push_str(
            "| Ticker | 公司 | 5Y ROIC | 5Y 毛利 | 严苛 FCF% | SBC/OCF | 3Y 股本Δ | 质量分 |\n",
        );
        md.push_str("| --- | --- | --- | --- | --- | --- | --- | --- |\n");
        for r in peer_rows {
            let fmt = |v: Option<Decimal>, suf: &str| {
                v.map(|d| format!("{}{suf}", d.round_dp(1).normalize()))
                    .unwrap_or_else(|| "—".to_owned())
            };
            md.push_str(&format!(
                "| `{}` | {} | {} | {} | {} | {} | {} | {} |\n",
                r.ticker,
                r.name,
                fmt(r.roic_5y_avg_pct, "%"),
                fmt(r.gross_margin_5y_avg_pct, "%"),
                fmt(r.strict_fcf_yield_pct, "%"),
                fmt(r.sbc_to_ocf_pct, "%"),
                fmt(r.share_change_3y_pct, "%"),
                r.quality_score.round_dp(1).normalize(),
            ));
        }
        md.push('\n');
    }

    if !session_errors.is_empty() {
        md.push_str("**部分视角执行提示**:\n");
        for e in session_errors {
            md.push_str(&format!("- {e}\n"));
        }
        md.push('\n');
    }

    let by_id: BTreeMap<&str, &PerspectiveOutcome> = outcomes
        .iter()
        .map(|o| (o.perspective.as_str(), o))
        .collect();

    for p in IndustryPerspective::all() {
        md.push_str(&format!("## {} ({})\n\n", p.title_cn(), p.id()));
        if let Some(o) = by_id.get(p.id()) {
            let tools = if o.tools_used.is_empty() {
                "none".to_owned()
            } else {
                o.tools_used.join(", ")
            };
            md.push_str(&format!(
                "_Tools: `{tools}` · Tokens: {} in / {} out_\n\n",
                o.input_tokens, o.output_tokens
            ));
            md.push_str(if o.content.is_empty() {
                "_暂无内容_\n\n"
            } else {
                &o.content
            });
            md.push_str("\n\n");
        }
    }

    let (sec_refs, web_refs) = collect_separated_citations(outcomes);
    md.push_str("## Evidence Sources (严格出处分离)\n\n");
    md.push_str("### 一手 SEC 财报出处 (SEC 10-K)\n\n");
    if sec_refs.is_empty() {
        md.push_str(
            "- 已绑定代表公司本地 L0 确定性财报与 10-K 原文切片（详见 DuckDB `doc_chunks`）。\n\n",
        );
    } else {
        for item in sec_refs {
            md.push_str(&format!("- {item}\n"));
        }
        md.push('\n');
    }

    md.push_str("### 公开网页参考来源 (Web Search · External)\n\n");
    if web_refs.is_empty() {
        md.push_str("- _本次分析未引用额外外部网页条目。_\n\n");
    } else {
        for item in web_refs {
            md.push_str(&format!("- {item}\n"));
        }
        md.push('\n');
    }

    md
}

fn collect_separated_citations(outcomes: &[PerspectiveOutcome]) -> (Vec<String>, Vec<String>) {
    let mut sec = Vec::new();
    let mut web = Vec::new();
    let mut seen_sec = HashSet::new();
    let mut seen_web = HashSet::new();

    for o in outcomes {
        let text = o.content.as_str();
        let mut idx = 0usize;
        while let Some(open) = text[idx..].find('[') {
            let abs_open = idx + open;
            let rest = &text[abs_open + 1..];
            if let Some(close) = rest.find(']') {
                let inner = rest[..close].trim();
                let lower = inner.to_ascii_lowercase();
                if lower.starts_with("sec 10-k:") || lower.starts_with("sec:") {
                    if seen_sec.insert(inner.to_owned()) {
                        sec.push(format!("`[{inner}]`"));
                    }
                } else if lower.starts_with("web:") && seen_web.insert(inner.to_owned()) {
                    web.push(format!("`[{inner}]`"));
                }
                idx = abs_open + 1 + close + 1;
            } else {
                break;
            }
        }
    }
    (sec, web)
}

fn extract_feynman_questions(outcomes: &[PerspectiveOutcome], title: &str) -> Vec<String> {
    let mut qs = Vec::new();
    for o in outcomes {
        let mut in_section = false;
        for line in o.content.lines() {
            let trimmed = line.trim();
            if trimmed.contains("费曼通关") || trimmed.to_ascii_lowercase().contains("feynman")
            {
                in_section = true;
                continue;
            }
            if in_section && trimmed.starts_with("##") {
                in_section = false;
            }
            if in_section {
                let cleaned = trimmed
                    .trim_start_matches(['-', '*', ' '])
                    .trim_start_matches(|c: char| c.is_ascii_digit() || c == '.' || c == '、')
                    .trim();
                if !cleaned.is_empty() && cleaned.chars().count() >= 8 {
                    qs.push(cleaned.to_owned());
                }
            }
            if qs.len() >= 5 {
                break;
            }
        }
        if qs.len() >= 5 {
            break;
        }
    }
    if qs.is_empty() {
        return vec![
            format!("我能否用 3 句话向外行讲清「{title}」最赚钱的环节及客户付费原因？"),
            format!("过去 10-20 年「{title}」中亏损或消亡的公司，最常见的死法是什么？"),
            "该行业的合理股东盈余倍数区间（估值锚）及核心领先 KPI 分别是什么？".to_owned(),
            "若头部公司连续两个季度核心 KPI 恶化，我在哪一个节点必须承认看错并放弃？".to_owned(),
        ];
    }
    qs
}

fn emit_event(tx: &Option<Sender<IndustryResearchEvent>>, ev: IndustryResearchEvent) {
    if let Some(sender) = tx {
        let _ = sender.send(ev);
    }
}
