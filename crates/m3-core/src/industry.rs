use crate::{
    budget::ThemeCluster,
    i18n::UiLang,
    lake::{LakeDb, UniverseRow},
    lake_l1::SimilarSearchFilter,
    latent,
    model::serde_decimal_opt,
    store::{self, WorkspacePaths},
};
use anyhow::{Context, Result};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use std::{collections::HashSet, fs, path::PathBuf};

pub const HUMAN_NOTES_HEADER: &str = "## Human Notes";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IndustryStatus {
    #[default]
    Studying,
    InsideCircle,
    TooHard,
}

impl IndustryStatus {
    pub fn sort_rank(self) -> u8 {
        match self {
            Self::InsideCircle => 0,
            Self::Studying => 1,
            Self::TooHard => 2,
        }
    }

    pub fn label_for(self, lang: UiLang) -> &'static str {
        match (self, lang) {
            (Self::Studying, UiLang::Zh) => "学习中",
            (Self::Studying, UiLang::En) => "Studying",
            (Self::InsideCircle, UiLang::Zh) => "能力圈内",
            (Self::InsideCircle, UiLang::En) => "Inside Circle",
            (Self::TooHard, UiLang::Zh) => "Too Hard",
            (Self::TooHard, UiLang::En) => "Too Hard",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct IndustryProfile {
    pub slug: String,
    pub title: String,
    #[serde(default)]
    pub status: IndustryStatus,
    #[serde(default)]
    pub sic_codes: Vec<u32>,
    #[serde(default)]
    pub keywords: Vec<String>,
    #[serde(default)]
    pub bound_tickers: Vec<String>,
    #[serde(default, with = "serde_decimal_opt")]
    pub multiple_low: Option<Decimal>,
    #[serde(default, with = "serde_decimal_opt")]
    pub multiple_high: Option<Decimal>,
    #[serde(default)]
    pub key_kpis: Vec<String>,
    #[serde(default)]
    pub feynman_questions: Vec<String>,
    #[serde(default)]
    pub feynman_checked: Vec<bool>,
    #[serde(default)]
    pub human_notes: String,
    #[serde(default)]
    pub created_at_ms: u64,
    #[serde(default)]
    pub updated_at_ms: u64,
    #[serde(default)]
    pub last_research_run_id: Option<String>,
}

impl IndustryProfile {
    pub fn valuation_anchor_label(&self) -> Option<String> {
        match (self.multiple_low, self.multiple_high) {
            (Some(lo), Some(hi)) => Some(format!(
                "{}–{}x",
                lo.round_dp(1).normalize(),
                hi.round_dp(1).normalize()
            )),
            (Some(lo), None) => Some(format!("{}x+", lo.round_dp(1).normalize())),
            (None, Some(hi)) => Some(format!("≤{}x", hi.round_dp(1).normalize())),
            (None, None) => None,
        }
    }
}

pub fn slugify_industry(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return "industry-study".to_owned();
    }
    let mut slug = String::with_capacity(trimmed.len().min(48));
    let mut prev_dash = false;
    for ch in trimmed.chars() {
        if ch.is_ascii_alphanumeric() {
            slug.push(ch.to_ascii_lowercase());
            prev_dash = false;
        } else if !prev_dash && !slug.is_empty() {
            slug.push('-');
            prev_dash = true;
        }
        if slug.len() >= 48 {
            break;
        }
    }
    let slug = slug.trim_matches('-').to_owned();
    if slug.len() >= 2 {
        return slug;
    }
    let hash = latent::sha256_hex(trimmed.as_bytes());
    let short = hash.get(..10).unwrap_or(&hash);
    format!("ind-{short}")
}

pub fn list_industries(paths: &WorkspacePaths) -> Result<Vec<IndustryProfile>> {
    let dir = paths.industries_dir();
    if !dir.exists() {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    for entry in fs::read_dir(&dir).with_context(|| format!("读取目录失败 {}", dir.display()))?
    {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        match store::read_json::<IndustryProfile>(&path) {
            Ok(mut profile) => {
                sync_feynman_checked_len(&mut profile);
                out.push(profile);
            }
            Err(err) => {
                log::warn!(
                    "Skipping malformed industry file {}: {err:#}",
                    path.display()
                );
            }
        }
    }
    out.sort_by(|a, b| {
        a.status
            .sort_rank()
            .cmp(&b.status.sort_rank())
            .then_with(|| b.updated_at_ms.cmp(&a.updated_at_ms))
            .then_with(|| a.title.cmp(&b.title))
    });
    Ok(out)
}

pub fn load_industry(paths: &WorkspacePaths, slug: &str) -> Result<IndustryProfile> {
    let path = paths.industry_profile_path(slug);
    let mut profile: IndustryProfile = store::read_json(&path)?;
    sync_feynman_checked_len(&mut profile);
    Ok(profile)
}

pub fn save_industry(paths: &WorkspacePaths, profile: &IndustryProfile) -> Result<()> {
    let mut normalized = profile.clone();
    normalized.bound_tickers = normalize_tickers(&normalized.bound_tickers);
    sync_feynman_checked_len(&mut normalized);
    let path = paths.industry_profile_path(&normalized.slug);
    store::write_json_atomic(&path, &normalized)
}

/// Removes an industry profile JSON and its companion Living Primer Markdown file.
/// Returns `Ok(true)` if at least one file existed and was removed.
pub fn delete_industry(paths: &WorkspacePaths, slug: &str) -> Result<bool> {
    let trimmed = slug.trim();
    if trimmed.is_empty()
        || trimmed.contains('/')
        || trimmed.contains('\\')
        || trimmed.contains("..")
    {
        anyhow::bail!("无效的行业标识: {slug}");
    }
    let mut removed = false;
    for path in [
        paths.industry_profile_path(trimmed),
        paths.industry_primer_path(trimmed),
    ] {
        if path.exists() {
            fs::remove_file(&path)
                .with_context(|| format!("删除行业文件失败 {}", path.display()))?;
            removed = true;
        }
    }
    Ok(removed)
}

pub fn load_industry_primer_md(paths: &WorkspacePaths, slug: &str) -> Option<String> {
    let path = paths.industry_primer_path(slug);
    fs::read_to_string(path).ok()
}

pub fn extract_human_notes_footer(markdown: &str) -> Option<&str> {
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

pub fn write_industry_primer_preserving_human_notes(
    paths: &WorkspacePaths,
    slug: &str,
    generated_body: &str,
    fallback_human_notes: Option<&str>,
) -> Result<PathBuf> {
    let dir = paths.industries_dir();
    fs::create_dir_all(&dir)?;
    let path = paths.industry_primer_path(slug);
    let existing_md = fs::read_to_string(&path).ok();

    let mut out = generated_body.trim_end().to_owned();
    if !out.is_empty() {
        out.push_str("\n\n");
    }

    if let Some(preserved_footer) = existing_md.as_deref().and_then(extract_human_notes_footer) {
        out.push_str(preserved_footer);
        if !preserved_footer.ends_with('\n') {
            out.push('\n');
        }
    } else {
        out.push_str(HUMAN_NOTES_HEADER);
        out.push_str("\n\n");
        if let Some(notes) = fallback_human_notes
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            out.push_str(notes);
            out.push('\n');
        } else {
            out.push_str("<!-- 人类投资者亲笔备忘区：Alan 行业研究永不覆盖此标题及以下内容 -->\n");
        }
    }

    let tmp = store::atomic_temp_path(&path);
    if let Err(err) = fs::write(&tmp, out.as_bytes()).and_then(|_| fs::rename(&tmp, &path)) {
        let _ = fs::remove_file(&tmp);
        return Err(err.into());
    }
    Ok(path)
}

pub fn find_industry_for_company<'a>(
    industries: &'a [IndustryProfile],
    ticker: &str,
    sic: Option<u32>,
) -> Option<&'a IndustryProfile> {
    let upper = ticker.trim().to_ascii_uppercase();
    if !upper.is_empty()
        && let Some(hit) = industries.iter().find(|ind| {
            ind.bound_tickers
                .iter()
                .any(|t| t.eq_ignore_ascii_case(&upper))
        })
    {
        return Some(hit);
    }
    if let Some(code) = sic {
        return industries.iter().find(|ind| ind.sic_codes.contains(&code));
    }
    None
}

