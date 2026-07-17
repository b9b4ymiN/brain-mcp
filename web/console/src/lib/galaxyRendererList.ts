/**
 * List Galaxy renderer (E2.2 Part D) — pure-DOM no-GPU fallback.
 *
 * This is the bottom of the fallback chain: works in any browser, including
 * those with no WebGL AND no 2D canvas (rare, but real on some embedded
 * WebViews), and is fully screen-reader accessible. It's also the renderer
 * the E2.3 heap-cycle test can drive WITHOUT needing a real GPU.
 *
 * Anti-XSS: zero `innerHTML`. All text is set via `element.textContent = …`
 * (the DOM text API auto-escapes). `escapeHtml()` is NOT called here —
 * `textContent` already does the right thing, and double-escaping (as the
 * safeText.ts comment warns) would visibly corrupt labels with `&amp;lt;`.
 */

import type { GalaxyPayload } from './api'
import type { GraphRenderer, RendererKind, RendererCallbacks } from './galaxyRendererDetect'

/** Build the DOM-list renderer. */
export function createRendererList(): GraphRenderer {
  let container: HTMLElement | null = null

  return {
    kind: 'list' satisfies RendererKind,

    mount(host: HTMLElement, payload: GalaxyPayload, callbacks: RendererCallbacks): void {
      container = host
      host.replaceChildren()

      // ── nodes ─────────────────────────────────────────────────────────
      const nodesHeading = document.createElement('h3')
      nodesHeading.textContent = `Nodes (${payload.node_count}${
        payload.max_nodes > 0 ? ` / max ${payload.max_nodes}` : ''
      })`
      host.appendChild(nodesHeading)

      if (payload.nodes.length === 0) {
        const empty = document.createElement('p')
        empty.textContent = 'No nodes at this zoom.'
        empty.className = 'galaxy-list-empty'
        host.appendChild(empty)
      } else {
        const nodeList = document.createElement('ul')
        nodeList.className = 'galaxy-list galaxy-list-nodes'
        for (const node of payload.nodes) {
          const li = document.createElement('li')
          li.className = 'galaxy-list-node'
          li.tabIndex = 0
          li.setAttribute('role', 'button')
          // Click + keyboard activation both fire the host callback.
          li.onclick = (): void => callbacks.onNodeClick?.(node)
          li.onkeydown = (event: KeyboardEvent): void => {
            if (event.key === 'Enter' || event.key === ' ') {
              event.preventDefault()
              callbacks.onNodeClick?.(node)
            }
          }
          li.onmouseenter = (): void => callbacks.onNodeHover?.(node)
          li.onmouseleave = (): void => callbacks.onNodeHover?.(null)

          const labelSpan = document.createElement('span')
          labelSpan.className = 'galaxy-list-node-label'
          labelSpan.textContent = node.label
          li.appendChild(labelSpan)

          const metaSpan = document.createElement('span')
          metaSpan.className = 'galaxy-list-node-meta'
          metaSpan.textContent = `${node.kind} · ${node.domain}`
          li.appendChild(metaSpan)

          nodeList.appendChild(li)
        }
        host.appendChild(nodeList)
      }

      // ── edges ─────────────────────────────────────────────────────────
      const edgesHeading = document.createElement('h3')
      edgesHeading.textContent = `Edges (${payload.edges.length})`
      host.appendChild(edgesHeading)

      if (payload.edges.length === 0) {
        const empty = document.createElement('p')
        empty.textContent = 'No edges at this zoom.'
        empty.className = 'galaxy-list-empty'
        host.appendChild(empty)
      } else {
        const edgeList = document.createElement('ul')
        edgeList.className = 'galaxy-list galaxy-list-edges'
        for (const edge of payload.edges) {
          const li = document.createElement('li')
          li.className = 'galaxy-list-edge'
          // Slices of the node id are enough to disambiguate visually; full
          // ids would make each row a wall of hex. Use textContent so the
          // slice can never carry injected markup.
          const src = edge.source.length > 8 ? `${edge.source.slice(0, 8)}…` : edge.source
          const tgt = edge.target.length > 8 ? `${edge.target.slice(0, 8)}…` : edge.target
          li.textContent = `${src} ──${edge.kind}──▶ ${tgt}`
          edgeList.appendChild(li)
        }
        host.appendChild(edgeList)
      }
    },

    destroy(): void {
      if (container !== null) {
        container.replaceChildren()
      }
      container = null
    },
  }
}
