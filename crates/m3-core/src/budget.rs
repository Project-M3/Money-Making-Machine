//! L2 AI budget configuration, hard cost/token gate, and cross-company Theme clustering.
//!
//! - `workbench/budget.json`: monthly & per-run USD/token hard ceilings and auto-trigger quota.
//! - `workbench/themes.json`: SIC + latent-space business-archetype clusters derived deterministically
//!   from `universe_snapshot` and `doc_chunks`, preserving human-edited `user_notes`.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::PathBuf;

use anyhow::Result;
use duckdb::params;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use crate::lake::LakeDb;
use crate::latent::{cosine_similarity, decode_embedding_json, l2_normalize};
use crate::model::{serde_decimal, serde_decimal_opt};
use crate::store::{self, WorkspacePaths};

#[allow(dead_code)]
pub const BUDGET_SCHEMA_VERSION: u32 = 1;
#[allow(dead_code)]
pub const THEMES_SCHEMA_VERSION: u32 = 1;

fn default_schema_version() -> u32 {
    BUDGET_SCHEMA_VERSION
}

fn default_themes_schema_version() -> u32 {
    THEMES_SCHEMA_VERSION
}

fn default_monthly_budget_usd() -> Decimal {
    Decimal::new(1000, 2) // $10.00
}

fn default_per_run_budget_usd() -> Decimal {
    Decimal::new(75, 2) // $0.75
}

fn default_monthly_token_limit() -> u64 {
    2_000_000
}

fn default_per_run_token_limit() -> u64 {
    120_000
}

fn default_input_usd_per_1m_tokens() -> Decimal {
    Decimal::new(300, 2) // $3.00 / 1M input tokens
}

fn default_output_usd_per_1m_tokens() -> Decimal {
    Decimal::new(1500, 2) // $15.00 / 1M output tokens
}

/// Persisted L2 Deep Research budget & auto-trigger limits (`workbench/budget.json`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BudgetConfig {
    #[serde(default = "default_schema_version")]
    pub schema_version: u32,
    #[serde(default = "default_monthly_budget_usd", with = "serde_decimal")]
    pub monthly_budget_usd: Decimal,
    #[serde(default = "default_per_run_budget_usd", with = "serde_decimal")]
    pub per_run_budget_usd: Decimal,
    #[serde(default = "default_monthly_token_limit")]
    pub monthly_token_limit: u64,
    #[serde(default = "default_per_run_token_limit")]
    pub per_run_token_limit: u64,
    #[serde(default)]
    pub auto_trigger_enabled: bool,
    #[serde(default)]
    pub weekly_auto_quota: usize,
    #[serde(default = "default_input_usd_per_1m_tokens", with = "serde_decimal")]
    pub input_usd_per_1m_tokens: Decimal,
    #[serde(default = "default_output_usd_per_1m_tokens", with = "serde_decimal")]
    pub output_usd_per_1m_tokens: Decimal,
}

impl Default for BudgetConfig {
    fn default() -> Self {
        Self {
            schema_version: BUDGET_SCHEMA_VERSION,
            monthly_budget_usd: default_monthly_budget_usd(),
            per_run_budget_usd: default_per_run_budget_usd(),
            monthly_token_limit: default_monthly_token_limit(),
            per_run_token_limit: default_per_run_token_limit(),
            auto_trigger_enabled: false,
            weekly_auto_quota: 0,
            input_usd_per_1m_tokens: default_input_usd_per_1m_tokens(),
            output_usd_per_1m_tokens: default_output_usd_per_1m_tokens(),
        }
    }
}

/// Result of pre-flight hard budget gating before booting `EmbeddedAlan` or invoking an LLM.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BudgetDecision {
    pub allowed: bool,
    pub reason: Option<String>,
    #[serde(with = "serde_decimal")]
    pub remaining_monthly_usd: Decimal,
    pub remaining_monthly_tokens: u64,
}

/// Canonical path to `workbench/budget.json`.
pub fn budget_path(paths: &WorkspacePaths) -> PathBuf {
    paths.workbench_dir.join("budget.json")
}

/// Loads `workbench/budget.json`, returning conservative defaults when the file is absent.
pub fn load_budget_config(paths: &WorkspacePaths) -> Result<BudgetConfig> {
    let path = budget_path(paths);
    if !path.exists() {
        return Ok(BudgetConfig::default());
    }
    store::read_json(&path)
}

/// Atomically persists `workbench/budget.json`.
#[cfg_attr(not(test), allow(dead_code))]
pub fn save_budget_config(paths: &WorkspacePaths, cfg: &BudgetConfig) -> Result<()> {
    store::write_json_atomic(&budget_path(paths), cfg)
}

