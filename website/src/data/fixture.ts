export type WorkbenchTab =
  | 'inbox'
  | 'universe'
  | 'industry'
  | 'baskets'
  | 'journal'
  | 'research'
  | 'settings';
export type CompanySubTab = 'memo' | 'alan' | 'xbrl';
export type InboxFilter = 'unread' | 'all' | 'discovery' | 'ripple' | 'strike' | 'archived';
export type UniverseFilter = 'all' | 'high_roic' | 'low_sbc' | 'strike';
export type UniverseSearchMode = 'keyword' | 'similar';
export type PerspectiveKey = 'moat' | 'accounting' | 'latent_peers' | 'premortem';
export type IndustryPerspectiveKey = 'value_chain' | 'moat' | 'failure_modes' | 'valuation_kpis';
export type IndustryStatusKey = 'inside_circle' | 'studying' | 'too_hard';

export interface IndustryStudyRecord {
  slug: string;
  title: string;
  sicLabel: string;
  status: IndustryStatusKey;
  statusLabel: string;
  anchorLabel: string;
  boundTickers: string[];
  feynmanChecklist: Array<{ text: string; checked: boolean }>;
  peers: Array<{
    ticker: string;
    name: string;
    roic5y: string;
    gm5y: string;
    fcfYield: string;
    sbcOcf: string;
    price: string;
    refStrike: string;
  }>;
  perspectives: Record<IndustryPerspectiveKey, { title: string; summary: string; points: string[] }>;
  secCitations: string[];
  webCitations: Array<{ title: string; url: string }>;
  humanNotes: string;
}

export interface InboxEvent {
  id: string;
  ticker: string;
  company: string;
  category: 'discovery' | 'ripple' | 'strike' | 'red_flag';
  kindLabel: string;
  circleStatusLabel: string;
  circleStatusTone: 'emerald' | 'sky' | 'slate';
  industrySlug?: string;
  severity: 'emerald' | 'gold' | 'sky' | 'violet' | 'coral';
  timestamp: string;
  headline: string;
  subline: string;
  businessProfile10k: string;
  evidenceTitle: string;
  evidenceQuote: string;
  sourceUrl: string;
  similarityPct: string;
  metrics: Array<{ label: string; value: string; note: string }>;
  summary: string;
  status: 'unread' | 'watch' | 'too_hard' | 'archived';
}

export interface ProvenanceMetric {
  id: string;
  label: string;
  value: string;
  formula: string;
  sha256: string;
  source: string;
}

export interface CompanyRecord {
  ticker: string;
  name: string;
  bucket: 'Core' | 'Watch' | 'Too Hard';
  sector: string;
  spot: string;
  strike: string;
  distanceLabel: string;
  statusLabel: string;
  statusTone: 'emerald' | 'gold' | 'sky' | 'coral' | 'slate';
  roic: string;
  ownerEarnings: string;
  sbcToOcf: string;
  shareDelta: string;
  similarityScore: string;
  lookalikeNote: string;
  lastFiling: string;
  metrics: ProvenanceMetric[];
  livingMemoSummary: string[];
  perspectives: Record<PerspectiveKey, { title: string; tools: string[]; points: string[] }>;
  humanJudgment: {
    ownerNote: string;
    falsification: string;
    convictionWeight: string;
  };
}

export interface ThemeCluster {
  id: string;
  name: string;
  tickers: string[];
  signal: string;
  summary: string;
}

export interface JournalEntry {
  date: string;
  ticker: string;
  action: 'STRIKE WATCH' | 'DEEP RESEARCH' | 'TOO HARD';
  tone: 'strike' | 'neutral' | 'toohard';
  note: string;
}

export interface PipelineLayer {
  id: string;
  tag: string;
  title: string;
  stat: string;
  heading: string;
  summary: string;
  bullets: string[];
  principle: string;
  cadence: string;
  command: string;
}

export interface CliItem {
  cmd: string;
  desc: string;
}

