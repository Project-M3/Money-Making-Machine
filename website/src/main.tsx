import { hydrate } from '@solidjs/web'

import './styles/app.css'
import { installAnalytics } from './analytics'
import { App } from './App'

const root = document.getElementById('root')
if (root) {
  // Hydrate the build-time prerendered markup in place: no repaint, no new LCP.
  hydrate(() => <App />, root)
}
installAnalytics()
