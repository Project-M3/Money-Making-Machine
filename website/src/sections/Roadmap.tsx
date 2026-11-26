import { For } from 'solid-js'

import { SectionHead } from '../components/SectionHead'
import { ROADMAP_URL, t } from '../i18n'

export function Roadmap() {
  return (
    <section id="roadmap" aria-labelledby="road-title" class="m3-container pt-20 sm:pt-24">
      <SectionHead id="road-title" kicker={t.roadKicker} title={t.roadTitle} />
      <ol class="mt-8 border-t border-hair">
        <For each={t.milestones}>
          {(m) => (
            <li class="grid gap-x-8 gap-y-1 border-b border-hair py-5 sm:grid-cols-[120px_1fr]">
              <p class="m3-num">
                <span class="font-serif text-[24px] font-medium text-ink">{m.v}</span>
                <span class="mt-0.5 block text-[12px] text-ink-3">{m.date}</span>
              </p>
              <div>
                <h3 class="font-serif text-[20px] font-semibold text-ink">{m.title}</h3>
                <p class="mt-1 max-w-[70ch] text-[15px] leading-[1.6] text-ink-2">{m.body}</p>
              </div>
            </li>
          )}
        </For>
      </ol>
      <p class="mt-6 max-w-[70ch] font-serif text-[17px] leading-[1.55] text-ink-2 italic">
        {t.roadNext}{' '}
        <a href={ROADMAP_URL} target="_blank" rel="noopener noreferrer" class="m3-link font-sans text-[14px] text-claret not-italic">
          {t.roadLink} ↗
        </a>
      </p>
    </section>
  )
}