/// Estimates USD cost from input/output token counts using strict `Decimal` arithmetic.
pub fn estimate_cost_usd(cfg: &BudgetConfig, input_tokens: u64, output_tokens: u64) -> Decimal {
    let one_million = Decimal::from(1_000_000u64);
    let in_rate = cfg.input_usd_per_1m_tokens.max(Decimal::ZERO);
    let out_rate = cfg.output_usd_per_1m_tokens.max(Decimal::ZERO);

    let in_cost = Decimal::from(input_tokens)
        .checked_mul(in_rate)
        .map_or(Decimal::MAX, |v| v / one_million);
    let out_cost = Decimal::from(output_tokens)
        .checked_mul(out_rate)
        .map_or(Decimal::MAX, |v| v / one_million);

    in_cost
        .checked_add(out_cost)
        .unwrap_or(Decimal::MAX)
        .round_dp(6)
        .normalize()
}

/// Evaluates whether a Deep Research run may proceed under monthly, per-run, and auto quotas.
pub fn evaluate_budget_gate(
    cfg: &BudgetConfig,
    monthly_tokens: u64,
    monthly_usd: Decimal,
    weekly_auto_runs: usize,
    is_auto_trigger: bool,
) -> BudgetDecision {
    let remaining_monthly_usd = (cfg.monthly_budget_usd - monthly_usd)
        .max(Decimal::ZERO)
        .normalize();
    let remaining_monthly_tokens = cfg.monthly_token_limit.saturating_sub(monthly_tokens);

    if cfg.monthly_budget_usd <= Decimal::ZERO {
        return BudgetDecision {
            allowed: false,
            reason: Some("月度 AI 预算上限为 $0.00，已硬拦截".to_owned()),
            remaining_monthly_usd: Decimal::ZERO,
            remaining_monthly_tokens,
        };
    }

    if monthly_usd >= cfg.monthly_budget_usd {
        return BudgetDecision {
            allowed: false,
            reason: Some(format!(
                "本月 AI 预算已用尽（已用 ${} / 上限 ${}）",
                monthly_usd.round_dp(4).normalize(),
                cfg.monthly_budget_usd.round_dp(2).normalize()
            )),
            remaining_monthly_usd: Decimal::ZERO,
            remaining_monthly_tokens,
        };
    }

    if cfg.monthly_token_limit == 0 {
        return BudgetDecision {
            allowed: false,
            reason: Some("月度 Token 上限为 0，已硬拦截".to_owned()),
            remaining_monthly_usd,
            remaining_monthly_tokens: 0,
        };
    }

    if monthly_tokens >= cfg.monthly_token_limit {
        return BudgetDecision {
            allowed: false,
            reason: Some(format!(
                "本月 Token 配额已用尽（已用 {} / 上限 {}）",
                monthly_tokens, cfg.monthly_token_limit
            )),
            remaining_monthly_usd,
            remaining_monthly_tokens: 0,
        };
    }

    if cfg.per_run_budget_usd <= Decimal::ZERO {
        return BudgetDecision {
            allowed: false,
            reason: Some("单次 Deep Research 预算上限为 $0.00，已硬拦截".to_owned()),
            remaining_monthly_usd,
            remaining_monthly_tokens,
        };
    }

    if cfg.per_run_token_limit == 0 {
        return BudgetDecision {
            allowed: false,
            reason: Some("单次 Deep Research Token 上限为 0，已硬拦截".to_owned()),
            remaining_monthly_usd,
            remaining_monthly_tokens,
        };
    }

    if is_auto_trigger && !cfg.auto_trigger_enabled {
        return BudgetDecision {
            allowed: false,
            reason: Some(
                "自动触发 Deep Research 默认关闭（请在 budget.json 开启 auto_trigger_enabled）"
                    .to_owned(),
            ),
            remaining_monthly_usd,
            remaining_monthly_tokens,
        };
    }

    if is_auto_trigger && weekly_auto_runs >= cfg.weekly_auto_quota {
        return BudgetDecision {
            allowed: false,
            reason: Some(format!(
                "本周自动触发 Deep Research 配额已用尽（{} / {}）",
                weekly_auto_runs, cfg.weekly_auto_quota
            )),
            remaining_monthly_usd,
            remaining_monthly_tokens,
        };
    }

    BudgetDecision {
        allowed: true,
        reason: None,
        remaining_monthly_usd,
        remaining_monthly_tokens,
    }
}

/// One company member inside a [`ThemeCluster`].
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ThemeClusterMember {
    pub ticker: String,
    pub cik: String,
    pub name: String,
    #[serde(default)]
    pub sic: Option<u32>,
    #[serde(default)]
    pub sic_description: Option<String>,
    #[serde(default, with = "serde_decimal_opt")]
    pub roic_5y_avg_pct: Option<Decimal>,
    #[serde(default, with = "serde_decimal_opt")]
    pub distance_to_strike_pct: Option<Decimal>,
    #[serde(default)]
    pub in_watchlist: bool,
}

/// Cross-company business-archetype & SIC cluster (`workbench/themes.json`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ThemeCluster {
    pub theme_id: String,
    pub title: String,
    pub archetype: String,
    #[serde(default)]
    pub sic_codes: Vec<u32>,
    #[serde(default)]
    pub keywords: Vec<String>,
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub tickers: Vec<String>,
    #[serde(default)]
    pub members: Vec<ThemeClusterMember>,
    #[serde(default)]
    pub avg_similarity: f32,
    #[serde(default, with = "serde_decimal_opt")]
    pub median_roic_5y_pct: Option<Decimal>,
    #[serde(default)]
    pub strike_zone_count: usize,
    #[serde(default)]
    pub user_notes: String,
}

