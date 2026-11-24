use super::{FontRoles, MungerWorkbench, NavView, components::*, format::*, theme::*};
use crate::{
    lake::{
        LakeAnnualRow, LakeDb, UniverseDrawerDetails, UniverseFilter, UniverseRow, UniverseSortBy,
        UniverseSummary,
    },
    lake_l1::{SimilarCompanyHit, SimilarSearchFilter},
    model::{Bucket, WarningFlag, WarningSeverity},
    store::{self, WorkspacePaths},
    universe::{self, UniverseSyncOptions, UniverseSyncReport},
};
use gpui::{Context, bounds, canvas, div, fill, point, prelude::*, px, rgb, rgba, size};
use rust_decimal::Decimal;
use std::collections::HashSet;
use std::sync::mpsc;
use std::time::Instant;

/// Search mode in the Universe Radar control strip.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum UniverseSearchMode {
    #[default]
    Keyword,
    Similar,
}

impl UniverseSearchMode {
    pub fn label(self) -> &'static str {
        match self {
            Self::Keyword => "关键词",
            Self::Similar => "相似 Lookalike",
        }
    }
}

/// Named Munger screening presets in the Universe Radar control strip.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UniversePreset {
    AllQuality,
    HighRoicCompounders,
    LowSbcCashCows,
    StrikeZone,
}

impl UniversePreset {
    pub fn label(self) -> &'static str {
        match self {
            Self::AllQuality => "全部质量池",
            Self::HighRoicCompounders => "高 ROIC 复利 (>=20%)",
            Self::LowSbcCashCows => "低 SBC 现金牛",
            Self::StrikeZone => "击球区",
        }
    }

    pub fn all() -> [Self; 4] {
        [
            Self::AllQuality,
            Self::HighRoicCompounders,
            Self::LowSbcCashCows,
            Self::StrikeZone,
        ]
    }
}

/// Filter, search, sort, and cursor state for the Universe Radar view.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UniverseViewState {
    pub preset: UniversePreset,
    pub exclude_financials: bool,
    pub require_5y_history: bool,
    pub in_price_pool_only: bool,
    pub positive_strict_oe_only: bool,
    pub min_roic_5y: Option<Decimal>,
    pub max_sbc_to_ocf: Option<Decimal>,
    pub max_share_change_3y: Option<Decimal>,
    pub max_net_debt_to_oe: Option<Decimal>,
    pub strike_zone_only: bool,
    pub search_mode: UniverseSearchMode,
    pub similar_as_ticker: bool,
    pub query: String,
    pub search_focused: bool,
    pub sort_by: UniverseSortBy,
    pub sort_asc: bool,
    pub cursor_index: usize,
    pub limit: usize,
}

impl Default for UniverseViewState {
    fn default() -> Self {
        Self {
            preset: UniversePreset::AllQuality,
            exclude_financials: true,
            require_5y_history: true,
            in_price_pool_only: false,
            positive_strict_oe_only: false,
            min_roic_5y: None,
            max_sbc_to_ocf: None,
            max_share_change_3y: None,
            max_net_debt_to_oe: None,
            strike_zone_only: false,
            search_mode: UniverseSearchMode::Keyword,
            similar_as_ticker: false,
            query: String::new(),
            search_focused: false,
            sort_by: UniverseSortBy::QualityScore,
            sort_asc: false,
            cursor_index: 0,
            limit: 200,
        }
    }
}

impl UniverseViewState {
    /// Applies a preset filter while preserving the search query and financial exclusion toggle.
    /// All 5Y Munger presets require at least 5 fiscal years (`require_5y_history = true`).
    pub fn apply_preset(&mut self, preset: UniversePreset) {
        self.preset = preset;
        self.require_5y_history = true;
        self.cursor_index = 0;
        match preset {
            UniversePreset::AllQuality => {
                self.min_roic_5y = None;
                self.max_sbc_to_ocf = None;
                self.max_share_change_3y = None;
                self.max_net_debt_to_oe = None;
                self.positive_strict_oe_only = false;
                self.in_price_pool_only = false;
                self.strike_zone_only = false;
                self.sort_by = UniverseSortBy::QualityScore;
                self.sort_asc = false;
            }
            UniversePreset::HighRoicCompounders => {
                self.min_roic_5y = Some(Decimal::from(20));
                self.max_sbc_to_ocf = None;
                self.max_share_change_3y = None;
                self.max_net_debt_to_oe = None;
                self.positive_strict_oe_only = true;
                self.in_price_pool_only = false;
                self.strike_zone_only = false;
                self.sort_by = UniverseSortBy::Roic5yAvg;
                self.sort_asc = false;
            }
            UniversePreset::LowSbcCashCows => {
                self.min_roic_5y = Some(Decimal::from(15));
                self.max_sbc_to_ocf = Some(Decimal::from(10));
                self.max_share_change_3y = Some(Decimal::from(2));
                self.max_net_debt_to_oe = Some(Decimal::from(3));
                self.positive_strict_oe_only = true;
                self.in_price_pool_only = false;
                self.strike_zone_only = false;
                self.sort_by = UniverseSortBy::StrictFcfYield;
                self.sort_asc = false;
            }
            UniversePreset::StrikeZone => {
                self.min_roic_5y = None;
                self.max_sbc_to_ocf = None;
                self.max_share_change_3y = None;
                self.max_net_debt_to_oe = None;
                self.positive_strict_oe_only = true;
                self.in_price_pool_only = true;
                self.strike_zone_only = true;
                self.sort_by = UniverseSortBy::DistanceToStrike;
                self.sort_asc = true;
            }
        }
    }

    pub fn toggle_exclude_financials(&mut self) {
        self.exclude_financials = !self.exclude_financials;
        self.cursor_index = 0;
    }

    pub fn toggle_require_5y_history(&mut self) {
        self.require_5y_history = !self.require_5y_history;
        self.cursor_index = 0;
    }

    pub fn toggle_in_price_pool_only(&mut self) {
        self.in_price_pool_only = !self.in_price_pool_only;
        self.cursor_index = 0;
    }

    pub fn toggle_sort(&mut self, col: UniverseSortBy) {
        if self.sort_by == col {
            self.sort_asc = !self.sort_asc;
        } else {
            self.sort_by = col;
            self.sort_asc = matches!(
                col,
                UniverseSortBy::Ticker
                    | UniverseSortBy::Name
                    | UniverseSortBy::Sic
                    | UniverseSortBy::DistanceToStrike
                    | UniverseSortBy::ShareChange3y
                    | UniverseSortBy::SbcToOcf
                    | UniverseSortBy::WarningsCount
            );
        }
        self.cursor_index = 0;
    }

    pub fn reset(&mut self) {
        *self = Self::default();
    }

    pub fn is_default(&self) -> bool {
        self.preset == UniversePreset::AllQuality
            && self.exclude_financials
            && self.require_5y_history
            && !self.in_price_pool_only
            && !self.positive_strict_oe_only
            && self.min_roic_5y.is_none()
            && self.max_sbc_to_ocf.is_none()
            && self.max_share_change_3y.is_none()
            && self.max_net_debt_to_oe.is_none()
            && !self.strike_zone_only
            && self.search_mode == UniverseSearchMode::Keyword
            && !self.similar_as_ticker
            && self.query.trim().is_empty()
            && self.sort_by == UniverseSortBy::QualityScore
            && !self.sort_asc
    }

    pub fn set_search_mode(&mut self, mode: UniverseSearchMode) {
        self.search_mode = mode;
        if mode == UniverseSearchMode::Keyword {
            self.similar_as_ticker = false;
        }
        self.cursor_index = 0;
    }

    pub fn start_similar_for_ticker(&mut self, ticker: &str) {
        self.search_mode = UniverseSearchMode::Similar;
        self.similar_as_ticker = true;
        self.query = ticker.trim().to_ascii_uppercase();
        self.search_focused = false;
        self.cursor_index = 0;
    }

    /// Converts UI state into a `LakeDb::search_similar` filter.
    pub fn to_similar_filter(&self) -> SimilarSearchFilter {
        SimilarSearchFilter {
            min_roic_5y: self.min_roic_5y,
            max_sbc_to_ocf: self.max_sbc_to_ocf,
            exclude_financials: self.exclude_financials,
            positive_strict_oe_only: self.positive_strict_oe_only,
            in_price_pool_only: self.in_price_pool_only,
            min_score: 0.0,
            limit: self.limit.clamp(1, 50),
        }
    }

    /// Converts UI state into a `LakeDb::screen_universe` query filter.
    pub fn to_lake_filter(&self) -> UniverseFilter {
        let trimmed = self.query.trim();
        UniverseFilter {
            min_roic_5y: self.min_roic_5y,
            max_sbc_to_ocf: self.max_sbc_to_ocf,
            max_share_change_3y: self.max_share_change_3y,
            max_net_debt_to_oe: self.max_net_debt_to_oe,
            min_years_of_data: self.require_5y_history.then_some(5),
            exclude_financials: self.exclude_financials,
            positive_strict_oe_only: self.positive_strict_oe_only,
            in_price_pool_only: self.in_price_pool_only,
            strike_zone_only: self.strike_zone_only,
            query: (!trimmed.is_empty()).then(|| trimmed.to_owned()),
            sort_by: self.sort_by,
            sort_asc: self.sort_asc,
            limit: self.limit,
        }
    }
}

/// Applies deterministic client-side UI invariants (e.g. requiring 5+ years for 5Y presets).
pub fn apply_ui_row_guards(
    mut rows: Vec<UniverseRow>,
    require_5y_history: bool,
) -> Vec<UniverseRow> {
    if require_5y_history {
        rows.retain(|r| r.years_of_data >= 5);
    }
    rows
}

/// Returns `true` when `m3.duckdb` does not exist yet or contains zero companies.
pub fn is_lake_empty(lake_present: bool, summary: Option<&UniverseSummary>) -> bool {
    !lake_present || summary.is_none_or(|s| s.total_companies == 0)
}

/// One plotted company point on the 2D ROIC × Strict FCF Yield scatter canvas.
#[derive(Debug, Clone, PartialEq)]
pub struct ScatterPoint {
    pub cik: String,
    pub ticker: String,
    pub roic_pct: f64,
    pub fcf_yield_pct: f64,
    pub x_norm: f32,
    pub y_norm: f32,
    pub in_strike_zone: bool,
    pub in_watchlist: bool,
    pub is_selected: bool,
    pub clipped: bool,
}

/// Normalized visual geometry for the 2D ROIC × Strict FCF Yield scatter plot.
///
/// Visual coordinate projection converts `Decimal` metrics to clamped `f64`/`f32` plot ratios
/// solely for canvas rendering; no financial calculation is performed in floating point.
#[derive(Debug, Clone, PartialEq)]
pub struct ScatterPlotModel {
    pub x_min: f64,
    pub x_max: f64,
    pub y_min: f64,
    pub y_max: f64,
    pub ref_roic_norm: Option<f32>,
    pub ref_fcf_norm: Option<f32>,
    pub zero_fcf_norm: Option<f32>,
    pub points: Vec<ScatterPoint>,
    pub clipped_count: usize,
}

fn percentile_sorted(sorted: &[f64], frac: f64) -> Option<f64> {
    if sorted.is_empty() {
        return None;
    }
    let idx = ((sorted.len() - 1) as f64 * frac.clamp(0.0, 1.0)).round() as usize;
    sorted.get(idx).copied()
}