pub fn create_or_open_from_topic(paths: &WorkspacePaths, topic: &str) -> Result<IndustryProfile> {
    let title = topic.trim();
    anyhow::ensure!(!title.is_empty(), "行业主题不能为空");
    let slug = slugify_industry(title);

    let existing = list_industries(paths).unwrap_or_default();
    if let Some(found) = existing
        .into_iter()
        .find(|p| p.slug == slug || p.title.eq_ignore_ascii_case(title))
    {
        return Ok(found);
    }

    let mut bound_tickers: Vec<String> = Vec::new();
    let mut sic_counts: Vec<(u32, usize)> = Vec::new();

    if let Ok(Some(db)) = LakeDb::open_read_only_if_exists(paths) {
        let mut collected: Vec<UniverseRow> =
            db.search_universe_top_tickers(title, 8).unwrap_or_default();
        if collected.len() < 4 {
            for token in
                title.split(|c: char| c.is_whitespace() || c == '/' || c == '&' || c == ',')
            {
                let tk = token.trim();
                if tk.chars().count() >= 2
                    && let Ok(more) = db.search_universe_top_tickers(tk, 5)
                {
                    for row in more {
                        if !collected
                            .iter()
                            .any(|r| r.ticker.eq_ignore_ascii_case(&row.ticker))
                        {
                            collected.push(row);
                        }
                    }
                }
                if collected.len() >= 8 {
                    break;
                }
            }
        }
        if collected.len() < 5 {
            let filter = SimilarSearchFilter {
                exclude_financials: false,
                limit: 6,
                ..Default::default()
            };
            if let Ok(hits) = db.search_similar(title, false, &filter) {
                for hit in hits {
                    if !collected
                        .iter()
                        .any(|r| r.ticker.eq_ignore_ascii_case(&hit.company.ticker))
                    {
                        collected.push(hit.company);
                    }
                    if collected.len() >= 8 {
                        break;
                    }
                }
            }
        }
        for row in collected.into_iter().take(8) {
            bound_tickers.push(row.ticker);
            if let Some(sic) = row.sic {
                if let Some(entry) = sic_counts.iter_mut().find(|(s, _)| *s == sic) {
                    entry.1 += 1;
                } else {
                    sic_counts.push((sic, 1));
                }
            }
        }
    }

    sic_counts.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    let sic_codes: Vec<u32> = sic_counts.into_iter().take(3).map(|(s, _)| s).collect();
    let now = store::now_ms();
    let keywords: Vec<String> = title
        .split(|c: char| c.is_whitespace() || c == '/' || c == '&')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .collect();

    let profile = IndustryProfile {
        slug,
        title: title.to_owned(),
        status: IndustryStatus::Studying,
        sic_codes,
        keywords,
        bound_tickers: normalize_tickers(&bound_tickers),
        multiple_low: None,
        multiple_high: None,
        key_kpis: Vec::new(),
        feynman_questions: Vec::new(),
        feynman_checked: Vec::new(),
        human_notes: String::new(),
        created_at_ms: now,
        updated_at_ms: now,
        last_research_run_id: None,
    };
    save_industry(paths, &profile)?;
    Ok(profile)
}

