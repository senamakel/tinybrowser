---
name: tinybrowser
description: Use TinyBrowser for bounded web navigation, research, form filling, screenshots, and browser tasks. Prefer the fast browser_task tool for outcome-oriented work and the low-level browser tool for observation, recovery, and unsupported interactions.
---

# TinyBrowser

TinyBrowser exposes two complementary tools:

- `browser_task` runs a bounded Jev-driven observe/decide/act loop. It is the
  default for ordinary browser work.
- `browser` exposes one direct browser operation. Use it for inspection,
  visual work, precise recovery, or behavior the task loop does not support.

The tools are the capability. This skill is operating guidance; never replace
an available tool with shell commands or guessed HTTP requests merely because
the site looks simple.

## Start with an outcome

Phrase a `browser_task` goal as an observable final state:

```json
{
  "goal": "Reach a page visibly listing one-way Mumbai to Goa flight options with airlines, times, and fares. Do not book or purchase.",
  "start_url": "https://www.trip.com/flights/city-bom-airport-goi/",
  "allowed_origins": ["trip.com"]
}
```

Avoid procedural goals such as “click this, fill that, then click search” when
the final page cannot prove those historical actions. Put exact values in
`inputs`, keyed by their meaning:

```json
{
  "goal": "Reach the page state that visibly says Task complete by completing the form.",
  "inputs": {
    "code word": "tinybrowser"
  }
}
```

The controller sends input names to the decision model and retains their values
for the selected typed browser action. Never invent absent names, addresses,
dates, contact details, payment data, or authentication secrets.

## Interpret task statuses

- `done`: the selected `DONE` operation and independent completion check agree.
- `done_unconfirmed`: the operation selected `DONE`, but visible evidence did
  not clear the completion threshold. Inspect with `browser snapshot` or
  `browser read`; do not silently call it success.
- `needs_confirmation`: a consequential action is pending. Present the exact
  effect to the user. Only the host confirmation mechanism may resume it.
- `needs_input`: ask only for the missing caller-owned values.
- `stuck`: use a fresh snapshot or screenshot once, diagnose the obstacle, and
  either recover with `browser` or report it.
- `blocked`: report the visible blocker. Do not evade CAPTCHA, origin policy,
  authentication, or site restrictions.
- `budget`: report that the finite action, time, or cost limit was reached.

## Downloads and external completion

A download can finish without changing the page. The harness should start
`WaitDownload` before or concurrently with the Jev loop and race the two. A
completed or cancelled `DownloadInfo` is authoritative browser state and should
stop the decision loop immediately; do not keep asking Jev whether an unchanged
page means the file finished. `ListDownloads` inspects retained handles without
consuming them.

The host—not the model—chooses the absolute download directory when opening the
session. A completed handle reports the expected path and byte counts. Verify
the file's checksum, signature, and media type before opening it.

## Consequential actions

Never let the model grant itself authority. Buying, booking, paying, sending,
posting, publishing, deleting, or confirming a transaction must stop before
the effect. The host obtains user confirmation and binds it to the pending
session, snapshot, target, and action. A boolean in model-authored tool input is
not confirmation.

For purchases and travel bookings:

1. Ask for material choices that are not specified: dates, route or airport,
   fare/refundability, baggage, passenger identity, and contact details.
2. Navigate and fill only the supplied information.
3. Stop at the final review or payment page.
4. Return the total price, currency, itinerary, restrictions, and what action
   remains. Do not submit payment unless the user separately authorizes it.

## Low-level recovery

Use `browser` when the task loop needs help:

1. `snapshot` before acting.
2. Prefer the current `@eN` ref over a guessed CSS selector.
3. After navigation or any page-changing action, snapshot again.
4. A stale ref means snapshot again; never retry the old ref.
5. If a click reports a covering element, handle the named banner or modal,
   snapshot again, then reconsider the original action.
6. Use `screenshot` for canvas, layout, visual state, or controls omitted from
   accessible text.
7. Close the session when the conversation ends.

Custom widgets can be absent from a site's accessibility tree. Prefer an
accessible canonical route or results page when one exists. Otherwise report
the limitation or use a deliberate low-level selector only after inspecting the
live DOM; never have the model invent one from prose.

## Keep output small and useful

Return status, final URL and title, a short evidence excerpt, action summary,
provider calls, token usage, estimated cost, and any pending confirmation.
Do not dump the full accessibility tree unless the caller asks for debugging.
