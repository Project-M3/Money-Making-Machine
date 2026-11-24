import { bindings, defineConfig } from 'cf/config';

export default defineConfig({
  accountId: 'b96556986b5109d88eb943aef8a7fa6d',
  worker: {
    name: 'm3-website',
    entrypoint: './worker/index.js',
    compatibilityDate: '2026-11-01',
    env: {
      ASSETS: bindings.assets(),
    },
    assets: {
      runWorkerFirst: ['/'],
    },
    domains: ['m3.thinksmart.systems'],
  },
});
