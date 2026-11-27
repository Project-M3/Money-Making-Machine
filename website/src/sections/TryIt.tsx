import { createSignal, For, onSettled, Show } from 'solid-js'

import { SectionHead } from '../components/SectionHead'
import { locale, t } from '../i18n'

/** Hotspot markers over the live Workbench, positioned against its fixed chrome. */
const MARKERS = [
  { n: 1, style: 'top: 184px; left: 18.2%' },
  { n: 2, style: 'top: 664px; left: 54.6%' },
  { n: 3, style: 'top: 282px; left: 17.2%' },
  { n: 4, style: 'top: 6px; left: 36.4%' },
]

function MacMenuBar() {
  return (
    <div
      aria-hidden="true"
      class="flex h-[26px] items-center justify-between border-b border-white/[0.07] bg-black/40 px-3.5 font-sans text-[12px] leading-none tracking-[-0.005em] text-white/85 select-none"
    >
      <div class="flex items-center gap-4">
        <svg viewBox="0 0 14 17" width="11" height="13" fill="currentColor" class="text-white/90">
          <path d="M11.6 9.1c0-2 1.6-3 1.7-3.1-.9-1.4-2.4-1.6-2.9-1.6-1.2-.1-2.4.7-3.1.7-.6 0-1.6-.7-2.6-.7-1.4 0-2.6.8-3.3 2-1.4 2.5-.4 6.1 1 8.1.7 1 1.5 2.1 2.6 2 1 0 1.4-.7 2.7-.7s1.6.7 2.7.7c1.1 0 1.8-1 2.5-2 .8-1.1 1.1-2.2 1.1-2.3-.1 0-2.4-.9-2.4-3.1ZM9.5 3.2c.6-.7 1-1.7.9-2.7-.9 0-1.9.6-2.5 1.3-.5.6-1 1.6-.9 2.6 1 .1 1.9-.5 2.5-1.2Z" />
        </svg>
        <span class="font-semibold text-white">M3</span>
        <For each={t.macMenus}>{(item) => <span class="text-white/70">{item}</span>}</For>
      </div>
      <div class="flex items-center gap-3 text-white/75">
        <span class="inline-flex items-center gap-1 rounded-[3px] border border-white/15 bg-white/[0.06] px-1.5 py-[2px] font-mono text-[10px] font-semibold tracking-[0.02em] text-white/90">
          M3
          <span class="size-1.5 rounded-full bg-[#34d399]" />
        </span>
        <span class="m3-num text-[11.5px] text-white/80">{t.macClock}</span>
      </div>
    </div>
  )
}

function WorkbenchIsland() {
  const [state, setState] = createSignal<'idle' | 'loading' | 'ready'>('idle')
  let host: HTMLDivElement | undefined
  let sentinel: HTMLDivElement | undefined

  onSettled(() => {
    if (!host || !sentinel) return
    const wide = window.matchMedia('(min-width: 1024px)')
    let observer: IntersectionObserver | undefined
    let disposed = false

    const load = () => {
      if (state() !== 'idle' || !host) return
      setState('loading')
      import('../workbench/mount')
        .then(({ mountIsland }) => {
          if (disposed || !host) return
          mountIsland(host, locale())
          setState('ready')
        })
        .catch(() => setState('idle'))
    }

    const watch = () => {
      if (!wide.matches || state() !== 'idle') return
      observer?.disconnect()
      observer = new IntersectionObserver(
        (entries) => {
          if (entries.some((e) => e.isIntersecting)) {
            observer?.disconnect()
            load()
          }
        },
        { rootMargin: '400px 0px' },
      )
      observer.observe(sentinel!)
    }

    watch()
    wide.addEventListener('change', watch)
    return () => {
      disposed = true
      observer?.disconnect()
      wide.removeEventListener('change', watch)
    }
  })

  return (
    <div ref={(el) => (sentinel = el)} class="relative">
      <Show when={state() !== 'ready'}>
        <div class="relative overflow-hidden rounded-[14px] border border-[#2a2a30] bg-shell shadow-[0_26px_64px_-18px_rgba(0,0,0,0.72)]">
          <img
            src="/shots/wb-inbox-1600.webp"
            width="1600"
            height="1192"
            alt=""
            loading="lazy"
            decoding="async"
            class="block h-auto w-full opacity-60"
          />
          <p class="absolute inset-x-0 top-1/2 text-center font-mono text-[13px] text-white/80" role="status">
            {t.tryLoading}
          </p>
        </div>
      </Show>
      <div ref={(el) => (host = el)} class={state() === 'ready' ? 'block' : 'hidden'} />
      <Show when={state() === 'ready'}>
        <For each={MARKERS}>
          {(m) => (
            <span
              aria-hidden="true"
              style={m.style}
              class="pointer-events-none absolute z-10 inline-flex size-7 items-center justify-center rounded-full bg-claret font-sans text-[13px] font-semibold text-white shadow-[0_0_0_3px_rgba(255,241,229,0.92)]"
            >
              {m.n}
            </span>
          )}
        </For>
      </Show>
    </div>
  )
}

