//! Three-round Socratic Feynman Defense & Human Judgment Sign-off core logic.
//!
//! Supports both Company Dossier defense (`workbench/memos/<TICKER>-feynman.md`)
//! and Industry Studio defense (`workbench/industries/<slug>/feynman-defense.md`),
//! keeping human judgment sovereignty strictly separated from Alan critique outputs.

use std::collections::HashSet;
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use alan_os_host::embed::{EmbedEvent, LlmSource, UiInputStatus};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::alan_connection;
use crate::alan_interrogator::{self, M3WorkbenchFs};
use crate::i18n::UiLang;
use crate::industry::IndustryProfile;
use crate::market;
use crate::model::Dossier;
use crate::store::{self, WorkspacePaths};

const DEFAULT_FEYNMAN_EVAL_TIMEOUT_SECS: u64 = 90;

/// Target scope of a 3-round Socratic Feynman Defense session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum FeynmanScope {
    Company { ticker: String },
    Industry { slug: String },
}

impl FeynmanScope {
    /// Returns a normalized copy (uppercased company ticker, trimmed industry slug).
    pub fn normalized(&self) -> Self {
        match self {
            Self::Company { ticker } => Self::Company {
                ticker: ticker.trim().to_ascii_uppercase(),
            },
            Self::Industry { slug } => Self::Industry {
                slug: slug.trim().to_owned(),
            },
        }
    }

    /// Short display identifier (`TICKER` or `slug`).
    pub fn identifier(&self) -> String {
        match self {
            Self::Company { ticker } => ticker.trim().to_ascii_uppercase(),
            Self::Industry { slug } => slug.trim().to_owned(),
        }
    }
}

/// Ordered Socratic defense rounds required before graduation / sign-off.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[repr(u8)]
pub enum FeynmanRound {
    MoatAndEconomics = 1,
    FailureModesAndFalsification = 2,
    ValuationAndDiscipline = 3,
}

impl FeynmanRound {
    /// All rounds in canonical sequence (`1 -> 2 -> 3`).
    pub const ALL: [Self; 3] = [
        Self::MoatAndEconomics,
        Self::FailureModesAndFalsification,
        Self::ValuationAndDiscipline,
    ];

    /// Numeric 1-based round identifier (`1..=3`).
    pub const fn as_u8(self) -> u8 {
        self as u8
    }

    /// Parse a 1-based round number (`1..=3`).
    pub const fn from_u8(raw: u8) -> Option<Self> {
        match raw {
            1 => Some(Self::MoatAndEconomics),
            2 => Some(Self::FailureModesAndFalsification),
            3 => Some(Self::ValuationAndDiscipline),
            _ => None,
        }
    }

    /// Localized round title.
    pub const fn title_for(self, lang: UiLang) -> &'static str {
        match (self, lang) {
            (Self::MoatAndEconomics, UiLang::Zh) => "第 1 轮 · 护城河与单位经济",
            (Self::MoatAndEconomics, UiLang::En) => "Round 1 · Moat & Unit Economics",
            (Self::FailureModesAndFalsification, UiLang::Zh) => "第 2 轮 · 五年死法与可证伪指标",
            (Self::FailureModesAndFalsification, UiLang::En) => {
                "Round 2 · 5-Year Failure Modes & Falsification"
            }
            (Self::ValuationAndDiscipline, UiLang::Zh) => "第 3 轮 · 估值安全边际与卖出纪律",
            (Self::ValuationAndDiscipline, UiLang::En) => {
                "Round 3 · Valuation Margin of Safety & Kill Criteria"
            }
        }
    }

    /// Next round in the 3-round defense sequence, if any.
    pub const fn next(self) -> Option<Self> {
        match self {
            Self::MoatAndEconomics => Some(Self::FailureModesAndFalsification),
            Self::FailureModesAndFalsification => Some(Self::ValuationAndDiscipline),
            Self::ValuationAndDiscipline => None,
        }
    }
}

/// One completed Socratic Q&A + Alan critique turn in a Feynman Defense session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FeynmanTurn {
    pub round: u8,
    pub prompt: String,
    pub human_answer: String,
    #[serde(default)]
    pub alan_critique: String,
    #[serde(default)]
    pub blind_spots: Vec<String>,
    #[serde(default)]
    pub suggested_metrics: Vec<String>,
}

/// Structured critique extracted from Alan's response to one Feynman defense round.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ParsedFeynmanCritique {
    #[serde(default)]
    pub critique: String,
    #[serde(default)]
    pub blind_spots: Vec<String>,
    #[serde(default)]
    pub suggested_falsification_metrics: Vec<String>,
}

/// Persisted summary of a completed human Feynman defense session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FeynmanDefenseSummary {
    pub scope: FeynmanScope,
    pub completed_at: String,
    pub my_stance: String,
    #[serde(default)]
    pub falsification_metrics: Vec<String>,
    #[serde(default)]
    pub blind_spots: Vec<String>,
    #[serde(default)]
    pub turns: Vec<FeynmanTurn>,
}

/// Returns `true` when promoting a company from `WATCH` to `IN` should show the
/// soft Feynman graduation gate warning.
pub fn needs_feynman_graduation_gate(dossier: Option<&Dossier>) -> bool {
    match dossier {
        None => true,
        Some(d) => {
            d.human_judgment.pending_human_review
                || d.human_judgment
                    .feynman_checked_at
                    .as_deref()
                    .map(str::trim)
                    .is_none_or(str::is_empty)
        }
    }
}