/// Computes normalized 2D scatter coordinates for priced rows with valid 5Y ROIC and FCF yield.
/// Selection highlighting is keyed by canonical SEC `cik`, and strike-zone highlighting requires
/// membership in `strike_zone_ciks` (enforcing the backend's 7-day quote freshness rule).
pub fn build_scatter_model(
    rows: &[UniverseRow],
    strike_zone_ciks: &HashSet<String>,
    selected_cik: Option<&str>,
) -> Option<ScatterPlotModel> {
    struct RawPt<'a> {
        row: &'a UniverseRow,
        roic: f64,
        fcf: f64,
    }

    let mut raw: Vec<RawPt<'_>> = Vec::new();
    for row in rows {
        if let (Some(roic_d), Some(fcf_d)) = (row.roic_5y_avg_pct, row.strict_fcf_yield_pct)
            && let (Some(roic), Some(fcf)) = (dec_to_f64(roic_d), dec_to_f64(fcf_d))
        {
            raw.push(RawPt { row, roic, fcf });
        }
    }
    if raw.is_empty() {
        return None;
    }

    let mut xs: Vec<f64> = raw.iter().map(|p| p.roic).collect();
    let mut ys: Vec<f64> = raw.iter().map(|p| p.fcf).collect();
    xs.sort_by(|a, b| a.total_cmp(b));
    ys.sort_by(|a, b| a.total_cmp(b));

    let x_min = if xs.first().is_some_and(|v| *v < 0.0) {
        xs.first().copied().unwrap_or(0.0).clamp(-20.0, -5.0)
    } else {
        0.0
    };
    let p90_x = percentile_sorted(&xs, 0.90).unwrap_or(45.0);
    let x_max = p90_x.clamp(35.0, 80.0);

    let y_min = if ys.first().is_some_and(|v| *v < 0.0) {
        ys.first().copied().unwrap_or(0.0).clamp(-15.0, -2.0)
    } else {
        0.0
    };
    let p90_y = percentile_sorted(&ys, 0.90).unwrap_or(12.0);
    let y_max = (p90_y * 1.15).clamp(12.0, 25.0);

    let x_span = (x_max - x_min).max(1.0);
    let y_span = (y_max - y_min).max(1.0);

    let norm_x = |v: f64| -> f32 { ((v.clamp(x_min, x_max) - x_min) / x_span) as f32 };
    let norm_y = |v: f64| -> f32 { ((v.clamp(y_min, y_max) - y_min) / y_span) as f32 };

    let mut clipped_count = 0usize;
    let mut points: Vec<ScatterPoint> = raw
        .into_iter()
        .map(|p| {
            let clipped = p.roic < x_min || p.roic > x_max || p.fcf < y_min || p.fcf > y_max;
            if clipped {
                clipped_count += 1;
            }
            let in_strike_zone = p.row.in_price_pool
                && strike_zone_ciks.contains(&p.row.cik)
                && p.row
                    .distance_to_strike_pct
                    .is_some_and(|d| d <= Decimal::ZERO);
            let is_selected = selected_cik.is_some_and(|sel| p.row.cik == sel);
            ScatterPoint {
                cik: p.row.cik.clone(),
                ticker: p.row.ticker.clone(),
                roic_pct: p.roic,
                fcf_yield_pct: p.fcf,
                x_norm: norm_x(p.roic),
                y_norm: norm_y(p.fcf),
                in_strike_zone,
                in_watchlist: p.row.in_watchlist,
                is_selected,
                clipped,
            }
        })
        .collect();

    // Paint background dots first, strike/watchlist dots above them, and selected dot on top.
    points.sort_by_key(|p| {
        if p.is_selected {
            3u8
        } else if p.in_strike_zone {
            2
        } else if p.in_watchlist {
            1
        } else {
            0
        }
    });

    let ref_roic_norm = (15.0 >= x_min && 15.0 <= x_max).then(|| norm_x(15.0));
    let ref_fcf_norm = (5.0 >= y_min && 5.0 <= y_max).then(|| norm_y(5.0));
    let zero_fcf_norm = (0.0 > y_min && 0.0 < y_max).then(|| norm_y(0.0));

    Some(ScatterPlotModel {
        x_min,
        x_max,
        y_min,
        y_max,
        ref_roic_norm,
        ref_fcf_norm,
        zero_fcf_norm,
        points,
        clipped_count,
    })
}

fn universe_distance_color(row: &UniverseRow, strike_zone_ciks: &HashSet<String>) -> u32 {
    match row.distance_to_strike_pct {
        Some(d)
            if d <= Decimal::ZERO && row.in_price_pool && strike_zone_ciks.contains(&row.cik) =>
        {
            EMERALD_FG
        }
        Some(d) if d <= Decimal::ZERO => INK_SECONDARY,
        Some(d) if d <= Decimal::from(15) => GOLD_FG,
        Some(_) => INK_SECONDARY,
        None => INK_MUTED,
    }
}

fn format_sic_line(row: &UniverseRow) -> String {
    match (
        row.sic,
        row.sic_description
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty()),
    ) {
        (Some(sic), Some(desc)) => format!("{} · SIC {sic} ({desc})", row.exchange),
        (Some(sic), None) => format!("{} · SIC {sic}", row.exchange),
        (None, Some(desc)) => format!("{} · {desc} (未分类SIC)", row.exchange),
        (None, None) => format!("{} · 未分类SIC", row.exchange),
    }
}

/// Resolves the active Universe row for inspection, preferring the drawer's committed snapshot copy
/// when `selected_cik` matches so drawer header metrics, 5Y annuals, and peers share one snapshot.
pub fn resolve_active_universe_row<'a>(
    selected_cik: Option<&str>,
    selected_row: Option<&'a UniverseRow>,
    rows: &'a [UniverseRow],
) -> Option<&'a UniverseRow> {
    selected_cik
        .and_then(|cik| {
            selected_row
                .filter(|r| r.cik == cik)
                .or_else(|| rows.iter().find(|r| r.cik == cik))
        })
        .or_else(|| rows.first())
}

pub(super) fn format_watch_promote_banner(
    ticker: &str,
    name: &str,
    disk_reload_err: Option<&str>,
    lake_refresh_err: Option<&str>,
    universe_reload_err: Option<&str>,
    universe_syncing: bool,
) -> (String, bool) {
    if let Some(err) = disk_reload_err {
        return (
            format!("已写入 workbench/watchlist.json ({ticker})，但刷新自选池失败：{err}"),
            true,
        );
    }
    if let Some(err) = lake_refresh_err {
        return (
            format!(
                "已写入 workbench/watchlist.json ({ticker})，但同步 DuckDB 观察标记失败：{err}"
            ),
            true,
        );
    }
    if let Some(err) = universe_reload_err {
        return (
            format!("已写入 workbench/watchlist.json ({ticker})，但重载全市场视图失败：{err}"),
            true,
        );
    }
    if universe_syncing {
        return (
            format!(
                "已将 {ticker} ({name}) 写入观察池 [WATCH]；后台全市场同步完成后将刷新湖仓标记"
            ),
            false,
        );
    }
    (
        format!(
            "已将 {ticker} ({name}) 加入观察池 [WATCH]（已写入 workbench/watchlist.json），可在「总览」或 Dossier / Alan 中查看"
        ),
        false,
    )
}

impl MungerWorkbench {
    /// Returns the currently inspected Universe row, preferring the drawer's committed snapshot copy.
    pub(super) fn active_universe_row(&self) -> Option<&UniverseRow> {
        resolve_active_universe_row(
            self.universe_selected_cik.as_deref(),
            self.universe_selected_row.as_ref(),
            &self.universe_rows,
        )
    }

    fn clear_universe_drawer_details(&mut self) {
        self.universe_selected_annuals.clear();
        self.universe_selected_peers = None;
    }

    fn apply_universe_drawer_details(&mut self, details: UniverseDrawerDetails) {
        if self.universe_selected_cik.as_deref() != Some(details.row.cik.as_str()) {
            return;
        }
        if let Some(existing) = self
            .universe_rows
            .iter_mut()
            .find(|r| r.cik == details.row.cik)
        {
            *existing = details.row.clone();
        }
        self.universe_selected_row = Some(details.row);
        self.universe_selected_annuals = details.annuals;
        self.universe_selected_peers = details.peers;
    }

    /// Reloads Universe Radar summary, screened rows, and selected company annuals/peers
    /// from one committed DuckDB snapshot.
    pub(super) fn reload_universe_from_lake(&mut self) {
        if self.universe_syncing {
            return;
        }
        match LakeDb::open_read_only_if_exists(&self.paths) {
            Ok(None) => {
                self.universe_lake_present = false;
                self.universe_summary = None;
                self.universe_rows.clear();
                self.universe_similar_hits.clear();
                self.universe_similar_query_ms = None;
                self.universe_strike_zone_ciks.clear();
                self.universe_selected_cik = None;
                self.universe_selected_row = None;
                self.clear_universe_drawer_details();
                self.universe_error = None;
            }
            Ok(Some(db)) => {
                self.universe_lake_present = true;
                let trimmed_query = self.universe_state.query.trim().to_owned();
                if self.universe_state.search_mode == UniverseSearchMode::Similar
                    && !trimmed_query.is_empty()
                {
                    match db.universe_summary() {
                        Ok(summary) => self.universe_summary = Some(summary),
                        Err(err) => {
                            self.clear_universe_drawer_details();
                            self.universe_error = Some(format!("读取全市场湖仓摘要失败: {err:#}"));
                            return;
                        }
                    }
                    let started = Instant::now();
                    let sim_filter = self.universe_state.to_similar_filter();
                    match db.search_similar(
                        &trimmed_query,
                        self.universe_state.similar_as_ticker,
                        &sim_filter,
                    ) {
                        Ok(hits) => {
                            let elapsed_ms =
                                u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
                            self.universe_similar_query_ms = Some(elapsed_ms);
                            self.universe_rows = hits.iter().map(|h| h.company.clone()).collect();
                            self.universe_similar_hits = hits;
                            self.universe_error = None;
                            self.sync_universe_drawer_selection(&db, None);
                        }
                        Err(err) => {
                            self.universe_similar_hits.clear();
                            self.universe_similar_query_ms = None;
                            self.universe_rows.clear();
                            self.clear_universe_drawer_details();
                            self.universe_error = Some(format!("混合语义相似搜索失败: {err:#}"));
                        }
                    }
                    return;
                }

                self.universe_similar_hits.clear();
                self.universe_similar_query_ms = None;

                match db.load_universe_snapshot(
                    &self.universe_state.to_lake_filter(),
                    self.universe_selected_cik.as_deref(),
                    5,
                ) {
                    Ok(snapshot) => {
                        self.universe_summary = Some(snapshot.summary);
                        self.universe_strike_zone_ciks = snapshot.strike_zone_ciks;
                        self.universe_rows = apply_ui_row_guards(
                            snapshot.rows,
                            self.universe_state.require_5y_history,
                        );
                        self.universe_error = None;
                        self.sync_universe_drawer_selection(&db, snapshot.selected_details);
                    }
                    Err(err) => {
                        self.clear_universe_drawer_details();
                        self.universe_error = Some(format!("读取全市场湖仓失败: {err:#}"));
                    }
                }
            }
            Err(err) => {
                self.universe_lake_present = self.paths.lake_db_path().exists();
                self.clear_universe_drawer_details();
                self.universe_error = Some(format!("打开 m3.duckdb 失败: {err:#}"));
            }
        }
    }

    fn sync_universe_drawer_selection(
        &mut self,
        db: &LakeDb,
        snapshot_details: Option<UniverseDrawerDetails>,
    ) {
        let target_idx = self
            .universe_selected_cik
            .as_ref()
            .and_then(|sel| self.universe_rows.iter().position(|r| r.cik == *sel))
            .or_else(|| (!self.universe_rows.is_empty()).then_some(0));

        if let Some(idx) = target_idx {
            self.universe_state.cursor_index = idx;
            let target_row = self.universe_rows[idx].clone();
            let target_cik = target_row.cik.clone();
            self.universe_selected_cik = Some(target_cik.clone());
            self.universe_selected_row = Some(target_row);
            if let Some(details) = snapshot_details.filter(|d| d.row.cik == target_cik) {
                self.apply_universe_drawer_details(details);
            } else {
                self.clear_universe_drawer_details();
                match db.load_drawer_details_by_cik(&target_cik, 5) {
                    Ok(Some(details)) => self.apply_universe_drawer_details(details),
                    Ok(None) => self.clear_universe_drawer_details(),
                    Err(err) => {
                        self.clear_universe_drawer_details();
                        self.universe_error = Some(format!("读取全市场湖仓详情失败: {err:#}"));
                    }
                }
            }
        } else {
            self.universe_state.cursor_index = 0;
            self.universe_selected_cik = None;
            self.universe_selected_row = None;
            self.clear_universe_drawer_details();
        }
    }

    pub(super) fn find_similar_for_company(&mut self, ticker: &str, cx: &mut Context<Self>) {
        self.universe_state.start_similar_for_ticker(ticker);
        self.universe_selected_cik = None;
        self.universe_selected_row = None;
        self.reload_universe_from_lake();
        cx.notify();
    }