export const INBOX_EVENTS: InboxEvent[] = [
  {
    id: 'ev-mco-strike',
    ticker: 'MCO',
    company: "Moody's Corporation",
    category: 'strike',
    kindLabel: 'Strike Zone',
    circleStatusLabel: 'Circle Ind. · Toll-Booth Franchises',
    circleStatusTone: 'emerald',
    industrySlug: 'toll-booth-franchises',
    severity: 'emerald',
    timestamp: '14m ago',
    headline: 'Spot moved within +3.9% of human strike price ($420.00)',
    subline: 'Graduated Circle · 18–24x industry anchor · Tier-1 quote',
    businessProfile10k:
      'SEC 10-K Item 1: Provides independent credit ratings, research, and risk assessment across global corporate and structured debt issuance alongside recurring analytics software.',
    evidenceTitle: 'FY25 Form 10-K — Sovereign & Corporate Issuance Toll Economics',
    evidenceQuote:
      'Moody’s Investors Service captures basis-point issuance fees on global bond refinancing schedules while Moody’s Analytics delivers >94% recurring subscription retention.',
    sourceUrl: 'https://www.sec.gov/Archives/edgar/data/1059556/000105955626000014',
    similarityPct: '96%',
    metrics: [
      { label: 'Spot / Strike', value: '$436.20 / $420', note: '+3.9% above strike' },
      { label: 'Strict ROIC', value: '31.8%', note: '5Y deterministic median' },
      { label: 'Owner Earnings', value: '$2.18B', note: 'OCF − CapEx − SBC' },
      { label: 'Share Delta (3Y)', value: '−1.2% / yr', note: 'Net shrinkage' },
    ],
    summary:
      'Strike signal allowed strictly because Contractual Toll-Booths & Registries graduated into your Circle (18x low anchor). Non-financial operating toll structure avoids bank balance-sheet distortion.',
    status: 'unread',
  },
  {
    id: 'ev-dpz-discovery',
    ticker: 'DPZ',
    company: "Domino's Pizza, Inc.",
    category: 'discovery',
    kindLabel: 'Quant Screen',
    circleStatusLabel: 'Studying · Consumer Franchises',
    circleStatusTone: 'sky',
    industrySlug: 'consumer-franchises',
    severity: 'sky',
    timestamp: '2h ago',
    headline: 'Asset-light franchise royalty compounder · 5Y ROIC 36.2% (no blind strike)',
    subline: 'SIC 5812 · Zero-LLM DuckDB 5Y Snapshot + SEC 10-K Item 1',
    businessProfile10k:
      'SEC 10-K Item 1: Operates and franchises QSR pizza delivery and carryout stores globally, monetizing franchise royalties and centralized commissary supply-chain throughput with minimal store CapEx.',
    evidenceTitle: 'Item 1 & Item 7 — Franchisee Unit Economics & Supply-Chain Leverage',
    evidenceQuote:
      'More than 98% of global locations are owned by independent franchisees paying fixed sales royalties while digital ordering captures fortress-level route density.',
    sourceUrl: 'https://www.sec.gov/Archives/edgar/data/1286681/000128668126000019',
    similarityPct: '89%',
    metrics: [
      { label: '5Y ROIC', value: '36.2%', note: '5Y gross margin 38.9%' },
      { label: 'Strict FCF Yield', value: '4.1%', note: 'Within +5.7% of draft strike' },
      { label: 'SBC / OCF', value: '7.1%', note: 'Low equity dilution' },
      { label: 'Share Delta (3Y)', value: '−2.4% / yr', note: 'Steady share retirement' },
    ],
    summary:
      'Surfaced as a Quantitative Candidate with inline 5Y financials and 10-K business summary. Launch "Research Industry" with Alan to verify same-store traffic vs price before promoting from Studying.',
    status: 'unread',
  },
  {
    id: 'ev-vrsn-ripple',
    ticker: 'VRSN',
    company: 'VeriSign, Inc.',
    category: 'ripple',
    kindLabel: 'Latent Ripple',
    circleStatusLabel: 'Circle · WATCH',
    circleStatusTone: 'emerald',
    industrySlug: 'toll-booth-franchises',
    severity: 'violet',
    timestamp: 'Yesterday',
    headline: 'Operational rhyme detected against Core holding MCO (0.91 cosine)',
    subline: 'Offline 10-K similarity scan · Contractual .com / .net registry toll',
    businessProfile10k:
      'SEC 10-K Item 1: Serves as exclusive registry operator for .com and .net top-level domains under long-term ICANN cooperative agreements, collecting predictable per-domain annual fees.',
    evidenceTitle: 'Item 1 Business — Authoritative DNS & Contractual Fee Schedule',
    evidenceQuote:
      'VeriSign processes hundreds of billions of DNS queries daily at 100% resolution availability while prepaid multi-year domain renewals generate negative working capital.',
    sourceUrl: 'https://www.sec.gov/Archives/edgar/data/1014473/000101447326000011',
    similarityPct: '91%',
    metrics: [
      { label: 'Cosine Match', value: '0.91', note: 'vs MCO 10-K disclosures' },
      { label: '5Y ROIC', value: '48.6%', note: 'Contractual pricing toll' },
      { label: 'Owner Earnings', value: '$824M', note: 'Strict OCF − CapEx − SBC' },
      { label: 'Share Delta (3Y)', value: '−4.1% / yr', note: 'Aggressive buybacks' },
    ],
    summary:
      'Full-market similarity search surfaced VeriSign outside consumer habits after filing language echoed MCO’s toll-booth pricing power and institutional switching lock.',
    status: 'unread',
  },
  {
    id: 'ev-aal-warning',
    ticker: 'AAL',
    company: 'American Airlines Group',
    category: 'red_flag',
    kindLabel: 'Red Flag · Gate',
    circleStatusLabel: 'Too Hard',
    circleStatusTone: 'slate',
    severity: 'coral',
    timestamp: '2d ago',
    headline: 'Fleet CapEx & lease obligations keep Strict Owner Earnings negative',
    subline: 'Basket: TOO HARD · Classic airline capital-trap calibration fixture',
    businessProfile10k:
      'SEC 10-K Item 1: Operates scheduled mainline and regional passenger air transportation across domestic hubs and international routes with high fixed aircraft and labor commitments.',
    evidenceTitle: 'Cash Flow Statement — Aircraft Purchases vs Debt Service Cushion',
    evidenceQuote:
      'Recurring fleet replacement outlays, jet-fuel volatility, and $30B+ balance-sheet debt routinely consume cyclical operating cash inflows before durable equity return.',
    sourceUrl: 'https://www.sec.gov/Archives/edgar/data/6201/000000620126000015',
    similarityPct: '100%',
    metrics: [
      { label: 'Strict OE', value: '−$0.64B', note: 'Fleet CapEx + cyclicality' },
      { label: '5Y ROIC', value: '4.1%', note: 'Below cost of capital' },
      { label: 'Share Delta (3Y)', value: '+3.8% / yr', note: 'Post-crisis dilution overhang' },
      { label: 'Discipline Gate', value: 'TOO HARD', note: 'Excluded from Strike Zone' },
    ],
    summary:
      'Kept intentionally in Too Hard as Buffett & Munger’s enduring airline reminder: high customer utility combined with commodity seat pricing and perpetual fleet CapEx destroys owner compounding.',
    status: 'too_hard',
  },
  {
    id: 'ev-intc-warning',
    ticker: 'INTC',
    company: 'Intel Corporation',
    category: 'red_flag',
    kindLabel: 'Red Flag · Gate',
    circleStatusLabel: 'Too Hard',
    circleStatusTone: 'slate',
    severity: 'coral',
    timestamp: '3d ago',
    headline: 'Strict Owner Earnings negative for 8th quarter on foundry CapEx',
    subline: 'Basket: TOO HARD · Semiconductor capex-treadmill calibration fixture',
    businessProfile10k:
      'SEC 10-K Item 1: Vertically integrated semiconductor designer and foundry operator undergoing multi-node capital-intensive fabrication expansion.',
    evidenceTitle: 'Cash Flow Statement — Capital Expenditure vs Operating Cash Flow',
    evidenceQuote:
      'Additions to property, plant and equipment consumed 142% of operating cash flow before external partner contributions and subsidy reimbursements.',
    sourceUrl: 'https://www.sec.gov/Archives/edgar/data/50863/000005086326000012',
    similarityPct: '100%',
    metrics: [
      { label: 'Strict OE', value: '−$9.40B', note: 'CapEx consumes OCF' },
      { label: '5Y ROIC', value: '1.8%', note: 'Below capital hurdle' },
      { label: 'Share Delta (3Y)', value: '+1.9% / yr', note: 'Net equity dilution' },
      { label: 'Discipline Gate', value: 'TOO HARD', note: 'Excluded from Strike Zone' },
    ],
    summary:
      'Maintained intentionally in the Too Hard pile: extreme reinvestment demand with uncertain foundry yields violates owner-earnings predictability.',
    status: 'too_hard',
  },
];

