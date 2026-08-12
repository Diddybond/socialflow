#!/usr/bin/env python3
"""Publish due SocialFlow single-image posts through Meta's official APIs.

Local photographs are exposed through a random, short-lived Cloudflare URL only
for the few seconds Meta needs to copy them. Tokens remain in macOS Keychain.
"""
from __future__ import annotations

import argparse
import http.server
import json
import mimetypes
import os
import re
import secrets
import signal
import socket
import sqlite3
import subprocess
import tempfile
import threading
import time
import urllib.parse
import urllib.request
from datetime import datetime, timedelta
from pathlib import Path

DB = Path.home() / "Library/Application Support/com.socialflow.desktop/socialflow.db"
CLOUDFLARED = "/usr/local/opt/cloudflared/bin/cloudflared"


def ensure_recovery_schema(db: sqlite3.Connection) -> None:
    """Durable retry state so recovery survives app and Mac restarts."""
    db.executescript(
        """
        CREATE TABLE IF NOT EXISTS publish_recovery(
          post_id INTEGER PRIMARY KEY REFERENCES posts(id) ON DELETE CASCADE,
          failure_class TEXT NOT NULL,
          retry_count INTEGER NOT NULL DEFAULT 0,
          next_retry_at TEXT,
          requires_action INTEGER NOT NULL DEFAULT 0,
          resolution_hint TEXT DEFAULT '',
          last_error TEXT DEFAULT '',
          updated_at TEXT DEFAULT CURRENT_TIMESTAMP
        );
        """
    )
    db.commit()


def classify_failure(error: Exception) -> tuple[str, bool, str]:
    message = str(error)
    lower = message.lower()
    # A format the publisher does not implement is permanent, not transient.
    # Retrying it eight times over eleven hours told the photographer their
    # network was flaky when in fact the post could never have been sent.
    if "accepts single photographs only" in lower or "reel, generated video or connected account" in lower:
        return ("unsupported_format", False,
                "SocialFlow cannot publish this format yet. The rendered asset is ready to post by hand; "
                "this post has been moved out of the queue so it stops retrying.")
    # Meta uses the generic OAuthException type for media-fetch failures too.
    # These are not authentication failures: the short-lived public URL can
    # need a few seconds before every Cloudflare edge serves the photograph.
    if any(marker in lower for marker in (
        '"code":9004', '"code": 9004', '"error_subcode":2207052',
        '"error_subcode": 2207052', 'only photo or video can be accepted',
    )):
        return "temporary", True, "SocialFlow is refreshing the secure photograph link and retrying automatically."
    # Authentication and permission changes require Meta's signed-in approval.
    if any(marker in lower for marker in (
        '"code":190', '"code": 190', 'oauth', 'access token', 'session has expired',
        'permission(s)', 'insufficient permission', 'not authorized',
    )):
        return "authentication", False, "Reconnect this account in Settings; SocialFlow will resume the post afterwards."
    if any(marker in lower for marker in (
        "original photograph is unavailable", "post or connected", "post, photograph or connected",
    )):
        return "content", False, "Open the affected post and restore its photograph or account connection."
    if any(marker in lower for marker in (
        "timed out", "timeout", "temporar", "tunnel", "connection", "network",
        "http error 429", "http error 500", "http error 502", "http error 503", "http error 504",
        "did not finish copying",
    )):
        return "temporary", True, "SocialFlow is retrying automatically."
    # Unknown provider failures get several increasingly spaced retries before
    # asking for attention; this catches short-lived API changes safely.
    return "provider", True, "SocialFlow is retrying automatically and preserving the post."


DIAGNOSIS_SCHEMA = json.dumps({
    "type": "object", "additionalProperties": False,
    "properties": {
        "failure_class": {"type": "string", "enum": ["temporary", "authentication", "content", "unsupported_format", "rate_limit", "provider"]},
        "retryable": {"type": "boolean"},
        "retry_in_seconds": {"type": "integer", "minimum": 0, "maximum": 86400},
        "diagnosis": {"type": "string"},
        "resolution_hint": {"type": "string"},
        "remedy": {"type": "string", "enum": ["none", "drop_photograph", "shrink_set", "recrop"]},
        "remedy_position": {"type": "integer", "minimum": 0, "maximum": 13},
    },
    "required": ["failure_class", "retryable", "retry_in_seconds", "diagnosis", "resolution_hint", "remedy"],
})


