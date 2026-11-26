# M3 Website (`m3.thinksmart.systems`)

The public front page for **M3 — the research instrument for patient investors**, built as a
financial-newspaper page and deployed to Cloudflare Workers.

## Stack

- **UI**: Solid 2.0 (`solid-js`, `@solidjs/web`, `@solidjs/vite-plugin`) + `@ui-sdk/solid` behaviours
  (Tabs for “Show your work”, Segmented Control for the install channel)
- **Styling & build**: Tailwind CSS v4 (`@tailwindcss/vite`) + Vite 8
- **Static prerender (SSG)**: `vite.ssr.config.ts` builds a build-time-only server bundle
  (`.ssr/entry-server.mjs`); `scripts/prerender.mjs` renders the whole page with Solid's
  `renderToString` for both locales, inlines the compiled CSS, and writes
  `assets/index.html` (en) and `assets/_i18n/zh.html` (zh-CN). The client then renders the same
  tree in place.
- **Try it**: the existing vanilla-TS Workbench simulator (`src/workbench/`) is lazy-loaded on
  ≥1024px viewports when scrolled near, inside a shadow root so its dark app stylesheet stays
  isolated. Phones get a screenshot carousel instead (`public/shots/`).
- **Edge**: `@cloudflare/vite-plugin`, custom Worker (`worker/index.js`), `cf` CLI
  (`cloudflare.config.ts`).

## Local development

```bash
pnpm install
pnpm dev        # Vite dev server
pnpm typecheck  # tsc --noEmit
pnpm build      # typecheck + client build + SSR bundle + prerender (en, zh)
pnpm preview    # serve the built Worker + assets locally
```

## Family conventions (Alan · M3 · Slowpage)

Shared components are copied per repository, not packaged.

- **Clean-URL i18n, English first**: `/` is the only page URL — no `/en` or `/zh` prefix.
  `worker/index.js` negotiates on `/`: `m3_lang` cookie > `Accept-Language` (q-values of `en*` vs
  `zh*`) > default `en`; zh is served from the internal asset `/_i18n/zh` (fetched without
  `.html`). Direct `/_i18n/*` requests return 404. Responses carry
  `Vary: Accept-Language, Cookie, User-Agent`, `Content-Language` and
  `Cache-Control: private, no-cache`. The 中文 / English button switches in place without a
  reload (`src/i18n.ts`), updating the cookie, `<html lang>`, `document.title` and the PostHog
  `locale` property.
- **curl card**: `curl https://m3.thinksmart.systems` returns an ANSI text card.
- **Install & manifests**: `/install.sh` (must equal `../scripts/install.sh`; CI diffs them),
  `/latest.json` and `/beta.json` proxy GitHub Release manifests.
- **Anonymous PostHog**: `public/ph.js` + `src/analytics.ts`, loaded after `load` on idle, no
  session recording, no person profiles. Events: `$pageview`, `install_copy`,
  `download_click`, `github_click`, `scroll_75`.
- **Footer**: disclaimer, privacy note, the M³ easter egg, “Powered by Alan”, and family links to
  alanworks.app and slowpage.app.

## Content rules

Facts come from `README.md`, `ROADMAP.md`, `CHANGELOG.md` and `docs/research/`. Numbers in the
ledger, Fig. 1 and “Show your work” are the public demo fixture (`src/workbench/fixture.ts`) and
are labelled as such. Keep “autonomous” / “execution” out of the hero and meta tags; the
Money-Making-Machine name appears only as the footer easter egg.

## Deployment

Production deploys through **Cloudflare Workers Builds** on `main` — no deploy credentials live in
GitHub, and nobody runs a production `cf deploy` by hand.

- Root directory: `/website`
- Build command: `pnpm run build`
- Deploy command: `pnpm exec cf deploy --prebuilt --mode production`
- Watched paths: `website/*`

[`.github/workflows/website.yml`](../.github/workflows/website.yml) only verifies the build on pull
requests and pushes.
