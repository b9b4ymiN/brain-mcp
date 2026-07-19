---
name: Brain Console
description: The operational surface for a long-term knowledge system — dark, atmospheric, precise.
colors:
  void: "#05070d"
  ink-deep: "#0a0e17"
  surface-raised: "#11151f"
  surface-elevated: "#161b27"
  ink: "#e8ecf4"
  ink-muted: "#9aa3b2"
  ink-faint: "#5c6675"
  hairline: "#2a3142"
  accent: "#f5b342"
  accent-deep: "#c8861f"
  accent-soft: "rgba(245, 179, 66, 0.14)"
  danger: "#e85a5a"
  danger-soft: "rgba(232, 90, 90, 0.14)"
  success: "#5fbb7a"
  info: "#6aa8d8"
typography:
  display:
    fontFamily: '"Bricolage Grotesque", system-ui, sans-serif'
    fontSize: "clamp(2rem, 4vw, 3rem)"
    fontWeight: 600
    lineHeight: 1.1
    letterSpacing: "-0.02em"
  headline:
    fontFamily: '"Bricolage Grotesque", system-ui, sans-serif'
    fontSize: "1.5rem"
    fontWeight: 600
    lineHeight: 1.25
    letterSpacing: "-0.01em"
  title:
    fontFamily: 'ui-sans-serif, system-ui, sans-serif'
    fontSize: "1.125rem"
    fontWeight: 600
    lineHeight: 1.4
  body:
    fontFamily: 'ui-sans-serif, system-ui, sans-serif'
    fontSize: "0.9375rem"
    fontWeight: 400
    lineHeight: 1.55
  label:
    fontFamily: 'ui-sans-serif, system-ui, sans-serif'
    fontSize: "0.75rem"
    fontWeight: 500
    lineHeight: 1.2
    letterSpacing: "0.04em"
  mono:
    fontFamily: '"JetBrains Mono", ui-monospace, SFMono-Regular, Menlo, monospace'
    fontSize: "0.8125rem"
    fontWeight: 400
    lineHeight: 1.5
rounded:
  sm: "4px"
  md: "8px"
  lg: "12px"
  pill: "9999px"
spacing:
  xs: "4px"
  sm: "8px"
  md: "16px"
  lg: "24px"
  xl: "40px"
  xxl: "64px"
components:
  button-primary:
    backgroundColor: "{colors.accent}"
    textColor: "#05070d"
    rounded: "{rounded.md}"
    padding: "10px 18px"
  button-primary-hover:
    backgroundColor: "{colors.accent-deep}"
  button-ghost:
    backgroundColor: "transparent"
    textColor: "{colors.ink}"
    rounded: "{rounded.md}"
    padding: "10px 18px"
  button-ghost-hover:
    backgroundColor: "rgba(255,255,255,0.06)"
  input:
    backgroundColor: "{colors.ink-deep}"
    textColor: "{colors.ink}"
    rounded: "{rounded.md}"
    padding: "10px 14px"
  nav-link:
    textColor: "{colors.ink-muted}"
    rounded: "{rounded.md}"
    padding: "8px 14px"
  nav-link-active:
    textColor: "{colors.ink}"
    backgroundColor: "rgba(255,255,255,0.06)"
---

# Design System: Brain Console

## 1. Overview

**Creative North Star: "The Observer's Deck"**

Brain Console is the operational surface for a long-term knowledge system — the place an engineer sits to review what an agent has learned, trace a claim to its capture event, and watch the shape of the knowledge move. The metaphor is the observation deck of a deep-space survey vessel: a single darkened console, ambient light kept low, every surface tuned so the graph stays the focal point and the chrome disappears into the task.

The system is **dark by scene, not by default**. The user works in long, focused review sessions under low ambient light, often mid-investigation; a bright UI would fight that state. Darkness is the answer because the physical scene forces it — the operator's eyes are adjusted for the graph, and the chrome should respect that adaptation. Light surfaces are not a "safe" alternative here; they would be the wrong answer to the wrong scene.

What this system explicitly rejects: the generic AI SaaS look (Inter-by-reflex, purple-to-blue gradient hero, identical card grids, hero-metric template, side-stripe accents), boring enterprise forms with no visual character, the 2024 glassmorphism cliché of blurred cards and aurora gradients applied everywhere, and gamey neon overload where saturated colors and constant lens flares fight the task. Cosmic atmosphere is part of the voice — the galaxy metaphor is the product's central idea — but it serves orientation and craft, never spectacle for its own sake.

**Key Characteristics:**
- **Dark scene, light type.** The void (`#05070d`) is the canvas; ink (`#e8ecf4`) carries every word at AA contrast or better.
- **One accent, used sparingly.** Stellar amber (`#f5b342`) is the single saturated color, reserved for primary actions, current selection, and state indicators. The One Voice Rule caps its appearance at ≤10% of any screen.
- **The graph is the product.** Galaxy visuals are first-class content, never decoration. Every screen makes the shape of the knowledge visible or one click away.
- **Quiet depth.** Atmosphere and motion reward attention without demanding it. An engineer mid-review must never be interrupted by choreography.
- **Provenance over polish.** Surfaces are honest about where data came from. Mono type marks machine output; human voice uses the sans.

