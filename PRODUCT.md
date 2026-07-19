# Product

## Register

product

## Platform

web

## Users

AI/knowledge engineers building and maintaining long-horizon agent systems on top of `brain-mcp`. They live in the console for focused, intensive sessions: reviewing semantic claims queued by extraction workers, searching the event ledger for evidence, and exploring entity timelines to understand what an agent has learned and why. They are technical, fast-moving, and allergic to friction — a missing state, a confusing label, or a dead-end navigation costs them context they will not easily rebuild.

## Product Purpose

Brain Console is the operational surface for a long-term knowledge system built on `brain-mcp`. Its job is to make a folder of Markdown plus an event ledger feel like a queryable, auditable, recoverable brain: review pending proposals before they are confirmed, trace any claim back to its capture event, and explore how concepts and entities relate across the graph. Success looks like an engineer who can drop in mid-session, orient in seconds, and decide with confidence whether to confirm, supersede, or reject a proposed claim — without leaving the console or losing their train of thought.

## Positioning

The only console that treats an agent's memory as a first-class, reviewable artifact rather than an opaque vector store: every claim has provenance, every change is auditable, and the galaxy graph makes the shape of the knowledge visible at a glance.

## Brand Personality

Dark, premium, intentional. Three words: **precise, atmospheric, expert**. The interface should feel like a tool an expert reaches for willingly — confident, quiet, with a sense of depth that rewards attention. Not loud, not decorative, not generic. Cosmic atmosphere is part of the voice (the galaxy metaphor is the product's central idea), but it serves orientation and craft, never spectacle for its own sake.

## Anti-references

- **Generic AI SaaS look** — Inter as a reflex, purple-to-blue gradient hero, identical card grids, hero-metric template, side-stripe accents. The most recognizable "AI made this" tells.
- **Boring enterprise forms** — corporate/banking panels where every surface is a form or a table with no visual character; nothing tells the user this tool is different.
- **2024 glassmorphism cliché** — blurred cards and aurora gradients applied decoratively everywhere. Glass earns its place when it is rare and purposeful; as a default it is the saturated AI scaffold of the last cycle.
- **Gamey neon overload** — sci-fi game UI with saturated neon, constant lens flares, motion that fights the task. Cosmic feel, not game trope.

## Design Principles

- **The graph is the product.** The knowledge graph is not decoration; it is the central artifact. Every screen should make the shape, scale, and relationships of the knowledge visible or one click away.
- **Quiet depth.** Atmosphere and motion should reward attention without demanding it. An engineer deep in a review session should never be interrupted by choreography; ambient detail should sit beneath the task.
- **Earned familiarity over surprise.** Standard affordances done exceptionally well beat invented ones. Consistency screen-to-screen is a virtue; delight is saved for moments, not pages.
- **Provenance is a first-class citizen.** Every claim, every change, every edge should be traceable. The interface teaches trust by showing where things came from.
- **State coverage is the bar.** Loading, empty, error, permission, edge cases — shipping the happy path only is unfinished work. The console is used under load; it must hold up.

## Accessibility & Inclusion

WCAG AA contrast minimums across all states, including placeholder text. Full keyboard navigation for graph and review flows. `prefers-reduced-motion` respected for every animation (galaxy orbit, star twinkle, nebula breathing, panel transitions) with crossfade or instant alternatives. No information conveyed by color alone — node kinds carry shape or label, not just hue. Touch targets meet 44×44 px minimum on mobile.