/// Persisted Themes knowledge graph (`workbench/themes.json`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ThemesFile {
    #[serde(default = "default_themes_schema_version")]
    pub schema_version: u32,
    #[serde(default)]
    pub updated_at_ms: u64,
    #[serde(default)]
    pub total_indexed_companies: usize,
    #[serde(default)]
    pub clusters: Vec<ThemeCluster>,
}

impl Default for ThemesFile {
    fn default() -> Self {
        Self {
            schema_version: THEMES_SCHEMA_VERSION,
            updated_at_ms: 0,
            total_indexed_companies: 0,
            clusters: Vec::new(),
        }
    }
}

/// Canonical path to `workbench/themes.json`.
pub fn themes_path(paths: &WorkspacePaths) -> PathBuf {
    paths.workbench_dir.join("themes.json")
}

/// Loads `workbench/themes.json`, returning an empty default if the file is absent.
pub fn load_themes(paths: &WorkspacePaths) -> Result<ThemesFile> {
    let path = themes_path(paths);
    if !path.exists() {
        return Ok(ThemesFile::default());
    }
    store::read_json(&path)
}

/// Atomically persists `workbench/themes.json`.
pub fn save_themes(paths: &WorkspacePaths, themes: &ThemesFile) -> Result<()> {
    store::write_json_atomic(&themes_path(paths), themes)
}

struct CompanyProfile {
    member: ThemeClusterMember,
    centroid: Vec<f32>,
    keywords: Vec<String>,
}

/// Derives deterministic SIC + latent-space business-archetype clusters from `universe_snapshot`
/// and `doc_chunks`, preserving any human-edited `user_notes` already saved in `workbench/themes.json`.
pub fn sync_themes_from_lake(paths: &WorkspacePaths, db: &LakeDb) -> Result<ThemesFile> {
    let prev_themes = if themes_path(paths).exists() {
        load_themes(paths).unwrap_or_default()
    } else {
        ThemesFile::default()
    };

    let mut notes_by_id: HashMap<String, String> = HashMap::new();
    let mut notes_by_tickers: Vec<(HashSet<String>, String)> = Vec::new();
    for c in &prev_themes.clusters {
        if !c.user_notes.trim().is_empty() {
            notes_by_id.insert(c.theme_id.clone(), c.user_notes.clone());
            let set: HashSet<String> = c
                .tickers
                .iter()
                .map(|t| t.trim().to_ascii_uppercase())
                .filter(|t| !t.is_empty())
                .collect();
            if !set.is_empty() {
                notes_by_tickers.push((set, c.user_notes.clone()));
            }
        }
    }

    let companies = load_candidate_companies_for_themes(db)?;
    let total_indexed_companies = companies.len();
    if companies.is_empty() {
        let out = ThemesFile {
            schema_version: THEMES_SCHEMA_VERSION,
            updated_at_ms: store::now_ms(),
            total_indexed_companies: 0,
            clusters: Vec::new(),
        };
        save_themes(paths, &out)?;
        return Ok(out);
    }

    let profiles = build_company_profiles(db, companies)?;
    let mut clusters = cluster_company_profiles(profiles);

    for cluster in &mut clusters {
        if let Some(note) = notes_by_id.get(&cluster.theme_id) {
            cluster.user_notes = note.clone();
            continue;
        }
        let cur_set: HashSet<String> = cluster.tickers.iter().cloned().collect();
        let mut best_overlap = 0usize;
        let mut best_note: Option<&String> = None;
        for (prev_set, note) in &notes_by_tickers {
            let inter = cur_set.intersection(prev_set).count();
            let min_sz = cur_set.len().min(prev_set.len()).max(1);
            if inter * 2 >= min_sz && inter > best_overlap {
                best_overlap = inter;
                best_note = Some(note);
            }
        }
        if let Some(note) = best_note {
            cluster.user_notes = note.clone();
        }
    }

    let out = ThemesFile {
        schema_version: THEMES_SCHEMA_VERSION,
        updated_at_ms: store::now_ms(),
        total_indexed_companies,
        clusters,
    };
    save_themes(paths, &out)?;
    Ok(out)
}

fn lake_table_exists(db: &LakeDb, table_name: &str) -> Result<bool> {
    let count: i64 = db.conn.query_row(
        "SELECT COUNT(*) FROM information_schema.tables WHERE table_name = ?",
        [table_name],
        |r| r.get(0),
    )?;
    Ok(count > 0)
}