def apply_remedy(post_id: int, remedy: str, position: int) -> str | None:
    """Carry out the model's suggested fix so the post can go out on its own.

    Deliberately conservative: it removes or trims photographs and never
    rewrites a caption. The photograph stays in the library — only its place in
    this post changes.
    """
    if remedy in (None, "none"):
        return None
    db = sqlite3.connect(DB)
    try:
        images = db.execute(
            "SELECT image_id,position FROM post_images WHERE post_id=? ORDER BY position", (post_id,)
        ).fetchall()
        if not images:
            return None
        if remedy == "drop_photograph" and len(images) > 1:
            target = images[min(position, len(images) - 1)][0]
            db.execute("DELETE FROM post_images WHERE post_id=? AND image_id=?", (post_id, target))
            note = f"removed photograph {target} and retried"
        elif remedy == "shrink_set" and len(images) > 3:
            keep = images[: max(3, len(images) // 2)]
            db.execute("DELETE FROM post_images WHERE post_id=? AND image_id NOT IN (%s)"
                       % ",".join(str(i[0]) for i in keep), (post_id,))
            note = f"trimmed the set from {len(images)} to {len(keep)} photographs and retried"
        elif remedy == "recrop":
            # aspect_safe_copy re-crops on every publish, so a retry is the fix.
            note = "re-cropped the photograph to Instagram's accepted range and retried"
        else:
            return None
        # Renumber so positions stay contiguous.
        remaining = db.execute("SELECT image_id FROM post_images WHERE post_id=? ORDER BY position", (post_id,)).fetchall()
        for index, (image_id,) in enumerate(remaining):
            db.execute("UPDATE post_images SET position=? WHERE post_id=? AND image_id=?", (index, post_id, image_id))
        db.commit()
        return note
    except sqlite3.Error:
        return None
    finally:
        db.close()


def ai_diagnose(post_id: int, error: Exception) -> dict | None:
    """Ask Claude what actually went wrong and whether it is worth retrying.

    Keyword matching cannot tell a permanent Meta policy rejection from a
    transient fetch failure — it guesses from substrings, which is how a
    structurally impossible post was retried eight times over eleven hours.
    Falls back to the keyword classifier when Claude is unavailable.
    """
    claude = subprocess.run(["/bin/zsh", "-lc", "command -v claude"], capture_output=True, text=True)
    if claude.returncode != 0 or not claude.stdout.strip():
        return None
    db = sqlite3.connect(DB)
    db.row_factory = sqlite3.Row
    row = db.execute(
        """SELECT p.post_type,COALESCE(p.platform,'instagram') platform,p.asset_path,
                  (SELECT COUNT(*) FROM post_images WHERE post_id=p.id) photographs,
                  (SELECT COUNT(*) FROM publish_attempts WHERE post_id=p.id AND status='failed') previous_failures
           FROM posts p WHERE p.id=?""", (post_id,)).fetchone()
    model = db.execute("SELECT value FROM app_settings WHERE key='claude_model_diagnosis'").fetchone()
    db.close()
    model = model[0] if model else "opus"
    context = dict(row) if row else {}
    prompt = (
        "A scheduled social media post failed to publish. Diagnose it.\n\n"
        f"Post: {json.dumps(context)}\n\n"
        f"Error returned by the platform:\n{str(error)[:2000]}\n\n"
        "Decide the failure class, whether retrying can plausibly succeed without a human, "
        "and how many seconds to wait before retrying (0 if not retryable). "
        "A permanent rejection — an unsupported format, a policy refusal, a revoked token, a missing file — "
        "must NOT be marked retryable, however transient the wording looks. "
        "Rate limits are retryable but need a long wait. "
        "Write resolution_hint as one plain sentence for a photographer, naming what to do next.\n"
        "If the post can be repaired automatically, set remedy: drop_photograph (with remedy_position, "
        "the zero-based index of the offending photograph) when one specific image is refused; shrink_set "
        "when the set is too large; recrop when the dimensions or aspect ratio are the problem. "
        "Use remedy 'none' when nothing safe can be done without a person — never guess."
    )
    try:
        result = subprocess.run(
            [claude.stdout.strip(), "--model", model, "--print", "--output-format", "json",
             "--json-schema", DIAGNOSIS_SCHEMA, "--permission-mode", "dontAsk", "--no-session-persistence", prompt],
            capture_output=True, text=True, timeout=180,
        )
        if result.returncode != 0:
            return None
        payload = json.loads(result.stdout)
        out = payload.get("structured_output")
        if out is None and isinstance(payload.get("result"), str):
            cleaned = payload["result"].strip().removeprefix("```json").removeprefix("```").removesuffix("```").strip()
            out = json.loads(cleaned)
        if not isinstance(out, dict) or "failure_class" not in out:
            return None
        return out
    except (subprocess.TimeoutExpired, json.JSONDecodeError, OSError):
        return None


def schedule_recovery(post_id: int, error: Exception) -> None:
    db = sqlite3.connect(DB)
    ensure_recovery_schema(db)
    diagnosis = ai_diagnose(post_id, error)
    remedy_note = None
    if diagnosis:
        failure_class = diagnosis["failure_class"]
        retryable = bool(diagnosis["retryable"])
        hint = diagnosis["resolution_hint"]
        ai_wait = int(diagnosis.get("retry_in_seconds") or 0)
        # Fix it rather than waiting for a human, where the fix is safe.
        remedy_note = apply_remedy(post_id, diagnosis.get("remedy", "none"), int(diagnosis.get("remedy_position") or 0))
        if remedy_note:
            retryable = True
            ai_wait = min(ai_wait or 120, 600)
            hint = f"SocialFlow {remedy_note}. {hint}"
            print(f"post {post_id}: {remedy_note}", flush=True)
    else:
        failure_class, retryable, hint = classify_failure(error)
        ai_wait = 0
    previous = db.execute("SELECT retry_count FROM publish_recovery WHERE post_id=?", (post_id,)).fetchone()
    retries = (previous[0] if previous else 0) + 1
    # Unknown failures are promoted to action-required after eight attempts;
    # known temporary failures continue recovering indefinitely at a safe pace.
    requires_action = not retryable or (failure_class == "provider" and retries >= 8)
    delays = [60, 180, 600, 1800, 3600, 10800, 21600]
    wait = ai_wait if ai_wait > 0 else delays[min(retries - 1, len(delays) - 1)]
    retry_at = None if requires_action else datetime.now() + timedelta(seconds=wait)
    # An unpublishable format is not a failed post — the photographs and the
    # rendered asset are fine. Return it to draft so it leaves the queue
    # cleanly instead of sitting in the failed pile implying something broke.
    if failure_class in {"unsupported_format", "content"}:
        status = "draft"
    elif requires_action:
        status = "failed"
    else:
        status = "scheduled"
    db.execute("UPDATE posts SET status=?,updated_at=CURRENT_TIMESTAMP WHERE id=?", (status, post_id))
    db.execute(
        """INSERT INTO publish_recovery(post_id,failure_class,retry_count,next_retry_at,requires_action,resolution_hint,last_error,updated_at)
           VALUES(?,?,?,?,?,?,?,CURRENT_TIMESTAMP)
           ON CONFLICT(post_id) DO UPDATE SET failure_class=excluded.failure_class,retry_count=excluded.retry_count,
           next_retry_at=excluded.next_retry_at,requires_action=excluded.requires_action,
           resolution_hint=excluded.resolution_hint,last_error=excluded.last_error,updated_at=CURRENT_TIMESTAMP""",
        (post_id, failure_class, retries, retry_at.isoformat(sep=" ") if retry_at else None,
         int(requires_action), hint,
         ((diagnosis["diagnosis"] + "\n\n") if diagnosis else "") + str(error)[:3500]),
    )
    if requires_action:
        summary = f"Post {post_id} needs attention: {hint}"
        db.execute("INSERT INTO app_settings(key,value)VALUES('publisher_action_required',?) ON CONFLICT(key) DO UPDATE SET value=excluded.value", (summary,))
    db.commit()
    db.close()


def clear_recovery(post_id: int) -> None:
    db = sqlite3.connect(DB)
    ensure_recovery_schema(db)
    db.execute("DELETE FROM publish_recovery WHERE post_id=?", (post_id,))
    remaining = db.execute("SELECT COUNT(*) FROM publish_recovery WHERE requires_action=1").fetchone()[0]
    if not remaining:
        db.execute("DELETE FROM app_settings WHERE key='publisher_action_required'")
    db.commit()
    db.close()


def api(url: str, token: str, data: dict[str, str] | None = None) -> dict:
    encoded = urllib.parse.urlencode(data).encode() if data is not None else None
    request = urllib.request.Request(url, data=encoded)
    request.add_header("Authorization", f"Bearer {token}")
    request.add_header("User-Agent", "SocialFlow/0.1")
    try:
        with urllib.request.urlopen(request, timeout=45) as response:
            return json.loads(response.read())
    except Exception as error:
        detail = getattr(error, "read", lambda: b"")().decode(errors="replace")
        raise RuntimeError(detail or str(error)) from error


def refresh_instagram_token_if_needed() -> None:
    """Refresh a long-lived Instagram token before its 60-day window closes."""
    db = sqlite3.connect(DB)
    row = db.execute("SELECT instagram_user_id,token_expiry FROM instagram_accounts WHERE connected=1 ORDER BY id DESC LIMIT 1").fetchone()
    if not row:
        db.close()
        return
    account_id, expiry_text = row
    try:
        expiry = datetime.fromisoformat((expiry_text or "").replace("Z", "+00:00")).replace(tzinfo=None)
        if expiry > datetime.now() + timedelta(days=7):
            db.close()
            return
    except ValueError:
        pass
    token = subprocess.check_output(
        ["/usr/bin/security", "find-generic-password", "-s", "com.socialflow.desktop.instagram", "-a", account_id, "-w"], text=True
    ).strip()
    url = "https://graph.instagram.com/refresh_access_token?" + urllib.parse.urlencode({"grant_type": "ig_refresh_token", "access_token": token})
    with urllib.request.urlopen(url, timeout=45) as response:
        result = json.loads(response.read())
    refreshed = result.get("access_token")
    if not refreshed:
        raise RuntimeError(f"Instagram did not return a refreshed token: {json.dumps(result)}")
    subprocess.run(["/usr/bin/security", "add-generic-password", "-U", "-s", "com.socialflow.desktop.instagram", "-a", account_id, "-w", refreshed], check=True, stdout=subprocess.DEVNULL)
    os.environ["SOCIALFLOW_IG_TOKEN"] = refreshed
    expires = datetime.now() + timedelta(seconds=int(result.get("expires_in", 5184000)))
    db.execute("UPDATE instagram_accounts SET token_expiry=?,last_successful_request=CURRENT_TIMESTAMP WHERE instagram_user_id=?", (expires.isoformat(), account_id))
    db.commit()
    db.close()


def refresh_tiktok_token() -> None:
    db=sqlite3.connect(DB)
    row=db.execute("SELECT ta.open_id,(SELECT value FROM app_settings WHERE key='tiktok_client_key') FROM tiktok_accounts ta WHERE ta.connected=1 LIMIT 1").fetchone()
    db.close()
    if not row: return
    open_id,client_key=row
    refresh=subprocess.check_output(["/usr/bin/security","find-generic-password","-s","com.socialflow.desktop.tiktok.refresh","-a",open_id,"-w"],text=True).strip()
    secret=subprocess.check_output(["/usr/bin/security","find-generic-password","-s","com.socialflow.desktop.tiktok.client","-a",client_key,"-w"],text=True).strip()
    payload=urllib.parse.urlencode({"client_key":client_key,"client_secret":secret,"grant_type":"refresh_token","refresh_token":refresh}).encode()
    request=urllib.request.Request("https://open.tiktokapis.com/v2/oauth/token/",data=payload,method="POST"); request.add_header("Content-Type","application/x-www-form-urlencoded")
    with urllib.request.urlopen(request,timeout=45) as response: result=json.loads(response.read())
    if result.get("error"): raise RuntimeError(result.get("error_description",result["error"]))
    for service,password in [("com.socialflow.desktop.tiktok",result["access_token"]),("com.socialflow.desktop.tiktok.refresh",result["refresh_token"])]:
        subprocess.run(["/usr/bin/security","add-generic-password","-U","-s",service,"-a",open_id,"-w",password],check=True,stdout=subprocess.DEVNULL)


class OneImage(http.server.BaseHTTPRequestHandler):
    """Serves the files of a single post over the temporary tunnel.

    A carousel needs every one of its photographs reachable at the same time,
    and a Reel needs its rendered video, so this holds a token->path map rather
    than one file. Tokens are random per publish and the server dies with it.
    """

    files: dict[str, Path] = {}

    def do_GET(self) -> None:
        token = self.path.split("?", 1)[0].lstrip("/")
        target = self.files.get(token)
        if target is None:
            self.send_error(404)
            return
        content = target.read_bytes()
        self.send_response(200)
        self.send_header("Content-Type", mimetypes.guess_type(target.name)[0] or "image/jpeg")
        self.send_header("Content-Length", str(len(content)))
        self.send_header("Accept-Ranges", "none")
        self.send_header("Cache-Control", "no-store")
        self.end_headers()
        self.wfile.write(content)

    def log_message(self, _format: str, *_args: object) -> None:
        return


def temporary_urls(paths: list[Path]):
    """Expose several local files on one quick tunnel. Returns (urls, server, tunnel)."""
    OneImage.files = {
        secrets.token_urlsafe(24) + path.suffix.lower(): path for path in paths
    }
    tokens = list(OneImage.files)
    urls, server, tunnel = _tunnel_for(tokens)
    return urls, server, tunnel


def temporary_url(image_path: Path):
    """Single-file form, kept so the existing photo path reads unchanged."""
    urls, server, tunnel = temporary_urls([image_path])
    return urls[0], server, tunnel


def _tunnel_for(tokens: list[str]):
    server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), OneImage)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    last_error = ""
    # Quick tunnels occasionally receive an edge hostname whose DNS record is
    # not usable. Replace that tunnel automatically instead of failing a post.
    # A fresh trycloudflare hostname routinely takes 30-60s to resolve
    # everywhere, so each tunnel gets a long readiness window before being
    # replaced, and there are more attempts than there used to be.
    for _attempt in range(5):
        tunnel = subprocess.Popen(
            [CLOUDFLARED, "tunnel", "--url", f"http://127.0.0.1:{server.server_port}", "--no-autoupdate"],
            stdout=subprocess.DEVNULL,
            stderr=subprocess.PIPE,
            text=True,
        )
        deadline = time.time() + 35
        public = ""
        while time.time() < deadline:
            line = tunnel.stderr.readline() if tunnel.stderr else ""
            match = re.search(r"https://[a-z0-9-]+\.trycloudflare\.com", line)
            if match:
                public = match.group(0)
                break
            if tunnel.poll() is not None:
                break
        if public:
            urls = [f"{public}/{token}" for token in tokens]
            # Wait for DNS to exist at all before spending the readiness window
            # on requests that cannot possibly succeed.
            host = urllib.parse.urlparse(public).hostname or ""
            dns_deadline = time.time() + 60
            while time.time() < dns_deadline:
                try:
                    socket.getaddrinfo(host, 443)
                    break
                except socket.gaierror:
                    time.sleep(3)
            readiness_deadline = time.time() + 90
            while time.time() < readiness_deadline:
                # Probe the first file only: once one edge serves this tunnel the
                # rest are reachable, and probing a large video wastes the window.
                check = subprocess.run(
                    ["/usr/bin/curl", "--silent", "--show-error", "--fail", "--location",
                     "--max-time", "12", "--output", "/dev/null", "--write-out", "%{http_code} %{content_type}", urls[0]],
                    capture_output=True,
                    text=True,
                )
                status = check.stdout.strip()
                if check.returncode == 0 and (status.startswith("200 image/") or status.startswith("200 video/")):
                    return urls, server, tunnel
                last_error = check.stderr.strip() or status or "public edge not ready"
                time.sleep(2)
        else:
            last_error = "quick tunnel did not provide a public address"
        tunnel.terminate()
        try:
            tunnel.wait(timeout=5)
        except subprocess.TimeoutExpired:
            tunnel.kill()
    server.shutdown()
    raise RuntimeError(f"Temporary secure media link was not ready after automatic tunnel replacement: {last_error}")