    /// Selects a universe row by canonical CIK and loads its summary, annuals, and peers
    /// from a single committed DuckDB snapshot.
    pub(super) fn select_universe_row(&mut self, row: &UniverseRow) {
        self.universe_selected_cik = Some(row.cik.clone());
        self.universe_selected_row = Some(row.clone());
        self.clear_universe_drawer_details();
        if let Some(idx) = self.universe_rows.iter().position(|r| r.cik == row.cik) {
            self.universe_state.cursor_index = idx;
        }
        if self.universe_syncing {
            return;
        }
        match LakeDb::open_read_only_if_exists(&self.paths) {
            Ok(Some(db)) => match db.load_drawer_details_by_cik(&row.cik, 5) {
                Ok(Some(details)) => self.apply_universe_drawer_details(details),
                Ok(None) => self.clear_universe_drawer_details(),
                Err(err) => {
                    self.clear_universe_drawer_details();
                    self.universe_error = Some(format!("读取全市场湖仓详情失败: {err:#}"));
                }
            },
            Ok(None) => {}
            Err(err) => {
                self.clear_universe_drawer_details();
                self.universe_error = Some(format!("打开 m3.duckdb 失败: {err:#}"));
            }
        }
    }

    pub(super) fn step_universe_cursor(&mut self, delta: isize) {
        let len = self.universe_rows.len();
        if len == 0 {
            return;
        }
        let cur = self.universe_state.cursor_index.min(len - 1);
        let next = (cur as isize + delta).clamp(0, (len - 1) as isize) as usize;
        if let Some(row) = self.universe_rows.get(next).cloned() {
            self.select_universe_row(&row);
        }
    }

    /// Re-runs the DuckDB screen query when search input or filter pills change.
    pub(super) fn apply_universe_query_change(&mut self) {
        self.universe_state.cursor_index = 0;
        self.reload_universe_from_lake();
    }

    /// Spawns background L0 SEC bulk + Tier-1 price sync without blocking the GPUI main thread.
    pub(super) fn start_universe_sync(&mut self, full_sec: bool, cx: &mut Context<Self>) {
        if self.universe_syncing || self.syncing {
            if self.syncing {
                self.banner = Some((
                    "自选池正在后台同步中，请等待完成后再同步全美股 L0".into(),
                    false,
                ));
                cx.notify();
            }
            return;
        }
        self.universe_syncing = true;
        self.universe_sync_progress =
            Some("正在启动全美股 L0 同步（SEC EDGAR Bulk + 质量分层）…".into());
        self.banner = Some((
            "正在后台同步全美股 L0 确定性湖仓（SEC companyfacts.zip + Top 400 报价）…".into(),
            false,
        ));

        let root = self.paths.repo_root.clone();
        let (done_tx, done_rx) = mpsc::channel();
        let (prog_tx, prog_rx) = mpsc::channel();
        self.universe_sync_rx = Some(done_rx);
        self.universe_progress_rx = Some(prog_rx);

        std::thread::spawn(move || {
            let paths = WorkspacePaths::from_root(root);
            let opts = UniverseSyncOptions {
                full_sec,
                ..UniverseSyncOptions::default()
            };
            let res = universe::sync_universe_with_progress(&paths, &opts, |msg| {
                let _ = prog_tx.send(msg.to_owned());
            })
            .map_err(|e| format!("{e:#}"));
            let _ = done_tx.send(res);
        });
        cx.notify();
    }

    /// Non-blocking poll for background Universe L0 sync progress and completion.
    pub(super) fn poll_universe_sync(&mut self) -> bool {
        let mut dirty = false;
        if let Some(prog_rx) = self.universe_progress_rx.as_ref() {
            while let Ok(msg) = prog_rx.try_recv() {
                self.universe_sync_progress = Some(msg);
                dirty = true;
            }
        }
        let Some(rx) = self.universe_sync_rx.as_ref() else {
            return dirty;
        };
        match rx.try_recv() {
            Ok(Ok(report)) => {
                self.universe_syncing = false;
                self.universe_sync_rx = None;
                self.universe_progress_rx = None;
                self.universe_sync_progress = None;
                self.reload_universe_from_lake();
                self.banner = Some(Self::format_universe_sync_banner(&report));
                true
            }
            Ok(Err(err)) => {
                self.universe_syncing = false;
                self.universe_sync_rx = None;
                self.universe_progress_rx = None;
                self.universe_sync_progress = None;
                self.banner = Some((format!("全美股 L0 同步失败：{err}"), true));
                true
            }
            Err(mpsc::TryRecvError::Empty) => dirty,
            Err(mpsc::TryRecvError::Disconnected) => {
                self.universe_syncing = false;
                self.universe_sync_rx = None;
                self.universe_progress_rx = None;
                self.universe_sync_progress = None;
                true
            }
        }
    }

    fn format_universe_sync_banner(report: &UniverseSyncReport) -> (String, bool) {
        let base = format!(
            "全美股 L0 同步完成 · 解析 {} 家 / 质量池 {} 家 / 报价 {}/{} 家",
            report.parsed_companies,
            report.quality_pool_count,
            report.prices_succeeded,
            report.prices_attempted
        );
        match report.sic_warning.as_deref() {
            Some(warn) => (format!("{base}（SIC 提示: {warn}）"), true),
            None => (base, false),
        }
    }

    /// Adds a discovered company from the Universe Radar into `workbench/watchlist.json` as WATCH
    /// using its already-verified lakehouse CIK without performing network lookup on the GPUI thread.
    pub(super) fn add_universe_company_to_watch(
        &mut self,
        row: &UniverseRow,
        cx: &mut Context<Self>,
    ) {
        let circle = match (row.sic, row.sic_description.as_deref()) {
            (Some(sic), Some(desc)) if !desc.trim().is_empty() => {
                Some(format!("{} / SIC {sic}", desc.trim()))
            }
            (None, Some(desc)) if !desc.trim().is_empty() => Some(desc.trim().to_owned()),
            (Some(sic), _) => Some(format!("SIC {sic}")),
            _ => None,
        };
        let verified_cik = (!row.cik.trim().is_empty()).then_some(row.cik.as_str());
        match store::add_or_update_company_with_cik(
            &self.paths,
            &row.ticker,
            Bucket::Watch,
            Some(&row.name),
            verified_cik,
            circle.as_deref(),
            None,
        ) {
            Ok(_) => {
                let lake_refresh_err =
                    if !self.universe_syncing && self.paths.lake_db_path().exists() {
                        LakeDb::open(&self.paths)
                            .and_then(|db| db.refresh_watchlist_flags(&self.paths))
                            .err()
                            .map(|err| format!("{err:#}"))
                    } else {
                        None
                    };
                self.banner = None;
                self.reload_from_disk();
                let disk_reload_err = self
                    .banner
                    .take()
                    .filter(|(_, is_err)| *is_err)
                    .map(|(msg, _)| msg);
                if !self.universe_syncing {
                    self.reload_universe_from_lake();
                }
                self.banner = Some(format_watch_promote_banner(
                    &row.ticker,
                    &row.name,
                    disk_reload_err.as_deref(),
                    lake_refresh_err.as_deref(),
                    (!self.universe_syncing)
                        .then_some(self.universe_error.as_deref())
                        .flatten(),
                    self.universe_syncing,
                ));
            }
            Err(err) => {
                self.banner = Some((format!("加入观察池失败 ({})：{err:#}", row.ticker), true));
            }
        }
        cx.notify();
    }

    fn live_watchlist_bucket(&self, row: &UniverseRow) -> Option<Bucket> {
        self.reports
            .iter()
            .find(|r| r.ticker.eq_ignore_ascii_case(&row.ticker))
            .map(|r| r.bucket)
            .or_else(|| {
                row.in_watchlist
                    .then_some(row.bucket_opt.unwrap_or(Bucket::Watch))
            })
    }

    pub(super) fn render_universe_view(&self, cx: &mut Context<Self>) -> gpui::Div {
        let lake_empty = is_lake_empty(self.universe_lake_present, self.universe_summary.as_ref());

        let mut root = div()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .overflow_hidden()
            .child(self.render_universe_summary_strip(cx))
            .child(self.render_universe_filter_strip(cx));

        if let Some(err) = &self.universe_error {
            root = root.child(
                div()
                    .px_5()
                    .py_2()
                    .bg(rgba(CORAL_BG))
                    .border_b_1()
                    .border_color(rgba(CORAL_BORDER))
                    .font_family(self.fonts.ui.clone())
                    .text_xs()
                    .text_color(rgb(CORAL_FG))
                    .child(err.clone()),
            );
        }

        if lake_empty {
            return root.child(
                div()
                    .id("universe-empty-scroll")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .p_6()
                    .child(self.render_universe_empty_state(cx)),
            );
        }

        let selected_row = self.active_universe_row();

        let mut left_pane = div()
            .id("universe-main-scroll")
            .flex_1()
            .min_w_0()
            .h_full()
            .overflow_y_scroll()
            .p_4()
            .flex()
            .flex_col()
            .gap_3();

        if self.universe_state.search_mode == UniverseSearchMode::Similar {
            if self.universe_state.query.trim().is_empty() {
                left_pane = left_pane.child(self.render_universe_similar_prompt());
            } else if self.universe_similar_hits.is_empty() {
                left_pane = left_pane.child(self.render_universe_similar_empty(cx));
            } else {
                left_pane = left_pane.child(
                    self.render_universe_similar_results(selected_row.map(|r| r.cik.as_str()), cx),
                );
            }
        } else if self.universe_rows.is_empty() {
            left_pane = left_pane.child(self.render_universe_no_matches(cx));
        } else {
            left_pane = left_pane
                .child(self.render_universe_scatter_card(selected_row))
                .child(self.render_universe_table(selected_row.map(|r| r.cik.as_str()), cx));
        }

        let mut body = div()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_row()
            .overflow_hidden()
            .child(left_pane);

        if let Some(row) = selected_row {
            body = body.child(self.render_universe_drawer(row, cx));
        }

        root.child(body)
    }