/// Builds the human-facing Socratic prompt for the specified Feynman defense round.
pub fn round_question_prompt(
    scope: &FeynmanScope,
    round: FeynmanRound,
    subject_label: &str,
    lang: UiLang,
) -> String {
    let fallback_label = scope.identifier();
    let label = if subject_label.trim().is_empty() {
        fallback_label.as_str()
    } else {
        subject_label.trim()
    };

    match (scope, round, lang) {
        (FeynmanScope::Company { .. }, FeynmanRound::MoatAndEconomics, UiLang::Zh) => format!(
            "请用大白话解释：{label} 到底靠什么赚超额利润？客户为什么离不开它（转换成本、网络效应或成本优势），它的单位经济和 ROIC 凭什么能长期高于同行？"
        ),
        (FeynmanScope::Company { .. }, FeynmanRound::MoatAndEconomics, UiLang::En) => format!(
            "Explain in plain language: how does {label} earn durable excess returns? Why can customers not easily switch away, and what keeps its unit economics and ROIC structurally above peers?"
        ),
        (FeynmanScope::Company { .. }, FeynmanRound::FailureModesAndFalsification, UiLang::Zh) => {
            format!(
                "做一次 5 年逆向事前验尸：如果 5 年后 {label} 让股东永久亏损，最可能的 2–3 种死法是什么？你会设定哪些可量化、可追踪的事前证伪指标（如毛利率、ROIC、SBC/OCF、份额）来证明自己看错了？"
            )
        }
        (FeynmanScope::Company { .. }, FeynmanRound::FailureModesAndFalsification, UiLang::En) => {
            format!(
                "Run a 5-year premortem: if {label} suffers permanent capital impairment over the next 5 years, what are the 2–3 most plausible failure modes, and which concrete, testable falsification metrics (e.g., gross margin, ROIC, SBC/OCF, share) would prove your thesis wrong?"
            )
        }
        (FeynmanScope::Company { .. }, FeynmanRound::ValuationAndDiscipline, UiLang::Zh) => {
            format!(
                "请明确你对 {label} 的击球价安全边际与卖出纪律：在什么严格业主盈余倍数或预期年化回报下你才愿意重仓？如果未来出现什么基本面或治理红线，你会毫不犹豫止损或移出核心击球区？"
            )
        }
        (FeynmanScope::Company { .. }, FeynmanRound::ValuationAndDiscipline, UiLang::En) => {
            format!(
                "State your valuation margin of safety and kill criteria for {label}: what strict owner-earnings multiple or target return justifies committing capital, and what fundamental or governance red line triggers an immediate exit or downgrade?"
            )
        }
        (FeynmanScope::Industry { .. }, FeynmanRound::MoatAndEconomics, UiLang::Zh) => format!(
            "请用大白话讲清 {label} 这门生意：产业链上哪个环节真正留住利润池与定价权？客户购买的核心不可替代价值是什么，行业龙头与二线玩家的单位经济差距源自哪里？"
        ),
        (FeynmanScope::Industry { .. }, FeynmanRound::MoatAndEconomics, UiLang::En) => format!(
            "Explain {label} in plain language: which layer of the value chain captures the profit pool and pricing power, what irreplaceable value do customers pay for, and why do leaders outperform laggards on unit economics?"
        ),
        (FeynmanScope::Industry { .. }, FeynmanRound::FailureModesAndFalsification, UiLang::Zh) => {
            format!(
                "复盘 {label} 过去 10–20 年的历史死法：这个行业最容易在什么周期陷阱、技术代际切换或资本开支竞赛中毁灭股东价值？有哪些可观测的领先指标能提前证伪你的行业判断？"
            )
        }
        (FeynmanScope::Industry { .. }, FeynmanRound::FailureModesAndFalsification, UiLang::En) => {
            format!(
                "Review 10–20 years of failure modes in {label}: which cyclical traps, technology shifts, or capex races destroy shareholder value, and which observable leading indicators would falsify your industry thesis early?"
            )
        }
        (FeynmanScope::Industry { .. }, FeynmanRound::ValuationAndDiscipline, UiLang::Zh) => {
            format!(
                "请说明你为 {label} 设定的合理股东盈余倍数区间（Valuation Anchor）依据是什么？当行业出现什么结构性信号时，说明它已经超出你的能力圈（Too Hard）必须果断放弃？"
            )
        }
        (FeynmanScope::Industry { .. }, FeynmanRound::ValuationAndDiscipline, UiLang::En) => {
            format!(
                "Justify your owner-earnings multiple anchor for {label}: why is that range durable across a full cycle, and what structural signal tells you the industry lies outside your circle of competence (Too Hard)?"
            )
        }
    }
}

/// Composes a single-turn Socratic critique prompt for Alan on a Company Dossier defense round.
pub fn compose_company_feynman_critique_prompt(
    ticker: &str,
    report_summary: &str,
    round: FeynmanRound,
    question: &str,
    human_answer: &str,
    existing_metrics: &[String],
    lang: UiLang,
) -> String {
    let norm_ticker = ticker.trim().to_ascii_uppercase();
    let summary_excerpt = if report_summary.trim().is_empty() {
        lang.t(
            "（暂无确定性财报摘要）",
            "(No deterministic report summary provided)",
        )
        .to_owned()
    } else {
        truncate_chars(report_summary.trim(), 2200)
    };
    let metrics_text = if existing_metrics.is_empty() {
        lang.t("（尚未登记事前证伪指标）", "(None recorded yet)")
            .to_owned()
    } else {
        existing_metrics.join("；")
    };
    let round_title = round.title_for(lang);

    match lang {
        UiLang::Zh => format!(
            "[费曼答辩审讯 · 公司标的：{norm_ticker} | {round_title}]\n\
             角色与纪律：\n\
             - 你是查理·芒格风格的严厉苏格拉底式审讯官 Alan。\n\
             - 严禁修改 `human_judgment`、`memos/` 或编造财务数字；严禁输出买卖指令。\n\
             - 重点挑刺人类回答中的模糊措辞、幸存者偏差、未量化的护城河假设与缺失的事前证伪阈值。\n\n\
             [确定性财务与档案摘要]\n{summary_excerpt}\n\n\
             [当前已登记事前证伪指标]\n{metrics_text}\n\n\
             [本轮费曼问题]\n{question}\n\n\
             [人类回答]\n{human_answer}\n\n\
             输出要求：请先给出一个合法的 ```json 代码块（必须包含 `critique`、`blind_spots`、`suggested_falsification_metrics` 三个字段），随后可附 1–2 句简明追问说明：\n\
             ```json\n\
             {{\n  \
               \"critique\": \"简明犀利的逻辑点评（120-260字）\",\n  \
               \"blind_spots\": [\"逻辑盲区1\", \"逻辑盲区2\"],\n  \
               \"suggested_falsification_metrics\": [\"可量化事前证伪指标1（含具体方向或阈值）\"]\n\
             }}\n\
             ```"
        ),
        UiLang::En => format!(
            "[Feynman Defense Critique · Company: {norm_ticker} | {round_title}]\n\
             Role & Guardrails:\n\
             - Act as Alan, a rigorous Munger-style Socratic defense coach.\n\
             - Never mutate `human_judgment` or `memos/`, never fabricate numbers, and never give buy/sell orders.\n\
             - Expose hand-waving, survivorship bias, untested moat claims, and missing quantitative falsification thresholds.\n\n\
             [Deterministic Report & Dossier Context]\n{summary_excerpt}\n\n\
             [Existing Falsification Metrics]\n{metrics_text}\n\n\
             [Round Question]\n{question}\n\n\
             [Human Answer]\n{human_answer}\n\n\
             Output format: reply in English starting with a valid ```json block containing `critique`, `blind_spots`, and `suggested_falsification_metrics`, optionally followed by a concise 1-2 sentence follow-up:\n\
             ```json\n\
             {{\n  \
               \"critique\": \"Concise, sharp logical critique (80-180 words)\",\n  \
               \"blind_spots\": [\"Blind spot 1\", \"Blind spot 2\"],\n  \
               \"suggested_falsification_metrics\": [\"Testable falsification metric with concrete threshold\"]\n\
             }}\n\
             ```"
        ),
    }
}

