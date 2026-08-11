# baseline.md — SocialFlow, observed 2026-08-11

Recorded before any fix. Everything here was observed by running the app or
querying the live database, not inferred from reading source.

The job the app has to do well: **take a wedding folder to approved, scheduled
posts, and never publish something it was not allowed to publish.**

## Environment

- Live database: `~/Library/Application Support/com.socialflow.desktop/socialflow.db`
  — 5,579 images, 10 weddings, 91 posts (16 published, 69 scheduled, 6 failed).
- Instagram, Facebook and TikTok all connected. `mock_publish=false`.
- Publisher running as PID 33354 throughout; left untouched.
- Backup taken before any work: `backups/pre-review-20260811-212031.db` (12 MB).
- Sandbox: dev build run with `HOME` redirected to a scratch directory, giving a
  fresh empty database and zero connected accounts. Cannot publish.

## Tooling output (pasted, not summarised)

```
npm run build   →  tsc clean; vite built in 2.02s; 1584 modules
npm test        →  3 passed (3)
cargo clippy --all-targets  →  Finished dev profile, no warnings
cargo test      →  9 passed; 0 failed
```

The suite is green and proves almost nothing about the defects below. 12 tests
across ~5,800 lines, none covering publishing, consent, or recovery.

## BROKEN

### B1. Carousels, reels and story packs can never publish

`publish_instagram` rejects any post whose `post_type != 'single'`
(`scripts/socialflow_live_publisher.py:313`), but `create_content_campaign`
generates carousel, reel and story_pack as its primary content mix
(`src-tauri/src/lib.rs:1284`). The app's main feature produces output its own
publisher structurally refuses.

Observed in live data — six posts dead of exactly this:

```
id  platform   post_type   status  scheduled_at
13  instagram  reel        failed  2026-08-09T10:00:00.000Z
15  instagram  story_pack  failed  2026-08-09 17:00:00 +01:00
21  instagram  carousel    failed  2026-08-10 08:00:00 +01:00
22  instagram  reel        failed  2026-08-10 11:00:00 +01:00
24  instagram  story_pack  failed  2026-08-10 17:00:00 +01:00
57  instagram  carousel    failed  2026-08-11 08:00:00 +01:00
```

Reproduction: create a content campaign with any format other than `single`,
wait for its scheduled time, observe `publish_recovery`.

### B2. The failure lies about itself and retries 8 times

`classify_failure` has no branch for the single-photo restriction, so it falls
through to the default `"provider"` class — retryable, "SocialFlow is retrying
automatically and preserving the post." Observed: every one of the six posts
reached `retry_count = 8` over roughly 11 hours before being flagged. Post 58
is currently at retry 7 with a next attempt scheduled.

A permanent structural refusal is presented to the user as a transient network
problem.

### B3. A fresh install cannot connect Facebook

`save_facebook_connection` queries `publish_recovery`
(`src-tauri/src/lib.rs:2435-2436`), but that table is created only by the Python
publisher (`scripts/socialflow_live_publisher.py:35`) and by no Rust migration.

Reproduced in the sandbox. Fresh database applies migrations 1-9 and produces:

```
publish_recovery → ABSENT
Error: in prepare, no such table: publish_recovery
```

The token is written to Keychain and the account row committed *before* the
failing statement, so the user sees an error on a connection that partly
succeeded.

### B4. Instagram insights never link to local posts

`sync_instagram_insights` matches on `SELECT id FROM posts WHERE caption=?`
(`src-tauri/src/lib.rs:2275`), but the publisher appends hashtags before sending
(`scripts/socialflow_live_publisher.py:325`), so the remote caption is
`caption + "\n\n" + tags` and never equals the stored column.

Observed in live data:

```
instagram_performance:  55 rows,  0 linked
```

Consequence: `instagram_media_id` is never backfilled, and the analytics brain
learns sections from `infer_section` keyword guessing instead of real analysis.

### B5. Unanalysed photographs bypass the "no children" consent filter

`marketing_safe_images` (`src-tauri/src/lib.rs:808`) looks up the description
from `image_analysis`; when no row exists the lookup returns
`unwrap_or_default()` → `""` → the substring test passes → the photograph is
approved for publishing.

On the one path where a false negative has real-world consequences, the failure
mode is permissive. The term list is also thin: it misses "kids", "children",
"pageboy", "flower girl", and matches "childhood".

## WEAK

- **W1.** Cloudflare quick tunnel is the single point of failure for every
  Instagram publish. Observed today: post 59 failed three times
  (`curl: (6) Could not resolve host`) before succeeding on the fourth attempt.
  Retry logic saved it; nothing else would have.
- **W2.** Three timestamp formats coexist in `posts.scheduled_at` —
  `2026-08-09T10:00:00.000Z`, `2026-08-10 08:00:00 +01:00`, and bare local — and
  are compared as strings against `datetime('now','localtime')`. `created_at` is
  UTC. Ordering and due-detection are unreliable across formats.
- **W3.** Autopilot (`src/App.tsx:133`) has no in-flight guard and depends on
  `data.*`, so a refresh during a long run re-enters it and can generate
  duplicate campaigns, each rendering reels through ffmpeg.
- **W4.** `create_content_campaign` runs ffmpeg renders inside an open SQLite
  transaction; `import_paths` holds the DB mutex across hashing and
  thumbnailing every file. Both produce minutes-long write locks.
- **W5.** The publisher is spawned detached and loops forever; quitting
  SocialFlow leaves it publishing, with no way to stop it from the UI.
- **W6.** Drag-and-drop import reads `file.path` (`src/App.tsx:271`), removed in
  Tauri v2. Dead code.
- **W7.** Migration v3 force-sets `allow_cloud_ai='true'`, overriding a user who
  had turned it off.

## ABSENT

- **A1.** Multi-image publishing. Carousel, reel and story pack exist as
  generated artefacts with rendered assets on disk, but no code path publishes
  them. Deferred deliberately: the decision this round is to stop generating
  what cannot ship, and build real multi-image publishing as its own piece of
  work afterwards.

## Not to be touched

TikTok is under review by TikTok. `tiktok_publish_mode='direct'` combined with
the hardcoded `privacy_level: "SELF_ONLY"`
(`scripts/socialflow_live_publisher.py:469`) means an approved direct publish
would go out visible only to the account owner while the UI claims it published.
**Recorded as a known defect and deliberately left unfixed** until review
clears.

## Areas for the loop

| Area | Covers | Class |
|---|---|---|
| A. Publishing truthfulness | B1, B2 | broken |
| B. Fresh-install integrity | B3 | broken |
| C. Consent safety | B5 | broken |
| D. Insights linkage | B4 | broken |

Weak items are not in scope this round. Polish on top of a real defect is
wasted work.
