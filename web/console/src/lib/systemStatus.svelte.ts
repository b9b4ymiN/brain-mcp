/**
 * systemStatus — a tiny global store for the cockpit HUD readout.
 *
 * Home's galaxy read updates `nodeCount` / `edgeCount` after each
 * successful `/galaxy` fetch; App.svelte's HudFrame + the cockpit
 * footer read them. Single source of truth, no prop-drilling through
 * the page tree.
 *
 * `dotVariant` flips to 'alert' when any page reports an error via
 * `setError`, 'warning' while a fetch is in flight, 'nominal' on
 * success. The HudFrame status dot pulses at the matching cadence.
 */
import { writable } from 'svelte/store'

export type DotVariant = 'nominal' | 'warning' | 'alert'

export interface SystemStatus {
  nodeCount: number
  edgeCount: number
  /** Connection state — drives the status dot. */
  dotVariant: DotVariant
  /** Short status line for the HudFrame readout (mono). */
  statusLine: string
}

export const systemStatus = writable<SystemStatus>({
  nodeCount: 0,
  edgeCount: 0,
  dotVariant: 'nominal',
  statusLine: 'BOOTING · STANDBY',
})

/** Update counts after a successful galaxy read. */
export function setGalaxyCounts(nodes: number, edges: number): void {
  systemStatus.update((s) => ({
    ...s,
    nodeCount: nodes,
    edgeCount: edges,
    dotVariant: 'nominal',
    statusLine: `SYSTEMS NOMINAL · NODES ${nodes} · EDGES ${edges} · SYNC ✓`,
  }))
}

/** Mark a fetch in flight (warning). */
export function setBusy(): void {
  systemStatus.update((s) => ({ ...s, dotVariant: 'warning', statusLine: 'SCANNING…' }))
}

/** Mark a connection failure (alert). */
export function setError(message: string): void {
  systemStatus.update((s) => ({
    ...s,
    dotVariant: 'alert',
    statusLine: `SIGNAL LOST · ${message.toUpperCase()}`,
  }))
}
