//! L2 Multi-perspective Deep Research orchestrator built on `EmbeddedAlan` child sessions.
//!
//! - Enforces a hard pre-flight budget gate (`budget::evaluate_budget_gate` + `LakeDb::monthly_budget_usage`)
//!   before booting `EmbeddedAlan` or invoking any LLM.
//! - Spawns 4 parallel child analyst sessions (`Moat`, `Accounting`, `LatentPeers`, `Premortem`)
//!   via `EmbeddedAlan::spawn_child_session(app.root_pid(), ...)` constrained by [`M3ToolPolicyHook`].
//! - Synthesizes deterministic financial facts (`store::recompute_ticker` / `LakeDb::query_peers`),
//!   `SignalEvidence` citations (`LakeDb::search_similar` / `doc_chunks`), and the 4 analyst perspectives
//!   into an atomic Living Research Memo at `workbench/memos/<SAFE_TICKER>.md`, preserving any
//!   human-authored `## Human Notes` footer verbatim.

use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::Sender;
use std::time::Duration;

use alan_os_host::embed::{
    EmbedEvent, EmbeddedAlan, LlmSource, SessionOptions, SystemPromptMode, UiInputStatus,
};
use anyhow::{Context, Result};
use duckdb::params;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use crate::alan_interrogator::{M3WorkbenchFs, boot_m3_embedded, friendly_m3_credentials_error};
use crate::budget::{self, BudgetConfig};
use crate::lake::{LakeDb, PeerComparison, UniverseRow};
use crate::lake_l1::{SignalEvidence, SimilarSearchFilter};
use crate::lake_l2::{ResearchRunRow, ResearchRunStatus};
use crate::latent::extract_verbatim_snippet;
use crate::market::safe_symbol_filename;
use crate::model::{MungerReport, serde_decimal};
use crate::store::{self, WorkspacePaths};

const DEFAULT_SESSION_TIMEOUT_SECS: u64 = 180;
const HUMAN_NOTES_HEADER: &str = "## Human Notes";
static RUN_SEQ: AtomicU64 = AtomicU64::new(1);

/// Request parameters for triggering an L2 Deep Research run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeepResearchRequest {
    pub ticker: String,
    pub focus: Option<String>,
    /// `"manual"`, `"inbox"`, `"cli"`, or `"auto"`.
    pub trigger: String,
}

/// The four specialized analyst perspectives executed in parallel per Deep Research run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeepResearchPerspective {
    Moat,
    Accounting,
    LatentPeers,
    Premortem,
}

impl DeepResearchPerspective {
    /// Returns all four perspectives in canonical presentation order.
    pub fn all() -> [Self; 4] {
        [
            Self::Moat,
            Self::Accounting,
            Self::LatentPeers,
            Self::Premortem,
        ]
    }

    /// Stable machine identifier for this perspective.
    pub fn id(&self) -> &'static str {
        match self {
            Self::Moat => "moat",
            Self::Accounting => "accounting",
            Self::LatentPeers => "latent_peers",
            Self::Premortem => "premortem",
        }
    }

    /// Concise Chinese title shown in GPUI Research Desk and memos.
    pub fn title_cn(&self) -> &'static str {
        match self {
            Self::Moat => "护城河",
            Self::Accounting => "会计排雷",
            Self::LatentPeers => "隐空间同行",
            Self::Premortem => "5年逆向验尸",
        }
    }
}

/// Real-time streaming event emitted via `std::sync::mpsc::Sender` for GPUI/CLI telemetry.
#[derive(Debug, Clone, PartialEq)]
pub enum DeepResearchEvent {
    Started {
        run_id: String,
        ticker: String,
    },
    BudgetBlocked {
        run_id: String,
        ticker: String,
        reason: String,
    },
    PerspectiveStarted {
        perspective: String,
        label: String,
    },
    ToolStarted {
        perspective: String,
        name: String,
        title: String,
    },
    ToolFinished {
        perspective: String,
        name: String,
        status: String,
    },
    PerspectiveDelta {
        perspective: String,
        chunk: String,
    },
    PerspectiveFinished {
        perspective: String,
        summary_chars: usize,
    },
    Synthesizing {
        ticker: String,
    },
    Completed {
        result: Box<DeepResearchResult>,
    },
    Failed {
        run_id: String,
        ticker: String,
        error: String,
    },
}

/// Output captured from one parallel analyst child session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PerspectiveOutcome {
    pub perspective: String,
    pub title: String,
    pub content: String,
    pub tools_used: Vec<String>,
    pub input_tokens: u64,
    pub output_tokens: u64,
}

/// Final outcome of a Deep Research run, returned to CLI/GPUI and persisted to `research_runs`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DeepResearchResult {
    pub run_id: String,
    pub ticker: String,
    pub trigger: String,
    /// `"completed"` | `"blocked_budget"` | `"failed"`.
    pub status: String,
    pub model: String,
    pub memo_path: Option<String>,
    pub input_tokens: u64,
    pub output_tokens: u64,
    #[serde(with = "serde_decimal")]
    pub cost_usd: Decimal,
    pub perspectives: Vec<PerspectiveOutcome>,
    pub citations: Vec<SignalEvidence>,
    pub error: Option<String>,
    pub finished_at_ms: u64,
}

/// Canonical filesystem path to `workbench/memos/<SAFE_TICKER>.md`.
pub fn memo_path_for(paths: &WorkspacePaths, ticker: &str) -> PathBuf {
    let upper = ticker.trim().to_ascii_uppercase();
    let safe = safe_symbol_filename(&upper);
    paths.workbench_dir.join("memos").join(format!("{safe}.md"))
}

/// Loads an existing Living Research Memo if present on disk.
pub fn load_research_memo(paths: &WorkspacePaths, ticker: &str) -> Option<String> {
    let path = memo_path_for(paths, ticker);
    fs::read_to_string(path).ok()
}

/// Executes a Deep Research run using the user's configured `alan` connection profile
/// (`LlmSource::SharedProductCredentials`) after enforcing the hard pre-flight budget gate.
pub fn run_deep_research(
    paths: &WorkspacePaths,
    req: &DeepResearchRequest,
    event_tx: Option<Sender<DeepResearchEvent>>,
) -> Result<DeepResearchResult> {
    run_deep_research_with_llm(paths, req, LlmSource::SharedProductCredentials, event_tx)
}

