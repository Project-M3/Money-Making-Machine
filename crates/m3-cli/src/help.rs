use m3_core::update;

/// Returns top-level CLI help text formatted for terminal readability.
pub fn render_root_help() -> String {
    let ver = update::current_binary_version();
    format!(
        "m3 — Project M3 (Money · Making · Machine) v{ver}
Local-first, event-driven Munger value investing workbench CLI

USAGE:
  m3 [GLOBAL OPTIONS] <COMMAND> [ARGS]

GLOBAL OPTIONS:
  -h, --help          Show top-level or command help
  -V, --version       Show short version (use `--version` for build details)
  --pretty            Pretty-print JSON output (auto-enabled on interactive TTYs)
  --compact, --json   Emit compact single-line JSON (default when piped)

COMMANDS:
  Workbench:
    status            Workspace summary, Circle companies, and data freshness
    sync              Sync SEC EDGAR XBRL financials and Yahoo daily prices
    report            Deterministic Charlie Calculator report for a ticker
    dossier           Load human-agent interrogation dossier for a ticker
    add               Add or update a company in your Circle watchlist
    journal           List or record paper decision journal entries
    industry          List or inspect Industry Research Studio profiles
    start             Launch the M3 desktop app (macOS)

  Lakehouse & Events:
    universe          Full-US DuckDB/Parquet lakehouse sync, screen, peers, similar
    events            Catch up and inspect L1 SEC filing / news event stream
    inbox             List and triage L1 Inbox signals
    themes            Sync and list commercial archetype theme clusters

  Research & Guardrails:
    research          Run or inspect L2 Alan Deep Research sessions
    budget            Inspect monthly/per-run AI research budget & usage
    briefing          Show, run, or list calm daily/weekly Circle briefings
    triage            Inspect or run L1.5 Circle Filing Watchdog signal triage
    falsification     List, add, or remove human falsification metrics on a dossier

  Alan & Desktop Copilot:
    alan              Inspect/test Alan connection, ask scoped questions, manage sessions
    app               Inspect and control the running M3 desktop window via IPC

  Agent Integration:
    mcp               Run full-capability (or --read-only) JSON-RPC 2.0 MCP server over stdio

  System & Maintenance:
    storage           Inspect and clean local SEC/DuckDB/cache storage footprint
    doctor            Check binary, workspace, lakehouse, storage, and host tools
    workspace         Show or set persistent workbench root path
    update            Check or apply signed/verified macOS app & CLI updates
    version           Show structured JSON version and build metadata
    help              Show help for a specific command

Run 'm3 help <command>' or 'm3 <command> --help' for command details.
"
    )
}