export const INDUSTRIES: IndustryStudyRecord[] = [
  {
    slug: 'toll-booth-franchises',
    title: 'Contractual Toll-Booths & Registries',
    sicLabel: 'SIC 7374 / 7323 · Essential Institutional Tolls',
    status: 'inside_circle',
    statusLabel: 'Inside Circle ✓',
    anchorLabel: '18–24x Strict OE',
    boundTickers: ['MCO', 'SPGI', 'VRSN'],
    feynmanChecklist: [
      {
        text: 'Explain why corporate CFOs pay Moody’s / S&P a tiny basis-point fee when issuing billions of debt.',
        checked: true,
      },
      {
        text: 'Contrast genuine credit-rating / DNS pricing power against bank balance-sheet leverage.',
        checked: true,
      },
      {
        text: 'Articulate why 18–24x Strict Owner Earnings (after full SBC) is justified for non-financial tollbooths.',
        checked: true,
      },
    ],
    peers: [
      {
        ticker: 'MCO',
        name: "Moody's Corporation",
        roic5y: '31.8%',
        gm5y: '72.4%',
        fcfYield: '4.0%',
        sbcOcf: '8.4%',
        price: '$436.20',
        refStrike: '$420.00',
      },
      {
        ticker: 'SPGI',
        name: 'S&P Global Inc.',
        roic5y: '22.9%',
        gm5y: '68.8%',
        fcfYield: '3.6%',
        sbcOcf: '9.1%',
        price: '$498.50',
        refStrike: '$435.00',
      },
      {
        ticker: 'VRSN',
        name: 'VeriSign, Inc.',
        roic5y: '48.6%',
        gm5y: '87.4%',
        fcfYield: '4.4%',
        sbcOcf: '6.8%',
        price: '$206.40',
        refStrike: '$190.00',
      },
    ],
    perspectives: {
      value_chain: {
        title: '产业链与利润池 · Value Chain & Profit Pool',
        summary: 'Standard-setting tollbooths collecting thin basis-point fees on mission-critical debt & web infrastructure.',
        points: [
          'Issuers pay a few basis points in rating fees to save 30–50 bps in bond funding spread with institutional buyers.',
          'VeriSign captures recurring per-domain fees ahead of every commercial website and registrar without retail customer churn.',
        ],
      },
      moat: {
        title: '商业模式与护城河 · Business Model & Moats',
        summary: 'Two-sided institutional trust and regulatory recognition lock incumbents in for decades.',
        points: [
          'Bond mandates written into global pension and insurer covenants specifically require Moody’s & S&P dual ratings.',
          'Non-financial accounting remains transparent: OCF minus modest CapEx and low SBC maps cleanly to distributable owner cash.',
        ],
      },
      failure_modes: {
        title: '历史死法与周期陷阱 · Failure Modes & Traps',
        summary: 'Where tollbooth monopolies experience drawdowns or reputational shocks.',
        points: [
          'Prolonged credit-spread freezes delay corporate refinancing issuance volumes for 4–6 quarters.',
          'ICANN pricing caps (.net / .com renewal schedule) or DOJ antitrust scrutiny on registry contract renewals.',
        ],
      },
      valuation_kpis: {
        title: '估值锚与核心 KPI · Valuation Anchor & Key KPIs',
        summary: 'Human-selected 18–24x Strict Owner Earnings anchor after full SBC & CapEx deduction.',
        points: [
          'Track Moody’s Analytics ARR growth, net share retirement pace, and debt-refinancing maturity walls.',
          'Because this industry graduated to Inside Circle, MCO now surfaces in Strike Radar near $420.',
        ],
      },
    },
    secCitations: [
      '[SEC 10-K: MCO] FY25 Item 1 & Item 7 — MIS Ratings transaction margin & MA recurring revenue mix',
      '[SEC 10-K: VRSN] FY25 Item 1 — ICANN .com Cooperative Agreement & deferred domain revenue schedule',
    ],
    webCitations: [
      {
        title: 'SEC NRSRO Annual Report on Nationally Recognized Statistical Rating Organizations',
        url: 'https://www.sec.gov/',
      },
      {
        title: 'ICANN .com Registry Agreement Public Terms & Renewal Framework',
        url: 'https://www.icann.org/',
      },
    ],
    humanNotes:
      'Graduated to Inside Circle cleanly. Non-financial tollbooths let M3 apply Strict OE directly without bank float distortion. Hold discipline around 19–22x OE.',
  },
  {
    slug: 'consumer-franchises',
    title: 'Membership Retail & Brand Royalties',
    sicLabel: 'SIC 5331 / 2080 / 5812 · Trust & Habit Moats',
    status: 'studying',
    statusLabel: 'Studying',
    anchorLabel: '16–22x (Draft)',
    boundTickers: ['COST', 'KO', 'DPZ'],
    feynmanChecklist: [
      {
        text: 'Contrast Costco’s 14% markup ceiling (scale economies shared) against Coca-Cola’s concentrate royalty pricing.',
        checked: true,
      },
      {
        text: 'Verify Domino’s franchisee cash-on-cash payback period when store openings accelerate.',
        checked: true,
      },
      {
        text: 'Confirm what macro slowdown or private-label mix shift invalidates KO or DPZ volume growth.',
        checked: false,
      },
    ],
    peers: [
      {
        ticker: 'COST',
        name: 'Costco Wholesale',
        roic5y: '33.1%',
        gm5y: '12.6%',
        fcfYield: '1.9%',
        sbcOcf: '7.3%',
        price: '$892.00',
        refStrike: '$640.00',
      },
      {
        ticker: 'KO',
        name: 'The Coca-Cola Company',
        roic5y: '26.4%',
        gm5y: '60.4%',
        fcfYield: '3.5%',
        sbcOcf: '2.3%',
        price: '$64.80',
        refStrike: '$60.00',
      },
      {
        ticker: 'DPZ',
        name: "Domino's Pizza, Inc.",
        roic5y: '36.2%',
        gm5y: '38.9%',
        fcfYield: '4.1%',
        sbcOcf: '7.1%',
        price: '$438.50',
        refStrike: '$415.00',
      },
    ],
    perspectives: {
      value_chain: {
        title: '产业链与利润池 · Value Chain & Profit Pool',
        summary: 'Asset-light brand/membership layers capturing consumer trust while keeping retail inventory turns extraordinarily lean.',
        points: [
          'Coca-Cola sells high-margin syrup concentrate upstream while independent bottlers carry trucks, coolers, and route CapEx.',
          'Costco caps merchandise gross margin near ~12–13% so high-margin membership renewals drop straight to pre-tax profit.',
        ],
      },
      moat: {
        title: '商业模式与护城河 · Business Model & Moats',
        summary: 'Munger’s classic “scale economies shared” paired with habitual brand mindshare.',
        points: [
          'Customer loyalty rises only when scale savings are handed back through lower SKU prices or faster delivery rather than extracted as short-term margin.',
        ],
      },
      failure_modes: {
        title: '历史死法与周期陷阱 · Failure Modes & Traps',
        summary: 'Overpaying 40x+ earnings for mature consumer staples or choking franchisee returns.',
        points: ['Buying a beloved franchise at 19% implied 10-year growth risks flat returns if multiples compress back toward 22x.'],
      },
      valuation_kpis: {
        title: '估值锚与核心 KPI · Valuation Anchor & Key KPIs',
        summary: 'Remains in Studying status — complete final Feynman item before unlocking automated Strike signals.',
        points: ['Membership renewal >91% (COST), organic concentrate unit case volume (KO), and franchisee store EBITDA (DPZ).'],
      },
    },
    secCitations: [
      '[SEC 10-K: COST] FY25 Item 1 — Membership fee renewal rates & Kirkland Signature mix',
      '[SEC 10-K: KO] FY25 Item 7 — Concentrate operations vs bottling investments return profile',
    ],
    webCitations: [
      {
        title: 'Berkshire Hathaway Annual Letters — See’s Candies, Coca-Cola & Capital-Light Compounders',
        url: 'https://www.berkshirehathaway.com/letters/letters.html',
      },
    ],
    humanNotes:
      'Timeless Munger/Berkshire businesses — main enemy here is not business failure, it is paying 50x Owner Earnings for Costco.',
  },
];