pub fn create_or_open_from_company(
    paths: &WorkspacePaths,
    ticker: &str,
    company_name: &str,
    sic: Option<u32>,
    sic_desc: Option<&str>,
) -> Result<IndustryProfile> {
    let upper = ticker.trim().to_ascii_uppercase();
    anyhow::ensure!(!upper.is_empty(), "公司代码不能为空");

    let existing = list_industries(paths).unwrap_or_default();
    if let Some(mut found) = find_industry_for_company(&existing, &upper, sic).cloned() {
        if !found
            .bound_tickers
            .iter()
            .any(|t| t.eq_ignore_ascii_case(&upper))
        {
            found.bound_tickers.insert(0, upper);
            found.updated_at_ms = store::now_ms();
            save_industry(paths, &found)?;
        }
        return Ok(found);
    }

    let cleaned_desc = sic_desc
        .map(str::trim)
        .filter(|s| !s.is_empty() && !s.eq_ignore_ascii_case("unclassified"));
    let title = match (cleaned_desc, sic) {
        (Some(desc), Some(code)) => format!("{desc} (SIC {code})"),
        (Some(desc), None) => desc.to_owned(),
        (None, Some(code)) => format!("SIC {code} 行业研究"),
        (None, None) => {
            if company_name.trim().is_empty() {
                format!("{upper} 所在行业")
            } else {
                format!("{upper} · {} 所在行业", company_name.trim())
            }
        }
    };
    let slug = match sic {
        Some(code) if cleaned_desc.is_some() => {
            slugify_industry(&format!("sic-{code}-{}", cleaned_desc.unwrap_or("")))
        }
        Some(code) => format!("sic-{code}"),
        None => slugify_industry(&format!("{upper}-industry")),
    };

    let mut bound_tickers = vec![upper.clone()];
    if let Some(code) = sic
        && let Ok(Some(db)) = LakeDb::open_read_only_if_exists(paths)
        && let Ok(peers) = db.load_universe_rows_by_sic(code, 8)
    {
        for p in peers {
            if bound_tickers.len() >= 8 {
                break;
            }
            if !bound_tickers
                .iter()
                .any(|t| t.eq_ignore_ascii_case(&p.ticker))
            {
                bound_tickers.push(p.ticker);
            }
        }
    }

    let now = store::now_ms();
    let mut profile = IndustryProfile {
        slug,
        title,
        status: IndustryStatus::Studying,
        sic_codes: sic.into_iter().collect(),
        keywords: cleaned_desc.map(|s| vec![s.to_owned()]).unwrap_or_default(),
        bound_tickers: normalize_tickers(&bound_tickers),
        multiple_low: None,
        multiple_high: None,
        key_kpis: Vec::new(),
        feynman_questions: Vec::new(),
        feynman_checked: Vec::new(),
        human_notes: String::new(),
        created_at_ms: now,
        updated_at_ms: now,
        last_research_run_id: None,
    };
    if !paths.industry_profile_path(&profile.slug).exists() {
        save_industry(paths, &profile)?;
    } else if let Ok(mut disk) = load_industry(paths, &profile.slug) {
        if !disk
            .bound_tickers
            .iter()
            .any(|t| t.eq_ignore_ascii_case(&upper))
        {
            disk.bound_tickers.insert(0, upper);
            disk.updated_at_ms = now;
            save_industry(paths, &disk)?;
        }
        profile = disk;
    }
    Ok(profile)
}

