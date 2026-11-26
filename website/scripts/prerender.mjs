import fs from 'node:fs/promises'
import path from 'node:path'
import { fileURLToPath, pathToFileURL } from 'node:url'

/**
 * Build-time SSG. Renders the full page for both locales with Solid's server
 * renderer, inlines the compiled stylesheet, and writes:
 *   assets/index.html     (en, default)
 *   assets/_i18n/zh.html  (zh-CN, served by the Worker at `/` after negotiation)
 */
const __dirname = path.dirname(fileURLToPath(import.meta.url))
const root = path.resolve(__dirname, '..')
const assetsDir = path.join(root, '.cloudflare/output/v1/workers/default/assets')
const enHtmlPath = path.join(assetsDir, 'index.html')
const zhHtmlPath = path.join(assetsDir, '_i18n/zh.html')

const { renderPage, hydrationScript } = await import(pathToFileURL(path.join(root, '.ssr/entry-server.mjs')).href)

const ZH_HEAD = {
  title: 'M3 — 耐心投资者的研究仪器',
  description:
    'M3 是为耐心投资者打造的开源 macOS 研究仪器：在本地索引全部美股 SEC 申报人，用确定性的 Rust 计算股东盈余、ROIC 与股本稀释，每个数字都可追溯到财报，所有判断留给你。',
  ogDescription: '读遍每一份财报，只挥击好球。在你的 Mac 上做确定性的价值研究，每个数字都能追溯到 10-K。',
  ogImage: 'https://m3.thinksmart.systems/og-zh.png',
}

function replaceRequired(source, pattern, replacement, label) {
  const next = source.replace(pattern, replacement)
  if (next === source) throw new Error(`[prerender] failed to replace ${label}`)
  return next
}

function localizeHeadForZh(template) {
  let out = template
  out = replaceRequired(out, '<html lang="en">', '<html lang="zh-CN">', 'html lang')
  out = replaceRequired(out, /<title>[^<]*<\/title>/, `<title>${ZH_HEAD.title}</title>`, 'title')
  out = replaceRequired(out, /(<meta\s+name="description"\s+content=")[^"]*(")/, `$1${ZH_HEAD.description}$2`, 'description')
  out = replaceRequired(out, /(<meta\s+property="og:locale"\s+content=")en_US(")/, '$1zh_CN$2', 'og:locale')
  out = replaceRequired(out, /(<meta\s+property="og:title"\s+content=")[^"]*(")/, `$1${ZH_HEAD.title}$2`, 'og:title')
  out = replaceRequired(out, /(<meta\s+property="og:description"\s+content=")[^"]*(")/, `$1${ZH_HEAD.ogDescription}$2`, 'og:description')
  out = replaceRequired(out, /(<meta\s+property="og:image"\s+content=")[^"]*(")/, `$1${ZH_HEAD.ogImage}$2`, 'og:image')
  out = replaceRequired(out, /(<meta\s+name="twitter:image"\s+content=")[^"]*(")/, `$1${ZH_HEAD.ogImage}$2`, 'twitter:image')
  out = replaceRequired(out, /("description":\s*")[^"]*(")/, `$1${ZH_HEAD.description}$2`, 'json-ld description')
  out = replaceRequired(out, /("inLanguage":\s*")en(")/, '$1zh-CN$2', 'json-ld inLanguage')
  out = replaceRequired(out, /("image":\s*")[^"]*(")/, `$1${ZH_HEAD.ogImage}$2`, 'json-ld image')
  return out
}

const ROOT_PLACEHOLDER = '<div id="root"></div>'
const STYLESHEET_RE = /<link\s+rel="stylesheet"[^>]*href="(\/assets\/[^"]+\.css)"[^>]*>/

const rawHtml = await fs.readFile(enHtmlPath, 'utf8')
if (!rawHtml.includes(ROOT_PLACEHOLDER)) throw new Error(`[prerender] ${enHtmlPath} lacks ${ROOT_PLACEHOLDER}`)

const cssMatch = STYLESHEET_RE.exec(rawHtml)
if (!cssMatch) throw new Error(`[prerender] ${enHtmlPath} missing built stylesheet <link>`)
const cssPath = path.join(assetsDir, cssMatch[1].replace(/^\//, ''))
const inlineCss = (await fs.readFile(cssPath, 'utf8')).replaceAll('</style>', '<\\/style>')
const template = rawHtml
  .replace(cssMatch[0], `<style>${inlineCss}</style>`)
  .replace('</head>', `${hydrationScript()}</head>`)

const enHtml = template.replace(ROOT_PLACEHOLDER, `<div id="root">${renderPage('en')}</div>`)
const zhHtml = localizeHeadForZh(template).replace(ROOT_PLACEHOLDER, `<div id="root">${renderPage('zh')}</div>`)

await fs.writeFile(enHtmlPath, enHtml, 'utf8')
await fs.mkdir(path.dirname(zhHtmlPath), { recursive: true })
await fs.writeFile(zhHtmlPath, zhHtml, 'utf8')

console.log(
  `[prerender] inlined ${cssMatch[1]}; wrote ${path.relative(root, enHtmlPath)} (${enHtml.length} B) + ${path.relative(root, zhHtmlPath)} (${zhHtml.length} B)`,
)