/// Composes a single-turn Socratic critique prompt for Alan on an Industry Studio defense round.
pub fn compose_industry_feynman_critique_prompt(
    profile: &IndustryProfile,
    primer_excerpt: &str,
    round: FeynmanRound,
    question: &str,
    human_answer: &str,
    lang: UiLang,
) -> String {
    let tickers = if profile.bound_tickers.is_empty() {
        lang.t("暂无绑定标的", "None").to_owned()
    } else {
        profile.bound_tickers.join(", ")
    };
    let anchor = profile
        .valuation_anchor_label()
        .unwrap_or_else(|| lang.t("尚未设定", "Unset").to_owned());
    let kpis = if profile.key_kpis.is_empty() {
        lang.t("（暂无预设 KPI）", "(No preset KPIs)").to_owned()
    } else {
        profile.key_kpis.join("；")
    };
    let excerpt = if primer_excerpt.trim().is_empty() {
        lang.t(
            "（暂无行业活手册摘要）",
            "(No industry primer excerpt available)",
        )
        .to_owned()
    } else {
        truncate_chars(primer_excerpt.trim(), 2000)
    };
    let round_title = round.title_for(lang);

    match lang {
        UiLang::Zh => format!(
            "[费曼答辩审讯 · 行业能力圈：{} (slug: `{}`) | {round_title}]\n\
             代表公司：{tickers} | 人类估值锚：{anchor} | 关注 KPI：{kpis}\n\
             角色与纪律：\n\
             - 你是查理·芒格风格的严厉苏格拉底式审讯官 Alan。\n\
             - 严禁修改 `workbench/industries/` 或 `human_judgment`；严禁输出买卖指令。\n\
             - 重点检验人类是否分清行业利润池、周期位置、历史死法与可观测证伪信号。\n\n\
             [行业活手册摘要]\n{excerpt}\n\n\
             [本轮费曼问题]\n{question}\n\n\
             [人类回答]\n{human_answer}\n\n\
             输出要求：请先给出一个合法的 ```json 代码块（必须包含 `critique`、`blind_spots`、`suggested_falsification_metrics` 三个字段），随后可附 1–2 句简明追问说明：\n\
             ```json\n\
             {{\n  \
               \"critique\": \"简明犀利的行业逻辑点评（120-260字）\",\n  \
               \"blind_spots\": [\"行业盲区1\", \"行业盲区2\"],\n  \
               \"suggested_falsification_metrics\": [\"可观测行业证伪指标1\"]\n\
             }}\n\
             ```",
            profile.title, profile.slug
        ),
        UiLang::En => format!(
            "[Feynman Defense Critique · Industry: {} (slug: `{}`) | {round_title}]\n\
             Bound Peers: {tickers} | Human Valuation Anchor: {anchor} | Key KPIs: {kpis}\n\
             Role & Guardrails:\n\
             - Act as Alan, a rigorous Munger-style Socratic defense coach.\n\
             - Never modify `workbench/industries/` or `human_judgment`, and never give buy/sell orders.\n\
             - Challenge weak profit-pool logic, cycle blindness, historical failure omissions, and untestable claims.\n\n\
             [Industry Primer Excerpt]\n{excerpt}\n\n\
             [Round Question]\n{question}\n\n\
             [Human Answer]\n{human_answer}\n\n\
             Output format: reply in English starting with a valid ```json block containing `critique`, `blind_spots`, and `suggested_falsification_metrics`, optionally followed by a concise 1-2 sentence follow-up:\n\
             ```json\n\
             {{\n  \
               \"critique\": \"Concise, sharp industry critique (80-180 words)\",\n  \
               \"blind_spots\": [\"Industry blind spot 1\", \"Industry blind spot 2\"],\n  \
               \"suggested_falsification_metrics\": [\"Observable industry falsification signal\"]\n\
             }}\n\
             ```",
            profile.title, profile.slug
        ),
    }
}

#[derive(Debug, Deserialize)]
struct RawCritiquePayload {
    #[serde(default)]
    critique: String,
    #[serde(default)]
    blind_spots: Vec<String>,
    #[serde(default, alias = "suggested_metrics")]
    suggested_falsification_metrics: Vec<String>,
}

/// Robustly parses Alan's critique response from fenced/inline JSON or markdown bullets.
pub fn parse_feynman_critique(raw: &str) -> ParsedFeynmanCritique {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return ParsedFeynmanCritique::default();
    }

    if let Some((payload, remainder)) = extract_critique_json(trimmed) {
        let mut critique = payload.critique.trim().to_owned();
        if critique.is_empty() && !remainder.trim().is_empty() {
            critique = remainder.trim().to_owned();
        }
        return ParsedFeynmanCritique {
            critique,
            blind_spots: normalize_items(payload.blind_spots),
            suggested_falsification_metrics: normalize_items(
                payload.suggested_falsification_metrics,
            ),
        };
    }

    parse_fallback_text_critique(trimmed)
}