## 2. Colors: The Deep Survey Palette

A restrained palette anchored on a near-black void, with one warm accent that reads as starlight against the dark. Cool neutrals carry the chrome; amber is the only saturated hue.

### Primary
- **Stellar Amber** (`#f5b342` / `oklch(0.82 0.14 75)`): The single brand accent. Reserved for primary actions (sign in, approve, enter the graph), the currently-selected nav item's marker, focus rings, and active graph-node highlights. Its warmth against the cool void is the entire point — it is the star the eye lands on.
- **Amber Deep** (`#c8861f` / `oklch(0.66 0.14 75)`): Hover/active state of the primary. Same hue, lower lightness, never a different color.

### Neutral
- **Void** (`#05070d` / `oklch(0.13 0.03 270)`): The body background. Near-black with a slight cool tilt toward indigo; reads as deep space, not pure black. Never warm-tinted.
- **Ink Deep** (`#0a0e17`): Default surface for inputs, the galaxy canvas surround, and the bottom of the elevation stack.
- **Surface Raised** (`#11151f`): Cards, panels, and the bottom glass strip on the home hero. One step up from void.
- **Surface Elevated** (`#161b27`): Hovered panels, popovers, side panels. Never a jump — always one notch lighter than its container.
- **Ink** (`#e8ecf4`): Primary text. AA contrast (≥12:1) against void and all surfaces.
- **Ink Muted** (`#9aa3b2`): Secondary text, labels, metadata. Still ≥4.5:1 against void — never the gray-on-tint failure mode.
- **Ink Faint** (`#5c6675`): Placeholders, disabled. Used only where contrast requirements for placeholder text (≥4.5:1) still hold.
- **Hairline** (`#2a3142`): 1px borders, dividers, the outline of inputs at rest. Never a side-stripe.

### Semantic
- **Danger** (`#e85a5a`): Destructive actions, error state. Always paired with a label — never color alone.
- **Success** (`#5fbb7a`): Confirmed claims, completed operations.
- **Info** (`#6aa8d8`): Flash banners, informational state.

### Named Rules
**The One Voice Rule.** Amber occupies ≤10% of any given screen. Its rarity is the point. If amber starts carrying decoration (a hover state here, a divider there), the system has lost its voice — revert.

**The Cool Void Rule.** The body background is the void, full stop. No warm-neutral body backgrounds, no cream/sand/paper, no off-whites. Warmth in this system is carried by the amber accent and the imagery, never by the canvas.

## 3. Typography

**Display Font:** Bricolage Grotesque (fallback: ui-sans-serif, system-ui)
**Body Font:** ui-sans-serif, system-ui (the OS native sans — SF Pro on macOS/iOS, Segoe UI on Windows, Roboto on Android)
**Mono Font:** JetBrains Mono (fallback: ui-monospace, SFMono-Regular, Menlo)

**Character:** Bricolage Grotesque brings genuine character to display — variable optical-size axis, slightly squared curves, the feel of a calibrated instrument label rather than another geometric sans. The detector flags Inter, Space Grotesk, Geist, Roboto, Fraunces, and Plus Jakarta Sans as the overused 2026 set; Bricolage sidesteps that tell while still feeling technically credible. Body deliberately uses the OS native sans instead of a Google Font — on a dark instrument surface, the platform's own type is the distinctive move, and product register explicitly permits familiar system stacks. Mono marks machine output: claim IDs, predicates, ledger sequences, code.

### Hierarchy
- **Display** (Space Grotesk 600, `clamp(2rem, 4vw, 3rem)`, line-height 1.1, letter-spacing -0.02em): Hero surfaces only — the "Brain Console" wordmark, the empty-state headline. Never on data.
- **Headline** (Space Grotesk 600, 1.5rem, line-height 1.25, letter-spacing -0.01em): Page titles, section leads.
- **Title** (Inter 600, 1.125rem, line-height 1.4): Card headers, panel titles, side-panel headings.
- **Body** (Inter 400, 0.9375rem, line-height 1.55): Default text, descriptions, prose. Line length capped at 65–75ch where the layout allows.
- **Label** (Inter 500, 0.75rem, letter-spacing 0.04em): Button labels, nav, table headers, status tags. Sentence case — not the tracked-uppercase eyebrow tell.
- **Mono** (JetBrains Mono 400, 0.8125rem, line-height 1.5): Identifiers, predicates, values, code, anything the machine produced.

### Named Rules
**The Mono-Marks-Machine Rule.** If the text was generated by the system (UUIDs, predicates, JSON values, event sequences), it is mono. If a human wrote it (labels, descriptions, microcopy), it is sans. The typeface tells the reader where the string came from.

**The No-Eyebrow Rule.** No tracked-uppercase eyebrows above every section. Section cadence is carried by headline size and spacing rhythm, not by a small all-caps kicker repeating across the page.

## 4. Elevation

