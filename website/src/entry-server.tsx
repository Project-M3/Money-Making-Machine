import { generateHydrationScript, renderToString } from '@solidjs/web'

import { App } from './App'
import { type Locale, setServerLocale } from './i18n'

/** Build-time only: render the whole page for one locale. */
export function renderPage(locale: Locale): string {
  setServerLocale(locale)
  return renderToString(() => <App />)
}

/** Tiny inline bootstrap that lets `hydrate()` claim the prerendered nodes. */
export function hydrationScript(): string {
  return generateHydrationScript()
}