/// Runs a single lightweight Feynman critique turn using Alan if a default
/// Alan connection profile is configured.
pub async fn evaluate_feynman_round_with_alan(
    paths: &WorkspacePaths,
    prompt: String,
    lang: UiLang,
) -> Result<(ParsedFeynmanCritique, Option<String>)> {
    anyhow::ensure!(
        !prompt.trim().is_empty(),
        "{}",
        lang.t(
            "费曼评估提示词不能为空。",
            "Feynman critique prompt cannot be empty."
        )
    );

    match alan_connection::list_alan_connection_status() {
        Ok(status) if status.active_profile().is_none() => {
            anyhow::bail!(
                "{}",
                lang.t(
                    "尚未配置默认 Alan 模型连接，无法执行费曼点评（可运行 `alan connection add ...` 或在设置页配置）。",
                    "No default Alan connection profile is configured; cannot run Feynman critique (run `alan connection add ...` or configure in Settings)."
                )
            );
        }
        Ok(_) => {}
        Err(err) => {
            anyhow::bail!(
                "{}",
                alan_interrogator::friendly_m3_credentials_error(&format!("{err:#}"))
            );
        }
    }

    evaluate_feynman_round_with_llm(paths, prompt, LlmSource::SharedProductCredentials, lang).await
}

/// Internal/testable runner executing a single lightweight Feynman critique turn with an explicit [`LlmSource`].
pub async fn evaluate_feynman_round_with_llm(
    paths: &WorkspacePaths,
    prompt: String,
    llm: LlmSource,
    lang: UiLang,
) -> Result<(ParsedFeynmanCritique, Option<String>)> {
    anyhow::ensure!(
        !prompt.trim().is_empty(),
        "{}",
        lang.t(
            "费曼评估提示词不能为空。",
            "Feynman critique prompt cannot be empty."
        )
    );

    let fs = Arc::new(M3WorkbenchFs::new(paths.clone()));
    let app = alan_interrogator::boot_m3_embedded(paths.clone(), fs, llm)
        .await
        .map_err(|err| {
            anyhow::anyhow!(
                "{}",
                alan_interrogator::friendly_m3_credentials_error(&format!("{err:#}"))
            )
        })?;

    let client = app.root_agent();
    let mut stream = match client.event_stream().await {
        Ok(stream) => stream,
        Err(err) => {
            let _ = app.shutdown().await;
            return Err(err).context("打开 Alan 费曼审讯事件流失败");
        }
    };

    let submission_id = match client.submit_text(prompt).await {
        Ok(id) => id,
        Err(err) => {
            let _ = app.shutdown().await;
            return Err(anyhow::anyhow!(
                "{}",
                alan_interrogator::friendly_m3_credentials_error(&format!("{err:#}"))
            ));
        }
    };

    let mut content = String::new();
    let deadline =
        tokio::time::Instant::now() + Duration::from_secs(DEFAULT_FEYNMAN_EVAL_TIMEOUT_SECS);
    let loop_res: Result<()> = loop {
        let ev = match tokio::time::timeout_at(deadline, stream.next_event()).await {
            Ok(Ok(ev)) => ev,
            Ok(Err(err)) => break Err(err).context("Alan 费曼事件流中断"),
            Err(_) => {
                break Err(anyhow::anyhow!(
                    "{}",
                    lang.t("Alan 费曼点评超时。", "Alan Feynman critique timed out.")
                ));
            }
        };

        match ev {
            EmbedEvent::TextDelta { chunk, .. } => {
                content.push_str(&chunk);
            }
            EmbedEvent::PendingRequest { request_id, .. } => {
                let _ = client
                    .respond_request(&request_id, "denied: non-interactive feynman critique")
                    .await;
            }
            EmbedEvent::Error {
                message,
                recoverable,
            } => {
                if !recoverable {
                    break Err(anyhow::anyhow!(
                        "{}",
                        alan_interrogator::friendly_m3_credentials_error(&message)
                    ));
                }
            }
            EmbedEvent::TurnFinished {
                submission_id: sid,
                status,
                error,
            } if sid == submission_id => {
                if status != UiInputStatus::Completed {
                    let msg = error.unwrap_or_else(|| format!("状态 {status:?}"));
                    break Err(anyhow::anyhow!(
                        "{}",
                        alan_interrogator::friendly_m3_credentials_error(&msg)
                    ));
                }
                break Ok(());
            }
            _ => {}
        }
    };

    let _ = app.shutdown().await;
    loop_res?;

    let trimmed = content.trim().to_owned();
    let parsed = parse_feynman_critique(&trimmed);
    let raw_opt = (!trimmed.is_empty()).then_some(trimmed);
    Ok((parsed, raw_opt))
}

/// Returns the markdown archive path for a Feynman Defense session.
///
/// - Company: `workbench/memos/<TICKER>-feynman.md`
/// - Industry: `workbench/industries/<slug>/feynman-defense.md`
pub fn defense_archive_path(paths: &WorkspacePaths, scope: &FeynmanScope) -> PathBuf {
    match scope {
        FeynmanScope::Company { ticker } => {
            let upper = ticker.trim().to_ascii_uppercase();
            let safe = market::safe_symbol_filename(&upper);
            paths
                .workbench_dir
                .join("memos")
                .join(format!("{safe}-feynman.md"))
        }
        FeynmanScope::Industry { slug } => {
            let safe = market::safe_symbol_filename(slug.trim());
            paths.industries_dir().join(safe).join("feynman-defense.md")
        }
    }
}

