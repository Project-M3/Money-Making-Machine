//! Reactive store domain types and UI state structs for `m3-nsh`.

use m3_core::alan_interrogator::AcmeEvidenceToken;
use m3_core::i18n::UiLang;
use m3_core::lake::{UniverseFilter, UniverseRow, UniverseSortBy};
use m3_core::lake_l1::SimilarSearchFilter;
use m3_core::model::Bucket;
use m3_core::research::DeepResearchPerspective;
use m3_core::update::ReleaseChannel;
use rust_decimal::Decimal;

/// Primary workspace tabs in the left sidebar (`⌘1..⌘5`, `⌘,`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum WorkspaceTab {
    #[default]
    Inbox,
    Universe,
    Industry,
    Baskets,
    Journal,
    Settings,
}

/// Active subview tab inside the Company Dossier / Research Desk overlay.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CompanyViewTab {
    #[default]
    FinancialsMemo,
    DeepResearchAlan,
    XbrlInspector,
}

/// View mode in Circle Overview (`Baskets`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BasketsViewMode {
    #[default]
    List,
    Board,
}

/// Price chart timeframe selector.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ChartTimeframe {
    #[default]
    FiveYearWeekly,
    ThreeYearWeekly,
    OneYearDaily,
}

impl ChartTimeframe {
    pub fn label_for(self, lang: UiLang) -> &'static str {
        match (self, lang) {
            (Self::FiveYearWeekly, UiLang::Zh) => "5Y 周K",
            (Self::FiveYearWeekly, UiLang::En) => "5Y Wk",
            (Self::ThreeYearWeekly, UiLang::Zh) => "3Y 周K",
            (Self::ThreeYearWeekly, UiLang::En) => "3Y Wk",
            (Self::OneYearDaily, UiLang::Zh) => "1Y 日K",
            (Self::OneYearDaily, UiLang::En) => "1Y Day",
        }
    }
}

/// Quick status filter pill in Circle Overview (`Baskets`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum StatusQuickFilter {
    #[default]
    All,
    StrikeOnly,
    WarnedOnly,
}

impl StatusQuickFilter {
    pub fn label_for(self, lang: UiLang) -> &'static str {
        match (self, lang) {
            (Self::All, UiLang::Zh) => "全部",
            (Self::All, UiLang::En) => "All",
            (Self::StrikeOnly, UiLang::Zh) => "击球区",
            (Self::StrikeOnly, UiLang::En) => "Strike Zone",
            (Self::WarnedOnly, UiLang::Zh) => "红灯",
            (Self::WarnedOnly, UiLang::En) => "Warnings",
        }
    }
}

/// Sort column in Circle Overview (`Baskets`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SortCol {
    #[default]
    Default,
    Ticker,
    Price,
    Margin,
    Roic,
    FcfYield,
    ShareDelta,
    NetDebt,
}

/// Summary of today's Circle pulse (strike candidates, red-flagged core, watch count).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TodayPulse {
    pub strike_tickers: Vec<String>,
    pub red_core_tickers: Vec<String>,
    pub watch_count: usize,
}

/// Command palette action executed on selection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PaletteAction {
    OpenDossier(String),
    OpenAgent(String),
    RunDeepResearch(String),
    ShowBudgetStatus,
    SyncThemes,
    SwitchTab(WorkspaceTab),
    OpenCompanyTab(CompanyViewTab),
    FocusBasket(Option<Bucket>),
    ToggleBasketsView,
    MoveCompanyBucket(String, Bucket),
    FilterCircle(Option<String>),
    ToggleBasis,
    ToggleInspector,
    CopyCli(String, String),
    SyncNow,
    SyncUniverseNow,
    CatchupEvents,
    FocusSimilarSearch,
    CheckUpdate,
    ApplyUpdate,
    SwitchUpdateChannel(ReleaseChannel),
    RestartApp,
    OpenSettings,
    SetLanguage(UiLang),
}

/// Single entry shown in the `⌘K` Command Palette.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaletteItem {
    pub group: String,
    pub title: String,
    pub subtitle: String,
    pub shortcut: Option<&'static str>,
    pub dot_color: u32,
    pub action: PaletteAction,
}

/// Filter tabs shown in the Event-Driven Inbox bar.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum InboxFilter {
    #[default]
    Unread,
    All,
    Discovery,
    Ripple,
    StrikeAndRedFlag,
    Archived,
}

impl InboxFilter {
    pub fn label_for(self, lang: UiLang) -> &'static str {
        match self {
            Self::Unread => lang.t("未读", "Unread"),
            Self::All => lang.t("全部", "All"),
            Self::Discovery => lang.t("量化候选", "Quant Screen"),
            Self::Ripple => lang.t("隐空间传导", "Ripple"),
            Self::StrikeAndRedFlag => lang.t("击球与红灯", "Strike & Red"),
            Self::Archived => lang.t("已归档", "Archived"),
        }
    }

    pub fn all() -> [Self; 6] {
        [
            Self::Unread,
            Self::All,
            Self::Discovery,
            Self::Ripple,
            Self::StrikeAndRedFlag,
            Self::Archived,
        ]
    }
}

