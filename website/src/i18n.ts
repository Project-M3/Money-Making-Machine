import { createSignal } from 'solid-js'

/**
 * Copy for both locales. English is the default; the edge Worker picks the
 * initial locale (Cookie `m3_lang` > Accept-Language > en) and serves the
 * matching prerendered HTML. In the browser `setLocale()` switches every
 * string in place without a reload.
 */
export type Locale = 'en' | 'zh'

export const LANG_COOKIE = 'm3_lang'
export const SITE_URL = 'https://m3.thinksmart.systems'
export const REPO_URL = 'https://github.com/thinksmart-systems/m3'
export const RELEASES_URL = `${REPO_URL}/releases/latest`
export const ROADMAP_URL = `${REPO_URL}/blob/main/ROADMAP.md`
export const CHANGELOG_URL = `${REPO_URL}/blob/main/CHANGELOG.md`
export const LICENSE_URL = `${REPO_URL}/blob/main/LICENSE`
export const RESEARCH_URL = `${REPO_URL}/tree/main/docs/research`
export const ALAN_URL = 'https://alanworks.app'
export const SLOWPAGE_URL = 'https://slowpage.app'
export const AUTHOR_URL = 'https://realmorrisliu.com'
export const FALLBACK_VERSION = 'v0.6.0'

export const INSTALL_COMMANDS = {
  stable: 'curl -fsSL https://m3.thinksmart.systems/install.sh | bash',
  beta: 'curl -fsSL https://m3.thinksmart.systems/install.sh | bash -s -- --channel beta',
} as const

export function detectInitialLocale(): Locale {
  if (typeof document === 'undefined') return 'en'
  const match = document.cookie.match(/(?:^|;\s*)m3_lang=(en|zh)(?:;|$)/)
  if (match) return match[1] as Locale
  return document.documentElement.lang.startsWith('zh') ? 'zh' : 'en'
}

const [locale, setLocaleSignal] = createSignal<Locale>(detectInitialLocale())

export { locale }

/** Used by the build-time prerender to render each locale. */
export function setServerLocale(next: Locale) {
  setLocaleSignal(next)
}

export function setLocale(next: Locale) {
  if (typeof document !== 'undefined') {
    document.cookie = `${LANG_COOKIE}=${next}; Path=/; Max-Age=31536000; SameSite=Lax`
    document.documentElement.lang = next === 'zh' ? 'zh-CN' : 'en'
    document.title = dict[next].metaTitle
  }
  if (typeof window !== 'undefined') {
    ;(
      window as unknown as { posthog?: { register?: (props: Record<string, unknown>) => void } }
    ).posthog?.register?.({ locale: next })
  }
  setLocaleSignal(next)
}

type Rule = { n: string; title: string; body: string; proof: string }
type Correction = { title: string; when: string; verdict: string; body: string; link: string }
type Milestone = { v: string; date: string; title: string; body: string }
type Hotspot = { title: string; body: string }
type Slide = { src: string; caption: string }