/// Appends a completed Feynman defense summary atomically to its markdown archive.
pub fn append_defense_summary(
    paths: &WorkspacePaths,
    summary: &FeynmanDefenseSummary,
) -> Result<PathBuf> {
    validate_scope(&summary.scope)?;
    let path = defense_archive_path(paths, &summary.scope);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("创建费曼归档目录失败 {}", parent.display()))?;
    }

    let existing = match fs::read_to_string(&path) {
        Ok(text) => text,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(err) => {
            return Err(err).with_context(|| format!("读取费曼归档失败 {}", path.display()));
        }
    };

    let section = format_defense_markdown(summary);
    let mut combined = existing.trim_end().to_owned();
    if !combined.is_empty() {
        combined.push_str("\n\n---\n\n");
    }
    combined.push_str(&section);
    if !combined.ends_with('\n') {
        combined.push('\n');
    }

    let tmp = store::atomic_temp_path(&path);
    if let Err(err) = fs::write(&tmp, combined.as_bytes()).and_then(|_| fs::rename(&tmp, &path)) {
        let _ = fs::remove_file(&tmp);
        return Err(err).with_context(|| format!("写入费曼归档失败 {}", path.display()));
    }
    Ok(path)
}

/// Loads the markdown Feynman defense archive for a scope, or returns an empty string if absent.
pub fn load_defense_history(paths: &WorkspacePaths, scope: &FeynmanScope) -> Result<String> {
    validate_scope(scope)?;
    let path = defense_archive_path(paths, scope);
    match fs::read_to_string(&path) {
        Ok(text) => Ok(text),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(err) => Err(err).with_context(|| format!("读取费曼归档失败 {}", path.display())),
    }
}

fn validate_scope(scope: &FeynmanScope) -> Result<()> {
    let raw = match scope {
        FeynmanScope::Company { ticker } => ticker.trim(),
        FeynmanScope::Industry { slug } => slug.trim(),
    };
    anyhow::ensure!(
        !raw.is_empty() && raw != "." && raw != ".." && !raw.contains('/') && !raw.contains('\\'),
        "无效的费曼答辩作用域标识: {raw:?}"
    );
    Ok(())
}

fn format_defense_markdown(summary: &FeynmanDefenseSummary) -> String {
    let scope_label = summary.scope.identifier();
    let completed = if summary.completed_at.trim().is_empty() {
        store::civil_today_iso()
    } else {
        summary.completed_at.trim().to_owned()
    };
    let mut out = String::new();
    out.push_str(&format!(
        "## Feynman Defense · {scope_label} ({completed})\n\n"
    ));

    out.push_str("### Human Stance\n\n");
    let stance = summary.my_stance.trim();
    if stance.is_empty() {
        out.push_str("_No stance recorded._\n\n");
    } else {
        out.push_str(stance);
        out.push_str("\n\n");
    }

    out.push_str("### Falsification Metrics\n\n");
    let metrics = normalize_items(summary.falsification_metrics.clone());
    if metrics.is_empty() {
        out.push_str("- _None recorded._\n\n");
    } else {
        for m in metrics {
            out.push_str(&format!("- {m}\n"));
        }
        out.push('\n');
    }

    let blind_spots = normalize_items(summary.blind_spots.clone());
    if !blind_spots.is_empty() {
        out.push_str("### Blind Spots Flagged\n\n");
        for b in blind_spots {
            out.push_str(&format!("- {b}\n"));
        }
        out.push('\n');
    }

    if !summary.turns.is_empty() {
        out.push_str("### Defense Rounds\n\n");
        for turn in &summary.turns {
            out.push_str(&format!("#### Round {}\n\n", turn.round));
            if !turn.prompt.trim().is_empty() {
                out.push_str(&format!("**Question:** {}\n\n", turn.prompt.trim()));
            }
            if !turn.human_answer.trim().is_empty() {
                out.push_str(&format!(
                    "**Human Answer:** {}\n\n",
                    turn.human_answer.trim()
                ));
            }
            if !turn.alan_critique.trim().is_empty() {
                out.push_str(&format!(
                    "**Alan Critique:** {}\n\n",
                    turn.alan_critique.trim()
                ));
            }
            let turn_spots = normalize_items(turn.blind_spots.clone());
            if !turn_spots.is_empty() {
                out.push_str("**Blind Spots:**\n");
                for s in turn_spots {
                    out.push_str(&format!("- {s}\n"));
                }
                out.push('\n');
            }
            let turn_metrics = normalize_items(turn.suggested_metrics.clone());
            if !turn_metrics.is_empty() {
                out.push_str("**Suggested Falsification Metrics:**\n");
                for m in turn_metrics {
                    out.push_str(&format!("- {m}\n"));
                }
                out.push('\n');
            }
        }
    }

    out.trim_end().to_owned()
}

fn extract_critique_json(raw: &str) -> Option<(RawCritiquePayload, String)> {
    // 1. Prefer fenced code blocks (```json ... ``` or ``` ... ```).
    let mut search_from = 0usize;
    while let Some(rel_start) = raw[search_from..].find("```") {
        let fence_start = search_from + rel_start;
        let after_ticks = &raw[fence_start + 3..];
        let Some(newline_rel) = after_ticks.find('\n') else {
            break;
        };
        let lang_tag = after_ticks[..newline_rel].trim().to_ascii_lowercase();
        let body_start = fence_start + 3 + newline_rel + 1;
        let Some(end_rel) = raw[body_start..].find("```") else {
            break;
        };
        let body_end = body_start + end_rel;
        let block = raw[body_start..body_end].trim();
        let fence_end = body_end + 3;
        if (lang_tag.is_empty() || lang_tag == "json")
            && let Some(payload) = parse_json_payload_str(block)
        {
            let before = raw[..fence_start].trim();
            let after = raw[fence_end..].trim();
            let remainder = join_non_empty(before, after);
            return Some((payload, remainder));
        }
        search_from = fence_end;
    }

    // 2. Fallback: scan balanced `{ ... }` objects anywhere in the response.
    let mut idx = 0usize;
    while let Some(rel_open) = raw[idx..].find('{') {
        let start = idx + rel_open;
        if let Some(end) = find_matching_json_object_end(raw, start) {
            let candidate = &raw[start..=end];
            if let Some(payload) = parse_json_payload_str(candidate) {
                let before = raw[..start].trim();
                let after = raw[end + 1..].trim();
                let remainder = join_non_empty(before, after);
                return Some((payload, remainder));
            }
            idx = end + 1;
        } else {
            break;
        }
    }

    None
}

