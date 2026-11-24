const W = '\x1b[38;2;255;255;255m';
const E = '\x1b[38;2;52;211;153m';
const G = '\x1b[38;2;251;191;36m';
const S = '\x1b[38;2;96;165;250m';
const B = '\x1b[1m';
const D = '\x1b[38;2;160;166;180m';
const M = '\x1b[38;2;107;114;130m';
const X = '\x1b[0m';

const REPO_URL = 'https://github.com/Project-M3/Money-Making-Machine';
const LATEST_MANIFEST_UPSTREAM = `${REPO_URL}/releases/latest/download/latest.json`;
const BETA_MANIFEST_UPSTREAM = `${REPO_URL}/releases/download/beta/latest.json`;
const RELEASES_URL = `${REPO_URL}/releases`;

const ANSI_BANNER = `
  ${W}██    ██  █████ ${X}   ${B}${W}PROJECT M3${X} ${M}·${X} ${E}v0.3.0 (Apache-2.0)${X}
  ${W}███  ███  ██████${X}   ${D}Scale like a machine. Decide like Munger.${X}
  ${W}████████      ██${X}   ${M}Read the whole market. Wait for the fat pitch.${X}
  ${W}██ ██ ██      ██${X}
  ${W}██ ██ ██    ███ ${X}   ${S}INSTALL (macOS)${X}  curl -fsSL https://m3.thinksmart.systems/install.sh | bash
  ${W}██    ██    ███ ${X}   ${S}BETA CHANNEL${X}     curl -fsSL https://m3.thinksmart.systems/install.sh | bash -s -- --channel beta
  ${W}██    ██      ██${X}   ${S}GITHUB${X}           ${REPO_URL}
  ${W}██    ██      ██${X}   ${S}MANIFESTS${X}        https://m3.thinksmart.systems/latest.json · /beta.json
  ${W}██    ██  ██████${X}
  ${W}██    ██  █████ ${X}   ${G}FOUR RULES${X}
                     01  Arithmetic is sacred — primary filing math untouched by LLM guesses
                     02  Invert before you believe — 4 adversarial lenses test every thesis
                     03  Sovereignty belongs to the owner — no machine emits buy/sell advice
                     04  Sit calmly, spend near zero — full-market scan at $0 standing cost

  ${M}Open https://m3.thinksmart.systems in a browser for the interactive M3.app workbench.${X}
`;

function withSecurityHeaders(headers, isRoot) {
  headers.set('x-content-type-options', 'nosniff');
  headers.set('referrer-policy', 'strict-origin-when-cross-origin');
  headers.set('x-frame-options', 'DENY');
  if (isRoot) {
    headers.set('vary', 'User-Agent');
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

export default {
  async fetch(request, env) {
    const url = new URL(request.url);
    const ua = (request.headers.get('user-agent') || '').toLowerCase();
    const isRoot = url.pathname === '/' || url.pathname === '';

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
        new Headers({
          'content-type': 'text/plain; charset=utf-8',
          'cache-control': 'public, max-age=300',
        }),
        true,
      );
      return new Response(request.method === 'HEAD' ? null : ANSI_BANNER, {
        status: 200,
        headers,
      });
    }

    const res = await env.ASSETS.fetch(request);
    const headers = withSecurityHeaders(new Headers(res.headers), isRoot);
    return new Response(request.method === 'HEAD' ? null : res.body, {
      status: res.status,
      statusText: res.statusText,
      headers,
    });
  },
};
