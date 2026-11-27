import { createSignal, onSettled } from 'solid-js'

import { locale, RELEASES_URL, REPO_URL, setLocale, t } from '../i18n'
import { useReleaseVersion } from '../release'

declare const __BUILD_DAY__: string

function formatDate(l: 'en' | 'zh', now: Date) {
  return l === 'zh'
    ? new Intl.DateTimeFormat('zh-CN', { dateStyle: 'full' }).format(now)
    : new Intl.DateTimeFormat('en-GB', { weekday: 'long', day: 'numeric', month: 'long', year: 'numeric' }).format(now)
}

export function Masthead() {
  const version = useReleaseVersion()
  // Start from the build day so hydration matches the prerendered HTML, then show today.
  const [now, setNow] = createSignal(new Date(`${__BUILD_DAY__}T12:00:00Z`))
  onSettled(() => {
    setNow(new Date())
  })
  const date = () => formatDate(locale(), now())

  return (
    <header class="m3-container">
      <div class="flex h-14 items-center justify-between gap-4 text-[13px] text-ink-2">
        <span class="hidden sm:inline">{t.topLeft}</span>
        <a href="#top" class="font-serif text-[20px] font-semibold tracking-[-0.02em] text-ink sm:hidden">
          M3
        </a>
        <nav aria-label={t.navLabel} class="flex items-center gap-5 lg:gap-6">
          <a href="#work" class="hidden hover:text-ink lg:inline">
            {t.navWork}
          </a>
          <a href="#try" class="hidden hover:text-ink lg:inline">
            {t.navTry}
          </a>
          <a href="#rules" class="hidden hover:text-ink lg:inline">
            {t.navRules}
          </a>
          <a href="#roadmap" class="hidden hover:text-ink lg:inline">
            {t.navRoadmap}
          </a>
          <a
            href={REPO_URL}
            target="_blank"
            rel="noopener noreferrer"
            data-track="github_click"
            class="hidden hover:text-ink md:inline"
          >
            GitHub
          </a>
          <button
            type="button"
            lang={locale() === 'en' ? 'zh-CN' : 'en'}
            title={t.navOtherLocaleLabel}
            class="inline-flex min-h-[44px] cursor-pointer items-center hover:text-ink"
            onClick={() => setLocale(locale() === 'en' ? 'zh' : 'en')}
          >
            {t.navOtherLocale}
          </button>
          <a
            href={RELEASES_URL}
            target="_blank"
            rel="noopener noreferrer"
            data-track="download_click"
            class="inline-flex h-9 items-center bg-ink px-3.5 text-[13px] font-medium text-paper hover:bg-[#1f1d1c]"
          >
            {t.navDownload}
          </a>
        </nav>
      </div>

      <div class="border-t border-ink pt-4 pb-3 text-center sm:pt-5">
        <p class="font-serif text-[52px] leading-none font-semibold tracking-[-0.02em] text-ink sm:text-[68px]" aria-hidden="true">
          M3
        </p>
        <p class="mt-2 font-serif text-[16px] text-ink-2 italic sm:text-[18px]">{t.tagline}</p>
      </div>

      <div class="m3-double-rule flex flex-wrap items-center justify-between gap-x-6 gap-y-1 border-b border-ink py-2 text-[12.5px] tracking-[0.01em] text-ink">
        <span class="m3-num">{date()}</span>
        <span class="m3-num hidden items-center gap-6 md:flex">
          <span>{t.datelineIssuers}</span>
          <span>{t.datelineArith}</span>
          <span class="hidden lg:inline">{t.datelineMcp}</span>
        </span>
        <span class="m3-num">{version()}</span>
      </div>
    </header>
  )
}