fn load_candidate_companies_for_themes(db: &LakeDb) -> Result<Vec<ThemeClusterMember>> {
    if !lake_table_exists(db, "universe_snapshot")? {
        return Ok(Vec::new());
    }
    let has_pool: i64 = db.conn.query_row(
        "SELECT COUNT(*) FROM universe_snapshot
         WHERE is_financial = FALSE AND (in_price_pool = TRUE OR in_watchlist = TRUE)",
        [],
        |r| r.get(0),
    )?;

    let sql = if has_pool > 0 {
        "SELECT ticker, cik, name, sic, sic_description, roic_5y_avg_pct,
                distance_to_strike_pct, in_watchlist
         FROM universe_snapshot
         WHERE is_financial = FALSE AND (in_price_pool = TRUE OR in_watchlist = TRUE)
         ORDER BY in_watchlist DESC, quality_score DESC, ticker ASC
         LIMIT 500"
    } else {
        "SELECT ticker, cik, name, sic, sic_description, roic_5y_avg_pct,
                distance_to_strike_pct, in_watchlist
         FROM universe_snapshot
         WHERE is_financial = FALSE AND years_of_data > 0
         ORDER BY in_watchlist DESC, quality_score DESC, ticker ASC
         LIMIT 500"
    };

    let mut stmt = db.conn.prepare(sql)?;
    let rows = stmt.query_map([], |r| {
        let sic_i32: Option<i32> = r.get(3)?;
        Ok(ThemeClusterMember {
            ticker: r.get::<_, String>(0)?.trim().to_ascii_uppercase(),
            cik: r.get(1)?,
            name: r.get(2)?,
            sic: sic_i32.and_then(|v| u32::try_from(v).ok()),
            sic_description: r.get(4)?,
            roic_5y_avg_pct: r.get(5)?,
            distance_to_strike_pct: r.get(6)?,
            in_watchlist: r.get(7)?,
        })
    })?;

    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    Ok(out)
}

fn build_company_profiles(
    db: &LakeDb,
    companies: Vec<ThemeClusterMember>,
) -> Result<Vec<CompanyProfile>> {
    let has_chunks = lake_table_exists(db, "doc_chunks")?;
    let mut chunk_stmt = if has_chunks {
        Some(db.conn.prepare(
            "SELECT section, content, embedding_model, embedding_json
             FROM doc_chunks
             WHERE cik = ?
             ORDER BY
                CASE section
                    WHEN 'item_1' THEN 0
                    WHEN 'company_profile' THEN 1
                    WHEN 'item_7' THEN 2
                    WHEN 'item_1a' THEN 3
                    WHEN 'item_8k' THEN 4
                    ELSE 5
                END ASC,
                chunk_id ASC
             LIMIT 12",
        )?)
    } else {
        None
    };

    let mut profiles = Vec::with_capacity(companies.len());
    for member in companies {
        let mut centroid: Vec<f32> = Vec::new();
        let mut target_model: Option<String> = None;
        let mut text_parts: Vec<String> = Vec::new();

        if let Some(desc) = member.sic_description.as_deref() {
            text_parts.push(desc.to_owned());
        }

        if let Some(stmt) = chunk_stmt.as_mut() {
            let rows = stmt.query_map(params![member.cik.as_str()], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                ))
            })?;
            for row in rows {
                let (section, content, model, emb_json) = row?;
                if target_model.is_none() {
                    target_model = Some(model.clone());
                }
                if target_model.as_deref() == Some(model.as_str())
                    && let Some(vec) = decode_embedding_json(&emb_json)
                {
                    let weight = match section.as_str() {
                        "item_1" | "company_profile" => 1.5f32,
                        "item_7" => 1.25f32,
                        "item_1a" => 1.0f32,
                        _ => 0.8f32,
                    };
                    if centroid.is_empty() {
                        centroid = vec![0.0f32; vec.len()];
                    }
                    if centroid.len() == vec.len() {
                        for (acc, val) in centroid.iter_mut().zip(vec.iter()) {
                            *acc += weight * val;
                        }
                    }
                }
                if text_parts.len() < 8 && !content.trim().is_empty() {
                    text_parts.push(content);
                }
            }
        }

        if !centroid.is_empty() {
            l2_normalize(&mut centroid);
        }
        let keywords = extract_top_keywords(&text_parts.join(" "), 8);
        profiles.push(CompanyProfile {
            member,
            centroid,
            keywords,
        });
    }
    Ok(profiles)
}

struct MutableCluster {
    members: Vec<CompanyProfile>,
    centroid: Vec<f32>,
}

impl MutableCluster {
    fn recompute_centroid(&mut self) {
        let Some(dim) = self
            .members
            .iter()
            .find(|m| !m.centroid.is_empty())
            .map(|m| m.centroid.len())
        else {
            self.centroid.clear();
            return;
        };
        let mut acc = vec![0.0f32; dim];
        let mut any = false;
        for m in &self.members {
            if m.centroid.len() == dim {
                any = true;
                for (dst, src) in acc.iter_mut().zip(m.centroid.iter()) {
                    *dst += *src;
                }
            }
        }
        if any {
            l2_normalize(&mut acc);
            self.centroid = acc;
        } else {
            self.centroid.clear();
        }
    }