const en = {
  metaTitle: 'M3 — the research instrument for patient investors',

  navLabel: 'Sections',
  navWork: 'Show your work',
  navTry: 'The desk',
  navRules: 'Editorial',
  navRoadmap: 'Roadmap',
  navOtherLocale: '中文',
  navOtherLocaleLabel: 'Switch to Chinese',
  navDownload: 'Download',
  topLeft: 'Open source · Apache-2.0',

  tagline: 'The research instrument for patient investors',
  datelineIssuers: 'US issuers 6,000+',
  datelineArith: 'LLM arithmetic 0',
  datelineMcp: 'MCP tools 40 · 24 read-only',

  kicker: 'Value research, on your Mac',
  h1a: 'Read every filing.',
  h1b: 'Swing only at the fat pitches.',
  deck: 'M3 indexes every US SEC filer on your own machine, computes owner earnings, ROIC and dilution in deterministic Rust — traceable to the exact filing fact — and leaves every judgment to you.',
  ctaDownload: 'Download for macOS',
  ctaGithub: 'View on GitHub',
  fine: 'Free and open source · macOS 14+, Apple Silicon · A research tool, not investment advice.',

  ledgerTitle: 'This week’s ledger',
  ledgerSub: 'From the whole market to one afternoon · demo workbench data',
  ledger: [
    ['5,142', 'US issuers in the local lakehouse'],
    ['8', 'companies inside your circle'],
    ['3', 'briefings waiting in the inbox'],
    ['1', 'is near its strike price'],
  ] as Array<[string, string]>,
  quote: '“Knowing every alternative removes the urge to swing early.”',
  quoteBy: '— Editorial rule 04, Sit calmly',

  figLabel: 'Fig. 1',
  figCaption: 'The Briefing Inbox only alerts on companies inside your circle.',
  figSource: 'M3.app, native macOS · demo data',
  figAlt:
    'M3 desktop app showing the Briefing Inbox: a Strike Zone alert for Moody’s with spot and strike price, strict ROIC, owner earnings, share delta and a verbatim 10-K excerpt.',

  workKicker: 'Show your work',
  workTitle: 'Every number comes with its arithmetic and its filing.',
  workLead:
    'Language models never touch the accounting. Rust computes each figure with exact decimals from SEC EDGAR XBRL facts, and keeps the SHA-256 of the filing it came from. Pick a figure and follow it back to the 10-K.',
  workTabsLabel: 'Worked examples',
  workStep1: 'The number',
  workStep2: 'The arithmetic',
  workStep3: 'The source',
  workNote: 'Figures from the public demo fixture (Moody’s, FY25). In the app, every value is computed from your own downloaded filings.',
  workEngine: 'rust_decimal · no floating point · no LLM',

  tryKicker: 'Inside M3.app',
  tryTitle: 'The whole research loop, in one window.',
  tryLead:
    'Screen the market, interrogate a business, study an industry, tend your circle, and log every call — all over plain files on your Mac.',
  fig2Label: 'Fig. 2',
  fig2Caption: 'Interactive — click around, or press 1–5.',
  fig2MobileCaption: 'Swipe through the five views.',
  fig2Source: 'M3.app, macOS · sample data',
  macMenus: ['File', 'View', 'Circle', 'Alan', 'Window'] as string[],
  macClock: 'Thu 09:41',
  tryLoading: 'Opening M3.app…',
  tryHotspotsLabel: 'Guided tour',
  hotspots: [
    { title: 'Circle-gated inbox', body: 'Alerts fire only for companies you have studied. Everything else stays a quiet screen.' },
    { title: 'Verbatim evidence', body: 'Each card quotes the filing word for word, with a link to sec.gov.' },
    { title: 'Your circle', body: 'WATCH → IN → OUT → TOO HARD. Only you move a company between tiers.' },
    { title: 'Command palette', body: 'Press ⌘K to jump anywhere, as in the desktop app.' },
  ] as Hotspot[],
  slides: [
    { src: '/shots/wb-inbox', caption: 'Briefing Inbox — alerts only inside your circle' },
    { src: '/shots/wb-research', caption: 'Company Desk — four lenses and the provenance of every figure' },
    { src: '/shots/wb-universe', caption: 'Universe Radar — screen the full US market locally' },
    { src: '/shots/wb-industry', caption: 'Industry Research — study an industry before you trust a number' },
    { src: '/shots/wb-journal', caption: 'Decision Journal — every call, kept as written' },
  ] as Slide[],
  carouselLabel: 'Workbench screenshots',
  carouselPrev: 'Previous screenshot',
  carouselNext: 'Next screenshot',

  rulesKicker: 'Editorial',
  rulesTitle: 'Four iron rules',
  rulesLead:
    'Charlie Munger’s habit of inverting a problem before believing an answer is the inspiration. These four rules are how the software keeps to it.',
  rules: [
    {
      n: '01',
      title: 'Arithmetic is sacred',
      body: 'No language model estimates, completes or rounds a financial figure. ROIC, strict owner earnings (OCF − CapEx − SBC), dilution and reverse-implied growth are solved in Rust from primary filings.',
      proof: 'Zero LLM arithmetic · SHA-256 provenance',
    },
    {
      n: '02',
      title: 'Invert, always invert',
      body: 'Deep research asks where a thesis dies before it asks what it pays: moat, accounting reality, latent peers and a five-year pre-mortem, each argued separately.',
      proof: 'Four adversarial lenses · TOO HARD is a valid answer',
    },
    {
      n: '03',
      title: 'Judgment belongs to the owner',
      body: 'M3 never says buy or sell. Your stance, weights and falsification metrics are written by you, and the AI cannot overwrite them.',
      proof: 'Human-only fields · Feynman check before IN',
    },
    {
      n: '04',
      title: 'Sit calmly',
      body: 'It runs on your Mac with no cloud backend. The filing inbox costs zero tokens, AI research has hard monthly caps, and a quiet week says so: “No fat pitches · Stay patient.”',
      proof: 'Local-first · Hard budget gate',
    },
  ] as Rule[],

  corrKicker: 'Corrections',
  corrTitle: 'Strategies we tried and abandoned',
  corrLead:
    'Before value research, this project tested short-horizon trading ideas under pre-registered rules. They failed, and the evidence stays in the repository, unedited.',
  corrections: [
    {
      title: 'BTC one-hour SMA20 trend rule',
      when: 'Pre-registered October 2026 · 2025 holdout',
      verdict: 'Abandoned',
      body: 'A 1,000 USDT simulated account lost 162.13 USDT (−16.21%) at standard costs over 525 round trips, with a 16.80% drawdown against 4.24% for buy-and-hold.',
      link: `${REPO_URL}/blob/main/docs/research/sma20-history-review.md`,
    },
    {
      title: 'BTC funding-rate carry hedge',
      when: 'Pre-registered October 2026 · 2024–2025',
      verdict: 'Not pursued',
      body: 'On ¥10,000, about ¥76 a year after assumed transfer costs, −¥40 at double costs, and the 2024 run showed the hedge’s margin wallet could run short.',
      link: `${REPO_URL}/blob/main/docs/research/funding-carry-review.md`,
    },
    {
      title: '48-hour live paper soak',
      when: 'October 2026',
      verdict: 'Failed acceptance',
      body: 'The loop traded and reconciled, but an 11-hour sleep gap broke the continuity requirement, and nothing showed a lasting edge.',
      link: `${REPO_URL}/blob/main/docs/research/paper-soak-review.md`,
    },
  ] as Correction[],
  corrFooter: 'Read every review',
  corrReview: 'Read the review',

  roadKicker: 'Roadmap',
  roadTitle: 'Shipped, so far',
  milestones: [
    { v: 'v0.6.5', date: '27 Nov 2026', title: 'Unified Alan & 40-tool MCP', body: 'Dual-canvas Alan workspace with pinned context, inline evidence cards, desktop control IPC, and a 40-tool `m3 mcp` server (`--read-only` keeps 24).' },
    { v: 'v0.6', date: '26 Nov 2026', title: 'Proactive Alan', body: 'Filing watchdog for WATCH + IN companies, calm daily and weekly briefings, a menu-bar keep-alive and a three-round Feynman coach.' },
    { v: 'v0.5', date: '26 Nov 2026', title: 'Onboarding and MCP', body: 'In-app setup for five model providers, a starter workspace, progressive lakehouse bootstrap and `m3 mcp`.' },
    { v: 'v0.4', date: '25 Nov 2026', title: 'Native macOS workbench', body: 'Rewritten on Longbridge NSH with a deterministic core, K-line and ROIC charts, a storage manager and verified in-app updates.' },
  ] as Milestone[],
  roadNext:
    'Next is point-in-time strategy backtesting and a local paper broker (v0.7), followed by Hong Kong filings. Further out, strictly opt-in broker connections sit behind a deterministic Rust risk gate.',
  roadLink: 'Read ROADMAP.md',

  installKicker: 'Install',
  installTitle: 'One line, on your desk.',
  installLead:
    'Installs /Applications/M3.app and the m3 command, verifies SHA-256 checksums from GitHub Releases, and updates in place from verified releases.',
  channelLabel: 'Release channel',
  channelStable: 'Stable',
  channelBeta: 'Beta',
  copy: 'Copy',
  copied: 'Copied',
  copyLabel: 'Copy install command',
  installReq: 'macOS 14+ · Apple Silicon',
  installReleases: 'All releases',
  cliTitle: 'Then, in the terminal',
  cli: [
    ['m3 doctor', 'Check the binary, workspace, lakehouse and storage'],
    ['m3 universe sync --full', 'Build the local US lakehouse from SEC EDGAR'],
    ['m3 universe screen --min-roic 15 --max-sbc-ocf 25', 'Screen on ROIC and stock-based compensation'],
    ['m3 inbox list --status unread', 'Read what changed, filing by filing'],
    ['m3 mcp', 'Serve read-only research tools to your agents'],
  ] as Array<[string, string]>,

  footerDisclaimer:
    'M3 is a local research and paper-journaling tool. It connects to no brokerage and places no orders. Nothing it produces is investment advice.',
  footerPrivacy:
    'This site counts anonymous visits and clicks with PostHog. No session recording, no personal profiles.',
  footerPowered: 'Powered by Alan',
  footerFamily: 'Also by the author',
  footerLicense: 'Apache-2.0',
  footerChangelog: 'Changelog',
  eggStates: [
    ['M3', ''],
    ['M × M × M', 'Compound patiently.'],
    ['Money · Making · Machine', 'The machine reads the filings; you swing at the fat pitches.'],
  ] as Array<[string, string]>,
  eggLabel: 'Inspect the M³ engraving',
}

