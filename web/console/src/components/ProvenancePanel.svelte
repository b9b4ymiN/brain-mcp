<script lang="ts">
  /**
   * ProvenancePanel — the four core provenance questions (Task E3.3 Part E).
   *
   * §5.3 requires the Console to answer four questions for any subject:
   *   - What     — predicate + value of the claims we hold.
   *   - Source   — provenance_kind (+ evidence excerpt where available).
   *   - When     — valid_from / valid_to interval from the timeline.
   *   - Connections — galaxy neighbours (close zoom) + clients that edited.
   *
   * Plus the §5.3 audit facet "client that edited" (TM-024: client/channel,
   * NOT person inference).
   *
   * This is a READ-ONLY synthesis panel — it combines data already fetched
   * by Entity.svelte (claims + timeline + clients). No new API surface. The
   * data shapes are:
   *   - `claims`        — `SubjectClaim[]` from `/get`.
   *   - `timelineByPredicate` — `Record<predicate, ClaimView[]>` from
   *     `/entity/timeline`. The parent assembles this map so we don't refetch.
   *   - `clientLabelById` — `Record<client_id, label>` from `/ops/clients`.
   *
   * Anti-XSS: every dynamic string (predicate, value, provenance_kind,
   * client label, dates) is bound via Svelte text binding. No `{@html}`.
   */
  import type { ClaimView, SubjectClaim, ClientActivity } from '../lib/api'
  import { formatValue, formatDate } from '../lib/format'

  interface Props {
    /** The subject these answers are about (for the header). */
    subject: string
    /** `/get` claims for the subject — drives the "What" facet. */
    claims: SubjectClaim[]
    /**
     * Timeline entries keyed by predicate — drives the "When" facet. The
     * parent builds this map from `/entity/timeline` results it already
     * fetched for row expansion.
     */
    timelineByPredicate: Record<string, ClaimView[]>
    /** `/ops/clients` — drives the "Client that edited" facet. */
    clients: ClientActivity[]
  }

  let { subject, claims, timelineByPredicate, clients }: Props = $props()

  // ── What ────────────────────────────────────────────────────────────────
  // Group claims by predicate (a subject may have many predicates). Each
  // entry renders as "predicate = value" so the panel answers "what do we
  // know about X?" without requiring the user to expand the table rows.
  //
  // Entity Identity Reform: after consolidation, a subject may carry the
  // SAME predicate under different domain tags (e.g. CATL has "customers" in
  // business, Business, and Finance). We dedup by predicate — keeping the
  // highest-confidence claim — so the {#each ... (w.predicate)} key is unique.
  let whats = $derived.by(() => {
    const byPred = new Map<string, { predicate: string; value: unknown; confidence: number }>()
    for (const c of claims) {
      const existing = byPred.get(c.predicate)
      if (!existing || c.confidence > existing.confidence) {
        byPred.set(c.predicate, {
          predicate: c.predicate,
          value: c.value,
          confidence: c.confidence,
        })
      }
    }
    return Array.from(byPred.values())
  })

  // ── Source ──────────────────────────────────────────────────────────────
  // Distinct provenance kinds across the claims. The "From: <kind>" line is
  // the audit facet — we don't synthesise a misleading single source.
  let sources = $derived(
    Array.from(new Set(claims.map((c) => c.provenance))).filter(
      (s) => s.length > 0,
    ),
  )

  // ── When ────────────────────────────────────────────────────────────────
  // For each predicate, find the still-current claim (status === 'confirmed'
  // in the timeline) and surface its valid_from / valid_to. Claims without a
  // timeline entry fall back to "(current scope — no time bounds)".
  interface WhenRow {
    predicate: string
    valid_from: string | null
    valid_to: string | null
  }
  let whens = $derived.by<WhenRow[]>(() => {
    // Entity Identity Reform: dedup by predicate — same predicate under
    // different domain tags collapses to one row (matching `whats` above).
    const byPred = new Map<string, WhenRow>()
    for (const claim of claims) {
      if (byPred.has(claim.predicate)) continue
      const entries = timelineByPredicate[claim.predicate] ?? []
      const confirmed = entries.find((e) => e.status === 'confirmed')
      byPred.set(claim.predicate, {
        predicate: claim.predicate,
        valid_from: confirmed?.valid_from ?? null,
        valid_to: confirmed?.valid_to ?? null,
      })
    }
    return Array.from(byPred.values())
  })

  // ── Connections + client that edited ────────────────────────────────────
  // Galaxy neighbours require a fetch the parent doesn't always do; we
  // surface the client-edit audit facet from `/ops/clients` instead. The
  // "edited by" line is the audit answer §5.3 demands (TM-024 — client_id +
  // label only, never person identity).
  // If the parent passes ZERO clients (panel rendered before ops page
  // loaded), show a neutral hint rather than an empty list.
  let clientRows = $derived(
    clients.map((c) => ({
      label: c.label,
      client_id: c.client_id,
      mutations: c.mutation_count,
      last_active_at: c.last_active_at,
    })),
  )