fn parse_json_payload_str(candidate: &str) -> Option<RawCritiquePayload> {
    let trimmed = candidate.trim();
    if !trimmed.starts_with('{') || !trimmed.ends_with('}') {
        return None;
    }
    let parsed: RawCritiquePayload = serde_json::from_str(trimmed).ok()?;
    if parsed.critique.trim().is_empty()
        && parsed.blind_spots.is_empty()
        && parsed.suggested_falsification_metrics.is_empty()
    {
        return None;
    }
    Some(parsed)
}

fn find_matching_json_object_end(s: &str, start: usize) -> Option<usize> {
    let mut depth = 0usize;
    let mut in_string = false;
    let mut escaped = false;
    for (rel_idx, ch) in s[start..].char_indices() {
        if in_string {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                in_string = false;
            }
            continue;
        }
        match ch {
            '"' => in_string = true,
            '{' => depth += 1,
            '}' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return Some(start + rel_idx);
                }
            }
            _ => {}
        }
    }
    None
}

fn join_non_empty(a: &str, b: &str) -> String {
    match (a.is_empty(), b.is_empty()) {
        (true, true) => String::new(),
        (false, true) => a.to_owned(),
        (true, false) => b.to_owned(),
        (false, false) => format!("{a}\n\n{b}"),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TextSection {
    Critique,
    BlindSpots,
    Metrics,
}

fn parse_fallback_text_critique(raw: &str) -> ParsedFeynmanCritique {
    let mut section = TextSection::Critique;
    let mut critique_lines: Vec<String> = Vec::new();
    let mut blind_spots: Vec<String> = Vec::new();
    let mut metrics: Vec<String> = Vec::new();

    for line in raw.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            if section == TextSection::Critique && !critique_lines.is_empty() {
                critique_lines.push(String::new());
            }
            continue;
        }

        if let Some((next_section, inline_rest)) = detect_section_heading(trimmed) {
            section = next_section;
            if let Some(rest) = inline_rest {
                match section {
                    TextSection::Critique => critique_lines.push(rest),
                    TextSection::BlindSpots => {
                        if let Some(item) = strip_bullet_prefix(&rest) {
                            blind_spots.push(item);
                        } else {
                            blind_spots.push(rest);
                        }
                    }
                    TextSection::Metrics => {
                        if let Some(item) = strip_bullet_prefix(&rest) {
                            metrics.push(item);
                        } else {
                            metrics.push(rest);
                        }
                    }
                }
            }
            continue;
        }

        match section {
            TextSection::Critique => critique_lines.push(trimmed.to_owned()),
            TextSection::BlindSpots => {
                if let Some(item) = strip_bullet_prefix(trimmed) {
                    blind_spots.push(item);
                } else {
                    blind_spots.push(trimmed.to_owned());
                }
            }
            TextSection::Metrics => {
                if let Some(item) = strip_bullet_prefix(trimmed) {
                    metrics.push(item);
                } else {
                    metrics.push(trimmed.to_owned());
                }
            }
        }
    }

    let critique = critique_lines.join("\n").trim().to_owned();
    let blind_spots = normalize_items(blind_spots);
    let suggested_falsification_metrics = normalize_items(metrics);

    if critique.is_empty() && blind_spots.is_empty() && suggested_falsification_metrics.is_empty() {
        return ParsedFeynmanCritique {
            critique: raw.trim().to_owned(),
            blind_spots: Vec::new(),
            suggested_falsification_metrics: Vec::new(),
        };
    }

    ParsedFeynmanCritique {
        critique,
        blind_spots,
        suggested_falsification_metrics,
    }
}

fn detect_section_heading(line: &str) -> Option<(TextSection, Option<String>)> {
    let cleaned = line.trim_start_matches('#').trim().trim_matches('*').trim();
    let (head, rest) = match cleaned.split_once([':', '：']) {
        Some((h, r)) => (h.trim().trim_matches('*').trim(), Some(r.trim().to_owned())),
        None => (cleaned, None),
    };
    let lower = head.to_ascii_lowercase();

    let sec = if lower.contains("blind spot")
        || lower.contains("blind_spot")
        || head.contains("盲区")
        || head.contains("盲点")
        || head.contains("逻辑漏洞")
    {
        TextSection::BlindSpots
    } else if lower.contains("falsification")
        || lower.contains("suggested_metrics")
        || head.contains("证伪指标")
        || head.contains("证伪条件")
    {
        TextSection::Metrics
    } else if lower == "critique" || head == "点评" || head == "逻辑点评" {
        TextSection::Critique
    } else {
        return None;
    };

    let rest = rest.filter(|s| !s.is_empty());
    Some((sec, rest))
}

fn strip_bullet_prefix(line: &str) -> Option<String> {
    let trimmed = line.trim();
    for prefix in ["- ", "* ", "• ", "· "] {
        if let Some(rest) = trimmed.strip_prefix(prefix) {
            let item = rest.trim();
            return (!item.is_empty()).then(|| item.to_owned());
        }
    }
    let digits_len = trimmed.bytes().take_while(|b| b.is_ascii_digit()).count();
    if digits_len > 0 {
        let after_digits = &trimmed[digits_len..];
        if let Some(rest) = after_digits
            .strip_prefix(". ")
            .or_else(|| after_digits.strip_prefix(") "))
            .or_else(|| after_digits.strip_prefix("、"))
        {
            let item = rest.trim();
            return (!item.is_empty()).then(|| item.to_owned());
        }
    }
    None
}

fn normalize_items(items: Vec<String>) -> Vec<String> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    for item in items {
        let trimmed = item.trim();
        if !trimmed.is_empty() && seen.insert(trimmed.to_owned()) {
            out.push(trimmed.to_owned());
        }
    }
    out
}

