//! Compile-time Chinese/English UI localization (`UiLang`).

use serde::{Deserialize, Serialize};
#[cfg(target_os = "macos")]
use std::process::{Command, Stdio};

/// Supported desktop UI languages.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum UiLang {
    #[default]
    Zh,
    En,
}

impl UiLang {
    /// Select Chinese or English copy inline at zero runtime cost.
    #[inline]
    pub const fn t(self, zh: &'static str, en: &'static str) -> &'static str {
        match self {
            Self::Zh => zh,
            Self::En => en,
        }
    }

    /// Parse explicit or locale-style language strings (e.g., `"zh"`, `"en_US.UTF-8"`).
    pub fn parse(raw: &str) -> Option<Self> {
        let lower = raw.trim().to_ascii_lowercase();
        if lower.is_empty() {
            return None;
        }
        if lower.starts_with("en") || lower == "english" {
            return Some(Self::En);
        }
        if lower.starts_with("zh") || lower == "chinese" || lower == "中文" {
            return Some(Self::Zh);
        }
        None
    }

    /// Detect system preference from environment variables or macOS defaults, falling back to `Zh`.
    pub fn detect_system_lang() -> Self {
        for var in ["M3_LANG", "LC_ALL", "LC_MESSAGES", "LANG"] {
            if let Ok(val) = std::env::var(var)
                && let Some(lang) = Self::parse(&val)
            {
                return lang;
            }
        }
        #[cfg(target_os = "macos")]
        if let Ok(out) = Command::new("/usr/bin/defaults")
            .args(["read", "-g", "AppleLanguages"])
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output()
            && out.status.success()
        {
            let text = String::from_utf8_lossy(&out.stdout);
            for line in text.lines() {
                let cleaned = line
                    .trim()
                    .trim_matches(|c| c == '"' || c == ',' || c == '(' || c == ')');
                if let Some(lang) = Self::parse(cleaned) {
                    return lang;
                }
            }
        }
        Self::Zh
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ui_lang_parse_t_and_serde_roundtrip() {
        assert_eq!(UiLang::parse("zh"), Some(UiLang::Zh));
        assert_eq!(UiLang::parse("zh_CN.UTF-8"), Some(UiLang::Zh));
        assert_eq!(UiLang::parse("en"), Some(UiLang::En));
        assert_eq!(UiLang::parse("en-US"), Some(UiLang::En));
        assert_eq!(UiLang::parse(""), None);
        assert_eq!(UiLang::parse("fr"), None);

        assert_eq!(UiLang::Zh.t("设置", "Settings"), "设置");
        assert_eq!(UiLang::En.t("设置", "Settings"), "Settings");

        let zh_json = serde_json::to_string(&UiLang::Zh).unwrap();
        let en_json = serde_json::to_string(&UiLang::En).unwrap();
        assert_eq!(zh_json, "\"zh\"");
        assert_eq!(en_json, "\"en\"");
        assert_eq!(
            serde_json::from_str::<UiLang>(&zh_json).unwrap(),
            UiLang::Zh
        );
        assert_eq!(
            serde_json::from_str::<UiLang>(&en_json).unwrap(),
            UiLang::En
        );
    }
}