    fn dominant_sic_major(&self) -> Option<u32> {
        let mut counts: BTreeMap<u32, usize> = BTreeMap::new();
        for m in &self.members {
            if let Some(sic) = m.member.sic {
                *counts.entry(sic / 100).or_insert(0) += 1;
            }
        }
        counts
            .into_iter()
            .max_by(|(a_code, a_cnt), (b_code, b_cnt)| {
                a_cnt.cmp(b_cnt).then_with(|| b_code.cmp(a_code))
            })
            .map(|(code, _)| code)
    }

    fn has_exact_sic(&self, sic: u32) -> bool {
        self.members.iter().any(|m| m.member.sic == Some(sic))
    }
}

fn cluster_company_profiles(profiles: Vec<CompanyProfile>) -> Vec<ThemeCluster> {
    let mut buckets: Vec<MutableCluster> = Vec::new();

    for profile in profiles {
        let mut best_idx: Option<usize> = None;
        let mut best_score = 0.0f32;

        for (idx, cluster) in buckets.iter().enumerate() {
            let sim = if !profile.centroid.is_empty()
                && profile.centroid.len() == cluster.centroid.len()
            {
                cosine_similarity(&profile.centroid, &cluster.centroid).max(0.0)
            } else {
                0.0
            };

            let same_exact_sic = profile.member.sic.is_some_and(|s| cluster.has_exact_sic(s));
            let same_major_sic = profile
                .member
                .sic
                .zip(cluster.dominant_sic_major())
                .is_some_and(|(s, maj)| s / 100 == maj);

            let both_unembedded = profile.centroid.is_empty() && cluster.centroid.is_empty();
            let joins = if both_unembedded {
                same_exact_sic || same_major_sic
            } else if same_exact_sic {
                sim >= 0.20 || profile.centroid.is_empty() || cluster.centroid.is_empty()
            } else if same_major_sic {
                sim >= 0.34
            } else {
                sim >= 0.48
            };

            if joins {
                let effective = sim
                    + if same_exact_sic {
                        0.20
                    } else if same_major_sic {
                        0.08
                    } else {
                        0.0
                    };
                if effective > best_score {
                    best_score = effective;
                    best_idx = Some(idx);
                }
            }
        }

        if let Some(idx) = best_idx {
            buckets[idx].members.push(profile);
            buckets[idx].recompute_centroid();
        } else {
            let centroid = profile.centroid.clone();
            buckets.push(MutableCluster {
                members: vec![profile],
                centroid,
            });
        }
    }

    let has_multi = buckets.iter().any(|b| b.members.len() >= 2);
    let mut out: Vec<ThemeCluster> = buckets
        .into_iter()
        .filter(|b| {
            if has_multi {
                b.members.len() >= 2
            } else {
                !b.members.is_empty()
            }
        })
        .map(finalize_cluster)
        .collect();

    out.sort_by(|a, b| {
        b.members
            .len()
            .cmp(&a.members.len())
            .then_with(|| b.strike_zone_count.cmp(&a.strike_zone_count))
            .then_with(|| a.theme_id.cmp(&b.theme_id))
    });
    out.truncate(24);
    out
}

