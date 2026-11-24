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
    id: 'ev-snps-strike',
    ticker: 'SNPS',
    company: 'Synopsys, Inc.',
    category: 'strike',
    kindLabel: 'Strike Zone',
    circleStatusLabel: 'Circle Ind. · EDA & Silicon IP',
    circleStatusTone: 'emerald',
    industrySlug: 'eda-silicon-ip',
    severity: 'emerald',
    timestamp: '14m ago',
    headline: 'Spot moved within +4.2% of human strike price ($468.00)',
    subline: 'Graduated Circle · 18–24x industry anchor · Tier-1 quote',
    businessProfile10k:
      'SEC 10-K Item 1: Licenses mission-critical Electronic Design Automation (EDA) verification tools and silicon IP blocks used across advanced sub-3nm chip tape-outs under multi-year time-based contracts.',
    evidenceTitle: 'FY25 Form 10-K — Multi-year EDA Backlog & Renewal Retention',
    evidenceQuote:
      'Committed backlog under non-cancellable multi-year software license and IP agreements reached $8.6B with >95% time-based visibility across leading-edge silicon nodes.',
    sourceUrl: 'https://www.sec.gov/Archives/edgar/data/883241/000088324126000018',
    similarityPct: '96%',
    metrics: [
      { label: 'Spot / Strike', value: '$487.60 / $468', note: '+4.2% above strike' },
      { label: 'Strict ROIC', value: '24.8%', note: '5Y deterministic median' },
      { label: 'Owner Earnings', value: '$1.39B', note: 'OCF − CapEx − SBC' },
      { label: 'Share Delta (3Y)', value: '−1.4% / yr', note: 'Net shrinkage' },
    ],
    summary:
      'Strike signal allowed strictly because EDA & Semiconductor IP graduated into your Circle (18x low anchor). Unfamiliar market companies never trigger blind strike claims.',
    status: 'unread',
  },
  {
    id: 'ev-veev-discovery',
    ticker: 'VEEV',
    company: 'Veeva Systems Inc.',
    category: 'discovery',
    kindLabel: 'Quant Screen',
    circleStatusLabel: 'Studying · Life Sciences Cloud',
    circleStatusTone: 'sky',
    industrySlug: 'life-sciences-cloud',
    severity: 'sky',
    timestamp: '2h ago',
    headline: 'High-ROIC compounder outside watchlist · 5Y ROIC 26.4% (no blind strike)',
    subline: 'SIC 7372 · Zero-LLM DuckDB 5Y Snapshot + SEC 10-K Item 1',
    businessProfile10k:
      'SEC 10-K Item 1: Cloud-based regulatory, clinical trial (Vault), and commercial software platform for global biopharmaceutical enterprises under multi-year enterprise subscriptions.',
    evidenceTitle: 'Item 1A & Item 7 — Vault Clinical Switching Costs & Platform Migration',
    evidenceQuote:
      'Regulated biopharma customers rely on Veeva Vault to maintain FDA 21 CFR Part 11 validated trial and quality documentation across multi-year drug pipelines.',
    sourceUrl: 'https://www.sec.gov/Archives/edgar/data/1393052/000139305226000014',
    similarityPct: '91%',
    metrics: [
      { label: '5Y ROIC', value: '26.4%', note: '5Y gross margin 73.1%' },
      { label: 'Strict FCF Yield', value: '4.1%', note: 'No strike until Circle' },
      { label: 'SBC / OCF', value: '29.4%', note: 'Deducted in full' },
      { label: 'Share Delta (3Y)', value: '+0.8% / yr', note: 'Net moderate dilution' },
    ],
    summary:
      'Surfaced as a Quantitative Candidate with inline 5Y financials and 10-K business summary. Launch "Research Industry" with Alan to study Vault switching economics before setting any strike price.',
    status: 'unread',
  },
  {
    id: 'ev-ptc-ripple',
    ticker: 'PTC',
    company: 'PTC Inc.',
    category: 'ripple',
    kindLabel: 'Latent Ripple',
    circleStatusLabel: 'Circle · WATCH',
    circleStatusTone: 'emerald',
    industrySlug: 'eda-silicon-ip',
    severity: 'violet',
    timestamp: 'Yesterday',
    headline: 'Operational rhyme detected against Core holding ADSK (0.89 cosine)',
    subline: 'Offline 10-K similarity scan · Windchill / Onshape switching costs',
    businessProfile10k:
      'SEC 10-K Item 1: Industrial CAD, Product Lifecycle Management (PLM), and IIoT software enabling discrete manufacturers to manage bill-of-materials revision histories.',
    evidenceTitle: 'Item 1 Business — Multi-Decade Product Lifecycle Traceability',
    evidenceQuote:
      'Windchill serves as the system of record for bill-of-materials and regulatory compliance across aerospace, medical device, and industrial OEM supply chains.',
    sourceUrl: 'https://www.sec.gov/Archives/edgar/data/857005/000085700525000077',
    similarityPct: '89%',
    metrics: [
      { label: 'Cosine Match', value: '0.89', note: 'vs ADSK 10-K disclosures' },
      { label: '5Y ROIC', value: '18.6%', note: 'Post-SaaS mix expansion' },
      { label: 'Owner Earnings', value: '$612M', note: 'Strict OCF − CapEx − SBC' },
      { label: 'Implied Growth', value: '9.4%', note: 'Reverse DCF @ 10% hurdle' },
    ],
    summary:
      'Full-market similarity search surfaced PTC outside initial habits after disclosure structure echoed mission-critical CAD/PLM switching barriers.',
    status: 'unread',
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
    timestamp: '2d ago',
    headline: 'Strict Owner Earnings negative for 8th quarter on foundry CapEx',
    subline: 'Basket: TOO HARD · Preserved as anti-pattern calibration fixture',
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
    slug: 'eda-silicon-ip',
    title: 'EDA & Semiconductor IP',
    sicLabel: 'SIC 7372 · Duopoly Software Tolls',
    status: 'inside_circle',
    statusLabel: 'Inside Circle ✓',
    anchorLabel: '18–24x Strict OE',
    boundTickers: ['SNPS', 'CDNS', 'ADSK', 'PTC'],
    feynmanChecklist: [
      {
        text: 'Explain why chip designers spend <5% of R&D on EDA yet cannot switch mid-node.',
        checked: true,
      },
      {
        text: 'Name the two failure modes from past EDA/IP roll-ups (debt after large M&A + foundry customer concentration).',
        checked: true,
      },
      {
        text: 'Articulate why 18–24x Strict Owner Earnings (after full SBC) is justified vs cyclical fab equipment.',
        checked: true,
      },
    ],
    peers: [
      {
        ticker: 'SNPS',
        name: 'Synopsys, Inc.',
        roic5y: '24.8%',
        gm5y: '80.2%',
        fcfYield: '4.1%',
        sbcOcf: '25.1%',
        price: '$487.60',
        refStrike: '$468.00',
      },
      {
        ticker: 'CDNS',
        name: 'Cadence Design',
        roic5y: '29.4%',
        gm5y: '89.1%',
        fcfYield: '3.4%',
        sbcOcf: '21.8%',
        price: '$294.20',
        refStrike: '$246.50',
      },
      {
        ticker: 'ADSK',
        name: 'Autodesk, Inc.',
        roic5y: '31.4%',
        gm5y: '91.0%',
        fcfYield: '3.9%',
        sbcOcf: '27.8%',
        price: '$278.40',
        refStrike: '$239.00',
      },
    ],
    perspectives: {
      value_chain: {
        title: '产业链与利润池 · Value Chain & Profit Pool',
        summary: 'Duopoly software tollbooth capturing recurring R&D budgets upstream of cyclical wafer fabs.',
        points: [
          'Synopsys + Cadence control >75% of front-end synthesis, place-and-route, and foundry sign-off certification.',
          'Time-based subscription contracts eliminate boom-bust license lumpiness seen in semiconductor capital equipment.',
        ],
      },
      moat: {
        title: '商业模式与护城河 · Business Model & Moats',
        summary: 'Foundry co-optimization + engineer muscle memory create multi-decade switching lock-in.',
        points: [
          'Every sub-3nm PDK from TSMC/Samsung is jointly validated on Synopsys & Cadence tools prior to customer tape-out.',
          'IP cores embed recurring royalty annuities directly into silicon shipped over 5–8 year product cycles.',
        ],
      },
      failure_modes: {
        title: '历史死法与周期陷阱 · Failure Modes & Traps',
        summary: 'Where vertical software/IP compounding breaks historically.',
        points: [
          'Over-leveraged mega-M&A (e.g., Ansys closing debt load) slowing net share buybacks during rate spikes.',
          'SBC masking true cash generation when equity grants exceed 25% of operating cash flow.',
        ],
      },
      valuation_kpis: {
        title: '估值锚与核心 KPI · Valuation Anchor & Key KPIs',
        summary: 'Human-selected 18–24x Strict Owner Earnings anchor after full SBC & CapEx deduction.',
        points: [
          'Track remaining performance obligations (RPO) growth, net SBC/OCF <28%, and IP royalty gross margin.',
          'Because this industry graduated to Inside Circle, SNPS now surfaces in Strike Radar near $468.',
        ],
      },
    },
    secCitations: [
      '[SEC 10-K: SNPS] FY25 Item 1 & Note 14 — $8.6B multi-year EDA/IP backlog',
      '[SEC 10-K: CDNS] FY25 Item 7 — 89.1% gross margin & recurring RPO mix',
    ],
    webCitations: [
      {
        title: 'TSMC Open Innovation Platform (OIP) EDA Ecosystem Certification',
        url: 'https://www.tsmc.com/english/dedicatedFoundry/technology/platform',
      },
      {
        title: 'IEEE Spectrum: Sub-3nm Tape-Out Sign-off Complexity & Verification Costs',
        url: 'https://spectrum.ieee.org/',
      },
    ],
    humanNotes:
      'Graduated to Inside Circle after verifying TSMC sign-off duplication risk. Hold strictly below 20x Strict OE unless post-Ansys debt pays down ahead of schedule.',
  },
  {
    slug: 'life-sciences-cloud',
    title: 'Life Sciences Regulated Cloud',
    sicLabel: 'SIC 7372 · Regulated R&D SaaS',
    status: 'studying',
    statusLabel: 'Studying',
    anchorLabel: '15–18x (Draft)',
    boundTickers: ['VEEV', 'IQV', 'MEDP'],
    feynmanChecklist: [
      {
        text: 'Compare Veeva CRM replatforming risk vs IQVIA data licensing lock-in.',
        checked: true,
      },
      {
        text: 'Verify clinical trial cycle sensitivity when biotech funding contracts.',
        checked: false,
      },
    ],
    peers: [
      {
        ticker: 'VEEV',
        name: 'Veeva Systems Inc.',
        roic5y: '26.4%',
        gm5y: '73.1%',
        fcfYield: '4.1%',
        sbcOcf: '29.4%',
        price: '$224.80',
        refStrike: '$194.20',
      },
      {
        ticker: 'MEDP',
        name: 'Medpace Holdings',
        roic5y: '41.2%',
        gm5y: '68.4%',
        fcfYield: '4.8%',
        sbcOcf: '3.9%',
        price: '$348.10',
        refStrike: '$292.00',
      },
    ],
    perspectives: {
      value_chain: {
        title: '产业链与利润池 · Value Chain & Profit Pool',
        summary: 'Comparing Clinical CRO execution vs GxP-validated Cloud Software Systems of Record.',
        points: [
          'Vault eTMF and CTMS own regulatory compliance trails whereas CROs monetize clinical trial labor hours.',
        ],
      },
      moat: {
        title: '商业模式与护城河 · Business Model & Moats',
        summary: 'FDA validation audits make migrating regulated clinical data extraordinarily risky.',
        points: ['Top-20 pharma Vault Clinical suite expansion offsets Commercial CRM migration noise.'],
      },
      failure_modes: {
        title: '历史死法与周期陷阱 · Failure Modes & Traps',
        summary: 'Customer biotech funding winter + Salesforce competitive bundling.',
        points: ['Monitor CRM attrition from Vault CRM migration vs Salesforce Life Sciences Cloud.'],
      },
      valuation_kpis: {
        title: '估值锚与核心 KPI · Valuation Anchor & Key KPIs',
        summary: 'Remains in Studying status — Strike Zone remains gated OFF until Feynman quiz passes.',
        points: ['R&D Solutions subscription growth >= 14% and net SBC/OCF compression below 25%.'],
      },
    },
    secCitations: ['[SEC 10-K: VEEV] FY25 Item 1 — Vault eTMF & EDC adoption across Top-20 biopharma'],
    webCitations: [
      {
        title: 'FDA 21 CFR Part 11 Electronic Records Compliance Guidelines',
        url: 'https://www.fda.gov/',
      },
    ],
    humanNotes:
      'Need to complete CRM transition analysis before deciding whether Veeva earns Circle graduation.',
  },
];

