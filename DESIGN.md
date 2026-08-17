# DESIGN.md — NBA Sim Spectator Shell

## 0. Research Log

| Lane | Deliverable |
|---|---|
| Product surface | Broadcast desk + top-down court: dark arena, amber accents, monospace clocks |
| Reference mood | Sports broadcast HUD (ESPN/NBA TV density) + Linear/Stripe operational calm |
| Constraints | Kernel is zone-label only; 2D is pure projection, no physics |

## 1. Tokens

| Token | Value | Use |
|---|---|---|
| `--bg` | `#0b0f14` | Page background |
| `--panel` | `#121821` | Cards / HUD panels |
| `--panel-border` | `#1e2a38` | Borders |
| `--ink` | `#e8eef6` | Primary text |
| `--muted` | `#8b9bb0` | Secondary text |
| `--home` | `#3d8bfd` | Home team |
| `--away` | `#f07178` | Away team |
| `--ball` | `#f0c14b` | Ball / accents |
| `--court` | `#1a3a2a` | Court fill |
| `--line` | `#c8d5c0` | Court lines |
| `--danger` | `#ff6b6b` | High-intensity narration |
| `--mono` | `"IBM Plex Mono", "SF Mono", ui-monospace, monospace` | Clocks / scores |
| `--sans` | `"IBM Plex Sans", "Segoe UI", system-ui, sans-serif` | UI copy |
| `--radius` | `10px` | Panels |
| `--gap` | `12px` | Layout gap |

## 2. Typography

- Score: mono 28–36px weight 600
- Clock: mono 14–16px
- Narration: sans 14px; high intensity uses `--danger`
- Labels: sans 11–12px uppercase tracking `0.06em` muted

## 3. Layout

- Desktop (≥1024): left court canvas (flex 1), right column stack (score + controls + log)
- Mobile: court on top (aspect ~1.9), controls + log below
- Court aspect: full court ≈ 94×50 → CSS aspect-ratio `1.88 / 1`

## 4. Primitives

- **CourtCanvas**: draws floor, paint, arcs, players as circles, ball, optional trail
- **Scoreboard**: period, game clock, shot clock, score
- **Transport**: play/pause, speed (0.5× / 1× / 2× / 4× / 16×), scrub slider
- **PlayByPlay**: scrollable narrated lines; auto-follow current frame

## 5. Motion

- GPU only: `transform` / `opacity` for player interpolation between frames
- Scrubbing is discrete by event index; optional short lerp (≤120ms) between frames when playing
- No decorative hover motion on non-controls

## 6. Accessibility

- Keyboard: Space play/pause, ←/→ step event, 1–5 speed
- Contrast: score/clock ≥ WCAG AA on dark panels
- Reduced motion: disable inter-frame lerp

## 7. Accepted debt

- Positions come from authoritative `JUMP_CIRCLE_ALIGN` / `ALIGNMENT` events (foundation 0.2.0+); continuous tracking still not simulated between events
- No audio, no 3D, no live multiplayer
- Roster display names optional (`Player.label` not in kernel)
- Off-ball tasks refresh on ball-moving steps (pass/handoff/drive/screen), not every micro-cut

## 8. Reference fidelity

- Not cloning a brand pixel-for-pixel; tokens above are the contract
