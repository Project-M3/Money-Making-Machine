import {
  COMPANIES,
  INBOX_EVENTS,
  INDUSTRIES,
  JOURNAL_ENTRIES,
  type CompanyRecord,
  type CompanySubTab,
  type InboxEvent,
  type InboxFilter,
  type IndustryPerspectiveKey,
  type PerspectiveKey,
  type UniverseFilter,
  type UniverseSearchMode,
  type WorkbenchTab,
} from '../data/fixture';

interface DockNotice {
  tag: string;
  message: string;
  tone: 'emerald' | 'sky' | 'gold' | 'coral';
  dismissible: boolean;
}

interface WorkbenchState {
  tab: WorkbenchTab;
  companySubTab: CompanySubTab;
  basketFilter: CompanyRecord['bucket'] | 'All';
  inboxFilter: InboxFilter;
  selectedEventId: string;
  eventStatuses: Record<string, InboxEvent['status']>;
  universeFilter: UniverseFilter;
  universeSearchMode: UniverseSearchMode;
  selectedTicker: string;
  perspective: PerspectiveKey;
  selectedIndustrySlug: string;
  industryPerspective: IndustryPerspectiveKey;
  feynmanChecked: Record<string, boolean>;
  settingsLang: 'en' | 'zh';
  settingsChannel: 'stable' | 'beta';
  openProvenanceId: string | null;
  paletteOpen: boolean;
  paletteQuery: string;
  dockNotice: DockNotice | null;
}

const PERSPECTIVE_LABELS: Array<{ key: PerspectiveKey; label: string }> = [
  { key: 'moat', label: 'Moat' },
  { key: 'accounting', label: 'Accounting' },
  { key: 'latent_peers', label: 'Latent Peers' },
  { key: 'premortem', label: 'Pre-Mortem' },
];

const IND_PERSPECTIVE_LABELS: Array<{ key: IndustryPerspectiveKey; label: string }> = [
  { key: 'value_chain', label: 'Value Chain' },
  { key: 'moat', label: 'Moats' },
  { key: 'failure_modes', label: 'Failure Modes' },
  { key: 'valuation_kpis', label: 'Anchor & KPIs' },
];

const NAV_ITEMS: Array<{
  tab: WorkbenchTab;
  label: string;
  zhLabel: string;
  shortcut: string;
  dotColor: string;
}> = [
  { tab: 'inbox', label: 'Briefing Inbox', zhLabel: '简报收件箱', shortcut: '⌘1', dotColor: 'var(--emerald)' },
  { tab: 'universe', label: 'Universe Radar', zhLabel: '全市场雷达', shortcut: '⌘2', dotColor: 'var(--sky)' },
  { tab: 'industry', label: 'Industry Research', zhLabel: '行业研究', shortcut: '⌘3', dotColor: 'var(--violet)' },
  { tab: 'baskets', label: 'Circle', zhLabel: '能力圈总览', shortcut: '⌘4', dotColor: 'var(--ink-hero)' },
  { tab: 'journal', label: 'Decision Journal', zhLabel: '决策账本', shortcut: '⌘5', dotColor: 'var(--gold)' },
];

function escapeHtml(raw: string): string {
  return raw
    .replaceAll('&', '&amp;')
    .replaceAll('<', '&lt;')
    .replaceAll('>', '&gt;')
    .replaceAll('"', '&quot;');
}

function conceptHint(tip: string): string {
  return `<span class="concept-hint" tabindex="0" role="note" aria-label="${escapeHtml(tip)}" data-tip="${escapeHtml(tip)}">?</span>`;
}

function toneClass(tone: CompanyRecord['statusTone'] | InboxEvent['severity']): string {
  switch (tone) {
    case 'emerald':
      return 'pill-emerald';
    case 'gold':
      return 'pill-gold';
    case 'sky':
      return 'pill-sky';
    case 'coral':
      return 'pill-coral';
    case 'violet':
      return 'pill-violet';
    default:
      return 'pill-slate';
  }
}