def facebook_page_credentials(db: sqlite3.Connection):
    """Page ID and token for the connected Page, or None."""
    row = db.execute("SELECT page_id FROM facebook_accounts WHERE connected=1 LIMIT 1").fetchone()
    if not row:
        return None
    page_id = row["page_id"] if isinstance(row, sqlite3.Row) else row[0]
    try:
        token = subprocess.check_output(
            ["/usr/bin/security", "find-generic-password", "-s", "com.socialflow.desktop.facebook",
             "-a", page_id, "-w"], text=True,
        ).strip()
    except subprocess.CalledProcessError:
        return None
    return page_id, token


def meta_cdn_urls(page_id: str, page_token: str, paths: list[Path]) -> tuple[list[str], list[str]]:
    """Stage photographs on Meta's own CDN and return their URLs.

    Instagram fetching from Meta removes the quick tunnel — and with it the
    "could not resolve host" failures that are the single largest cause of
    failed posts. The staged photographs are uploaded unpublished, so they never
    appear on the Page, and are deleted once Instagram has copied them.
    """
    urls, staged = [], []
    for photograph in paths:
        media_id = facebook_upload_photo(page_id, page_token, photograph, published=False)
        staged.append(media_id)
        detail = api(
            f"https://graph.facebook.com/v23.0/{media_id}?fields=images&access_token={urllib.parse.quote(page_token)}",
            page_token,
        )
        candidates = detail.get("images") or []
        if not candidates:
            raise RuntimeError(f"Meta returned no CDN image for {photograph.name}")
        best = max(candidates, key=lambda item: item.get("width", 0) * item.get("height", 0))
        urls.append(best["source"])
    return urls, staged