fn truncate_chars(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        return s.to_owned();
    }
    let mut out: String = s.chars().take(max_chars).collect();
    out.push('…');
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::industry::IndustryStatus;
    use crate::model::{HumanJudgment, Latticework};
    use alan_llm::{GenerationResponse, MockLlmProvider, TokenUsage};
    use alan_os_host::embed::LlmClient;
    use std::sync::atomic::{AtomicU64, Ordering};

    static TEST_COUNTER: AtomicU64 = AtomicU64::new(0);

    fn temp_workspace(label: &str) -> WorkspacePaths {
        let seq = TEST_COUNTER.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "m3_feynman_test_{label}_{}_{}",
            std::process::id(),
            seq
        ));
        let _ = fs::remove_dir_all(&root);
        let paths = WorkspacePaths::from_root(root);
        fs::create_dir_all(&paths.workbench_dir).unwrap();
        fs::create_dir_all(&paths.data_dir).unwrap();
        paths
    }

    fn sample_dossier(pending: bool, checked_at: Option<&str>) -> Dossier {
        Dossier {
            schema_version: 1,
            ticker: "HRMY".into(),
            name: "Harmony Biosciences".into(),
            updated_at: "2026-11-26".into(),
            draft_source: "test".into(),
            three_sentence_business: "Narcolepsy specialty pharma.".into(),
            latticework: Latticework {
                microeconomics: "Orphan drug exclusivity".into(),
                ecology: "Narrow rare-disease niche".into(),
                psychology_incentives: "Focused commercial team".into(),
                capital_allocation: "Debt paydown and pipeline".into(),
            },
            premortem: vec!["LOE cliff in 2030".into()],
            open_questions: vec!["Next-gen pitolisant uptake?".into()],
            human_judgment: HumanJudgment {
                pending_human_review: pending,
                my_stance: None,
                weight_rationale: None,
                falsification_metrics: Vec::new(),
                feynman_checked_at: checked_at.map(str::to_owned),
            },
        }
    }

    #[test]
    fn round_enum_conversions_and_titles_work() {
        assert_eq!(FeynmanRound::MoatAndEconomics.as_u8(), 1);
        assert_eq!(FeynmanRound::FailureModesAndFalsification.as_u8(), 2);
        assert_eq!(FeynmanRound::ValuationAndDiscipline.as_u8(), 3);
        assert_eq!(
            FeynmanRound::from_u8(1),
            Some(FeynmanRound::MoatAndEconomics)
        );
        assert_eq!(
            FeynmanRound::from_u8(2),
            Some(FeynmanRound::FailureModesAndFalsification)
        );
        assert_eq!(
            FeynmanRound::from_u8(3),
            Some(FeynmanRound::ValuationAndDiscipline)
        );
        assert_eq!(FeynmanRound::from_u8(0), None);
        assert_eq!(FeynmanRound::from_u8(4), None);

        assert!(
            FeynmanRound::MoatAndEconomics
                .title_for(UiLang::Zh)
                .contains("护城河")
        );
        assert!(
            FeynmanRound::ValuationAndDiscipline
                .title_for(UiLang::En)
                .contains("Margin of Safety")
        );
    }

    #[test]
    fn graduation_gate_checks_missing_pending_and_timestamp() {
        assert!(needs_feynman_graduation_gate(None));
        assert!(needs_feynman_graduation_gate(Some(&sample_dossier(
            true,
            Some("2026-11-26")
        ))));
        assert!(needs_feynman_graduation_gate(Some(&sample_dossier(
            false, None
        ))));
        assert!(needs_feynman_graduation_gate(Some(&sample_dossier(
            false,
            Some("   ")
        ))));
        assert!(!needs_feynman_graduation_gate(Some(&sample_dossier(
            false,
            Some("2026-11-26")
        ))));
    }

    #[test]
    fn round_prompts_and_critique_composers_include_context() {
        let comp_scope = FeynmanScope::Company {
            ticker: "cost".into(),
        };
        let q1 = round_question_prompt(
            &comp_scope,
            FeynmanRound::MoatAndEconomics,
            "Costco",
            UiLang::En,
        );
        assert!(q1.contains("Costco"));
        assert!(q1.contains("ROIC"));

        let ind_scope = FeynmanScope::Industry {
            slug: "eda-ip".into(),
        };
        let q2 = round_question_prompt(
            &ind_scope,
            FeynmanRound::FailureModesAndFalsification,
            "",
            UiLang::Zh,
        );
        assert!(q2.contains("eda-ip"));
        assert!(q2.contains("死法"));

        let comp_prompt = compose_company_feynman_critique_prompt(
            "cost",
            "ROIC 28%, FCF yield 3.1%",
            FeynmanRound::ValuationAndDiscipline,
            "What is your strike?",
            "18x owner earnings",
            &["Membership renewal < 90%".into()],
            UiLang::En,
        );
        assert!(comp_prompt.contains("COST"));
        assert!(comp_prompt.contains("Membership renewal < 90%"));
        assert!(comp_prompt.contains("suggested_falsification_metrics"));

        let profile = IndustryProfile {
            slug: "eda-ip".into(),
            title: "EDA & IP".into(),
            status: IndustryStatus::Studying,
            sic_codes: vec![7372],
            keywords: vec!["EDA".into()],
            bound_tickers: vec!["SNPS".into(), "CDNS".into()],
            multiple_low: None,
            multiple_high: None,
            key_kpis: vec!["R&D / Revenue".into()],
            feynman_questions: vec![],
            feynman_checked: vec![],
            human_notes: String::new(),
            created_at_ms: 0,
            updated_at_ms: 0,
            last_research_run_id: None,
        };
        let ind_prompt = compose_industry_feynman_critique_prompt(
            &profile,
            "Duopoly with high switching costs.",
            FeynmanRound::MoatAndEconomics,
            "Where is the profit pool?",
            "Core signoff tools.",
            UiLang::Zh,
        );
        assert!(ind_prompt.contains("SNPS, CDNS"));
        assert!(ind_prompt.contains("suggested_falsification_metrics"));
    }

    #[test]
    fn parse_feynman_critique_handles_fenced_inline_and_bullet_fallbacks() {
        let fenced = r#"
Here is my review:
```json
{
  "critique": "Good moat intuition, but pricing power is asserted without cohort proof.",
  "blind_spots": ["Ignores generic entry", "Ignores generic entry", "Single-product concentration"],
  "suggested_falsification_metrics": ["Gross margin < 75%", "Net cash turns negative"]
}
```
Please sharpen round 2.
"#;
        let parsed = parse_feynman_critique(fenced);
        assert!(parsed.critique.contains("cohort proof"));
        assert_eq!(
            parsed.blind_spots,
            vec!["Ignores generic entry", "Single-product concentration"]
        );
        assert_eq!(
            parsed.suggested_falsification_metrics,
            vec!["Gross margin < 75%", "Net cash turns negative"]
        );

        let inline = r#"Alan note: {"critique": "Tight valuation discipline.", "blind_spots": ["FX headwind"], "suggested_metrics": ["ROIC < 15%"]} Done."#;
        let parsed_inline = parse_feynman_critique(inline);
        assert_eq!(parsed_inline.critique, "Tight valuation discipline.");
        assert_eq!(parsed_inline.blind_spots, vec!["FX headwind"]);
        assert_eq!(
            parsed_inline.suggested_falsification_metrics,
            vec!["ROIC < 15%"]
        );

        let fallback = "\
逻辑点评：护城河描述偏笼统，缺少对替代品的定量反驳。\n\
## 盲点\n\
- 未解释客户自研边界\n\
- 忽略行业资本开支周期\n\
## 建议证伪指标\n\
1. 连续两个季度毛利率低于 60%\n\
2. SBC / OCF > 25%\n";
        let parsed_fb = parse_feynman_critique(fallback);
        assert!(parsed_fb.critique.contains("护城河描述偏笼统"));
        assert_eq!(
            parsed_fb.blind_spots,
            vec!["未解释客户自研边界", "忽略行业资本开支周期"]
        );
        assert_eq!(
            parsed_fb.suggested_falsification_metrics,
            vec!["连续两个季度毛利率低于 60%", "SBC / OCF > 25%"]
        );
    }

    #[test]
    fn defense_archive_paths_and_append_load_roundtrip() {
        let paths = temp_workspace("archive");
        let comp_scope = FeynmanScope::Company {
            ticker: "hrmy".into(),
        };
        let comp_path = defense_archive_path(&paths, &comp_scope);
        assert!(comp_path.ends_with("workbench/memos/HRMY-feynman.md"));
        assert_eq!(load_defense_history(&paths, &comp_scope).unwrap(), "");

        let summary_1 = FeynmanDefenseSummary {
            scope: comp_scope.clone(),
            completed_at: "2026-11-25".into(),
            my_stance: "Hold watchlist until patent clarity".into(),
            falsification_metrics: vec!["WAKIX Rx growth < 5%".into()],
            blind_spots: vec!["Single-asset concentration".into()],
            turns: vec![FeynmanTurn {
                round: 1,
                prompt: "What is the moat?".into(),
                human_answer: "Orphan specialty distribution.".into(),
                alan_critique: "Check patent cliffs.".into(),
                blind_spots: vec!["LOE in 2030".into()],
                suggested_metrics: vec!["WAKIX Rx growth < 5%".into()],
            }],
        };
        append_defense_summary(&paths, &summary_1).unwrap();

        let summary_2 = FeynmanDefenseSummary {
            scope: comp_scope.clone(),
            completed_at: "2026-11-26".into(),
            my_stance: "Graduate to Core at 14x OE".into(),
            falsification_metrics: vec!["Operating margin < 25%".into()],
            blind_spots: vec![],
            turns: vec![],
        };
        append_defense_summary(&paths, &summary_2).unwrap();

        let history = load_defense_history(&paths, &comp_scope).unwrap();
        assert!(history.contains("2026-11-25"));
        assert!(history.contains("2026-11-26"));
        assert!(history.contains("Graduate to Core at 14x OE"));
        assert!(history.contains("---"));

        let ind_scope = FeynmanScope::Industry {
            slug: "eda-ip".into(),
        };
        let ind_path = defense_archive_path(&paths, &ind_scope);
        assert!(ind_path.ends_with("workbench/industries/eda-ip/feynman-defense.md"));
        append_defense_summary(
            &paths,
            &FeynmanDefenseSummary {
                scope: ind_scope.clone(),
                completed_at: "2026-11-26".into(),
                my_stance: "Inside Circle: duopoly pricing power".into(),
                falsification_metrics: vec!["Top-2 share < 65%".into()],
                blind_spots: vec![],
                turns: vec![],
            },
        )
        .unwrap();
        let ind_history = load_defense_history(&paths, &ind_scope).unwrap();
        assert!(ind_history.contains("Inside Circle: duopoly pricing power"));

        let _ = fs::remove_dir_all(&paths.repo_root);
    }

    #[test]
    fn evaluate_feynman_round_with_mock_llm_parses_structured_turn() {
        let paths = temp_workspace("mock_llm");
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let provider = MockLlmProvider::new().with_responses(vec![GenerationResponse {
            content: r#"```json
{
  "critique": "Strong grasp of switching costs, but capex intensity needs verification.",
  "blind_spots": ["Assumes zero pricing pushback"],
  "suggested_falsification_metrics": ["ROIC drops below 18%"]
}
```"#
                .to_string(),
            thinking: None,
            thinking_signature: None,
            redacted_thinking: Vec::new(),
            tool_calls: Vec::new(),
            usage: Some(TokenUsage {
                prompt_tokens: 120,
                cached_prompt_tokens: None,
                completion_tokens: 45,
                total_tokens: 165,
                reasoning_tokens: None,
                ..Default::default()
            }),
            finish_reason: None,
            warnings: Vec::new(),
            provider_response_id: None,
            provider_response_status: None,
        }]);

        let (parsed, raw) = rt
            .block_on(evaluate_feynman_round_with_llm(
                &paths,
                "Evaluate my answer".to_owned(),
                LlmSource::Client(LlmClient::new(provider)),
                UiLang::En,
            ))
            .unwrap();

        assert!(parsed.critique.contains("switching costs"));
        assert_eq!(parsed.blind_spots, vec!["Assumes zero pricing pushback"]);
        assert_eq!(
            parsed.suggested_falsification_metrics,
            vec!["ROIC drops below 18%"]
        );
        assert!(raw.is_some());

        let _ = fs::remove_dir_all(&paths.repo_root);
    }
}
