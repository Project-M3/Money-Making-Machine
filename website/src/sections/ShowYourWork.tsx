import { For } from 'solid-js'
import { Tabs, TabsList, TabsPanel, TabsTab } from '@ui-sdk/solid/tabs'

import { SectionHead } from '../components/SectionHead'
import { locale, t } from '../i18n'

type Example = {
  id: string
  label: { en: string; zh: string }
  value: string
  caption: { en: string; zh: string }
  chain: Array<{ op: string; label: { en: string; zh: string }; amount: string }>
  formula?: string
  tags: string[]
  sha: string
  filing: string
  url: string
}

const FILING = '10-K FY25 · 0001059556-26-000014'
const FILING_URL = 'https://www.sec.gov/Archives/edgar/data/1059556/000105955626000014'

/** Worked examples taken verbatim from the public demo fixture (Moody's, FY25). */
const EXAMPLES: Example[] = [
  {
    id: 'oe',
    label: { en: 'Owner earnings', zh: '股东盈余' },
    value: '$2.18B',
    caption: { en: 'Strict owner earnings · Moody’s · FY25', zh: '严格股东盈余 · 穆迪 · FY25' },
    chain: [
      { op: '', label: { en: 'Operating cash flow', zh: '经营现金流' }, amount: '2,490' },
      { op: '−', label: { en: 'Capital expenditure', zh: '资本开支' }, amount: '101' },
      { op: '−', label: { en: 'Stock-based compensation', zh: '股权激励' }, amount: '209' },
      { op: '=', label: { en: 'Owner earnings ($M)', zh: '股东盈余（百万美元）' }, amount: '2,180' },
    ],
    tags: [
      'us-gaap:NetCashProvidedByUsedInOperatingActivities',
      'us-gaap:PaymentsToAcquirePropertyPlantAndEquipment',
      'us-gaap:ShareBasedCompensation',
    ],
    sha: 'sha256:3a90de14…4b22f08c',
    filing: FILING,
    url: FILING_URL,
  },
  {
    id: 'shares',
    label: { en: 'Share count', zh: '股本变化' },
    value: '−1.2%',
    caption: { en: '3-year share count delta, per year · Moody’s', zh: '三年股本年化变化 · 穆迪' },
    chain: [
      { op: '', label: { en: 'Diluted shares, FY22', zh: 'FY22 稀释后股本' }, amount: 'S₀' },
      { op: '', label: { en: 'Diluted shares, FY25', zh: 'FY25 稀释后股本' }, amount: 'S₃' },
      { op: '=', label: { en: 'Annualised change, net of buybacks', zh: '扣除回购后的年化变化' }, amount: '−1.2%' },
    ],
    formula: '(S₃ ÷ S₀)^(1/3) − 1',
    tags: ['dei:EntityCommonStockSharesOutstanding'],
    sha: 'sha256:71c40e52…11d92fa0',
    filing: FILING,
    url: FILING_URL,
  },
  {
    id: 'implied',
    label: { en: 'Implied growth', zh: '隐含增速' },
    value: '10.4%',
    caption: { en: 'Reverse-implied growth, 10 years · Moody’s', zh: '反推隐含增速（十年）· 穆迪' },
    chain: [
      { op: '', label: { en: 'Spot price', zh: '现价' }, amount: '$436.20' },
      { op: '', label: { en: 'Owner earnings', zh: '股东盈余' }, amount: '$2.18B' },
      { op: '', label: { en: 'Hurdle rate', zh: '门槛回报率' }, amount: '10%' },
      { op: '=', label: { en: 'Growth the price already assumes', zh: '价格已经假设的增速' }, amount: '10.4%' },
    ],
    formula: 'solve g: PV(OE, g, 10y, 10%) = price',
    tags: ['calculator::reverse_dcf'],
    sha: 'sha256:d049b661…6e71c309',
    filing: FILING,
    url: FILING_URL,
  },
]

