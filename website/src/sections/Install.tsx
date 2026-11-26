import { createSignal, For } from 'solid-js'
import {
  SegmentedControl,
  SegmentedControlIndicator,
  SegmentedControlItem,
} from '@ui-sdk/solid/segmented-control'

import { capture } from '../analytics'
import { SectionHead } from '../components/SectionHead'
import { INSTALL_COMMANDS, REPO_URL, t } from '../i18n'

type Channel = keyof typeof INSTALL_COMMANDS

export function Install() {
  const [channel, setChannel] = createSignal<Channel>('stable')
  const [copied, setCopied] = createSignal(false)
  let timer: ReturnType<typeof setTimeout> | undefined

  const copy = async () => {
    try {
      await navigator.clipboard.writeText(INSTALL_COMMANDS[channel()])
      setCopied(true)
      clearTimeout(timer)
      timer = setTimeout(() => setCopied(false), 1600)
    } catch {
      // Clipboard can be unavailable (permissions, insecure context); the command stays selectable.
    }
    capture('install_copy', { channel: channel() })
  }

  return (
    <section id="install" aria-labelledby="install-title" class="m3-container pt-20 pb-20 sm:pt-24 sm:pb-24">
      <SectionHead id="install-title" kicker={t.installKicker} title={t.installTitle} lead={t.installLead} />

      <div class="mt-8 grid gap-10 lg:grid-cols-[1.15fr_1fr] lg:gap-12">
        <div class="min-w-0">
          <div class="flex flex-wrap items-center justify-between gap-3">
            <SegmentedControl
              class="m3-seg"
              aria-label={t.channelLabel}
              value={channel()}
              onValueChange={(next) => next && setChannel(next as Channel)}
            >
              <SegmentedControlIndicator />
              <SegmentedControlItem value="stable">{t.channelStable}</SegmentedControlItem>
              <SegmentedControlItem value="beta">{t.channelBeta}</SegmentedControlItem>
            </SegmentedControl>
            <span class="text-[12.5px] text-ink-3">{t.installReq}</span>
          </div>

          <div class="mt-4 flex items-stretch border border-ink bg-shell">
            <pre class="min-w-0 flex-1 overflow-x-auto px-4 py-4 font-mono text-[13px] leading-relaxed text-[#eceef2]">
              <code>{INSTALL_COMMANDS[channel()]}</code>
            </pre>
            <button
              type="button"
              aria-label={t.copyLabel}
              onClick={copy}
              class="shrink-0 cursor-pointer border-l border-white/15 px-4 font-mono text-[13px] font-medium text-white hover:bg-white/10"
            >
              <span aria-live="polite">{copied() ? t.copied : t.copy}</span>
            </button>
          </div>

          <div class="mt-5 flex flex-wrap gap-3">
            <a
              href={`${REPO_URL}/releases`}
              target="_blank"
              rel="noopener noreferrer"
              data-track="download_click"
              class="m3-btn m3-btn-primary"
            >
              {t.installReleases}
            </a>
            <a
              href={REPO_URL}
              target="_blank"
              rel="noopener noreferrer"
              data-track="github_click"
              class="m3-btn m3-btn-secondary"
            >
              GitHub
            </a>
          </div>
        </div>

        <div class="min-w-0">
          <h3 class="font-serif text-[20px] font-semibold text-ink">{t.cliTitle}</h3>
          <dl class="mt-3 border-t border-hair">
            <For each={t.cli}>
              {(row) => (
                <div class="border-b border-hair py-3">
                  <dt class="font-mono text-[13px] break-words text-ink">{row[0]}</dt>
                  <dd class="mt-0.5 text-[14px] text-ink-2">{row[1]}</dd>
                </div>
              )}
            </For>
          </dl>
        </div>
      </div>
    </section>
  )
}