pub fn create_or_open_from_theme(
    paths: &WorkspacePaths,
    theme: &ThemeCluster,
) -> Result<IndustryProfile> {
    let title = if theme.title.trim().is_empty() {
        theme.theme_id.trim()
    } else {
        theme.title.trim()
    };
    let slug = slugify_industry(title);
    let existing = list_industries(paths).unwrap_or_default();
    if let Some(found) = existing
        .into_iter()
        .find(|p| p.slug == slug || p.title.eq_ignore_ascii_case(title))
    {
        return Ok(found);
    }

    let mut bound = theme.tickers.clone();
    for m in &theme.members {
        bound.push(m.ticker.clone());
    }
    let now = store::now_ms();
    let profile = IndustryProfile {
        slug,
        title: title.to_owned(),
        status: IndustryStatus::Studying,
        sic_codes: theme.sic_codes.clone(),
        keywords: theme.keywords.clone(),
        bound_tickers: normalize_tickers(&bound),
        multiple_low: None,
        multiple_high: None,
        key_kpis: Vec::new(),
        feynman_questions: Vec::new(),
        feynman_checked: Vec::new(),
        human_notes: theme.user_notes.clone(),
        created_at_ms: now,
        updated_at_ms: now,
        last_research_run_id: None,
    };
    save_industry(paths, &profile)?;
    Ok(profile)
}

pub fn set_industry_status(
    paths: &WorkspacePaths,
    slug: &str,
    status: IndustryStatus,
) -> Result<IndustryProfile> {
    let mut profile = load_industry(paths, slug)?;
    profile.status = status;
    profile.updated_at_ms = store::now_ms();
    save_industry(paths, &profile)?;
    Ok(profile)
}

