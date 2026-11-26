import { For } from 'solid-js'

import { RESEARCH_URL, t } from '../i18n'

export function Corrections() {
  return (
    <section id="corrections" aria-labelledby="corr-title" class="m3-container pt-20 sm:pt-24">
      <div class="border-2 border-ink bg-paper-2 px-5 py-6 sm:px-8 sm:py-8">
        <p class="text-[13px] font-semibold tracking-[0.04em] text-claret uppercase">{t.corrKicker}</p>
        <h2
          id="corr-title"
          class="mt-2 font-serif text-[30px] leading-[1.1] font-medium tracking-[-0.02em] text-ink sm:text-[38px]"
        >
          {t.corrTitle}
        </h2>
        <p class="mt-3 max-w-[64ch] font-serif text-[17px] leading-[1.55] text-ink-2 sm:text-[19px]">{t.corrLead}</p>

        <ol class="mt-6 grid gap-6 border-t border-ink pt-6 md:grid-cols-3 md:gap-8">
          <For each={t.corrections}>
            {(c) => (
              <li class="flex flex-col">
                <p class="text-[12px] tracking-[0.02em] text-ink-3">{c.when}</p>
                <h3 class="mt-1 font-serif text-[21px] leading-[1.2] font-semibold text-ink">{c.title}</h3>
                <p class="mt-2 text-[12px] font-semibold tracking-[0.06em] text-claret uppercase">{c.verdict}</p>
                <p class="m3-num mt-2 text-[14.5px] leading-[1.6] text-ink-2">{c.body}</p>
                <a
                  href={c.link}
                  target="_blank"
                  rel="noopener noreferrer"
                  class="m3-link mt-3 inline-flex min-h-[24px] items-center text-[14px] text-ink"
                >
                  {t.corrReview} ↗
                </a>
              </li>
            )}
          </For>
        </ol>
        <p class="mt-6 border-t border-hair pt-3 text-[13px]">
          <a href={RESEARCH_URL} target="_blank" rel="noopener noreferrer" class="m3-link text-claret">
            {t.corrFooter} ↗
          </a>
        </p>
      </div>
    </section>
  )
}