export const COMPANIES: CompanyRecord[] = [
  {
    ticker: 'MCO',
    name: "Moody's Corporation",
    bucket: 'Core',
    sector: 'Credit Ratings & Analytics',
    spot: '$436.20',
    strike: '$420.00',
    distanceLabel: '+3.9%',
    statusLabel: 'Near Strike',
    statusTone: 'gold',
    roic: '31.8%',
    ownerEarnings: '$2.18B',
    sbcToOcf: '8.4%',
    shareDelta: '−1.2%',
    similarityScore: '0.96',
    lookalikeNote: 'Berkshire 20+ year non-bank financial tollbooth benchmark',
    lastFiling: '10-K FY25 · 0001059556-26-000014',
    metrics: [
      {
        id: 'mco-roic',
        label: '5Y Median ROIC',
        value: '31.8%',
        formula: 'NOPAT / (Net Working Capital + Net PP&E + Acquired Intangibles adj.)',
        sha256: 'sha256:8f41c2b9...90e4a17d (us-gaap:OperatingIncomeLoss)',
        source: 'SEC EDGAR Company Facts · CIK 0001059556',
      },
      {
        id: 'mco-oe',
        label: 'Strict Owner Earnings',
        value: '$2.18B',
        formula: 'OCF ($2.49B) − CapEx ($101M) − SBC ($209M)',
        sha256: 'sha256:3a90de14...4b22f08c (us-gaap:ShareBasedCompensation)',
        source: 'Deterministic Accounting Engine · Exact Decimal',
      },
      {
        id: 'mco-shares',
        label: '3Y Share Count Delta',
        value: '−1.2% / yr',
        formula: 'Annualized change in diluted shares net of buybacks',
        sha256: 'sha256:71c40e52...11d92fa0 (dei:EntityCommonStockSharesOutstanding)',
        source: 'SEC EDGAR XBRL Primary Fact',
      },
      {
        id: 'mco-implied',
        label: 'Reverse Implied Growth',
        value: '10.4% / 10y',
        formula: 'Implied annual OE compounding required at current $436.20 spot (10% hurdle)',
        sha256: 'sha256:d049b661...6e71c309 (calculator::reverse_dcf)',
        source: 'Deterministic Reverse-DCF Solver',
      },
    ],
    livingMemoSummary: [
      'Classic Buffett tollbooth without bank-balance-sheet distortion: Moody’s takes zero credit-loss balance-sheet risk while clipping basis points on global debt issuance.',
      'Moody’s Analytics stabilizes cyclical issuance troughs with >94% recurring institutional risk subscriptions.',
      'Clean accounting profile (8.4% SBC/OCF) and consistent net share shrinkage justify sitting patiently near the $420 strike.',
    ],
    perspectives: {
      moat: {
        title: 'Moat Durability · Two-Sided Institutional Trust',
        tools: ['m3_lake_company_snapshot', 'm3_peer_percentiles'],
        points: [
          'Corporate borrowers save more in spread compression than the rating fee costs; asset managers require dual rating governance.',
          'Decades of default-history calibration make displacing the Moody’s + S&P duopoly nearly impossible.',
        ],
      },
      accounting: {
        title: 'Accounting Reality · Pure Non-Bank Cash Conversion',
        tools: ['m3_xbrl_facts', 'm3_owner_earnings_audit'],
        points: [
          'Stock-based compensation consumes only 8.4% of operating cash flow while physical CapEx sits below 5% of OCF.',
          'Unlike commercial banks, Owner Earnings maps directly from operating cash flows with zero regulatory capital distortion.',
        ],
      },
      latent_peers: {
        title: 'Latent Peers · Contractual Toll-Booths Across 5,142 Filings',
        tools: ['m3_similar_companies', 'm3_excerpt_search'],
        points: [
          'Closest operational rhymes: SPGI (0.95 cosine), VRSN (0.91 cosine), MSFT (0.82 cosine) — all exhibit institutional standard-setting tolls.',
        ],
      },
      premortem: {
        title: 'Pre-Mortem · How This Compounding Stops',
        tools: ['m3_filing_risk_scan', 'm3_balance_sheet_stress'],
        points: [
          'Multi-year private-credit bypasses reduce public syndicated bond ratings share faster than Analytics private-credit tools offset.',
          'High-yield issuance freezes temporarily compress transaction margins during rate shocks.',
        ],
      },
    },
    humanJudgment: {
      ownerNote: 'Inside circle of competence (contractual tollbooth franchise held by Berkshire for decades). Wait calmly around $420 strike.',
      falsification: 'Review immediately if strict Owner Earnings margin falls below 25% or Moody’s Analytics net retention slips below 89%.',
      convictionWeight: 'Tier-A Core · Paper Cap 18%',
    },
  },
  {
    ticker: 'COST',
    name: 'Costco Wholesale',
    bucket: 'Core',
    sector: 'Membership Warehouse',
    spot: '$892.00',
    strike: '$640.00',
    distanceLabel: '+39.4%',
    statusLabel: 'Wait',
    statusTone: 'slate',
    roic: '33.1%',
    ownerEarnings: '$6.84B',
    sbcToOcf: '7.3%',
    shareDelta: '−0.1%',
    similarityScore: '0.88',
    lookalikeNote: 'Munger’s lifelong benchmark; rich multiple requires strict price discipline',
    lastFiling: '10-K FY25 · 0000909832-25-000084',
    metrics: [
      {
        id: 'cost-roic',
        label: '5Y Median ROIC',
        value: '33.1%',
        formula: 'High inventory turns + membership annuity on owned warehouse real estate',
        sha256: 'sha256:29b1c4e8...01fa77d3',
        source: 'SEC EDGAR Company Facts · CIK 0000909832',
      },
      {
        id: 'cost-oe',
        label: 'Strict Owner Earnings',
        value: '$6.84B',
        formula: 'OCF ($11.8B) − CapEx ($4.1B) − SBC ($860M)',
        sha256: 'sha256:64de09fa...9c23b51e',
        source: 'Deterministic Accounting Engine',
      },
      {
        id: 'cost-shares',
        label: '3Y Share Count Delta',
        value: '−0.1% / yr',
        formula: 'Clean share base; surplus capital returned via special cash dividends',
        sha256: 'sha256:ab12e940...440cd17a',
        source: 'SEC EDGAR XBRL Primary Fact',
      },
      {
        id: 'cost-implied',
        label: 'Reverse Implied Growth',
        value: '18.9% / 10y',
        formula: 'Market spot embeds ~19% annual Owner Earnings compounding for a decade',
        sha256: 'sha256:ef0913da...2284cc91',
        source: 'Deterministic Reverse-DCF Solver',
      },
    ],
    livingMemoSummary: [
      'Charlie Munger sat on Costco’s board for decades for one reason: a strict 14% markup ceiling converts scale directly into customer devotion.',
      'Paying 50x+ Owner Earnings guarantees mediocre forward returns even in the finest retail institution ever built — keep the strike anchor honest.',
    ],
    perspectives: {
      moat: {
        title: 'Moat Durability · Scale Economies Shared',
        tools: ['m3_lake_company_snapshot'],
        points: [
          '76M households renew >92% annually because every purchasing efficiency lowers shelf price rather than inflating retail margin.',
        ],
      },
      accounting: {
        title: 'Accounting Reality · Real Asset Backing',
        tools: ['m3_xbrl_facts'],
        points: [
          'SBC represents only 7.3% of operating cash flow while annual CapEx builds unencumbered warehouse land.',
        ],
      },
      latent_peers: {
        title: 'Latent Peers · High-Trust Discounters',
        tools: ['m3_similar_companies'],
        points: [
          'Outlier across full US filings for negative working capital cycle paired with single-digit dilution and special-dividend discipline.',
        ],
      },
      premortem: {
        title: 'Pre-Mortem · Multiple Compression Math',
        tools: ['m3_balance_sheet_stress'],
        points: [
          'A wonderful business bought at 19% implied 10-year growth risks a lost decade if terminal multiple normalizes.',
        ],
      },
    },
    humanJudgment: {
      ownerNote: 'Gold-standard Munger business model; stay disciplined on price and wait for broad market fear near $640.',
      falsification: 'Renewal rate slips below 89% or markup discipline erodes.',
      convictionWeight: 'Tier-A Core · Patient at $640 strike',
    },
  },
  {
    ticker: 'AAPL',
    name: 'Apple Inc.',
    bucket: 'Core',
    sector: 'Consumer Ecosystem',
    spot: '$228.40',
    strike: '$185.00',
    distanceLabel: '+23.5%',
    statusLabel: 'Wait',
    statusTone: 'slate',
    roic: '54.2%',
    ownerEarnings: '$93.1B',
    sbcToOcf: '9.8%',
    shareDelta: '−2.8%',
    similarityScore: '0.87',
    lookalikeNote: 'Berkshire’s largest consumer-franchise capital cannibal benchmark',
    lastFiling: '10-K FY25 · 0000320193-25-000106',
    metrics: [
      {
        id: 'aapl-roic',
        label: '5Y Median ROIC',
        value: '54.2%',
        formula: 'Brand ecosystem + negative working capital float',
        sha256: 'sha256:a190bc22...319d44f0',
        source: 'SEC EDGAR Company Facts · CIK 0000320193',
      },
      {
        id: 'aapl-oe',
        label: 'Strict Owner Earnings',
        value: '$93.1B',
        formula: 'OCF − CapEx − SBC ($11.7B)',
        sha256: 'sha256:77cc01e9...6012ab44',
        source: 'Deterministic Accounting Engine',
      },
      {
        id: 'aapl-shares',
        label: '3Y Share Count Delta',
        value: '−2.8% / yr',
        formula: 'Persistent net share retirement',
        sha256: 'sha256:55fe1902...8a19c033',
        source: 'SEC EDGAR XBRL Primary Fact',
      },
      {
        id: 'aapl-implied',
        label: 'Reverse Implied Growth',
        value: '11.8% / 10y',
        formula: 'Current valuation assumes sustained double-digit Services + hardware refresh',
        sha256: 'sha256:19a4c8e0...f102994a',
        source: 'Deterministic Reverse-DCF Solver',
      },
    ],
    livingMemoSummary: [
      'Buffett recognized iPhone as a daily consumer habit rather than cyclical tech hardware, creating >50% ROIC plus relentless share buybacks.',
    ],
    perspectives: {
      moat: {
        title: 'Moat Durability · Installed-Base Habit Lock',
        tools: ['m3_lake_company_snapshot'],
        points: ['Two billion active devices anchor hardware-software integration and family ecosystem switching cost.'],
      },
      accounting: {
        title: 'Accounting Reality · Clean Cash Conversion',
        tools: ['m3_xbrl_facts'],
        points: ['SBC under 10% of OCF with steady −2.8% annual share shrinkage.'],
      },
      latent_peers: {
        title: 'Latent Peers',
        tools: ['m3_similar_companies'],
        points: ['Clustered with high-ecosystem consumer franchises; higher installed-base habit lock than traditional retail peers.'],
      },
      premortem: {
        title: 'Pre-Mortem · App Store Unbundling',
        tools: ['m3_filing_risk_scan'],
        points: ['Global antitrust mandates chip away at high-margin Services take rates while smartphone replacement cycles lengthen.'],
      },
    },
    humanJudgment: {
      ownerNote: 'Hold discipline on strike price $185; trim or pause when implied expectations push above 12% forever.',
      falsification: 'Services gross margin compresses >400 bps due to regulatory fee caps.',
      convictionWeight: 'Core · Waiting at $185 strike',
    },
  },
  {
    ticker: 'KO',
    name: 'The Coca-Cola Company',
    bucket: 'Core',
    sector: 'Beverage Concentrate Franchise',
    spot: '$64.80',
    strike: '$60.00',
    distanceLabel: '+8.0%',
    statusLabel: 'Wait',
    statusTone: 'slate',
    roic: '26.4%',
    ownerEarnings: '$9.62B',
    sbcToOcf: '2.3%',
    shareDelta: '−0.3%',
    similarityScore: '0.85',
    lookalikeNote: 'Timeless Buffett-era concentrate franchise with ultra-low SBC',
    lastFiling: '10-K FY25 · 0000021344-26-000009',
    metrics: [
      {
        id: 'ko-roic',
        label: '5Y Median ROIC',
        value: '26.4%',
        formula: 'Concentrate royalty economics separated from capital-heavy bottlers',
        sha256: 'sha256:41f0bc88...81d3a210',
        source: 'SEC EDGAR Company Facts · CIK 0000021344',
      },
      {
        id: 'ko-oe',
        label: 'Strict Owner Earnings',
        value: '$9.62B',
        formula: 'OCF ($11.9B) − CapEx ($2.01B) − SBC ($274M)',
        sha256: 'sha256:884c110f...20ce719b',
        source: 'Deterministic Accounting Engine',
      },
      {
        id: 'ko-shares',
        label: '3Y Share Count Delta',
        value: '−0.3% / yr',
        formula: 'Ultra-low SBC (2.3% of OCF) keeps dilution near zero while paying 60+ years of growing dividends',
        sha256: 'sha256:73a902ce...198d441f',
        source: 'SEC EDGAR XBRL Primary Fact',
      },
      {
        id: 'ko-implied',
        label: 'Reverse Implied Growth',
        value: '7.6% / 10y',
        formula: 'Requires mid-single-digit price/mix compounding to deliver 10% total return',
        sha256: 'sha256:3e9901da...57bc8102',
        source: 'Deterministic Reverse-DCF Solver',
      },
    ],
    livingMemoSummary: [
      'Held uninterrupted by Berkshire since 1988: 2.2 billion daily servings turn an affordable trademark habit into inflation-passing concentrate cash flow.',
    ],
    perspectives: {
      moat: {
        title: 'Moat Durability · Global Mindshare & Route Density',
        tools: ['m3_lake_company_snapshot'],
        points: ['Refranchised bottling separates heavy truck/cooler CapEx from 60%+ concentrate gross margins.'],
      },
      accounting: {
        title: 'Accounting Reality · Minimal Dilution Ballast',
        tools: ['m3_xbrl_facts'],
        points: ['Only 2.3% SBC/OCF — virtually zero equity leakage between reported cash flow and true owner earnings.'],
      },
      latent_peers: {
        title: 'Latent Peers',
        tools: ['m3_similar_companies'],
        points: ['Pairs naturally with asset-light brand royalties like COST and DPZ inside Consumer Franchises.'],
      },
      premortem: {
        title: 'Pre-Mortem · GLP-1 & FX Headwinds',
        tools: ['m3_filing_risk_scan'],
        points: ['Sustained calorie-consumption shifts or emerging-market currency depreciation outpace local pricing power.'],
      },
    },
    humanJudgment: {
      ownerNote: 'Steady compounding ballast. Wait calmly around $60 strike for wider yield cushion.',
      falsification: 'Global unit case volume contracts for three consecutive years alongside flat pricing.',
      convictionWeight: 'Core · Waiting at $60 strike',
    },
  },
  {
    ticker: 'VRSN',
    name: 'VeriSign, Inc.',
    bucket: 'Watch',
    sector: 'Critical DNS Registry',
    spot: '$206.40',
    strike: '$190.00',
    distanceLabel: '+8.6%',
    statusLabel: 'In Research',
    statusTone: 'sky',
    roic: '48.6%',
    ownerEarnings: '$824M',
    sbcToOcf: '6.8%',
    shareDelta: '−4.1%',
    similarityScore: '0.91',
    lookalikeNote: 'Surfaced via L0+Latent lookalike query against MCO',
    lastFiling: '10-K FY25 · 0001014473-26-000011',
    metrics: [
      {
        id: 'vrsn-roic',
        label: '5Y Median ROIC',
        value: '48.6%',
        formula: 'Exclusive contractual .com / .net registry + prepaid renewal float',
        sha256: 'sha256:55ab91de...1408cf22',
        source: 'SEC EDGAR Company Facts · CIK 0001014473',
      },
      {
        id: 'vrsn-oe',
        label: 'Strict Owner Earnings',
        value: '$824M',
        formula: 'OCF ($925M) − CapEx ($38M) − SBC ($63M)',
        sha256: 'sha256:93f8102c...66ba43d0',
        source: 'Deterministic Accounting Engine',
      },
      {
        id: 'vrsn-shares',
        label: '3Y Share Count Delta',
        value: '−4.1% / yr',
        formula: 'Nearly 100% of strict Owner Earnings recycled into share buybacks',
        sha256: 'sha256:08ce721f...9503aa18',
        source: 'SEC EDGAR XBRL Primary Fact',
      },
      {
        id: 'vrsn-implied',
        label: 'Reverse Implied Growth',
        value: '8.2% / 10y',
        formula: 'Conservative single-digit growth implied; contractual price steps provide visibility',
        sha256: 'sha256:7d1208cc...8815ff09',
        source: 'Deterministic Reverse-DCF Solver',
      },
    ],
    livingMemoSummary: [
      'A textbook Berkshire hidden champion: 100% DNS uptime across .com/.net with prepaid cash collection, 87% gross margin, and −4.1% annual share shrinkage.',
    ],
    perspectives: {
      moat: {
        title: 'Moat Durability · ICANN Authoritative Registry',
        tools: ['m3_lake_company_snapshot', 'm3_excerpt_search'],
        points: ['Presumptive renewal under ICANN agreement makes .com registry displacement practically non-existent barring performance failure.'],
      },
      accounting: {
        title: 'Accounting Reality · Cannibal Cash Engine',
        tools: ['m3_xbrl_facts'],
        points: ['CapEx takes ~4% of OCF and SBC ~6.8%; remaining free cash steadily shrinks the float every quarter.'],
      },
      latent_peers: {
        title: 'Latent Peers · 0.91 Rhyme with MCO',
        tools: ['m3_similar_companies'],
        points: ['Shares standard-setting tollbooth economics and negative-working-capital resilience with Moody’s and S&P Global.'],
      },
      premortem: {
        title: 'Pre-Mortem · Renewal Cap Renegotiation',
        tools: ['m3_filing_risk_scan'],
        points: ['Regulatory freeze on contractual .com wholesale price hikes flattens domain base sluggishly.'],
      },
    },
    humanJudgment: {
      ownerNote: 'Verifying ICANN renewal cadence and registrar renewal elasticity before promoting from Watch to Core.',
      falsification: 'Domain base contracts >3% YoY alongside elimination of contractual fee increases.',
      convictionWeight: 'Watch · Ready near $190',
    },
  },
  {
    ticker: 'DPZ',
    name: "Domino's Pizza, Inc.",
    bucket: 'Watch',
    sector: 'QSR Franchise Royalties',
    spot: '$438.50',
    strike: '$415.00',
    distanceLabel: '+5.7%',
    statusLabel: 'Near Strike',
    statusTone: 'gold',
    roic: '36.2%',
    ownerEarnings: '$478M',
    sbcToOcf: '7.1%',
    shareDelta: '−2.4%',
    similarityScore: '0.84',
    lookalikeNote: 'Berkshire asset-light QSR royalty compounder near Watch strike',
    lastFiling: '10-K FY25 · 0001286681-26-000019',
    metrics: [
      {
        id: 'dpz-roic',
        label: '5Y Median ROIC',
        value: '36.2%',
        formula: 'Franchise royalties + supply-chain dough commissary asset turnover',
        sha256: 'sha256:b09312cf...442ea911',
        source: 'SEC EDGAR Company Facts · CIK 0001286681',
      },
      {
        id: 'dpz-oe',
        label: 'Strict Owner Earnings',
        value: '$478M',
        formula: 'OCF ($624M) − CapEx ($102M) − SBC ($44M)',
        sha256: 'sha256:c41289af...7701da33',
        source: 'Deterministic Accounting Engine',
      },
      {
        id: 'dpz-shares',
        label: '3Y Share Count Delta',
        value: '−2.4% / yr',
        formula: 'Consistent franchise-cash buyback cannibalization',
        sha256: 'sha256:91ce440b...2018fe55',
        source: 'SEC EDGAR XBRL Primary Fact',
      },
      {
        id: 'dpz-implied',
        label: 'Reverse Implied Growth',
        value: '9.8% / 10y',
        formula: 'Reasonable high-single-digit global store growth expectation',
        sha256: 'sha256:20ab77e1...63fc0981',
        source: 'Deterministic Reverse-DCF Solver',
      },
    ],
    livingMemoSummary: [
      'A quintessential non-tech franchise machine: 98% franchised footprint shifts store-level real estate and labor CapEx off the corporate balance sheet.',
    ],
    perspectives: {
      moat: {
        title: 'Moat Durability · Delivery Fortress Economics',
        tools: ['m3_lake_company_snapshot'],
        points: ['Shortened delivery radius lowers driver cost per drop while franchisee shared-commissary rebates align supply chain incentives.'],
      },
      accounting: {
        title: 'Accounting Reality · Franchise Royalty Purity',
        tools: ['m3_xbrl_facts'],
        points: ['Minimal 7.1% SBC/OCF; monitor secular securitized debt refinancing coupons against fixed interest coverage.'],
      },
      latent_peers: {
        title: 'Latent Peers · Royalty Franchisors',
        tools: ['m3_similar_companies'],
        points: ['Clustered structurally with asset-light royalty models (KO, ORLY, YUM) inside Consumer Franchises.'],
      },
      premortem: {
        title: 'Pre-Mortem · Aggregator Margin Squeeze',
        tools: ['m3_balance_sheet_stress'],
        points: ['Third-party delivery apps erode direct customer data ownership if carryout traffic softens.'],
      },
    },
    humanJudgment: {
      ownerNote: 'Close to draft strike ($415). Complete final Consumer Franchises Feynman item before promoting to Core.',
      falsification: 'US same-store order count stagnates while franchisee store-level EBITDA drops >15%.',
      convictionWeight: 'Watch · Under Evaluation',
    },
  },
  {
    ticker: 'AAL',
    name: 'American Airlines Group',
    bucket: 'Too Hard',
    sector: 'Commercial Airlines',
    spot: '$14.20',
    strike: 'N/A',
    distanceLabel: '—',
    statusLabel: 'Too Hard',
    statusTone: 'coral',
    roic: '4.1%',
    ownerEarnings: '−$0.64B',
    sbcToOcf: '4.9%',
    shareDelta: '+3.8%',
    similarityScore: '0.24',
    lookalikeNote: 'Classic Buffett / Munger airline CapEx trap kept visible in Too Hard',
    lastFiling: '10-K FY25 · 0000006201-26-000015',
    metrics: [
      {
        id: 'aal-roic',
        label: '5Y Median ROIC',
        value: '4.1%',
        formula: 'High aircraft asset base + union/fuel cost squeeze below cost of capital',
        sha256: 'sha256:7f1288cc...90ae1402',
        source: 'SEC EDGAR Company Facts · CIK 0000006201',
      },
      {
        id: 'aal-oe',
        label: 'Strict Owner Earnings',
        value: '−$0.64B',
        formula: 'Fleet CapEx + aircraft lease burden consume cyclical operating cash',
        sha256: 'sha256:19ac8820...45bc712e',
        source: 'Deterministic Accounting Engine',
      },
      {
        id: 'aal-shares',
        label: '3Y Share Count Delta',
        value: '+3.8% / yr',
        formula: 'Structural dilution during downturns; permanent capital impairment risk',
        sha256: 'sha256:66df0912...30aa55bc',
        source: 'SEC EDGAR XBRL Primary Fact',
      },
      {
        id: 'aal-implied',
        label: 'Reverse Implied Growth',
        value: 'Blocked',
        formula: 'Fails negative Owner Earnings & leverage durability gates',
        sha256: 'sha256:0000guard...too_hard',
        source: 'Too Hard Veto Gate',
      },
    ],
    livingMemoSummary: [
      'Buffett’s canonical cautionary tale: an indispensable modern industry whose undifferentiated seats and perpetual fleet CapEx transfer surplus to travelers rather than owners.',
    ],
    perspectives: {
      moat: {
        title: 'Moat Durability · Commodity Seat Inventory',
        tools: ['m3_lake_company_snapshot'],
        points: ['Price transparency apps clear marginal airline seats at thin spreads above fuel & crew cost whenever capacity grows.'],
      },
      accounting: {
        title: 'Accounting Reality · Fleet Replacement Treadmill',
        tools: ['m3_xbrl_facts'],
        points: ['Headline EBITDA masks massive recurring widebody/narrowbody replacement CapEx and heavy lease leverage.'],
      },
      latent_peers: {
        title: 'Latent Peers',
        tools: ['m3_similar_companies'],
        points: ['Near-zero structural similarity (<0.25 cosine) with tollbooth and consumer-royalty holdings.'],
      },
      premortem: {
        title: 'Pre-Mortem · Leverage Meets Any Recession',
        tools: ['m3_balance_sheet_stress'],
        points: ['Fixed interest + union step-ups collide with any 10% business-travel dip, forcing fresh dilution or bailout debt.'],
      },
    },
    humanJudgment: {
      ownerNote: 'Rejected permanently under Buffett/Munger capital-intensity & leverage rules (even at single-digit P/E).',
      falsification: 'Revisit only after debt drops <1.5x EBITDA and industry achieves unsubsidized 10Y >12% ROIC through a full recession.',
      convictionWeight: '0% · TOO HARD',
    },
  },
  {
    ticker: 'INTC',
    name: 'Intel Corporation',
    bucket: 'Too Hard',
    sector: 'Foundry / Semis',
    spot: '$21.40',
    strike: 'N/A',
    distanceLabel: '—',
    statusLabel: 'Too Hard',
    statusTone: 'coral',
    roic: '1.8%',
    ownerEarnings: '−$9.40B',
    sbcToOcf: '31.8%',
    shareDelta: '+1.9%',
    similarityScore: '0.31',
    lookalikeNote: 'Parked intentionally in Too Hard pile',
    lastFiling: '10-K FY25 · 0000050863-26-000012',
    metrics: [
      {
        id: 'intc-roic',
        label: '5Y Median ROIC',
        value: '1.8%',
        formula: 'Foundry buildout depresses capital efficiency far below hurdle',
        sha256: 'sha256:fe90221a...701bc344',
        source: 'SEC EDGAR Company Facts · CIK 0000050863',
      },
      {
        id: 'intc-oe',
        label: 'Strict Owner Earnings',
        value: '−$9.40B',
        formula: 'CapEx ($23.9B) overwhelms OCF ($11.0B) + SBC',
        sha256: 'sha256:bc145982...19ef007a',
        source: 'Deterministic Accounting Engine',
      },
      {
        id: 'intc-shares',
        label: '3Y Share Count Delta',
        value: '+1.9% / yr',
        formula: 'Persistent equity dilution',
        sha256: 'sha256:40aa817c...92d183be',
        source: 'SEC EDGAR XBRL Primary Fact',
      },
      {
        id: 'intc-implied',
        label: 'Reverse Implied Growth',
        value: 'Blocked',
        formula: 'Negative strict Owner Earnings fails Munger threshold gate',
        sha256: 'sha256:0000guard...too_hard',
        source: 'Too Hard Veto Gate',
      },
    ],
    livingMemoSummary: [
      'Kept visible inside TOO HARD as an explicit reminder: avoid businesses demanding endless capital just to stay in the race.',
    ],
    perspectives: {
      moat: {
        title: 'Moat Durability · Eroded Process Lead',
        tools: ['m3_lake_company_snapshot'],
        points: ['Lost leading-edge node density lead forces simultaneous defense on x86 server and external foundry fronts.'],
      },
      accounting: {
        title: 'Accounting Reality · Subsidy-Masked Outflows',
        tools: ['m3_xbrl_facts'],
        points: ['Unadjusted owner earnings remain deeply negative before partner capital and public grants.'],
      },
      latent_peers: {
        title: 'Latent Peers',
        tools: ['m3_similar_companies'],
        points: ['Low structural overlap with asset-light compounder cluster (<0.35 cosine).'],
      },
      premortem: {
        title: 'Pre-Mortem · Turnarounds Seldom Turn',
        tools: ['m3_balance_sheet_stress'],
        points: ['CapEx scale treadmill repeats every 24 months regardless of short-term product hype.'],
      },
    },
    humanJudgment: {
      ownerNote: 'Rejected intentionally under circle-of-competence and capital-intensity rules.',
      falsification: 'Revisit only after 8 quarters of unsubsidized positive strict Owner Earnings and >15% ROIC.',
      convictionWeight: '0% · TOO HARD',
    },
  },
];