fn finalize_cluster(cluster: MutableCluster) -> ThemeCluster {
    let mut sic_counts: BTreeMap<u32, usize> = BTreeMap::new();
    let mut desc_counts: BTreeMap<String, usize> = BTreeMap::new();
    let mut kw_counts: BTreeMap<String, usize> = BTreeMap::new();
    let mut roics: Vec<Decimal> = Vec::new();
    let mut strike_zone_count = 0usize;

    for p in &cluster.members {
        if let Some(sic) = p.member.sic {
            *sic_counts.entry(sic).or_insert(0) += 1;
        }
        if let Some(desc) = p
            .member
            .sic_description
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            *desc_counts.entry(desc.to_owned()).or_insert(0) += 1;
        }
        for kw in &p.keywords {
            *kw_counts.entry(kw.clone()).or_insert(0) += 1;
        }
        if let Some(roic) = p.member.roic_5y_avg_pct {
            roics.push(roic);
        }
        if p.member
            .distance_to_strike_pct
            .is_some_and(|d| d <= Decimal::ZERO)
        {
            strike_zone_count += 1;
        }
    }

    let sic_codes: Vec<u32> = sic_counts.keys().copied().collect();
    let dominant_sic = sic_counts
        .iter()
        .max_by(|(a_sic, a_cnt), (b_sic, b_cnt)| a_cnt.cmp(b_cnt).then_with(|| b_sic.cmp(a_sic)))
        .map(|(sic, _)| *sic);
    let dominant_desc = desc_counts
        .into_iter()
        .max_by(|(a_d, a_cnt), (b_d, b_cnt)| a_cnt.cmp(b_cnt).then_with(|| b_d.cmp(a_d)))
        .map(|(d, _)| d);

    let mut kw_list: Vec<(String, usize)> = kw_counts.into_iter().collect();
    kw_list.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    let keywords: Vec<String> = kw_list.into_iter().take(6).map(|(k, _)| k).collect();

    let mut sim_sum = 0.0f32;
    let mut sim_cnt = 0usize;
    if !cluster.centroid.is_empty() {
        for p in &cluster.members {
            if p.centroid.len() == cluster.centroid.len() {
                sim_sum += cosine_similarity(&p.centroid, &cluster.centroid).clamp(0.0, 1.0);
                sim_cnt += 1;
            }
        }
    }
    let avg_similarity = if sim_cnt > 0 {
        sim_sum / (sim_cnt as f32)
    } else if sic_codes.len() == 1 {
        1.0
    } else {
        0.0
    };

    let median_roic_5y_pct = decimal_median(&mut roics);

    let tickers: Vec<String> = cluster
        .members
        .iter()
        .map(|m| m.member.ticker.clone())
        .collect();
    let members: Vec<ThemeClusterMember> = cluster.members.into_iter().map(|p| p.member).collect();

    let title = match (&dominant_desc, dominant_sic) {
        (Some(desc), Some(sic)) => format!("{desc} (SIC {sic})"),
        (Some(desc), None) => desc.clone(),
        (None, Some(sic)) => format!("SIC {sic} Cluster"),
        (None, None) => {
            if keywords.is_empty() {
                format!("{} Archetype", tickers.first().map_or("US", String::as_str))
            } else {
                keywords
                    .iter()
                    .take(3)
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(" · ")
            }
        }
    };

    let archetype = if let Some(desc) = dominant_desc {
        if keywords.is_empty() {
            desc
        } else {
            format!(
                "{desc} · {}",
                keywords
                    .iter()
                    .take(3)
                    .cloned()
                    .collect::<Vec<_>>()
                    .join("/")
            )
        }
    } else if !keywords.is_empty() {
        keywords
            .iter()
            .take(3)
            .cloned()
            .collect::<Vec<_>>()
            .join(" / ")
    } else {
        "Uncategorized Archetype".to_owned()
    };

    let theme_id = match dominant_sic {
        Some(sic) => {
            let slug = keywords
                .first()
                .map(|k| slugify(k))
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| "cluster".to_owned());
            format!("sic-{sic}-{slug}")
        }
        None => {
            let slug = keywords
                .iter()
                .take(2)
                .map(|k| slugify(k))
                .filter(|s| !s.is_empty())
                .collect::<Vec<_>>()
                .join("-");
            if slug.is_empty() {
                format!(
                    "theme-{}",
                    tickers
                        .first()
                        .map(|t| slugify(t))
                        .unwrap_or_else(|| "group".to_owned())
                )
            } else {
                format!("theme-{slug}")
            }
        }
    };

    let roic_str = median_roic_5y_pct
        .map(|r| format!("{}%", r.round_dp(1).normalize()))
        .unwrap_or_else(|| "—".to_owned());
    let summary = format!(
        "{} 家公司 · 5Y ROIC 中位数 {} · 击球区 {} 家 · 隐空间相似度 {:.2}",
        members.len(),
        roic_str,
        strike_zone_count,
        avg_similarity
    );

    ThemeCluster {
        theme_id,
        title,
        archetype,
        sic_codes,
        keywords,
        summary,
        tickers,
        members,
        avg_similarity,
        median_roic_5y_pct,
        strike_zone_count,
        user_notes: String::new(),
    }
}

fn decimal_median(values: &mut [Decimal]) -> Option<Decimal> {
    if values.is_empty() {
        return None;
    }
    values.sort();
    let mid = values.len() / 2;
    if values.len() % 2 == 1 {
        Some(values[mid].normalize())
    } else {
        let sum = values[mid - 1] + values[mid];
        Some((sum / Decimal::from(2)).normalize())
    }
}

fn slugify(s: &str) -> String {
    let mut out = String::new();
    let mut prev_dash = false;
    for ch in s.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
            prev_dash = false;
        } else if !prev_dash && !out.is_empty() {
            out.push('-');
            prev_dash = true;
        }
    }
    out.trim_matches('-').chars().take(28).collect()
}

