---
name: Hacienda Studio
description: Redact sensitive documents without letting them leave your laptop.
colors:
  background: "hsl(222 22% 7%)"
  foreground: "hsl(210 20% 96%)"
  card: "hsl(222 20% 10%)"
  card-foreground: "hsl(210 20% 96%)"
  primary: "hsl(28 92% 54%)"
  primary-foreground: "hsl(20 40% 10%)"
  secondary: "hsl(222 15% 16%)"
  secondary-foreground: "hsl(210 20% 96%)"
  muted: "hsl(222 15% 14%)"
  muted-foreground: "hsl(215 12% 62%)"
  accent: "hsl(222 15% 17%)"
  accent-foreground: "hsl(210 20% 96%)"
  destructive: "hsl(0 72% 55%)"
  destructive-foreground: "hsl(210 20% 96%)"
  border: "hsl(222 15% 18%)"
  emerald-success: "oklch(69.6% 0.17 162.48)"
typography:
  display:
    fontFamily: "-apple-system, BlinkMacSystemFont, 'Segoe UI', Helvetica, Arial, sans-serif"
    fontSize: "clamp(2.5rem, 5vw, 3.375rem)"
    fontWeight: 600
    lineHeight: 1.1
  body:
    fontFamily: "-apple-system, BlinkMacSystemFont, 'Segoe UI', Helvetica, Arial, sans-serif"
    fontSize: "1rem"
    fontWeight: 400
    lineHeight: 1.5
  mono:
    fontFamily: "'SF Mono', 'Fira Code', 'Fira Mono', Menlo, Consolas, monospace"
    fontSize: "0.6875rem"
    fontWeight: 400
rounded:
  sm: "4px"
  md: "6px"
  lg: "8px"
  full: "9999px"
spacing:
  xs: "4px"
  sm: "8px"
  md: "16px"
  lg: "24px"
  xl: "40px"
components:
  button-primary:
    backgroundColor: "{colors.primary}"
    textColor: "{colors.primary-foreground}"
    rounded: "{rounded.md}"
    padding: "8px 16px"
  button-outline:
    backgroundColor: "{colors.background}"
    textColor: "{colors.foreground}"
    rounded: "{rounded.md}"
    padding: "8px 16px"
  card:
    backgroundColor: "{colors.card}"
    textColor: "{colors.card-foreground}"
    rounded: "{rounded.lg}"
    padding: "16px"
  badge-outline:
    backgroundColor: "{colors.background}"
    textColor: "{colors.muted-foreground}"
    rounded: "{rounded.full}"
    padding: "2px 8px"
---

# Design System: Hacienda Studio

## Overview

**Creative North Star: "The Locked Terminal"**

Hacienda Studio looks like a piece of trusted infrastructure, not a marketing surface — closer to a well-built ops console or a security appliance's admin panel than a SaaS dashboard. The palette is a single near-black charcoal-navy with one warm, unapologetic accent: a terracotta orange that reads as "trust this, act on this," never decorative. Everything else — cards, borders, muted text — sits in a narrow band of desaturated cool grays that never compete with that accent.

The interface earns its calm by being honest about what's happening: pipeline stages render as literal monospace tokens (`extract → ner → pii → redact`), findings show real confidence percentages, and the audit tab prints raw hash values rather than a friendly checkmark. This is a tool for people who need to trust it, not be charmed by it — but it isn't cold: rounded corners (8px), generous line-height, and a warm accent keep it from reading as a bare terminal.

**Key Characteristics:**
- Single accent color, used sparingly — primary actions, active state, and nothing else
- Monospace for anything that is literally a system token (pipeline stages, hashes, findings ids)
- Flat, bordered surfaces — no shadows, depth comes from a one-step lightness shift between background and card
- Generous whitespace on marketing surfaces (Landing), dense and functional on tool surfaces (Studio, Documents, DocumentDetail)

## Colors

A near-monochrome charcoal-navy system with one accent color carrying all "act on this" signal.

