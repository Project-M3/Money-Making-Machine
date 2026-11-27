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
        <div class="relative overflow-hidden rounded-[14px] border border-[#2a2a30] bg-shell">
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
              class="pointer-events-none absolute z-10 inline-flex size-7 items-center justify-center rounded-full bg-claret font-sans text-[13px] font-semibold text-white shadow-[0_0_0_3px_rgba(255,241,229,0.9)]"
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
          <WorkbenchIsland />
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