Flat by default; depth is conveyed by surface lightness, not shadow. The elevation stack is the four-step neutral ramp (void → ink-deep → surface-raised → surface-elevated); moving up the stack reads as moving closer to the user. Shadows appear only where a surface genuinely lifts off the page — the bottom glass panel on the home hero, dialogs, popovers — and even there they are diffuse and low-opacity, never crisp or heavy.

### Shadow Vocabulary
- **Lift** (`box-shadow: 0 8px 32px rgba(0,0,0,0.5)`): Dialogs, the home hero's bottom panel. Diffuse, deep, never crisp.
- **Hover Glow** (`box-shadow: 0 0 0 1px var(--accent), 0 0 24px rgba(245,179,66,0.18)`): Reserved for the currently-focused primary action and the active graph node. Amber-tinted; rare.

### Named Rules
**The Flat-By-Default Rule.** Surfaces are flat at rest. Shadows appear only as a response to state (lifted dialogs, hover, focus) or where a panel genuinely floats over the galaxy canvas. A card at rest with a shadow is the SaaS cliché — refuse it.

## 5. Components

### Buttons
- **Shape:** Soft rectangle (`{rounded.md}`, 8px).
- **Primary:** Amber background (`#f5b342`), void text (`#05070d`), padding `10px 18px`, Inter 500 label-case. The amber-to-ink contrast is ≥7:1 — the highest-contrast element on the page, by design.
- **Hover / Focus:** Amber Deep background on hover; Hover Glow shadow on focus-visible. Never both at once.
- **Ghost / Secondary:** Transparent background, ink text, hairline border. Hover bumps the background to `rgba(255,255,255,0.06)`. Used for cancel, dismiss, and secondary actions.

### Inputs / Fields
- **Style:** Ink-deep background, ink text, 1px hairline border, `{rounded.md}`.
- **Focus:** Border swaps to amber, no glow ring (the border change is enough; a ring would be visual noise).
- **Placeholder:** Ink-faint, never the muted-gray-on-tint failure. ≥4.5:1 contrast enforced.
- **Error:** Border swaps to danger; helper text in danger below.

### Navigation
- **Top overlay on home:** Transparent over the galaxy canvas, ink-muted labels, hairline divider on scroll.
- **Side / top on other pages:** Void background, ink-muted default, ink + subtle raised-surface pill on active.
- **Active state:** Ink text + `{rgba(255,255,255,0.06)}` background pill; never an amber underline or side-stripe.

### Galaxy Canvas (signature component)
- **Background:** Pure void with the SpaceBackdrop layered behind (stars + nebula radial gradients).
- **Nodes:** Default ink-muted dots; hovered node swells and glows amber; selected node carries the amber ring.
- **Edges:** Hairline at low opacity; brighten toward ink on hover of connected nodes.
- **Side panel:** Surface-raised, headline + mono ID + label-case metadata, ghost "Open as entity" button.

### Flash Banners
- **Style:** Full-width strip, surface-raised, 1px hairline top border, semantic background at 14% opacity.
- **Dismissible:** Ghost × button right-aligned; aria-label "Dismiss".

### StateBox (loading / empty / error)
- **Loading:** Skeleton pulse on surface-raised shapes, never a centered spinner.
- **Empty:** Display headline + body explainer + primary CTA. Teaches the interface; never "nothing here."
- **Error:** Danger-tinted panel + retry ghost button. Specific copy naming the failure, never generic "Something went wrong."

## 6. Do's and Don'ts

### Do
- **Do** use OKLCH for every color decision. Hex values in this document are the sRGB projection for tooling; the OKLCH in parens is canonical.
- **Do** keep amber ≤10% of any screen (The One Voice Rule). It is the star, not the wallpaper.
- **Do** mark machine output (UUIDs, predicates, values, code) with JetBrains Mono. The typeface tells the reader where the string came from.
- **Do** respect `prefers-reduced-motion`: every galaxy orbit, star twinkle, nebula breathe, and panel transition needs an instant or crossfade alternative.
- **Do** cover every state (loading, empty, error, permission, hover, focus, active, disabled). Shipping the happy path only is unfinished work.
- **Do** use the four-step elevation ramp for depth, not shadows. Flat-by-default.

### Don't
- **Don't** use `border-left` or `border-right` greater than 1px as a colored accent on cards, list items, callouts, or alerts (the side-stripe ban).
- **Don't** use `background-clip: text` with a gradient (gradient text). Single solid color; emphasis via weight or size.
- **Don't** apply blur/backdrop-filter decoratively across the surface. Glass earns its place on the home hero panel and dialogs — nowhere else.
- **Don't** ship the hero-metric template (big number, small label, supporting stats, gradient accent). It is the SaaS cliché.
- **Don't** repeat identical icon-heading-text cards in a grid. Cards are the lazy answer.
- **Don't** put a tracked-uppercase eyebrow above every section (the saturated AI scaffold). Choose a different cadence.
- **Don't** use Inter as a reflex for display headings. Space Grotesk is the display voice; Inter is body and UI.
- **Don't** warm-tint the body background. The Cool Void Rule: the canvas is the cool void, always.
- **Don't** convey state by color alone. Node kinds, claim status, and danger actions always carry a label or shape.
