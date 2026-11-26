import { createSignal, onSettled } from 'solid-js'

import { FALLBACK_VERSION } from './i18n'

const [version, setVersion] = createSignal(FALLBACK_VERSION)
let requested = false

/** Latest stable version, read from the Worker's `/latest.json` manifest proxy. */
export function useReleaseVersion() {
  onSettled(() => {
    if (requested) return
    requested = true
    fetch('/latest.json', { cache: 'default' })
      .then((res) => (res.ok ? res.json() : null))
      .then((data: { version?: unknown } | null) => {
        if (typeof data?.version !== 'string') return
        const trimmed = data.version.trim().replace(/^v/, '')
        if (/^\d+\.\d+\.\d+/.test(trimmed)) setVersion(`v${trimmed}`)
      })
      .catch(() => {})
  })
  return version
}