export function mountWorkbench(container: HTMLElement): void {
  const state: WorkbenchState = {
    tab: 'inbox',
    companySubTab: 'alan',
    basketFilter: 'All',
    inboxFilter: 'unread',
    selectedEventId: INBOX_EVENTS[0].id,
    eventStatuses: Object.fromEntries(INBOX_EVENTS.map((e) => [e.id, e.status])),
    universeFilter: 'all',
    universeSearchMode: 'keyword',
    selectedTicker: 'MCO',
    perspective: 'premortem',
    selectedIndustrySlug: INDUSTRIES[0].slug,
    industryPerspective: 'value_chain',
    feynmanChecked: Object.fromEntries(
      INDUSTRIES.flatMap((ind) =>
        ind.feynmanChecklist.map((q, idx) => [`${ind.slug}:${idx}`, q.checked]),
      ),
    ),
    settingsLang: 'en',
    settingsChannel: 'stable',
    openProvenanceId: 'mco-oe',
    paletteOpen: false,
    paletteQuery: '',
    dockNotice: {
      tag: 'CATCH-UP',
      message: 'Circle-gated Strike Zone active. 3 unread briefings await triage.',
      tone: 'emerald',
      dismissible: true,
    },
  };

  let markClicks = 0;
  let markLastClickMs = 0;

  function getBreadcrumb(): string {
    const company = COMPANIES.find((c) => c.ticker === state.selectedTicker) || COMPANIES[0];
    const ind = INDUSTRIES.find((i) => i.slug === state.selectedIndustrySlug) || INDUSTRIES[0];
    const zh = state.settingsLang === 'zh';
    switch (state.tab) {
      case 'inbox':
        return zh ? '简报收件箱 · Event-Driven L1' : 'Briefing Inbox · Event-Driven L1';
      case 'universe':
        return zh ? '全市场雷达 · US Lakehouse (5,142)' : 'Universe Radar · Full-US Ledger (5,142)';
      case 'industry':
        return `${zh ? '行业研究' : 'Industry Studio'} · ${ind.title} (${ind.statusLabel})`;
      case 'baskets':
        return state.basketFilter === 'All'
          ? zh
            ? '能力圈总览 · Circle Overview'
            : 'Circle · Overview'
          : `Circle / ${state.basketFilter}`;
      case 'research':
        return `${company.ticker} · ${company.name} · Research Desk`;
      case 'journal':
        return zh ? '决策账本 · Decision Journal' : 'Decision Journal · Append-Only Ledger';
      case 'settings':
        return zh ? '设置 · Preferences & Guardrails' : 'Settings · Preferences & Guardrails';
    }
  }

  function getStatusLegend(): { dot: string; legend: string; mode: string } {
    if (state.paletteOpen) {
      return {
        dot: 'var(--sky)',
        legend: 'click or type to filter · ↵ jump · esc close palette',
        mode: 'COMMAND PALETTE',
      };
    }
    switch (state.tab) {
      case 'inbox':
        return {
          dot: 'var(--emerald)',
          legend: 'w watch · x too hard · e archive · research industry · ⌘K commands',
          mode: 'CIRCLE-GATED L1 · 0 TOKEN',
        };
      case 'universe':
        return {
          dot: 'var(--sky)',
          legend: 'presets · lookalike toggle · research industry · ↵ inspect · ⌘K commands',
          mode: 'DUCKDB L0/L1 · ZERO-LLM',
        };
      case 'industry':
        return {
          dot: 'var(--violet)',
          legend: '4-lens Alan study · SEC 10-K vs Web · Feynman check · human valuation anchor',
          mode: 'EMBEDDED ALAN + WEB · PAPER ONLY',
        };
      case 'baskets':
        return {
          dot: 'var(--emerald)',
          legend: 'filter bucket · click company to inspect Research Desk · ⌘K commands',
          mode: 'ZERO-LLM · PAPER ONLY',
        };
      case 'research':
        return {
          dot: state.companySubTab === 'alan' ? 'var(--violet)' : 'var(--emerald)',
          legend: '1 Memo · 2 Deep Research & Alan · 3 XBRL · esc back to Circle',
          mode: state.companySubTab === 'alan' ? 'EMBEDDED ALAN · PAPER ONLY' : 'ZERO-LLM ARITHMETIC',
        };
      case 'journal':
        return {
          dot: 'var(--gold)',
          legend: 'append-only pre-committed decision journal vs S&P 500 · ⌘K commands',
          mode: 'ZERO-LLM · PAPER ONLY',
        };
      case 'settings':
        return {
          dot: 'var(--sky)',
          legend: 'live Zh/En bilingual switch · release channel · L2 budget guardrails',
          mode: 'SETTINGS · LOCAL-FIRST',
        };
    }
  }

  function render(): void {
    const unreadCount = Object.values(state.eventStatuses).filter((s) => s === 'unread').length;
    const status = getStatusLegend();

    container.innerHTML = `
      <div class="m3-window">
        ${renderSidebar(unreadCount)}

        <div class="m3-viewport-gutter">
          <div class="m3-viewport">
            <div class="m3-toolbar">
              <div class="m3-toolbar-left">
                ${
                  state.tab === 'research'
                    ? `<button type="button" class="tb-back-btn" data-back-baskets>←</button>`
                    : ''
                }
                <span class="m3-breadcrumb">${escapeHtml(getBreadcrumb())}</span>
              </div>

              <div class="m3-toolbar-right">
                <div class="bench-pill">
                  <span style="color: var(--ink-muted)">S&amp;P 500</span>
                  <strong class="mono tabular" style="color: var(--ink-primary)">5,942.10</strong>
                </div>
                <span class="sync-indicator">
                  <span class="mini-dot" style="background: var(--emerald)"></span>
                  <span>14m ago</span>
                </span>
                <button type="button" class="tb-sync-btn" data-trigger-sync>
                  <span>Sync</span>
                  <span class="m3-kbd">⌘R</span>
                </button>
              </div>
            </div>

            <div class="m3-main-body">
              ${
                state.tab === 'inbox'
                  ? renderInboxView()
                  : state.tab === 'universe'
                    ? renderUniverseView()
                    : state.tab === 'industry'
                      ? renderIndustryView()
                      : state.tab === 'baskets'
                        ? renderBasketsView()
                        : state.tab === 'research'
                          ? renderResearchView()
                          : state.tab === 'settings'
                            ? renderSettingsView()
                            : renderJournalView()
              }
            </div>

            <div class="m3-statusline">
              <div class="statusline-group">
                <span class="mini-dot" style="background: ${status.dot}"></span>
                <span class="statusline-legend">${escapeHtml(status.legend)}</span>
              </div>
              <div class="statusline-group">
                <span>${escapeHtml(status.mode)}</span>
              </div>
            </div>
          </div>
        </div>

        ${state.paletteOpen ? renderPaletteOverlay() : ''}
      </div>
    `;

    bindEvents();
  }

  function renderSidebar(unreadCount: number): string {
    const buckets: Array<{ key: CompanyRecord['bucket']; code: string; dot: string }> = [
      { key: 'Core', code: 'CORE', dot: 'var(--emerald)' },
      { key: 'Watch', code: 'WATCH', dot: 'var(--sky)' },
      { key: 'Too Hard', code: 'TOO HARD', dot: 'var(--slate)' },
    ];

    return `
      <aside class="m3-sidebar" aria-label="M3 Sidebar Navigation">
        <div class="m3-sidebar-header">
          <div class="traffic-lights" aria-hidden="true">
            <span></span><span></span><span></span>
          </div>
          <svg
            xmlns="http://www.w3.org/2000/svg"
            viewBox="0 0 16 10"
            width="23"
            height="14"
            shape-rendering="crispEdges"
            aria-label="M3"
            role="img"
            data-m3-mark
            style="cursor: pointer"
          >
            <rect x="0" y="0" width="2" height="10" fill="#FFFFFF" />
            <rect x="6" y="0" width="2" height="10" fill="#FFFFFF" />
            <rect x="2" y="1" width="1" height="2" fill="#FFFFFF" />
            <rect x="5" y="1" width="1" height="2" fill="#FFFFFF" />
            <rect x="3" y="2" width="2" height="1" fill="#FFFFFF" />
            <rect x="3" y="3" width="1" height="2" fill="#FFFFFF" />
            <rect x="4" y="3" width="1" height="2" fill="#FFFFFF" />
            <rect x="10" y="0" width="5" height="1" fill="#FFFFFF" />
            <rect x="10" y="1" width="6" height="1" fill="#FFFFFF" />
            <rect x="14" y="2" width="2" height="2" fill="#FFFFFF" />
            <rect x="12" y="4" width="3" height="2" fill="#FFFFFF" />
            <rect x="14" y="6" width="2" height="2" fill="#FFFFFF" />
            <rect x="10" y="8" width="6" height="1" fill="#FFFFFF" />
            <rect x="10" y="9" width="5" height="1" fill="#FFFFFF" />
          </svg>
        </div>

        <div class="m3-sidebar-palette-wrap">
          <button type="button" class="m3-cmd-trigger" data-open-palette>
            <span>&gt; Jump / Command…</span>
            <span class="m3-kbd">⌘K</span>
          </button>
        </div>

        <div class="m3-sidebar-nav" role="tablist" aria-label="Workspace Sections">
          <div class="m3-rail-heading">${state.settingsLang === 'zh' ? '工作台' : 'WORKBENCH'}</div>
          ${NAV_ITEMS.map((item) => {
            const active =
              state.tab === item.tab && (item.tab !== 'baskets' || state.basketFilter === 'All');
            const label = state.settingsLang === 'zh' ? item.zhLabel : item.label;
            return `
              <button
                type="button"
                role="tab"
                aria-selected="${active ? 'true' : 'false'}"
                class="m3-nav-item ${active ? 'is-selected' : ''}"
                data-nav-tab="${item.tab}"
              >
                <span class="m3-nav-left">
                  <span class="mini-dot" style="background: ${active ? item.dotColor : 'var(--border-strong)'}"></span>
                  <span>${escapeHtml(label)}</span>
                </span>
                <span class="m3-nav-right">
                  ${
                    item.tab === 'inbox' && unreadCount > 0
                      ? `<span class="m3-unread-pill tabular">${unreadCount}</span>`
                      : ''
                  }
                  <span class="m3-kbd">${item.shortcut}</span>
                </span>
              </button>
            `;
          }).join('')}
        </div>

        <div class="m3-sidebar-baskets">
          <div class="m3-rail-heading">${state.settingsLang === 'zh' ? '能力圈' : 'CIRCLE'}</div>
          ${buckets
            .map((b) => {
              const members = COMPANIES.filter((c) => c.bucket === b.key);
              const bActive = state.tab === 'baskets' && state.basketFilter === b.key;
              return `
                <div class="m3-bucket-group">
                  <button
                    type="button"
                    class="m3-bucket-header ${bActive ? 'is-selected' : ''}"
                    data-focus-bucket="${b.key}"
                  >
                    <span class="m3-nav-left">
                      <span class="mini-dot" style="background: ${b.dot}"></span>
                      <span class="mono" style="font-size: 0.67rem; font-weight: 600">${b.code}</span>
                    </span>
                    <span class="mono tabular" style="font-size: 0.67rem; color: var(--ink-muted)">${members.length}</span>
                  </button>
                  ${members
                    .map((c) => {
                      const isSel = state.tab === 'research' && state.selectedTicker === c.ticker;
                      const dotColor =
                        c.statusTone === 'emerald'
                          ? 'var(--emerald)'
                          : c.statusTone === 'gold'
                            ? 'var(--gold)'
                            : c.statusTone === 'sky'
                              ? 'var(--sky)'
                              : c.statusTone === 'coral'
                                ? 'var(--coral)'
                                : 'var(--slate)';
                      return `
                        <button
                          type="button"
                          class="m3-ticker-item ${isSel ? 'is-selected' : ''}"
                          data-open-desk="${c.ticker}"
                        >
                          <span class="m3-nav-left">
                            <span class="mini-dot" style="background: ${dotColor}"></span>
                            <span class="mono" style="font-weight: 600">${escapeHtml(c.ticker)}</span>
                          </span>
                          <span class="mono tabular m3-ticker-price">${escapeHtml(c.spot)}</span>
                        </button>
                      `;
                    })
                    .join('')}
                </div>
              `;
            })
            .join('')}
        </div>

        ${renderSidebarDock()}
      </aside>
    `;
  }

  function renderSidebarDock(): string {
    const n = state.dockNotice;
    const dotCol =
      n?.tone === 'coral'
        ? 'var(--coral)'
        : n?.tone === 'gold'
          ? 'var(--gold)'
          : n?.tone === 'sky'
            ? 'var(--sky)'
            : 'var(--emerald)';
    const borderCol =
      n?.tone === 'coral'
        ? 'var(--coral-border)'
        : n?.tone === 'gold'
          ? 'var(--gold-border)'
          : n?.tone === 'sky'
            ? 'var(--sky-border)'
            : 'var(--emerald-border)';

    return `
      <div class="m3-sidebar-dock" aria-label="System Status & Notifications Dock">
        ${
          n
            ? `
          <div class="m3-dock-card" style="border-color: ${borderCol}">
            <div class="m3-dock-head">
              <span class="m3-nav-left">
                <span class="mini-dot" style="background: ${dotCol}"></span>
                <span class="mono" style="font-size: 0.65rem; font-weight: 700; color: ${dotCol}">${escapeHtml(n.tag)}</span>
              </span>
              ${
                n.dismissible
                  ? `<button type="button" class="m3-dock-close" data-dismiss-dock aria-label="Dismiss notice">×</button>`
                  : ''
              }
            </div>
            <div class="m3-dock-msg">${escapeHtml(n.message)}</div>
          </div>
        `
            : ''
        }

        <div style="margin-bottom: 0.4rem;">
          <button
            type="button"
            class="m3-nav-item ${state.tab === 'settings' ? 'is-selected' : ''}"
            data-nav-tab="settings"
          >
            <span class="m3-nav-left">
              <span class="mini-dot" style="background: ${state.tab === 'settings' ? 'var(--sky)' : 'var(--border-strong)'}"></span>
              <span>${state.settingsLang === 'zh' ? '设置' : 'Settings'}</span>
            </span>
            <span class="m3-kbd">⌘,</span>
          </button>
        </div>

        <div class="m3-dock-tray">
          <div class="m3-dock-tray-row">
            <span class="m3-nav-left">
              <span class="mini-dot" style="background: var(--emerald)"></span>
              <span class="mono">L0 5,142 Co</span>
            </span>
            <span class="mono" style="color: var(--emerald)">\$0.42 / \$10.00</span>
          </div>
          <div class="m3-dock-tray-row" style="color: var(--ink-muted)">
            <span class="mono">v0.4.2</span>
            <span class="mono">${escapeHtml(state.settingsChannel)}</span>
          </div>
        </div>
      </div>
    `.replaceAll('\\$', '$');
  }

  function renderInboxView(): string {
    const unreadCount = Object.values(state.eventStatuses).filter((s) => s === 'unread').length;
    const filtered = INBOX_EVENTS.filter((ev) => {
      const st = state.eventStatuses[ev.id];
      switch (state.inboxFilter) {
        case 'unread':
          return st === 'unread';
        case 'all':
          return true;
        case 'discovery':
          return ev.category === 'discovery' && st !== 'archived';
        case 'ripple':
          return ev.category === 'ripple' && st !== 'archived';
        case 'strike':
          return (ev.category === 'strike' || ev.category === 'red_flag') && st !== 'archived';
        case 'archived':
          return st === 'archived';
      }
    });

    const selected = filtered.find((e) => e.id === state.selectedEventId) || filtered[0] || INBOX_EVENTS[0];
    const selectedSt = state.eventStatuses[selected.id];

    const filters: Array<{ key: InboxFilter; label: string }> = [
      { key: 'unread', label: 'Unread' },
      { key: 'all', label: 'All' },
      { key: 'discovery', label: 'Discovery' },
      { key: 'ripple', label: 'Latent Ripple' },
      { key: 'strike', label: 'Strike & Red Flag' },
      { key: 'archived', label: 'Archived' },
    ];

    return `
      <div class="m3-kpi-ribbon">
        <div class="m3-kpi-items">
          <div class="m3-kpi"><span>US Universe</span><strong class="tabular">5,142</strong></div>
          <span class="v-div"></span>
          <div class="m3-kpi"><span>Last Catch-up</span><strong>14m ago</strong></div>
          <span class="v-div"></span>
          <div class="m3-kpi"><span>Filing Chunks</span><strong style="color: var(--sky)" class="tabular">18,420</strong></div>
          <span class="v-div"></span>
          <div class="m3-kpi"><span>Unread</span><strong style="color: var(--emerald)" class="tabular">${unreadCount}</strong></div>
        </div>
        <div class="m3-kpi-badges">
          <button type="button" class="action-btn action-btn--primary" data-trigger-sync>Catch-up</button>
        </div>
      </div>

      <div class="view-toolbar">
        <div class="chip-group" role="group" aria-label="Inbox Filter">
          ${filters
            .map(
              (f) => `
            <button
              type="button"
              class="filter-chip ${state.inboxFilter === f.key ? 'is-active' : ''}"
              data-inbox-filter="${f.key}"
            >
              ${escapeHtml(f.label)}
            </button>
          `,
            )
            .join('')}
        </div>
        <div class="view-summary tabular">${filtered.length} briefings in queue</div>
      </div>

      <div class="split-pane">
        <div class="pane-left" role="listbox" aria-label="Briefing Queue">
          ${
            filtered.length === 0
              ? `<div style="padding: 2rem 1.25rem; color: var(--ink-muted); font-size: 0.85rem;">Queue clean. Switch to "All" or reset filters to inspect previous briefings.</div>`
              : filtered
                  .map((ev) => {
                    const isSel = ev.id === selected.id;
                    const st = state.eventStatuses[ev.id];
                    return `
                  <button
                    type="button"
                    role="option"
                    aria-selected="${isSel ? 'true' : 'false'}"
                    class="event-row ${isSel ? 'is-selected' : ''} ${st !== 'unread' ? 'is-muted' : ''}"
                    data-select-event="${ev.id}"
                  >
                    <span class="severity-dot dot-${ev.severity}" aria-hidden="true"></span>
                    <div>
                      <div class="event-title-line">
                        <span class="ticker-code">${escapeHtml(ev.ticker)}</span>
                        <span class="status-pill ${toneClass(ev.severity)}">${escapeHtml(ev.kindLabel)}</span>
                        ${st !== 'unread' ? `<span class="event-kind">${escapeHtml(st)}</span>` : ''}
                      </div>
                      <p class="event-headline">${escapeHtml(ev.headline)}</p>
                      <p class="event-sub">${escapeHtml(ev.subline)}</p>
                    </div>
                    <span class="event-time">${escapeHtml(ev.timestamp)}</span>
                  </button>
                `;
                  })
                  .join('')
          }
        </div>

        <div class="pane-right">
          <div class="inspector-head">
            <div>
              <div class="event-title-line">
                <span class="ticker-code">${escapeHtml(selected.ticker)}</span>
                <span style="color: var(--ink-secondary); font-size: 0.84rem;">${escapeHtml(selected.company)}</span>
                <span class="status-pill ${toneClass(selected.severity)}">${escapeHtml(selected.kindLabel)}</span>
                <span class="status-pill ${toneClass(selected.circleStatusTone)}">${escapeHtml(selected.circleStatusLabel)}</span>
              </div>
              <h3 class="inspector-title">${escapeHtml(selected.headline)}</h3>
            </div>
            <span class="event-time">${escapeHtml(selected.timestamp)}</span>
          </div>

          <div class="evidence-box">
            <div class="evidence-top">
              <span class="kv-label">SEC 10-K Company Profile &amp; Circle Context (0 LLM)</span>
              <span class="mono" style="font-size: 0.68rem; color: var(--sky);">Item 1 / Item 1A</span>
            </div>
            <div class="evidence-quote">${escapeHtml(selected.businessProfile10k)}</div>
          </div>

          <div class="kv-grid">
            ${selected.metrics
              .map(
                (m) => `
              <div class="kv-card">
                <span class="kv-label">${escapeHtml(m.label)}</span>
                <span class="kv-value tabular">${escapeHtml(m.value)}</span>
                <span class="kv-sub">${escapeHtml(m.note)}</span>
              </div>
            `,
              )
              .join('')}
          </div>

          <!-- Uniform 1px hairline evidence excerpt well (zero thick colored left bar) -->
          <div class="evidence-box">
            <div class="evidence-top">
              <span class="event-kind">${escapeHtml(selected.evidenceTitle)}</span>
              <span class="mono" style="font-size: 0.7rem; color: var(--emerald);">Match ${escapeHtml(selected.similarityPct)}</span>
            </div>
            <div class="evidence-quote">${escapeHtml(selected.evidenceQuote)}</div>
            <div class="evidence-url">${escapeHtml(selected.sourceUrl)}</div>
          </div>

          <div class="inspector-actions">
            <button type="button" class="action-btn action-btn--primary" data-open-desk="${selected.ticker}">
              Inspect ${escapeHtml(selected.ticker)} →
            </button>
            <button type="button" class="action-btn" style="border-color: var(--sky-border); color: var(--sky);" data-open-industry="${escapeHtml(selected.industrySlug || INDUSTRIES[0].slug)}">
              Research Industry (⌘3)
            </button>
            <button type="button" class="action-btn" data-triage-event="${selected.id}" data-triage-action="watch">
              ${selectedSt === 'watch' ? '✓ Watch' : 'w Watch'}
            </button>
            <button type="button" class="action-btn" data-triage-event="${selected.id}" data-triage-action="too_hard">
              ${selectedSt === 'too_hard' ? '✓ Too Hard' : 'x Too Hard'}
            </button>
            <button type="button" class="action-btn" data-triage-event="${selected.id}" data-triage-action="archived">
              ${selectedSt === 'archived' ? '✓ Archived' : 'e Archive'}
            </button>
          </div>
        </div>
      </div>
    `.replaceAll('\\$', '$');
  }

  function renderUniverseView(): string {
    const presets: Array<{ key: UniverseFilter; label: string }> = [
      { key: 'all', label: 'All Quality' },
      { key: 'high_roic', label: 'High ROIC (>=20%)' },
      { key: 'low_sbc', label: 'Low SBC Cash Cows' },
      { key: 'strike', label: 'Strike Zone' },
    ];

    const filtered = COMPANIES.filter((c) => {
      if (state.universeFilter === 'all') return true;
      if (state.universeFilter === 'high_roic') return Number.parseFloat(c.roic) >= 20;
      if (state.universeFilter === 'low_sbc') return Number.parseFloat(c.sbcToOcf) <= 15;
      return c.statusLabel === 'Near Strike';
    });
    const selected = filtered.find((c) => c.ticker === state.selectedTicker) || filtered[0] || COMPANIES[0];

    return `
      <div class="view-toolbar">
        <div class="chip-group" role="group" aria-label="Universe Munger Presets">
          ${presets
            .map(
              (p) => `
            <button
              type="button"
              class="filter-chip ${state.universeFilter === p.key ? 'is-active' : ''}"
              data-universe-preset="${p.key}"
            >
              ${escapeHtml(p.label)}
            </button>
          `,
            )
            .join('')}
        </div>
        <div class="chip-group">
          <button
            type="button"
            class="filter-chip ${state.universeSearchMode === 'keyword' ? 'is-active' : ''}"
            data-search-mode="keyword"
          >
            Keyword
          </button>
          <button
            type="button"
            class="filter-chip ${state.universeSearchMode === 'similar' ? 'is-active' : ''}"
            data-search-mode="similar"
          >
            Lookalike Search
          </button>
        </div>
      </div>

      <div class="table-wrap">
        <table class="m3-table">
          <thead>
            <tr>
              <th>Ticker</th>
              <th>Company</th>
              <th>Bucket</th>
              <th>Spot / Strike</th>
              <th>5Y ROIC</th>
              <th>Strict OE</th>
              <th>SBC / OCF</th>
              <th>Share Δ</th>
              <th>Cosine</th>
              <th>Status</th>
            </tr>
          </thead>
          <tbody>
            ${filtered
              .map(
                (c) => `
              <tr data-select-company="${c.ticker}" class="${c.ticker === selected.ticker ? 'is-selected' : ''}">
                <td><span class="ticker-code">${escapeHtml(c.ticker)}</span></td>
                <td>${escapeHtml(c.name)}</td>
                <td class="mono" style="font-size: 0.74rem; color: var(--ink-secondary);">${escapeHtml(c.bucket)}</td>
                <td class="mono tabular">${escapeHtml(c.spot)} <span style="color: var(--ink-muted)">/ ${escapeHtml(c.strike)}</span></td>
                <td class="mono tabular" style="color: var(--emerald);">${escapeHtml(c.roic)}</td>
                <td class="mono tabular">${escapeHtml(c.ownerEarnings)}</td>
                <td class="mono tabular">${escapeHtml(c.sbcToOcf)}</td>
                <td class="mono tabular">${escapeHtml(c.shareDelta)}</td>
                <td class="mono tabular" style="color: var(--violet);">${escapeHtml(c.similarityScore)}</td>
                <td><span class="status-pill ${toneClass(c.statusTone)}">${escapeHtml(c.statusLabel)}</span></td>
              </tr>
            `,
              )
              .join('')}
          </tbody>
        </table>
      </div>

      <div class="universe-drawer">
        <div>
          <span class="ticker-code">${escapeHtml(selected.ticker)}</span>
          <span style="margin-left: 0.55rem; font-size: 0.83rem; color: var(--ink-secondary);">
            ${escapeHtml(selected.lookalikeNote)} · ${escapeHtml(selected.lastFiling)}
          </span>
        </div>
        <div class="chip-group">
          <button type="button" class="action-btn" style="border-color: var(--sky-border); color: var(--sky);" data-open-industry="${escapeHtml(INDUSTRIES[0].slug)}">
            Research Industry (⌘3)
          </button>
          <button type="button" class="action-btn action-btn--primary" data-open-desk="${selected.ticker}">
            Open Research Desk →
          </button>
        </div>
      </div>
    `;
  }

  function renderBasketsView(): string {
    const visibleBuckets: Array<CompanyRecord['bucket']> =
      state.basketFilter === 'All' ? ['Core', 'Watch', 'Too Hard'] : [state.basketFilter];
    const tabs: Array<CompanyRecord['bucket'] | 'All'> = ['All', 'Core', 'Watch', 'Too Hard'];

    return `
      <div class="view-toolbar">
        <div class="chip-group" role="group" aria-label="Filter Circle">
          ${tabs
            .map(
              (b) => `
            <button
              type="button"
              class="filter-chip ${state.basketFilter === b ? 'is-active' : ''}"
              data-focus-bucket="${b}"
            >
              ${escapeHtml(b)}
            </button>
          `,
            )
            .join('')}
        </div>
        <div class="view-summary">Core / Watch / Too Hard</div>
      </div>

      <div class="baskets-scroll">
        ${visibleBuckets
          .map((bucket) => {
            const list = COMPANIES.filter((c) => c.bucket === bucket);
            return `
              <div class="basket-block">
                <div class="basket-block-head">
                  <span class="mono" style="font-size: 0.75rem; font-weight: 700; color: var(--ink-hero);">${bucket.toUpperCase()}</span>
                  <span class="mono" style="font-size: 0.72rem; color: var(--ink-muted);">${list.length} holdings</span>
                </div>
                ${list
                  .map(
                    (c) => `
                  <button
                    type="button"
                    class="basket-row ${state.selectedTicker === c.ticker ? 'is-selected' : ''}"
                    data-open-desk="${c.ticker}"
                  >
                    <div>
                      <span class="ticker-code">${escapeHtml(c.ticker)}</span>
                      <span style="margin-left: 0.55rem; color: var(--ink-primary); font-size: 0.85rem;">${escapeHtml(c.name)}</span>
                      <span style="margin-left: 0.55rem; font-size: 0.75rem; color: var(--ink-muted);">${escapeHtml(c.sector)}</span>
                    </div>
                    <div class="basket-row-metrics">
                      <span class="mono tabular" style="color: var(--ink-secondary);">ROIC ${escapeHtml(c.roic)}</span>
                      <span class="mono tabular">${escapeHtml(c.spot)} / ${escapeHtml(c.strike)}</span>
                      <span class="status-pill ${toneClass(c.statusTone)}">${escapeHtml(c.statusLabel)}</span>
                    </div>
                  </button>
                `,
                  )
                  .join('')}
              </div>
            `;
          })
          .join('')}
      </div>
    `;
  }

  function renderResearchView(): string {
    const company = COMPANIES.find((c) => c.ticker === state.selectedTicker) || COMPANIES[0];
    const activePerspective = company.perspectives[state.perspective];

    return `
      <div class="company-subheader">
        <div class="company-sub-left">
          <span class="ticker-code" style="font-size: 1rem;">${escapeHtml(company.ticker)}</span>
          <span style="font-size: 0.88rem; color: var(--ink-secondary);">${escapeHtml(company.name)}</span>
          <span class="status-pill ${toneClass(company.statusTone)}">${escapeHtml(company.statusLabel)}</span>
          <span class="v-div"></span>
          <div class="seg-group" role="tablist" aria-label="Research Sub-Views">
            <button
              type="button"
              role="tab"
              aria-selected="${state.companySubTab === 'memo' ? 'true' : 'false'}"
              class="seg-tab ${state.companySubTab === 'memo' ? 'is-active' : ''}"
              data-subtab="memo"
            >
              1 Financials &amp; Living Memo
            </button>
            <button
              type="button"
              role="tab"
              aria-selected="${state.companySubTab === 'alan' ? 'true' : 'false'}"
              class="seg-tab seg-tab--alan ${state.companySubTab === 'alan' ? 'is-active' : ''}"
              data-subtab="alan"
            >
              2 Deep Research &amp; Alan
            </button>
            <button
              type="button"
              role="tab"
              aria-selected="${state.companySubTab === 'xbrl' ? 'true' : 'false'}"
              class="seg-tab ${state.companySubTab === 'xbrl' ? 'is-active' : ''}"
              data-subtab="xbrl"
            >
              3 XBRL Inspector
            </button>
          </div>
        </div>

        <div class="view-summary mono">${escapeHtml(company.lastFiling)}</div>
      </div>

      ${
        state.companySubTab === 'xbrl'
          ? `
        <div style="padding: 1rem 1.25rem; display: flex; flex-direction: column; gap: 0.65rem; flex: 1;">
          <div style="display: flex; align-items: center; justify-content: space-between;">
            <span class="kv-label">Primary SEC EDGAR XBRL Fact Audit Trail</span>
            <span class="status-pill pill-emerald">Zero-LLM Deterministic Decimal</span>
          </div>
          ${company.metrics
            .map(
              (m) => `
            <div class="evidence-box">
              <div class="evidence-top">
                <strong style="color: var(--ink-hero); font-size: 0.84rem;">${escapeHtml(m.label)}</strong>
                <span class="mono tabular" style="color: var(--emerald); font-weight: 600;">${escapeHtml(m.value)}</span>
              </div>
              <div style="font-size: 0.82rem; color: var(--ink-secondary);">${escapeHtml(m.formula)}</div>
              <div class="evidence-url">${escapeHtml(m.source)} · ${escapeHtml(m.sha256)}</div>
            </div>
          `,
            )
            .join('')}
        </div>
      `
          : `
        <div class="split-pane">
          <div class="pane-left" style="padding: 1rem; display: flex; flex-direction: column; gap: 0.65rem;">
            <div style="display: flex; align-items: center; justify-content: space-between;">
              <span class="kv-label">Deterministic Arithmetic</span>
              <span class="status-pill pill-emerald">Click row for SHA-256</span>
            </div>

            ${company.metrics
              .map((m) => {
                const isOpen = state.openProvenanceId === m.id;
                return `
                  <button
                    type="button"
                    class="provenance-card ${isOpen ? 'is-open' : ''}"
                    data-toggle-provenance="${m.id}"
                  >
                    <div class="provenance-row">
                      <span class="kv-label">${escapeHtml(m.label)}</span>
                      <span class="mono tabular" style="font-size: 0.94rem; font-weight: 600; color: var(--ink-hero);">
                        ${escapeHtml(m.value)}
                      </span>
                    </div>
                    <div style="font-size: 0.78rem; color: var(--ink-secondary); margin-top: 0.25rem;">
                      ${escapeHtml(m.formula)}
                    </div>
                    ${
                      isOpen
                        ? `<div class="provenance-hash">${escapeHtml(m.source)}<br/>${escapeHtml(m.sha256)}</div>`
                        : ''
                    }
                  </button>
                `;
              })
              .join('')}
          </div>

          <div class="pane-right">
            ${
              state.companySubTab === 'memo'
                ? `
              <div style="display: flex; align-items: center; justify-content: space-between;">
                <span class="kv-label">Living Research Memo</span>
                <span class="status-pill pill-sky">workbench/memos/${escapeHtml(company.ticker)}.md</span>
              </div>
              <div class="evidence-box">
                <ul class="pipeline-bullets">
                  ${company.livingMemoSummary.map((line) => `<li>${escapeHtml(line)}</li>`).join('')}
                </ul>
              </div>
            `
                : `
              <div>
                <div style="display: flex; align-items: center; justify-content: space-between; gap: 0.5rem; margin-bottom: 0.6rem;">
                  <span class="kv-label">EmbeddedAlan 4-Perspective Interrogation</span>
                  <span class="status-pill pill-violet">Read-Only Tools</span>
                </div>

                <div class="perspective-tabs" role="tablist" aria-label="4 Deep Research Perspectives">
                  ${PERSPECTIVE_LABELS.map(
                    (p) => `
                    <button
                      type="button"
                      role="tab"
                      aria-selected="${state.perspective === p.key ? 'true' : 'false'}"
                      class="perspective-btn ${state.perspective === p.key ? 'is-active' : ''}"
                      data-perspective="${p.key}"
                    >
                      ${p.label}
                    </button>
                  `,
                  ).join('')}
                </div>
              </div>

              <div class="evidence-box">
                <div style="display: flex; flex-wrap: wrap; align-items: center; justify-content: space-between; gap: 0.5rem;">
                  <strong style="color: var(--ink-hero); font-size: 0.88rem;">${escapeHtml(activePerspective.title)}</strong>
                  <div class="chip-group">
                    ${activePerspective.tools.map((t) => `<span class="event-kind">${escapeHtml(t)}</span>`).join('')}
                  </div>
                </div>
                <ul class="pipeline-bullets" style="margin-top: 0.35rem;">
                  ${activePerspective.points.map((pt) => `<li>${escapeHtml(pt)}</li>`).join('')}
                </ul>
              </div>
            `
            }

            <div class="sovereignty-box">
              <div class="sovereignty-header">
                <span>human_judgment · Preserved Across Re-Runs</span>
                <span>${escapeHtml(company.humanJudgment.convictionWeight)}</span>
              </div>
              <p class="sovereignty-body">${escapeHtml(company.humanJudgment.ownerNote)}</p>
              <p style="margin: 0.45rem 0 0; font-family: var(--font-mono); font-size: 0.72rem; color: var(--ink-secondary);">
                Falsification Trigger: ${escapeHtml(company.humanJudgment.falsification)}
              </p>
            </div>
          </div>
        </div>
      `
      }
    `;
  }

  function renderIndustryView(): string {
    const ind = INDUSTRIES.find((i) => i.slug === state.selectedIndustrySlug) || INDUSTRIES[0];
    const lens = ind.perspectives[state.industryPerspective];
    const stPill =
      ind.status === 'inside_circle'
        ? 'pill-emerald'
        : ind.status === 'too_hard'
          ? 'pill-coral'
          : 'pill-sky';

    return `
      <div class="view-toolbar">
        <div class="chip-group" role="tablist" aria-label="Industry Studies">
          ${INDUSTRIES.map(
            (item) => `
            <button
              type="button"
              class="filter-chip ${item.slug === ind.slug ? 'is-active' : ''}"
              data-select-industry="${escapeHtml(item.slug)}"
            >
              ${escapeHtml(item.title)} (${escapeHtml(item.statusLabel)})
            </button>
          `,
          ).join('')}
        </div>
        <div class="view-summary" style="display: inline-flex; align-items: center; gap: 0.4rem;">
          <span>Industry Research</span>
          ${conceptHint('Graduate an industry to Inside Circle after Feynman verification to unlock Strike Zone signals.')}
        </div>
      </div>

      <div class="pane-right" style="padding: 1rem 1.15rem; gap: 0.85rem; overflow-y: auto; flex: 1;">
        <div class="inspector-head">
          <div>
            <div class="event-title-line">
              <span class="status-pill ${stPill}">${escapeHtml(ind.statusLabel)}</span>
              <span class="ticker-code" style="font-size: 0.96rem;">${escapeHtml(ind.title)}</span>
              <span class="event-kind">${escapeHtml(ind.sicLabel)}</span>
            </div>
            <div style="margin-top: 0.35rem; font-size: 0.78rem; color: var(--ink-secondary);">
              Human Valuation Anchor: <strong class="mono" style="color: var(--emerald);">${escapeHtml(ind.anchorLabel)}</strong>
              · Bound Peers: <span class="mono">${escapeHtml(ind.boundTickers.join(', '))}</span>
            </div>
          </div>
          <span class="status-pill pill-violet">Embedded Alan 4-Lens Ready</span>
        </div>

        <div class="perspective-tabs" role="tablist" aria-label="Industry 4 Perspectives">
          ${IND_PERSPECTIVE_LABELS.map(
            (p) => `
            <button
              type="button"
              role="tab"
              aria-selected="${state.industryPerspective === p.key ? 'true' : 'false'}"
              class="perspective-btn ${state.industryPerspective === p.key ? 'is-active' : ''}"
              data-ind-perspective="${p.key}"
            >
              ${escapeHtml(p.label)}
            </button>
          `,
          ).join('')}
        </div>

        <div class="evidence-box">
          <div class="evidence-top">
            <strong style="color: var(--ink-hero); font-size: 0.86rem;">${escapeHtml(lens.title)}</strong>
            <span class="status-pill pill-sky">SEC 10-K vs Web Separated</span>
          </div>
          <p style="margin: 0; font-size: 0.82rem; color: var(--ink-secondary);">${escapeHtml(lens.summary)}</p>
          <ul class="pipeline-bullets">
            ${lens.points.map((pt) => `<li>${escapeHtml(pt)}</li>`).join('')}
          </ul>
          <div style="display: grid; grid-template-columns: repeat(2, minmax(0, 1fr)); gap: 0.55rem; margin-top: 0.25rem;">
            <div class="kv-card">
              <span class="kv-label" style="color: var(--emerald);">Primary SEC 10-K Citations</span>
              ${ind.secCitations.map((c) => `<div class="kv-sub">${escapeHtml(c)}</div>`).join('')}
            </div>
            <div class="kv-card">
              <span class="kv-label" style="color: var(--sky);">External Web Search Citations</span>
              ${ind.webCitations
                .map((w) => `<div class="kv-sub">[Web] ${escapeHtml(w.title)}</div>`)
                .join('')}
            </div>
          </div>
        </div>

        <div class="table-wrap" style="border: 1px solid var(--border-hairline); border-radius: var(--radius-sm);">
          <table class="m3-table">
            <thead>
              <tr>
                <th>Peer</th>
                <th>5Y ROIC</th>
                <th>5Y GM</th>
                <th>FCF%</th>
                <th>SBC/OCF</th>
                <th>Spot</th>
                <th>Industry Anchor Ref</th>
              </tr>
            </thead>
            <tbody>
              ${ind.peers
                .map(
                  (peer) => `
                <tr>
                  <td><span class="ticker-code">${escapeHtml(peer.ticker)}</span> <span style="color: var(--ink-muted); font-size: 0.74rem;">${escapeHtml(peer.name)}</span></td>
                  <td class="mono tabular" style="color: var(--emerald);">${escapeHtml(peer.roic5y)}</td>
                  <td class="mono tabular">${escapeHtml(peer.gm5y)}</td>
                  <td class="mono tabular">${escapeHtml(peer.fcfYield)}</td>
                  <td class="mono tabular">${escapeHtml(peer.sbcOcf)}</td>
                  <td class="mono tabular">${escapeHtml(peer.price)}</td>
                  <td class="mono tabular" style="color: var(--sky);">${escapeHtml(peer.refStrike)}</td>
                </tr>
              `,
                )
                .join('')}
            </tbody>
          </table>
        </div>

        <div class="sovereignty-box">
          <div class="sovereignty-header">
            <span>Feynman Mastery Check &amp; Human Notes</span>
            <span>workbench/industries/${escapeHtml(ind.slug)}.md</span>
          </div>
          <div style="display: flex; flex-direction: column; gap: 0.35rem; margin-bottom: 0.5rem;">
            ${ind.feynmanChecklist
              .map((q, idx) => {
                const key = `${ind.slug}:${idx}`;
                const checked = state.feynmanChecked[key] ?? q.checked;
                return `
                  <button
                    type="button"
                    style="text-align: left; display: flex; gap: 0.45rem; font-size: 0.78rem; color: var(--ink-primary);"
                    data-toggle-feynman="${escapeHtml(key)}"
                  >
                    <span class="mono" style="color: ${checked ? 'var(--emerald)' : 'var(--ink-muted)'};">${checked ? '[✓]' : '[ ]'}</span>
                    <span>${escapeHtml(q.text)}</span>
                  </button>
                `;
              })
              .join('')}
          </div>
          <p class="sovereignty-body">${escapeHtml(ind.humanNotes)}</p>
        </div>
      </div>
    `;
  }

  function renderJournalView(): string {
    return `
      <div class="view-toolbar">
        <div class="view-summary" style="display: inline-flex; align-items: center; gap: 0.4rem;">
          <span>Decision Journal</span>
          ${conceptHint('Append-only pre-committed ledger benchmarked against 100% Paper Cash & S&P 500.')}
        </div>
        <div class="view-summary tabular">${JOURNAL_ENTRIES.length} entries</div>
      </div>

      <div class="journal-list" style="flex: 1; overflow-y: auto;">
        ${JOURNAL_ENTRIES.map(
          (j) => `
          <div class="journal-card">
            <div style="display: flex; align-items: center; justify-content: space-between; gap: 0.5rem; margin-bottom: 0.35rem;">
              <span class="m3-nav-left">
                <span class="mini-dot" style="background: ${
                  j.tone === 'strike'
                    ? 'var(--emerald)'
                    : j.tone === 'toohard'
                      ? 'var(--coral)'
                      : 'var(--sky)'
                }"></span>
                <span class="ticker-code">${escapeHtml(j.ticker)}</span>
                <span class="event-kind">${escapeHtml(j.action)}</span>
              </span>
              <span class="mono" style="font-size: 0.7rem; color: var(--ink-muted);">${escapeHtml(j.date)}</span>
            </div>
            <p style="margin: 0; font-size: 0.85rem; color: var(--ink-primary); line-height: 1.5;">${escapeHtml(j.note)}</p>
          </div>
        `,
        ).join('')}
      </div>
    `;
  }

  function renderSettingsView(): string {
    const zh = state.settingsLang === 'zh';
    const budgetTip = zh
      ? 'L0/L1 全程零 LLM；仅在手动启动 Embedded Alan 深研或行业研究时计费，到达月度上限自动拦截。'
      : 'L0/L1 spend 0 LLM tokens. Hard ceilings block Embedded Alan L2 runs before spending.';
    return `
      <div class="view-toolbar">
        <div class="view-summary">${zh ? '偏好设置' : 'Settings'}</div>
      </div>
      <div class="pane-right" style="padding: 1.25rem; gap: 0.75rem; overflow-y: auto; flex: 1;">
        <div class="evidence-box">
          <div style="display: flex; align-items: center; justify-content: space-between; gap: 0.75rem;">
            <strong style="color: var(--ink-hero); font-size: 0.86rem;">${zh ? '显示语言' : 'Display Language'}</strong>
            <div class="seg-group">
              <button type="button" class="seg-tab ${state.settingsLang === 'zh' ? 'is-active' : ''}" data-set-lang="zh">中文</button>
              <button type="button" class="seg-tab ${state.settingsLang === 'en' ? 'is-active' : ''}" data-set-lang="en">English</button>
            </div>
          </div>
        </div>

        <div class="evidence-box">
          <div style="display: flex; align-items: center; justify-content: space-between; gap: 0.75rem;">
            <strong style="color: var(--ink-hero); font-size: 0.86rem;">${zh ? '更新通道' : 'Release Channel'}</strong>
            <div class="seg-group">
              <button type="button" class="seg-tab ${state.settingsChannel === 'stable' ? 'is-active' : ''}" data-set-channel="stable">stable</button>
              <button type="button" class="seg-tab ${state.settingsChannel === 'beta' ? 'is-active' : ''}" data-set-channel="beta">beta</button>
            </div>
          </div>
        </div>

        <div class="evidence-box">
          <div style="display: flex; align-items: center; justify-content: space-between; gap: 0.75rem;">
            <span style="display: inline-flex; align-items: center; gap: 0.45rem;">
              <strong style="color: var(--ink-hero); font-size: 0.86rem;">${zh ? 'L2 AI 预算护栏' : 'L2 AI Budget'}</strong>
              ${conceptHint(budgetTip)}
            </span>
            <span class="mono" style="font-size: 0.72rem; color: var(--emerald);">\$10.00 / MO HARD CAP</span>
          </div>
        </div>

        <div class="evidence-box">
          <div style="display: flex; align-items: center; justify-content: space-between; gap: 0.75rem; margin-bottom: 0.6rem;">
            <strong style="color: var(--ink-hero); font-size: 0.86rem;">${zh ? '本地存储' : 'Storage'}</strong>
            <span class="mono" style="font-size: 0.75rem; color: var(--ink-hero);">3.52 GB <span style="color: var(--ink-muted);">${zh ? '已占用' : 'used'}</span></span>
          </div>
          <div class="storage-bar" role="img" aria-label="Storage breakdown">
            <span class="storage-seg" style="flex: 2810; background: var(--sky);"></span>
            <span class="storage-seg" style="flex: 658; background: var(--violet);"></span>
            <span class="storage-seg" style="flex: 68; background: var(--emerald);"></span>
            <span class="storage-seg" style="flex: 29; background: var(--gold);"></span>
            <span class="storage-seg" style="flex: 6; background: var(--ink-secondary);"></span>
          </div>
          <div class="storage-legend" style="margin-top: 0.6rem;">
            <span class="storage-legend-item"><span class="mini-dot" style="background: var(--sky)"></span>SEC Bulk 2.81 GB</span>
            <span class="storage-legend-item"><span class="mini-dot" style="background: var(--violet)"></span>10-K Filings 658 MB</span>
            <span class="storage-legend-item"><span class="mini-dot" style="background: var(--emerald)"></span>DuckDB 68 MB</span>
            <span class="storage-legend-item"><span class="mini-dot" style="background: var(--gold)"></span>Quotes 29 MB</span>
            <span class="storage-legend-item"><span class="mini-dot" style="background: var(--ink-secondary)"></span>Workbench 44 KB</span>
          </div>
        </div>
      </div>
    `.replaceAll('\\$', '$');
  }

  function renderPaletteOverlay(): string {
    const q = state.paletteQuery.trim().toLowerCase();
    const actions: Array<{ label: string; group: string; key: string; cmd: string }> = [
      { label: 'Briefing Inbox (Event-Driven L1)', group: 'VIEW', key: '⌘1', cmd: 'view:inbox' },
      { label: 'Universe Radar (5,142 US Issuers)', group: 'VIEW', key: '⌘2', cmd: 'view:universe' },
      { label: 'Industry Research Studio (Alan + Web)', group: 'VIEW', key: '⌘3', cmd: 'view:industry' },
      { label: 'Circle Overview', group: 'VIEW', key: '⌘4', cmd: 'view:baskets' },
      { label: 'Decision Journal', group: 'VIEW', key: '⌘5', cmd: 'view:journal' },
      { label: 'Settings (Bilingual Zh/En & Updates)', group: 'VIEW', key: '⌘,', cmd: 'view:settings' },
      { label: 'Trigger L1 / Core Sync', group: 'ACTION', key: '⌘R', cmd: 'action:sync' },
      ...COMPANIES.map((c) => ({
        label: `Open ${c.ticker} · ${c.name} (${c.bucket})`,
        group: 'DOSSIER',
        key: c.spot,
        cmd: `ticker:${c.ticker}`,
      })),
    ].filter((it) => !q || it.label.toLowerCase().includes(q) || it.group.toLowerCase().includes(q));

    return `
      <div class="m3-palette-scrim" data-close-palette>
        <div class="m3-palette-modal" role="dialog" aria-label="M3 Command Palette">
          <div class="m3-palette-input-row">
            <span class="mono" style="color: var(--emerald);">&gt;</span>
            <input
              id="m3-palette-input"
              type="text"
              class="m3-palette-input"
              placeholder="Jump to view, ticker, or run command…"
              value="${escapeHtml(state.paletteQuery)}"
            />
            <button type="button" class="m3-kbd" data-close-palette>ESC</button>
          </div>
          <div class="m3-palette-list">
            ${actions
              .map(
                (a) => `
              <button type="button" class="m3-palette-row" data-palette-cmd="${escapeHtml(a.cmd)}">
                <span>
                  <span class="mono" style="font-size: 0.65rem; color: var(--ink-muted); margin-right: 0.5rem;">${escapeHtml(a.group)}</span>
                  <span>${escapeHtml(a.label)}</span>
                </span>
                <span class="m3-kbd">${escapeHtml(a.key)}</span>
              </button>
            `,
              )
              .join('')}
          </div>
        </div>
      </div>
    `;
  }

  function bindEvents(): void {
    container.querySelectorAll<HTMLElement>('[data-nav-tab]').forEach((btn) => {
      btn.addEventListener('click', () => {
        const next = btn.dataset.navTab as WorkbenchTab | undefined;
        if (!next) return;
        state.tab = next;
        if (next === 'baskets') state.basketFilter = 'All';
        render();
      });
    });

    container.querySelectorAll<HTMLElement>('[data-focus-bucket]').forEach((btn) => {
      btn.addEventListener('click', () => {
        const bucket = btn.dataset.focusBucket as CompanyRecord['bucket'] | 'All' | undefined;
        if (!bucket) return;
        state.basketFilter = bucket;
        state.tab = 'baskets';
        render();
      });
    });

    container.querySelectorAll<HTMLElement>('[data-open-desk]').forEach((btn) => {
      btn.addEventListener('click', () => {
        const ticker = btn.dataset.openDesk;
        if (!ticker) return;
        state.selectedTicker = ticker;
        state.tab = 'research';
        state.openProvenanceId =
          COMPANIES.find((c) => c.ticker === ticker)?.metrics[0]?.id ?? null;
        render();
      });
    });

    container.querySelector('[data-back-baskets]')?.addEventListener('click', () => {
      state.tab = 'baskets';
      render();
    });

    container.querySelectorAll<HTMLElement>('[data-trigger-sync]').forEach((btn) => {
      btn.addEventListener('click', () => {
        state.dockNotice = {
          tag: 'SYNCED',
          message: 'SEC XBRL facts & Tier-1 quotes verified across 5,142 US issuers. 0 LLM tokens spent.',
          tone: 'emerald',
          dismissible: true,
        };
        render();
      });
    });

    container.querySelector('[data-dismiss-dock]')?.addEventListener('click', () => {
      state.dockNotice = null;
      render();
    });

    container.querySelector('[data-m3-mark]')?.addEventListener('click', () => {
      const now = performance.now();
      markClicks = now - markLastClickMs <= 1400 ? markClicks + 1 : 1;
      markLastClickMs = now;
      if (markClicks >= 3) {
        markClicks = 0;
        const zh = state.settingsLang === 'zh';
        state.dockNotice = {
          tag: 'M³',
          message: zh
            ? 'M³ = Money · Making · Machine — 机器读完 6,000 家财报，你只挥击甜区好球。'
            : 'M³ = Money · Making · Machine — The machine reads 6,000 filings; you swing at fat pitches.',
          tone: 'emerald',
          dismissible: true,
        };
        render();
      }
    });

    container.querySelectorAll<HTMLElement>('[data-inbox-filter]').forEach((btn) => {
      btn.addEventListener('click', () => {
        const f = btn.dataset.inboxFilter as InboxFilter | undefined;
        if (f) {
          state.inboxFilter = f;
          render();
        }
      });
    });

    container.querySelectorAll<HTMLElement>('[data-select-event]').forEach((btn) => {
      btn.addEventListener('click', () => {
        const id = btn.dataset.selectEvent;
        if (id) {
          state.selectedEventId = id;
          render();
        }
      });
    });

    container.querySelectorAll<HTMLElement>('[data-triage-event]').forEach((btn) => {
      btn.addEventListener('click', () => {
        const id = btn.dataset.triageEvent;
        const action = btn.dataset.triageAction as InboxEvent['status'] | undefined;
        if (!id || !action) return;
        state.eventStatuses[id] = action;
        const ev = INBOX_EVENTS.find((item) => item.id === id);
        if (ev) {
          state.dockNotice = {
            tag: 'TRIAGE',
            message: `${ev.ticker} marked as ${action.replace('_', ' ')} in local L1 queue.`,
            tone: action === 'too_hard' ? 'gold' : 'sky',
            dismissible: true,
          };
        }
        render();
      });
    });

    container.querySelectorAll<HTMLElement>('[data-universe-preset]').forEach((btn) => {
      btn.addEventListener('click', () => {
        const preset = btn.dataset.universePreset as UniverseFilter | undefined;
        if (preset) {
          state.universeFilter = preset;
          render();
        }
      });
    });

    container.querySelectorAll<HTMLElement>('[data-search-mode]').forEach((btn) => {
      btn.addEventListener('click', () => {
        const mode = btn.dataset.searchMode as UniverseSearchMode | undefined;
        if (mode) {
          state.universeSearchMode = mode;
          render();
        }
      });
    });

    container.querySelectorAll<HTMLElement>('[data-select-company]').forEach((row) => {
      row.addEventListener('click', () => {
        const ticker = row.dataset.selectCompany;
        if (ticker) {
          state.selectedTicker = ticker;
          render();
        }
      });
    });

    container.querySelectorAll<HTMLElement>('[data-subtab]').forEach((btn) => {
      btn.addEventListener('click', () => {
        const next = btn.dataset.subtab as CompanySubTab | undefined;
        if (next) {
          state.companySubTab = next;
          render();
        }
      });
    });

    container.querySelectorAll<HTMLElement>('[data-toggle-provenance]').forEach((btn) => {
      btn.addEventListener('click', () => {
        const id = btn.dataset.toggleProvenance;
        if (id) {
          state.openProvenanceId = state.openProvenanceId === id ? null : id;
          render();
        }
      });
    });

    container.querySelectorAll<HTMLElement>('[data-perspective]').forEach((btn) => {
      btn.addEventListener('click', () => {
        const key = btn.dataset.perspective as PerspectiveKey | undefined;
        if (key) {
          state.perspective = key;
          render();
        }
      });
    });

    container.querySelectorAll<HTMLElement>('[data-open-industry]').forEach((btn) => {
      btn.addEventListener('click', () => {
        const slug = btn.dataset.openIndustry;
        if (slug) state.selectedIndustrySlug = slug;
        state.tab = 'industry';
        render();
      });
    });

    container.querySelectorAll<HTMLElement>('[data-select-industry]').forEach((btn) => {
      btn.addEventListener('click', () => {
        const slug = btn.dataset.selectIndustry;
        if (slug) {
          state.selectedIndustrySlug = slug;
          render();
        }
      });
    });

    container.querySelectorAll<HTMLElement>('[data-ind-perspective]').forEach((btn) => {
      btn.addEventListener('click', () => {
        const key = btn.dataset.indPerspective as IndustryPerspectiveKey | undefined;
        if (key) {
          state.industryPerspective = key;
          render();
        }
      });
    });

    container.querySelectorAll<HTMLElement>('[data-toggle-feynman]').forEach((btn) => {
      btn.addEventListener('click', () => {
        const key = btn.dataset.toggleFeynman;
        if (key) {
          state.feynmanChecked[key] = !state.feynmanChecked[key];
          render();
        }
      });
    });

    container.querySelectorAll<HTMLElement>('[data-set-lang]').forEach((btn) => {
      btn.addEventListener('click', () => {
        const lang = btn.dataset.setLang as 'en' | 'zh' | undefined;
        if (lang) {
          state.settingsLang = lang;
          render();
        }
      });
    });

    container.querySelectorAll<HTMLElement>('[data-set-channel]').forEach((btn) => {
      btn.addEventListener('click', () => {
        const ch = btn.dataset.setChannel as 'stable' | 'beta' | undefined;
        if (ch) {
          state.settingsChannel = ch;
          render();
        }
      });
    });

    container.querySelector('[data-open-palette]')?.addEventListener('click', () => {
      state.paletteOpen = true;
      state.paletteQuery = '';
      render();
      document.getElementById('m3-palette-input')?.focus();
    });

    container.querySelectorAll<HTMLElement>('[data-close-palette]').forEach((el) => {
      el.addEventListener('click', (ev) => {
        if (ev.target === el || el.tagName === 'BUTTON') {
          state.paletteOpen = false;
          render();
        }
      });
    });

    const paletteInput = document.getElementById('m3-palette-input') as HTMLInputElement | null;
    if (paletteInput) {
      paletteInput.addEventListener('input', () => {
        const caret = paletteInput.selectionStart ?? paletteInput.value.length;
        state.paletteQuery = paletteInput.value;
        render();
        const fresh = document.getElementById('m3-palette-input') as HTMLInputElement | null;
        if (fresh) {
          fresh.focus();
          fresh.setSelectionRange(caret, caret);
        }
      });
    }

    container.querySelectorAll<HTMLElement>('[data-palette-cmd]').forEach((btn) => {
      btn.addEventListener('click', () => {
        const cmd = btn.dataset.paletteCmd || '';
        state.paletteOpen = false;
        if (cmd.startsWith('view:')) {
          state.tab = cmd.slice(5) as WorkbenchTab;
        } else if (cmd.startsWith('ticker:')) {
          state.selectedTicker = cmd.slice(7);
          state.tab = 'research';
        } else if (cmd === 'action:sync') {
          state.dockNotice = {
            tag: 'SYNCED',
            message: 'Command Palette triggered L0/L1 lakehouse refresh.',
            tone: 'emerald',
            dismissible: true,
          };
        }
        render();
      });
    });
  }

  window.addEventListener('keydown', (ev) => {
    const target = ev.target as HTMLElement | null;
    const isInput =
      target &&
      (target.tagName === 'INPUT' || target.tagName === 'TEXTAREA' || target.isContentEditable);

    if ((ev.metaKey || ev.ctrlKey) && ev.key.toLowerCase() === 'k') {
      ev.preventDefault();
      state.paletteOpen = !state.paletteOpen;
      render();
      if (state.paletteOpen) document.getElementById('m3-palette-input')?.focus();
      return;
    }

    if (ev.key === 'Escape' && state.paletteOpen) {
      state.paletteOpen = false;
      render();
      return;
    }

    if (isInput || ev.metaKey || ev.ctrlKey || ev.altKey) return;

    const map: Record<string, WorkbenchTab> = {
      '1': 'inbox',
      '2': 'universe',
      '3': 'industry',
      '4': 'baskets',
      '5': 'journal',
      ',': 'settings',
    };
    const next = map[ev.key];
    if (next && state.tab !== next) {
      state.tab = next;
      render();
    }
  });

  render();
}