function StepLabel(props: { n: string; text: string }) {
  return (
    <p class="flex items-center gap-2 text-[12px] font-semibold tracking-[0.06em] text-ink-3 uppercase">
      <span class="inline-flex size-5 items-center justify-center rounded-full border border-ink text-[11px] text-ink">
        {props.n}
      </span>
      {props.text}
    </p>
  )
}

export function ShowYourWork() {
  return (
    <section id="work" aria-labelledby="work-title" class="m3-container pt-20 sm:pt-24">
      <SectionHead id="work-title" kicker={t.workKicker} title={t.workTitle} lead={t.workLead} />

      <Tabs defaultValue="oe" class="mt-8">
        <TabsList aria-label={t.workTabsLabel} class="flex gap-6 border-b border-hair">
          <For each={EXAMPLES}>
            {(ex) => (
              <TabsTab value={ex.id} class="m3-tab">
                {ex.label[locale()]}
              </TabsTab>
            )}
          </For>
        </TabsList>

        <For each={EXAMPLES}>
          {(ex) => (
            <TabsPanel value={ex.id} class="grid gap-8 pt-8 lg:grid-cols-[0.8fr_1.2fr_1.1fr] lg:gap-0">
              <div class="lg:border-r lg:border-hair lg:pr-8">
                <StepLabel n="1" text={t.workStep1} />
                <p class="m3-num mt-4 font-serif text-[64px] leading-none font-medium tracking-[-0.03em] text-claret sm:text-[76px]">
                  {ex.value}
                </p>
                <p class="mt-3 text-[14px] text-ink-2">{ex.caption[locale()]}</p>
              </div>

              <div class="lg:border-r lg:border-hair lg:px-8">
                <StepLabel n="2" text={t.workStep2} />
                <table class="m3-num mt-4 w-full border-collapse font-mono text-[13.5px]">
                  <thead class="sr-only">
                    <tr>
                      <th scope="col">±</th>
                      <th scope="col">{t.workStep2}</th>
                      <th scope="col">$M</th>
                    </tr>
                  </thead>
                  <tbody>
                    <For each={ex.chain}>
                      {(row) => (
                        <tr class={row.op === '=' ? 'border-t border-ink font-semibold' : ''}>
                          <td class="w-5 py-1.5 align-top text-ink-3">{row.op}</td>
                          <td class="py-1.5 pr-3 align-top font-sans text-[14px] text-ink">{row.label[locale()]}</td>
                          <td class="py-1.5 text-right align-top whitespace-nowrap text-ink">{row.amount}</td>
                        </tr>
                      )}
                    </For>
                  </tbody>
                </table>
                {ex.formula && <p class="mt-3 font-mono text-[12.5px] text-ink-2">{ex.formula}</p>}
                <p class="mt-3 text-[12px] text-teal">{t.workEngine}</p>
              </div>

              <div class="min-w-0 lg:pl-8">
                <StepLabel n="3" text={t.workStep3} />
                <ul class="mt-4 space-y-1 font-mono text-[11.5px] text-ink [overflow-wrap:anywhere]">
                  <For each={ex.tags}>{(tag) => <li>{tag}</li>}</For>
                </ul>
                <dl class="mt-4 space-y-1.5 text-[13px]">
                  <div class="flex flex-wrap gap-x-2">
                    <dt class="text-ink-3">{locale() === 'zh' ? '财报' : 'Filing'}</dt>
                    <dd class="font-mono text-[12.5px] text-ink">{ex.filing}</dd>
                  </div>
                  <div class="flex flex-wrap gap-x-2">
                    <dt class="text-ink-3">{locale() === 'zh' ? '哈希' : 'Hash'}</dt>
                    <dd class="font-mono text-[12.5px] text-ink">{ex.sha}</dd>
                  </div>
                </dl>
                <a href={ex.url} target="_blank" rel="noopener noreferrer" class="m3-link mt-4 inline-block text-[14px] text-claret">
                  sec.gov ↗
                </a>
              </div>
            </TabsPanel>
          )}
        </For>
      </Tabs>

      <p class="mt-8 border-t border-hair pt-3 text-[12.5px] text-ink-3">{t.workNote}</p>
    </section>
  )
}