/// Executes a Deep Research run with an explicit [`LlmSource`] (production credentials or `MockLlmProvider`).
pub fn run_deep_research_with_llm(
    paths: &WorkspacePaths,
    req: &DeepResearchRequest,
    llm: LlmSource,
    event_tx: Option<Sender<DeepResearchEvent>>,
) -> Result<DeepResearchResult> {
    let ticker = req.ticker.trim().to_ascii_uppercase();
    anyhow::ensure!(!ticker.is_empty(), "Deep Research 缺少 ticker 参数");

    let trigger = normalize_trigger(&req.trigger);
    let is_auto_trigger = trigger == "auto";
    let focus = req
        .focus
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned);

    let started_at_ms = store::now_ms();
    let seq = RUN_SEQ.fetch_add(1, Ordering::Relaxed);
    let run_id = format!("dr-{ticker}-{started_at_ms}-{seq}");

    // 1. Hard pre-flight budget gate BEFORE booting EmbeddedAlan or invoking any LLM.
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
        is_auto_trigger,
    );

    if !decision.allowed {
        let reason = decision
            .reason
            .unwrap_or_else(|| "AI 预算或配额上限已达，Deep Research 已硬拦截".to_owned());
        let finished_at_ms = store::now_ms();
        db.insert_research_run(&ResearchRunRow {
            run_id: run_id.clone(),
            ticker: ticker.clone(),
            trigger: trigger.clone(),
            focus,
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
            DeepResearchEvent::BudgetBlocked {
                run_id: run_id.clone(),
                ticker: ticker.clone(),
                reason: reason.clone(),
            },
        );

        return Ok(DeepResearchResult {
            run_id,
            ticker,
            trigger,
            status: ResearchRunStatus::BlockedBudget.as_str().to_owned(),
            model: "budget-gate".to_owned(),
            memo_path: None,
            input_tokens: 0,
            output_tokens: 0,
            cost_usd: Decimal::ZERO,
            perspectives: Vec::new(),
            citations: Vec::new(),
            error: Some(reason),
            finished_at_ms,
        });
    }

    // 2. Insert `Running` row in `research_runs`, then drop RW LakeDb before booting EmbeddedAlan
    // so read-only Lakehouse tools (`LakeDb::open_read_only_if_exists`) never hit an RW/RO lock conflict.
    let model_label = model_label_for_llm(&llm);
    db.insert_research_run(&ResearchRunRow {
        run_id: run_id.clone(),
        ticker: ticker.clone(),
        trigger: trigger.clone(),
        focus: focus.clone(),
        model: model_label.clone(),
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
        DeepResearchEvent::Started {
            run_id: run_id.clone(),
            ticker: ticker.clone(),
        },
    );

    // 3. Boot EmbeddedAlan & run 4 parallel child analyst sessions on a dedicated runtime thread.
    let batch_outcome = run_parallel_sessions(
        paths.clone(),
        ticker.clone(),
        focus.clone(),
        llm,
        event_tx.clone(),
    );

    match batch_outcome {
        Err(boot_or_runtime_err) => {
            let friendly = friendly_m3_credentials_error(&format!("{boot_or_runtime_err:#}"));
            let finished_at_ms = store::now_ms();
            record_failed_run(
                paths,
                &run_id,
                0,
                0,
                Decimal::ZERO,
                finished_at_ms,
                &friendly,
                "[]",
                "[]",
            )?;
            emit_event(
                &event_tx,
                DeepResearchEvent::Failed {
                    run_id: run_id.clone(),
                    ticker: ticker.clone(),
                    error: friendly.clone(),
                },
            );
            Ok(DeepResearchResult {
                run_id,
                ticker,
                trigger,
                status: ResearchRunStatus::Failed.as_str().to_owned(),
                model: model_label,
                memo_path: None,
                input_tokens: 0,
                output_tokens: 0,
                cost_usd: Decimal::ZERO,
                perspectives: Vec::new(),
                citations: Vec::new(),
                error: Some(friendly),
                finished_at_ms,
            })
        }
        Ok(session_batch) => {
            let total_input_tokens: u64 = session_batch
                .outcomes
                .iter()
                .fold(0u64, |acc, o| acc.saturating_add(o.input_tokens));
            let total_output_tokens: u64 = session_batch
                .outcomes
                .iter()
                .fold(0u64, |acc, o| acc.saturating_add(o.output_tokens));
            let total_tokens = total_input_tokens.saturating_add(total_output_tokens);
            let cost_usd =
                budget::estimate_cost_usd(&budget_cfg, total_input_tokens, total_output_tokens);
            let perspectives_json =
                serde_json::to_string(&session_batch.outcomes).unwrap_or_else(|_| "[]".to_owned());

            // 4. Enforce per-run ceiling and session failure checks before synthesizing memo.
            if let Some(ceiling_err) = check_per_run_ceiling(
                &budget_cfg,
                total_tokens,
                cost_usd,
                &session_batch.session_errors,
                session_batch.outcomes.len(),
            ) {
                let finished_at_ms = store::now_ms();
                record_failed_run(
                    paths,
                    &run_id,
                    total_input_tokens,
                    total_output_tokens,
                    cost_usd,
                    finished_at_ms,
                    &ceiling_err,
                    &perspectives_json,
                    "[]",
                )?;
                emit_event(
                    &event_tx,
                    DeepResearchEvent::Failed {
                        run_id: run_id.clone(),
                        ticker: ticker.clone(),
                        error: ceiling_err.clone(),
                    },
                );
                return Ok(DeepResearchResult {
                    run_id,
                    ticker,
                    trigger,
                    status: ResearchRunStatus::Failed.as_str().to_owned(),
                    model: model_label,
                    memo_path: None,
                    input_tokens: total_input_tokens,
                    output_tokens: total_output_tokens,
                    cost_usd,
                    perspectives: session_batch.outcomes,
                    citations: Vec::new(),
                    error: Some(ceiling_err),
                    finished_at_ms,
                });
            }

            // 5. Gather deterministic facts (`store::recompute_ticker` / `LakeDb::query_peers` / `search_similar`)
            // and synthesize atomic Living Research Memo preserving `## Human Notes`.
            emit_event(
                &event_tx,
                DeepResearchEvent::Synthesizing {
                    ticker: ticker.clone(),
                },
            );

            let deterministic_report = store::recompute_ticker(paths, &ticker).ok();
            let db = LakeDb::open(paths)?;
            let facts =
                gather_deterministic_lake_facts(&db, &ticker, deterministic_report.as_ref());
            let citations_json =
                serde_json::to_string(&facts.citations).unwrap_or_else(|_| "[]".to_owned());

            let rel_memo_path = format!("workbench/memos/{}.md", safe_symbol_filename(&ticker));
            let memo_abs_path = memo_path_for(paths, &ticker);
            let finished_at_ms = store::now_ms();

            let memo_markdown = render_living_memo(
                &MemoRenderContext {
                    run_id: &run_id,
                    ticker: &ticker,
                    trigger: &trigger,
                    focus: focus.as_deref(),
                    model: &model_label,
                    input_tokens: total_input_tokens,
                    output_tokens: total_output_tokens,
                    cost_usd,
                    finished_at_ms,
                },
                deterministic_report.as_ref(),
                &facts,
                &session_batch.outcomes,
                &session_batch.session_errors,
                fs::read_to_string(&memo_abs_path).ok().as_deref(),
            );

            write_text_atomic(&memo_abs_path, &memo_markdown)?;

            let non_fatal_note = if session_batch.session_errors.is_empty() {
                None
            } else {
                Some(session_batch.session_errors.join(" | "))
            };

            db.finish_research_run(
                &run_id,
                ResearchRunStatus::Completed,
                total_input_tokens,
                total_output_tokens,
                cost_usd,
                finished_at_ms,
                non_fatal_note.as_deref(),
                Some(&rel_memo_path),
                &perspectives_json,
                &citations_json,
            )?;

            let result = DeepResearchResult {
                run_id,
                ticker,
                trigger,
                status: ResearchRunStatus::Completed.as_str().to_owned(),
                model: model_label,
                memo_path: Some(rel_memo_path),
                input_tokens: total_input_tokens,
                output_tokens: total_output_tokens,
                cost_usd,
                perspectives: session_batch.outcomes,
                citations: facts.citations,
                error: non_fatal_note,
                finished_at_ms,
            };

            emit_event(
                &event_tx,
                DeepResearchEvent::Completed {
                    result: Box::new(result.clone()),
                },
            );

            Ok(result)
        }
    }
}

struct SessionBatchOutput {
    outcomes: Vec<PerspectiveOutcome>,
    session_errors: Vec<String>,
}

struct DeterministicLakeFacts {
    universe_row: Option<UniverseRow>,
    peers: Option<PeerComparison>,
    citations: Vec<SignalEvidence>,
}

struct MemoRenderContext<'a> {
    run_id: &'a str,
    ticker: &'a str,
    trigger: &'a str,
    focus: Option<&'a str>,
    model: &'a str,
    input_tokens: u64,
    output_tokens: u64,
    cost_usd: Decimal,
    finished_at_ms: u64,
}

fn normalize_trigger(raw: &str) -> String {
    let trimmed = raw.trim().to_ascii_lowercase();
    match trimmed.as_str() {
        "manual" | "inbox" | "cli" | "auto" => trimmed,
        "" => "manual".to_owned(),
        other => other.to_owned(),
    }
}

fn model_label_for_llm(llm: &LlmSource) -> String {
    match llm {
        LlmSource::SharedProductCredentials => "connection-service-default".to_owned(),
        LlmSource::IsolatedHostStore => "isolated-host-store".to_owned(),
        LlmSource::Client(_) => "embedded-llm-client".to_owned(),
        LlmSource::SharedProvider(_) => "shared-llm-provider".to_owned(),
        LlmSource::Factory(_) => "custom-llm-factory".to_owned(),
    }
}

fn emit_event(tx: &Option<Sender<DeepResearchEvent>>, event: DeepResearchEvent) {
    if let Some(sender) = tx {
        let _ = sender.send(event);
    }
}

#[allow(clippy::too_many_arguments)]
fn record_failed_run(
    paths: &WorkspacePaths,
    run_id: &str,
    input_tokens: u64,
    output_tokens: u64,
    cost_usd: Decimal,
    finished_at_ms: u64,
    error: &str,
    perspectives_json: &str,
    citations_json: &str,
) -> Result<()> {
    let db = LakeDb::open(paths)?;
    let _ = db.finish_research_run(
        run_id,
        ResearchRunStatus::Failed,
        input_tokens,
        output_tokens,
        cost_usd,
        finished_at_ms,
        Some(error),
        None,
        perspectives_json,
        citations_json,
    )?;
    Ok(())
}

fn check_per_run_ceiling(
    cfg: &BudgetConfig,
    total_tokens: u64,
    cost_usd: Decimal,
    session_errors: &[String],
    outcome_count: usize,
) -> Option<String> {
    if outcome_count == 0 || session_errors.len() >= 4 {
        let joined = if session_errors.is_empty() {
            "全部 4 个研究视角均未返回有效结论".to_owned()
        } else {
            session_errors.join("; ")
        };
        return Some(friendly_m3_credentials_error(&joined));
    }
    if total_tokens > cfg.per_run_token_limit {
        return Some(format!(
            "单次 Deep Research Token 消耗 ({total_tokens}) 超过单次上限 ({})",
            cfg.per_run_token_limit
        ));
    }
    if cost_usd > cfg.per_run_budget_usd {
        return Some(format!(
            "单次 Deep Research 费用 (${}) 超过单次上限 (${})",
            cost_usd.round_dp(4).normalize(),
            cfg.per_run_budget_usd.round_dp(2).normalize()
        ));
    }
    None
}

