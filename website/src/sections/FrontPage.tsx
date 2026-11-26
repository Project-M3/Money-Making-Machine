import { For } from 'solid-js'

import { RELEASES_URL, REPO_URL, t } from '../i18n'

export function FrontPage() {
  return (
    <section aria-labelledby="front-title" class="m3-container">
      <div class="grid gap-10 pt-9 pb-10 sm:pt-11 lg:grid-cols-[1.55fr_1px_0.9fr] lg:gap-10 lg:pb-12">
        <div>
          <p class="mb-3.5 text-[13px] font-semibold tracking-[0.02em] text-claret">{t.kicker}</p>
          <h1
            id="front-title"
            class="font-serif text-[44px] leading-[1.02] font-medium tracking-[-0.025em] text-ink [text-wrap:balance] sm:text-[60px] lg:text-[72px]"
          >
            <span class="block">{t.h1a}</span>
            <span class="block">{t.h1b}</span>
          </h1>
          <p class="mt-6 max-w-[34em] font-serif text-[19px] leading-[1.45] text-ink-2 [text-wrap:pretty] sm:text-[22px]">
            {t.deck}
          </p>
          <div class="mt-8 flex flex-wrap items-center gap-3">
            <a
              href={RELEASES_URL}
              target="_blank"
              rel="noopener noreferrer"
              data-track="download_click"
              class="m3-btn m3-btn-primary"
            >
              {t.ctaDownload}
            </a>
            <a
              href={REPO_URL}
              target="_blank"
              rel="noopener noreferrer"
              data-track="github_click"
              class="m3-btn m3-btn-secondary"
            >
              {t.ctaGithub}
            </a>
          </div>
          <p class="mt-4 text-[12.5px] leading-relaxed text-ink-3">{t.fine}</p>
        </div>

        <div class="hidden bg-hair lg:block" aria-hidden="true" />

        <aside aria-labelledby="ledger-title">
          <h2 id="ledger-title" class="font-serif text-[22px] font-semibold text-ink">
            {t.ledgerTitle}
          </h2>
          <p class="mt-1 mb-3.5 text-[12.5px] text-ink-3">{t.ledgerSub}</p>
          <table class="w-full border-collapse text-[14.5px]">
            <tbody>
              <For each={t.ledger}>
                {(row, i) => (
                  <tr class="border-t border-hair last:border-b last:border-b-ink">
                    <td
                      class={`m3-num w-[112px] py-3 pr-4 text-right align-baseline font-serif font-medium tracking-[-0.02em] ${
                        i() === t.ledger.length - 1 ? 'text-[46px] leading-none text-claret' : 'text-[34px] leading-none'
                      }`}
                    >
                      {row[0]}
                    </td>
                    <td class="py-3 align-baseline text-ink">{row[1]}</td>
                  </tr>
                )}
              </For>
            </tbody>
          </table>
          <blockquote class="mt-6 border-l-2 border-claret pl-3.5 font-serif text-[18px] leading-[1.45] text-ink-2 italic">
            <p>{t.quote}</p>
            <footer class="mt-1.5 font-sans text-[12px] text-ink-3 not-italic">{t.quoteBy}</footer>
          </blockquote>
        </aside>
      </div>

      <figure class="border-t border-ink pt-3">
        <figcaption class="mb-3 flex flex-wrap justify-between gap-x-6 gap-y-1 text-[12.5px] text-ink-2">
          <span>
            <b class="font-semibold text-ink">{t.figLabel}</b> {t.figCaption}
          </span>
          <span>{t.figSource}</span>
        </figcaption>
        <div class="relative max-h-[240px] overflow-hidden rounded-t-[6px] border border-b-0 border-[#2a2a30] bg-shell sm:max-h-[420px]">
          <img
            src="/shots/wb-inbox-1600.webp"
            srcset="/shots/wb-inbox-600.webp 600w, /shots/wb-inbox-900.webp 900w, /shots/wb-inbox-1600.webp 1600w"
            sizes="(min-width: 1200px) 1136px, calc(100vw - 40px)"
            width="1600"
            height="1192"
            alt={t.figAlt}
            loading="lazy"
            decoding="async"
            class="block h-auto w-full"
          />
          <div aria-hidden="true" class="absolute inset-x-0 bottom-0 h-16 bg-gradient-to-b from-transparent to-paper" />
        </div>
      </figure>
    </section>
  )
}
