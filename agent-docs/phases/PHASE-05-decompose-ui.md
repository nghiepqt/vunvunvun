# Phase 05 - Decomposition and workspace

Read `../CONTEXT.md` first.

## Goal

Build the feature the product is actually for: turning a task into steps small enough to start, at a granularity the user controls. Then build the workspace that holds it and the settings surface that makes the personalization real.

Estimated three weeks. This is the largest MVP phase.

## Prerequisites

Phase 04 for the router, prompts, and validators. Phase 01 for the task table.

## Design intent

For a stalled user the barrier is almost never the task. It is the first physical movement. Someone who cannot start writing a report can usually open a laptop, and once the laptop is open the next step is smaller than it was. Granularity level 5 exists to walk that gap, and every step at that level must be executable with no decision-making at all.

The second intent is control. The user decides how small the steps get, and the slider is right there. A system that decides for them is one more thing happening to them.

## Scope

### 1. Decomposition

A granularity slider from 1 to 5, with the initial position taken from the user's stored default for that task category. If the defaults are right the slider should rarely need moving.

Level 1 produces three to five coarse steps of roughly an hour. Level 3 produces six to ten steps of ten to thirty minutes. Level 5 produces twelve to twenty-five steps of ten seconds to two minutes each. Step count must land within twenty percent of the target for the chosen level.

Personalization that must be respected:

- If the user requires a physical first step, step one is a body action such as opening the laptop, never a cognitive one such as thinking about structure.
- Plan visibility controls whether the user sees all steps, the next one, or the next three. This is a rendering choice only. The full plan is always generated and always stored, so switching the setting never re-runs the model.
- Invitational demand style changes phrasing from "Open the file" to something like "Opening the file is the next piece". Direct style leaves imperatives alone. This is enforced by the style validator, not left to the model's discretion.
- Step verbosity caps words per step at roughly eight, fifteen, or thirty.
- If interest anchoring is enabled, the model may offer one analogy drawn from the user's stated interests when it flags a task as likely aversive. Off by default, because it can read as patronizing.

Re-splitting any single step is always available and re-runs decomposition scoped to that one step, not the whole task.

Personalization without fine-tuning comes from retrieval: pull the two most similar past decompositions the user accepted and use them as few-shot examples. Their own accepted step style becomes the model's style.

Streaming is required. Render steps as they arrive; first step visible within eight hundred milliseconds, with a skeleton before that.

### 2. Task and step model

Tasks and steps live in one self-referencing table. A step is a task with a parent.

On accept, write rows through the op-log, recording which tier, model, and prompt version produced them. `accepted_at` is set here and only here, by the user's explicit action.

Reordering uses the base-62 fractional index from phase 01. Inserting between two siblings must be one operation, not a renumbering of everything after it.

### 3. Workspace

Spatially stable blocks with fixed positions, because spatial memory is load-bearing for these users:

- Now: exactly one task, the current one, given the most visual weight.
- Today: what is scheduled or chosen for today.
- Inbox: unstructured captures awaiting review.
- Parked: explicitly set aside, with no guilt attached to being there.

Blocks are user-arrangeable but never rearrange themselves, and never reflow between sessions.

The visual system maps directly to stored preferences. Density, color intensity, and motion become resource values consumed by the styles. The operating system reduced-motion setting always wins when it is stricter than the application's.

Color and emoji coding for categories and urgency come from a deterministic rule table, not from a model. Predictability matters more than expressiveness here; the same category must look the same every day. Muted color intensity swaps the palette without ever changing the mapping.

Performance: sixty frames per second while scrolling five hundred items. Virtualize the lists.

### 4. Settings

Every preference in the profile object gets a control. This is not a dumping ground; it is the mechanism by which the product fits users whose needs contradict each other.

Organize by what the user is trying to change, not by internal structure: how the app talks to me, how small the steps get, how it looks and moves, when it may interrupt, and privacy.

Two settings need explanatory text more than the others, because their defaults will look wrong to someone who has not thought about it: countdowns are off because a visible countdown is a demand signal that triggers freeze in many users, and elapsed-time display is separate from countdowns because some users are activated by watching time pass while others panic.

Every change takes effect immediately. No apply button, no restart.

Changing a preference that affects prompts recomputes the profile hash and cluster identifier locally. These are derived values; never merge them from elsewhere.

### 5. Inbox review

Candidates from structuring appear as a review list. Accept, edit then accept, or discard, each in one action. Accepting a task candidate can flow straight into decomposition without a second trip.

## Accessibility requirements

This phase produces most of the application's surface area, so most of the accessibility work lands here.

- Full keyboard operability including the granularity slider, which needs arrow-key adjustment with an announced value.
- Every block, control, and step has an accessible name. Steps announce their position, such as step three of eleven.
- Streaming steps must not spam a screen reader. Use a polite live region and announce completion, not every token.
- Visible focus indicators throughout, with a logical tab order that follows the visual layout.
- Contrast at least 4.5:1 for text, 3:1 for components, in every color intensity setting including muted.
- Test the whole flow with NVDA and Narrator.

## Out of scope

No estimation display; that is phase 06 and the fields stay empty until then. No energy routing, no stall detection, no obstacle dialogue, no weekly review. No retrieval beyond the keyword search from phase 04.

## Definition of done

- Step count lands within twenty percent of target at every granularity level.
- Every step is verb-initial with no compound "and then", verified by the validator in tests.
- At granularity 5 no step exceeds two minutes of estimated work, once phase 06 provides estimates.
- No step references a tool or file absent from the input and retrieved context.
- Latency: first step visible within eight hundred milliseconds, full response p95 under two and a half seconds on the cloud tier.
- Re-splitting a single step affects only that step.
- Every preference in the profile object has a working control that takes effect immediately.
- The workspace holds sixty frames per second with five hundred items.
- Layout is identical across restarts.
- The full flow, capture through accepted steps, is completable with a keyboard and NVDA alone.
