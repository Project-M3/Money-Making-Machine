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

bindCopyButtons();
setupRevealObserver();