function ScreenshotCarousel() {
  const [index, setIndex] = createSignal(0)
  let track: HTMLDivElement | undefined

  const go = (next: number) => {
    const count = t.slides.length
    const i = (next + count) % count
    const child = track?.children[i] as HTMLElement | undefined
    if (track && child) track.scrollTo({ left: child.offsetLeft - track.offsetLeft, behavior: 'smooth' })
    setIndex(i)
  }

  const onScroll = () => {
    if (!track) return
    const i = Math.round(track.scrollLeft / track.clientWidth)
    if (i !== index()) setIndex(i)
  }

  return (
    <div role="region" aria-roledescription="carousel" aria-label={t.carouselLabel}>
      <div ref={(el) => (track = el)} onScroll={onScroll} class="m3-carousel flex gap-4 overflow-x-auto">
        <For each={t.slides}>
          {(slide, i) => (
            <figure
              class="w-full shrink-0"
              role="group"
              aria-roledescription="slide"
              aria-label={`${i() + 1} / ${t.slides.length}`}
            >
              <div class="overflow-hidden rounded-[8px] border border-[#2a2a30] bg-shell">
                <img
                  src={`${slide.src}-900.webp`}
                  srcset={`${slide.src}-600.webp 600w, ${slide.src}-900.webp 900w`}
                  sizes="(min-width: 640px) 576px, calc(100vw - 40px)"
                  width="900"
                  height="671"
                  alt={slide.caption}
                  loading="lazy"
                  decoding="async"
                  class="block h-auto w-full"
                />
              </div>
              <figcaption class="mt-2.5 text-[13px] text-ink-2">
                <b class="m3-num font-semibold text-ink">{i() + 1}</b> · {slide.caption}
              </figcaption>
            </figure>
          )}
        </For>
      </div>
      <div class="mt-3 flex items-center justify-between">
        <div class="flex gap-1.5" aria-hidden="true">
          <For each={t.slides}>
            {(_, i) => <span class={`h-1.5 w-5 ${i() === index() ? 'bg-ink' : 'bg-hair'}`} />}
          </For>
        </div>
        <div class="flex gap-2">
          <button
            type="button"
            aria-label={t.carouselPrev}
            onClick={() => go(index() - 1)}
            class="inline-flex size-11 cursor-pointer items-center justify-center border border-ink text-[18px]"
          >
            ←
          </button>
          <button
            type="button"
            aria-label={t.carouselNext}
            onClick={() => go(index() + 1)}
            class="inline-flex size-11 cursor-pointer items-center justify-center border border-ink text-[18px]"
          >
            →
          </button>
        </div>
      </div>
    </div>
  )
}

export function TryIt() {
  return (
    <section id="try" aria-labelledby="try-title" class="m3-container pt-20 sm:pt-24">
      <SectionHead id="try-title" kicker={t.tryKicker} title={t.tryTitle} lead={t.tryLead} />

      <div class="mt-8 hidden lg:block">
        <figure class="border-t border-ink pt-3">
          <figcaption class="mb-3 flex flex-wrap justify-between gap-x-6 gap-y-1 text-[12.5px] text-ink-2">
            <span>
              <b class="font-semibold text-ink">{t.fig2Label}</b> {t.fig2Caption}
            </span>
            <span>{t.fig2Source}</span>
          </figcaption>
          <div class="overflow-hidden rounded-[8px] border border-[#2a2624] bg-[radial-gradient(95%_70%_at_50%_0%,#38302c_0%,#24201e_55%,#1a1817_100%)]">
            <MacMenuBar />
            <div class="p-4 sm:p-5 sm:pb-6">
              <WorkbenchIsland />
            </div>
          </div>
        </figure>

        <ol aria-label={t.tryHotspotsLabel} class="mt-6 grid grid-cols-4 gap-6 border-t border-hair pt-4">
          <For each={t.hotspots}>
            {(h, i) => (
              <li class="flex gap-3">
                <span class="m3-num inline-flex size-6 shrink-0 items-center justify-center rounded-full bg-claret text-[12px] font-semibold text-white">
                  {i() + 1}
                </span>
                <span>
                  <b class="block text-[14px] font-semibold text-ink">{h.title}</b>
                  <span class="mt-0.5 block text-[13.5px] leading-snug text-ink-2">{h.body}</span>
                </span>
              </li>
            )}
          </For>
        </ol>
      </div>

      <figure class="mt-8 border-t border-ink pt-3 lg:hidden">
        <figcaption class="mb-3 flex flex-wrap justify-between gap-x-6 gap-y-1 text-[12.5px] text-ink-2">
          <span>
            <b class="font-semibold text-ink">{t.fig2Label}</b> {t.fig2MobileCaption}
          </span>
          <span>{t.fig2Source}</span>
        </figcaption>
        <ScreenshotCarousel />
      </figure>
    </section>
  )
}