/// UI navigation state for the Inbox view.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct InboxViewState {
    pub filter: InboxFilter,
    pub selected_signal_id: Option<String>,
    pub cursor_index: usize,
}

/// Search mode in the Universe Radar control strip.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum UniverseSearchMode {
    #[default]
    Keyword,
    Similar,
}

impl UniverseSearchMode {
    pub fn label_for(self, lang: UiLang) -> &'static str {
        match self {
            Self::Keyword => lang.t("关键词", "Keyword"),
            Self::Similar => lang.t("相似 Lookalike", "Lookalike"),
        }
    }
}

/// Named Munger screening presets in the Universe Radar control strip.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum UniversePreset {
    #[default]
    AllQuality,
    HighRoicCompounders,
    LowSbcCashCows,
    StrikeZone,
}

impl UniversePreset {
    pub fn label_for(self, lang: UiLang) -> &'static str {
        match self {
            Self::AllQuality => lang.t("全部质量池", "All Quality"),
            Self::HighRoicCompounders => lang.t("高 ROIC 复利 (>=20%)", "High ROIC (>=20%)"),
            Self::LowSbcCashCows => lang.t("低 SBC 现金牛", "Low-SBC Cash Cows"),
            Self::StrikeZone => lang.t("击球区", "Strike Zone"),
        }
    }

    pub const fn id_slug(self) -> &'static str {
        match self {
            Self::AllQuality => "all-quality",
            Self::HighRoicCompounders => "high-roic",
            Self::LowSbcCashCows => "low-sbc",
            Self::StrikeZone => "strike-zone",
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
            sort_by: UniverseSortBy::QualityScore,
            sort_asc: false,
            cursor_index: 0,
            limit: 6500,
        }
    }
}

impl UniverseViewState {
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
        self.cursor_index = 0;
    }

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

pub fn apply_ui_row_guards(
    mut rows: Vec<UniverseRow>,
    require_5y_history: bool,
) -> Vec<UniverseRow> {
    if require_5y_history {
        rows.retain(|r| r.years_of_data >= 5);
    }
    rows
}

/// One plotted company point on the 2D ROIC x Strict FCF Yield scatter canvas.
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

#[derive(Debug, Clone, PartialEq, Default)]
pub struct ScatterPlotModel {
    pub points: Vec<ScatterPoint>,
    pub x_min: f64,
    pub x_max: f64,
    pub y_min: f64,
    pub y_max: f64,
    pub ref_roic_norm: Option<f32>,
    pub ref_fcf_norm: Option<f32>,
    pub zero_fcf_norm: Option<f32>,
    pub clipped_count: usize,
    pub strike_count: usize,
    pub watch_count: usize,
}

/// Chat role in EmbeddedAlan / Industry Tutor panels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentMessageRole {
    User,
    Assistant,
    SystemNote,
}

/// Chat message rendered in EmbeddedAlan / Industry Tutor panels.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentUiMessage {
    pub id: String,
    pub role: AgentMessageRole,
    pub raw_text: String,
    pub text: String,
    pub tokens: Vec<AcmeEvidenceToken>,
    pub streaming: bool,
}

/// Routes EmbeddedAlan replies to either Company Research Desk or Industry Tutor.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum AlanReplyTarget {
    #[default]
    Company,
    Industry(String),
}

/// Parsed Living Research Memo section (`workbench/memos/<TICKER>.md`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoSection {
    pub id: String,
    pub heading: String,
    pub body: String,
}

/// Parsed Living Research Memo structure.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ParsedResearchMemo {
    pub title: String,
    pub metadata: Vec<(String, String)>,
    pub sections: Vec<MemoSection>,
    pub citations: Vec<String>,
    pub human_notes: Option<String>,
}

/// Status of one Deep Research perspective.
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

/// Live UI state for one Deep Research perspective stream.
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

/// Notice badge rendered in the left sidebar Status Dock.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SidebarDockNotice {
    pub tag: &'static str,
    pub message: String,
    pub dot_color: u32,
    pub border_color: u32,
    pub dismissible: bool,
}

/// Valuation multiple preset for Industry Research Studio.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IndustryValuationPreset {
    pub label_zh: &'static str,
    pub label_en: &'static str,
    pub low: u32,
    pub high: u32,
}

pub const INDUSTRY_VALUATION_PRESETS: [IndustryValuationPreset; 4] = [
    IndustryValuationPreset {
        label_zh: "10–14x 保守周期",
        label_en: "10–14x Cyclical",
        low: 10,
        high: 14,
    },
    IndustryValuationPreset {
        label_zh: "14–18x 稳健复利",
        label_en: "14–18x Steady",
        low: 14,
        high: 18,
    },
    IndustryValuationPreset {
        label_zh: "18–22x 优质护城河",
        label_en: "18–22x Quality Moat",
        low: 18,
        high: 22,
    },
    IndustryValuationPreset {
        label_zh: "22–28x 特许经营权",
        label_en: "22–28x Franchise",
        low: 22,
        high: 28,
    },
];
