import { PIPELINE_LAYERS } from '../data/fixture';

function escapeHtml(raw: string): string {
  return raw
    .replaceAll('&', '&amp;')
    .replaceAll('<', '&lt;')
    .replaceAll('>', '&gt;')
    .replaceAll('"', '&quot;');
}

export function mountPipeline(container: HTMLElement): void {
  let activeId = PIPELINE_LAYERS[0].id;

  function render(): void {
    const active = PIPELINE_LAYERS.find((l) => l.id === activeId) || PIPELINE_LAYERS[0];

    container.innerHTML = `
      <div class="pipeline-rail" role="tablist" aria-label="M3 Four-Stage Method">
        ${PIPELINE_LAYERS.map(
          (layer) => `
          <button
            type="button"
            role="tab"
            aria-selected="${layer.id === active.id ? 'true' : 'false'}"
            class="pipeline-node ${layer.id === active.id ? 'is-active' : ''}"
            data-pipeline-id="${layer.id}"
          >
            <span class="node-tag">${escapeHtml(layer.tag)}</span>
            <span class="node-title">${escapeHtml(layer.title)}</span>
            <span class="node-stat">${escapeHtml(layer.stat)}</span>
          </button>
        `,
        ).join('')}
      </div>

      <div class="pipeline-stage" role="tabpanel">
        <div>
          <h3>${escapeHtml(active.heading)}</h3>
          <p>${escapeHtml(active.summary)}</p>
          <ul class="pipeline-bullets">
            ${active.bullets.map((b) => `<li>${escapeHtml(b)}</li>`).join('')}
          </ul>
        </div>

        <div class="pipeline-spec-box">
          <div>
            <span class="kv-label">Mental Model</span>
            <div style="font-size: 0.9rem; color: var(--ink-hero); margin-top: 0.3rem; line-height: 1.5;">
              ${escapeHtml(active.principle)}
            </div>
          </div>

          <div>
            <span class="kv-label">Operating Discipline</span>
            <div class="mono" style="font-size: 0.79rem; color: var(--gold); margin-top: 0.3rem;">
              ${escapeHtml(active.cadence)}
            </div>
          </div>

          <div>
            <span class="kv-label">Quiet CLI Entrypoint</span>
            <pre class="cmd-snippet" style="margin: 0.35rem 0 0;"><code>${escapeHtml(active.command)}</code></pre>
          </div>
        </div>
      </div>
    `;

    container.querySelectorAll<HTMLElement>('[data-pipeline-id]').forEach((btn) => {
      btn.addEventListener('click', () => {
        const nextId = btn.dataset.pipelineId;
        if (nextId && nextId !== activeId) {
          activeId = nextId;
          render();
        }
      });
    });
  }

  render();
}
