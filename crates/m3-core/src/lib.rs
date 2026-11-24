//! Core domain library for Project M3 (Munger Workbench).
//!
//! Contains deterministic `rust_decimal` financial calculations, SEC EDGAR and market data ingestion,
//! DuckDB/Parquet lakehouse storage (`lake`, `lake_l1`, `lake_l2`), EmbeddedAlan orchestration,
//! and platform-agnostic configuration/update state via [`M3Paths`].

pub mod alan_interrogator;
pub mod app_config;
pub mod budget;
pub mod calculator;
pub mod edgar;
pub mod events;
pub mod i18n;
pub mod industry;
pub mod industry_research;
pub mod lake;
pub mod lake_l1;
pub mod lake_l2;
pub mod latent;
pub mod market;
pub mod model;
pub mod research;
pub mod store;
pub mod universe;
pub mod update;

pub use app_config::{AppConfig, M3Paths};
pub use i18n::UiLang;
pub use store::WorkspacePaths;
