import legacyCss from './legacy.css?inline'
import tokensCss from './tokens.css?inline'
import { mountWorkbench } from './workbench'

const FONT_HREF = 'https://fonts.googleapis.com/css2?family=JetBrains+Mono:wght@400;500;600&display=swap'

/** Paper-page overrides for the dark app frame (shadow, radius) inside the island. */
const HOST_CSS = `
:host {
  display: block;
  color: var(--ink-primary);
  font-family: var(--font-sans);
  font-size: 16px;
  line-height: 1.6;
  -webkit-font-smoothing: antialiased;
  text-align: left;
}
.app-frame {
  box-shadow: 0 0 0 1px rgba(255, 255, 255, 0.08), 0 26px 64px -18px rgba(0, 0, 0, 0.72);
}
`

/**
 * Mounts the existing vanilla-TS Workbench simulator inside a shadow root so
 * its dark app stylesheet cannot leak into (or be affected by) the paper page.
 */
export function mountIsland(host: HTMLElement, lang: 'en' | 'zh') {
  if (host.shadowRoot) return
  if (!document.querySelector(`link[href="${FONT_HREF}"]`)) {
    const link = document.createElement('link')
    link.rel = 'stylesheet'
    link.href = FONT_HREF
    document.head.appendChild(link)
  }

  const shadow = host.attachShadow({ mode: 'open' })
  const style = document.createElement('style')
  style.textContent = tokensCss.replace(/:root\b/g, ':host') + legacyCss + HOST_CSS
  const frame = document.createElement('div')
  frame.className = 'app-frame'
  frame.setAttribute('aria-label', lang === 'zh' ? 'M3 工作台交互演示' : 'Interactive M3 Workbench demo')
  frame.setAttribute('role', 'region')
  shadow.append(style, frame)

  let hovering = false
  host.addEventListener('pointerenter', () => (hovering = true))
  host.addEventListener('pointerleave', () => (hovering = false))

  mountWorkbench(frame, {
    lang,
    isActive: () => hovering || host.matches(':focus-within'),
  })
}