def discard_staged_media(page_token: str, staged: list[str]) -> None:
    for media_id in staged:
        try:
            request = urllib.request.Request(
                f"https://graph.facebook.com/v23.0/{media_id}?access_token={urllib.parse.quote(page_token)}",
                method="DELETE",
            )
            urllib.request.urlopen(request, timeout=30).close()
        except Exception:
            # Staging cleanup must never fail a published post.
            pass


def public_urls_for(db: sqlite3.Connection, paths: list[Path]):
    """(urls, server, tunnel, page_token, staged) — Meta CDN first, tunnel second."""
    credentials = facebook_page_credentials(db)
    if credentials:
        page_id, page_token = credentials
        try:
            urls, staged = meta_cdn_urls(page_id, page_token, paths)
            return urls, None, None, page_token, staged
        except Exception as error:
            print(f"Meta CDN staging unavailable, falling back to tunnel: {error}", flush=True)
    urls, server, tunnel = temporary_urls(paths)
    return urls, server, tunnel, None, []


def await_container(container: str, token: str, limit: int) -> None:
    """Block until Meta has finished copying the media, or explain why it failed."""
    deadline = time.time() + limit
    while time.time() < deadline:
        status = api(
            f"https://graph.instagram.com/{container}?fields=status_code,status&access_token={urllib.parse.quote(token)}",
            token,
        )
        if status.get("status_code") == "FINISHED":
            return
        if status.get("status_code") in {"ERROR", "EXPIRED"}:
            raise RuntimeError(status.get("status") or status["status_code"])
        time.sleep(4)
    raise RuntimeError("Instagram did not finish copying the media in time")