</script>

<section class="provenance-panel" aria-label={`Provenance for ${subject}`}>
  <h3>Provenance — {subject}</h3>

  <div class="grid">
    <div class="facet">
      <h4>What</h4>
      {#if whats.length === 0}
        <p class="empty">No claims held for this subject.</p>
      {:else}
        <ul class="facet-list">
          {#each whats as w (w.predicate)}
            <li>
              <span class="mono">{w.predicate}</span>
              =
              <span class="value">{formatValue(w.value)}</span>
              <span class="meta">({Math.round(w.confidence * 100)}%)</span>
            </li>
          {/each}
        </ul>
      {/if}
    </div>

    <div class="facet">
      <h4>Source</h4>
      {#if sources.length === 0}
        <p class="empty">No provenance recorded.</p>
      {:else}
        <p class="facet-line">
          From: {sources.join(', ')}
        </p>
        <p class="hint">
          Provenance kind is the ADR Decision 6 variant (evidence / inference
          / user_assertion / mechanical). Open the claim row for the evidence
          excerpt.
        </p>
      {/if}
    </div>

    <div class="facet">
      <h4>When true</h4>
      {#if whens.length === 0}
        <p class="empty">No time bounds available.</p>
      {:else}
        <ul class="facet-list">
          {#each whens as w (w.predicate)}
            <li>
              <span class="mono">{w.predicate}</span>:
              {#if w.valid_from || w.valid_to}
                {formatDate(w.valid_from)} → {formatDate(w.valid_to)}
              {:else}
                <span class="hint">current scope — no time bounds</span>
              {/if}
            </li>
          {/each}
        </ul>
      {/if}
    </div>

    <div class="facet">
      <h4>Connections</h4>
      <p class="facet-line">
        Open the Galaxy view (Entity → Galaxy) for the close-zoom
        neighbourhood; this panel surfaces the client-edit audit facet below.
      </p>
    </div>

    <div class="facet facet-clients">
      <h4>Client that edited</h4>
      {#if clientRows.length === 0}
        <p class="empty">
          Client activity unavailable here — open the Operations page to load
          `/ops/clients`.
        </p>
      {:else}
        <ul class="facet-list">
          {#each clientRows as c (c.client_id)}
            <li>
              <span class="label">{c.label}</span>
              <span class="meta">({c.mutations} mutation{c.mutations === 1 ? '' : 's'})</span>
              <span class="hint">last active {formatDate(c.last_active_at)}</span>
            </li>
          {/each}
        </ul>
        <p class="hint">
          Audit only — channel/client, not person identity (TM-024).
        </p>
      {/if}
    </div>
  </div>
</section>

<style>
  .provenance-panel {
    margin: 1rem 0;
    padding: 0.85rem 1rem;
    border-radius: 0.5rem;
    border: 1px solid rgba(127, 127, 127, 0.35);
    background: rgba(127, 127, 127, 0.04);
  }

  .provenance-panel h3 {
    margin: 0 0 0.6rem;
    font-size: 1rem;
    word-break: break-word;
  }

  .grid {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(14rem, 1fr));
    gap: 0.6rem;
  }

  .facet {
    padding: 0.5rem 0.65rem;
    border-radius: 0.4rem;
    background: rgba(127, 127, 127, 0.06);
    font-size: 0.88rem;
  }

  .facet h4 {
    margin: 0 0 0.4rem;
    font-size: 0.78rem;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    opacity: 0.7;
  }

  .facet-line {
    margin: 0 0 0.25rem;
    word-break: break-word;
  }

  .facet-list {
    list-style: none;
    margin: 0;
    padding: 0;
    display: grid;
    gap: 0.25rem;
  }

  .facet-list li {
    word-break: break-word;
  }

  .empty {
    margin: 0.2rem 0;
    font-style: italic;
    opacity: 0.65;
  }

  .hint {
    display: block;
    margin: 0.2rem 0 0;
    font-size: 0.78rem;
    opacity: 0.65;
    font-style: italic;
  }

  .meta {
    opacity: 0.7;
    font-size: 0.82rem;
  }

  .label {
    font-weight: 600;
  }

  .value {
    font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
    font-size: 0.85em;
  }

  .mono {
    font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
    font-size: 0.85em;
  }
</style>