fn run_parallel_sessions(
    paths: WorkspacePaths,
    ticker: String,
    focus: Option<String>,
    llm: LlmSource,
    event_tx: Option<Sender<DeepResearchEvent>>,
) -> Result<SessionBatchOutput> {
    let handle = std::thread::Builder::new()
        .name(format!("m3-deep-research-{ticker}"))
        .stack_size(8 * 1024 * 1024)
        .spawn(move || -> Result<SessionBatchOutput> {
            let rt = tokio::runtime::Builder::new_multi_thread()
                .worker_threads(4)
                .enable_all()
                .build()
                .context("无法创建 Deep Research Tokio 运行时")?;

            rt.block_on(async move {
                let fs = Arc::new(M3WorkbenchFs::new(paths.clone()));
                let app = boot_m3_embedded(paths.clone(), fs, llm).await?;

                let (r_moat, r_acc, r_peers, r_pre) = tokio::join!(
                    run_single_perspective(
                        &app,
                        DeepResearchPerspective::Moat,
                        &ticker,
                        focus.as_deref(),
                        &event_tx,
                    ),
                    run_single_perspective(
                        &app,
                        DeepResearchPerspective::Accounting,
                        &ticker,
                        focus.as_deref(),
                        &event_tx,
                    ),
                    run_single_perspective(
                        &app,
                        DeepResearchPerspective::LatentPeers,
                        &ticker,
                        focus.as_deref(),
                        &event_tx,
                    ),
                    run_single_perspective(
                        &app,
                        DeepResearchPerspective::Premortem,
                        &ticker,
                        focus.as_deref(),
                        &event_tx,
                    ),
                );

                let shutdown_res = app.shutdown().await;
                if let Err(err) = shutdown_res {
                    log::warn!("EmbeddedAlan shutdown warning during deep research: {err:#}");
                }

                let mut outcomes = Vec::with_capacity(4);
                let mut session_errors = Vec::new();
                for (perspective, res) in [
                    (DeepResearchPerspective::Moat, r_moat),
                    (DeepResearchPerspective::Accounting, r_acc),
                    (DeepResearchPerspective::LatentPeers, r_peers),
                    (DeepResearchPerspective::Premortem, r_pre),
                ] {
                    match res {
                        Ok(outcome) => outcomes.push(outcome),
                        Err(err) => {
                            let msg = friendly_m3_credentials_error(&format!("{err:#}"));
                            session_errors.push(format!(
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

                Ok(SessionBatchOutput {
                    outcomes,
                    session_errors,
                })
            })
        })
        .context("无法启动 Deep Research 工作线程")?;

    match handle.join() {
        Ok(res) => res,
        Err(_) => anyhow::bail!("Deep Research 执行线程意外中止"),
    }
}

async fn run_single_perspective(
    app: &EmbeddedAlan,
    perspective: DeepResearchPerspective,
    ticker: &str,
    focus: Option<&str>,
    event_tx: &Option<Sender<DeepResearchEvent>>,
) -> Result<PerspectiveOutcome> {
    let perspective_id = perspective.id().to_owned();
    let label = perspective.title_cn().to_owned();

    emit_event(
        event_tx,
        DeepResearchEvent::PerspectiveStarted {
            perspective: perspective_id.clone(),
            label: label.clone(),
        },
    );

    let opts = SessionOptions::default().with_system_prompt_mode(SystemPromptMode::Replace {
        prompt: perspective_system_prompt(perspective),
    });
    let session = app
        .spawn_child_session(app.root_pid(), opts)
        .await
        .with_context(|| format!("创建子会话 {perspective_id} 失败"))?;

    let result =
        drive_perspective_session(session.client(), perspective, ticker, focus, event_tx).await;

    if let Err(err) = session.close().await {
        log::warn!("关闭子会话 {perspective_id} 时发生警告: {err:#}");
    }

    result
}

async fn drive_perspective_session(
    client: &alan_os_host::embed::RootAgentClient,
    perspective: DeepResearchPerspective,
    ticker: &str,
    focus: Option<&str>,
    event_tx: &Option<Sender<DeepResearchEvent>>,
) -> Result<PerspectiveOutcome> {
    let perspective_id = perspective.id().to_owned();
    let label = perspective.title_cn().to_owned();

    let mut stream = client
        .event_stream()
        .await
        .with_context(|| format!("打开子会话 {perspective_id} 事件流失败"))?;

    let prompt = compose_perspective_prompt(perspective, ticker, focus);
    let submission_id = client
        .submit_text(prompt)
        .await
        .with_context(|| format!("向子会话 {perspective_id} 提交任务失败"))?;

    let mut content = String::new();
    let mut tools_used = Vec::new();
    let mut seen_tools = HashSet::new();
    let mut input_tokens: u64 = 0;
    let mut output_tokens: u64 = 0;

    let deadline = tokio::time::Instant::now() + Duration::from_secs(DEFAULT_SESSION_TIMEOUT_SECS);
    loop {
        let ev = tokio::time::timeout_at(deadline, stream.next_event())
            .await
            .with_context(|| format!("子会话 {perspective_id} 执行超时"))??;

        match ev {
            EmbedEvent::TextDelta { chunk, .. } => {
                if !chunk.is_empty() {
                    content.push_str(&chunk);
                    emit_event(
                        event_tx,
                        DeepResearchEvent::PerspectiveDelta {
                            perspective: perspective_id.clone(),
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
                    DeepResearchEvent::ToolStarted {
                        perspective: perspective_id.clone(),
                        name,
                        title,
                    },
                );
            }
            EmbedEvent::ToolFinished { name, status, .. } => {
                if seen_tools.insert(name.clone()) {
                    tools_used.push(name.clone());
                }
                emit_event(
                    event_tx,
                    DeepResearchEvent::ToolFinished {
                        perspective: perspective_id.clone(),
                        name,
                        status,
                    },
                );
            }
            EmbedEvent::Usage { usage, .. } => {
                input_tokens = input_tokens.max(usage.session_input_tokens.max(usage.input_tokens));
                output_tokens =
                    output_tokens.max(usage.session_output_tokens.max(usage.output_tokens));
            }
            EmbedEvent::PendingRequest { request_id, .. } => {
                let _ = client
                    .respond_request(&request_id, "denied: non-interactive deep research")
                    .await;
            }
            EmbedEvent::Error {
                message,
                recoverable,
            } => {
                if !recoverable {
                    anyhow::bail!("{message}");
                }
            }
            EmbedEvent::TurnFinished {
                submission_id: finished_id,
                status,
                error,
            } if finished_id == submission_id => {
                if status != UiInputStatus::Completed {
                    let msg = error.unwrap_or_else(|| format!("状态 {status:?}"));
                    anyhow::bail!("{msg}");
                }
                break;
            }
            EmbedEvent::ThinkingDelta { .. }
            | EmbedEvent::ToolProgress { .. }
            | EmbedEvent::TurnFinished { .. } => {}
        }
    }

    let trimmed_content = content.trim().to_owned();
    let summary_chars = trimmed_content.chars().count();
    emit_event(
        event_tx,
        DeepResearchEvent::PerspectiveFinished {
            perspective: perspective_id.clone(),
            summary_chars,
        },
    );

    Ok(PerspectiveOutcome {
        perspective: perspective_id,
        title: label,
        content: trimmed_content,
        tools_used,
        input_tokens,
        output_tokens,
    })
}

fn perspective_system_prompt(perspective: DeepResearchPerspective) -> String {
    let role_focus = match perspective {
        DeepResearchPerspective::Moat => {
            "你的专责视角是【护城河与商业生态 (Moat)】。\
             优先调用 `m3_report`、`m3_dossier`、`m3_events_for_ticker` 与 `m3_universe_query`，\
             聚焦单位经济模型、转换成本、网络效应、定价权、资本配置（回购/再投资效率）与 10-K 业务壁垒。"
        }
        DeepResearchPerspective::Accounting => {
            "你的专责视角是【会计排雷与现金流真相 (Accounting)】。\
             优先调用 `m3_report`、`m3_events_for_ticker` 与 `m3_universe_query`，\
             严查 OCF 与营业利润背离、CapEx 资本化比例、SBC 对经营现金流的侵蚀 (`sbc_to_ocf_pct`)、\
             净负债覆盖率、`warnings` 预警条目及 XBRL 溯源异常。"
        }
        DeepResearchPerspective::LatentPeers => {
            "你的专责视角是【隐空间同行与相对位置 (LatentPeers)】。\
             优先调用 `m3_peer_percentiles`、`m3_semantic_neighbors` 与 `m3_report`，\
             将目标公司的 5 年 ROIC、毛利率、严格股东盈余收益率放入 SIC 同行分位数与隐空间相似商业原型中对比，\
             引用 `SignalEvidence` 一手原文摘要说明竞争格局差异。"
        }
        DeepResearchPerspective::Premortem => {
            "你的专责视角是【5 年逆向事前验尸 (Premortem)】。\
             优先调用 `m3_dossier`、`m3_report`、`m3_events_for_ticker` 与 `m3_semantic_neighbors`，\
             假设 5 年后该公司内在价值永久损失 50%，逆向推演最可能的 3–5 条致死路径、证伪指标与待向管理层/人机审讯台提出的费曼尖锐问题。"
        }
    };

    format!(
        "你是 Project-M3 Deep Research 并行研究团队的资深芒格派分析师。\n\
         {role_focus}\n\
         铁律：\n\
         1. 严禁自行心算、编造或篡改任何财务数字、估值倍数、ROIC 或击球价；所有财务数据必须来自只读 M3 工具输出。\n\
         2. 严禁修改或试图写入 `human_judgment`、`watchlist.json`、`journal.json`、`budget.json`、`themes.json` 或 `data/munger/` / `m3.duckdb`。\n\
         3. 引用确定性数据时附上证据标记 `[report:TICKER]`、`[xbrl:TICKER]`、`[dossier:TICKER]`；严禁给出直接买卖指令。\n\
         4. 输出精炼、结构化的中文分析要点（300–600 字），直击事实与反脆弱边界。"
    )
}

fn compose_perspective_prompt(
    perspective: DeepResearchPerspective,
    ticker: &str,
    focus: Option<&str>,
) -> String {
    let focus_line = focus
        .map(|f| format!("\n人类研究聚焦问题：{f}"))
        .unwrap_or_default();
    let tools_hint = match perspective {
        DeepResearchPerspective::Moat => {
            "`m3_report`、`m3_dossier`、`m3_events_for_ticker`（若非观察池公司可配合 `m3_universe_query`）"
        }
        DeepResearchPerspective::Accounting => {
            "`m3_report`、`m3_events_for_ticker`（若非观察池公司可配合 `m3_universe_query`）"
        }
        DeepResearchPerspective::LatentPeers => {
            "`m3_peer_percentiles`、`m3_semantic_neighbors`、`m3_report`"
        }
        DeepResearchPerspective::Premortem => {
            "`m3_dossier`、`m3_report`、`m3_events_for_ticker`、`m3_semantic_neighbors`"
        }
    };

    format!(
        "[Deep Research 并行视角：{} ({}) | 标的：{ticker}]{focus_line}\n\
         请先调用 {tools_hint} 核对一手确定性事实与财报/事件切片，再给出该视角的独立研究结论。",
        perspective.title_cn(),
        perspective.id()
    )
}

fn gather_deterministic_lake_facts(
    db: &LakeDb,
    ticker: &str,
    report: Option<&MungerReport>,
) -> DeterministicLakeFacts {
    let peers = db.query_peers(ticker, 5).ok();
    let universe_row = peers
        .as_ref()
        .map(|p| p.target.clone())
        .or_else(|| load_universe_row_fallback(db, ticker));

    let mut citations = Vec::new();
    let mut seen_keys = HashSet::new();

    // 1. Target company's own top SEC filing chunks (if L1 doc_chunks is populated).
    for ev in load_ticker_doc_chunk_citations(db, ticker, 5) {
        let key = format!(
            "{}:{}:{}",
            ev.chunk_id.as_deref().unwrap_or(""),
            ev.ticker,
            ev.source_url
        );
        if seen_keys.insert(key) {
            citations.push(ev);
        }
    }

    // 2. Cross-company lookalike citations via `LakeDb::search_similar`.
    let similar_filter = SimilarSearchFilter {
        exclude_financials: false,
        limit: 5,
        ..Default::default()
    };
    let as_ticker = universe_row.is_some();
    let query_needle = if as_ticker {
        ticker.to_owned()
    } else if let Some(rep) = report {
        format!("{} {} {}", rep.ticker, rep.name, rep.circle)
    } else {
        ticker.to_owned()
    };

    if let Ok(hits) = db.search_similar(&query_needle, as_ticker, &similar_filter) {
        for hit in hits {
            for ev in hit.evidence {
                let key = format!(
                    "{}:{}:{}",
                    ev.chunk_id.as_deref().unwrap_or(""),
                    ev.ticker,
                    ev.source_url
                );
                if seen_keys.insert(key) {
                    citations.push(ev);
                    if citations.len() >= 10 {
                        break;
                    }
                }
            }
            if citations.len() >= 10 {
                break;
            }
        }
    }

    DeterministicLakeFacts {
        universe_row,
        peers,
        citations,
    }
}

fn load_universe_row_fallback(db: &LakeDb, ticker: &str) -> Option<UniverseRow> {
    let filter = crate::lake::UniverseFilter {
        query: Some(ticker.to_owned()),
        exclude_financials: false,
        limit: 20,
        ..Default::default()
    };
    db.screen_universe(&filter).ok().and_then(|rows| {
        rows.into_iter()
            .find(|r| r.ticker.eq_ignore_ascii_case(ticker))
    })
}

fn load_ticker_doc_chunk_citations(db: &LakeDb, ticker: &str, limit: usize) -> Vec<SignalEvidence> {
    let table_ok = db
        .conn
        .query_row(
            "SELECT COUNT(*) FROM information_schema.tables WHERE table_name = 'doc_chunks'",
            [],
            |r| r.get::<_, i64>(0),
        )
        .unwrap_or(0)
        > 0;
    if !table_ok || limit == 0 {
        return Vec::new();
    }

    let sql_limit = i64::try_from(limit).unwrap_or(5);
    let Ok(mut stmt) = db.conn.prepare(
        "SELECT chunk_id, ticker, doc_type, section, filing_date, title, content, source_url
         FROM doc_chunks
         WHERE UPPER(ticker) = UPPER(?)
         ORDER BY
            CASE section
                WHEN 'item_1' THEN 0
                WHEN 'item_1a' THEN 1
                WHEN 'item_7' THEN 2
                WHEN 'item_8k' THEN 3
                ELSE 4
            END ASC,
            COALESCE(filing_date, '') DESC,
            chunk_id ASC
         LIMIT ?",
    ) else {
        return Vec::new();
    };

    let Ok(rows) = stmt.query_map(params![ticker, sql_limit], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, String>(3)?,
            r.get::<_, Option<String>>(4)?,
            r.get::<_, String>(5)?,
            r.get::<_, String>(6)?,
            r.get::<_, String>(7)?,
        ))
    }) else {
        return Vec::new();
    };

    let mut out = Vec::new();
    for item in rows.flatten() {
        let (chunk_id, sym, doc_type, section, filing_date, title, content, source_url) = item;
        let quote = extract_verbatim_snippet(&content, &sym, 280);
        out.push(SignalEvidence {
            chunk_id: Some(chunk_id),
            ticker: sym,
            doc_type,
            section,
            filing_date,
            title,
            quote,
            source_url,
            similarity: 1.0,
        });
    }
    out
}

fn render_living_memo(
    ctx: &MemoRenderContext<'_>,
    report: Option<&MungerReport>,
    facts: &DeterministicLakeFacts,
    outcomes: &[PerspectiveOutcome],
    session_errors: &[String],
    existing_memo: Option<&str>,
) -> String {
    let company_name = report
        .map(|r| r.name.as_str())
        .or_else(|| facts.universe_row.as_ref().map(|u| u.name.as_str()))
        .unwrap_or(ctx.ticker);

    let mut md = String::new();
    md.push_str(&format!(
        "# {} · {} — Living Research Memo\n\n",
        ctx.ticker, company_name
    ));
    md.push_str(&format!("- **Run ID**: `{}`\n", ctx.run_id));
    md.push_str(&format!(
        "- **Updated Date**: {}\n",
        store::civil_today_iso()
    ));
    md.push_str(&format!(
        "- **Updated At (ms)**: `{}`\n",
        ctx.finished_at_ms
    ));
    md.push_str(&format!("- **Trigger**: `{}`\n", ctx.trigger));
    md.push_str(&format!("- **Model**: `{}`\n", ctx.model));
    md.push_str(&format!(
        "- **Usage**: `{}` in / `{}` out tokens · `${}` USD\n",
        ctx.input_tokens,
        ctx.output_tokens,
        ctx.cost_usd.round_dp(4).normalize()
    ));
    if let Some(focus) = ctx.focus {
        md.push_str(&format!("- **Research Focus**: {focus}\n"));
    }
    md.push('\n');

    // Deterministic facts block — strictly sourced from `recompute_ticker` / `LakeDb`.
    md.push_str("## Deterministic Financial Snapshot\n\n");
    md.push_str(
        "> 以下财务与分位数指标全部直接来自 `store::recompute_ticker` 与 `LakeDb`，严禁由 LLM 改写或推算。\n\n",
    );

    if let Some(rep) = report {
        let val = rep.ttm_valuation.as_ref().or(rep.fy_valuation.as_ref());
        let price_str = rep
            .price
            .as_ref()
            .map(|p| format!("{} {}", rep.currency, p.price.round_dp(2).normalize()))
            .unwrap_or_else(|| "—".to_owned());
        let strike_str = val
            .and_then(|v| v.strike_price)
            .map(|s| format!("{} {}", rep.currency, s.round_dp(2).normalize()))
            .unwrap_or_else(|| "—".to_owned());
        let dist_str = val
            .and_then(|v| v.distance_to_strike_pct)
            .map(|d| format!("{:.2}%", d))
            .unwrap_or_else(|| "—".to_owned());
        let yield_str = val
            .and_then(|v| v.strict_fcf_yield_mcap_pct)
            .map(|y| format!("{:.2}%", y))
            .unwrap_or_else(|| "—".to_owned());
        let oe_str = val
            .and_then(|v| v.strict_owner_earnings)
            .map(|oe| format!("{} {}", rep.currency, oe.normalize()))
            .unwrap_or_else(|| "—".to_owned());
        let roic_str = rep
            .roic_5y_avg_pct
            .map(|v| format!("{:.2}%", v))
            .unwrap_or_else(|| "—".to_owned());
        let gm_str = rep
            .gross_margin_5y_avg_pct
            .map(|v| format!("{:.2}%", v))
            .unwrap_or_else(|| "—".to_owned());
        let share_5y_str = rep
            .share_change_5y_pct
            .map(|v| format!("{:.2}%", v))
            .unwrap_or_else(|| "—".to_owned());
        let nd_oe_str = rep
            .net_debt_to_owner_earnings
            .map(|v| format!("{:.2}x", v))
            .unwrap_or_else(|| "—".to_owned());

        md.push_str("| 指标 | 确定性数值 | 来源 |\n");
        md.push_str("| --- | --- | --- |\n");
        md.push_str(&format!(
            "| 篮子 / 击球状态 | `{}` / {} | `watchlist.json` + `calculator.rs` |\n",
            rep.bucket.code(),
            rep.status.label()
        ));
        md.push_str(&format!("| 最新价格 | {price_str} | `raw/prices` |\n"));
        md.push_str(&format!(
            "| 击球价 / 距击球 | {strike_str} / {dist_str} | `calculator.rs` |\n"
        ));
        md.push_str(&format!(
            "| 严格股东盈余 (OE) | {oe_str} | `calculator.rs` |\n"
        ));
        md.push_str(&format!(
            "| 严格 FCF 收益率 | {yield_str} | `calculator.rs` |\n"
        ));
        md.push_str(&format!(
            "| 5Y ROIC 均值 | {roic_str} | `calculator.rs` |\n"
        ));
        md.push_str(&format!("| 5Y 毛利率均值 | {gm_str} | `calculator.rs` |\n"));
        md.push_str(&format!(
            "| 5Y 股本变化 | {share_5y_str} | `calculator.rs` |\n"
        ));
        md.push_str(&format!(
            "| 净负债 / 股东盈余 | {nd_oe_str} | `calculator.rs` |\n"
        ));
        md.push_str(&format!(
            "| 最近财报期 / 预警 | {} / {} 项 | `raw/sec` |\n\n",
            rep.latest_filing_end.as_deref().unwrap_or("—"),
            rep.warnings.len()
        ));

        if !rep.warnings.is_empty() {
            md.push_str("**财报预警 (Deterministic Warnings)**:\n");
            for w in rep.warnings.iter().take(6) {
                md.push_str(&format!("- `{}` — {}: {}\n", w.code, w.title, w.detail));
            }
            md.push('\n');
        }
    } else if let Some(u) = facts.universe_row.as_ref() {
        let price_str = u
            .latest_price
            .map(|p| format!("${}", p.round_dp(2).normalize()))
            .unwrap_or_else(|| "—".to_owned());
        let roic_str = u
            .roic_5y_avg_pct
            .map(|v| format!("{:.2}%", v))
            .unwrap_or_else(|| "—".to_owned());
        let gm_str = u
            .gross_margin_5y_avg_pct
            .map(|v| format!("{:.2}%", v))
            .unwrap_or_else(|| "—".to_owned());
        let yield_str = u
            .strict_fcf_yield_pct
            .map(|v| format!("{:.2}%", v))
            .unwrap_or_else(|| "—".to_owned());

        md.push_str("| 指标 | 确定性数值 | 来源 |\n");
        md.push_str("| --- | --- | --- |\n");
        md.push_str(&format!(
            "| CIK / 交易所 | `{}` / {} | `m3.duckdb` |\n",
            u.cik, u.exchange
        ));
        md.push_str(&format!(
            "| SIC 行业 | {} ({}) | `m3.duckdb` |\n",
            u.sic.map_or_else(|| "—".to_owned(), |s| s.to_string()),
            u.sic_description.as_deref().unwrap_or("—")
        ));
        md.push_str(&format!(
            "| 最新价格 | {price_str} | `universe_snapshot` |\n"
        ));
        md.push_str(&format!(
            "| 5Y ROIC 均值 | {roic_str} | `universe_snapshot` |\n"
        ));
        md.push_str(&format!(
            "| 5Y 毛利率均值 | {gm_str} | `universe_snapshot` |\n"
        ));
        md.push_str(&format!(
            "| 严格 FCF 收益率 | {yield_str} | `universe_snapshot` |\n"
        ));
        md.push_str(&format!(
            "| 预警数量 | {} 项 | `universe_snapshot` |\n\n",
            u.warnings_count
        ));
    } else {
        md.push_str("_暂未找到该标的的本地计算报告或 Lakehouse 财报快照。_\n\n");
    }

    if let Some(peers) = facts.peers.as_ref() {
        let roic_p = peers
            .roic_percentile
            .map(|v| format!("P{:.0}", v))
            .unwrap_or_else(|| "—".to_owned());
        let gm_p = peers
            .gross_margin_percentile
            .map(|v| format!("P{:.0}", v))
            .unwrap_or_else(|| "—".to_owned());
        let fcf_p = peers
            .strict_fcf_yield_percentile
            .map(|v| format!("P{:.0}", v))
            .unwrap_or_else(|| "—".to_owned());
        md.push_str(&format!(
            "**同行分位数 ({} · {} 家对比)**: ROIC `{roic_p}` · 毛利率 `{gm_p}` · 严格 FCF 收益率 `{fcf_p}`\n\n",
            peers.sic_group, peers.peer_count
        ));
        if !peers.top_peers.is_empty() {
            let peer_list: Vec<String> = peers
                .top_peers
                .iter()
                .take(5)
                .map(|p| {
                    let r = p
                        .roic_5y_avg_pct
                        .map(|v| format!("{:.1}%", v))
                        .unwrap_or_else(|| "—".to_owned());
                    format!("{} (ROIC {r})", p.ticker)
                })
                .collect();
            md.push_str(&format!("- 头部同行：{}\n\n", peer_list.join(" · ")));
        }
    }

    if !session_errors.is_empty() {
        md.push_str("**部分视角执行提示**:\n");
        for err in session_errors {
            md.push_str(&format!("- {err}\n"));
        }
        md.push('\n');
    }

    // 4 Perspective sections
    let by_id: BTreeMap<&str, &PerspectiveOutcome> = outcomes
        .iter()
        .map(|o| (o.perspective.as_str(), o))
        .collect();
    for perspective in DeepResearchPerspective::all() {
        md.push_str(&format!(
            "## {} ({})\n\n",
            perspective.title_cn(),
            perspective.id()
        ));
        if let Some(outcome) = by_id.get(perspective.id()) {
            let tools = if outcome.tools_used.is_empty() {
                "none".to_owned()
            } else {
                outcome.tools_used.join(", ")
            };
            md.push_str(&format!(
                "_Tools: `{tools}` · Tokens: {} in / {} out_\n\n",
                outcome.input_tokens, outcome.output_tokens
            ));
            if outcome.content.is_empty() {
                md.push_str("_该视角未产生正文输出。_\n\n");
            } else {
                md.push_str(&outcome.content);
                md.push_str("\n\n");
            }
        } else {
            md.push_str("_未返回该视角结果。_\n\n");
        }
    }

    // Citations section
    md.push_str("## Evidence & Citations\n\n");
    if facts.citations.is_empty() {
        md.push_str("_暂无可引用的 SEC 原文切片或隐空间同行证据。_\n\n");
    } else {
        for (idx, ev) in facts.citations.iter().enumerate() {
            let date = ev.filing_date.as_deref().unwrap_or("—");
            let single_line_quote = ev.quote.split_whitespace().collect::<Vec<_>>().join(" ");
            md.push_str(&format!(
                "{}. **[{}]** `{} · {}` ({date}, sim `{:.2}`): \"{}\" — {}\n",
                idx + 1,
                ev.ticker,
                ev.doc_type,
                ev.section,
                ev.similarity,
                single_line_quote,
                ev.source_url
            ));
        }
        md.push('\n');
    }

    // Preserve any existing `## Human Notes` footer verbatim.
    if let Some(preserved_footer) = existing_memo.and_then(extract_human_notes_footer) {
        md.push_str(preserved_footer);
        if !preserved_footer.ends_with('\n') {
            md.push('\n');
        }
    } else {
        md.push_str(HUMAN_NOTES_HEADER);
        md.push_str(
            "\n\n<!-- 人类投资者亲笔备忘区：Alan Deep Research 永不覆盖此标题及以下内容 -->\n",
        );
    }

    md
}

fn extract_human_notes_footer(markdown: &str) -> Option<&str> {
    let mut offset = 0usize;
    for line in markdown.split_inclusive('\n') {
        let trimmed = line.trim_end_matches(['\r', '\n']).trim();
        if trimmed == HUMAN_NOTES_HEADER {
            return Some(&markdown[offset..]);
        }
        offset += line.len();
    }
    None
}

fn write_text_atomic(dest: &std::path::Path, content: &str) -> Result<()> {
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create memo dir {}", parent.display()))?;
    }
    let tmp = store::atomic_temp_path(dest);
    fs::write(&tmp, content.as_bytes())
        .with_context(|| format!("failed to write temp memo {}", tmp.display()))?;
    if let Err(err) = fs::rename(&tmp, dest) {
        let _ = fs::remove_file(&tmp);
        return Err(err).with_context(|| format!("failed to commit memo {}", dest.display()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::alan_interrogator::M3ToolPolicyHook;
    use crate::budget::BudgetConfig;
    use crate::lake_l1::{DocChunkRow, LakeEventRow, LakeFilingRow};
    use crate::latent::local_embed;
    use crate::model::{
        AnnualFinancials, Bucket, CompanyEntry, DataSource, DecisionJournal, Dossier,
        FieldProvenance, HumanJudgment, Latticework, PriceSnapshot, RawCompanyFactsSummary,
        ValuationAssumptions, WatchlistFile,
    };
    use alan_agent_engine::tools::ToolCapability;
    use alan_llm::{GenerationResponse, MockLlmProvider, TokenUsage, ToolCall};
    use alan_os_host::embed::{HookDecision, LlmClient, ToolPolicyHook};
    use serde_json::json;
    use std::sync::mpsc;

    fn temp_workspace(tag: &str) -> (PathBuf, WorkspacePaths) {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let dir = std::env::temp_dir().join(format!(
            "m3-research-test-{tag}-{}-{nanos}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        let paths = WorkspacePaths::from_root(dir.clone());
        fs::create_dir_all(&paths.workbench_dir).unwrap();
        fs::create_dir_all(paths.workbench_dir.join("dossiers")).unwrap();
        fs::create_dir_all(paths.data_dir.join("raw/sec")).unwrap();
        fs::create_dir_all(paths.data_dir.join("raw/prices")).unwrap();
        fs::create_dir_all(paths.data_dir.join("computed")).unwrap();

        let wl = WatchlistFile {
            schema_version: 1,
            benchmark_symbol: "^GSPC".into(),
            default_target_return_pct: Decimal::from(10),
            companies: vec![CompanyEntry {
                ticker: "AAPL".into(),
                name: "Apple Inc.".into(),
                bucket: Bucket::Core,
                source: DataSource::SecEdgar,
                cik: Some("0000320193".into()),
                currency: "USD".into(),
                circle: "消费生态 / 硬件+服务".into(),
                too_hard_reason: None,
                assumptions: ValuationAssumptions {
                    multiple_low: Decimal::from(18),
                    multiple_high: Decimal::from(22),
                    target_return_pct: Decimal::from(10),
                    expected_growth_pct: Decimal::from(8),
                    strike_price_override: None,
                },
            }],
        };
        store::save_watchlist(&paths, &wl).unwrap();

        let journal = DecisionJournal {
            schema_version: 1,
            account_note: "TEST".into(),
            currency: "USD".into(),
            starting_cash: Decimal::from(100_000),
            cash_balance: Decimal::from(100_000),
            benchmark_symbol: "^GSPC".into(),
            benchmark_start_price: None,
            positions: Vec::new(),
            entries: Vec::new(),
        };
        store::save_journal(&paths, &journal).unwrap();

        let facts = RawCompanyFactsSummary {
            schema_version: crate::edgar::SEC_FACTS_SCHEMA_VERSION,
            extractor_version: crate::edgar::SEC_EXTRACTOR_VERSION,
            ticker: "AAPL".into(),
            entity_name: "Apple Inc.".into(),
            cik: "0000320193".into(),
            source_url: "https://data.sec.gov/api/xbrl/companyfacts/CIK0000320193.json".into(),
            raw_sha256: "deadbeef12345678".into(),
            fetched_at_ms: 1_700_000_000_000,
            annuals: vec![AnnualFinancials {
                fiscal_year: 2025,
                period_end: "2025-09-27".into(),
                filed: Some("2025-11-01".into()),
                revenue: Decimal::from(400_000_000_000i64),
                gross_profit: Decimal::from(185_000_000_000i64),
                ebit: Decimal::from(125_000_000_000i64),
                tax_expense: Decimal::from(20_000_000_000i64),
                pretax_income: Decimal::from(124_000_000_000i64),
                ocf: Decimal::from(118_000_000_000i64),
                capex: Decimal::from(10_000_000_000i64),
                sbc: Decimal::from(12_000_000_000i64),
                buybacks: Decimal::from(90_000_000_000i64),
                cash_and_st: Some(Decimal::from(65_000_000_000i64)),
                total_debt: Decimal::from(100_000_000_000i64),
                equity: Decimal::from(60_000_000_000i64),
                diluted_shares: Decimal::from(15_200_000_000i64),
                ending_shares: Decimal::from(15_000_000_000i64),
                provenance: vec![FieldProvenance {
                    field: "ocf".into(),
                    source_tag: "us-gaap:NetCashProvidedByUsedInOperatingActivities".into(),
                    filed: Some("2025-11-01".into()),
                    period_end: "2025-09-27".into(),
                    note: None,
                }],
            }],
            ttm: None,
            manual_note: None,
        };
        store::write_json_atomic(&paths.sec_facts_summary_path("AAPL"), &facts).unwrap();

        let price = PriceSnapshot {
            schema_version: 1,
            symbol: "AAPL".into(),
            price: Decimal::from(180),
            previous_close: Some(Decimal::from(179)),
            currency: "USD".into(),
            exchange: "NMS".into(),
            market_time_ms: Some(1_700_000_000_000),
            fetched_at_ms: 1_700_000_000_000,
            source_url: "https://query1.finance.yahoo.com/v8/finance/chart/AAPL".into(),
        };
        store::write_json_atomic(&paths.price_path("AAPL"), &price).unwrap();

        let dossier = Dossier {
            schema_version: 1,
            ticker: "AAPL".into(),
            name: "Apple Inc.".into(),
            updated_at: "2026-11-18".into(),
            draft_source: "manual_seed".into(),
            three_sentence_business: "端到端软硬件生态锁定高留存用户并用回购放大每股股东盈余。"
                .into(),
            latticework: Latticework {
                microeconomics: "生态转换成本高".into(),
                ecology: "端侧 AI 与服务黏性".into(),
                psychology_incentives: "以每股自由现金流为核心配资".into(),
                capital_allocation: "持续净回购压缩股本".into(),
            },
            premortem: vec!["硬件换机周期拉长叠加强监管拆分 App Store".into()],
            open_questions: vec!["服务业务毛利率能否维持 70% 以上？".into()],
            human_judgment: HumanJudgment {
                pending_human_review: true,
                my_stance: Some("WAITING_FOR_STRIKE_150".into()),
                weight_rationale: Some("核心能力圈保留".into()),
                falsification_metrics: vec!["活跃设备增长转负".into()],
                feynman_checked_at: Some("2026-11-18".into()),
            },
        };
        store::write_json_atomic(&paths.dossier_path("AAPL"), &dossier).unwrap();

        // Seed LakeDb with L0 universe_snapshot + L1 filings/events/doc_chunks for citations & peers.
        let db = LakeDb::open(&paths).unwrap();
        db.conn
            .execute_batch(
                "INSERT INTO companies (cik, ticker, name, exchange, sic, sic_description, is_financial, is_active)
                 VALUES
                   ('0000320193', 'AAPL', 'Apple Inc.', 'Nasdaq', 3571, 'Electronic Computers', false, true),
                   ('0000789019', 'MSFT', 'Microsoft Corp.', 'Nasdaq', 7372, 'Prepackaged Software', false, true);
                 INSERT INTO universe_snapshot (
                   cik, ticker, name, exchange, sic, sic_description, is_financial,
                   latest_filing_end, years_of_data, quality_score, in_price_pool, in_watchlist,
                   bucket_opt, latest_price, strike_price, distance_to_strike_pct, strict_fcf_yield_pct,
                   roic_5y_avg_pct, roic_5y_stddev_pct, gross_margin_5y_avg_pct, gross_margin_5y_stddev_pct,
                   share_change_3y_pct, sbc_to_ocf_pct, net_debt_to_oe, strict_owner_earnings,
                   warnings_count, warnings_json, raw_sha256, exact_decimals_json
                 ) VALUES
                   ('0000320193', 'AAPL', 'Apple Inc.', 'Nasdaq', 3571, 'Electronic Computers', false,
                    '2025-09-27', 5, 92.5, true, true, 'core', 180.0, 150.0, 20.0, 4.2,
                    48.5, 2.1, 46.2, 1.0, -8.5, 10.1, 0.4, 96000000000.0, 0, '[]', 'deadbeef', '{}'),
                   ('0000789019', 'MSFT', 'Microsoft Corp.', 'Nasdaq', 7372, 'Prepackaged Software', false,
                    '2025-06-30', 5, 90.0, true, false, NULL, 420.0, 380.0, 10.5, 3.1,
                    32.0, 1.8, 69.0, 0.9, -2.0, 11.2, -0.3, 65000000000.0, 0, '[]', 'cafebabe', '{}');",
            )
            .unwrap();

        db.upsert_filings(&[LakeFilingRow {
            accession_no: "0000320193-25-000106".into(),
            cik: "0000320193".into(),
            ticker: "AAPL".into(),
            form: "10-K".into(),
            filing_date: "2025-11-01".into(),
            report_date: Some("2025-09-27".into()),
            primary_document: Some("aapl-20250927.htm".into()),
            primary_doc_description: Some("10-K".into()),
            items_json: "[]".into(),
            filing_url: "https://www.sec.gov/Archives/edgar/data/320193/000032019325000106/aapl-20250927.htm".into(),
            ingested_at_ms: 1_700_000_000_000,
        }])
        .unwrap();

        db.upsert_events(&[LakeEventRow {
            event_id: "evt-aapl-10k".into(),
            source: "sec_edgar".into(),
            cik: Some("0000320193".into()),
            ticker: Some("AAPL".into()),
            event_type: "10-K".into(),
            occurred_at_ms: 1_700_000_000_000,
            filing_date: Some("2025-11-01".into()),
            title: "AAPL 10-K FY2025".into(),
            summary: "Apple ecosystem services and installed base disclosure.".into(),
            url: "https://www.sec.gov/Archives/edgar/data/320193/000032019325000106/aapl-20250927.htm".into(),
            metadata_json: "{}".into(),
            processed_mask: 0,
        }])
        .unwrap();

        let aapl_text = "Apple ecosystem services recurring revenue installed base hardware software switching costs";
        let msft_text = "Microsoft enterprise cloud ecosystem recurring subscription software platform switching costs";
        let aapl_vec = serde_json::to_string(&local_embed(aapl_text)).unwrap();
        let msft_vec = serde_json::to_string(&local_embed(msft_text)).unwrap();

        db.upsert_doc_chunks(&[
            DocChunkRow {
                chunk_id: "aapl-item1-0".into(),
                cik: "0000320193".into(),
                ticker: "AAPL".into(),
                doc_type: "10-K".into(),
                source_id: "0000320193-25-000106".into(),
                period_end: Some("2025-09-27".into()),
                filing_date: Some("2025-11-01".into()),
                section: "item_1".into(),
                title: "Item 1. Business".into(),
                content: aapl_text.into(),
                content_sha256: "sha-aapl-1".into(),
                source_url: "https://www.sec.gov/Archives/edgar/data/320193/000032019325000106/aapl-20250927.htm".into(),
                embedding_model: "local-hashed-tfidf-v1".into(),
                embedding_dim: 256,
                embedding_json: aapl_vec,
            },
            DocChunkRow {
                chunk_id: "msft-item1-0".into(),
                cik: "0000789019".into(),
                ticker: "MSFT".into(),
                doc_type: "10-K".into(),
                source_id: "0000789019-25-000050".into(),
                period_end: Some("2025-06-30".into()),
                filing_date: Some("2025-07-30".into()),
                section: "item_1".into(),
                title: "Item 1. Business".into(),
                content: msft_text.into(),
                content_sha256: "sha-msft-1".into(),
                source_url: "https://www.sec.gov/Archives/edgar/data/789019/000078901925000050/msft-20250630.htm".into(),
                embedding_model: "local-hashed-tfidf-v1".into(),
                embedding_dim: 256,
                embedding_json: msft_vec,
            },
        ])
        .unwrap();

        (dir, paths)
    }

    fn tool_call_response(
        id: &str,
        name: &str,
        arguments: serde_json::Value,
    ) -> GenerationResponse {
        GenerationResponse {
            content: String::new(),
            thinking: None,
            thinking_signature: None,
            redacted_thinking: Vec::new(),
            tool_calls: vec![ToolCall {
                id: Some(id.to_string()),
                name: name.to_string(),
                arguments,
            }],
            usage: Some(TokenUsage {
                prompt_tokens: 20,
                cached_prompt_tokens: None,
                completion_tokens: 10,
                total_tokens: 30,
                reasoning_tokens: None,
                ..Default::default()
            }),
            finish_reason: Some("tool_calls".to_string()),
            warnings: Vec::new(),
            provider_response_id: None,
            provider_response_status: None,
        }
    }

    fn text_response(text: &str) -> GenerationResponse {
        GenerationResponse {
            content: text.to_string(),
            thinking: None,
            thinking_signature: None,
            redacted_thinking: Vec::new(),
            tool_calls: Vec::new(),
            usage: Some(TokenUsage {
                prompt_tokens: 35,
                cached_prompt_tokens: None,
                completion_tokens: 25,
                total_tokens: 60,
                reasoning_tokens: None,
                ..Default::default()
            }),
            finish_reason: Some("stop".to_string()),
            warnings: Vec::new(),
            provider_response_id: None,
            provider_response_status: None,
        }
    }

    #[test]
    fn deep_research_runs_four_parallel_perspectives_streams_events_preserves_human_notes_and_logs_run()
     {
        let (dir, paths) = temp_workspace("happy");

        // Pre-seed a Living Research Memo with custom `## Human Notes` that MUST be preserved verbatim.
        let existing_memo = "# Old Header\n\nStale AI summary.\n\n## Human Notes\n\n- 我自己的观察：必须紧盯服务毛利率是否跌破 68%。\n- 目标击球价：$150。\n";
        let memo_path = memo_path_for(&paths, "AAPL");
        fs::create_dir_all(memo_path.parent().unwrap()).unwrap();
        fs::write(&memo_path, existing_memo).unwrap();

        let provider = MockLlmProvider::new()
            .with_responses(vec![
                tool_call_response("c1", "m3_report", json!({"ticker": "AAPL"})),
                tool_call_response("c2", "m3_dossier", json!({"ticker": "AAPL"})),
                tool_call_response("c3", "m3_peer_percentiles", json!({"ticker": "AAPL", "limit": 5})),
                tool_call_response("c4", "m3_semantic_neighbors", json!({"ticker": "AAPL", "limit": 5})),
                tool_call_response("c5", "m3_events_for_ticker", json!({"ticker": "AAPL"})),
            ])
            .with_response(text_response(
                "已核对 AAPL 确定性报告 [report:AAPL]、XBRL 溯源 [xbrl:AAPL] 与审讯档案 [dossier:AAPL]。",
            ));

        let (tx, rx) = mpsc::channel();
        let req = DeepResearchRequest {
            ticker: "aapl".into(),
            focus: Some("服务生态韧性与回购效率".into()),
            trigger: "manual".into(),
        };

        let result = run_deep_research_with_llm(
            &paths,
            &req,
            LlmSource::Client(LlmClient::new(provider.clone())),
            Some(tx),
        )
        .unwrap();

        assert_eq!(result.status, "completed");
        assert_eq!(result.ticker, "AAPL");
        assert_eq!(result.perspectives.len(), 4);
        assert_eq!(result.memo_path.as_deref(), Some("workbench/memos/AAPL.md"));
        assert!(result.input_tokens > 0);
        assert!(result.output_tokens > 0);
        assert!(result.cost_usd > Decimal::ZERO);
        assert!(!result.citations.is_empty());

        let events: Vec<DeepResearchEvent> = rx.try_iter().collect();
        assert!(
            events.iter().any(
                |e| matches!(e, DeepResearchEvent::Started { ticker, .. } if ticker == "AAPL")
            )
        );
        let started_perspectives: HashSet<&str> = events
            .iter()
            .filter_map(|e| match e {
                DeepResearchEvent::PerspectiveStarted { perspective, .. } => {
                    Some(perspective.as_str())
                }
                _ => None,
            })
            .collect();
        assert_eq!(started_perspectives.len(), 4);
        assert!(
            events
                .iter()
                .any(|e| matches!(e, DeepResearchEvent::ToolStarted { .. }))
        );
        assert!(events.iter().any(
            |e| matches!(e, DeepResearchEvent::ToolFinished { status, .. } if status == "completed")
        ));
        assert!(
            events
                .iter()
                .any(|e| matches!(e, DeepResearchEvent::PerspectiveDelta { .. }))
        );
        assert!(
            events
                .iter()
                .any(|e| matches!(e, DeepResearchEvent::PerspectiveFinished { .. }))
        );
        assert!(
            events.iter().any(
                |e| matches!(e, DeepResearchEvent::Synthesizing { ticker } if ticker == "AAPL")
            )
        );
        assert!(
            events
                .iter()
                .any(|e| matches!(e, DeepResearchEvent::Completed { .. }))
        );

        // Verify memo on disk has deterministic figures + all 4 perspectives + preserved Human Notes.
        let written_memo = load_research_memo(&paths, "AAPL").unwrap();
        assert!(written_memo.contains("## Deterministic Financial Snapshot"));
        assert!(written_memo.contains("## 护城河 (moat)"));
        assert!(written_memo.contains("## 会计排雷 (accounting)"));
        assert!(written_memo.contains("## 隐空间同行 (latent_peers)"));
        assert!(written_memo.contains("## 5年逆向验尸 (premortem)"));
        assert!(written_memo.contains("## Evidence & Citations"));
        assert!(written_memo.contains("我自己的观察：必须紧盯服务毛利率是否跌破 68%。"));
        assert!(written_memo.contains("目标击球价：$150。"));

        // Verify research_runs table recorded `completed`.
        let db = LakeDb::open_read_only_if_exists(&paths).unwrap().unwrap();
        let runs = db.list_research_runs(Some("AAPL"), 10).unwrap();
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].status, ResearchRunStatus::Completed);
        assert_eq!(runs[0].run_id, result.run_id);
        assert_eq!(
            runs[0].memo_path.as_deref(),
            Some("workbench/memos/AAPL.md")
        );
        assert_eq!(runs[0].input_tokens, result.input_tokens);
        assert_eq!(runs[0].output_tokens, result.output_tokens);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn deep_research_budget_hard_gate_blocks_before_booting_or_calling_llm() {
        // Case 1: monthly_budget_usd = 0.00
        let (dir1, paths1) = temp_workspace("budget-zero-usd");
        let cfg1 = BudgetConfig {
            monthly_budget_usd: Decimal::ZERO,
            ..Default::default()
        };
        budget::save_budget_config(&paths1, &cfg1).unwrap();

        let provider1 = MockLlmProvider::new();
        let (tx1, rx1) = mpsc::channel();
        let res1 = run_deep_research_with_llm(
            &paths1,
            &DeepResearchRequest {
                ticker: "AAPL".into(),
                focus: None,
                trigger: "cli".into(),
            },
            LlmSource::Client(LlmClient::new(provider1.clone())),
            Some(tx1),
        )
        .unwrap();

        assert_eq!(res1.status, "blocked_budget");
        assert_eq!(res1.input_tokens, 0);
        assert_eq!(res1.output_tokens, 0);
        assert_eq!(res1.cost_usd, Decimal::ZERO);
        assert!(provider1.recorded_requests().is_empty());

        let events1: Vec<DeepResearchEvent> = rx1.try_iter().collect();
        assert!(events1.iter().any(
            |e| matches!(e, DeepResearchEvent::BudgetBlocked { ticker, .. } if ticker == "AAPL")
        ));
        assert!(
            !events1
                .iter()
                .any(|e| matches!(e, DeepResearchEvent::Started { .. }))
        );

        let db1 = LakeDb::open_read_only_if_exists(&paths1).unwrap().unwrap();
        let runs1 = db1.list_research_runs(Some("AAPL"), 10).unwrap();
        assert_eq!(runs1.len(), 1);
        assert_eq!(runs1[0].status, ResearchRunStatus::BlockedBudget);
        drop(db1);
        let _ = fs::remove_dir_all(&dir1);

        // Case 2: monthly_token_limit exceeded by prior run in same UTC month
        let (dir2, paths2) = temp_workspace("budget-token-limit");
        let cfg2 = BudgetConfig {
            monthly_token_limit: 500,
            ..Default::default()
        };
        budget::save_budget_config(&paths2, &cfg2).unwrap();

        let now = store::now_ms();
        {
            let db2 = LakeDb::open(&paths2).unwrap();
            db2.insert_research_run(&ResearchRunRow {
                run_id: "prior-heavy-run".into(),
                ticker: "MSFT".into(),
                trigger: "manual".into(),
                focus: None,
                model: "test".into(),
                input_tokens: 400,
                output_tokens: 150,
                cost_usd: Decimal::new(1, 2),
                started_at_ms: now,
                finished_at_ms: Some(now + 1000),
                status: ResearchRunStatus::Completed,
                error: None,
                memo_path: None,
                perspectives_json: "[]".into(),
                citations_json: "[]".into(),
            })
            .unwrap();
        }

        let provider2 = MockLlmProvider::new();
        let (tx2, rx2) = mpsc::channel();
        let res2 = run_deep_research_with_llm(
            &paths2,
            &DeepResearchRequest {
                ticker: "AAPL".into(),
                focus: None,
                trigger: "inbox".into(),
            },
            LlmSource::Client(LlmClient::new(provider2.clone())),
            Some(tx2),
        )
        .unwrap();

        assert_eq!(res2.status, "blocked_budget");
        assert!(res2.error.as_deref().unwrap_or("").contains("Token"));
        assert!(provider2.recorded_requests().is_empty());
        let events2: Vec<DeepResearchEvent> = rx2.try_iter().collect();
        assert!(
            events2
                .iter()
                .any(|e| matches!(e, DeepResearchEvent::BudgetBlocked { .. }))
        );

        let db2_ro = LakeDb::open_read_only_if_exists(&paths2).unwrap().unwrap();
        let runs2 = db2_ro.list_research_runs(Some("AAPL"), 10).unwrap();
        assert_eq!(runs2.len(), 1);
        assert_eq!(runs2[0].status, ResearchRunStatus::BlockedBudget);
        let _ = fs::remove_dir_all(&dir2);
    }

    #[test]
    fn deep_research_enforces_policy_redlines_and_never_mutates_forbidden_files() {
        let (dir, paths) = temp_workspace("redline");
        let budget_cfg = BudgetConfig::default();
        budget::save_budget_config(&paths, &budget_cfg).unwrap();

        // Direct hook assertions for watchlist.json, m3.duckdb, human_judgment
        let hook = M3ToolPolicyHook;
        assert!(matches!(
            hook.evaluate(
                "write_file",
                &json!({"path": "workbench/watchlist.json", "content": "{}"}),
                ToolCapability::Write
            ),
            HookDecision::Deny(_)
        ));
        assert!(matches!(
            hook.evaluate(
                "write_file",
                &json!({"path": "data/munger/lake/m3.duckdb", "content": "corrupt"}),
                ToolCapability::Write
            ),
            HookDecision::Deny(_)
        ));

        let wl_before = fs::read_to_string(paths.watchlist_path()).unwrap();
        let journal_before = fs::read_to_string(paths.journal_path()).unwrap();
        let budget_before = fs::read_to_string(budget::budget_path(&paths)).unwrap();
        let dossier_before = store::load_dossier(&paths, "AAPL").unwrap().unwrap();

        // In-process EmbeddedAlan run where MockLlmProvider tries forbidden writes via m3_update_draft
        let provider = MockLlmProvider::new()
            .with_responses(vec![
                tool_call_response(
                    "bad_hj",
                    "m3_update_draft",
                    json!({
                        "ticker": "AAPL",
                        "human_judgment": {"my_stance": "FORCE_BUY", "pending_human_review": false}
                    }),
                ),
                tool_call_response(
                    "bad_wl",
                    "m3_update_draft",
                    json!({
                        "ticker": "AAPL",
                        "three_sentence_business": "tamper workbench/watchlist.json"
                    }),
                ),
                tool_call_response(
                    "bad_db",
                    "m3_update_draft",
                    json!({
                        "ticker": "AAPL",
                        "open_questions": ["overwrite data/munger/lake/m3.duckdb"]
                    }),
                ),
            ])
            .with_response(text_response("严格只读核对完成 [report:AAPL]。"));

        let res = run_deep_research_with_llm(
            &paths,
            &DeepResearchRequest {
                ticker: "AAPL".into(),
                focus: None,
                trigger: "manual".into(),
            },
            LlmSource::Client(LlmClient::new(provider.clone())),
            None,
        )
        .unwrap();

        assert_eq!(res.status, "completed");

        // Verify policy hook intercepted all three forbidden attempts in LLM conversation history.
        let recorded = provider.recorded_requests();
        assert!(recorded.iter().any(|r| {
            r.messages
                .iter()
                .any(|m| m.content.contains("红线拦截") && m.content.contains("human_judgment"))
        }));
        assert!(recorded.iter().any(|r| {
            r.messages
                .iter()
                .any(|m| m.content.contains("红线拦截") && m.content.contains("watchlist.json"))
        }));
        assert!(recorded.iter().any(|r| {
            r.messages
                .iter()
                .any(|m| m.content.contains("红线拦截") && m.content.contains("m3.duckdb"))
        }));

        // Verify forbidden files & human_judgment remained byte-for-byte intact.
        assert_eq!(
            fs::read_to_string(paths.watchlist_path()).unwrap(),
            wl_before
        );
        assert_eq!(
            fs::read_to_string(paths.journal_path()).unwrap(),
            journal_before
        );
        assert_eq!(
            fs::read_to_string(budget::budget_path(&paths)).unwrap(),
            budget_before
        );
        let dossier_after = store::load_dossier(&paths, "AAPL").unwrap().unwrap();
        assert_eq!(
            dossier_after.human_judgment.my_stance,
            dossier_before.human_judgment.my_stance
        );
        assert_eq!(
            dossier_after.human_judgment.pending_human_review,
            dossier_before.human_judgment.pending_human_review
        );

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn deep_research_records_failed_runs_on_boot_error_and_per_run_ceiling() {
        // Case 1: unconfigured / failing LLM boot records Failed row & emits Failed event.
        let (dir1, paths1) = temp_workspace("boot-fail");
        let (tx1, rx1) = mpsc::channel();
        let res1 = run_deep_research_with_llm(
            &paths1,
            &DeepResearchRequest {
                ticker: "AAPL".into(),
                focus: None,
                trigger: "manual".into(),
            },
            LlmSource::IsolatedHostStore,
            Some(tx1),
        )
        .unwrap();
        assert_eq!(res1.status, "failed");
        assert!(res1.error.is_some());
        let events1: Vec<DeepResearchEvent> = rx1.try_iter().collect();
        assert!(
            events1
                .iter()
                .any(|e| matches!(e, DeepResearchEvent::Started { .. }))
        );
        assert!(
            events1
                .iter()
                .any(|e| matches!(e, DeepResearchEvent::Failed { .. }))
        );
        let db1 = LakeDb::open_read_only_if_exists(&paths1).unwrap().unwrap();
        let runs1 = db1.list_research_runs(Some("AAPL"), 10).unwrap();
        assert_eq!(runs1.len(), 1);
        assert_eq!(runs1[0].status, ResearchRunStatus::Failed);
        drop(db1);
        let _ = fs::remove_dir_all(&dir1);

        // Case 2: per-run token ceiling exceeded records Failed with accurate token usage & cost.
        let (dir2, paths2) = temp_workspace("per-run-ceiling");
        let cfg2 = BudgetConfig {
            per_run_token_limit: 50,
            ..Default::default()
        };
        budget::save_budget_config(&paths2, &cfg2).unwrap();

        let provider2 = MockLlmProvider::new().with_response(text_response("ok"));
        let (tx2, rx2) = mpsc::channel();
        let res2 = run_deep_research_with_llm(
            &paths2,
            &DeepResearchRequest {
                ticker: "AAPL".into(),
                focus: None,
                trigger: "cli".into(),
            },
            LlmSource::Client(LlmClient::new(provider2)),
            Some(tx2),
        )
        .unwrap();
        assert_eq!(res2.status, "failed");
        assert!(res2.input_tokens + res2.output_tokens > 50);
        assert!(res2.cost_usd > Decimal::ZERO);
        assert!(res2.error.as_deref().unwrap_or("").contains("超过单次上限"));
        let events2: Vec<DeepResearchEvent> = rx2.try_iter().collect();
        assert!(
            events2
                .iter()
                .any(|e| matches!(e, DeepResearchEvent::Failed { .. }))
        );
        let db2 = LakeDb::open_read_only_if_exists(&paths2).unwrap().unwrap();
        let runs2 = db2.list_research_runs(Some("AAPL"), 10).unwrap();
        assert_eq!(runs2.len(), 1);
        assert_eq!(runs2[0].status, ResearchRunStatus::Failed);
        assert_eq!(runs2[0].input_tokens, res2.input_tokens);
        assert_eq!(runs2[0].output_tokens, res2.output_tokens);
        let _ = fs::remove_dir_all(&dir2);
    }
}
