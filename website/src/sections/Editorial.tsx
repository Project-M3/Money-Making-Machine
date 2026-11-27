import { For } from 'solid-js'

import { SectionHead } from '../components/SectionHead'
import { t } from '../i18n'

export function Editorial() {
  return (
    <section id="rules" aria-labelledby="rules-title" class="m3-container pt-20 sm:pt-24">
      <SectionHead id="rules-title" kicker={t.rulesKicker} title={t.rulesTitle} lead={t.rulesLead} />
      <div class="mt-10 grid border-t border-hair sm:grid-cols-2">
        <For each={t.rules}>
          {(rule, i) => (
            <article
              class={`border-b border-hair py-7 sm:py-8 ${i() % 2 === 0 ? 'sm:border-r sm:pr-8' : 'sm:pl-8'}`}
            >
              <p class="m3-num font-serif text-[15px] text-claret italic">No. {rule.n}</p>
              <h3 class="mt-1.5 font-serif text-[28px] leading-[1.15] font-medium tracking-[-0.015em] text-ink">
                {rule.title}
              </h3>
              <p class="mt-3 text-[15.5px] leading-[1.65] text-ink-2 [text-wrap:pretty]">{rule.body}</p>
              <p class="mt-4 font-mono text-[12px] tracking-[0.01em] text-ink-3">{rule.proof}</p>
            </article>
          )}
        </For>
      </div>
    </section>
  )
}
