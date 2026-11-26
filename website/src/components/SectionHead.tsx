import type { JSX } from '@solidjs/web'

/** Newspaper section head: a heavy rule, a small claret kicker and a serif title. */
export function SectionHead(props: { kicker: string; title: string; lead?: string; id?: string; aside?: JSX.Element }) {
  return (
    <header class="border-t-2 border-ink pt-3">
      <div class="flex flex-wrap items-baseline justify-between gap-x-6 gap-y-2">
        <p class="text-[13px] font-semibold tracking-[0.04em] text-claret uppercase">{props.kicker}</p>
        {props.aside}
      </div>
      <h2
        id={props.id}
        class="mt-3 max-w-[22ch] font-serif text-[34px] leading-[1.08] font-medium tracking-[-0.02em] text-ink [text-wrap:balance] sm:text-[44px]"
      >
        {props.title}
      </h2>
      {props.lead && (
        <p class="mt-4 max-w-[62ch] font-serif text-[18px] leading-[1.55] text-ink-2 [text-wrap:pretty] sm:text-[20px]">
          {props.lead}
        </p>
      )}
    </header>
  )
}
