import { createSignal, Show } from 'solid-js'

import {
  ALAN_URL,
  AUTHOR_URL,
  CHANGELOG_URL,
  LICENSE_URL,
  REPO_URL,
  SLOWPAGE_URL,
  t,
} from '../i18n'

const linkClass = 'hover:text-ink'

export function Footer() {
  const [egg, setEgg] = createSignal(0)
  const state = () => t.eggStates[egg()]

  return (
    <footer class="m3-container pb-14">
      <div class="m3-double-rule pt-6">
        <div class="flex flex-col gap-4 sm:flex-row sm:items-baseline sm:justify-between">
          <p class="flex flex-wrap items-baseline gap-x-3 gap-y-1">
            <button
              type="button"
              title={t.eggLabel}
              onClick={() => setEgg((egg() + 1) % t.eggStates.length)}
              class="cursor-pointer font-serif text-[22px] font-semibold tracking-[-0.01em] text-ink"
            >
              {state()[0]}
            </button>
            <Show when={state()[1]}>
              <span aria-live="polite" class="font-serif text-[15px] text-ink-2 italic">
                {state()[1]}
              </span>
            </Show>
          </p>
          <nav aria-label="Project" class="flex flex-wrap gap-x-5 gap-y-2 text-[13px] text-ink-2">
            <a href={REPO_URL} target="_blank" rel="noopener noreferrer" data-track="github_click" class={linkClass}>
              GitHub
            </a>
            <a href={CHANGELOG_URL} target="_blank" rel="noopener noreferrer" class={linkClass}>
              {t.footerChangelog}
            </a>
            <a href={LICENSE_URL} target="_blank" rel="noopener noreferrer" class={linkClass}>
              {t.footerLicense}
            </a>
            <a href={ALAN_URL} target="_blank" rel="noopener noreferrer" class={linkClass}>
              {t.footerPowered}
            </a>
          </nav>
        </div>

        <div class="mt-5 flex flex-col gap-4 border-t border-hair pt-4 text-[12.5px] leading-relaxed text-ink-3 sm:flex-row sm:justify-between">
          <div class="max-w-[680px] space-y-1">
            <p>{t.footerDisclaimer}</p>
            <p>{t.footerPrivacy}</p>
          </div>
          <nav aria-label={t.footerFamily} class="flex items-baseline gap-3 whitespace-nowrap">
            <span>{t.footerFamily}</span>
            <a href={ALAN_URL} target="_blank" rel="noopener noreferrer" class={`text-ink-2 ${linkClass}`}>
              Alan
            </a>
            <a href={SLOWPAGE_URL} target="_blank" rel="noopener noreferrer" class={`text-ink-2 ${linkClass}`}>
              Slowpage
            </a>
            <a href={AUTHOR_URL} target="_blank" rel="noopener noreferrer" class={`text-ink-2 ${linkClass}`}>
              Morris Liu
            </a>
          </nav>
        </div>
      </div>
    </footer>
  )
}
