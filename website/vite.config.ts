import { cloudflare } from '@cloudflare/vite-plugin'
import solid from '@solidjs/vite-plugin'
import tailwindcss from '@tailwindcss/vite'
import { defineConfig } from 'vite'

export default defineConfig({
  plugins: [solid({ ssr: true }), tailwindcss(), cloudflare()],
  define: {
    __BUILD_DAY__: JSON.stringify(new Date().toISOString().slice(0, 10)),
  },
  resolve: {
    dedupe: ['solid-js', '@solidjs/web', '@ui-sdk/core', '@ui-sdk/solid'],
  },
  build: {
    target: 'es2022',
  },
})
