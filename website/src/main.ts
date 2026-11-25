import './styles/tokens.css';
import './styles/main.css';

import { mountPipeline } from './components/pipeline';
import { mountWorkbench } from './components/workbench';
import { CLI_COMMANDS } from './data/fixture';

function escapeHtml(raw: string): string {
  return raw
    .replaceAll('&', '&amp;')
    .replaceAll('<', '&lt;')
    .replaceAll('>', '&gt;')
    .replaceAll('"', '&quot;');
}

function mountCliDeck(container: HTMLElement): void {
  container.innerHTML = CLI_COMMANDS.map(
    (item) => `
    <div class="cli-item">
      <div style="min-width: 0;">
        <code class="cli-cmd">${escapeHtml(item.cmd)}</code>
        <span class="cli-desc">${escapeHtml(item.desc)}</span>
      </div>
      <button type="button" class="copy-btn" data-copy-text="${escapeHtml(item.cmd)}">Copy</button>
    </div>
  `,
  ).join('');
}

function bindInstallChannelToggle(): void {
  const buttons = document.querySelectorAll<HTMLButtonElement>('[data-install-channel]');
  const codeEl = document.getElementById('install-cmd');
  const copyBtn = document.getElementById('install-copy-btn');
  const hintEl = document.getElementById('install-channel-hint');
  if (!buttons.length || !codeEl || !copyBtn) return;

  const commands: Record<'stable' | 'beta', { cmd: string; hint: string }> = {
    stable: {
      cmd: 'curl -fsSL https://m3.thinksmart.systems/install.sh | bash',
      hint: 'Stable release',
    },
    beta: {
      cmd: 'curl -fsSL https://m3.thinksmart.systems/install.sh | bash -s -- --channel beta',
      hint: 'Rolling beta track',
    },
  };

  buttons.forEach((btn) => {
    btn.addEventListener('click', () => {
      const ch = btn.dataset.installChannel === 'beta' ? 'beta' : 'stable';
      const spec = commands[ch];
      codeEl.textContent = spec.cmd;
      copyBtn.dataset.copyText = spec.cmd;
      if (hintEl) hintEl.textContent = spec.hint;
      buttons.forEach((other) => {
        const active = other.dataset.installChannel === ch;
        other.classList.toggle('is-active', active);
        other.setAttribute('aria-pressed', active ? 'true' : 'false');
      });
    });
  });
}

function bindCopyButtons(): void {
  document.querySelectorAll<HTMLButtonElement>('[data-copy-text]').forEach((btn) => {
    btn.addEventListener('click', async () => {
      const text = btn.dataset.copyText;
      if (!text) return;
      try {
        await navigator.clipboard.writeText(text);
        const prev = btn.textContent || 'Copy';
        btn.textContent = 'Copied';
        btn.classList.add('is-copied');
        window.setTimeout(() => {
          btn.textContent = prev;
          btn.classList.remove('is-copied');
        }, 1400);
      } catch {
        btn.textContent = 'Select';
      }
    });
  });
}

async function hydrateLatestVersion(): Promise<void> {
  try {
    const res = await fetch('/latest.json', { cache: 'default' });
    if (!res.ok) return;
    const data = (await res.json()) as { version?: unknown };
    if (typeof data.version !== 'string') return;
    const trimmed = data.version.trim().replace(/^v/, '');
    if (!trimmed) return;
    document.querySelectorAll<HTMLElement>('[data-m3-version]').forEach((el) => {
      el.textContent = `v${trimmed}`;
    });
  } catch {
    // Keep baked fallback version if manifest endpoint is unreachable.
  }
}

function setupRevealObserver(): void {
  const targets = document.querySelectorAll<HTMLElement>('[data-reveal]');
  if (!('IntersectionObserver' in window)) {
    targets.forEach((el) => el.classList.add('is-visible'));
    return;
  }
  const io = new IntersectionObserver(
    (entries) => {
      for (const entry of entries) {
        if (entry.isIntersecting) {
          entry.target.classList.add('is-visible');
          io.unobserve(entry.target);
        }
      }
    },
    { threshold: 0.08 },
  );
  targets.forEach((el) => io.observe(el));
}

const workbenchRoot = document.getElementById('workbench-root');
if (workbenchRoot) {
  mountWorkbench(workbenchRoot);
}

const pipelineRoot = document.getElementById('pipeline-root');
if (pipelineRoot) {
  mountPipeline(pipelineRoot);
}

const cliListRoot = document.getElementById('cli-list');
if (cliListRoot) {
  mountCliDeck(cliListRoot);
}

function bindCasebackEasterEgg(): void {
  const btn = document.getElementById("m3-caseback-btn");
  const note = document.getElementById("m3-caseback-note");
  if (!btn || !note) return;

  const states = [
    { label: "Project M3", note: "", engraved: false },
    { label: "M × M × M", note: "· Compound patiently.", engraved: true },
    {
      label: "Money · Making · Machine",
      note: "· The machine reads 6,000 filings; you swing at fat pitches.",
      engraved: true,
    },
  ];
  let idx = 0;

  btn.addEventListener("click", () => {
    idx = (idx + 1) % states.length;
    const state = states[idx];
    btn.textContent = state.label;
    btn.classList.toggle("is-engraved", state.engraved);
    if (state.note) {
      note.textContent = state.note;
      note.hidden = false;
    } else {
      note.textContent = "";
      note.hidden = true;
    }
  });
}

bindInstallChannelToggle();
bindCasebackEasterEgg();
bindCopyButtons();
setupRevealObserver();
void hydrateLatestVersion();
