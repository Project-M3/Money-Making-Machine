const W = '\x1b[38;2;255;255;255m';
const E = '\x1b[38;2;52;211;153m';
const G = '\x1b[38;2;251;191;36m';
const S = '\x1b[38;2;96;165;250m';
const B = '\x1b[1m';
const D = '\x1b[38;2;160;166;180m';
const M = '\x1b[38;2;107;114;130m';
const X = '\x1b[0m';

const REPO_URL = 'https://github.com/thinksmart-systems/m3';
const LATEST_MANIFEST_UPSTREAM = `${REPO_URL}/releases/latest/download/latest.json`;
const BETA_MANIFEST_UPSTREAM = `${REPO_URL}/releases/download/beta/latest.json`;
const RELEASES_URL = `${REPO_URL}/releases`;

const ANSI_BANNER = `
  ${W}██    ██  █████ ${X}   ${B}${W}M3${X} ${M}·${X} ${E}the research instrument for patient investors${X}
  ${W}███  ███  ██████${X}   ${D}Read every filing. Swing only at the fat pitches.${X}
  ${W}████████      ██${X}   ${M}耐心投资者的研究仪器 · 读遍每一份财报，只挥击好球。${X}
  ${W}██ ██ ██      ██${X}
  ${W}██ ██ ██    ███ ${X}   ${S}INSTALL (macOS)${X}  curl -fsSL https://m3.thinksmart.systems/install.sh | bash
  ${W}██    ██    ███ ${X}   ${S}BETA CHANNEL${X}     curl -fsSL https://m3.thinksmart.systems/install.sh | bash -s -- --channel beta
  ${W}██    ██      ██${X}   ${S}GITHUB${X}           ${REPO_URL}
  ${W}██    ██      ██${X}   ${S}MANIFESTS${X}        https://m3.thinksmart.systems/latest.json · /beta.json
  ${W}██    ██  ██████${X}
  ${W}██    ██  █████ ${X}   ${G}FOUR IRON RULES${X}
                     01  Arithmetic is sacred — no LLM touches a financial figure
                     02  Invert, always invert — four adversarial lenses per thesis
                     03  Judgment belongs to the owner — no buy/sell advice, ever
                     04  Sit calmly — local-first, zero-token filing inbox, hard AI budget

  ${M}Native macOS app · Interactive UI preview at https://m3.thinksmart.systems${X}
  ${M}Apache-2.0 · Powered by Alan (https://alanworks.app) · Not investment advice.${X}
`;

function withSecurityHeaders(headers, isRoot) {
  headers.set('x-content-type-options', 'nosniff');
  headers.set('referrer-policy', 'strict-origin-when-cross-origin');
  headers.set('x-frame-options', 'DENY');
  if (isRoot) {
    headers.set('vary', ROOT_VARY);
  }
  return headers;
}

function jsonResponse(payload, status = 200, cacheControl = 'no-store') {
  const headers = withSecurityHeaders(
    new Headers({
      'content-type': 'application/json; charset=utf-8',
      'cache-control': cacheControl,
      'access-control-allow-origin': '*',
    }),
    false,
  );
  return new Response(JSON.stringify(payload, null, 2), { status, headers });
}

async function serveInstaller(request, env, url) {
  if (request.method !== 'GET' && request.method !== 'HEAD') {
    return new Response('Method Not Allowed\n', {
      status: 405,
      headers: { allow: 'GET, HEAD', 'content-type': 'text/plain; charset=utf-8' },
    });
  }

  const assetUrl = new URL('/install.sh', url.origin);
  const assetResponse = await env.ASSETS.fetch(
    new Request(assetUrl.toString(), { method: request.method, headers: request.headers }),
  );
  if (!assetResponse.ok && assetResponse.status !== 304) {
    return new Response('Installer script not found\n', {
      status: assetResponse.status === 404 ? 404 : 502,
      headers: { 'content-type': 'text/plain; charset=utf-8', 'cache-control': 'no-store' },
    });
  }

  const headers = withSecurityHeaders(new Headers(assetResponse.headers), false);
  headers.set('content-type', 'text/x-shellscript; charset=utf-8');
  headers.set('cache-control', 'public, max-age=300');
  return new Response(request.method === 'HEAD' ? null : assetResponse.body, {
    status: assetResponse.status,
    headers,
  });
}

async function serveManifest(request, upstreamUrl, cacheMaxAge) {
  if (request.method !== 'GET' && request.method !== 'HEAD') {
    return jsonResponse({ error: 'method_not_allowed', message: 'Use GET or HEAD.' }, 405);
  }

  let upstream;
  try {
    upstream = await fetch(upstreamUrl, {
      method: request.method,
      headers: { 'user-agent': 'm3-website-worker', accept: 'application/json' },
      redirect: 'follow',
    });
  } catch {
    return jsonResponse(
      {
        error: 'upstream_unavailable',
        message: 'Unable to reach GitHub Releases for manifest.',
        releases_url: RELEASES_URL,
      },
      502,
    );
  }

  if (upstream.status === 404) {
    return jsonResponse(
      {
        error: 'release_not_found',
        message: 'No published M3 release manifest found on this channel yet.',
        releases_url: RELEASES_URL,
      },
      404,
      'public, max-age=60',
    );
  }

  if (!upstream.ok) {
    return jsonResponse(
      {
        error: 'upstream_error',
        message: 'GitHub Releases manifest fetch returned an unexpected status.',
        upstream_status: upstream.status,
        releases_url: RELEASES_URL,
      },
      502,
    );
  }

  const cacheControl = `public, max-age=${cacheMaxAge}`;
  if (request.method === 'HEAD') {
    const headers = withSecurityHeaders(
      new Headers({
        'content-type': 'application/json; charset=utf-8',
        'cache-control': cacheControl,
        'access-control-allow-origin': '*',
      }),
      false,
    );
    return new Response(null, { status: 200, headers });
  }

  let bodyText;
  try {
    bodyText = await upstream.text();
    JSON.parse(bodyText);
  } catch {
    return jsonResponse(
      {
        error: 'invalid_manifest',
        message: 'Upstream release manifest was not valid JSON.',
        releases_url: RELEASES_URL,
      },
      502,
    );
  }

  const headers = withSecurityHeaders(
    new Headers({
      'content-type': 'application/json; charset=utf-8',
      'cache-control': cacheControl,
      'access-control-allow-origin': '*',
    }),
    false,
  );
  return new Response(bodyText, { status: 200, headers });
}