pub fn set_industry_valuation_anchor(
    paths: &WorkspacePaths,
    slug: &str,
    multiple_low: Option<Decimal>,
    multiple_high: Option<Decimal>,
) -> Result<IndustryProfile> {
    let mut profile = load_industry(paths, slug)?;
    profile.multiple_low = multiple_low.filter(|v| *v > Decimal::ZERO);
    profile.multiple_high = multiple_high.filter(|v| *v > Decimal::ZERO);
    profile.updated_at_ms = store::now_ms();
    save_industry(paths, &profile)?;
    Ok(profile)
}

pub fn add_bound_ticker(
    paths: &WorkspacePaths,
    slug: &str,
    ticker: &str,
) -> Result<IndustryProfile> {
    let upper = ticker.trim().to_ascii_uppercase();
    anyhow::ensure!(!upper.is_empty(), "公司代码不能为空");
    let mut profile = load_industry(paths, slug)?;
    if !profile
        .bound_tickers
        .iter()
        .any(|t| t.eq_ignore_ascii_case(&upper))
    {
        profile.bound_tickers.push(upper);
        profile.updated_at_ms = store::now_ms();
        save_industry(paths, &profile)?;
    }
    Ok(profile)
}

pub fn remove_bound_ticker(
    paths: &WorkspacePaths,
    slug: &str,
    ticker: &str,
) -> Result<IndustryProfile> {
    let upper = ticker.trim().to_ascii_uppercase();
    let mut profile = load_industry(paths, slug)?;
    profile
        .bound_tickers
        .retain(|t| !t.eq_ignore_ascii_case(&upper));
    profile.updated_at_ms = store::now_ms();
    save_industry(paths, &profile)?;
    Ok(profile)
}

pub fn toggle_feynman_check(
    paths: &WorkspacePaths,
    slug: &str,
    idx: usize,
) -> Result<IndustryProfile> {
    let mut profile = load_industry(paths, slug)?;
    sync_feynman_checked_len(&mut profile);
    if let Some(slot) = profile.feynman_checked.get_mut(idx) {
        *slot = !*slot;
        profile.updated_at_ms = store::now_ms();
        save_industry(paths, &profile)?;
    }
    Ok(profile)
}

pub fn save_human_notes(
    paths: &WorkspacePaths,
    slug: &str,
    notes: &str,
) -> Result<IndustryProfile> {
    let mut profile = load_industry(paths, slug)?;
    profile.human_notes = notes.trim().to_owned();
    profile.updated_at_ms = store::now_ms();
    save_industry(paths, &profile)?;

    let primer_path = paths.industry_primer_path(slug);
    if primer_path.exists()
        && let Ok(existing) = fs::read_to_string(&primer_path)
    {
        let body = if let Some(footer) = extract_human_notes_footer(&existing) {
            let prefix_len = existing.len().saturating_sub(footer.len());
            &existing[..prefix_len]
        } else {
            &existing
        };
        let mut rebuilt = body.trim_end().to_owned();
        if !rebuilt.is_empty() {
            rebuilt.push_str("\n\n");
        }
        rebuilt.push_str(HUMAN_NOTES_HEADER);
        rebuilt.push_str("\n\n");
        if profile.human_notes.is_empty() {
            rebuilt
                .push_str("<!-- 人类投资者亲笔备忘区：Alan 行业研究永不覆盖此标题及以下内容 -->\n");
        } else {
            rebuilt.push_str(&profile.human_notes);
            rebuilt.push('\n');
        }
        let tmp = store::atomic_temp_path(&primer_path);
        if fs::write(&tmp, rebuilt.as_bytes())
            .and_then(|_| fs::rename(&tmp, &primer_path))
            .is_err()
        {
            let _ = fs::remove_file(&tmp);
        }
    }

    Ok(profile)
}

pub fn compute_industry_strike_price(
    row: &UniverseRow,
    multiple_low: Option<Decimal>,
) -> (Option<Decimal>, Option<Decimal>) {
    let Some(base_strike_18x) = row.strike_price else {
        return (None, None);
    };
    if base_strike_18x <= Decimal::ZERO {
        return (None, None);
    }
    let adj_strike = match multiple_low {
        Some(mult) if mult > Decimal::ZERO => {
            (base_strike_18x * mult / Decimal::from(18)).round_dp(2)
        }
        _ => base_strike_18x,
    };
    if adj_strike <= Decimal::ZERO {
        return (None, None);
    }
    let dist_pct = row
        .latest_price
        .map(|price| ((price - adj_strike) / adj_strike * Decimal::from(100)).round_dp(2));
    (Some(adj_strike), dist_pct)
}

