//! Curated Berkshire × Munger Non-Financial Classics starter pack.
//!
//! Provides an idempotent, non-destructive [`apply_starter_pack`] helper for
//! first-run onboarding on fresh workspaces.

use crate::model::{
    Bucket, CompanyEntry, DataSource, Dossier, HumanJudgment, Latticework, ValuationAssumptions,
    WatchlistFile,
};
use crate::store::{self, WorkspacePaths};
use anyhow::{Context, Result};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use std::fs;

/// Summary returned after applying the curated starter pack to a workspace.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StarterPackReport {
    pub total_starters: usize,
    pub added_tickers: Vec<String>,
    pub skipped_existing_tickers: Vec<String>,
    pub watchlist_total: usize,
}

struct StarterSpec {
    ticker: &'static str,
    name: &'static str,
    bucket: Bucket,
    cik: &'static str,
    circle: &'static str,
    too_hard_reason: Option<&'static str>,
    multiple_low: i64,
    multiple_high: i64,
    target_return_pct: i64,
    expected_growth_pct: i64,
}

const STARTER_SPECS: &[StarterSpec] = &[
    // ── Core (4): Verified compounders inside Circle of Competence ─────────────
    StarterSpec {
        ticker: "MCO",
        name: "Moody's Corporation",
        bucket: Bucket::Core,
        cik: "0001059556",
        circle: "信用评级 / 制度性收费站",
        too_hard_reason: None,
        multiple_low: 18,
        multiple_high: 22,
        target_return_pct: 10,
        expected_growth_pct: 8,
    },
    StarterSpec {
        ticker: "COST",
        name: "Costco Wholesale Corp.",
        bucket: Bucket::Core,
        cik: "0000909832",
        circle: "会员制零售 / 规模经济共享",
        too_hard_reason: None,
        multiple_low: 20,
        multiple_high: 25,
        target_return_pct: 10,
        expected_growth_pct: 8,
    },
    StarterSpec {
        ticker: "AAPL",
        name: "Apple Inc.",
        bucket: Bucket::Core,
        cik: "0000320193",
        circle: "消费生态 / 硬件+服务",
        too_hard_reason: None,
        multiple_low: 18,
        multiple_high: 22,
        target_return_pct: 10,
        expected_growth_pct: 7,
    },
    StarterSpec {
        ticker: "KO",
        name: "The Coca-Cola Company",
        bucket: Bucket::Core,
        cik: "0000021344",
        circle: "消费品牌 / 浓缩液特许经营",
        too_hard_reason: None,
        multiple_low: 16,
        multiple_high: 20,
        target_return_pct: 10,
        expected_growth_pct: 5,
    },
    // ── Watch (2): High-ROIC franchises awaiting deeper Feynman triage ────────
    StarterSpec {
        ticker: "VRSN",
        name: "VeriSign, Inc.",
        bucket: Bucket::Watch,
        cik: "0001014473",
        circle: "互联网基建 / .com 垄断注册",
        too_hard_reason: None,
        multiple_low: 18,
        multiple_high: 22,
        target_return_pct: 10,
        expected_growth_pct: 7,
    },
    StarterSpec {
        ticker: "DPZ",
        name: "Domino's Pizza, Inc.",
        bucket: Bucket::Watch,
        cik: "0001286681",
        circle: "特许加盟餐饮 / 外卖供应链",
        too_hard_reason: None,
        multiple_low: 18,
        multiple_high: 22,
        target_return_pct: 10,
        expected_growth_pct: 8,
    },
    // ── TooHard (2): Anti-FOMO reference museum ──────────────────────────────
    StarterSpec {
        ticker: "AAL",
        name: "American Airlines Group Inc.",
        bucket: Bucket::TooHard,
        cik: "0000006201",
        circle: "航空运输 / 周期性重资产",
        too_hard_reason: Some("高杠杆重资产与油价/经济周期高度敏感，缺乏定价权与可持续自由现金流"),
        multiple_low: 10,
        multiple_high: 12,
        target_return_pct: 12,
        expected_growth_pct: 0,
    },
    StarterSpec {
        ticker: "INTC",
        name: "Intel Corporation",
        bucket: Bucket::TooHard,
        cik: "0000050863",
        circle: "晶圆代工 / 半导体制造",
        too_hard_reason: Some(
            "重资产晶圆代工追赶期，巨额 CapEx 与竞争格局导致自由现金流高度不可测",
        ),
        multiple_low: 12,
        multiple_high: 15,
        target_return_pct: 12,
        expected_growth_pct: 0,
    },
];