const LANG_COOKIE = 'm3_lang';
const ROOT_VARY = 'Accept-Language, Cookie, User-Agent';

/** Reads an explicit `m3_lang` choice from the Cookie header. */
function parseCookieLang(cookieHeader) {
  if (!cookieHeader) return null;
  for (const part of cookieHeader.split(';')) {
    const eq = part.indexOf('=');
    if (eq === -1) continue;
    if (part.slice(0, eq).trim() !== LANG_COOKIE) continue;
    const val = part.slice(eq + 1).trim();
    if (val === 'en' || val === 'zh') return val;
  }
  return null;
}

/** Compares the best q-values for `en*` and `zh*`; ties go to the earlier tag; default `en`. */
function parseAcceptLanguage(header) {
  if (!header) return 'en';
  let bestEn = 0;
  let bestZh = 0;
  let firstEn = Infinity;
  let firstZh = Infinity;
  const parts = header.split(',');
  for (let i = 0; i < parts.length; i++) {
    const segments = parts[i].trim().split(';');
    const tag = (segments[0] || '').trim().toLowerCase();
    if (!tag) continue;
    let q = 1;
    for (let j = 1; j < segments.length; j++) {
      const param = segments[j].trim().toLowerCase();
      if (param.startsWith('q=')) {
        const parsed = Number.parseFloat(param.slice(2));
        if (!Number.isNaN(parsed)) q = parsed;
      }
    }
    if (q <= 0) continue;
    if (tag === 'en' || tag.startsWith('en-')) {
      if (q > bestEn) {
        bestEn = q;
        firstEn = i;
      }
    } else if (tag === 'zh' || tag.startsWith('zh-')) {
      if (q > bestZh) {
        bestZh = q;
        firstZh = i;
      }
    }
  }
  if (bestZh > bestEn) return 'zh';
  if (bestEn > bestZh) return 'en';
  if (bestZh > 0 && bestZh === bestEn) return firstZh < firstEn ? 'zh' : 'en';
  return 'en';
}

function negotiateLocale(request) {
  return (
    parseCookieLang(request.headers.get('cookie')) ??
    parseAcceptLanguage(request.headers.get('accept-language'))
  );
}

function notFound() {
  return new Response('Not Found\n', {
    status: 404,
    headers: withSecurityHeaders(
      new Headers({ 'content-type': 'text/plain; charset=utf-8', 'cache-control': 'no-store' }),
      false,
    ),
  });
}

export default {
  async fetch(request, env) {
    const url = new URL(request.url);
    const ua = (request.headers.get('user-agent') || '').toLowerCase();
    const isRoot = url.pathname === '/' || url.pathname === '' || url.pathname === '/index.html';

    // The zh page is an internal asset reached only through negotiation at `/`.
    if (url.pathname === '/_i18n' || url.pathname.startsWith('/_i18n/')) {
      return notFound();
    }
    if (url.pathname === '/install' || url.pathname === '/install.sh') {
      return serveInstaller(request, env, url);
    }
    if (url.pathname === '/latest.json') {
      return serveManifest(request, LATEST_MANIFEST_UPSTREAM, 300);
    }
    if (url.pathname === '/beta.json') {
      return serveManifest(request, BETA_MANIFEST_UPSTREAM, 60);
    }

    if (isRoot && (ua.includes('curl') || ua.includes('wget') || ua.includes('httpie'))) {
      const headers = withSecurityHeaders(
        new Headers({ 'content-type': 'text/plain; charset=utf-8', 'cache-control': 'public, max-age=300' }),
        true,
      );
      return new Response(request.method === 'HEAD' ? null : ANSI_BANNER, { status: 200, headers });
    }

    if (isRoot) {
      const lang = negotiateLocale(request);
      // Fetch without `.html`: the assets binding serves `_i18n/zh.html` at `/_i18n/zh`.
      const target = new URL(lang === 'zh' ? '/_i18n/zh' : '/', url);
      const res = await env.ASSETS.fetch(new Request(target, { method: request.method, headers: request.headers }));
      const headers = withSecurityHeaders(new Headers(res.headers), true);
      headers.set('vary', ROOT_VARY);
      headers.set('content-language', lang === 'zh' ? 'zh-CN' : 'en');
      headers.set('cache-control', 'private, no-cache');
      return new Response(request.method === 'HEAD' ? null : res.body, {
        status: res.status,
        statusText: res.statusText,
        headers,
      });
    }

    const res = await env.ASSETS.fetch(request);
    const headers = withSecurityHeaders(new Headers(res.headers), false);
    return new Response(request.method === 'HEAD' ? null : res.body, {
      status: res.status,
      statusText: res.statusText,
      headers,
    });
  },
};