def publish_instagram_carousel(account: str, token: str, urls: list[str], caption: str) -> str:
    """Instagram's real carousel flow: an item container per photograph, then a
    parent container listing them, then one publish."""
    children = []
    for url in urls[:10]:  # Instagram accepts a maximum of ten items.
        item = api(
            f"https://graph.instagram.com/{account}/media",
            token,
            {"image_url": url, "is_carousel_item": "true"},
        )["id"]
        children.append(item)
    if len(children) < 2:
        raise RuntimeError("A carousel needs at least two photographs")
    for child in children:
        await_container(child, token, 120)
    container = api(
        f"https://graph.instagram.com/{account}/media",
        token,
        {"media_type": "CAROUSEL", "children": ",".join(children), "caption": caption},
    )["id"]
    await_container(container, token, 180)
    return api(
        f"https://graph.instagram.com/{account}/media_publish",
        token,
        {"creation_id": container},
    )["id"]


def publish_instagram_reel(account: str, token: str, video_url: str, caption: str) -> str:
    container = api(
        f"https://graph.instagram.com/{account}/media",
        token,
        {"media_type": "REELS", "video_url": video_url, "caption": caption},
    )["id"]
    # Video transcoding is far slower than a photograph copy.
    await_container(container, token, 600)
    return api(
        f"https://graph.instagram.com/{account}/media_publish",
        token,
        {"creation_id": container},
    )["id"]


def publish_instagram_story(account: str, token: str, image_url: str) -> str:
    # Stories carry no caption; Instagram rejects the field outright.
    container = api(
        f"https://graph.instagram.com/{account}/media",
        token,
        {"media_type": "STORIES", "image_url": image_url},
    )["id"]
    await_container(container, token, 180)
    return api(
        f"https://graph.instagram.com/{account}/media_publish",
        token,
        {"creation_id": container},
    )["id"]


def image_dimensions(path: Path) -> tuple[int, int]:
    result = subprocess.run(
        ["/usr/bin/sips", "-g", "pixelWidth", "-g", "pixelHeight", str(path)],
        capture_output=True, text=True,
    )
    values = dict(
        line.strip().split(": ", 1)
        for line in result.stdout.splitlines()
        if ": " in line
    )

    def measure(key: str) -> int:
        # sips reports "<nil>" for anything it cannot read as an image — a video,
        # for instance. Treat that as unknown rather than crashing the publish.
        try:
            return int(values.get(key, 0))
        except ValueError:
            return 0

    return measure("pixelWidth"), measure("pixelHeight")


# Instagram accepts 4:5 (0.80) through 1.91:1. Anything outside is refused with
# an "aspect ratio ()" message that names neither the photograph nor the ratio.
NARROWEST, WIDEST = 0.8, 1.91


def aspect_safe_copy(source: Path, folder: Path, name: str) -> Path:
    """Centre-crop into Instagram's accepted range, only when necessary.

    A panorama or a tall crop is otherwise rejected at publish time. Cropping
    happens on a copy; the photographer's original is never touched.
    """
    width, height = image_dimensions(source)
    if width <= 0 or height <= 0:
        return source
    ratio = width / height
    if NARROWEST <= ratio <= WIDEST:
        return source
    if ratio > WIDEST:
        target_width, target_height = int(height * WIDEST), height
    else:
        target_width, target_height = width, int(width / NARROWEST)
    cropped = folder / f"crop-{name}"
    result = subprocess.run(
        ["/usr/bin/sips", "--cropToHeightWidth", str(target_height), str(target_width),
         str(source), "--out", str(cropped)],
        capture_output=True, text=True,
    )
    if result.returncode != 0 or not cropped.is_file():
        raise RuntimeError(
            f"Could not fit {source.name} ({width}x{height}, ratio {ratio:.2f}) to Instagram's "
            f"accepted range: {result.stderr.strip()}"
        )
    return cropped


def instagram_ready_copy(source: Path, folder: Path, name: str = "socialflow-instagram.jpg") -> Path:
    """Create a standard-size sRGB JPEG without altering the photographer's file."""
    source = aspect_safe_copy(source, folder, name)
    output = folder / name
    result = subprocess.run(
        [
            "/usr/bin/sips",
            "-s", "format", "jpeg",
            "-s", "formatOptions", "92",
            "-s", "profile", "/System/Library/ColorSync/Profiles/sRGB Profile.icc",
            "--resampleWidth", "1440",
            str(source),
            "--out", str(output),
        ],
        capture_output=True,
        text=True,
    )
    if result.returncode != 0 or not output.is_file() or output.stat().st_size == 0:
        raise RuntimeError(f"Could not prepare an Instagram-compatible JPEG: {result.stderr.strip()}")
    return output


def facebook_cdn_url_for_image(db: sqlite3.Connection, image_id: int) -> str:
    """Reuse Meta's own CDN when the same photograph is already on the Page."""
    mirror = db.execute(
        """SELECT fp.facebook_post_id,fa.page_id FROM posts fp
           JOIN post_images fpi ON fpi.post_id=fp.id AND fpi.image_id=?
           JOIN facebook_accounts fa ON fa.profile_id=fp.profile_id AND fa.connected=1
           WHERE fp.platform='facebook' AND fp.status='published' AND fp.facebook_post_id IS NOT NULL
           ORDER BY fp.published_at DESC LIMIT 1""", (image_id,)
    ).fetchone()
    if not mirror:
        raise RuntimeError("No published Facebook copy is available as a secure Meta media fallback")
    token = subprocess.check_output(
        ["/usr/bin/security", "find-generic-password", "-s", "com.socialflow.desktop.facebook", "-a", mirror["page_id"], "-w"], text=True
    ).strip()
    result = api(
        f"https://graph.facebook.com/v23.0/{mirror['facebook_post_id']}?fields=full_picture&access_token={urllib.parse.quote(token)}",
        token,
    )
    url = result.get("full_picture")
    if not url:
        raise RuntimeError("Facebook returned no CDN photograph for the published Page post")
    return url