export const COMPANIES: CompanyRecord[] = [
  {
    ticker: 'SNPS',
    name: 'Synopsys, Inc.',
    bucket: 'Core',
    sector: 'EDA / Semiconductor IP',
    spot: '$487.60',
    strike: '$468.00',
    distanceLabel: '+4.2%',
    statusLabel: 'Near Strike',
    statusTone: 'gold',
    roic: '24.8%',
    ownerEarnings: '$1.39B',
    sbcToOcf: '25.1%',
    shareDelta: '−1.4%',
    similarityScore: '0.91',
    lookalikeNote: 'Duopoly engineering toolchain matching ADSK/CDNS switching durability',
    lastFiling: '10-K FY25 · 0000883241-26-000018',
    metrics: [
      {
        id: 'snps-roic',
        label: '5Y Median ROIC',
        value: '24.8%',
        formula: 'NOPAT / (Net Working Capital + Net PP&E + Acquired Intangibles adj.)',
        sha256: 'sha256:8f41c2b9...90e4a17d (us-gaap:OperatingIncomeLoss)',
        source: 'SEC EDGAR Company Facts · CIK 0000883241',
      },
      {
        id: 'snps-oe',
        label: 'Strict Owner Earnings',
        value: '$1.39B',
        formula: 'OCF ($2.08B) − CapEx ($168M) − SBC ($522M)',
        sha256: 'sha256:3a90de14...4b22f08c (us-gaap:ShareBasedCompensation)',
        source: 'Deterministic Accounting Engine · Exact Decimal',
      },
      {
        id: 'snps-shares',
        label: '3Y Share Count Delta',
        value: '−1.4% / yr',
        formula: 'Annualized change in diluted shares net of buybacks',
        sha256: 'sha256:71c40e52...11d92fa0 (dei:EntityCommonStockSharesOutstanding)',
        source: 'SEC EDGAR XBRL Primary Fact',
      },
      {
        id: 'snps-implied',
        label: 'Reverse Implied Growth',
        value: '11.2% / 10y',
        formula: 'Implied annual OE compounding required at current $487.60 spot (10% hurdle)',
        sha256: 'sha256:d049b661...6e71c309 (calculator::reverse_dcf)',
        source: 'Deterministic Reverse-DCF Solver',
      },
    ],
    livingMemoSummary: [
      'Physical tollbooth on silicon complexity: sub-3nm verification costs far less than one failed tape-out.',
      'Management GAAP-adjusted numbers understate true employee equity cost; M3 strips $522M SBC cleanly before valuation.',
      'Primary watch item remains balance-sheet deleveraging pace post-Ansys before expanding conviction weight.',
    ],
    perspectives: {
      moat: {
        title: 'Moat Durability · Duopoly Switching Barrier',
        tools: ['m3_lake_company_snapshot', 'm3_peer_percentiles'],
        points: [
          'Every sub-3nm chip requires Signoff + Synthesis co-certified against foundry PDKs; switching chains risks tape-out delays measured in tens of millions.',
          'Time-based licensing and IP royalties convert R&D scale directly into recurring cash flow with 95%+ renewal visibility.',
        ],
      },
      accounting: {
        title: 'Accounting Reality · Full SBC Deduction',
        tools: ['m3_xbrl_facts', 'm3_owner_earnings_audit'],
        points: [
          'Stock-based compensation represents ~25% of operating cash flow; strict Owner Earnings ($1.39B) stays meaningfully lower than headline non-GAAP cash flow.',
          'Gross deferred revenue growth supports genuine upfront cash collection rather than aggressive receivable pull-forward.',
        ],
      },
      latent_peers: {
        title: 'Latent Peers · Structural Twins Across 5,142 Filings',
        tools: ['m3_similar_companies', 'm3_excerpt_search'],
        points: [
          'Closest operational rhymes: CDNS (0.94 cosine), ADSK (0.91 cosine), PTC (0.86 cosine) — all exhibit mission-critical CAD/EDA file lock-in.',
        ],
      },
      premortem: {
        title: 'Pre-Mortem · How This Compounding Stops',
        tools: ['m3_filing_risk_scan', 'm3_balance_sheet_stress'],
        points: [
          'Post-Ansys acquisition leverage collides with a multi-year semiconductor capex digestion cycle.',
          'Export restrictions escalate faster than domestic custom-ASIC demand offsets geographic license losses.',
        ],
      },
    },
    humanJudgment: {
      ownerNote: 'Inside circle of competence (industrial design & semiconductor software). Wait calmly for $468 strike band or visible debt paydown.',
      falsification: 'Review immediately if strict Owner Earnings margin falls below 20% for two consecutive quarters or Net Debt / OE exceeds 4.0x.',
      convictionWeight: 'Tier-A Core · Paper Cap 18%',
    },
  },
  {
    ticker: 'ADSK',
    name: 'Autodesk, Inc.',
    bucket: 'Core',
    sector: 'Industrial CAD / AEC',
    spot: '$274.20',
    strike: '$248.00',
    distanceLabel: '+10.6%',
    statusLabel: 'Wait',
    statusTone: 'slate',
    roic: '31.4%',
    ownerEarnings: '$1.48B',
    sbcToOcf: '27.8%',
    shareDelta: '−0.9%',
    similarityScore: '1.00',
    lookalikeNote: 'Anchor reference vector for CAD/PLM compounders',
    lastFiling: '10-K FY26 · 0000769397-26-000142',
    metrics: [
      {
        id: 'adsk-roic',
        label: '5Y Median ROIC',
        value: '31.4%',
        formula: 'Asset-light subscription float yields high capital efficiency',
        sha256: 'sha256:1b88a7d0...53fc11e2 (us-gaap:OperatingIncomeLoss)',
        source: 'SEC EDGAR Company Facts · CIK 0000769397',
      },
      {
        id: 'adsk-oe',
        label: 'Strict Owner Earnings',
        value: '$1.48B',
        formula: 'OCF ($2.12B) − CapEx ($51M) − SBC ($589M)',
        sha256: 'sha256:c55190ae...8f02b419 (us-gaap:ShareBasedCompensation)',
        source: 'Deterministic Accounting Engine',
      },
      {
        id: 'adsk-shares',
        label: '3Y Share Count Delta',
        value: '−0.9% / yr',
        formula: 'Repurchases consistently exceed equity grants net of forfeitures',
        sha256: 'sha256:41f009bc...72ae384d (PaymentsForRepurchaseOfCommonStock)',
        source: 'SEC EDGAR XBRL Primary Fact',
      },
      {
        id: 'adsk-implied',
        label: 'Reverse Implied Growth',
        value: '13.6% / 10y',
        formula: 'Spot implies double-digit 10y Owner Earnings growth above hurdle',
        sha256: 'sha256:902e17ab...3a148c21 (calculator::reverse_dcf)',
        source: 'Deterministic Reverse-DCF Solver',
      },
    ],
    livingMemoSummary: [
      'DWG and Revit file standards act as a two-sided coordination lock across architects, structural firms, and general contractors.',
      'Margin of safety is insufficient at +10.6% above the human-set $248 strike; patience preserves optionality.',
    ],
    perspectives: {
      moat: {
        title: 'Moat Durability · Coordination-Game Standard',
        tools: ['m3_lake_company_snapshot', 'm3_peer_percentiles'],
        points: [
          'A contractor cannot switch CAD software unless downstream structural engineers and owners switch in lockstep.',
        ],
      },
      accounting: {
        title: 'Accounting Reality · SBC Discipline Improving',
        tools: ['m3_xbrl_facts'],
        points: [
          'True Owner Earnings yield sits at 3.9% after expensing $589M SBC in full — materially lower than headline FCF yields.',
        ],
      },
      latent_peers: {
        title: 'Latent Peers · CAD/PLM Cluster',
        tools: ['m3_similar_companies'],
        points: [
          'Surfaced PTC (0.89), SNPS (0.91), and Dassault Systemes as nearest operational cousins.',
        ],
      },
      premortem: {
        title: 'Pre-Mortem · Public BIM Mandates',
        tools: ['m3_filing_risk_scan'],
        points: [
          'Open-BIM public infrastructure standards plus stubbornly high SBC could compress terminal pricing power.',
        ],
      },
    },
    humanJudgment: {
      ownerNote: 'Verified high-quality franchise trading above strike band. Sit still and wait for $248.',
      falsification: 'Net retention slips below 100% or SBC/OCF rises back above 32%.',
      convictionWeight: 'Tier-A Core · Waiting at $248 strike',
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
    similarityScore: '0.78',
    lookalikeNote: 'Scale-economies-shared benchmark; rich multiple requires patience',
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
      'A strict 14% markup ceiling turns rising scale directly into member trust rather than bloated retail margin.',
      'Paying 50x+ Owner Earnings guarantees mediocre forward returns even in an extraordinary business.',
    ],
    perspectives: {
      moat: {
        title: 'Moat Durability · Scale Economies Shared',
        tools: ['m3_lake_company_snapshot'],
        points: [
          '76M households renew >92% annually because every purchasing efficiency lowers shelf price.',
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
          'Outlier across full US filings for negative working capital cycle paired with single-digit dilution.',
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
      ownerNote: 'Benchmark business model; stay disciplined on price and wait for broad market fear.',
      falsification: 'Renewal rate slips below 89% or markup discipline erodes.',
      convictionWeight: 'Core Watch · Patient at $640 strike',
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
    similarityScore: '0.84',
    lookalikeNote: 'Capital-return cannibal benchmark; disciplined buyback yield',
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
      'Two billion active devices anchor a daily habit platform with unmatched gross-margin stability and consistent share shrinkage.',
    ],
    perspectives: {
      moat: {
        title: 'Moat Durability · Installed-Base Habit Lock',
        tools: ['m3_lake_company_snapshot'],
        points: ['Hardware-software integration plus family ecosystem switching cost.'],
      },
      accounting: {
        title: 'Accounting Reality · Clean Cash Conversion',
        tools: ['m3_xbrl_facts'],
        points: ['SBC under 10% of OCF with steady −2.8% annual share shrinkage.'],
      },
      latent_peers: {
        title: 'Latent Peers',
        tools: ['m3_similar_companies'],
        points: ['Clustered with high-ecosystem toll platforms; lower regulatory risk diversification score than ADSK/SNPS.'],
      },
      premortem: {
        title: 'Pre-Mortem · App Store Unbundling',
        tools: ['m3_filing_risk_scan'],
        points: ['Global antitrust mandates chip away at high-margin Services take rates while smartphone replacement cycles lengthen.'],
      },
    },
    humanJudgment: {
      ownerNote: 'Hold discipline on strike price $185; refrain from chasing rich multiples.',
      falsification: 'Services gross margin compresses >400 bps due to regulatory fee caps.',
      convictionWeight: 'Core · Waiting at $185 strike',
    },
  },
  {
    ticker: 'PTC',
    name: 'PTC Inc.',
    bucket: 'Watch',
    sector: 'PLM / Industrial SaaS',
    spot: '$178.40',
    strike: '$162.00',
    distanceLabel: '+10.1%',
    statusLabel: 'In Research',
    statusTone: 'sky',
    roic: '18.6%',
    ownerEarnings: '$612M',
    sbcToOcf: '26.8%',
    shareDelta: '+0.4%',
    similarityScore: '0.89',
    lookalikeNote: 'Surfaced via L0+Latent lookalike query against ADSK',
    lastFiling: '10-K FY25 · 0000857005-25-000077',
    metrics: [
      {
        id: 'ptc-roic',
        label: '5Y Median ROIC',
        value: '18.6%',
        formula: 'Expanding post-Onshape/ServiceMax SaaS integration',
        sha256: 'sha256:55ab91de...1408cf22',
        source: 'SEC EDGAR Company Facts · CIK 0000857005',
      },
      {
        id: 'ptc-oe',
        label: 'Strict Owner Earnings',
        value: '$612M',
        formula: 'OCF ($865M) − CapEx ($21M) − SBC ($232M)',
        sha256: 'sha256:93f8102c...66ba43d0',
        source: 'Deterministic Accounting Engine',
      },
      {
        id: 'ptc-shares',
        label: '3Y Share Count Delta',
        value: '+0.4% / yr',
        formula: 'Debt paydown prioritized ahead of buybacks post-ServiceMax',
        sha256: 'sha256:08ce721f...9503aa18',
        source: 'SEC EDGAR XBRL Primary Fact',
      },
      {
        id: 'ptc-implied',
        label: 'Reverse Implied Growth',
        value: '9.4% / 10y',
        formula: 'Within conservative single-digit compounder expectations',
        sha256: 'sha256:7d1208cc...8815ff09',
        source: 'Deterministic Reverse-DCF Solver',
      },
    ],
    livingMemoSummary: [
      'Discovered outside initial habits by querying structural CAD/PLM twins of Autodesk across all US filings.',
    ],
    perspectives: {
      moat: {
        title: 'Moat Durability · Windchill BOM System of Record',
        tools: ['m3_lake_company_snapshot', 'm3_excerpt_search'],
        points: ['Regulated manufacturers rarely rip out 20-year bill-of-materials audit histories.'],
      },
      accounting: {
        title: 'Accounting Reality · M&A Goodwill Watch',
        tools: ['m3_xbrl_facts'],
        points: ['Incremental ROIC on acquired SaaS lines must clear 16% before Core promotion.'],
      },
      latent_peers: {
        title: 'Latent Peers · 0.89 Rhyme with ADSK',
        tools: ['m3_similar_companies'],
        points: ['Shares mission-critical engineering workflow vocabulary with ADSK and Dassault Systemes.'],
      },
      premortem: {
        title: 'Pre-Mortem · Suite Bundling Squeeze',
        tools: ['m3_filing_risk_scan'],
        points: ['Dassault and Siemens bundle end-to-end automotive suites faster than Onshape gains enterprise ground.'],
      },
    },
    humanJudgment: {
      ownerNote: 'Completing competitive moat check before deciding whether PTC earns Core bucket promotion.',
      falsification: 'Constant-currency ARR slows under 6% or gross churn breaches 8%.',
      convictionWeight: 'Watch · Under Evaluation',
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
    id: 'th-design',
    name: 'Industrial Design & Silicon Tolls',
    tickers: ['SNPS', 'ADSK', 'PTC'],
    signal: 'Cosine 0.89 · High switching cost',
    summary:
      'Mission-critical engineering workflows (EDA, BIM, PLM) where customer failure costs dwarf seat pricing and multi-year contracts enforce sticky renewals.',
  },
  {
    id: 'th-scale',
    name: 'Scale Economies Shared & Ecosystem Float',
    tickers: ['COST', 'AAPL'],
    signal: 'High ROIC · Awaiting price',
    summary:
      'Rare consumer franchises with structural trust or ecosystem lock; disciplined strike prices force patience rather than overpaying.',
  },
  {
    id: 'th-capex-trap',
    name: 'Capital-Intensive Treadmills (Too Hard)',
    tickers: ['INTC'],
    signal: 'Strict OE negative · Parked',
    summary:
      'Businesses forced to reinvest more than 100% of operating cash just to stay competitive on commodity capacity.',
  },
];

export const JOURNAL_ENTRIES: JournalEntry[] = [
  {
    date: '2026-11-24',
    ticker: 'SNPS',
    action: 'DEEP RESEARCH',
    tone: 'neutral',
    note: 'Completed 4-lens Alan dossier (Moat, Accounting, Latent Peers, Pre-Mortem). Holding $468 strike while watching Ansys deleveraging.',
  },
  {
    date: '2026-11-21',
    ticker: 'PTC',
    action: 'STRIKE WATCH',
    tone: 'strike',
    note: 'Discovered through latent analogy to ADSK (0.89 cosine). Reviewing Windchill renewal moat before considering Core promotion.',
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
      'When an opportunity clears every quantitative filter, M3 deploys four read-only EmbeddedAlan lenses—Moat, Accounting, Latent Peers, and a harsh Pre-Mortem—against locked filing facts before handing judgment back to you.',
    bullets: [
      'Structurally forbidden from inventing numbers or issuing buy/sell advice',
      'Hard monthly dollar cap enforces calm selectivity instead of shallow volume',
      'Preserves your personal falsification triggers and strike note alongside the memo',
    ],
    principle: 'Never hold a conviction you have not tried hard to disprove first.',
    cadence: 'Budget-gated deep interrogation ($10/mo default ceiling)',
    command: 'm3 budget status && m3 research run SNPS',
  },
];

export const CLI_COMMANDS: CliItem[] = [
  {
    cmd: 'm3 universe screen --munger --json',
    desc: 'Filter 5,000+ issuers on 5Y ROIC, strict Owner Earnings, and share shrinkage.',
  },
  {
    cmd: 'm3 universe similar ADSK --top 10',
    desc: 'Discover businesses across the market whose economics rhyme with Core holdings.',
  },
  {
    cmd: 'm3 events catchup && m3 inbox list',
    desc: 'Catch up calmly on missed official filings and strike-band alerts in Inbox.',
  },
  {
    cmd: 'm3 research run SNPS',
    desc: 'Run 4-perspective adversarial deep research behind your hard monthly budget gate.',
  },
  {
    cmd: 'm3 update channel beta && m3 update',
    desc: 'Switch release tracks and apply SHA-256 verified M3.app updates cleanly.',
  },
];
