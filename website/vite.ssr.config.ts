import solid from '@solidjs/vite-plugin'
import { defineConfig } from 'vite'

/**
 * Build-time-only server bundle. `scripts/prerender.mjs` imports it to render
 * the full page to static HTML for each locale; it is never deployed.
 */
export default defineConfig({
  plugins: [solid({ ssr: true })],
  define: {
    __BUILD_DAY__: JSON.stringify(new Date().toISOString().slice(0, 10)),
  },
  resolve: {
    dedupe: ['solid-js', '@solidjs/web', '@ui-sdk/core', '@ui-sdk/solid'],
  },
  ssr: {
    noExternal: ['@ui-sdk/core', '@ui-sdk/solid'],
  },
  build: {
    ssr: 'src/entry-server.tsx',
    outDir: '.ssr',
    emptyOutDir: true,
    target: 'node22',
    rollupOptions: {
      output: { entryFileNames: 'entry-server.mjs' },
    },
  },
})