def publish_instagram(post_id: int) -> None:
    db = sqlite3.connect(DB)
    db.row_factory = sqlite3.Row
    row = db.execute(
        """SELECT p.id,p.caption,p.hashtags_json,p.post_type,p.asset_path,i.id image_id,i.source_path,
                  ia.instagram_user_id
           FROM posts p JOIN post_images pi ON pi.post_id=p.id AND pi.position=0
           JOIN images i ON i.id=pi.image_id
           JOIN instagram_accounts ia ON ia.profile_id=p.profile_id AND ia.connected=1
           WHERE p.id=? AND COALESCE(p.platform,'instagram')='instagram'""",
        (post_id,),
    ).fetchone()
    if not row:
        raise RuntimeError("Post or connected Instagram account was not found")
    post_type = row["post_type"]
    if post_type not in {"single", "carousel", "story_pack", "reel"}:
        raise RuntimeError(f"SocialFlow does not publish {post_type} to Instagram")
    # Every photograph of the post, in the order the photographer arranged them.
    sources = [
        Path(item["source_path"])
        for item in db.execute(
            """SELECT i.source_path FROM post_images pi JOIN images i ON i.id=pi.image_id
               WHERE pi.post_id=? ORDER BY pi.position""",
            (post_id,),
        ).fetchall()
    ]
    missing = [path.name for path in sources if not path.is_file()]
    if missing:
        raise RuntimeError(f"Original photograph is unavailable: {', '.join(missing[:3])}")
    if post_type == "reel":
        rendered = Path(row["asset_path"] or "")
        if not rendered.is_file():
            raise RuntimeError(f"Original photograph is unavailable: the rendered Reel {rendered.name or 'video'} is missing")
    if post_type == "carousel" and len(sources) < 2:
        raise RuntimeError("A carousel needs at least two photographs")
    original = sources[0]
    token = os.environ.get("SOCIALFLOW_IG_TOKEN", "").strip()
    if not token:
        token = subprocess.check_output(
            ["/usr/bin/security", "find-generic-password", "-s", "com.socialflow.desktop.instagram", "-a", row["instagram_user_id"], "-w"],
            text=True,
        ).strip()
    tags = " ".join(json.loads(row["hashtags_json"] or "[]"))
    caption = row["caption"].rstrip() + ("\n\n" + tags if tags else "")
    db.execute("UPDATE posts SET status='publishing',updated_at=CURRENT_TIMESTAMP WHERE id=?", (post_id,))
    db.execute("INSERT INTO publish_attempts(post_id,started_at,status) VALUES(?,CURRENT_TIMESTAMP,'publishing')", (post_id,))
    attempt = db.execute("SELECT last_insert_rowid()").fetchone()[0]
    db.commit()
    server = tunnel = None
    page_token, staged = None, []
    temporary = tempfile.TemporaryDirectory(prefix="socialflow-instagram-")
    try:
        account = row["instagram_user_id"]
        if post_type == "reel":
            # The rendered vertical video is served as-is; Meta transcodes it.
            urls, server, tunnel = temporary_urls([Path(row["asset_path"])])
            media_id = publish_instagram_reel(account, token, urls[0], caption)
        elif post_type == "carousel":
            folder = Path(temporary.name)
            copies = [
                instagram_ready_copy(source, folder, f"carousel-{index:02d}.jpg")
                for index, source in enumerate(sources[:10])
            ]
            urls, server, tunnel, page_token, staged = public_urls_for(db, copies)
            media_id = publish_instagram_carousel(account, token, urls, caption)
        elif post_type == "story_pack":
            # A story pack is a set of rendered 1080x1920 frames carrying the
            # couple, date, venue and a line of the caption. Publish the first
            # rendered frame — sending the raw original instead would silently
            # drop the overlay, since Instagram cannot add text itself.
            rendered = sorted(Path(row["asset_path"]).glob("story-*.jpg")) if row["asset_path"] else []
            if rendered:
                image = rendered[0]
            else:
                image = instagram_ready_copy(original, Path(temporary.name))
            urls, server, tunnel, page_token, staged = public_urls_for(db, [image])
            media_id = publish_instagram_story(account, token, urls[0])
        else:
            image = instagram_ready_copy(original, Path(temporary.name))
            urls, server, tunnel, page_token, staged = public_urls_for(db, [image])
            image_url = urls[0]
            container = api(
                f"https://graph.instagram.com/{account}/media",
                token,
                {"image_url": image_url, "caption": caption},
            )["id"]
            await_container(container, token, 120)
            media_id = api(
                f"https://graph.instagram.com/{account}/media_publish",
                token,
                {"creation_id": container},
            )["id"]
        db.execute("UPDATE posts SET status='published',instagram_media_id=?,published_at=CURRENT_TIMESTAMP,updated_at=CURRENT_TIMESTAMP WHERE id=?", (media_id, post_id))
        db.execute("UPDATE images SET used_count=used_count+1,last_used_at=CURRENT_TIMESTAMP WHERE id=?", (row["image_id"],))
        db.execute("UPDATE publish_attempts SET finished_at=CURRENT_TIMESTAMP,status='published',provider_response=? WHERE id=?", (json.dumps({"media_id": media_id}), attempt))
        db.commit()
    except Exception as error:
        db.execute("UPDATE posts SET status='failed',updated_at=CURRENT_TIMESTAMP WHERE id=?", (post_id,))
        db.execute("UPDATE publish_attempts SET finished_at=CURRENT_TIMESTAMP,status='failed',error_message=? WHERE id=?", (str(error), attempt))
        db.commit()
        raise
    finally:
        if tunnel:
            tunnel.terminate()
            try:
                tunnel.wait(timeout=5)
            except subprocess.TimeoutExpired:
                tunnel.kill()
        if server:
            server.shutdown()
        if page_token and staged:
            discard_staged_media(page_token, staged)
        temporary.cleanup()
        db.close()