### Primary
- **Terracotta orange** (`hsl(28 92% 54%)`, ~#f6821e): the only saturated color in the system. Used for primary buttons, the active redaction-mode toggle, active tab/nav state, and icon accents on the Landing feature cards. Never used for large fills or backgrounds — always a small, deliberate mark.

### Neutral
- **Void charcoal** (`hsl(222 22% 7%)`): page background.
- **Raised charcoal** (`hsl(222 20% 10%)`): card/panel surface — one lightness step above the page, the system's only elevation cue.
- **Near-white** (`hsl(210 20% 96%)`): primary text.
- **Cool slate** (`hsl(215 12% 62%)`): secondary/muted text — labels, helper copy, timestamps.
- **Hairline border** (`hsl(222 15% 18%)`): the only border color in the system; every card, input, and divider uses it at 1px.

### Named Rules
**The One Accent Rule.** Terracotta orange appears on buttons/active-states/small icon marks only — never as a background fill, never on more than one element's "primary" state at a time. Its rarity is what makes it read as action, not decoration.

## Typography

**Display/Body Font:** -apple-system / Segoe UI stack (system font, no webfont load).
**Mono Font:** SF Mono / Fira Code stack, used exclusively for system-literal values.

**Character:** A plain system sans for everything a human reads, switching hard to monospace the instant the content is a machine-produced token (a pipeline stage, a hash, a byte count) — the font change itself is the signal that "this value is exact, not prose."

### Hierarchy
- **Display** (600 weight, `clamp(2.5rem, 5vw, 3.375rem)`, 1.1 line-height): Landing hero headline only.
- **Title** (600 weight, 1.5rem): page titles ("Documents", "Add documents", "Prepare the workspace").
- **Body** (400 weight, 1rem, 1.5 line-height): all prose and UI copy.
- **Label** (500 weight, 0.75rem, muted-foreground): field labels, section headers inside a tab.
- **Mono/System** (400 weight, 0.6875rem, muted or emerald/primary by state): pipeline stage pills, hash values, byte sizes, code panels.

### Named Rules
**The Mono-Means-Machine Rule.** If a value was produced by the pipeline rather than typed by a human (a stage name, a hash, an entity offset), it renders in the mono stack. Prose never does, no matter how technical the sentence.

## Layout

Landing is generously spaced (24–40px section padding, wide measure, centered narrow columns for hero copy) because it is the one surface meant to be read, not scanned. Every tool surface (Assets, Studio, Documents, DocumentDetail) switches to a dense, left-aligned, full-width layout: a slim header bar (border-bottom, 12px vertical padding), then content that runs edge-to-edge in a two-column split where relevant (Documents' folder sidebar + list; DocumentDetail's tab content). Tool surfaces never center content in a narrow column — the point is to see everything at once.

## Elevation & Depth

Flat by default. There are no shadows anywhere in the system; every surface is a flat fill differentiated from its neighbor by exactly one step of lightness (background → card) and a 1px hairline border. Depth is implied by layering (a popover or dialog sits on `card` over `background`), never simulated with blur or shadow.

### Named Rules
**The Flat-By-Default Rule.** A new surface never gets a shadow to "lift" it. If it needs to read as elevated, give it the card background + border combination instead.

## Shapes

Corners are consistently rounded at a small, restrained radius (`--radius: 0.5rem` / 8px for large containers, scaling down to 4-6px for nested elements via `calc(var(--radius) - 2px|4px)`). Pills (redaction-mode toggle, badges, stage indicators) go fully round (`rounded-full`). Nothing is sharp-cornered; nothing is heavily rounded either — the radius reads as "considered," not "playful."

## Components

### Buttons
- **Shape:** 6px radius (`rounded-md`), 1px border on `outline`/`secondary` variants, none on `default`/`ghost`.
- **Primary (`default`):** terracotta background, near-black text (`primary-foreground`) for contrast — the only button that pairs a saturated fill with dark-on-light text; every other surface in the system is light-on-dark.
- **Outline:** transparent/background fill, hairline border, foreground text — used for secondary actions sitting next to a primary button ("Skip to the studio" beside "Prepare the workspace").
- **Hover:** `default` darkens toward `primary/90`; `outline`/`ghost` pick up the `accent` fill.
- **Active redaction-mode state:** the selected mode button (Mask/Hash/Pseudonymize/Remove) switches to the `default` (filled) variant; the other three stay `outline` — state is shown by variant swap, not a separate "selected" style.

### Cards / Containers
- **Corner style:** 8px radius (`rounded-lg`).
- **Background:** `card` (one step lighter than page background).
- **Shadow strategy:** none — see Elevation & Depth.
- **Border:** 1px hairline (`border`) on every card; this is the system's only depth cue, so it is never omitted.
- **Internal padding:** 16-24px depending on density (Landing feature cards: 24px; Studio queue rows: 16px).

### Badges / Pills
- **Style:** fully rounded (`rounded-full`), hairline border, no fill — text color carries the meaning (muted for neutral, emerald for "done"/"verified", primary-outline for "active").
- **Pipeline stage pills** (extract/ner/pii/redact) are a distinct sub-variant: square-ish (`rounded-md`), monospace text, border color shifts from `border` (pending) → `primary` (active) → `emerald-500/40` (done).

### Inputs / Fields
- **Style:** `background` fill, hairline border, 6px radius.
- **Focus:** border shifts to `primary`, plus a soft ring in `primary/50`.

### Tabs
- **Style:** underline-free segmented control — inactive tabs sit in a muted background pill, the active tab lifts to `background` fill with `foreground` text. No color accent on the active tab itself; the lift is the only signal.

### Toasts (sonner)
- **Style:** `card` background, hairline border, 8px radius — visually identical to a small card, appearing bottom-right, auto-dismissing.

## Do's and Don'ts

### Do:
- **Do** keep terracotta orange to a single active element per view — one primary action, one active tab, one selected redaction mode.
- **Do** render any pipeline-produced value (hash, stage name, byte size, offset) in the mono stack.
- **Do** use the `card` + hairline-border combination for anything that needs to read as a distinct surface — never a shadow.
- **Do** keep tool surfaces (Studio, Documents, DocumentDetail) dense and edge-to-edge; reserve generous whitespace for Landing.

### Don't:
- **Don't** add a second saturated accent color — every non-primary surface stays in the charcoal/gray range.
- **Don't** add shadows, blur, or glassmorphism anywhere — the system is flat by declared rule.
- **Don't** use the mono font for anything a human wrote (copy, labels, error messages) — it's reserved for machine-produced values.
- **Don't** center tool-surface content in a narrow column the way Landing does — tool surfaces run full-width.
