# Reduced animations (R2), Philips 65OLED818/12, 2026-10-23

`rusttv optimize apply reduced-animations` (profile v1: window, transition
and animator animation scales set to 0.5), measured A–B–A as decided in
OPTIMIZER.md § Decisions 2. RustTV was Home (takeover active), 22 privacy
packages disabled. First run on this model, so applied with
`--allow-untested`; the owner typed both confirmations.

## Method

- Metric: key press → end of the last frame of the window transition.
  Key press = first `interceptKeyBeforeQueueing` slice of the press;
  transition frames = Perfetto `actual_frame_timeline_slice` rows on
  `%animation-leash%`, `%ActivityRecord{%` and `TX - Task=%` layers within 3 s
  of the press (frame duration capped at 50 ms, because the last frame of a
  still screen stays open until the next one). Script: `transition_metric.py`.
- Trace: `android.surfaceflinger.frametimeline` + atrace `input`, `wm`
  (`block.sh`). Raw traces are not committed (they name installed apps).
- Each block: 2 warm-up cycles (not counted), then 30 cycles of "OK on the
  app focused in RustTV, wait 4 s, Home, wait 3.5 s", keys injected with
  `input keyevent`.
- Original values on this TV: window 1.0, transition 1.0, **animator 0.6**
  (Philips default).

## Results (median, IQR)

| Block | Scales | Open app | Back to Home | n |
|---|---|---|---|---|
| A1 before | 1.0 / 1.0 / 0.6 | 617 ms (608–623) | 478 ms (472–485) | 30 / 30 |
| B applied | 0.5 / 0.5 / 0.5 | **407 ms** (400–414) | **279 ms** (273–284) | 27 / 27 |

- Open app: −210 ms (−34 %); back to Home: −199 ms (−42 %). The IQRs don't
  overlap (claim rule: this device only).
- B: for the first ~35 s of its trace the frame timeline carried no layer
  names, so 3 open and 3 Home presses couldn't be attributed and are left out
  (27 each). Nothing else was discarded.
- Restore (`optimize restore reduced-animations`) put back 1.0 / 1.0 / 0.6,
  read back with `settings get global` afterwards.
- The owner watched block B: the faster animations "look ok".

The launcher's own animations (Slint) don't use these scales; this is a
system-wide change to Android's window and app transitions.
