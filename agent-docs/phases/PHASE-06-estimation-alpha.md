# Phase 06 - Estimation and alpha release

Read `../CONTEXT.md` first.

## Goal

Add duration estimation, which is the last MVP feature, then get the thing into real users' hands. Everything here is either the estimator or the work of shipping.

Estimated two weeks.

## Prerequisites

Phase 05, because estimation needs tasks and steps to estimate and a workspace to display in.

## Part one: duration estimation

### Why this is not a model call

Language models are unreliable at numeric estimation, and a hallucinated deadline is the fastest possible way to destroy trust in a tool whose entire promise is that the user can stop holding things in their head. The language model may label a step. It may never time one.

This is a small gradient-boosted regression model, exported to ONNX, running in-process through Microsoft.ML.OnnxRuntime. No network, no language model, under five milliseconds.

### The model

Quantile regression with three heads at the fiftieth, eightieth, and ninetieth percentiles, so the user's buffer preference selects a real trained quantile rather than applying a heuristic multiplier to a point estimate.

Features: task category as one-hot, step word count, whether the step is a physical action, position in the plan, total steps in the plan, hour of day, day of week, energy level at start once that exists, the user's exponentially weighted moving average of actual over estimated duration for this category, and the count of prior completions in this category.

Ship it as a population-level artifact trained on whatever data is available, including synthetic and hand-labelled examples. It does not need to be good at launch; it needs to exist and be calibrated honestly.

### Personalization without training

OnnxRuntime is inference-only and cannot train. Personalization therefore does not happen by retraining on the user's machine. It happens three ways:

1. Personal features fed into the frozen model, notably the moving average of their own estimation bias and their completion counts.
2. Empirical Bayes blending of the frozen population prediction with a running personal statistic. Weight personal by n over n plus k, with k around fifteen, so a new user sees population priors and someone with roughly fifteen completions in a category sees mostly their own history.
3. An additive per-user residual correction updated by quantile stochastic gradient descent after each completion. Plain C#, no machine learning runtime, stored in the device-local model state table.

Do not write a ticket that assumes on-device retraining. Nothing in this stack can do it.

### Display

Always a range, never a point. The user's buffer preference selects which quantile is shown.

Countdowns default off, because a visible countdown is a demand signal. Elapsed time is a separate toggle, also off by default, because users split on whether watching time pass activates them or panics them.

Concrete anchors such as "about as long as making coffee" are allowed only when the user's language mode is natural. Under literal mode they are noise at best and confusing at worst.

### Calibration monitoring

Track empirical coverage of the eightieth-percentile interval. It should sit between seventy-five and eighty-five percent. Surface this in a settings or debug readout in this phase; the weekly review that would normally carry it is post-MVP.

If coverage drifts outside the band, the interval is lying to the user. Widen or narrow it and say so.

## Part two: alpha release

### Packaging

Produce both channels from CI:

- The MSIX package for the Store, which is the primary channel. Microsoft signs it after certification, so no certificate is needed and users see no warnings.
- A self-contained unpackaged installer for direct download, signed with an organization-validated certificate. This channel will show SmartScreen warnings until reputation accrues; that is expected and cannot be shortcut.

Verify both install cleanly on a fresh Windows 11 machine and on Windows 10 22H2 with features degraded rather than broken.

### Store submission

The skeleton submission from phase 00 becomes the real one. Confirm `runFullTrust` approval is still in place.

The store listing and every user-facing string must pass the banned-vocabulary check. This includes the description, the screenshots' captions, and any marketing copy on a website, because they are all read as statements of what the product does.

A privacy policy is required and must actually describe what happens: what is stored locally, what is sent to a model provider and under what consent, what is never transmitted, and how to export or delete everything.

### First-run experience

This is the highest-churn moment for these users, so treat it as a feature, not a formality.

The application must be useful before it asks for anything. No permission wall, no account, no tour. Capture works on first launch.

Ask for the cloud model preference at the point of first use, not up front, and explain the trade-off in one sentence without hedging. The default is to ask each time.

Preference setup is optional and skippable, with sensible low-demand defaults already applied. Offer it again later rather than blocking entry.

### Telemetry

Off by default, opt-in, and honest about what it collects. Activity samples, nudge outcomes, and model state never leave the machine regardless of this setting.

What is worth collecting when the user opts in: time from hotkey to captured item, decomposition acceptance split into accepted clean, accepted with edit, and discarded, estimation calibration, cost per user, and schema validation failure rate.

What must never be collected or displayed: streaks, completion percentage as a score, comparison to other users, any productivity index.

### Crash and diagnostics

Structured logs to a rotating local file. A user-visible way to open the log folder and to export a diagnostic bundle they can inspect before sending. Never auto-upload anything containing task content.

### Alpha gate

Ten to twenty real users, recruited from the target population rather than from developers.

Success criteria, measured at two weeks:

- At least sixty percent still capturing in week two. This is the only metric that matters; a tool for people who struggle to start is worthless if they stop opening it.
- Median hotkey-to-captured under five seconds.
- Decomposition accepted without edit at least half the time.
- Cost per active user inside the MVP budget.
- No crash loop, no data loss, no accessibility blocker reported.

If retention misses, do not add features. Find out what made them stop, because it will be friction somewhere in capture or a step that felt wrong, and both are fixable.

## Out of scope

Everything post-MVP: retrieval over history, energy routing, stall detection, obstacle dialogue, meeting notes, weekly review, local model inference, and multi-device sync. The data layer already accommodates all of them; resist the urge to start.

## Definition of done

- Estimation runs in-process, under five milliseconds, with no network and no language model, verified by a test that fails if a model call appears in the path.
- Displayed durations are always ranges from real quantile heads.
- Cold start uses population priors and is never blank.
- Eightieth-percentile coverage is measured and visible.
- Both distribution channels produce a clean install on Windows 11 and Windows 10 22H2.
- The Store submission is accepted.
- The banned-vocabulary check passes across application strings and store metadata.
- A privacy policy exists and is accurate.
- First run is useful without granting anything or creating an account.
- Alpha is in the hands of ten to twenty target users with the retention measurement running.