/// Returns detailed help text for a single CLI command, or `None` if unknown.
pub fn render_command_help(command: &str) -> Option<String> {
    let text = match command {
        "status" => {
            "m3 status — Workspace summary, Circle companies, and sync freshness

USAGE:
  m3 status [GLOBAL OPTIONS]

EXAMPLES:
  m3 status
  m3 status --pretty
"
        }
        "sync" => {
            "m3 sync — Sync SEC EDGAR XBRL financials and Yahoo daily prices

USAGE:
  m3 sync [--ticker <SYM>] [--prices-only]

OPTIONS:
  --ticker <SYM>      Sync only a single ticker in the watchlist
  --prices-only       Refresh daily price snapshots without fetching SEC XBRL

EXAMPLES:
  m3 sync
  m3 sync --ticker MCO
  m3 sync --prices-only
"
        }
        "report" => {
            "m3 report — Deterministic Charlie Calculator report for a ticker

USAGE:
  m3 report <TICKER>

EXAMPLES:
  m3 report AAPL
  m3 report COST --pretty
"
        }
        "dossier" => {
            "m3 dossier — Load human-agent interrogation dossier for a ticker

USAGE:
  m3 dossier <TICKER>

EXAMPLES:
  m3 dossier AAPL
"
        }
        "add" => {
            "m3 add — Add or update a company in your Circle watchlist

USAGE:
  m3 add <TICKER> [--bucket core|in|watch|out|too_hard] [--name <TEXT>]
                  [--circle <TEXT>] [--reason <TEXT>]

OPTIONS:
  --bucket <BUCKET>   Circle bucket: core (alias: in), watch, out, too_hard (default: watch)
  --name <TEXT>       Override company display name
  --circle <TEXT>     Circle / thesis label
  --reason <TEXT>     Note or Too Hard rationale

EXAMPLES:
  m3 add MCO --bucket core --circle \"Credit Ratings & Analytics\"
  m3 add INTC --bucket too_hard --reason \"Capital-intensive foundry execution risk\"
"
        }
        "journal" => {
            "m3 journal — List or record paper decision journal entries

USAGE:
  m3 journal [list]
  m3 journal record --ticker <SYM> --action <ACTION> --note <TEXT>
                    [--shares <DECIMAL>] [--falsification <TEXT>]

SUBCOMMANDS:
  list                Show recorded decision journal entries and positions (default)
  record              Append a pre-committed paper research journal entry

OPTIONS (record):
  --ticker <SYM>      Ticker symbol
  --action <ACTION>   wait | out | too_hard | paper_buy | paper_sell
  --note <TEXT>       Pre-commitment thesis or wait rationale
  --shares <DECIMAL>  Share count for paper_buy / paper_sell
  --falsification     Observable metric or condition that falsifies the thesis

EXAMPLES:
  m3 journal list
  m3 journal record --ticker COST --action wait --note \"Waiting for wider margin of safety\"
"
        }
        "industry" => {
            "m3 industry — List or inspect Industry Research Studio profiles

USAGE:
  m3 industry [list]
  m3 industry show <SLUG>

SUBCOMMANDS:
  list                List all saved industry study profiles (default)
  show <SLUG>         Show full industry profile JSON and Markdown primer

EXAMPLES:
  m3 industry list
  m3 industry show enterprise-cad-software
"
        }
        "universe" => {
            "m3 universe — Full-US DuckDB/Parquet lakehouse operations

USAGE:
  m3 universe [status]
  m3 universe sync [--full] [--prices-top <N>] [--skip-prices]
  m3 universe screen [SCREEN OPTIONS]
  m3 universe peers <TICKER> [--limit <N>]
  m3 universe similar <TICKER_OR_QUERY> [--ticker] [SIMILAR OPTIONS]

SUBCOMMANDS:
  status              Show lakehouse company/fact/price counts (default)
  sync                Build or update L0 SEC EDGAR + DuckDB/Parquet lakehouse
  screen              Filter and rank companies by deterministic metrics
  peers <TICKER>      Compare a company against its SIC peers
  similar <TEXT>      Hybrid financial + 256d cosine/BM25 similarity search

SCREEN OPTIONS:
  --min-roic <N>              Minimum 5Y average ROIC (%)
  --max-sbc-ocf <N>           Maximum SBC / OCF (%)
  --max-share-change-3y <N>   Maximum 3Y share count change (%)
  --max-net-debt-to-oe <N>    Maximum net debt / Owner Earnings ratio
  --min-years <N>             Minimum years of SEC data (1..=20)
  --exclude-financials        Exclude SIC 6000-6799 financials (default)
  --include-financials        Include financial sector companies
  --positive-oe-only          Require positive strict Owner Earnings
  --price-pool-only           Require latest price snapshot
  --strike-only               Filter to companies inside Strike Zone
  --query <TEXT>              Case-insensitive ticker/name/SIC filter
  --sort <COL>                quality|roic|fcf_yield|distance|share_change|sbc_ocf|price|strike|warnings|ticker|name|sic|oe
  --asc | --desc              Sort direction
  --limit <N>                 Max rows returned (1..=5000, default: 200)

EXAMPLES:
  m3 universe sync --full --prices-top 400
  m3 universe screen --min-roic 15 --max-sbc-ocf 25 --limit 25
  m3 universe peers MCO --limit 15
  m3 universe similar \"specialty chemical moat\" --min-roic 15 --limit 10
"
        }
        "events" => {
            "m3 events — Catch up and inspect L1 SEC filing / news event stream

USAGE:
  m3 events [list] [--ticker <SYM>] [--type <TYPE>] [--unprocessed-only] [--limit <N>]
  m3 events catchup [--bootstrap] [--skip-sec] [--skip-news] [--skip-docs]
                    [--max-ciks <N>] [--max-docs <N>] [--max-news-tickers <N>]

SUBCOMMANDS:
  list                List ingested L1 events from DuckDB (default)
  catchup             Incrementally ingest SEC filings, 10-K/Q chunks, and RSS news

EXAMPLES:
  m3 events catchup --bootstrap --max-ciks 25 --max-docs 12
  m3 events list --ticker AAPL --limit 20
"
        }
        "inbox" => {
            "m3 inbox — List and triage L1 Inbox signals

USAGE:
  m3 inbox [list] [--status <STATUS>] [--kind <KIND>] [--limit <N>]
  m3 inbox triage <SIGNAL_ID> --status <TARGET_STATUS> [--promote-watch]

OPTIONS:
  --status <STATUS>   list: unread|all|watch|too_hard|researching|archived (default: all)
                      triage: unread|researching|watch|too_hard|archived
  --kind <KIND>       discovery | ripple | strike | red_flag
  --limit <N>         Max signals returned (1..=5000, default: 50)
  --promote-watch     Add primary_ticker to watchlist WATCH bucket when triaging

EXAMPLES:
  m3 inbox list --status unread --limit 20
  m3 inbox triage sig-123 --status watch --promote-watch
"
        }
        "research" => {
            "m3 research — Run or inspect L2 Alan Deep Research sessions

USAGE:
  m3 research [status] [--ticker <SYM>] [--limit <N>]
  m3 research run <TICKER> [--focus <TEXT>] [--trigger manual|inbox|cli]

SUBCOMMANDS:
  status              Reclaim stale `running` runs (>= 15m) if RW DuckDB is free, list runs & budget (default)
  run <TICKER>        Execute budget-gated 4-perspective Deep Research and write memo

EXAMPLES:
  m3 research status --ticker MCO --limit 10
  m3 research run MCO --focus \"Pricing power and structured finance cycle\"
"
        }
        "budget" => {
            "m3 budget — Inspect monthly/per-run AI research budget & usage

USAGE:
  m3 budget [status]

EXAMPLES:
  m3 budget status
"
        }
        "briefing" => {
            "m3 briefing — Show, run, or list calm daily/weekly Circle briefings

USAGE:
  m3 briefing [show] [--period daily|weekly]
  m3 briefing run [--period daily|weekly] [--no-llm]
  m3 briefing list [--period daily|weekly] [--limit <N>]

SUBCOMMANDS:
  show                Show latest saved briefing for period, or 0-token preview (default)
  run                 Build, optionally summarize with Alan, and persist a briefing
  list                List persisted calm briefings (newest first)

OPTIONS:
  --period <PERIOD>   Briefing cadence: daily | weekly (default: daily for show/run)
  --no-llm            Skip Alan executive summary (0 LLM tokens)
  --limit <N>         Maximum briefings returned by `list` (1..=50, default: 10)

EXAMPLES:
  m3 briefing
  m3 briefing show --period weekly
  m3 briefing run --period daily --no-llm
  m3 briefing list --limit 5
"
        }
        "triage" => {
            "m3 triage — Inspect or run L1.5 Circle Filing Watchdog signal triage

USAGE:
  m3 triage [status] [--ticker <SYM>] [--verdict noise|material|falsification_alert] [--limit <N>]
  m3 triage run [--limit <N>] [--dry-run]

SUBCOMMANDS:
  status              List recent signal triage records and daily quota usage (default)
  run                 Evaluate pending WATCH/CORE Circle signals against falsification metrics

OPTIONS:
  --ticker <SYM>      Filter `status` records by ticker symbol
  --verdict <VERDICT> Filter `status`: noise | material | falsification_alert
  --limit <N>         Max records returned (`status`, default: 50) or evaluated (`run`, default: 10)
  --dry-run           Preview triage prompts/records without calling Alan or writing DuckDB

EXAMPLES:
  m3 triage status --ticker MCO
  m3 triage run --dry-run --limit 5
"
        }
        "falsification" => {
            "m3 falsification — List, add, or remove human falsification metrics on a dossier

USAGE:
  m3 falsification <TICKER> [list]
  m3 falsification <TICKER> add <TEXT>
  m3 falsification <TICKER> remove <INDEX>

SUBCOMMANDS:
  list                Show `human_judgment.falsification_metrics` for <TICKER> (default)
  add <TEXT>          Append a falsification metric (deduplicated, preserves human sovereignty)
  remove <INDEX>      Remove the 0-based falsification metric at <INDEX>

EXAMPLES:
  m3 falsification MCO list
  m3 falsification MCO add \"Operating margin < 35%\"
  m3 falsification MCO remove 0
"
        }
        "mcp" => {
            "m3 mcp — Run full-capability (or --read-only) JSON-RPC 2.0 MCP server over stdio

USAGE:
  m3 mcp [--workspace <PATH>] [--read-only]

OPTIONS:
  --workspace <PATH>  Override M3 workspace root (defaults to M3_REPO_ROOT / cwd / configured root)
  --read-only         Filter tools/list to readOnlyHint=true tools and reject write/action calls

NOTES:
  - Reads line-delimited JSON-RPC 2.0 requests from stdin and writes responses to stdout.
  - All diagnostics go to stderr only; stdout carries JSON-RPC frames exclusively.
  - Default mode exposes 40 tools (24 read-only + 16 controlled action/Alan/desktop tools)
    with accurate MCP annotations (`readOnlyHint`, `destructiveHint: false`).
  - Never mutates `human_judgment` (except explicit `m3_falsification_update`), never auto-signs
    `feynman_checked_at`, never overwrites `## Human Notes`, and never deletes `personal_workbench`.

EXAMPLES:
  m3 mcp
  m3 mcp --read-only
  m3 mcp --workspace ~/Documents/m3-workbench
"
        }
        "alan" => {
            "m3 alan — Inspect/test Alan connection, ask scoped questions, and manage sessions

USAGE:
  m3 alan [status]
  m3 alan test [--profile <ID>]
  m3 alan ask --scope <SCOPE> \"<QUESTION>\" [--lang zh|en] [--timeout <SECS>]
  m3 alan session show --scope <SCOPE>
  m3 alan session clear --scope <SCOPE>

SCOPES:
  company:<TICKER>    Company dossier & computed report scope (e.g. company:COST)
  industry:<SLUG>     Industry Research Studio scope (e.g. industry:retail)
  briefing:latest     Latest Circle briefing scope (or briefing:<ID>)
  portfolio           Global Circle of Competence portfolio scope

OPTIONS:
  --profile <ID>      Target Alan connection profile ID for `test`
  --scope <SCOPE>     Target AlanScope (`company:TICKER|industry:SLUG|briefing:latest|portfolio`)
  --lang zh|en        Override response language (defaults to configured UI language)
  --timeout <SECS>    Max seconds to wait for scoped reply (1..=600, default: 120)

EXAMPLES:
  m3 alan status
  m3 alan test
  m3 alan ask --scope company:COST \"Verify Costco membership moat [report:COST]\" --lang en
  m3 alan session show --scope company:COST
  m3 alan session clear --scope company:COST
"
        }
        "app" => {
            "m3 app — Inspect and control the running M3 desktop window via IPC

USAGE:
  m3 app ping [--timeout <SECS>]
  m3 app state [--timeout <SECS>]
  m3 app navigate [--tab <TAB>] [--ticker <T>] [--industry <SLUG>] [--timeout <SECS>]
  m3 app alan [--mode closed|split|immersive] [--tab interrogate|feynman] [--scope <SCOPE>]
              [--pin true|false] [--anchor <LABEL>] [--prompt <TEXT>] [--submit] [--timeout <SECS>]
  m3 app action <ACTION> [--ticker <T>] [--timeout <SECS>]
  m3 app screenshot <PATH> [--timeout <SECS>]

OPTIONS:
  --tab <TAB>         Workspace tab (`inbox|universe|industry|baskets|journal|settings`)
                      or Alan canvas tab (`interrogate|feynman`) under `m3 app alan`
  --ticker <T>        Company ticker (pass `\"\"` to close company overlay in `navigate`)
  --industry <SLUG>   Industry profile slug
  --mode <MODE>       Alan stage mode: closed | split | immersive
  --scope <SCOPE>     AlanScope (`company:TICKER|industry:SLUG|briefing:latest|portfolio`)
  --pin true|false    Lock or unlock Alan auto-follow scope pin
  --anchor <LABEL>    Optional UI anchor label for Alan context
  --prompt <TEXT>     Pre-fill Alan input prompt
  --submit            Submit the prompt immediately to Alan
  --timeout <SECS>    IPC socket timeout in seconds (1..=120, default: 5)

ACTIONS:
  toggle_sidebar | open_palette | close_palette | close_company |
  sync | refresh_briefing | reset_alan | deep_research

EXAMPLES:
  m3 app ping
  m3 app state
  m3 app navigate --tab universe --ticker COST
  m3 app alan --mode split --scope company:COST --pin true
  m3 app screenshot /tmp/m3-window.png
"
        }
        "themes" => {
            "m3 themes — Sync and list commercial archetype theme clusters

USAGE:
  m3 themes [list | sync]

SUBCOMMANDS:
  list                Read saved theme clusters from workbench/themes.json (default)
  sync                Deterministically recompute theme clusters from DuckDB lakehouse

EXAMPLES:
  m3 themes sync
  m3 themes list
"
        }
        "storage" => {
            "m3 storage — Inspect and clean local SEC, DuckDB, and cache storage

USAGE:
  m3 storage [status]
  m3 storage clean <CATEGORY>

SUBCOMMANDS:
  status              Report disk usage across all local M3 storage buckets (default)
  clean <CATEGORY>    Reclaim rebuildable cache data for a single category or `all`

CATEGORIES:
  sec_bulk_archives   (alias: sec-bulk)           SEC submissions.zip / companyfacts.zip archives
  sec_filings_cache   (alias: sec-filings)        Cached raw 10-K / 10-Q / 8-K text files
  duck_db_lakehouse   (aliases: duckdb_lakehouse, duckdb)  m3.duckdb and ZSTD Parquet generations
  quotes_and_cache    (alias: quotes)             Raw quotes, computed reports, updater cache
  all                                       Clean all 4 rebuildable cache categories above
  personal_workbench                        PROTECTED: refused by cleanup (watchlist, dossiers, memos, journal)

EXAMPLES:
  m3 storage status
  m3 storage clean sec_bulk_archives
  m3 storage clean all
"
        }
        "doctor" => {
            "m3 doctor — Diagnose binary, workspace, lakehouse, Alan connection, stale runs, storage, and host tools

USAGE:
  m3 doctor [GLOBAL OPTIONS]

EXAMPLES:
  m3 doctor
  m3 doctor --pretty
"
        }
        "workspace" => {
            "m3 workspace — Show or set persistent workbench root path

USAGE:
  m3 workspace [show]
  m3 workspace set <PATH>

EXAMPLES:
  m3 workspace show
  m3 workspace set ~/Documents/m3-workbench
"
        }
        "update" => {
            "m3 update — Check or apply signed/verified macOS app & CLI updates

USAGE:
  m3 update [--check] [--channel stable|beta] [--version <VER>] [--force]
            [--app-dir <DIR>] [--bin-dir <DIR>] [--no-cli-link] [--manifest-url <URL>]
  m3 update channel [stable|beta]

OPTIONS:
  --check                 Check latest available release without installing
  --channel <CH>          Override release channel (`stable` or `beta`)
  --version <VER>         Install a specific version tag
  --force                 Reinstall even if already on target version
  --app-dir <DIR>         Override target .app installation directory
  --bin-dir <DIR>         Override ~/.local/bin symlink directory
  --no-cli-link           Skip updating the `m3` CLI symlink
  --manifest-url <URL>    Override release manifest URL

EXAMPLES:
  m3 update --check
  m3 update
  m3 update channel beta
"
        }
        "version" => {
            "m3 version — Show structured JSON version and build metadata

USAGE:
  m3 version

EXAMPLES:
  m3 version
  m3 -V
  m3 --version
"
        }
        "start" => {
            "m3 start — Launch the M3 macOS desktop app

USAGE:
  m3 start

EXAMPLES:
  m3 start
"
        }
        "help" => {
            "m3 help — Show help for a specific command

USAGE:
  m3 help [COMMAND]

EXAMPLES:
  m3 help
  m3 help storage
"
        }
        _ => return None,
    };
    Some(text.to_owned())
}
