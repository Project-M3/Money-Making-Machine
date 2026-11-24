//! Company Dossier overlay shell (Financials & Memo / Deep Research & Alan / XBRL Inspector).

use super::{agent::agent_view, chart::chart_placeholder, skeleton_placeholder};
use crate::components::{action_button, seg_group, seg_tab};
use crate::stores::{AppStore, CompanyViewTab};
use crate::theme::*;
use nsh_core::prelude::*;

pub fn company_view(store: AppStore) -> impl IntoElement {
    view()
        .flex_1()
        .min_h(px(0.0))
        .flex()
        .flex_col()
        .child(
            view()
                .px(px(16.0))
                .py(px(8.0))
                .bg(rgb(BG_TOOLBAR))
                .border_b_1()
                .border_color(rgb(BORDER_HAIRLINE))
                .flex()
                .flex_row()
                .items_center()
                .justify_between()
                .child(
                    view()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(10.0))
                        .child(action_button(
                            move || store.lang.get().t("← 返回", "← Back").to_owned(),
                            false,
                            move || store.close_company_overlay(),
                        ))
                        .child(
                            text(move || {
                                let t = store.selected_ticker.get();
                                if t.is_empty() {
                                    "Research Desk".to_owned()
                                } else {
                                    t
                                }
                            })
                            .font_family(FONT_MONO_NUM)
                            .text_size(px(TEXT_MD))
                            .font_weight(FontWeight::BOLD)
                            .text_color(rgb(INK_HERO)),
                        ),
                )
                .child(dynamic(move || {
                    let cur = store.company_view_tab.get();
                    let l = store.lang.get();
                    seg_group()
                        .child(seg_tab(
                            l.t("1 财报与活研报", "1 Financials & Memo"),
                            cur == CompanyViewTab::FinancialsMemo,
                            None,
                            false,
                            move || store.company_view_tab.set(CompanyViewTab::FinancialsMemo),
                        ))
                        .child(seg_tab(
                            l.t("2 Deep Research & Alan", "2 Deep Research & Alan"),
                            cur == CompanyViewTab::DeepResearchAlan,
                            Some((VIOLET_FG, VIOLET_BG)),
                            false,
                            move || store.company_view_tab.set(CompanyViewTab::DeepResearchAlan),
                        ))
                        .child(seg_tab(
                            l.t("3 XBRL 底表", "3 XBRL Ledger"),
                            cur == CompanyViewTab::XbrlInspector,
                            None,
                            false,
                            move || store.company_view_tab.set(CompanyViewTab::XbrlInspector),
                        ))
                })),
        )
        .child(dynamic(move || match store.company_view_tab.get() {
            CompanyViewTab::FinancialsMemo => view()
                .flex_1()
                .min_h(px(0.0))
                .flex()
                .flex_col()
                .child(chart_placeholder(store))
                .child(skeleton_placeholder(
                    store,
                    "FINANCIALS & LIVING MEMO",
                    "确定性财报、Charlie 沙盒与 Living Memo",
                    "Deterministic Financials, Charlie Sandbox & Living Memo",
                    "已接入 Charlie 沙盒假设、TTM/FY 切换与 K 线数据。",
                    "Phase 1 store wired: Charlie Sandbox, TTM/FY toggle & price history.",
                    move || {
                        format!(
                            "{} · {}x / {}% / {}%",
                            store.selected_ticker.get(),
                            store.sandbox_multiple.get(),
                            store.sandbox_target_return.get(),
                            store.sandbox_expected_growth.get(),
                        )
                    },
                    "保存沙盒假设",
                    "Save Sandbox",
                    move || store.save_sandbox_assumptions(),
                ))
                .into_element(),
            CompanyViewTab::DeepResearchAlan => agent_view(store).into_element(),
            CompanyViewTab::XbrlInspector => skeleton_placeholder(
                store,
                "XBRL AUDIT LEDGER",
                "XBRL 原账与 SHA-256 算术溯源",
                "XBRL Filings & SHA-256 Provenance",
                "已接入 selected_dossier 与年报明细数据。",
                "Phase 1 store wired: XBRL filings & deterministic audit rows.",
                move || store.selected_ticker.get(),
                "切换口径 TTM/FY",
                "Toggle TTM/FY",
                move || store.toggle_basis(),
            )
            .into_element(),
        }))
}