export const THEMES: ThemeCluster[] = [
  {
    id: 'th-tollbooths',
    name: 'Contractual Toll-Booths & Digital Registries',
    tickers: ['MCO', 'VRSN'],
    signal: 'Cosine 0.91 · Non-bank cash tolls',
    summary:
      'Essential institutional infrastructure (credit ratings, authoritative DNS registries) capturing thin basis-point fees with minimal CapEx and zero bank-balance-sheet opacity.',
  },
  {
    id: 'th-scale',
    name: 'Scale Economies Shared & Brand Royalties',
    tickers: ['COST', 'KO', 'AAPL', 'DPZ'],
    signal: 'High ROIC · Low SBC · Wait for price',
    summary:
      'Rare consumer franchises with habitual lock-in, ultra-low equity dilution, and relentless share buybacks; disciplined strike prices prevent overpaying.',
  },
  {
    id: 'th-capex-trap',
    name: 'Capital-Intensive Treadmills (Too Hard)',
    tickers: ['AAL', 'INTC'],
    signal: 'Strict OE negative · Parked',
    summary:
      'Businesses forced to reinvest more than 100% of operating cash into fleets or fabs just to stay in a commodity or catch-up race.',
  },
];

export const JOURNAL_ENTRIES: JournalEntry[] = [
  {
    date: '2026-11-24',
    ticker: 'MCO',
    action: 'DEEP RESEARCH',
    tone: 'neutral',
    note: 'Completed 4-lens Alan dossier (Moat, Accounting, Latent Peers, Pre-Mortem). Holding $420 strike anchored to non-financial tollbooth cash conversion.',
  },
  {
    date: '2026-11-22',
    ticker: 'VRSN',
    action: 'STRIKE WATCH',
    tone: 'strike',
    note: 'Discovered through latent analogy to MCO (0.91 cosine). Reviewing ICANN .com fee schedule and −4.1% share retirement before promoting to Core.',
  },
  {
    date: '2026-11-20',
    ticker: 'AAL',
    action: 'TOO HARD',
    tone: 'toohard',
    note: 'Canonical Buffett/Munger reminder: fleet CapEx + cyclicality leave Strict OE negative (−$0.64B). Parked in TOO HARD regardless of low headline P/E.',
  },
  {
    date: '2026-11-19',
    ticker: 'INTC',
    action: 'TOO HARD',
    tone: 'toohard',
    note: 'Deterministic Owner Earnings printed −$9.40B. Parked in TOO HARD pile to keep attention clear.',
  },
];