/// Returns the curated **Berkshire × Munger Non-Financial Classics** starter entries.
pub fn starter_pack_entries() -> Vec<CompanyEntry> {
    STARTER_SPECS
        .iter()
        .map(|spec| CompanyEntry {
            ticker: spec.ticker.to_owned(),
            name: spec.name.to_owned(),
            bucket: spec.bucket,
            source: DataSource::SecEdgar,
            cik: Some(spec.cik.to_owned()),
            currency: "USD".to_owned(),
            circle: spec.circle.to_owned(),
            too_hard_reason: spec.too_hard_reason.map(str::to_owned),
            assumptions: ValuationAssumptions {
                multiple_low: Decimal::from(spec.multiple_low),
                multiple_high: Decimal::from(spec.multiple_high),
                target_return_pct: Decimal::from(spec.target_return_pct),
                expected_growth_pct: Decimal::from(spec.expected_growth_pct),
                strike_price_override: None,
            },
        })
        .collect()
}

/// Adds missing Berkshire × Munger starter companies to `watchlist.json` idempotently.
///
/// Existing user entries in `watchlist.json` (matched case-insensitively by ticker)
/// and existing dossier files are never overwritten.
pub fn apply_starter_pack(paths: &WorkspacePaths) -> Result<StarterPackReport> {
    let watchlist_path = paths.watchlist_path();
    let (mut wl, watchlist_was_missing) = match fs::symlink_metadata(&watchlist_path) {
        Ok(_) => (store::load_watchlist(paths)?, false),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => (
            WatchlistFile {
                schema_version: 1,
                benchmark_symbol: "^GSPC".to_owned(),
                default_target_return_pct: Decimal::from(10),
                companies: Vec::new(),
            },
            true,
        ),
        Err(err) => {
            return Err(err)
                .with_context(|| format!("inspect watchlist `{}`", watchlist_path.display()));
        }
    };

    let entries = starter_pack_entries();
    let total_starters = entries.len();
    let mut added_tickers = Vec::new();
    let mut skipped_existing_tickers = Vec::new();
    let mut added_entries = Vec::new();

    for entry in entries {
        let already_exists = wl
            .companies
            .iter()
            .any(|existing| existing.ticker.eq_ignore_ascii_case(&entry.ticker));
        if already_exists {
            skipped_existing_tickers.push(entry.ticker);
        } else {
            added_tickers.push(entry.ticker.clone());
            added_entries.push(entry.clone());
            wl.companies.push(entry);
        }
    }

    if watchlist_was_missing || !added_tickers.is_empty() {
        store::save_watchlist(paths, &wl)?;
    }

    let today = store::civil_today_iso();
    for entry in &added_entries {
        let dossier_path = paths.dossier_path(&entry.ticker);
        if matches!(
            fs::symlink_metadata(&dossier_path),
            Err(ref err) if err.kind() == std::io::ErrorKind::NotFound
        ) {
            let skeleton = Dossier {
                schema_version: 1,
                ticker: entry.ticker.clone(),
                name: entry.name.clone(),
                updated_at: today.clone(),
                draft_source: "agent_draft".to_owned(),
                three_sentence_business: "待运行 Alan 审讯工作流生成三句话商业逻辑。".to_owned(),
                latticework: Latticework {
                    microeconomics: "待补充".to_owned(),
                    ecology: "待补充".to_owned(),
                    psychology_incentives: "待补充".to_owned(),
                    capital_allocation: "待补充".to_owned(),
                },
                premortem: Vec::new(),
                open_questions: Vec::new(),
                human_judgment: HumanJudgment {
                    pending_human_review: true,
                    my_stance: None,
                    weight_rationale: None,
                    falsification_metrics: Vec::new(),
                    feynman_checked_at: None,
                },
            };
            store::write_json_atomic(&dossier_path, &skeleton)?;
        }
    }

    Ok(StarterPackReport {
        total_starters,
        added_tickers,
        skipped_existing_tickers,
        watchlist_total: wl.companies.len(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    static TEST_COUNTER: AtomicU64 = AtomicU64::new(0);

    fn temp_workspace(label: &str) -> PathBuf {
        let seq = TEST_COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "m3_starter_test_{label}_{}_{seq}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn starter_pack_has_expected_buckets_and_metadata() {
        let entries = starter_pack_entries();
        assert_eq!(entries.len(), 8);

        let core: Vec<&str> = entries
            .iter()
            .filter(|e| e.bucket == Bucket::Core)
            .map(|e| e.ticker.as_str())
            .collect();
        let watch: Vec<&str> = entries
            .iter()
            .filter(|e| e.bucket == Bucket::Watch)
            .map(|e| e.ticker.as_str())
            .collect();
        let too_hard: Vec<&str> = entries
            .iter()
            .filter(|e| e.bucket == Bucket::TooHard)
            .map(|e| e.ticker.as_str())
            .collect();

        assert_eq!(core, vec!["MCO", "COST", "AAPL", "KO"]);
        assert_eq!(watch, vec!["VRSN", "DPZ"]);
        assert_eq!(too_hard, vec!["AAL", "INTC"]);

        for entry in &entries {
            assert_eq!(entry.source, DataSource::SecEdgar);
            assert!(
                entry
                    .cik
                    .as_deref()
                    .is_some_and(|c| c.len() == 10 && c.bytes().all(|b| b.is_ascii_digit())),
                "invalid CIK for {}",
                entry.ticker
            );
            assert!(!entry.circle.trim().is_empty());
            assert!(entry.assumptions.multiple_low < entry.assumptions.multiple_high);
            assert!(entry.assumptions.target_return_pct >= Decimal::from(10));
            if entry.bucket == Bucket::TooHard {
                assert!(
                    entry
                        .too_hard_reason
                        .as_deref()
                        .is_some_and(|r| !r.trim().is_empty()),
                    "missing too_hard_reason for {}",
                    entry.ticker
                );
            } else {
                assert!(entry.too_hard_reason.is_none());
            }
        }
    }

    #[test]
    fn apply_starter_pack_creates_watchlist_and_is_idempotent() {
        let dir = temp_workspace("create_and_idempotent");
        let paths = WorkspacePaths::from_root(dir.clone());

        let first = apply_starter_pack(&paths).unwrap();
        assert_eq!(first.total_starters, 8);
        assert_eq!(first.added_tickers.len(), 8);
        assert!(first.skipped_existing_tickers.is_empty());
        assert_eq!(first.watchlist_total, 8);

        let wl = store::load_watchlist(&paths).unwrap();
        assert_eq!(wl.companies.len(), 8);
        for ticker in ["MCO", "COST", "AAPL", "KO", "VRSN", "DPZ", "AAL", "INTC"] {
            assert!(paths.dossier_path(ticker).exists());
        }

        let second = apply_starter_pack(&paths).unwrap();
        assert_eq!(second.total_starters, 8);
        assert!(second.added_tickers.is_empty());
        assert_eq!(second.skipped_existing_tickers.len(), 8);
        assert_eq!(second.watchlist_total, 8);

        let wl_after = store::load_watchlist(&paths).unwrap();
        assert_eq!(wl_after, wl);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn apply_starter_pack_never_overwrites_existing_user_entries() {
        let dir = temp_workspace("preserve_existing");
        let paths = WorkspacePaths::from_root(dir.clone());

        let custom_aapl = CompanyEntry {
            ticker: "aapl".to_owned(),
            name: "My Custom Apple".to_owned(),
            bucket: Bucket::Out,
            source: DataSource::SecEdgar,
            cik: Some("0000320193".to_owned()),
            currency: "USD".to_owned(),
            circle: "Custom Circle".to_owned(),
            too_hard_reason: Some("User-specified note".to_owned()),
            assumptions: ValuationAssumptions {
                multiple_low: Decimal::from(14),
                multiple_high: Decimal::from(16),
                target_return_pct: Decimal::from(15),
                expected_growth_pct: Decimal::from(4),
                strike_price_override: Some(Decimal::from(120)),
            },
        };
        let extra_user = CompanyEntry {
            ticker: "BRK.B".to_owned(),
            name: "Berkshire Hathaway".to_owned(),
            bucket: Bucket::Core,
            source: DataSource::Manual,
            cik: None,
            currency: "USD".to_owned(),
            circle: "Conglomerate".to_owned(),
            too_hard_reason: None,
            assumptions: ValuationAssumptions {
                multiple_low: Decimal::from(15),
                multiple_high: Decimal::from(18),
                target_return_pct: Decimal::from(10),
                expected_growth_pct: Decimal::from(7),
                strike_price_override: None,
            },
        };

        store::save_watchlist(
            &paths,
            &WatchlistFile {
                schema_version: 1,
                benchmark_symbol: "^GSPC".to_owned(),
                default_target_return_pct: Decimal::from(10),
                companies: vec![custom_aapl.clone(), extra_user.clone()],
            },
        )
        .unwrap();

        let report = apply_starter_pack(&paths).unwrap();
        assert_eq!(report.total_starters, 8);
        assert_eq!(report.added_tickers.len(), 7);
        assert_eq!(report.skipped_existing_tickers, vec!["AAPL"]);
        assert_eq!(report.watchlist_total, 9);

        let wl = store::load_watchlist(&paths).unwrap();
        assert_eq!(wl.companies.len(), 9);
        assert_eq!(wl.companies[0], custom_aapl);
        assert_eq!(wl.companies[1], extra_user);

        let _ = fs::remove_dir_all(&dir);
    }
}