def facebook_upload_photo(page_id: str, token: str, photograph: Path, published: bool,
                          message: str = "", raw: bool = False):
    """Upload one photograph to a Page. Unpublished uploads become carousel items."""
    boundary = "----SocialFlow" + secrets.token_hex(16)
    content_type = mimetypes.guess_type(photograph.name)[0] or "image/jpeg"
    body = bytearray()
    fields = {"published": "true" if published else "false"}
    if message:
        fields["message"] = message
    for name, value in fields.items():
        body.extend(f"--{boundary}\r\nContent-Disposition: form-data; name=\"{name}\"\r\n\r\n{value}\r\n".encode())
    body.extend(
        f"--{boundary}\r\nContent-Disposition: form-data; name=\"source\"; filename=\"{photograph.name}\"\r\n"
        f"Content-Type: {content_type}\r\n\r\n".encode()
    )
    body.extend(photograph.read_bytes())
    body.extend(f"\r\n--{boundary}--\r\n".encode())
    request = urllib.request.Request(
        f"https://graph.facebook.com/v23.0/{page_id}/photos", data=bytes(body), method="POST"
    )
    request.add_header("Authorization", f"Bearer {token}")
    request.add_header("Content-Type", f"multipart/form-data; boundary={boundary}")
    request.add_header("User-Agent", "SocialFlow/0.1")
    with urllib.request.urlopen(request, timeout=180) as response:
        result = json.loads(response.read())
    if raw:
        return result
    media_id = result.get("id")
    if not media_id:
        raise RuntimeError(f"Facebook returned no media ID for {photograph.name}: {json.dumps(result)}")
    return media_id


def publish_facebook(post_id: int) -> None:
    db = sqlite3.connect(DB)
    db.row_factory = sqlite3.Row
    row = db.execute(
        """SELECT p.id,p.caption,p.hashtags_json,p.post_type,i.id image_id,i.source_path,fa.page_id
           FROM posts p JOIN post_images pi ON pi.post_id=p.id AND pi.position=0
           JOIN images i ON i.id=pi.image_id
           JOIN facebook_accounts fa ON fa.profile_id=p.profile_id AND fa.connected=1
           WHERE p.id=? AND p.platform='facebook'""",
        (post_id,),
    ).fetchone()
    if not row:
        raise RuntimeError("Post, photograph or connected Facebook Page was not found")
    # Every photograph, not just the first. Sending only position 0 silently
    # published one frame of a seven-photograph post and reported success.
    photographs = [
        Path(item["source_path"])
        for item in db.execute(
            """SELECT i.source_path FROM post_images pi JOIN images i ON i.id=pi.image_id
               WHERE pi.post_id=? ORDER BY pi.position""",
            (post_id,),
        ).fetchall()
    ]
    missing = [path.name for path in photographs if not path.is_file()]
    if missing:
        raise RuntimeError(f"Original photograph is unavailable: {', '.join(missing[:3])}")
    image = photographs[0]
    token = subprocess.check_output(
        ["/usr/bin/security", "find-generic-password", "-s", "com.socialflow.desktop.facebook", "-a", row["page_id"], "-w"],
        text=True,
    ).strip()
    tags = " ".join(json.loads(row["hashtags_json"] or "[]"))
    message = row["caption"].rstrip() + ("\n\n" + tags if tags else "")
    db.execute("UPDATE posts SET status='publishing',updated_at=CURRENT_TIMESTAMP WHERE id=?", (post_id,))
    db.execute("INSERT INTO publish_attempts(post_id,started_at,status) VALUES(?,CURRENT_TIMESTAMP,'publishing')", (post_id,))
    attempt = db.execute("SELECT last_insert_rowid()").fetchone()[0]
    db.commit()
    try:
        if len(photographs) > 1:
            # Upload each photograph unpublished, then attach them all to one
            # feed post so the Page shows the whole set.
            attached = []
            for photograph in photographs[:10]:
                uploaded = facebook_upload_photo(row["page_id"], token, photograph, published=False)
                attached.append(uploaded)
            fields = {"message": message}
            for index, media_id in enumerate(attached):
                fields[f"attached_media[{index}]"] = json.dumps({"media_fbid": media_id})
            result = api(f"https://graph.facebook.com/v23.0/{row['page_id']}/feed", token, fields)
        else:
            result = facebook_upload_photo(row["page_id"], token, image, published=True, message=message, raw=True)
        external_id = result.get("post_id") or result.get("id")
        if not external_id:
            raise RuntimeError("Facebook accepted the request but returned no post ID")
        db.execute("UPDATE posts SET status='published',facebook_post_id=?,published_at=CURRENT_TIMESTAMP,updated_at=CURRENT_TIMESTAMP WHERE id=?", (str(external_id), post_id))
        db.execute("UPDATE images SET used_count=used_count+1,last_used_at=CURRENT_TIMESTAMP WHERE id=?", (row["image_id"],))
        db.execute("UPDATE publish_attempts SET finished_at=CURRENT_TIMESTAMP,status='published',provider_response=? WHERE id=?", (json.dumps(result), attempt))
        db.commit()
    except Exception as error:
        detail = getattr(error, "read", lambda: b"")().decode(errors="replace")
        message = detail or str(error)
        db.execute("UPDATE posts SET status='failed',updated_at=CURRENT_TIMESTAMP WHERE id=?", (post_id,))
        db.execute("UPDATE publish_attempts SET finished_at=CURRENT_TIMESTAMP,status='failed',error_message=? WHERE id=?", (message, attempt))
        db.commit()
        raise RuntimeError(message) from error
    finally:
        db.close()