export const PIPELINE_LAYERS: PipelineLayer[] = [
  {
    id: 'l0',
    tag: 'Stage 01 · Whole-Market Sight',
    title: 'The Quiet Ledger',
    stat: '5,142 issuers · $0 daily cost',
    heading: 'See the whole field without paying for market noise',
    summary:
      'A human mind saturates at a dozen companies. M3 maintains an entire local ledger of primary US corporate filings—computing clean cash returns and dilution quietly before you ever open an article.',
    bullets: [
      'Separates true Owner Earnings from management stories (OCF minus real capex minus full SBC)',
      'Tracks net share shrinkage or dilution across years of primary filings',
      'Keeps every calculation traceable down to its exact official source hash',
    ],
    principle: 'Width creates patience: knowing 5,000 alternatives removes the urge to swing early.',
    cadence: '$0.00 standing cost · Offline & private on your disk',
    command: 'm3 universe screen --min-roic 15 --positive-owner-earnings',
  },
  {
    id: 'l1',
    tag: 'Stage 02 · Attention Hygiene',
    title: 'Event-Driven Calm',
    stat: 'Catch up on facts, never ticks',
    heading: 'React only when reality crosses a line you drew in advance',
    summary:
      'Intraday quotes optimize for adrenaline, not compounding. M3 catches up on missed filings, strike-band entries, and material disclosures only when you sit down at your desk—never asking for 24/7 vigilance.',
    bullets: [
      'Surfaces factual changes cleanly inside a single calm Inbox',
      'Reminds you when a pre-registered strike price approaches—not when headlines shout',
      'Lets you dismiss noise to TOO HARD with one keystroke',
    ],
    principle: 'Activity is the enemy of return; react slowly to fundamentals, never to volatility.',
    cadence: 'Zero LLM tokens spent on routine screening & alerts',
    command: 'm3 events catchup --since 7d && m3 inbox list',
  },
  {
    id: 'ls',
    tag: 'Stage 03 · Expand Your Circle',
    title: 'Industry Research',
    stat: 'Alan 4-Lens · 10-K + Web',
    heading: 'Turn unfamiliar "Too Hard" companies into earned Circle competence',
    summary:
      'Instead of swinging blindly at quant screens or mechanical 18x targets, launch an Industry Research project from any topic, SIC cluster, or company. Alan pulls representative 10-Ks, builds a Zero-LLM peer matrix, searches public history, and quizzes you via Feynman Mastery before YOU graduate the industry into your Circle.',
    bullets: [
      'Synthesizes Value Chain, Moats, Failure Modes & Key KPIs with separated SEC 10-K vs Web citations',
      'Only graduated Inside-Circle industries unlock Strike Zone signals at your human multiple anchor',
      'Protects your ## Human Notes & Feynman checklist verbatim across every re-run',
    ],
    principle: 'Width of reading without depth of understanding is noise; graduate industries deliberately.',
    cadence: 'Human-gated Strike Zone · Manual Alan Industry Runs',
    command: 'workbench/industries/<slug>.json + .md (human sovereignty)',
  },
  {
    id: 'l2',
    tag: 'Stage 04 · Adversarial Thesis Testing',
    title: 'Invert, Always Invert',
    stat: '4 independent lenses · Hard cap',
    heading: 'Test where an idea breaks before allowing yourself to believe it',
    summary:
      'When an opportunity clears every quantitative filter, M3 deploys four read-only Alan lenses—Moat, Accounting, Latent Peers, and a harsh Pre-Mortem—against locked filing facts before handing judgment back to you.',
    bullets: [
      'Structurally forbidden from inventing numbers or issuing buy/sell advice',
      'Hard monthly dollar cap enforces calm selectivity instead of shallow volume',
      'Preserves your personal falsification triggers and strike note alongside the memo',
    ],
    principle: 'Never hold a conviction you have not tried hard to disprove first.',
    cadence: 'Budget-gated deep interrogation ($10/mo default ceiling)',
    command: 'm3 budget status && m3 research run MCO',
  },
];

export const CLI_COMMANDS: CliItem[] = [
  {
    cmd: 'm3 doctor',
    desc: 'Inspect binary channel, DuckDB lakehouse, storage usage, and host tools.',
  },
  {
    cmd: 'm3 universe screen --min-roic 15 --max-sbc-ocf 25',
    desc: 'Filter 6,000+ issuers on 5Y ROIC, strict Owner Earnings, and share dilution.',
  },
  {
    cmd: 'm3 universe similar MCO --ticker --limit 10',
    desc: 'Discover businesses across the market whose economics rhyme with Core holdings.',
  },
  {
    cmd: 'm3 events catchup && m3 inbox list --status unread',
    desc: 'Catch up calmly on missed official filings and strike-band alerts in Inbox.',
  },
  {
    cmd: 'm3 storage status',
    desc: 'Inspect hardlink-aware local footprint and reclaim rebuildable SEC/DuckDB caches safely.',
  },
  {
    cmd: 'm3 mcp',
    desc: 'Serve 14 read-only deterministic research tools over stdio MCP for Claude Code, Codex, and Cursor.',
  },
];