fn extract_top_keywords(text: &str, max_k: usize) -> Vec<String> {
    const STOP: &[&str] = &[
        "the",
        "and",
        "for",
        "with",
        "that",
        "this",
        "from",
        "are",
        "was",
        "were",
        "have",
        "has",
        "had",
        "our",
        "its",
        "not",
        "but",
        "into",
        "over",
        "such",
        "also",
        "other",
        "may",
        "can",
        "will",
        "more",
        "than",
        "their",
        "they",
        "which",
        "these",
        "those",
        "company",
        "business",
        "products",
        "services",
        "operations",
        "financial",
        "including",
        "million",
        "billion",
        "fiscal",
        "year",
        "years",
        "item",
        "form",
        "report",
        "united",
        "states",
        "inc",
        "corp",
        "corporation",
        "ltd",
    ];
    let stop_set: HashSet<&str> = STOP.iter().copied().collect();
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for token in text
        .split(|c: char| !c.is_ascii_alphanumeric())
        .map(|w| w.trim().to_ascii_lowercase())
    {
        if token.len() < 3
            || token.chars().all(|c| c.is_ascii_digit())
            || stop_set.contains(token.as_str())
        {
            continue;
        }
        *counts.entry(token).or_insert(0) += 1;
    }
    let mut pairs: Vec<(String, usize)> = counts.into_iter().collect();
    pairs.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    pairs.into_iter().take(max_k).map(|(w, _)| w).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lake::{LakeCompanyRow, UniverseRow};
    use crate::lake_l1::DocChunkRow;
    use crate::latent::{
        LOCAL_EMBEDDING_DIM, LOCAL_EMBEDDING_MODEL, encode_embedding_json, local_embed, sha256_hex,
    };
    use crate::model::Bucket;
    use std::fs;

    fn temp_paths(tag: &str) -> WorkspacePaths {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let root = std::env::temp_dir().join(format!(
            "m3_budget_test_{tag}_{}_{}",
            std::process::id(),
            nanos
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("workbench")).unwrap();
        WorkspacePaths::from_root(root)
    }

    #[test]
    fn budget_config_defaults_and_roundtrip_work() {
        let paths = temp_paths("cfg_roundtrip");
        let cfg = load_budget_config(&paths).unwrap();
        assert_eq!(cfg.schema_version, 1);
        assert_eq!(cfg.monthly_budget_usd, Decimal::new(1000, 2));
        assert_eq!(cfg.per_run_budget_usd, Decimal::new(75, 2));
        assert_eq!(cfg.monthly_token_limit, 2_000_000);
        assert_eq!(cfg.per_run_token_limit, 120_000);
        assert!(!cfg.auto_trigger_enabled);
        assert_eq!(cfg.weekly_auto_quota, 0);
        assert_eq!(cfg.input_usd_per_1m_tokens, Decimal::new(300, 2));
        assert_eq!(cfg.output_usd_per_1m_tokens, Decimal::new(1500, 2));

        let custom = BudgetConfig {
            monthly_budget_usd: Decimal::new(2550, 2),
            auto_trigger_enabled: true,
            weekly_auto_quota: 3,
            ..cfg
        };
        save_budget_config(&paths, &custom).unwrap();
        let reloaded = load_budget_config(&paths).unwrap();
        assert_eq!(reloaded, custom);
        let _ = fs::remove_dir_all(&paths.repo_root);
    }

    #[test]
    fn estimate_cost_and_budget_gate_enforce_all_hard_ceilings() {
        let cfg = BudgetConfig::default();
        let cost = estimate_cost_usd(&cfg, 100_000, 20_000);
        // 0.1M * $3 + 0.02M * $15 = $0.30 + $0.30 = $0.60
        assert_eq!(cost, Decimal::new(60, 2));

        let ok = evaluate_budget_gate(&cfg, 100_000, Decimal::new(50, 2), 0, false);
        assert!(ok.allowed);
        assert!(ok.reason.is_none());
        assert_eq!(ok.remaining_monthly_usd, Decimal::new(950, 2));
        assert_eq!(ok.remaining_monthly_tokens, 1_900_000);

        let blocked_usd = evaluate_budget_gate(&cfg, 10_000, Decimal::new(1000, 2), 0, false);
        assert!(!blocked_usd.allowed);
        assert_eq!(blocked_usd.remaining_monthly_usd, Decimal::ZERO);

        let blocked_zero_budget = evaluate_budget_gate(
            &BudgetConfig {
                monthly_budget_usd: Decimal::ZERO,
                ..cfg.clone()
            },
            0,
            Decimal::ZERO,
            0,
            false,
        );
        assert!(!blocked_zero_budget.allowed);

        let blocked_tokens = evaluate_budget_gate(&cfg, 2_000_000, Decimal::ZERO, 0, false);
        assert!(!blocked_tokens.allowed);
        assert_eq!(blocked_tokens.remaining_monthly_tokens, 0);

        let blocked_per_run = evaluate_budget_gate(
            &BudgetConfig {
                per_run_budget_usd: Decimal::ZERO,
                ..cfg.clone()
            },
            0,
            Decimal::ZERO,
            0,
            false,
        );
        assert!(!blocked_per_run.allowed);

        let blocked_per_run_tok = evaluate_budget_gate(
            &BudgetConfig {
                per_run_token_limit: 0,
                ..cfg.clone()
            },
            0,
            Decimal::ZERO,
            0,
            false,
        );
        assert!(!blocked_per_run_tok.allowed);

        let blocked_auto_off = evaluate_budget_gate(&cfg, 0, Decimal::ZERO, 0, true);
        assert!(!blocked_auto_off.allowed);

        let auto_cfg = BudgetConfig {
            auto_trigger_enabled: true,
            weekly_auto_quota: 2,
            ..cfg
        };
        assert!(evaluate_budget_gate(&auto_cfg, 0, Decimal::ZERO, 1, true).allowed);
        assert!(!evaluate_budget_gate(&auto_cfg, 0, Decimal::ZERO, 2, true).allowed);
    }

    #[test]
    fn sync_themes_from_lake_clusters_by_sic_and_latent_and_preserves_user_notes() {
        let paths = temp_paths("themes_sync");
        assert!(load_themes(&paths).unwrap().clusters.is_empty());

        let mut db = LakeDb::open(&paths).unwrap();
        let make_row =
            |ticker: &str, cik: &str, name: &str, sic: u32, desc: &str, roic: i64, dist: i64| {
                UniverseRow {
                    ticker: ticker.to_owned(),
                    cik: cik.to_owned(),
                    name: name.to_owned(),
                    exchange: "NASDAQ".to_owned(),
                    sic: Some(sic),
                    sic_description: Some(desc.to_owned()),
                    is_financial: false,
                    latest_filing_end: Some("2026-01-31".to_owned()),
                    years_of_data: 7,
                    quality_score: Decimal::from(85),
                    in_price_pool: true,
                    in_watchlist: true,
                    bucket_opt: Some(Bucket::Watch),
                    latest_price: Some(Decimal::from(100)),
                    strike_price: Some(Decimal::from(110)),
                    distance_to_strike_pct: Some(Decimal::from(dist)),
                    strict_fcf_yield_pct: Some(Decimal::from(6)),
                    roic_5y_avg_pct: Some(Decimal::from(roic)),
                    roic_5y_stddev_pct: Some(Decimal::from(2)),
                    gross_margin_5y_avg_pct: Some(Decimal::from(75)),
                    gross_margin_5y_stddev_pct: Some(Decimal::from(1)),
                    share_change_3y_pct: Some(Decimal::from(-2)),
                    sbc_to_ocf_pct: Some(Decimal::from(8)),
                    net_debt_to_oe: Some(Decimal::ZERO),
                    strict_owner_earnings: Some(Decimal::from(500_000_000i64)),
                    warnings_count: 0,
                    warnings_json: "[]".to_owned(),
                    raw_sha256: Some("abc".to_owned()),
                }
            };

        let rows = vec![
            make_row(
                "ADSK",
                "0000769397",
                "Autodesk",
                7372,
                "Prepackaged Software",
                40,
                -5,
            ),
            make_row(
                "PTC",
                "0000857005",
                "PTC Inc.",
                7372,
                "Prepackaged Software",
                20,
                10,
            ),
            make_row(
                "COST",
                "0000909832",
                "Costco",
                5331,
                "Retail-Variety Stores",
                32,
                20,
            ),
            make_row(
                "BJ",
                "0001531152",
                "BJ's Wholesale",
                5331,
                "Retail-Variety Stores",
                18,
                -3,
            ),
        ];
        let companies: Vec<LakeCompanyRow> = rows
            .iter()
            .map(|r| LakeCompanyRow {
                cik: r.cik.clone(),
                ticker: r.ticker.clone(),
                name: r.name.clone(),
                exchange: r.exchange.clone(),
                sic: r.sic,
                sic_description: r.sic_description.clone(),
                is_financial: false,
                is_active: true,
            })
            .collect();
        db.replace_sec_lake_data(&companies, &[], &[], &rows)
            .unwrap();

        let mk_chunk = |id: &str, cik: &str, ticker: &str, text: &str| DocChunkRow {
            chunk_id: id.to_owned(),
            cik: cik.to_owned(),
            ticker: ticker.to_owned(),
            doc_type: "10-K".to_owned(),
            source_id: id.to_owned(),
            period_end: Some("2026-01-31".to_owned()),
            filing_date: Some("2026-03-01".to_owned()),
            section: "item_1".to_owned(),
            title: "Business".to_owned(),
            content: text.to_owned(),
            content_sha256: sha256_hex(text.as_bytes()),
            source_url: "https://www.sec.gov/example".to_owned(),
            embedding_model: LOCAL_EMBEDDING_MODEL.to_owned(),
            embedding_dim: LOCAL_EMBEDDING_DIM,
            embedding_json: encode_embedding_json(&local_embed(text)),
        };

        db.upsert_doc_chunks(&[
            mk_chunk(
                "c1",
                "0000769397",
                "ADSK",
                "Engineering CAD subscription software for architecture and manufacturing design.",
            ),
            mk_chunk(
                "c2",
                "0000857005",
                "PTC",
                "Engineering CAD and PLM subscription software for industrial manufacturing.",
            ),
            mk_chunk(
                "c3",
                "0000909832",
                "COST",
                "Membership warehouse club retail merchandising with low gross margin.",
            ),
            mk_chunk(
                "c4",
                "0001531152",
                "BJ",
                "Membership warehouse club retail merchandising groceries and general goods.",
            ),
        ])
        .unwrap();

        let mut first = sync_themes_from_lake(&paths, &db).unwrap();
        assert_eq!(first.clusters.len(), 2);
        assert_eq!(first.total_indexed_companies, 4);
        first.clusters[0].user_notes = "重点跟踪工业 CAD 订阅提价权".to_owned();
        let kept_id = first.clusters[0].theme_id.clone();
        save_themes(&paths, &first).unwrap();

        let second = sync_themes_from_lake(&paths, &db).unwrap();
        let matched = second
            .clusters
            .iter()
            .find(|c| c.theme_id == kept_id)
            .expect("cluster id should remain stable");
        assert_eq!(matched.user_notes, "重点跟踪工业 CAD 订阅提价权");
        let _ = fs::remove_dir_all(&paths.repo_root);
    }
}
