# Review loop — progress

Round 1. Branch `fix/publishing-truthfulness`. Baseline: `review/baseline.md`.

## Areas

| Area | Defect | Fix | Behaviour | Integrity | Craft |
|---|---|---|---|---|---|
| A. Publishing truthfulness | B1, B2 | done | self-verified | **not run** | blind |
| B. Fresh-install integrity | B3 | done | self-verified | **not run** | blind |
| C. Consent safety | B5 | done | self-verified | **not run** | blind |
| D. Insights linkage | B4 | done | self-verified | **not run** | blind |

## Critic status — this is not a pass

- **Craft critic: blind.** The visual reference was withdrawn at the user's
  request, so there is no bar to judge against. No craft verdict was issued and
  none should be inferred.
- **Behaviour and integrity critics: terminated early** by an API session limit
  before reaching a verdict, not by anything they found. Neither returned a
  result.
- What stands in their place is the fixer's own verification, which is the
  weakest form of review this method has and the failure mode it explicitly
  warns about. **Round 1 is unverified.** Re-run both critics with fresh context
  before treating this branch as sound.

## Defects closed against baseline.md

| # | Verified how |
|---|---|
| B1 | Old `approve_all` sweeps a carousel to `scheduled`; new one leaves it at `needs_review`, single still schedules. Reproduced in SQL. |
| B2 | `classify_failure` returns `unsupported_format`, retryable=False; authentication/temporary/provider classes unchanged. Exercised against the real module. |
| B3 | Statement from baseline runs clean on a fresh post-fix database; recorded failing on the pre-fix database earlier in the session. |
| B4 | Prefix match links a caption with appended hashtags; equality match still links nothing. |
| B5 | Unanalysed photograph withheld, adults-only photograph still passes. |

## Regression introduced and fixed within round 1

The first cut of `publishable()` treated Facebook like Instagram — single
photographs only. Live publish history says otherwise: Facebook has successfully
published 3 carousels, 1 Reel and 4 singles, because `publish_facebook` sends
the post's first photograph whatever the type. The fix would have blocked all
Facebook multi-photo posting, which is working behaviour and therefore an
automatic fail.

Corrected: Facebook is publishable for every type. The predicate now matches
observed reality exactly — every combination that has ever published is allowed,
every combination that has ever failed is blocked. The underlying truncation is
recorded as W8 and is not fixed.

This was caught by live evidence, not by a critic. It is precisely the kind of
thing the behaviour critic exists to catch, and it got through because that
critic never ran.

## Found by self-review, fixed in round 1

- `PUBLISHABLE_SQL` evaluated to SQL NULL for a NULL `post_type`, so `NOT
  publishable` was also NULL and such a post could be neither queued nor stood
  down. Both columns now COALESCEd; test `publishable_predicate_is_total`.

## Not addressed

- All WEAK items (W1-W7) — out of scope by classification.
- A1 multi-image publishing — deferred by decision.
- TikTok `SELF_ONLY` — deliberately untouched, review pending.

## Live system

Untouched. No writes to the production database; 5,579 images and 10 weddings
verified intact after the work. Publisher PID 33354 left running on old code, so
the Python change is not yet in effect.

## Tooling

```
cargo test    15 passed (was 9)
cargo clippy  0 warnings
npm run build clean
npm test      3 passed
```