fn normalize_tickers(raw: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    for item in raw {
        let up = item.trim().to_ascii_uppercase();
        if !up.is_empty() && seen.insert(up.clone()) {
            out.push(up);
        }
    }
    out
}

pub(crate) fn sync_feynman_checked_len(profile: &mut IndustryProfile) {
    profile
        .feynman_checked
        .resize(profile.feynman_questions.len(), false);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Bucket;
    use std::path::Path;

    fn temp_paths(label: &str) -> WorkspacePaths {
        let root = std::env::temp_dir().join(format!(
            "m3_industry_test_{label}_{}_{}",
            std::process::id(),
            store::now_ms()
        ));
        let _ = fs::remove_dir_all(&root);
        let paths = WorkspacePaths::from_root(root);
        let _ = fs::create_dir_all(&paths.workbench_dir);
        let _ = fs::create_dir_all(&paths.data_dir);
        paths
    }

    fn sample_universe_row(ticker: &str, sic: u32, price: i64, strike_18x: i64) -> UniverseRow {
        UniverseRow {
            cik: format!("000000{sic}"),
            ticker: ticker.to_owned(),
            name: format!("{ticker} Corp"),
            exchange: "NASDAQ".to_owned(),
            sic: Some(sic),
            sic_description: Some("Prepackaged Software".to_owned()),
            is_financial: false,
            latest_filing_end: Some("2026-02-01".to_owned()),
            years_of_data: 5,
            quality_score: Decimal::from(88),
            in_price_pool: true,
            in_watchlist: false,
            bucket_opt: None,
            latest_price: Some(Decimal::from(price)),
            strike_price: Some(Decimal::from(strike_18x)),
            distance_to_strike_pct: Some(Decimal::ZERO),
            strict_fcf_yield_pct: Some(Decimal::from(6)),
            roic_5y_avg_pct: Some(Decimal::from(24)),
            roic_5y_stddev_pct: None,
            gross_margin_5y_avg_pct: Some(Decimal::from(72)),
            gross_margin_5y_stddev_pct: None,
            share_change_3y_pct: Some(Decimal::from(-2)),
            sbc_to_ocf_pct: Some(Decimal::from(10)),
            net_debt_to_oe: Some(Decimal::from(0)),
            strict_owner_earnings: Some(Decimal::from(1_000_000_000)),
            warnings_count: 0,
            warnings_json: "[]".to_owned(),
            raw_sha256: None,
        }
    }

    #[test]
    fn slugify_handles_ascii_and_cjk() {
        assert_eq!(
            slugify_industry("EDA & Semiconductor IP"),
            "eda-semiconductor-ip"
        );
        let zh = slugify_industry("工业软件与医疗耗材");
        assert!(zh.starts_with("ind-"), "expected ind- prefix, got {zh}");
        assert_eq!(zh, slugify_industry("工业软件与医疗耗材"));
    }

    #[test]
    fn primer_writer_preserves_human_notes_verbatim() {
        let paths = temp_paths("human_notes");
        let slug = "eda-ip";
        let primer_path = paths.industry_primer_path(slug);
        fs::create_dir_all(Path::new(&primer_path).parent().unwrap()).unwrap();
        let initial = "# Old Title\n\nOld AI body\n\n## Human Notes\n\nMy private moat note!\nDo not touch this line.\n";
        fs::write(&primer_path, initial).unwrap();

        write_industry_primer_preserving_human_notes(
            &paths,
            slug,
            "# New Title\n\nFresh Alan analysis",
            None,
        )
        .unwrap();

        let after = fs::read_to_string(&primer_path).unwrap();
        assert!(after.contains("Fresh Alan analysis"));
        assert!(after.contains("## Human Notes\n\nMy private moat note!\nDo not touch this line."));
        let _ = fs::remove_dir_all(&paths.repo_root);
    }

    #[test]
    fn find_industry_prefers_bound_ticker_over_sic() {
        let now = 1_700_000_000_000;
        let a = IndustryProfile {
            slug: "sic-7372".into(),
            title: "Software".into(),
            status: IndustryStatus::Studying,
            sic_codes: vec![7372],
            keywords: vec![],
            bound_tickers: vec!["ORCL".into()],
            multiple_low: None,
            multiple_high: None,
            key_kpis: vec![],
            feynman_questions: vec![],
            feynman_checked: vec![],
            human_notes: String::new(),
            created_at_ms: now,
            updated_at_ms: now,
            last_research_run_id: None,
        };
        let b = IndustryProfile {
            slug: "eda".into(),
            title: "EDA".into(),
            status: IndustryStatus::InsideCircle,
            sic_codes: vec![7373],
            keywords: vec![],
            bound_tickers: vec!["SNPS".into()],
            multiple_low: Some(Decimal::from(20)),
            multiple_high: Some(Decimal::from(26)),
            key_kpis: vec![],
            feynman_questions: vec![],
            feynman_checked: vec![],
            human_notes: String::new(),
            created_at_ms: now,
            updated_at_ms: now,
            last_research_run_id: None,
        };
        let all = vec![a, b];
        let hit = find_industry_for_company(&all, "snps", Some(7372)).unwrap();
        assert_eq!(hit.slug, "eda");
        let sic_hit = find_industry_for_company(&all, "ADBE", Some(7372)).unwrap();
        assert_eq!(sic_hit.slug, "sic-7372");
    }

    #[test]
    fn compute_industry_strike_scales_18x_anchor_to_human_multiple() {
        let row = sample_universe_row("NUE", 3312, 110, 180);
        let (strike, dist) = compute_industry_strike_price(&row, Some(Decimal::from(12)));
        assert_eq!(strike, Some(Decimal::from(120)));
        assert_eq!(dist, Some(Decimal::new(-833, 2)));

        let (default_strike, _) = compute_industry_strike_price(&row, None);
        assert_eq!(default_strike, Some(Decimal::from(180)));
    }

    #[test]
    fn create_from_company_and_theme_roundtrips() {
        let paths = temp_paths("create_company_theme");
        let ind = create_or_open_from_company(
            &paths,
            "snps",
            "Synopsys",
            Some(7372),
            Some("Prepackaged Software"),
        )
        .unwrap();
        assert_eq!(ind.bound_tickers, vec!["SNPS".to_owned()]);
        assert_eq!(ind.status, IndustryStatus::Studying);

        let updated = set_industry_valuation_anchor(
            &paths,
            &ind.slug,
            Some(Decimal::from(18)),
            Some(Decimal::from(24)),
        )
        .unwrap();
        assert_eq!(updated.valuation_anchor_label().as_deref(), Some("18–24x"));

        let theme = ThemeCluster {
            theme_id: "t-eda".into(),
            title: "EDA Oligopoly".into(),
            archetype: "Tollbooth".into(),
            sic_codes: vec![7372],
            keywords: vec!["eda".into()],
            summary: String::new(),
            tickers: vec!["CDNS".into(), "SNPS".into()],
            members: vec![],
            avg_similarity: 0.9,
            median_roic_5y_pct: None,
            strike_zone_count: 0,
            user_notes: "Human theme note".into(),
        };
        let promoted = create_or_open_from_theme(&paths, &theme).unwrap();
        assert_eq!(
            promoted.bound_tickers,
            vec!["CDNS".to_owned(), "SNPS".to_owned()]
        );
        assert_eq!(promoted.human_notes, "Human theme note");

        write_industry_primer_preserving_human_notes(&paths, &ind.slug, "# Primer", None).unwrap();
        assert!(paths.industry_profile_path(&ind.slug).exists());
        assert!(paths.industry_primer_path(&ind.slug).exists());
        assert!(delete_industry(&paths, &ind.slug).unwrap());
        assert!(!paths.industry_profile_path(&ind.slug).exists());
        assert!(!paths.industry_primer_path(&ind.slug).exists());
        assert!(!delete_industry(&paths, &ind.slug).unwrap());
        assert!(delete_industry(&paths, "../escape").is_err());

        let _ = Bucket::Core;
        let _ = fs::remove_dir_all(&paths.repo_root);
    }
}