    fn render_universe_summary_strip(&self, cx: &mut Context<Self>) -> gpui::Div {
        let sum = self.universe_summary.clone().unwrap_or_default();
        let last_sync = fmt_time_cn(sum.last_sec_sync_ms.or(sum.last_price_sync_ms));

        div()
            .h(px(40.))
            .flex_shrink_0()
            .px_5()
            .bg(rgb(BG_SURFACE))
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
                    .gap_3()
                    .min_w_0()
                    .child(journal_stat(
                        "覆盖美股",
                        format!(
                            "{} (解析 {})",
                            sum.total_companies, sum.parsed_financial_companies
                        ),
                        INK_HERO,
                        &self.fonts,
                    ))
                    .child(v_divider())
                    .child(journal_stat(
                        "质量候选池",
                        format!("{}", sum.quality_pool_count),
                        SKY_FG,
                        &self.fonts,
                    ))
                    .child(v_divider())
                    .child(journal_stat(
                        "已估值",
                        format!("{}", sum.priced_count),
                        INK_PRIMARY,
                        &self.fonts,
                    ))
                    .child(v_divider())
                    .child(journal_stat(
                        "击球区",
                        format!("{}", sum.strike_zone_count),
                        if sum.strike_zone_count > 0 {
                            EMERALD_FG
                        } else {
                            INK_MUTED
                        },
                        &self.fonts,
                    ))
                    .child(v_divider())
                    .child(journal_stat(
                        "上次同步",
                        last_sync,
                        INK_SECONDARY,
                        &self.fonts,
                    )),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .when_some(self.universe_sync_progress.clone(), |d, msg| {
                        d.child(
                            div()
                                .max_w(px(260.))
                                .px_2()
                                .py_0p5()
                                .rounded_xs()
                                .bg(rgba(SKY_BG))
                                .border_1()
                                .border_color(rgba(SKY_BORDER))
                                .font_family(self.fonts.mono_term.clone())
                                .text_size(px(10.))
                                .text_color(rgb(SKY_FG))
                                .truncate()
                                .child(msg),
                        )
                    })
                    .child(
                        div()
                            .id("univ-refresh-btn")
                            .px_2p5()
                            .py_1()
                            .rounded_sm()
                            .bg(rgb(BG_WELL))
                            .border_1()
                            .border_color(rgb(BORDER_SUBTLE))
                            .hover(|s| s.bg(rgb(BG_HOVER)))
                            .cursor_pointer()
                            .font_family(self.fonts.ui.clone())
                            .text_xs()
                            .text_color(rgb(INK_SECONDARY))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.reload_universe_from_lake();
                                cx.notify();
                            }))
                            .child("刷新"),
                    )
                    .child(
                        div()
                            .id("univ-sync-l0-btn")
                            .px_2p5()
                            .py_1()
                            .rounded_sm()
                            .cursor_pointer()
                            .border_1()
                            .when(self.universe_syncing, |d| {
                                d.bg(rgb(BG_WELL))
                                    .border_color(rgb(BORDER_SUBTLE))
                                    .text_color(rgb(INK_MUTED))
                            })
                            .when(!self.universe_syncing, |d| {
                                d.bg(rgb(CTA_BG))
                                    .border_color(rgb(CTA_BG))
                                    .hover(|s| s.bg(rgb(CTA_BG_HOVER)))
                                    .active(|s| s.bg(rgb(CTA_BG_ACTIVE)))
                                    .text_color(rgb(CTA_INK))
                            })
                            .font_family(self.fonts.ui.clone())
                            .text_xs()
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.start_universe_sync(false, cx);
                            }))
                            .child(if self.universe_syncing {
                                "L0 同步中…"
                            } else {
                                "同步全美股 L0"
                            }),
                    ),
            )
    }

    fn render_universe_filter_strip(&self, cx: &mut Context<Self>) -> gpui::Div {
        let search_border = if self.universe_state.search_focused {
            rgba(SKY_BORDER)
        } else {
            rgb(BORDER_PILL)
        };
        let search_placeholder = match self.universe_state.search_mode {
            UniverseSearchMode::Keyword => "/ 搜索 Ticker / 公司名 / SIC…",
            UniverseSearchMode::Similar => "/ 输入业务描述或 Ticker 找相似…",
        };
        let search_text = if self.universe_state.query.is_empty() {
            if self.universe_state.search_focused {
                "_".to_owned()
            } else {
                search_placeholder.to_owned()
            }
        } else if self.universe_state.search_focused {
            format!("{}_", self.universe_state.query)
        } else {
            self.universe_state.query.clone()
        };

        div()
            .flex_shrink_0()
            .px_5()
            .py_2()
            .bg(rgb(BG_ROW_ALT))
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
                    .gap_1p5()
                    .children(UniversePreset::all().into_iter().map(|preset| {
                        let active = self.universe_state.preset == preset;
                        div()
                            .id(gpui::SharedString::from(format!(
                                "univ-preset-{}",
                                preset.label()
                            )))
                            .px_2p5()
                            .py_1()
                            .rounded_xs()
                            .cursor_pointer()
                            .when(active, |d| {
                                d.bg(rgba(SKY_BG)).border_1().border_color(rgba(SKY_BORDER))
                            })
                            .when(!active, |d| {
                                d.bg(rgb(BG_SURFACE))
                                    .border_1()
                                    .border_color(rgb(BORDER_SUBTLE))
                                    .hover(|s| s.bg(rgb(BG_HOVER)))
                            })
                            .font_family(self.fonts.ui.clone())
                            .text_size(px(11.))
                            .font_weight(if active {
                                gpui::FontWeight::SEMIBOLD
                            } else {
                                gpui::FontWeight::NORMAL
                            })
                            .text_color(rgb(if active { SKY_FG } else { INK_SECONDARY }))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.universe_state.apply_preset(preset);
                                this.reload_universe_from_lake();
                                cx.notify();
                            }))
                            .child(preset.label())
                    }))
                    .child(v_divider())
                    .child(Self::render_toggle_chip(
                        "univ-toggle-fin",
                        "剔除金融业",
                        self.universe_state.exclude_financials,
                        true,
                        &self.fonts,
                        cx.listener(|this, _, _, cx| {
                            this.universe_state.toggle_exclude_financials();
                            this.reload_universe_from_lake();
                            cx.notify();
                        }),
                    ))
                    .child(Self::render_toggle_chip(
                        "univ-toggle-5y",
                        "5Y+ 完整财报",
                        self.universe_state.require_5y_history,
                        true,
                        &self.fonts,
                        cx.listener(|this, _, _, cx| {
                            this.universe_state.toggle_require_5y_history();
                            this.reload_universe_from_lake();
                            cx.notify();
                        }),
                    ))
                    .child(Self::render_toggle_chip(
                        "univ-toggle-priced",
                        "仅看报价池",
                        self.universe_state.in_price_pool_only,
                        false,
                        &self.fonts,
                        cx.listener(|this, _, _, cx| {
                            this.universe_state.toggle_in_price_pool_only();
                            this.reload_universe_from_lake();
                            cx.notify();
                        }),
                    )),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        seg_group()
                            .child(seg_tab(
                                "univ-mode-keyword",
                                UniverseSearchMode::Keyword.label(),
                                self.universe_state.search_mode == UniverseSearchMode::Keyword,
                                Some((SKY_FG, SKY_BG)),
                                false,
                                &self.fonts,
                                cx.listener(|this, _, _, cx| {
                                    this.universe_state
                                        .set_search_mode(UniverseSearchMode::Keyword);
                                    this.reload_universe_from_lake();
                                    cx.notify();
                                }),
                            ))
                            .child(seg_tab(
                                "univ-mode-similar",
                                UniverseSearchMode::Similar.label(),
                                self.universe_state.search_mode == UniverseSearchMode::Similar,
                                Some((VIOLET_FG, VIOLET_BG)),
                                false,
                                &self.fonts,
                                cx.listener(|this, _, _, cx| {
                                    this.universe_state
                                        .set_search_mode(UniverseSearchMode::Similar);
                                    this.universe_state.similar_as_ticker = false;
                                    this.universe_state.search_focused = true;
                                    this.reload_universe_from_lake();
                                    cx.notify();
                                }),
                            )),
                    )
                    .child(
                        div()
                            .id("univ-search-input")
                            .w(px(230.))
                            .px_2p5()
                            .py_1()
                            .rounded_sm()
                            .bg(rgb(BG_VIEWPORT))
                            .border_1()
                            .border_color(search_border)
                            .cursor_pointer()
                            .flex()
                            .items_center()
                            .justify_between()
                            .gap_2()
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.universe_state.search_focused = true;
                                cx.notify();
                            }))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .font_family(self.fonts.mono_term.clone())
                                    .text_size(px(11.))
                                    .text_color(rgb(if self.universe_state.query.is_empty() {
                                        INK_MUTED
                                    } else {
                                        INK_HERO
                                    }))
                                    .truncate()
                                    .child(search_text),
                            )
                            .when(!self.universe_state.query.is_empty(), |d| {
                                d.child(
                                    div()
                                        .id("univ-search-clear")
                                        .px_1()
                                        .cursor_pointer()
                                        .font_family(self.fonts.mono_term.clone())
                                        .text_xs()
                                        .text_color(rgb(INK_MUTED))
                                        .hover(|s| s.text_color(rgb(INK_HERO)))
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.universe_state.query.clear();
                                            this.universe_state.search_focused = false;
                                            this.apply_universe_query_change();
                                            cx.stop_propagation();
                                            cx.notify();
                                        }))
                                        .child("×"),
                                )
                            }),
                    )
                    .when(!self.universe_state.is_default(), |d| {
                        d.child(
                            div()
                                .id("univ-reset-filters")
                                .px_2()
                                .py_1()
                                .rounded_xs()
                                .bg(rgba(GOLD_BG))
                                .border_1()
                                .border_color(rgba(GOLD_BORDER))
                                .cursor_pointer()
                                .font_family(self.fonts.ui.clone())
                                .text_size(px(11.))
                                .text_color(rgb(GOLD_FG))
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.universe_state.reset();
                                    this.reload_universe_from_lake();
                                    cx.notify();
                                }))
                                .child("× 重置"),
                        )
                    }),
            )
    }

    fn render_toggle_chip(
        id: &'static str,
        label: &'static str,
        active: bool,
        emerald_accent: bool,
        fonts: &FontRoles,
        on_click: impl Fn(&gpui::ClickEvent, &mut gpui::Window, &mut gpui::App) + 'static,
    ) -> gpui::Stateful<gpui::Div> {
        let (fg, bg, br) = if emerald_accent {
            (EMERALD_FG, EMERALD_BG, EMERALD_BORDER)
        } else {
            (INK_HERO, BG_HOVER, BORDER_PILL)
        };
        div()
            .id(id)
            .px_2()
            .py_1()
            .rounded_xs()
            .cursor_pointer()
            .when(active, |d| {
                if emerald_accent {
                    d.bg(rgba(bg)).border_1().border_color(rgba(br))
                } else {
                    d.bg(rgb(bg)).border_1().border_color(rgb(br))
                }
            })
            .when(!active, |d| {
                d.bg(rgb(BG_SURFACE))
                    .border_1()
                    .border_color(rgb(BORDER_SUBTLE))
            })
            .hover(|s| s.bg(rgb(BG_HOVER)))
            .flex()
            .items_center()
            .gap_1p5()
            .on_click(on_click)
            .when(emerald_accent, |d| {
                d.child(div().w(px(5.)).h(px(5.)).rounded_full().bg(rgb(if active {
                    fg
                } else {
                    INK_MUTED
                })))
            })
            .child(
                div()
                    .font_family(fonts.ui.clone())
                    .text_size(px(11.))
                    .text_color(rgb(if active { fg } else { INK_MUTED }))
                    .child(label),
            )
    }

    fn render_universe_empty_state(&self, cx: &mut Context<Self>) -> gpui::Div {
        let sync_cli = "./apps/m3-desktop/m3 universe sync --full --prices-top 400";

        card_shell().p_6().flex().flex_col().gap_4().child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .pb_3()
                .border_b_1()
                .border_color(rgb(BORDER_HAIRLINE))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2p5()
                        .child(div().w(px(7.)).h(px(7.)).rounded_full().bg(rgb(SKY_FG)))
                        .child(
                            div()
                                .font_family(self.fonts.ui.clone())
                                .text_sm()
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .text_color(rgb(INK_HERO))
                                .child("全美股 L0 确定性湖仓尚未同步"),
                        ),
                )
                .child(badge(
                    "MILESTONE A · ZERO-LLM L0".into(),
                    SKY_FG,
                    SKY_BG,
                    SKY_BORDER,
                    &self.fonts,
                )),
        )
        .child(
            div()
                .font_family(self.fonts.ui.clone())
                .text_size(px(13.))
                .line_height(px(22.))
                .text_color(rgb(INK_SECONDARY))
                .child("点击「同步全美股 L0」或在终端运行 CLI，即可从 SEC EDGAR 官方全量归档（company_tickers_exchange.json + companyfacts.zip）拉取美股财报，使用与 m3 report 完全一致的 rust_decimal 引擎并行计算 5,000+ 公司护城河指标，并按价格方案 1 仅对 Top 400 质量候选池拉取免费日级股价。"),
        )
        .when_some(self.universe_sync_progress.clone(), |d, msg| {
            d.child(
                div()
                    .px_3()
                    .py_2()
                    .rounded_md()
                    .bg(rgba(SKY_BG))
                    .border_1()
                    .border_color(rgba(SKY_BORDER))
                    .font_family(self.fonts.mono_term.clone())
                    .text_xs()
                    .text_color(rgb(SKY_FG))
                    .child(format!("同步进度：{msg}")),
            )
        })
        .child(
            div()
                .flex()
                .items_center()
                .flex_wrap()
                .gap_3()
                .child(
                    div()
                        .id("univ-empty-sync-btn")
                        .px_4()
                        .py_2()
                        .rounded_md()
                        .cursor_pointer()
                        .when(self.universe_syncing, |d| {
                            d.bg(rgb(BG_WELL))
                                .border_1()
                                .border_color(rgb(BORDER_SUBTLE))
                                .text_color(rgb(INK_MUTED))
                        })
                        .when(!self.universe_syncing, |d| {
                            d.bg(rgb(CTA_BG))
                                .hover(|s| s.bg(rgb(CTA_BG_HOVER)))
                                .active(|s| s.bg(rgb(CTA_BG_ACTIVE)))
                                .text_color(rgb(CTA_INK))
                        })
                        .font_family(self.fonts.ui.clone())
                        .text_xs()
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.start_universe_sync(true, cx);
                        }))
                        .child(if self.universe_syncing {
                            "正在后台同步 SEC 湖仓…"
                        } else {
                            "立即同步全美股 L0"
                        }),
                )
                .child(
                    div()
                        .id("univ-empty-copy-cli")
                        .px_3()
                        .py_2()
                        .rounded_md()
                        .bg(rgb(BG_WELL))
                        .border_1()
                        .border_color(rgb(BORDER_PILL))
                        .hover(|s| s.bg(rgb(BG_HOVER)))
                        .cursor_pointer()
                        .flex()
                        .items_center()
                        .gap_2()
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.copy_cli_command(sync_cli.into(), "universe sync", cx);
                            cx.notify();
                        }))
                        .child(
                            div()
                                .font_family(self.fonts.mono_term.clone())
                                .text_xs()
                                .text_color(rgb(INK_PRIMARY))
                                .child(sync_cli),
                        )
                        .child(kbd("复制", &self.fonts)),
                ),
        )
    }

    fn render_universe_no_matches(&self, cx: &mut Context<Self>) -> gpui::Div {
        let hint = if self.universe_state.require_5y_history {
            "当前芒格滤镜（含 5Y+ 完整财报）或搜索词下无匹配公司。可关闭「5Y+ 完整财报」或重置。"
        } else {
            "当前芒格滤镜或搜索词下无匹配公司。可放宽筛选条件或重置。"
        };
        card_shell()
            .p_5()
            .flex()
            .items_center()
            .justify_between()
            .gap_4()
            .child(
                div()
                    .font_family(self.fonts.ui.clone())
                    .text_xs()
                    .text_color(rgb(INK_SECONDARY))
                    .child(hint),
            )
            .child(
                div()
                    .id("univ-empty-reset-btn")
                    .px_3()
                    .py_1p5()
                    .rounded_sm()
                    .bg(rgb(BG_WELL))
                    .border_1()
                    .border_color(rgb(BORDER_PILL))
                    .hover(|s| s.bg(rgb(BG_HOVER)))
                    .cursor_pointer()
                    .font_family(self.fonts.ui.clone())
                    .text_xs()
                    .text_color(rgb(INK_HERO))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.universe_state.reset();
                        this.reload_universe_from_lake();
                        cx.notify();
                    }))
                    .child("重置全部滤镜"),
            )
    }

    fn render_universe_scatter_card(&self, selected_row: Option<&UniverseRow>) -> gpui::Div {
        let model = build_scatter_model(
            &self.universe_rows,
            &self.universe_strike_zone_ciks,
            selected_row.map(|r| r.cik.as_str()),
        );

        let mut card = card_shell().p_3p5().flex().flex_col().gap_2p5().child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .gap_3()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2p5()
                        .child(
                            div()
                                .font_family(self.fonts.ui.clone())
                                .text_xs()
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .text_color(rgb(INK_HERO))
                                .child("ROIC × 严苛 FCF 收益率散点矩阵"),
                        )
                        .when_some(model.as_ref(), |d, m| {
                            let clip_note = if m.clipped_count > 0 {
                                format!(" · {} 家极端值贴边", m.clipped_count)
                            } else {
                                String::new()
                            };
                            d.child(
                                div()
                                    .font_family(self.fonts.mono_num.clone())
                                    .text_size(px(11.))
                                    .text_color(rgb(INK_MUTED))
                                    .child(format!("含报价 {} 家{clip_note}", m.points.len())),
                            )
                        }),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_3()
                        .when_some(selected_row, |d, r| {
                            let roic_s = fmt_pct_clean(r.roic_5y_avg_pct, false).0;
                            let fcf_s = fmt_pct_clean(r.strict_fcf_yield_pct, false).0;
                            d.child(
                                div()
                                    .px_2()
                                    .py_0p5()
                                    .rounded_xs()
                                    .bg(rgb(BG_WELL))
                                    .border_1()
                                    .border_color(rgb(BORDER_PILL))
                                    .font_family(self.fonts.mono_num.clone())
                                    .text_size(px(10.))
                                    .text_color(rgb(INK_HERO))
                                    .child(format!(
                                        "选中 {} · ROIC {roic_s} · FCF% {fcf_s}",
                                        r.ticker
                                    )),
                            )
                        })
                        .child(Self::scatter_legend_dot(EMERALD_FG, "击球区", &self.fonts))
                        .child(Self::scatter_legend_dot(SKY_FG, "观察池", &self.fonts))
                        .child(Self::scatter_legend_dot(GOLD_FG, "候选池", &self.fonts)),
                ),
        );

        let Some(m) = model else {
            return card.child(
                div()
                    .h(px(72.))
                    .rounded_md()
                    .bg(rgb(BG_VIEWPORT))
                    .border_1()
                    .border_color(rgb(BORDER_HAIRLINE))
                    .flex()
                    .items_center()
                    .justify_center()
                    .font_family(self.fonts.ui.clone())
                    .text_xs()
                    .text_color(rgb(INK_MUTED))
                    .child("当前筛选结果暂无同时具备 5Y ROIC 与日级报价的标的（可切换筛选或同步 Top 400 报价池）。"),
            );
        };

        let y_ticks = [
            format!("{:.0}%", m.y_max),
            format!("{:.0}%", m.y_min + (m.y_max - m.y_min) * 0.5),
            format!("{:.0}%", m.y_min),
        ];
        let x_ticks = [
            format!("ROIC {:.0}%", m.x_min),
            format!("{:.0}%", (m.x_min + m.x_max) * 0.5),
            format!("{:.0}%", m.x_max),
        ];

        card = card
            .child(
                div()
                    .flex()
                    .gap_2()
                    .child(
                        div()
                            .w(px(36.))
                            .h(px(164.))
                            .flex_shrink_0()
                            .py_1()
                            .flex()
                            .flex_col()
                            .justify_between()
                            .items_end()
                            .children(y_ticks.into_iter().map(|lbl| {
                                div()
                                    .font_family(self.fonts.mono_num.clone())
                                    .text_size(px(10.))
                                    .text_color(rgb(INK_MUTED))
                                    .child(lbl)
                            })),
                    )
                    .child(Self::render_universe_scatter_canvas(m)),
            )
            .child(
                div()
                    .pl(px(44.))
                    .flex()
                    .items_center()
                    .justify_between()
                    .children(x_ticks.into_iter().map(|lbl| {
                        div()
                            .font_family(self.fonts.mono_num.clone())
                            .text_size(px(10.))
                            .text_color(rgb(INK_MUTED))
                            .child(lbl)
                    })),
            );

        card
    }

    fn render_universe_scatter_canvas(plot_model: ScatterPlotModel) -> gpui::Div {
        div()
            .relative()
            .flex_1()
            .h(px(164.))
            .rounded_md()
            .bg(rgb(BG_VIEWPORT))
            .border_1()
            .border_color(rgb(BORDER_HAIRLINE))
            .overflow_hidden()
            .child(
                canvas(
                    |_, _, _| (),
                    move |b, _, window, _| {
                        let ox = f32::from(b.origin.x);
                        let oy = f32::from(b.origin.y);
                        let w = f32::from(b.size.width);
                        let h = f32::from(b.size.height);
                        if w <= 24.0 || h <= 24.0 {
                            return;
                        }
                        let pad_x = 10.0f32;
                        let pad_y = 10.0f32;
                        let pw = (w - pad_x * 2.0).max(8.0);
                        let ph = (h - pad_y * 2.0).max(8.0);
                        let to_px = |xn: f32, yn: f32| -> (f32, f32) {
                            (ox + pad_x + xn * pw, oy + pad_y + (1.0 - yn) * ph)
                        };

                        for frac in [0.0f32, 0.5, 1.0] {
                            let (_, gy) = to_px(0.0, frac);
                            window.paint_quad(fill(
                                bounds(point(px(ox), px(gy)), size(px(w), px(1.))),
                                rgb(BORDER_HAIRLINE),
                            ));
                            let (gx, _) = to_px(frac, 0.0);
                            window.paint_quad(fill(
                                bounds(point(px(gx), px(oy)), size(px(1.), px(h))),
                                rgb(BORDER_HAIRLINE),
                            ));
                        }

                        if let (Some(rx), Some(ry)) =
                            (plot_model.ref_roic_norm, plot_model.ref_fcf_norm)
                        {
                            let (x0, y0) = to_px(rx, 1.0);
                            let (x1, y1) = to_px(1.0, ry);
                            window.paint_quad(fill(
                                bounds(
                                    point(px(x0), px(y0)),
                                    size(px((x1 - x0).max(1.0)), px((y1 - y0).max(1.0))),
                                ),
                                rgba(EMERALD_BAND),
                            ));
                        }

                        if let Some(rx) = plot_model.ref_roic_norm {
                            let (gx, _) = to_px(rx, 0.0);
                            let mut dy = oy;
                            while dy < oy + h {
                                let seg = (oy + h - dy).min(4.0);
                                window.paint_quad(fill(
                                    bounds(point(px(gx - 0.5), px(dy)), size(px(1.0), px(seg))),
                                    rgba(GOLD_BORDER),
                                ));
                                dy += 8.0;
                            }
                        }
                        if let Some(ry) = plot_model.ref_fcf_norm {
                            let (_, gy) = to_px(0.0, ry);
                            let mut dx = ox;
                            while dx < ox + w {
                                let seg = (ox + w - dx).min(4.0);
                                window.paint_quad(fill(
                                    bounds(point(px(dx), px(gy - 0.5)), size(px(seg), px(1.0))),
                                    rgba(EMERALD_BORDER),
                                ));
                                dx += 8.0;
                            }
                        }
                        if let Some(zy) = plot_model.zero_fcf_norm {
                            let (_, gy) = to_px(0.0, zy);
                            window.paint_quad(fill(
                                bounds(point(px(ox), px(gy)), size(px(w), px(1.0))),
                                rgb(BORDER_SUBTLE),
                            ));
                        }

                        let dot_color = |pt: &ScatterPoint| -> u32 {
                            if pt.in_strike_zone {
                                EMERALD_FG
                            } else if pt.in_watchlist {
                                SKY_FG
                            } else {
                                GOLD_FG
                            }
                        };
                        for pt in &plot_model.points {
                            let (cx_p, cy_p) = to_px(pt.x_norm, pt.y_norm);
                            let col = dot_color(pt);
                            if pt.is_selected {
                                window.paint_quad(fill(
                                    bounds(point(px(ox), px(cy_p - 0.5)), size(px(w), px(1.0))),
                                    rgba(0xFFFFFF22),
                                ));
                                window.paint_quad(fill(
                                    bounds(point(px(cx_p - 0.5), px(oy)), size(px(1.0), px(h))),
                                    rgba(0xFFFFFF22),
                                ));
                                window.paint_quad(fill(
                                    bounds(
                                        point(px(cx_p - 5.5), px(cy_p - 5.5)),
                                        size(px(11.0), px(11.0)),
                                    ),
                                    rgb(INK_HERO),
                                ));
                                window.paint_quad(fill(
                                    bounds(
                                        point(px(cx_p - 3.5), px(cy_p - 3.5)),
                                        size(px(7.0), px(7.0)),
                                    ),
                                    rgb(col),
                                ));
                            } else {
                                let r = if pt.in_strike_zone || pt.in_watchlist {
                                    3.2f32
                                } else {
                                    2.4f32
                                };
                                window.paint_quad(fill(
                                    bounds(
                                        point(px(cx_p - r), px(cy_p - r)),
                                        size(px(r * 2.0), px(r * 2.0)),
                                    ),
                                    rgb(col),
                                ));
                            }
                        }
                    },
                )
                .size_full(),
            )
    }

    fn scatter_legend_dot(color: u32, label: &str, fonts: &FontRoles) -> gpui::Div {
        div()
            .flex()
            .items_center()
            .gap_1()
            .child(div().w(px(6.)).h(px(6.)).rounded_xs().bg(rgb(color)))
            .child(
                div()
                    .font_family(fonts.ui.clone())
                    .text_size(px(10.))
                    .text_color(rgb(INK_MUTED))
                    .child(label.to_owned()),
            )
    }

    fn render_universe_table(
        &self,
        selected_cik: Option<&str>,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        let headers: &[(&str, f32, bool, UniverseSortBy)] = &[
            ("标的 / 交易所·SIC", 0., false, UniverseSortBy::Ticker),
            ("5Y ROIC", 58., true, UniverseSortBy::Roic5yAvg),
            ("严苛 FCF%", 64., true, UniverseSortBy::StrictFcfYield),
            ("3Y 股本Δ", 62., true, UniverseSortBy::ShareChange3y),
            ("SBC/OCF", 58., true, UniverseSortBy::SbcToOcf),
            ("现价", 62., true, UniverseSortBy::LatestPrice),
            ("击球价", 62., true, UniverseSortBy::StrikePrice),
            ("距击球", 62., true, UniverseSortBy::DistanceToStrike),
            ("预警", 42., true, UniverseSortBy::WarningsCount),
        ];

        let quality_sorted = self.universe_state.sort_by == UniverseSortBy::QualityScore;
        let quality_arrow = if quality_sorted {
            if self.universe_state.sort_asc {
                " ↑"
            } else {
                " ↓"
            }
        } else {
            ""
        };

        let mut card = card_shell()
            .child(
                div()
                    .px_3()
                    .py_2()
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
                            .child(
                                div()
                                    .font_family(self.fonts.ui.clone())
                                    .text_xs()
                                    .font_weight(gpui::FontWeight::SEMIBOLD)
                                    .text_color(rgb(INK_HERO))
                                    .child(format!(
                                        "全市场筛选榜单 ({} 家)",
                                        self.universe_rows.len()
                                    )),
                            )
                            .child(
                                div()
                                    .font_family(self.fonts.mono_term.clone())
                                    .text_size(px(10.))
                                    .text_color(rgb(INK_MUTED))
                                    .child("点击列头排序 · 点击行右侧下钻 5Y 财报"),
                            ),
                    )
                    .child(
                        div()
                            .id("univ-sort-quality")
                            .px_2()
                            .py_0p5()
                            .rounded_xs()
                            .cursor_pointer()
                            .when(quality_sorted, |d| {
                                d.bg(rgba(SKY_BG)).border_1().border_color(rgba(SKY_BORDER))
                            })
                            .when(!quality_sorted, |d| {
                                d.bg(rgb(BG_SURFACE))
                                    .border_1()
                                    .border_color(rgb(BORDER_SUBTLE))
                            })
                            .hover(|s| s.bg(rgb(BG_HOVER)))
                            .font_family(self.fonts.mono_term.clone())
                            .text_size(px(10.))
                            .text_color(rgb(if quality_sorted { SKY_FG } else { INK_MUTED }))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.universe_state
                                    .toggle_sort(UniverseSortBy::QualityScore);
                                this.reload_universe_from_lake();
                                cx.notify();
                            }))
                            .child(format!("按质量分{quality_arrow}")),
                    ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .px_3()
                    .py_2()
                    .bg(rgb(BG_ROW_ALT))
                    .border_b_1()
                    .border_color(rgb(BORDER_HAIRLINE))
                    .children(headers.iter().map(|(label, w, right, sort_col)| {
                        let is_sorted = self.universe_state.sort_by == *sort_col;
                        let arrow = if is_sorted {
                            if self.universe_state.sort_asc {
                                " ↑"
                            } else {
                                " ↓"
                            }
                        } else {
                            ""
                        };
                        let col = *sort_col;
                        div()
                            .id(gpui::SharedString::from(format!("univ-hdr-{label}")))
                            .px_1()
                            .flex()
                            .items_center()
                            .when(*right, |d| d.justify_end())
                            .gap_0p5()
                            .when(*w > 0., |d| d.w(px(*w)).flex_shrink_0())
                            .when(*w == 0., |d| d.flex_1().min_w(px(120.)))
                            .cursor_pointer()
                            .hover(|s| s.text_color(rgb(INK_PRIMARY)))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.universe_state.toggle_sort(col);
                                this.reload_universe_from_lake();
                                cx.notify();
                            }))
                            .child(
                                div()
                                    .font_family(self.fonts.ui.clone())
                                    .text_size(px(11.))
                                    .font_weight(gpui::FontWeight::SEMIBOLD)
                                    .text_color(rgb(if is_sorted {
                                        INK_PRIMARY
                                    } else {
                                        INK_MUTED
                                    }))
                                    .truncate()
                                    .child(*label),
                            )
                            .when(!arrow.is_empty(), |d| {
                                d.child(
                                    div()
                                        .font_family(self.fonts.mono_term.clone())
                                        .text_size(px(10.))
                                        .text_color(rgb(INK_HERO))
                                        .child(arrow),
                                )
                            })
                    })),
            );

        for (idx, row) in self.universe_rows.iter().enumerate() {
            let is_selected = selected_cik.is_some_and(|sel| row.cik == sel);
            card = card.child(self.render_universe_table_row(idx, row, is_selected, cx));
        }

        card
    }

    fn render_universe_table_row(
        &self,
        idx: usize,
        row: &UniverseRow,
        is_selected: bool,
        cx: &mut Context<Self>,
    ) -> gpui::Stateful<gpui::Div> {
        let row_bg = if is_selected {
            BG_SELECTED_ROW
        } else if idx.is_multiple_of(2) {
            BG_VIEWPORT
        } else {
            BG_ROW_ALT
        };

        let (roic_str, _) = fmt_pct_clean(row.roic_5y_avg_pct, false);
        let roic_col = match row.roic_5y_avg_pct {
            Some(v) if v >= Decimal::from(20) => EMERALD_FG,
            Some(v) if v >= Decimal::from(15) => INK_PRIMARY,
            Some(_) => GOLD_FG,
            None => INK_MUTED,
        };
        let (fcf_str, fcf_pol) = fmt_pct_clean(row.strict_fcf_yield_pct, false);
        let fcf_col = match row.strict_fcf_yield_pct {
            Some(_) if fcf_pol < 0 => CORAL_FG,
            Some(v) if v >= Decimal::from(5) => EMERALD_FG,
            Some(_) => INK_PRIMARY,
            None => INK_MUTED,
        };
        let (share_str, share_pol) = fmt_pct_clean(row.share_change_3y_pct, true);
        let share_col = share_delta_color(row.share_change_3y_pct, share_pol);
        let (sbc_str, _) = fmt_pct_clean(row.sbc_to_ocf_pct, false);
        let sbc_col = match row.sbc_to_ocf_pct {
            Some(v) if v > Decimal::from(25) => GOLD_FG,
            Some(v) if v <= Decimal::from(10) => EMERALD_FG,
            Some(_) => INK_SECONDARY,
            None => INK_MUTED,
        };
        let (dist_str, _) = fmt_pct_clean(row.distance_to_strike_pct, true);
        let dist_col = universe_distance_color(row, &self.universe_strike_zone_ciks);

        let warn_col = if row.warnings_count > 0 {
            let has_red = row
                .parsed_warnings()
                .iter()
                .any(|w| w.severity == WarningSeverity::Red);
            if has_red { CORAL_FG } else { GOLD_FG }
        } else {
            INK_MUTED
        };

        let live_bkt = self.live_watchlist_bucket(row);
        let row_for_click = row.clone();

        div()
            .id(gpui::SharedString::from(format!("univ-row-{}", row.cik)))
            .relative()
            .flex()
            .items_center()
            .px_3()
            .py_2()
            .bg(rgb(row_bg))
            .border_b_1()
            .border_color(rgb(BORDER_HAIRLINE))
            .cursor_pointer()
            .hover(|s| s.bg(rgb(BG_HOVER)))
            .active(|s| s.bg(rgb(BG_ACTIVE)))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.select_universe_row(&row_for_click);
                cx.notify();
            }))
            .when(is_selected, |d| {
                d.child(
                    div()
                        .absolute()
                        .left_0()
                        .top_0()
                        .bottom_0()
                        .w(px(2.))
                        .bg(rgb(SKY_FG)),
                )
            })
            .child(
                div()
                    .flex_1()
                    .min_w(px(120.))
                    .px_1()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_1p5()
                            .min_w_0()
                            .child(
                                div()
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
                                    .text_color(rgb(INK_PRIMARY))
                                    .truncate()
                                    .child(row.name.clone()),
                            )
                            .when_some(live_bkt, |d, bkt| {
                                let (fg, bg, br) = bucket_colors(bkt);
                                d.child(badge(bkt.code().to_owned(), fg, bg, br, &self.fonts))
                            })
                            .when(row.is_financial, |d| {
                                d.child(badge(
                                    "FIN".into(),
                                    ORANGE_FG,
                                    ORANGE_BG,
                                    ORANGE_BORDER,
                                    &self.fonts,
                                ))
                            })
                            .when(row.sic.is_none(), |d| {
                                d.child(badge(
                                    "未分类".into(),
                                    SLATE_FG,
                                    SLATE_BG,
                                    SLATE_BORDER,
                                    &self.fonts,
                                ))
                            }),
                    )
                    .child(
                        div()
                            .mt_0p5()
                            .font_family(self.fonts.mono_term.clone())
                            .text_size(px(10.))
                            .text_color(rgb(INK_MUTED))
                            .truncate()
                            .child(format_sic_line(row)),
                    ),
            )
            .child(Self::table_num_cell(roic_str, 58., roic_col, &self.fonts))
            .child(Self::table_num_cell(fcf_str, 64., fcf_col, &self.fonts))
            .child(Self::table_num_cell(share_str, 62., share_col, &self.fonts))
            .child(Self::table_num_cell(sbc_str, 58., sbc_col, &self.fonts))
            .child(Self::table_num_cell(
                fmt_price(row.latest_price, "USD"),
                62.,
                if row.latest_price.is_some() {
                    INK_HERO
                } else {
                    INK_MUTED
                },
                &self.fonts,
            ))
            .child(Self::table_num_cell(
                fmt_price(row.strike_price, "USD"),
                62.,
                if row.strike_price.is_some() {
                    INK_SECONDARY
                } else {
                    INK_MUTED
                },
                &self.fonts,
            ))
            .child(Self::table_num_cell(dist_str, 62., dist_col, &self.fonts))
            .child(Self::table_num_cell(
                if row.warnings_count == 0 {
                    "0".into()
                } else {
                    format!("{}", row.warnings_count)
                },
                42.,
                warn_col,
                &self.fonts,
            ))
    }

    fn table_num_cell(text: String, width: f32, color: u32, fonts: &FontRoles) -> gpui::Div {
        div()
            .w(px(width))
            .flex_shrink_0()
            .px_1()
            .text_right()
            .font_family(fonts.mono_num.clone())
            .text_xs()
            .text_color(rgb(color))
            .truncate()
            .child(text)
    }

    fn render_universe_drawer(
        &self,
        row: &UniverseRow,
        cx: &mut Context<Self>,
    ) -> gpui::Stateful<gpui::Div> {
        let live_bkt = self.live_watchlist_bucket(row);
        let ticker_dossier = row.ticker.clone();
        let ticker_alan = row.ticker.clone();
        let ticker_similar = row.ticker.clone();
        let row_for_add = row.clone();
        let warnings = row.parsed_warnings();

        let roic_txt = match (row.roic_5y_avg_pct, row.roic_5y_stddev_pct) {
            (Some(avg), Some(sd)) => format!("{avg:.1}% ± {sd:.1}%"),
            (Some(avg), None) => format!("{avg:.1}%"),
            _ => "—".into(),
        };
        let gm_txt = match (row.gross_margin_5y_avg_pct, row.gross_margin_5y_stddev_pct) {
            (Some(avg), Some(sd)) => format!("{avg:.1}% ± {sd:.1}%"),
            (Some(avg), None) => format!("{avg:.1}%"),
            _ => "—".into(),
        };
        let (fcf_txt, fcf_pol) = fmt_pct_clean(row.strict_fcf_yield_pct, false);
        let (share_txt, share_pol) = fmt_pct_clean(row.share_change_3y_pct, true);
        let (sbc_txt, _) = fmt_pct_clean(row.sbc_to_ocf_pct, false);
        let (dist_txt, _) = fmt_pct_clean(row.distance_to_strike_pct, true);

        let mut drawer = div()
            .id(gpui::SharedString::from(format!(
                "universe-inspector-{}",
                row.cik
            )))
            .w(px(304.))
            .flex_shrink_0()
            .h_full()
            .overflow_y_scroll()
            .bg(rgb(0x111118))
            .border_l_1()
            .border_color(rgb(BORDER_HAIRLINE))
            .p_4()
            .flex()
            .flex_col()
            .gap_3p5()
            .child(
                div()
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
                                    .font_family(self.fonts.mono_num.clone())
                                    .text_lg()
                                    .font_weight(gpui::FontWeight::BOLD)
                                    .text_color(rgb(INK_HERO))
                                    .child(row.ticker.clone()),
                            )
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_1p5()
                                    .when(row.is_financial, |d| {
                                        d.child(badge(
                                            "金融业口径预警".into(),
                                            ORANGE_FG,
                                            ORANGE_BG,
                                            ORANGE_BORDER,
                                            &self.fonts,
                                        ))
                                    })
                                    .when(row.sic.is_none(), |d| {
                                        d.child(badge(
                                            "未分类SIC".into(),
                                            SLATE_FG,
                                            SLATE_BG,
                                            SLATE_BORDER,
                                            &self.fonts,
                                        ))
                                    })
                                    .when_some(live_bkt, |d, bkt| {
                                        let (fg, bg, br) = bucket_colors(bkt);
                                        d.child(badge(
                                            format!("已入池 · {}", bkt.code()),
                                            fg,
                                            bg,
                                            br,
                                            &self.fonts,
                                        ))
                                    }),
                            ),
                    )
                    .child(
                        div()
                            .font_family(self.fonts.ui.clone())
                            .text_xs()
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .text_color(rgb(INK_PRIMARY))
                            .child(row.name.clone()),
                    )
                    .child(
                        div()
                            .font_family(self.fonts.mono_term.clone())
                            .text_size(px(10.))
                            .text_color(rgb(INK_MUTED))
                            .child(format_sic_line(row)),
                    ),
            )
            .child(if live_bkt.is_some() {
                div()
                    .flex()
                    .gap_2()
                    .child(
                        div()
                            .id(gpui::SharedString::from(format!(
                                "univ-open-dossier-{}",
                                row.cik
                            )))
                            .flex_1()
                            .py_2()
                            .rounded_md()
                            .bg(rgb(CTA_BG))
                            .hover(|s| s.bg(rgb(CTA_BG_HOVER)))
                            .active(|s| s.bg(rgb(CTA_BG_ACTIVE)))
                            .cursor_pointer()
                            .text_center()
                            .font_family(self.fonts.ui.clone())
                            .text_xs()
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .text_color(rgb(CTA_INK))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.select_company(&ticker_dossier, Some(NavView::Dossier));
                                cx.notify();
                            }))
                            .child("打开研报 Dossier →"),
                    )
                    .child(
                        div()
                            .id(gpui::SharedString::from(format!(
                                "univ-open-alan-{}",
                                row.cik
                            )))
                            .px_3()
                            .py_2()
                            .rounded_md()
                            .bg(rgba(VIOLET_BG))
                            .border_1()
                            .border_color(rgba(VIOLET_BORDER))
                            .hover(|s| s.bg(rgb(BG_HOVER)))
                            .cursor_pointer()
                            .font_family(self.fonts.ui.clone())
                            .text_xs()
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .text_color(rgb(VIOLET_FG))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.select_company(&ticker_alan, Some(NavView::Agent));
                                cx.notify();
                            }))
                            .child("Alan 审讯"),
                    )
            } else {
                div().child(
                    div()
                        .id(gpui::SharedString::from(format!(
                            "univ-add-watch-{}",
                            row.cik
                        )))
                        .w_full()
                        .py_2()
                        .rounded_md()
                        .bg(rgb(CTA_BG))
                        .hover(|s| s.bg(rgb(CTA_BG_HOVER)))
                        .active(|s| s.bg(rgb(CTA_BG_ACTIVE)))
                        .cursor_pointer()
                        .text_center()
                        .font_family(self.fonts.ui.clone())
                        .text_xs()
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(rgb(CTA_INK))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.add_universe_company_to_watch(&row_for_add, cx);
                        }))
                        .child("加入观察池 (WATCH)"),
                )
            })
            .child(
                div()
                    .id(gpui::SharedString::from(format!(
                        "univ-find-similar-{}",
                        row.cik
                    )))
                    .w_full()
                    .py_1p5()
                    .rounded_md()
                    .bg(rgba(VIOLET_BG))
                    .border_1()
                    .border_color(rgba(VIOLET_BORDER))
                    .hover(|s| s.bg(rgb(BG_HOVER)))
                    .active(|s| s.bg(rgb(BG_ACTIVE)))
                    .cursor_pointer()
                    .text_center()
                    .font_family(self.fonts.ui.clone())
                    .text_xs()
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .text_color(rgb(VIOLET_FG))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.find_similar_for_company(&ticker_similar, cx);
                    }))
                    .child("找相似公司 · Lookalike"),
            )
            .child(
                prop_section("5Y DETERMINISTIC SNAPSHOT", &self.fonts)
                    .child(kv_line(
                        "现价 / 击球价",
                        format!(
                            "{} / {}",
                            fmt_price(row.latest_price, "USD"),
                            fmt_price(row.strike_price, "USD")
                        ),
                        INK_HERO,
                        &self.fonts,
                    ))
                    .child(kv_line(
                        "距击球价",
                        dist_txt,
                        universe_distance_color(row, &self.universe_strike_zone_ciks),
                        &self.fonts,
                    ))
                    .child(kv_line(
                        "严苛 FCF 收益率",
                        fcf_txt,
                        if fcf_pol < 0 {
                            CORAL_FG
                        } else if row.strict_fcf_yield_pct.is_some() {
                            INK_PRIMARY
                        } else {
                            INK_MUTED
                        },
                        &self.fonts,
                    ))
                    .child(kv_line(
                        "最新严苛 OE",
                        row.strict_owner_earnings
                            .map(|oe| fmt_money_compact(oe, "USD"))
                            .unwrap_or_else(|| "—".into()),
                        match row.strict_owner_earnings {
                            Some(oe) if oe > Decimal::ZERO => EMERALD_FG,
                            Some(_) => CORAL_FG,
                            None => INK_MUTED,
                        },
                        &self.fonts,
                    ))
                    .child(kv_line("5Y ROIC", roic_txt, INK_HERO, &self.fonts))
                    .child(kv_line("5Y 毛利率", gm_txt, INK_PRIMARY, &self.fonts))
                    .child(kv_line(
                        "3Y 股本变化",
                        share_txt,
                        share_delta_color(row.share_change_3y_pct, share_pol),
                        &self.fonts,
                    ))
                    .child(kv_line("SBC / OCF", sbc_txt, INK_SECONDARY, &self.fonts))
                    .child(kv_line(
                        "净负债 / 严苛 OE",
                        row.net_debt_to_oe
                            .map(|v| format!("{v:.2}x"))
                            .unwrap_or_else(|| "—".into()),
                        INK_SECONDARY,
                        &self.fonts,
                    ))
                    .child(kv_line(
                        "质量综合分",
                        format!("{:.1}", row.quality_score),
                        SKY_FG,
                        &self.fonts,
                    )),
            )
            .child(self.render_universe_annuals_table(&row.cik));

        if let Some(peers_sec) = self.render_universe_drawer_peers(&row.cik, cx) {
            drawer = drawer.child(peers_sec);
        }
        if !warnings.is_empty() {
            drawer = drawer.child(Self::render_universe_drawer_warnings(
                &warnings,
                &self.fonts,
            ));
        }
        drawer.child(self.render_universe_drawer_provenance(row, cx))
    }

    fn render_universe_drawer_peers(
        &self,
        target_cik: &str,
        cx: &mut Context<Self>,
    ) -> Option<gpui::Div> {
        let peers = self.universe_selected_peers.as_ref()?;
        if peers.target.cik != target_cik || peers.peer_count <= 1 {
            return None;
        }
        let roic_p = peers
            .roic_percentile
            .map(|p| format!("P{:.0}", p))
            .unwrap_or_else(|| "—".into());
        let gm_p = peers
            .gross_margin_percentile
            .map(|p| format!("P{:.0}", p))
            .unwrap_or_else(|| "—".into());
        let fcf_p = peers
            .strict_fcf_yield_percentile
            .map(|p| format!("P{:.0}", p))
            .unwrap_or_else(|| "—".into());

        Some(
            prop_section(&format!("SIC PEERS · N={}", peers.peer_count), &self.fonts)
                .child(kv_line(
                    "同业 ROIC / 毛利 / FCF 分位",
                    format!("{roic_p} / {gm_p} / {fcf_p}"),
                    SKY_FG,
                    &self.fonts,
                ))
                .when(!peers.top_peers.is_empty(), |d| {
                    d.child(div().flex().flex_wrap().gap_1().children(
                        peers.top_peers.iter().take(4).map(|p| {
                            let peer_row = p.clone();
                            div()
                                .id(gpui::SharedString::from(format!("univ-peer-{}", p.cik)))
                                .px_1p5()
                                .py_0p5()
                                .rounded_xs()
                                .bg(rgb(BG_WELL))
                                .border_1()
                                .border_color(rgb(BORDER_SUBTLE))
                                .hover(|s| s.bg(rgb(BG_HOVER)))
                                .cursor_pointer()
                                .font_family(self.fonts.mono_num.clone())
                                .text_size(px(10.))
                                .text_color(rgb(INK_SECONDARY))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.select_universe_row(&peer_row);
                                    cx.notify();
                                }))
                                .child(p.ticker.clone())
                        }),
                    ))
                }),
        )
    }

    fn render_universe_drawer_warnings(warnings: &[WarningFlag], fonts: &FontRoles) -> gpui::Div {
        prop_section(&format!("WARNINGS · {} 项预警", warnings.len()), fonts).children(
            warnings.iter().map(|w| {
                let (fg, bg, br) = match w.severity {
                    WarningSeverity::Red => (CORAL_FG, CORAL_BG, CORAL_BORDER),
                    WarningSeverity::Amber => (GOLD_FG, GOLD_BG, GOLD_BORDER),
                    WarningSeverity::Info => (SLATE_FG, SLATE_BG, SLATE_BORDER),
                };
                div()
                    .p_2()
                    .rounded_sm()
                    .bg(rgba(bg))
                    .border_1()
                    .border_color(rgba(br))
                    .flex()
                    .flex_col()
                    .gap_0p5()
                    .child(
                        div()
                            .font_family(fonts.ui.clone())
                            .text_size(px(11.))
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .text_color(rgb(fg))
                            .child(w.title.clone()),
                    )
                    .child(
                        div()
                            .font_family(fonts.ui.clone())
                            .text_size(px(10.))
                            .text_color(rgb(INK_PRIMARY))
                            .child(w.detail.clone()),
                    )
            }),
        )
    }

    fn render_universe_drawer_provenance(
        &self,
        row: &UniverseRow,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        let sha_short = row
            .raw_sha256
            .as_deref()
            .map(|s| &s[..s.len().min(16)])
            .unwrap_or("—");
        let peers_cli = format!("./apps/m3-desktop/m3 universe peers {}", row.ticker);

        prop_section("PROVENANCE · XBRL 溯源", &self.fonts)
            .child(kv_line("CIK", row.cik.clone(), INK_SECONDARY, &self.fonts))
            .child(kv_line(
                "最近财报",
                row.latest_filing_end.clone().unwrap_or_else(|| "—".into()),
                INK_SECONDARY,
                &self.fonts,
            ))
            .child(kv_line(
                "年报年数",
                format!("{} 年", row.years_of_data),
                INK_SECONDARY,
                &self.fonts,
            ))
            .child(kv_line(
                "RAW_SHA256",
                sha_short.to_owned(),
                INK_MUTED,
                &self.fonts,
            ))
            .child(
                div()
                    .id(gpui::SharedString::from(format!(
                        "univ-copy-peers-cli-{}",
                        row.cik
                    )))
                    .mt_1()
                    .px_2p5()
                    .py_1()
                    .rounded_sm()
                    .bg(rgb(BG_WELL))
                    .border_1()
                    .border_color(rgb(BORDER_PILL))
                    .hover(|s| s.bg(rgb(BG_HOVER)))
                    .cursor_pointer()
                    .font_family(self.fonts.mono_term.clone())
                    .text_size(px(10.))
                    .text_color(rgb(INK_SECONDARY))
                    .truncate()
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.copy_cli_command(peers_cli.clone(), "universe peers", cx);
                        cx.notify();
                    }))
                    .child(format!("复制 CLI: m3 universe peers {}", row.ticker)),
            )
    }

    fn render_universe_annuals_table(&self, target_cik: &str) -> gpui::Div {
        let mut sec = prop_section("ANNUAL METRICS · 5Y 底表", &self.fonts);
        let recent: Vec<&LakeAnnualRow> = self
            .universe_selected_annuals
            .iter()
            .filter(|a| a.cik == target_cik)
            .rev()
            .take(5)
            .collect();
        if recent.is_empty() {
            return sec.child(
                div()
                    .font_family(self.fonts.ui.clone())
                    .text_xs()
                    .text_color(rgb(INK_MUTED))
                    .child("暂无年度明细行"),
            );
        }
        let recent_len = recent.len();

        let header = div()
            .flex()
            .items_center()
            .py_1()
            .border_b_1()
            .border_color(rgb(BORDER_HAIRLINE))
            .font_family(self.fonts.mono_term.clone())
            .text_size(px(10.))
            .text_color(rgb(INK_MUTED))
            .child(div().w(px(48.)).flex_shrink_0().child("财年"))
            .child(div().w(px(54.)).flex_shrink_0().text_right().child("营收"))
            .child(div().w(px(46.)).flex_shrink_0().text_right().child("毛利%"))
            .child(div().w(px(46.)).flex_shrink_0().text_right().child("ROIC"))
            .child(div().flex_1().min_w_0().text_right().child("严苛OE"));

        let mut rows_box = div()
            .rounded_sm()
            .bg(rgb(BG_VIEWPORT))
            .border_1()
            .border_color(rgb(BORDER_HAIRLINE))
            .px_2()
            .py_1()
            .child(header);

        for (idx, a) in recent.into_iter().enumerate() {
            let gm = a
                .gross_margin_pct
                .map(|v| format!("{v:.0}%"))
                .unwrap_or_else(|| "—".into());
            let roic = a
                .roic_pct
                .map(|v| format!("{v:.0}%"))
                .unwrap_or_else(|| "—".into());
            let roic_col = match a.roic_pct {
                Some(v) if v >= Decimal::from(15) => EMERALD_FG,
                Some(_) => GOLD_FG,
                None => INK_MUTED,
            };
            let (oe_str, oe_col) = match a.strict_owner_earnings {
                Some(oe) => {
                    let col = if oe > Decimal::ZERO {
                        EMERALD_FG
                    } else {
                        CORAL_FG
                    };
                    (fmt_money_compact(oe, "USD"), col)
                }
                None => ("—".into(), INK_MUTED),
            };
            rows_box = rows_box.child(
                div()
                    .flex()
                    .items_center()
                    .py_1()
                    .when(idx + 1 < recent_len, |d| {
                        d.border_b_1().border_color(rgb(BORDER_HAIRLINE))
                    })
                    .font_family(self.fonts.mono_num.clone())
                    .text_size(px(10.))
                    .child(
                        div()
                            .w(px(48.))
                            .flex_shrink_0()
                            .text_color(rgb(INK_PRIMARY))
                            .child(a.label.clone()),
                    )
                    .child(
                        div()
                            .w(px(54.))
                            .flex_shrink_0()
                            .text_right()
                            .text_color(rgb(INK_SECONDARY))
                            .child(fmt_money_compact(a.revenue, "USD")),
                    )
                    .child(
                        div()
                            .w(px(46.))
                            .flex_shrink_0()
                            .text_right()
                            .text_color(rgb(INK_SECONDARY))
                            .child(gm),
                    )
                    .child(
                        div()
                            .w(px(46.))
                            .flex_shrink_0()
                            .text_right()
                            .text_color(rgb(roic_col))
                            .child(roic),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_right()
                            .text_color(rgb(oe_col))
                            .child(oe_str),
                    ),
            );
        }

        sec = sec.child(rows_box);
        sec
    }

    fn render_universe_similar_prompt(&self) -> gpui::Div {
        card_shell()
            .p_5()
            .flex()
            .flex_col()
            .gap_3()
            .child(section_header(
                "混合语义相似搜索 · Lookalike Discovery",
                "L1 SQL HARD FILTER × LATENT COSINE × BM25".into(),
                &self.fonts,
            ))
            .child(
                div()
                    .font_family(self.fonts.ui.clone())
                    .text_xs()
                    .line_height(px(20.))
                    .text_color(rgb(INK_SECONDARY))
                    .child("在右上角搜索框输入商业模式关键词（例如 \"recurring industrial CAD subscription\"）或直接输入美股 Ticker（如 ADSK），也可在右侧详情抽屉点击「找相似公司 · Lookalike」。前置叠加当前左侧芒格财务硬过滤条件（剔除金融 / 报价池 / ROIC / SBC），零大模型 Token 毫秒级返回可溯源相似标的。"),
            )
    }

    fn render_universe_similar_empty(&self, cx: &mut Context<Self>) -> gpui::Div {
        let l1_chunks = self
            .inbox_summary
            .as_ref()
            .map(|s| s.doc_chunks_count)
            .unwrap_or(0);
        let hint = if l1_chunks == 0 {
            "本地 L1 语义索引片段尚为空（doc_chunks=0）。请先到「简报收件箱」点击「追平事件流」或运行 `m3 events catchup --bootstrap` 构建文本索引。"
        } else {
            "当前相似搜索或财务硬过滤条件无匹配公司，可放宽过滤条件或更换关键词。"
        };
        card_shell()
            .p_5()
            .flex()
            .items_center()
            .justify_between()
            .gap_4()
            .child(
                div()
                    .font_family(self.fonts.ui.clone())
                    .text_xs()
                    .text_color(rgb(INK_SECONDARY))
                    .child(hint),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .when(l1_chunks == 0, |d| {
                        d.child(
                            div()
                                .id("univ-similar-go-catchup")
                                .px_3()
                                .py_1p5()
                                .rounded_sm()
                                .bg(rgb(CTA_BG))
                                .hover(|s| s.bg(rgb(CTA_BG_HOVER)))
                                .cursor_pointer()
                                .font_family(self.fonts.ui.clone())
                                .text_xs()
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .text_color(rgb(CTA_INK))
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.switch_view(NavView::Inbox);
                                    this.start_l1_catchup(true, cx);
                                }))
                                .child("追平 L1 文本索引"),
                        )
                    })
                    .child(
                        div()
                            .id("univ-similar-back-kw")
                            .px_3()
                            .py_1p5()
                            .rounded_sm()
                            .bg(rgb(BG_WELL))
                            .border_1()
                            .border_color(rgb(BORDER_PILL))
                            .hover(|s| s.bg(rgb(BG_HOVER)))
                            .cursor_pointer()
                            .font_family(self.fonts.ui.clone())
                            .text_xs()
                            .text_color(rgb(INK_HERO))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.universe_state
                                    .set_search_mode(UniverseSearchMode::Keyword);
                                this.reload_universe_from_lake();
                                cx.notify();
                            }))
                            .child("返回关键词筛选"),
                    ),
            )
    }

    fn render_universe_similar_results(
        &self,
        selected_cik: Option<&str>,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        let elapsed = self
            .universe_similar_query_ms
            .map(|ms| format!("{ms}ms"))
            .unwrap_or_else(|| "—".into());
        let subtitle = format!(
            "命中 {} 家 · 查询 {} · 0 Flagship Token",
            self.universe_similar_hits.len(),
            elapsed
        );

        div()
            .flex()
            .flex_col()
            .gap_3()
            .child(card_shell().p_4().child(section_header(
                &format!("Lookalike 相似公司 · {}", self.universe_state.query.trim()),
                subtitle,
                &self.fonts,
            )))
            .children(
                self.universe_similar_hits
                    .iter()
                    .enumerate()
                    .map(|(idx, hit)| {
                        self.render_universe_similar_card(hit, idx, selected_cik, cx)
                    }),
            )
    }

    fn render_universe_similar_card(
        &self,
        hit: &SimilarCompanyHit,
        idx: usize,
        selected_cik: Option<&str>,
        cx: &mut Context<Self>,
    ) -> gpui::Stateful<gpui::Div> {
        let row = &hit.company;
        let is_selected = selected_cik == Some(row.cik.as_str());
        let row_click = row.clone();
        let row_watch = row.clone();
        let ticker_dossier = row.ticker.clone();
        let live_bkt = self.live_watchlist_bucket(row);
        let (roic_s, _) = fmt_pct_clean(row.roic_5y_avg_pct, false);
        let (fcf_s, _) = fmt_pct_clean(row.strict_fcf_yield_pct, false);
        let (sbc_s, _) = fmt_pct_clean(row.sbc_to_ocf_pct, false);

        div()
            .id(gpui::SharedString::from(format!(
                "univ-sim-hit-{}",
                row.cik
            )))
            .rounded_lg()
            .bg(rgb(if is_selected {
                BG_SELECTED_ROW
            } else {
                BG_SURFACE
            }))
            .border_1()
            .border_color(if is_selected {
                rgba(VIOLET_BORDER)
            } else {
                rgb(BORDER_SUBTLE)
            })
            .hover(|s| s.bg(rgb(BG_HOVER)))
            .cursor_pointer()
            .p_4()
            .on_click(cx.listener(move |this, _, _, cx| {
                this.select_universe_row(&row_click);
                cx.notify();
            }))
            .flex()
            .flex_col()
            .gap_2p5()
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
                                    .font_family(self.fonts.mono_num.clone())
                                    .text_xs()
                                    .text_color(rgb(INK_MUTED))
                                    .child(format!("#{}", idx + 1)),
                            )
                            .child(
                                div()
                                    .font_family(self.fonts.mono_num.clone())
                                    .text_sm()
                                    .font_weight(gpui::FontWeight::BOLD)
                                    .text_color(rgb(INK_HERO))
                                    .child(row.ticker.clone()),
                            )
                            .child(
                                div()
                                    .font_family(self.fonts.ui.clone())
                                    .text_xs()
                                    .text_color(rgb(INK_PRIMARY))
                                    .truncate()
                                    .child(row.name.clone()),
                            )
                            .when_some(live_bkt, |d, bkt| {
                                let (fg, bg, br) = bucket_colors(bkt);
                                d.child(badge(bkt.code().to_owned(), fg, bg, br, &self.fonts))
                            }),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(badge(
                                format!("综合 {:.0}%", hit.score * 100.0),
                                VIOLET_FG,
                                VIOLET_BG,
                                VIOLET_BORDER,
                                &self.fonts,
                            ))
                            .child(
                                div()
                                    .font_family(self.fonts.mono_num.clone())
                                    .text_size(px(10.))
                                    .text_color(rgb(INK_MUTED))
                                    .child(format!(
                                        "语义 {:.0}% · BM25 {:.2} · {} 片段",
                                        hit.semantic_score * 100.0,
                                        hit.lexical_score,
                                        hit.matched_chunk_count
                                    )),
                            ),
                    ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_3()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_3()
                            .font_family(self.fonts.mono_num.clone())
                            .text_size(px(11.))
                            .child(
                                div()
                                    .text_color(rgb(INK_SECONDARY))
                                    .child(format!("ROIC {roic_s}")),
                            )
                            .child(
                                div()
                                    .text_color(rgb(INK_SECONDARY))
                                    .child(format!("FCF% {fcf_s}")),
                            )
                            .child(
                                div()
                                    .text_color(rgb(INK_SECONDARY))
                                    .child(format!("SBC/OCF {sbc_s}")),
                            )
                            .child(div().text_color(rgb(INK_MUTED)).child(format_sic_line(row))),
                    )
                    .child(if live_bkt.is_some() {
                        div()
                            .id(gpui::SharedString::from(format!(
                                "univ-sim-open-{}",
                                row.cik
                            )))
                            .px_2p5()
                            .py_1()
                            .rounded_xs()
                            .bg(rgb(BG_WELL))
                            .border_1()
                            .border_color(rgb(BORDER_PILL))
                            .hover(|s| s.bg(rgb(BG_ACTIVE)))
                            .font_family(self.fonts.ui.clone())
                            .text_size(px(11.))
                            .text_color(rgb(INK_HERO))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.select_company(&ticker_dossier, Some(NavView::Dossier));
                                cx.stop_propagation();
                                cx.notify();
                            }))
                            .child("打开 Dossier →")
                    } else {
                        div()
                            .id(gpui::SharedString::from(format!(
                                "univ-sim-watch-{}",
                                row.cik
                            )))
                            .px_2p5()
                            .py_1()
                            .rounded_xs()
                            .bg(rgba(EMERALD_BG))
                            .border_1()
                            .border_color(rgba(EMERALD_BORDER))
                            .hover(|s| s.bg(rgb(BG_HOVER)))
                            .font_family(self.fonts.ui.clone())
                            .text_size(px(11.))
                            .text_color(rgb(EMERALD_FG))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.add_universe_company_to_watch(&row_watch, cx);
                                cx.stop_propagation();
                            }))
                            .child("+ 入观察池")
                    }),
            )
            .children(hit.evidence.iter().take(2).enumerate().map(|(e_idx, ev)| {
                self.render_evidence_card(ev, &format!("sim-{}", row.cik), e_idx, cx)
            }))
    }
}