def publish_tiktok(post_id: int) -> None:
    db = sqlite3.connect(DB)
    db.row_factory = sqlite3.Row
    row = db.execute(
        """SELECT p.id,p.caption,p.hashtags_json,p.asset_path,ta.open_id,
                  COALESCE((SELECT value FROM app_settings WHERE key='tiktok_publish_mode'),'draft') mode
           FROM posts p JOIN tiktok_accounts ta ON ta.profile_id=p.profile_id AND ta.connected=1
           WHERE p.id=? AND p.platform='tiktok' AND p.post_type='reel'""", (post_id,)
    ).fetchone()
    if not row or not row["asset_path"]:
        raise RuntimeError("TikTok Reel, generated video or connected account was not found")
    video = Path(row["asset_path"])
    if not video.is_file():
        raise RuntimeError(f"TikTok video is unavailable: {video.name}")
    token = subprocess.check_output(
        ["/usr/bin/security", "find-generic-password", "-s", "com.socialflow.desktop.tiktok", "-a", row["open_id"], "-w"], text=True
    ).strip()
    tags = " ".join(json.loads(row["hashtags_json"] or "[]"))
    title = (row["caption"].rstrip() + ("\n\n" + tags if tags else ""))[:2200]
    direct = row["mode"] == "direct"
    endpoint = "https://open.tiktokapis.com/v2/post/publish/video/init/" if direct else "https://open.tiktokapis.com/v2/post/publish/inbox/video/init/"
    size = video.stat().st_size
    payload = {"source_info": {"source": "FILE_UPLOAD", "video_size": size, "chunk_size": size, "total_chunk_count": 1}}
    if direct:
        payload["post_info"] = {"title": title, "privacy_level": "SELF_ONLY", "disable_duet": True, "disable_comment": False, "disable_stitch": True, "video_cover_timestamp_ms": 1000}
    db.execute("UPDATE posts SET status='publishing',updated_at=CURRENT_TIMESTAMP WHERE id=?", (post_id,))
    db.execute("INSERT INTO publish_attempts(post_id,started_at,status) VALUES(?,CURRENT_TIMESTAMP,'publishing')", (post_id,))
    attempt = db.execute("SELECT last_insert_rowid()").fetchone()[0]
    db.commit()
    try:
        request = urllib.request.Request(endpoint, data=json.dumps(payload).encode(), method="POST")
        request.add_header("Authorization", f"Bearer {token}")
        request.add_header("Content-Type", "application/json; charset=UTF-8")
        with urllib.request.urlopen(request, timeout=60) as response:
            result = json.loads(response.read())
        if result.get("error", {}).get("code") not in (None, "ok"):
            raise RuntimeError(json.dumps(result["error"]))
        upload_url = result.get("data", {}).get("upload_url")
        publish_id = result.get("data", {}).get("publish_id")
        if not upload_url or not publish_id:
            raise RuntimeError(f"TikTok returned no upload address: {json.dumps(result)}")
        upload = urllib.request.Request(upload_url, data=video.read_bytes(), method="PUT")
        upload.add_header("Content-Type", "video/mp4")
        upload.add_header("Content-Length", str(size))
        upload.add_header("Content-Range", f"bytes 0-{size - 1}/{size}")
        with urllib.request.urlopen(upload, timeout=300):
            pass
        final_status = "SENT_TO_TIKTOK" if not direct else "PROCESSING_UPLOAD"
        if direct:
            deadline = time.time() + 300
            while time.time() < deadline:
                status_request = urllib.request.Request(
                    "https://open.tiktokapis.com/v2/post/publish/status/fetch/",
                    data=json.dumps({"publish_id": publish_id}).encode(), method="POST",
                )
                status_request.add_header("Authorization", f"Bearer {token}")
                status_request.add_header("Content-Type", "application/json; charset=UTF-8")
                with urllib.request.urlopen(status_request, timeout=45) as response:
                    status = json.loads(response.read())
                final_status = status.get("data", {}).get("status", final_status)
                if final_status == "PUBLISH_COMPLETE":
                    break
                if final_status in {"FAILED", "PUBLISH_FAILED"}:
                    raise RuntimeError(json.dumps(status))
                time.sleep(10)
            else:
                raise RuntimeError("TikTok did not finish processing the video in time; SocialFlow will check again automatically")
        db.execute("UPDATE posts SET status='published',tiktok_publish_id=?,published_at=CURRENT_TIMESTAMP,updated_at=CURRENT_TIMESTAMP WHERE id=?", (publish_id, post_id))
        db.execute("UPDATE publish_attempts SET finished_at=CURRENT_TIMESTAMP,status='published',provider_response=? WHERE id=?", (json.dumps({"publish_id": publish_id, "status": final_status, "mode": row["mode"]}), attempt))
        db.commit()
    except Exception as error:
        detail = getattr(error, "read", lambda: b"")().decode(errors="replace")
        message = detail or str(error)
        db.execute("UPDATE posts SET status='failed',updated_at=CURRENT_TIMESTAMP WHERE id=?", (post_id,))
        db.execute("UPDATE publish_attempts SET finished_at=CURRENT_TIMESTAMP,status='failed',error_message=? WHERE id=?", (message, attempt))
        db.commit()
        raise RuntimeError(message) from error
    finally:
        db.close()


def publish(post_id: int) -> None:
    db = sqlite3.connect(DB)
    row = db.execute("SELECT COALESCE(platform,'instagram') FROM posts WHERE id=?", (post_id,)).fetchone()
    db.close()
    if not row:
        raise RuntimeError("Post was not found")
    if row[0] == "facebook":
        publish_facebook(post_id)
    elif row[0] == "tiktok":
        publish_tiktok(post_id)
    else:
        publish_instagram(post_id)
    clear_recovery(post_id)


def run_due() -> None:
    try:
        refresh_instagram_token_if_needed()
    except Exception as error:
        # Publishing recovery will surface an expired token if renewal really
        # becomes necessary; a temporary refresh outage must not stop Facebook.
        print(f"Instagram token refresh deferred: {error}", flush=True)
    try:
        refresh_tiktok_token()
    except Exception as error:
        print(f"TikTok token refresh deferred: {error}",flush=True)
    startup = sqlite3.connect(DB)
    ensure_recovery_schema(startup)
    # A terminated process must never strand a post in "publishing".
    startup.execute(
        "UPDATE posts SET status='scheduled',updated_at=CURRENT_TIMESTAMP "
        "WHERE status='publishing' AND updated_at < datetime('now','-10 minutes')"
    )
    startup.commit()
    startup.close()
    while True:
        db = sqlite3.connect(DB)
        due = db.execute(
            """SELECT p.id FROM posts p LEFT JOIN publish_recovery r ON r.post_id=p.id
               WHERE p.status='scheduled' AND p.scheduled_at<=datetime('now','localtime')
               AND (r.next_retry_at IS NULL OR r.next_retry_at<=datetime('now','localtime'))
               ORDER BY p.scheduled_at,p.id"""
        ).fetchall()
        db.close()
        for (post_id,) in due:
            try:
                publish(post_id)
            except Exception as error:
                schedule_recovery(post_id, error)
        time.sleep(30)


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--post", type=int)
    parser.add_argument("--due", action="store_true")
    args = parser.parse_args()
    if args.post:
        publish(args.post)
    else:
        run_due()