const zh: typeof en = {
  metaTitle: 'M3 — 耐心投资者的研究仪器',

  navLabel: '版块',
  navWork: '演算过程',
  navTry: '工作台',
  navRules: '社论',
  navRoadmap: '路线图',
  navOtherLocale: 'English',
  navOtherLocaleLabel: '切换到英文',
  navDownload: '下载',
  topLeft: '开源 · Apache-2.0',

  tagline: '耐心投资者的研究仪器',
  datelineIssuers: '美股发行人 6,000+',
  datelineArith: 'LLM 算术 0',
  datelineMcp: 'MCP 工具 40 · 24 只读',

  kicker: '价值研究，就在你的 Mac 上',
  h1a: '读遍每一份财报。',
  h1b: '只挥击好球。',
  deck: 'M3 在你自己的电脑上索引全部美股 SEC 申报人，用确定性的 Rust 计算股东盈余、ROIC 与股本稀释，每个数字都能追溯到财报里的那一条事实，所有判断都留给你。',
  ctaDownload: '下载 macOS 版',
  ctaGithub: '在 GitHub 查看',
  fine: '免费开源 · macOS 14+，Apple Silicon · 研究工具，不构成投资建议。',

  ledgerTitle: '本周账本',
  ledgerSub: '从全市场到一个下午 · 演示工作台数据',
  ledger: [
    ['5,142', '家美股发行人在本地湖仓'],
    ['8', '家公司在你的能力圈内'],
    ['3', '条简报在收件箱等你'],
    ['1', '家接近击球价'],
  ],
  quote: '“知道所有备选，才不急着出手。”',
  quoteBy: '— 社论第 04 条，安坐',

  figLabel: '图 1',
  figCaption: '简报收件箱只为能力圈内的公司发出提醒。',
  figSource: 'M3.app，原生 macOS · 演示数据',
  figAlt:
    'M3 桌面应用的简报收件箱：穆迪的击球区提醒，显示现价与击球价、严格 ROIC、股东盈余、股本变化以及 10-K 原文摘录。',

  workKicker: '演算过程',
  workTitle: '每个数字都附上算式和出处。',
  workLead:
    '财务算术从不交给语言模型。Rust 用精确小数从 SEC EDGAR XBRL 事实算出每个数字，并保留来源财报的 SHA-256。选一个数字，一路追回 10-K。',
  workTabsLabel: '演算示例',
  workStep1: '数字',
  workStep2: '算式',
  workStep3: '出处',
  workNote: '数字来自公开演示数据（穆迪，FY25）。在应用里，每个值都由你自己下载的财报算出。',
  workEngine: 'rust_decimal · 不用浮点 · 不用 LLM',

  tryKicker: 'M3.app',
  tryTitle: '整套研究闭环，收进一个窗口。',
  tryLead:
    '筛全市场、审讯公司、拆解行业、守住能力圈、记下每一次判断——全部读写你 Mac 上的本地文件。',
  fig2Label: '图 2',
  fig2Caption: '可交互 — 直接点击，或按 1–5 切换视图。',
  fig2MobileCaption: '滑动查看五个视图。',
  fig2Source: 'M3.app，macOS · 样本数据',
  macMenus: ['文件', '视图', '能力圈', 'Alan', '窗口'],
  macClock: '周四 09:41',
  tryLoading: '正在打开 M3.app…',
  tryHotspotsLabel: '导览',
  hotspots: [
    { title: '能力圈门控的收件箱', body: '只有你研究过的公司才会提醒，其他公司安静地留在筛选里。' },
    { title: '原文证据', body: '每张卡片逐字引用财报原文，并链接到 sec.gov。' },
    { title: '你的能力圈', body: 'WATCH → IN → OUT → TOO HARD，只有你能调整公司所在的层级。' },
    { title: '命令面板', body: '按 ⌘K 跳到任何地方，和桌面应用一样。' },
  ],
  slides: [
    { src: '/shots/wb-inbox', caption: '简报收件箱 — 只提醒能力圈内的公司' },
    { src: '/shots/wb-research', caption: '公司研究台 — 四个视角，每个数字都有出处' },
    { src: '/shots/wb-universe', caption: '全市场雷达 — 在本地筛选整个美股' },
    { src: '/shots/wb-industry', caption: '行业研究 — 先懂行业，再信数字' },
    { src: '/shots/wb-journal', caption: '决策账本 — 每个决定都按原样保留' },
  ],
  carouselLabel: '工作台截图',
  carouselPrev: '上一张截图',
  carouselNext: '下一张截图',

  rulesKicker: '社论',
  rulesTitle: '四条铁律',
  rulesLead: '灵感来自查理·芒格的习惯：先反过来想，再相信答案。这四条规则是软件守住它的方式。',
  rules: [
    {
      n: '01',
      title: '算术神圣',
      body: '语言模型不估算、不补全、不凑整任何财务数字。ROIC、严格股东盈余（OCF − CapEx − SBC）、稀释率和反推增速，全部由 Rust 从一手财报算出。',
      proof: 'LLM 算术为零 · SHA-256 溯源',
    },
    {
      n: '02',
      title: '反过来想，总是反过来想',
      body: '深度研究先问论点会在哪里失效，再问它能赚多少：护城河、会计真相、潜在同类公司和五年事前验尸，分别论证。',
      proof: '四个对抗视角 · “太难”也是答案',
    },
    {
      n: '03',
      title: '判断属于主人',
      body: 'M3 从不说买或卖。你的立场、仓位理由和证伪指标只能由你填写，AI 无法覆盖。',
      proof: '仅限人类的字段 · 进入 IN 前先过费曼检验',
    },
    {
      n: '04',
      title: '安坐',
      body: '它运行在你的 Mac 上，没有云端后台。财报收件箱不花 token，AI 研究有每月硬上限，平静的一周会直说：“无球可挥 · 耐心等待”。',
      proof: '本地优先 · 预算硬门槛',
    },
  ],

  corrKicker: '更正栏',
  corrTitle: '试过并放弃的策略',
  corrLead: '在转向价值研究之前，这个项目按预先登记的规则测试过短线交易想法。它们失败了，证据原样留在仓库里。',
  corrections: [
    {
      title: 'BTC 一小时 SMA20 趋势规则',
      when: '2026 年 10 月预登记 · 2025 留出检验',
      verdict: '已放弃',
      body: '1,000 USDT 模拟账户在标准成本下亏损 162.13 USDT（−16.21%），全年 525 轮买卖，最大回撤 16.80%，同仓位买入持有仅 4.24%。',
      link: `${REPO_URL}/blob/main/docs/research/sma20-history-review.md`,
    },
    {
      title: 'BTC 资金费率对冲',
      when: '2026 年 10 月预登记 · 2024–2025',
      verdict: '不再推进',
      body: '一万元本金扣除假设的出入金损耗后一年约 76 元，双倍成本下为 −40 元；2024 年还暴露出合约钱包保证金可能不足。',
      link: `${REPO_URL}/blob/main/docs/research/funding-carry-review.md`,
    },
    {
      title: '48 小时实时模拟运行',
      when: '2026 年 10 月',
      verdict: '未通过验收',
      body: '买卖与对账闭环跑通了，但电脑休眠造成 11 小时缺口，连续性不达标，也没有证据显示长期优势。',
      link: `${REPO_URL}/blob/main/docs/research/paper-soak-review.md`,
    },
  ],
  corrFooter: '阅读全部复盘',
  corrReview: '阅读复盘',

  roadKicker: '路线图',
  roadTitle: '已经发布的',
  milestones: [
    { v: 'v0.6.5', date: '2026-11-27', title: '统一的 Alan 与 40 工具 MCP', body: '双画布 Alan 工作区、上下文锁定、内联证据卡、桌面 IPC 控制，以及 40 工具 `m3 mcp` 服务（`--read-only` 保留 24 个）。' },
    { v: 'v0.6', date: '2026-11-26', title: '主动的 Alan', body: '盯住 WATCH + IN 公司的财报哨兵、平静的日报与周报、菜单栏常驻，以及三轮费曼答辩教练。' },
    { v: 'v0.5', date: '2026-11-26', title: '上手与 MCP', body: '应用内配置五家模型服务、入门工作区、渐进式湖仓初始化，以及 `m3 mcp` 服务。' },
    { v: 'v0.4', date: '2026-11-25', title: '原生 macOS 工作台', body: '基于 Longbridge NSH 重写，确定性内核、K 线与 ROIC 图表、存储管理和校验过的应用内更新。' },
  ],
  roadNext: '下一步是防前视策略回测与本地模拟盘（v0.7），随后扩展港股财报。更远的计划包含严格选择加入、受确定性 Rust 风控约束的券商连接。',
  roadLink: '阅读 ROADMAP.md',

  installKicker: '安装',
  installTitle: '一行命令，装到你的桌面。',
  installLead: '安装 /Applications/M3.app 和 m3 命令，从 GitHub Releases 校验 SHA-256，并用校验过的版本原地更新。',
  channelLabel: '发布通道',
  channelStable: '稳定版',
  channelBeta: '测试版',
  copy: '复制',
  copied: '已复制',
  copyLabel: '复制安装命令',
  installReq: 'macOS 14+ · Apple Silicon',
  installReleases: '全部版本',
  cliTitle: '然后，在终端里',
  cli: [
    ['m3 doctor', '检查程序、工作区、湖仓与存储'],
    ['m3 universe sync --full', '从 SEC EDGAR 构建本地美股湖仓'],
    ['m3 universe screen --min-roic 15 --max-sbc-ocf 25', '按 ROIC 与股权激励占比筛选'],
    ['m3 inbox list --status unread', '逐份阅读新的申报'],
    ['m3 mcp', '把只读研究工具提供给你的 Agent'],
  ],

  footerDisclaimer: 'M3 是本地研究与模拟决策记录工具，不连接券商，也不下单。它产出的任何内容都不构成投资建议。',
  footerPrivacy: '本站用 PostHog 统计匿名访问与点击，不录屏，不识别个人。',
  footerPowered: 'Powered by Alan',
  footerFamily: '同一作者',
  footerLicense: 'Apache-2.0',
  footerChangelog: '更新日志',
  eggStates: [
    ['M3', ''],
    ['M × M × M', '耐心复利。'],
    ['Money · Making · Machine', '机器读财报，你只挥击好球。'],
  ],
  eggLabel: '查看 M³ 刻印',
}

const dict = { en, zh }

export const t: typeof en = new Proxy({} as typeof en, {
  get(_target, prop) {
    return dict[locale()][prop as keyof typeof en]
  },
})

export function dictFor(l: Locale) {
  return dict[l]
}
