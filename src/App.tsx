import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { convertFileSrc } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import {
  AlertTriangle,
  BookImage,
  BrainCircuit,
  BarChart3,
  CalendarDays,
  Check,
  ChevronDown,
  Clock3,
  FolderInput,
  Film,
  Facebook,
  Gauge,
  Grid3X3,
  ImagePlus,
  Images,
  Instagram,
  Library,
  Loader2,
  Megaphone,
  Music2,
  Pencil,
  Download,
  RefreshCw,
  Plus,
  Search,
  Settings,
  Sparkles,
  Users,
  WifiOff,
  X,
} from "lucide-react";
import { command, formatDate, isTauri, parseHashtags } from "./lib";
import type { AppData, ImageRecord, Post, View, Wedding } from "./types";
const views: [View, any][] = [
  ["Today", Gauge],
  ["Library", Library],
  ["Queue", Clock3],
  ["Calendar", CalendarDays],
  ["Campaigns", Megaphone],
  ["Autopilot", Sparkles],
  ["Analytics", BrainCircuit],
  ["Published", Check],
  ["Settings", Settings],
];
const platformName=(platform:Post["platform"])=>platform==="facebook"?"Facebook":platform==="tiktok"?"TikTok":"Instagram";
const blank: AppData = {
  images: [],
  posts: [],
  profiles: [],
  collections: [],
  weddings: [],
  suppliers: [],
  settings: {},
  analytics: { measured_posts: 0, last_synced_at: null, formats: [], best_times: [], recommendations: [], permission_needed: true },
  marketing: { leads: 0, booked_value: 0, failed_posts: 0, visual_duplicates_indexed: 0, last_backup_at: null, token_expiry: null },
};
const photoSrc = (p?: string | null) =>
  p ? (isTauri() ? convertFileSrc(p) : `file://${p}`) : "";
const momentLabel = (value?: string | null) =>
  value
    ? value
        .replace(/[_-]+/g, " ")
        .replace(/\b\w/g, (letter) => letter.toUpperCase())
    : "";
export default function App() {
  const [data, setData] = useState(blank),
    [view, setView] = useState<View>("Today"),
    [busy, setBusy] = useState(""),
    [error, setError] = useState(""),
    [toast, setToast] = useState(""),
    [query, setQuery] = useState(""),
    [selected, setSelected] = useState(new Set<number>()),
    [preview, setPreview] = useState<ImageRecord | null>(null),
    [weddingOpen, setWeddingOpen] = useState(false),
    [editingWedding, setEditingWedding] = useState<Wedding | null>(null),
    [campaignWedding, setCampaignWedding] = useState<Wedding | null>(null),
    [editing, setEditing] = useState<Post | null>(null),
    [backgroundAnalysis, setBackgroundAnalysis] = useState<
      Record<number, "queued" | "running">
    >({});
  const analysisQueue = useRef(Promise.resolve());
  // Autopilot re-enters whenever data refreshes, and the "already prepared"
  // marker is only written at the end of a long run. Without this guard a
  // refresh mid-run starts a second preparation and duplicates the week.
  const autopilotRunning = useRef(false);
  const refresh = useCallback(async () => {
    try {
      setData(await command<AppData>("load_data"));
    } catch (e) {
      setError(String(e));
    }
  }, []);
  useEffect(() => {
    refresh();
  }, [refresh]);
  useEffect(() => {
    // Publishing runs in a separate durable worker, so its SQLite updates do
    // not emit a Tauri event into this window. Refresh while queue state can
    // still change and whenever the user returns to SocialFlow.
    const backgroundRefresh = () => {
      if (document.visibilityState === "visible") {
        command<AppData>("load_data").then(setData).catch(() => undefined);
      }
    };
    const changing = data.posts.some((post) =>
      ["scheduled", "publishing", "failed"].includes(post.status),
    );
    const timer = changing ? window.setInterval(backgroundRefresh, 5_000) : undefined;
    window.addEventListener("focus", backgroundRefresh);
    document.addEventListener("visibilitychange", backgroundRefresh);
    return () => {
      if (timer) window.clearInterval(timer);
      window.removeEventListener("focus", backgroundRefresh);
      document.removeEventListener("visibilitychange", backgroundRefresh);
    };
  }, [data.posts]);
  const scheduledPosts = data.posts.filter((post) => post.status === "scheduled").length;
  useEffect(() => {
    if (!data.profiles.length || !scheduledPosts) return;
    const keepPublisherHealthy = () => command("start_live_publisher").catch((e) => setError(String(e)));
    keepPublisherHealthy();
    // The worker normally stays alive. This watchdog restarts it after a crash,
    // app update or macOS interruption without waiting for the user to notice.
    const watchdog = window.setInterval(keepPublisherHealthy, 60_000);
    return () => window.clearInterval(watchdog);
  }, [data.profiles.length, scheduledPosts]);
  useEffect(() => {
    if (!data.profiles.length) return;
    const last = data.marketing.last_backup_at ? new Date(data.marketing.last_backup_at).getTime() : 0;
    if (Date.now() - last < 24 * 60 * 60 * 1000) return;
    command("backup_socialflow").then(refresh).catch(() => undefined);
  }, [data.marketing.last_backup_at, data.profiles.length, refresh]);
  useEffect(() => {
    if (data.settings.autopilot_enabled !== "true" || new Date().getDay() !== 0) return;
    const week = new Date().toISOString().slice(0, 10);
    if (data.settings.autopilot_last_prepared === week || autopilotRunning.current) return;
    const eligible = data.weddings.filter((w) =>
      w.collection_id && !["none", "portfolio_only"].includes(w.consent_level) &&
      (!w.embargo_until || new Date(w.embargo_until) <= new Date()));
    if (!eligible.length) return;
    autopilotRunning.current = true;
    (async () => {
      // Claim the week before the long run, not after it.
      await command("set_setting", { key: "autopilot_last_prepared", value: week });
      await command("index_visual_duplicates");
      let remaining = 105;
      for (let index = 0; index < eligible.length && remaining > 0; index++) {
        const wedding = eligible[index];
        const imageIds = data.images.filter((image) =>
          image.collection_id === wedding.collection_id && image.analysis_status === "completed").map((image) => image.id);
        if (!imageIds.length) continue;
        const count = Math.ceil(remaining / (eligible.length - index));
        await command("create_content_campaign", { profileId: 1, imageIds, count, postsPerDay: 5, weddingId: wedding.id, formats: ["carousel", "reel", "single", "story_pack"] });
        remaining -= count;
      }
      await refresh();
      setToast("Sunday campaign prepared and waiting for review");
    })()
      .catch((reason) => setError(String(reason)))
      .finally(() => { autopilotRunning.current = false; });
  }, [data.images, data.settings, data.weddings, refresh]);
  useEffect(() => {
    const key = (e: KeyboardEvent) => {
      if (e.metaKey && e.key === ",") {
        e.preventDefault();
        setView("Settings");
      }
      if (e.metaKey && e.key.toLowerCase() === "o") {
        e.preventDefault();
        importMedia(e.shiftKey);
      }
      if (e.metaKey && e.key === "a" && view === "Library") {
        e.preventDefault();
        setSelected(new Set(data.images.map((x) => x.id)));
      }
      if (e.key === "Escape") {
        setPreview(null);
        setEditing(null);
        setWeddingOpen(false);
        setEditingWedding(null);
        setCampaignWedding(null);
      }
      if (e.key === " " && selected.size === 1) {
        e.preventDefault();
        setPreview(data.images.find((x) => selected.has(x.id)) || null);
      }
    };
    addEventListener("keydown", key);
    return () => removeEventListener("keydown", key);
  }, [data.images, selected, view]);
  const run = async (label: string, fn: () => Promise<any>, done?: string) => {
    try {
      setBusy(label);
      await fn();
      await refresh();
      if (done) setToast(done);
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy("");
    }
  };
  const queueMomentAnalysis = (wedding: Wedding, imageIds: number[]) => {
    if (!wedding.collection_id || backgroundAnalysis[wedding.id]) return;
    setBackgroundAnalysis((jobs) => ({ ...jobs, [wedding.id]: "queued" }));
    analysisQueue.current = analysisQueue.current
      .catch(() => undefined)
      .then(async () => {
        setBackgroundAnalysis((jobs) => ({ ...jobs, [wedding.id]: "running" }));
        try {
          await command("organise_collection", {
            collectionId: wedding.collection_id,
          });
          if (data.settings.allow_cloud_ai === "true") {
            await command("analyse_images_ai", {
              imageIds,
              weddingId: wedding.id,
            });
          }
          await refresh();
          setToast(`${wedding.couple_names} moments organised`);
        } catch (e) {
          setError(`${wedding.couple_names}: ${String(e)}`);
        } finally {
          setBackgroundAnalysis((jobs) => {
            const next = { ...jobs };
            delete next[wedding.id];
            return next;
          });
        }
      });
  };
  useEffect(() => {
    const pendingWeddingId = Number(data.settings.pending_analysis_wedding_id || 0);
    if (!pendingWeddingId || backgroundAnalysis[pendingWeddingId]) return;
    const wedding = data.weddings.find((item) => item.id === pendingWeddingId);
    if (!wedding?.collection_id) return;
    const imageIds = data.images
      .filter((image) => image.collection_id === wedding.collection_id)
      .map((image) => image.id);
    if (!imageIds.length) return;
    command("set_setting", { key: "pending_analysis_wedding_id", value: "" })
      .then(() => queueMomentAnalysis(wedding, imageIds))
      .catch((e) => setError(String(e)));
  }, [data.settings.pending_analysis_wedding_id, data.weddings, data.images]);
  const importMedia = async (folder = false, paths?: string[]) => {
    let chosen: any = paths;
    if (!chosen)
      chosen = await open({
        directory: folder,
        multiple: true,
        filters: folder
          ? undefined
          : [
              {
                name: "Photographs",
                extensions: ["jpg", "jpeg", "png", "heic", "webp"],
              },
            ],
      });
    if (!chosen) return;
    const arr = Array.isArray(chosen) ? chosen : [chosen];
    await run("Indexing photographs…", async () => {
      const r = await command<{ imported: number; duplicates: number }>(
        "import_paths",
        { paths: arr, recursive: true, profileId: 1 },
      );
      setToast(`${r.imported} imported · ${r.duplicates} duplicates skipped`);
    });
  };
  useEffect(() => {
    const over = (e: DragEvent) => e.preventDefault(),
      drop = (e: DragEvent) => {
        e.preventDefault();
        const paths = [...(e.dataTransfer?.files || [])]
          .map((f: any) => f.path)
          .filter(Boolean);
        if (paths.length) importMedia(false, paths);
      };
    addEventListener("dragover", over);
    addEventListener("drop", drop);
    return () => {
      removeEventListener("dragover", over);
      removeEventListener("drop", drop);
    };
  }, []);
  const createPost = () =>
    run(
      "Creating post…",
      () => command("create_post", { imageIds: [...selected], profileId: 1 }),
      "Draft created",
    ).then(() => {
      setSelected(new Set());
      setView("Queue");
    });
  return (
    <div className="app">
      <aside>
        <div className="brand">
          <div className="mark">
            <img src="/socialflow-icon.png" alt="" />
          </div>
          <b>SocialFlow</b>
        </div>
        <nav>
          {views.map(([v, I]) => (
            <button
              key={v}
              className={view === v ? "active" : ""}
              onClick={() => setView(v)}
            >
              <I size={17} />
              {v}
              {v === "Queue" && (
                <em>
                  {data.posts.filter((p) => p.status !== "published").length}
                </em>
              )}
            </button>
          ))}
        </nav>
        <div className="account">
          <div>
            <span>The Bearded Wedding Photographer</span>
            <small>Wedding marketing</small>
          </div>
          <ChevronDown size={14} />
          <p>
            <i className="dot amber" />
            Instagram ·{" "}
            {data.settings.mock_publish === "true"
              ? "Setup needed"
              : "Connected"}
          </p>
          <p>
            <i
              className={
                "dot " +
                (data.settings.claude_installed === "true" ? "green" : "amber")
              }
            />
            Claude ·{" "}
            {data.settings.claude_installed === "true"
              ? "Installed"
              : "Setup needed"}
          </p>
        </div>
      </aside>
      <main>
        <Header
          view={view}
          query={query}
          setQuery={setQuery}
          importMedia={importMedia}
          newWedding={() => setWeddingOpen(true)}
        />
        {error && (
          <div className="error">
            <WifiOff size={16} />
            <span>{error}</span>
            <button onClick={() => setError("")}>
              <X size={15} />
            </button>
          </div>
        )}
        {view === "Today" && (
          <Today
            data={data}
            newWedding={() => setWeddingOpen(true)}
            setView={setView}
            edit={setEditing}
          />
        )}{" "}
        {view === "Library" && (
          <LibraryView
            data={data}
            query={query}
            selected={selected}
            setSelected={setSelected}
            preview={setPreview}
            importMedia={importMedia}
            createPost={createPost}
            editWedding={setEditingWedding}
            run={run}
            queueMomentAnalysis={queueMomentAnalysis}
            backgroundAnalysis={backgroundAnalysis}
          />
        )}{" "}
        {view === "Queue" && (
          <Queue data={data} refresh={refresh} run={run} edit={setEditing} />
        )}{" "}
        {view === "Calendar" && <Calendar data={data} edit={setEditing} />}{" "}
        {view === "Campaigns" && (
          <Campaigns data={data} build={setCampaignWedding} />
        )}{" "}
        {view === "Autopilot" && (
          <Autopilot data={data} run={run} setView={setView} />
        )}{" "}
        {view === "Analytics" && <AnalyticsBrain data={data} run={run} />}{" "}
        {view === "Published" && <Published data={data} />}{" "}
        {view === "Settings" && (
          <SettingsView data={data} refresh={refresh} error={setError} />
        )}
      </main>
      {busy && (
        <div className="busy">
          <Loader2 className="spin" /> {busy}
        </div>
      )}
      {Object.keys(backgroundAnalysis).length > 0 && (
        <div className="background-job">
          <Loader2 className="spin" size={16} />
          <div>
            <b>Organising moments in the background</b>
            <span>
              {Object.values(backgroundAnalysis).filter((x) => x === "running").length} running · {Object.values(backgroundAnalysis).filter((x) => x === "queued").length} queued
            </span>
          </div>
        </div>
      )}
      {toast && (
        <div className="toast" onAnimationEnd={() => setToast("")}>
          {toast}
        </div>
      )}
      {preview && (
        <ImageDetail image={preview} close={() => setPreview(null)} />
      )}{" "}
      {weddingOpen && (
        <WeddingWizard
          data={data}
          close={() => setWeddingOpen(false)}
          run={run}
        />
      )}{" "}
      {editingWedding && (
        <WeddingWizard
          data={data}
          wedding={editingWedding}
          close={() => setEditingWedding(null)}
          run={run}
        />
      )}{" "}
      {campaignWedding && (
        <CampaignWizard
          wedding={campaignWedding}
          data={data}
          close={() => setCampaignWedding(null)}
          run={run}
          openQueue={() => setView("Queue")}
        />
      )}{" "}
      {editing && (
        <PostEditor post={editing} close={() => setEditing(null)} run={run} />
      )}
    </div>
  );
}
function Header({ view, query, setQuery, importMedia, newWedding }: any) {
  return (
    <header>
      <div>
        <h1>{view}</h1>
        <span>
          {view === "Today"
            ? "What needs your attention"
            : view === "Library"
              ? "Finished photographs, safely indexed"
              : view === "Queue"
                ? "One place to review and approve"
                : view === "Campaigns"
                  ? "One wedding into months of marketing"
                : view === "Analytics"
                    ? "Learn what earns attention, saves and enquiries"
                    : view === "Autopilot"
                      ? "Prepare a week, review each morning, improve continuously"
                  : view === "Calendar"
                    ? "Your publishing rhythm"
                    : view === "Published"
                      ? "Confirmed publishing history"
                      : "Brand and integrations"}
        </span>
      </div>
      <div className="head-actions">
        {view === "Library" && (
          <label className="search">
            <Search size={15} />
            <input
              value={query}
              onChange={(e) => setQuery(e.target.value)}
              placeholder="confetti, speeches, dancing…"
            />
          </label>
        )}
        <button onClick={() => importMedia(false)}>
          <ImagePlus size={16} /> Photos
        </button>
        <button className="primary" onClick={newWedding}>
          <Plus size={16} /> New Wedding
        </button>
      </div>
    </header>
  );
}
function Today({ data, newWedding, setView, edit }: any) {
  const review = data.posts.filter((p: Post) => p.status === "needs_review"),
    failed = data.posts.filter((p: Post) => p.status === "failed"),
    unanalysed = data.images.filter(
      (i: ImageRecord) => i.analysis_status !== "completed",
    );
  const organisedCount = data.images.length - unanalysed.length;
  const stages = [
    {
      label: "Import weddings",
      done: data.weddings.length > 0 && data.images.length > 0,
    },
    {
      label: `Organise moments (${organisedCount}/${data.images.length})`,
      done: data.images.length > 0 && unanalysed.length === 0,
    },
    { label: "Build campaign", done: data.posts.length > 0 },
    {
      label: "Review once",
      done: data.posts.some((post: Post) =>
        ["approved", "scheduled", "publishing", "published"].includes(
          post.status,
        ),
      ),
    },
    {
      label: "Publish",
      done: data.posts.some((post: Post) => post.status === "published"),
    },
  ];
  const currentStage = stages.findIndex((stage) => !stage.done);
  return (
    <section className="content today">
      <div className="welcome">
        <div>
          <span className="eyebrow">MARKETING INBOX</span>
          <h2>Morning, Wayne.</h2>
          <p>Only the work that needs a decision appears here.</p>
        </div>
        <button className="primary" onClick={newWedding}>
          <Plus size={16} /> Import a wedding
        </button>
      </div>
      <div className="action-grid">
        <Action
          count={review.length}
          title="Posts ready for review"
          detail="Photographs, captions and dates are prepared"
          action="Review campaign"
          onClick={() => setView("Queue")}
        />
        <Action
          count={unanalysed.length}
          title="Photographs need organising"
          detail="Run local grouping or Claude visual analysis"
          action="Open library"
          onClick={() => setView("Library")}
        />
        <Action
          count={failed.length}
          title="Publishing failures"
          detail="Nothing retries without your approval"
          action="Inspect failures"
          danger
          onClick={() => setView("Queue")}
        />
      </div>
      <div className="pipeline">
        <h3>Your streamlined workflow</h3>
        <div>
          {stages.map((stage, i) => (
            <span
              key={stage.label}
              className={
                stage.done ? "done" : i === currentStage ? "current" : ""
              }
            >
              <b>{i + 1}</b>
              {stage.label}
            </span>
          ))}
        </div>
      </div>
      <div className="recent">
        <h3>Next scheduled</h3>
        {data.posts
          .filter((p: Post) => p.status === "scheduled")
          .slice(0, 4)
          .map((p: Post) => (
            <MiniPost p={p} onClick={() => edit(p)} />
          ))}
        {!data.posts.some((p: Post) => p.status === "scheduled") && (
          <p className="muted-copy">
            Approve a campaign and its planned dates become your schedule.
          </p>
        )}
      </div>
    </section>
  );
}
function Action({ count, title, detail, action, onClick, danger }: any) {
  return (
    <article className={"action-card " + (danger && count ? "danger" : "")}>
      <strong>{count}</strong>
      <div>
        <b>{title}</b>
        <p>{detail}</p>
      </div>
      <button onClick={onClick}>{action}</button>
    </article>
  );
}
function LibraryView({
  data,
  query,
  selected,
  setSelected,
  preview,
  importMedia,
  createPost,
  editWedding,
  run,
  queueMomentAnalysis,
  backgroundAnalysis,
}: any) {
  const [collectionFilter, setCollectionFilter] = useState<number | null>(null);
  const [weddingFilterId, setWeddingFilterId] = useState<number | null>(null);
  const [sectionFilter, setSectionFilter] = useState<string | null>(null);
  const imgs = useMemo(
    () =>
      data.images.filter(
        (x: ImageRecord) =>
          (weddingFilterId !== null && collectionFilter === null
            ? false
            : collectionFilter === null || x.collection_id === collectionFilter) &&
          (sectionFilter === null || x.category === sectionFilter) &&
          `${x.filename} ${x.description || ""} ${x.category || ""}`
            .toLowerCase()
            .includes(query.toLowerCase()),
      ),
    [data.images, query, collectionFilter, weddingFilterId, sectionFilter],
  );
  const chooseCollection = (collectionId: number | null) => {
    setCollectionFilter(collectionId);
    setWeddingFilterId(null);
    setSectionFilter(null);
    setSelected(new Set());
  };
  const chooseWedding = (wedding: Wedding) => {
    setWeddingFilterId(wedding.id);
    setCollectionFilter(wedding.collection_id);
    setSectionFilter(null);
    setSelected(new Set());
  };
  const filteredWedding = data.weddings.find(
    (w: Wedding) => w.id === weddingFilterId,
  );
  const filteredWeddingImages = filteredWedding
    ? data.images.filter(
        (image: ImageRecord) =>
          image.collection_id === filteredWedding.collection_id,
      )
    : [];
  const availableSections = useMemo(() => {
    if (collectionFilter === null) return [];
    const counts = new Map<string, number>();
    data.images
      .filter(
        (image: ImageRecord) =>
          image.collection_id === collectionFilter &&
          image.category &&
          image.category.toLowerCase() !== "wedding",
      )
      .forEach((image: ImageRecord) =>
        counts.set(image.category!, (counts.get(image.category!) || 0) + 1),
      );
    return [...counts.entries()].sort(([a], [b]) => a.localeCompare(b));
  }, [data.images, collectionFilter]);
  const momentsRemaining = filteredWeddingImages.filter(
    (image: ImageRecord) => image.analysis_status !== "completed",
  ).length;
  const organiseWedding = () => {
    if (!filteredWedding?.collection_id) return;
    queueMomentAnalysis(
      filteredWedding,
      filteredWeddingImages.map((image: ImageRecord) => image.id),
    );
  };
  const pick = (e: any, id: number) =>
    setSelected((old: Set<number>) => {
      const n = new Set(e.metaKey ? old : []);
      n.has(id) ? n.delete(id) : n.add(id);
      return n;
    });
  return (
    <section className="content">
      <div className="toolbar">
        <div>
          <button
            className={collectionFilter === null && weddingFilterId === null ? "active" : ""}
            onClick={() => chooseCollection(null)}
            aria-pressed={collectionFilter === null}
          >
            <Grid3X3 size={15} /> All photographs
          </button>
          {data.weddings.map((w: Wedding) => (
            <button
              key={w.id}
              className={
                "chip " +
                (weddingFilterId === w.id ? "active" : "")
              }
              onClick={() => chooseWedding(w)}
              aria-pressed={weddingFilterId === w.id}
            >
              {w.couple_names}
            </button>
          ))}
          {filteredWedding && (
            <>
              <button onClick={() => editWedding(filteredWedding)}>
                <Pencil size={13} /> Edit wedding details
              </button>
              <button
                className="primary"
                onClick={organiseWedding}
                disabled={
                  momentsRemaining === 0 || !!backgroundAnalysis[filteredWedding.id]
                }
              >
                <Sparkles size={13} />
                {backgroundAnalysis[filteredWedding.id] === "running"
                  ? "Organising in background…"
                  : backgroundAnalysis[filteredWedding.id] === "queued"
                    ? "Queued…"
                    : momentsRemaining > 0
                  ? `AI organise ${momentsRemaining} remaining`
                  : "Moments organised"}
              </button>
            </>
          )}
        </div>
        <span>{imgs.length} photographs · originals untouched</span>
      </div>
      {filteredWedding && !filteredWedding.collection_id && (
        <div className="empty compact empty-wedding">
          <FolderInput />
          <h2>{filteredWedding.couple_names} has no photographs yet</h2>
          <p>Open the wedding details to choose its finished gallery folder or attach an existing collection.</p>
          <button className="primary" onClick={() => editWedding(filteredWedding)}><FolderInput size={14}/> Add photographs</button>
        </div>
      )}
      {availableSections.length > 0 && (
        <div className="section-bar" aria-label="Wedding-day moments">
          <span>Moments</span>
          <button
            className={sectionFilter === null ? "active" : ""}
            onClick={() => setSectionFilter(null)}
          >
            All moments <small>{filteredWeddingImages.length}</small>
          </button>
          {availableSections.map(([section, count]) => (
            <button
              key={section}
              className={sectionFilter === section ? "active" : ""}
              onClick={() => setSectionFilter(section)}
            >
              {momentLabel(section)} <small>{count}</small>
            </button>
          ))}
        </div>
      )}
      {!imgs.length ? (
        <div className="empty">
          <ImagePlus />
          <h2>Drop in a finished wedding</h2>
          <p>
            SocialFlow indexes the gallery in place, creates previews and
            prepares it for marketing.
          </p>
          <button className="primary" onClick={() => importMedia(true)}>
            <FolderInput size={16} /> Choose wedding folder
          </button>
        </div>
      ) : (
        <div className="grid">
          {imgs.map((im: ImageRecord) => (
            <article
              key={im.id}
              className={selected.has(im.id) ? "selected" : ""}
              onClick={(e) => pick(e, im.id)}
              onDoubleClick={() => preview(im)}
            >
              <div className="photo">
                {im.thumbnail_path ? (
                  <img src={photoSrc(im.thumbnail_path)} />
                ) : (
                  <BookImage />
                )}
                {im.missing && <b>FILE UNAVAILABLE</b>}
                {im.analysis_status === "completed" && (
                  <Sparkles className="ai" size={15} />
                )}
                {im.category && im.category.toLowerCase() !== "wedding" && (
                  <span className="moment-badge">
                    {momentLabel(im.category)}
                  </span>
                )}
              </div>
              <p>{im.filename}</p>
              <small>
                {im.description
                  ?.replace("Local provisional grouping: ", "")
                  .split(".")[0] || "Not organised"}{" "}
                · {im.social_score || "—"}
              </small>
            </article>
          ))}
        </div>
      )}
      {selected.size > 0 && (
        <div className="selection">
          <b>{selected.size} selected</b>
          <button className="primary" onClick={createPost}>
            Create post/carousel
          </button>
          <button onClick={() => setSelected(new Set())}>Clear</button>
        </div>
      )}
    </section>
  );
}
function Queue({ data, refresh, run, edit }: any) {
  const approve = () =>
    run(
      "Approving and filling schedule…",
      () => command("approve_all"),
      "Campaign approved",
    );
  return (
    <section className="content">
      <div className="tabs">
        <b>Needs review</b>
        <span>Approved</span>
        <span>Scheduled</span>
        <span>Failed</span>
        <button className="primary push-right" onClick={approve}>
          <Check size={15} /> Approve all & schedule
        </button>
      </div>
      {!data.posts.length ? (
        <div className="empty compact">
          <Clock3 />
          <h2>No campaign waiting</h2>
          <p>Import a wedding, then create its campaign.</p>
        </div>
      ) : (
        <div className="review-list">
          {data.posts
            .filter((p: Post) => p.status !== "published")
            .map((p: Post) => (
              <article
                className="review-row"
                key={p.id}
                onClick={() => edit(p)}
              >
                <div className="post-thumb">
                  {p.images[0]?.thumbnail_path ? (
                    <img src={photoSrc(p.images[0].thumbnail_path)} />
                  ) : (
                    <BookImage />
                  )}
                  {(p.images.length > 1 || p.post_type !== "single") && (
                    <i>{p.post_type === "single" ? p.images.length : p.post_type.replace("_", " ")}</i>
                  )}
                </div>
                <div className="review-caption">
                  <textarea readOnly value={p.caption} />
                  <small>{p.hashtags.join(" ")}</small>
                </div>
                <div className="status">
                  <small>{platformName(p.platform)}</small>
                  <span>{p.status.replace("_", " ")}</span>
                  <small>{formatDate(p.scheduled_at)}</small>
                  <button>Edit</button>
                </div>
              </article>
            ))}
        </div>
      )}
    </section>
  );
}
function Campaigns({ data, build }: any) {
  return (
    <section className="content">
      <div className="campaign-hero">
        <div>
          <span className="eyebrow">WEDDING CAMPAIGNS</span>
          <h2>One gallery. Months of useful marketing.</h2>
          <p>
            Balanced documentary moments, venue authority, supplier credits and
            occasional enquiry prompts.
          </p>
        </div>
      </div>
      <div className="wedding-list">
        {data.weddings.map((w: Wedding) => {
          const n = data.images.filter(
            (x: ImageRecord) =>
              data.collections.find((c: any) => c.id === x.collection_id)
                ?.id === w.collection_id,
          ).length;
          return (
            <article>
              <div>
                <Users />
                <span>
                  <b>{w.couple_names}</b>
                  <small>
                    {w.venue || "Venue not set"} · {n} photographs ·{" "}
                    {w.consent_level.replace("_", " ")}
                  </small>
                </span>
              </div>
              <button
                className="primary"
                disabled={w.consent_level === "none"}
                onClick={() => build(w)}
              >
                <Sparkles size={15} /> Build campaign
              </button>
            </article>
          );
        })}
        {!data.weddings.length && (
          <div className="empty compact">
            <Megaphone />
            <h2>No weddings prepared yet</h2>
            <p>Create a wedding and attach its imported collection.</p>
          </div>
        )}
      </div>
    </section>
  );
}
function Autopilot({ data, run, setView }: any) {
  const [leadValue, setLeadValue] = useState("");
  const enabled = data.settings.autopilot_enabled === "true";
  const eligible = data.weddings.filter(
    (w: Wedding) =>
      w.collection_id &&
      !["none", "portfolio_only"].includes(w.consent_level) &&
      (!w.embargo_until || new Date(w.embargo_until) <= new Date()),
  );
  const ready = data.images.filter(
    (image: ImageRecord) => image.analysis_status === "completed",
  ).length;
  const failed = data.posts.filter((post: Post) => post.status === "failed").length;
  const waiting = data.posts.filter((post: Post) => post.status === "needs_review").length;
  const month = new Date().getMonth();
  const seasonalAdvice = month <= 1 || month === 11
    ? "Winter: lean into atmosphere, candlelight, speeches and evening energy."
    : month <= 4 ? "Spring: prioritise anticipation, colour, confetti and fresh venue stories."
    : month <= 7 ? "Summer: show full-day variety, outdoor moments and lively receptions."
    : "Autumn: favour warm light, texture, family connection and indoor storytelling.";
  const buildWeek = () =>
    run(
      "Building your seven-day marketing plan…",
      async () => {
        if (!eligible.length) throw Error("No marketing-approved weddings are ready");
        await command("index_visual_duplicates");
        // One wedding per day: seven days, seven couples, seven posts each —
        // five carousels, one Reel and one Story. reflow_wedding_rotation then
        // keeps each day to a single wedding.
        const dailyQuota = { carousel: 5, reel: 1, story_pack: 1 };
        const perDay = 7;
        const days = Math.min(7, eligible.length);
        let produced = 0;
        for (let index = 0; index < days; index++) {
          const wedding = eligible[index];
          const imageIds = data.images
            .filter(
              (image: ImageRecord) =>
                image.collection_id === wedding.collection_id &&
                image.analysis_status === "completed",
            )
            .map((image: ImageRecord) => image.id);
          if (!imageIds.length) continue;
          await command("create_content_campaign", {
            profileId: 1,
            imageIds,
            count: perDay,
            postsPerDay: perDay,
            weddingId: wedding.id,
            formats: ["carousel", "reel", "story_pack"],
            dailyQuota,
            formatOffset: produced,
          });
          produced += perDay;
        }
      },
      "Seven-day plan prepared for review",
    ).then(() => setView("Queue"));
  const backup = () => run("Creating a safe database backup…", () => command("backup_socialflow"), "SocialFlow backup completed");
  const recover = () => run("Returning failed posts for review…", () => command("return_failed_to_review"), "Failed posts are ready for review");
  const recordLead = () => run(
    "Recording marketing outcome…",
    () => command("record_marketing_lead", { sourcePostId: null, source: "website enquiry", value: Number(leadValue || 0) }),
    "Website enquiry added to the learning record",
  ).then(() => setLeadValue(""));
  return (
    <section className="content autopilot">
      <div className="autopilot-hero">
        <span className="eyebrow">MARKETING AUTOPILOT</span>
        <h2>One wedding a day. Five carousels, a Reel and a Story. One approval session.</h2>
        <p>Seven days, seven couples. SocialFlow chooses the photographs, story mix, captions, North West hashtags and the times your audience actually engages. Nothing publishes until you approve it.</p>
        <button className="primary" onClick={buildWeek} disabled={!eligible.length || ready === 0}>
          <Sparkles size={16}/> Prepare the next seven days
        </button>
      </div>
      {data.settings.publisher_action_required && (
        <div className="permission-banner"><AlertTriangle size={16}/><span>
          <b>SocialFlow has protected a post that needs your approval</b>
          <small>{data.settings.publisher_action_required} Nothing has been discarded, and automatic publishing will continue for unaffected accounts.</small>
        </span><button onClick={() => setView("Settings")}>Resolve now</button></div>
      )}
      <div className="autopilot-grid">
        <article><b>{eligible.length}</b><span>marketing-approved weddings</span><small>Rotated by day so each day tells one coherent story.</small></article>
        <article><b>{ready}</b><span>AI-organised photographs</span><small>Ranked for quality, emotion, variety and learned engagement.</small></article>
        <article><b>{waiting}</b><span>posts awaiting approval</span><small><button onClick={() => setView("Queue")}>Open morning review</button></small></article>
        <article className={failed ? "warning-card" : ""}><b>{failed}</b><span>posts needing intervention</span><small>{failed ? "Protected after automatic recovery could not safely finish." : "Publishing health is clear."}</small></article>
      </div>
      <div className="autopilot-rules">
        <h3>What the brain checks automatically</h3>
        <div>
          <span>✓ Consent and embargo safety</span><span>✓ Near-duplicate and repeat-use control</span>
          <span>✓ Balanced wedding-day storytelling</span><span>✓ Caption and hashtag repetition</span>
          <span>✓ Best format and posting-time evidence</span><span>✓ Venue and supplier context</span>
          <span>✓ Content-fatigue warnings</span><span>✓ Failed-post recovery queue</span>
        </div>
      </div>
      <div className="autopilot-health">
        <span><b>This week’s marketing brief</b><small>{seasonalAdvice} {data.analytics.recommendations[0]?.recommendation || "The brain will refine this after more posts are measured."}</small></span>
        <button onClick={() => setView("Analytics")}>View evidence</button>
      </div>
      {data.marketing.token_expiry && new Date(data.marketing.token_expiry).getTime() - Date.now() < 14 * 86400000 && (
        <div className="permission-banner"><AlertTriangle size={16}/><span><b>Instagram connection needs attention soon</b><small>The access token expires {formatDate(data.marketing.token_expiry)}. Replace it in Settings before publishing is interrupted.</small></span></div>
      )}
      <div className="autopilot-health">
        <span><b>Sunday campaign preparation</b><small>{enabled ? "On — prepares the next seven days every Sunday, if SocialFlow is open" : "Off — use Prepare the next seven days manually"}</small></span>
        <button onClick={() => run(enabled ? "Pausing Sunday preparation…" : "Enabling Sunday preparation…", () => command("set_setting", { key: "autopilot_enabled", value: enabled ? "false" : "true" }), enabled ? "Sunday preparation is off" : "Sunday preparation is on")}>{enabled ? "Turn off" : "Turn on"}</button>
      </div>
      <div className="autopilot-health">
        <span><b>Recovery and backups</b><small>Last backup: {data.marketing.last_backup_at || "Not created yet"} · {data.marketing.visual_duplicates_indexed} images safety-indexed</small></span>
        <button onClick={backup}><Download size={14}/> Back up now</button>
        <button onClick={recover} disabled={!failed}><RefreshCw size={14}/> Review failed posts</button>
      </div>
      <div className="autopilot-health">
        <span><b>Enquiry and booking outcomes</b><small>{data.marketing.leads} recorded enquiries · £{data.marketing.booked_value.toLocaleString()} attributed value</small></span>
        <input aria-label="Booking value" type="number" min="0" placeholder="Booking value £" value={leadValue} onChange={event => setLeadValue(event.target.value)}/>
        <button onClick={recordLead}>Record website enquiry</button>
      </div>
    </section>
  );
}

function AnalyticsBrain({ data, run }: any) {
  const analytics = data.analytics;
  const automaticTimes = data.settings.posting_time_mode === "auto";
  const sync = () =>
    run(
      "Reading Instagram performance…",
      () => command("sync_instagram_insights"),
      "Instagram history imported · reconnect for detailed insights if prompted",
    );
  const toggleAutomaticTimes = () =>
    run(
      automaticTimes ? "Returning to suggested times…" : "Enabling smart scheduling…",
      () => command("set_setting", {
        key: "posting_time_mode",
        value: automaticTimes ? "suggest" : "auto",
      }),
      automaticTimes
        ? "Posting times are suggestions only"
        : "Future campaigns will use learned posting times",
    );
  return (
    <section className="content analytics-brain">
      <div className="brain-hero">
        <div className="brain-mark"><BrainCircuit /></div>
        <div>
          <span className="eyebrow">ANALYTICS BRAIN</span>
          <h2>Make the next campaign better than the last.</h2>
          <p>{analytics.measured_posts} Instagram posts measured · Confidence grows at 10 posts and becomes strong at 30.</p>
        </div>
        <button className="primary" onClick={sync}><RefreshCw size={15}/> Sync Instagram</button>
      </div>
      {analytics.permission_needed && <div className="permission-banner"><AlertTriangle size={16}/><span><b>Detailed insights permission needed</b><small>Add <code>instagram_business_manage_insights</code> to the Meta app, generate a replacement token, then reconnect in Settings.</small></span></div>}
      <div className="brain-grid">
        <div className="format-performance">
          <h3>Formats ranked by engagement</h3>
          {analytics.formats.map((item:any,index:number)=><article key={item.format}>
            <b>{index+1}</b><span><strong>{item.format.replace("_"," ")}</strong><small>{item.posts} measured posts</small></span><div><strong>{item.average_score.toFixed(1)}</strong><small>score</small></div><div><strong>{Math.round(item.average_reach)}</strong><small>reach</small></div>
          </article>)}
          {!analytics.formats.length&&<div className="brain-empty"><BarChart3/><p>Sync Instagram to establish your baseline.</p></div>}
        </div>
        <div className="recommendations">
          <h3>What the brain recommends</h3>
          {analytics.recommendations.map((item:any)=><article key={item.title}>
            <span>{item.confidence}</span><h4>{item.title}</h4><p>{item.recommendation}</p><small>{item.evidence}</small>
          </article>)}
        </div>
        <div className="posting-times">
          <div className="posting-times-head">
            <span>
              <h3>Your five learned posting times</h3>
              <small>{automaticTimes ? "AUTO-SCHEDULE ENABLED" : "SUGGESTIONS ONLY"}</small>
            </span>
            <button className={automaticTimes ? "primary" : ""} onClick={toggleAutomaticTimes}>
              {automaticTimes ? "Use suggestions only" : "Enable auto-schedule"}
            </button>
          </div>
          <div>
            {analytics.best_times.map((item: any, index: number) => (
              <article key={item.hour}>
                <b>{String(item.hour).padStart(2, "0")}:00</b>
                <span>#{index + 1}</span>
                <small>{item.posts} posts · {item.average_score.toFixed(1)} score</small>
              </article>
            ))}
          </div>
          {!analytics.best_times.length && <p>SocialFlow will fill these after your first Instagram sync.</p>}
          <small>{automaticTimes ? "New campaigns automatically use these learned times. Untested gaps use the balanced schedule." : "These are recommendations only. New campaigns keep the balanced schedule until you enable auto-schedule."}</small>
        </div>
        <div className="content-brain-status">
          <BrainCircuit size={20} />
          <span><b>Automatic image selection enabled</b><small>The brain combines visual quality, proven moment engagement, AI verification and image-use history, then balances the wedding-day sections.</small></span>
        </div>
      </div>
      <p className="analytics-note">The score weights shares and saves most heavily, followed by comments and likes, and normalises against reach when Instagram supplies it. SocialFlow will not change your schedule automatically—you approve every recommendation.</p>
    </section>
  );
}
function Calendar({ data, edit }: any) {
  const scheduled = data.posts.filter((p: Post) => p.scheduled_at);
  return (
    <section className="content agenda">
      <h2>Publishing agenda</h2>
      {scheduled.map((p: Post) => (
        <MiniPost p={p} onClick={() => edit(p)} />
      ))}
      {!scheduled.length && (
        <div className="empty compact">
          <CalendarDays />
          <h2>No planned dates</h2>
        </div>
      )}
    </section>
  );
}
function MiniPost({ p, onClick }: any) {
  return (
    <article className="mini-post" onClick={onClick}>
      {p.images[0]?.thumbnail_path ? (
        <img src={photoSrc(p.images[0].thumbnail_path)} />
      ) : (
        <BookImage />
      )}
      <div>
        <b>{p.caption.split("\n")[0] || "Untitled post"}</b>
        <small>{platformName(p.platform)} · {p.post_type.replace("_", " ")} · {formatDate(p.scheduled_at)}</small>
      </div>
      <span>{p.status}</span>
    </article>
  );
}
function Published({ data }: any) {
  return (
    <section className="content agenda">
      <h2>Published posts</h2>
      {data.posts
        .filter((p: Post) => p.status === "published")
        .map((p: Post) => (
          <MiniPost p={p} />
        ))}
      {!data.posts.some((p: Post) => p.status === "published") && (
        <div className="empty compact">
          <Instagram />
          <h2>Nothing published yet</h2>
          <p>Only confirmed Meta results appear here.</p>
        </div>
      )}
    </section>
  );
}
function WeddingWizard({ data, wedding, close, run }: any) {
  const [f, setF] = useState({
    coupleNames: wedding?.couple_names || "",
    weddingDate: wedding?.wedding_date || "",
    venue: wedding?.venue || "",
    region: wedding?.region || "Lancashire",
    consentLevel: wedding?.consent_level || "full",
    embargoUntil: wedding?.embargo_until || "",
    campaignGoal: wedding?.campaign_goal || "enquiries",
    collectionId: String(
      wedding?.collection_id || data.collections[0]?.id || "",
    ),
    folderPath: "",
  });
  const [suppliers, setSuppliers] = useState(
    wedding
      ? data.suppliers
          .filter((supplier: any) => supplier.wedding_id === wedding.id)
          .map((supplier: any) => ({
            role: supplier.role,
            name: supplier.name,
            instagramHandle: supplier.instagram_handle,
            website: supplier.website,
            instagramConfirmed: supplier.instagram_confirmed,
          }))
      : [],
  );
  const addSupplierRow = () =>
    setSuppliers([
      ...suppliers,
      { role: "Venue", name: "", instagramHandle: "", website: "", instagramConfirmed: false },
    ]);
  const changeSupplier = (index: number, changes: any) =>
    setSuppliers(suppliers.map((supplier: any, position: number) =>
      position === index ? { ...supplier, ...changes } : supplier));
  const saveSuppliers = (weddingId: number) =>
    command("replace_suppliers", {
      weddingId,
      suppliers: suppliers.map((supplier: any) => ({
        role: supplier.role,
        name: supplier.name,
        instagramHandle: supplier.instagramHandle,
        website: supplier.website,
        instagramConfirmed: supplier.instagramConfirmed,
      })),
    });
  const choose = async () => {
    const p = await open({ directory: true, multiple: false });
    if (p) setF({ ...f, folderPath: String(p), collectionId: "" });
  };
  const save = async () => {
    if (wedding) {
      await run(
        "Saving wedding details…",
        async () => {
          if (f.folderPath)
            await command("import_paths", {
              paths: [f.folderPath],
              recursive: true,
              profileId: 1,
            });
          await command("update_wedding", {
            weddingId: wedding.id,
            collectionId: f.collectionId ? Number(f.collectionId) : null,
            folderPath: f.folderPath || null,
            coupleNames: f.coupleNames,
            weddingDate: f.weddingDate,
            venue: f.venue,
            region: f.region,
            consentLevel: f.consentLevel,
            embargoUntil: f.embargoUntil || null,
            campaignGoal: f.campaignGoal,
          });
          await saveSuppliers(wedding.id);
        },
        "Wedding details updated",
      );
      close();
      return;
    }
    await run(
      "Importing and preparing wedding…",
      async () => {
        if (f.folderPath)
          await command("import_paths", {
            paths: [f.folderPath],
            recursive: true,
            profileId: 1,
          });
        const weddingId = await command<number>("create_wedding", {
          ...f,
          collectionId: f.collectionId ? Number(f.collectionId) : null,
          folderPath: f.folderPath || null,
          profileId: 1,
          embargoUntil: f.embargoUntil || null,
        });
        await saveSuppliers(weddingId);
      },
      "Wedding ready",
    );
    close();
  };
  return (
    <Modal close={close}>
      <div className="wizard">
        <span className="eyebrow">
          {wedding ? "EDIT WEDDING" : "NEW WEDDING"}
        </span>
        <h2>
          {wedding
            ? `Correct ${wedding.couple_names}'s details.`
            : "Tell SocialFlow what it needs to know."}
        </h2>
        <p>
          {wedding
            ? "Changes update the library and future campaign copy. Photographs remain untouched."
            : "Choose the exported gallery and enter the details once. Originals remain where they are."}
        </p>
        {(!wedding || !wedding.collection_id) && (
          <button onClick={choose}>
            <FolderInput size={15} />
            {f.folderPath
              ? f.folderPath.split("/").pop()
              : "Choose finished gallery folder"}
          </button>
        )}
        <div className="form-grid">
          <Field label="Couple's names">
            <input
              value={f.coupleNames}
              onChange={(e) => setF({ ...f, coupleNames: e.target.value })}
              placeholder="Amy & James"
            />
          </Field>
          <Field label="Wedding date">
            <input
              type="date"
              value={f.weddingDate}
              onChange={(e) => setF({ ...f, weddingDate: e.target.value })}
            />
          </Field>
          <Field label="Venue">
            <input
              value={f.venue}
              onChange={(e) => setF({ ...f, venue: e.target.value })}
              placeholder="Browsholme Hall"
            />
          </Field>
          <Field label="Region">
            <select
              value={f.region}
              onChange={(e) => setF({ ...f, region: e.target.value })}
            >
              <option>Lancashire</option>
              <option>Cheshire</option>
              <option>Lake District</option>
              <option>North West</option>
              <option>Elsewhere</option>
            </select>
          </Field>
          <Field label="Existing collection">
            <select
              value={f.collectionId}
              disabled={!!f.folderPath}
              onChange={(e) => setF({ ...f, collectionId: e.target.value })}
            >
              <option value="">None</option>
              {data.collections.map((c: any) => (
                <option value={c.id}>{c.name}</option>
              ))}
            </select>
          </Field>
          <Field label="Marketing permission">
            <select
              value={f.consentLevel}
              onChange={(e) => setF({ ...f, consentLevel: e.target.value })}
            >
              <option value="full">Full permission</option>
              <option value="selected_only">Selected images only</option>
              <option value="no_children">No children</option>
              <option value="anonymous">Anonymous use</option>
              <option value="portfolio_only">Portfolio only</option>
              <option value="none">No marketing use</option>
            </select>
          </Field>
          <Field label="Embargo until">
            <input
              type="date"
              value={f.embargoUntil}
              onChange={(e) => setF({ ...f, embargoUntil: e.target.value })}
            />
          </Field>
          <Field label="Campaign goal">
            <select
              value={f.campaignGoal}
              onChange={(e) => setF({ ...f, campaignGoal: e.target.value })}
            >
              <option value="enquiries">Generate enquiries</option>
              <option value="venue_authority">Build venue authority</option>
              <option value="supplier_relationships">
                Supplier relationships
              </option>
              <option value="availability">Promote availability</option>
            </select>
          </Field>
        </div>
        <div className="supplier-editor">
          <div className="supplier-head">
            <span><b>Wedding suppliers</b><small>Add the businesses you may want to credit or invite as collaborators.</small></span>
            <button type="button" onClick={addSupplierRow}><Plus size={14}/> Add supplier</button>
          </div>
          {suppliers.map((supplier: any, index: number) => {
            const search = encodeURIComponent(`${supplier.name} ${f.venue} ${f.region} Instagram`);
            return <div className="supplier-row" key={index}>
              <select value={supplier.role} onChange={(e) => changeSupplier(index, { role: e.target.value })}>
                {['Venue','Wedding planner','Videographer','Florist','Stylist','Dress','Suit','Hair','Make-up','Cake','Entertainment','Band or DJ','Catering','Transport','Celebrant','Other'].map(role => <option key={role}>{role}</option>)}
              </select>
              <input value={supplier.name} onChange={(e) => changeSupplier(index, { name: e.target.value, instagramConfirmed: false })} placeholder="Supplier business name"/>
              <input value={supplier.instagramHandle} onChange={(e) => changeSupplier(index, { instagramHandle: e.target.value.replace(/^@/, ''), instagramConfirmed: false })} placeholder="Instagram handle"/>
              <a className="button" href={`https://www.google.com/search?q=${search}`} target="_blank" rel="noreferrer">Find handle</a>
              <label className="supplier-confirm"><input type="checkbox" checked={supplier.instagramConfirmed} disabled={!supplier.instagramHandle.trim()} onChange={(e) => changeSupplier(index, { instagramConfirmed: e.target.checked })}/> I checked this account</label>
              <button type="button" aria-label="Remove supplier" onClick={() => setSuppliers(suppliers.filter((_: any, position: number) => position !== index))}><X size={14}/></button>
            </div>;
          })}
          {!suppliers.length && <p className="muted-copy">No suppliers added yet. SocialFlow will never guess or publish an unapproved tag.</p>}
        </div>
        <div className="modal-actions">
          {wedding && (
            <button className="danger-button" onClick={async () => {
              if (!confirm(`Remove ${wedding.couple_names} from SocialFlow? The original photographs will not be deleted.`)) return;
              await run("Removing wedding…", () => command("delete_wedding", { weddingId: wedding.id }), "Wedding removed");
              close();
            }}>Remove wedding</button>
          )}
          <button onClick={close}>Cancel</button>
          <button className="primary" onClick={save}>
            {wedding ? "Save changes" : "Import & create wedding"}
          </button>
        </div>
      </div>
    </Modal>
  );
}
function CampaignWizard({ wedding, data, close, run, openQueue }: any) {
  const [posts, setPosts] = useState(30),
    [daily, setDaily] = useState(5),
    [formats, setFormats] = useState(["carousel", "reel", "single", "story_pack"]);
  const collection = data.collections.find(
    (c: any) => c.id === wedding.collection_id,
  );
  const ids = data.images
    .filter((i: ImageRecord) => i.collection_id === wedding.collection_id)
    .map((i: ImageRecord) => i.id);
  const build = async () => {
    if (!wedding.collection_id)
      throw Error("Attach an imported collection first");
    await run("Organising the wedding…", () =>
      command("organise_collection", { collectionId: wedding.collection_id }),
    );
    if (data.settings.allow_cloud_ai === "true") {
      await run("Reading the photographs and writing their stories…", () =>
        command("analyse_images_ai", {
          imageIds: ids.slice(0, Math.min(posts * 12, ids.length)),
          weddingId: wedding.id,
        }),
      );
    }
    await run(
      "Creating carousels, Reels and Story packs…",
      () =>
        command("create_content_campaign", {
          profileId: 1,
          imageIds: ids,
          count: posts,
          postsPerDay: daily,
          weddingId: wedding.id,
          formats,
        }),
      "Content Studio campaign ready to review",
    );
    close();
    openQueue();
  };
  return (
    <Modal close={close}>
      <div className="wizard">
        <span className="eyebrow">CONTENT STUDIO</span>
        <h2>{wedding.couple_names}</h2>
        <p>
          {collection?.name || "No collection attached"} · {wedding.venue} ·{" "}
          {ids.length} photographs
        </p>
        <div className="preset">
          <b>Five-piece daily marketing engine</b>
          <span>
            One wedding owns each day. Posts are spaced at 08:00, 11:00,
            14:00, 17:00 and 20:00, then SocialFlow rotates to the next wedding.
          </span>
        </div>
        <Field label="Content formats">
          <div className="format-picker">
            {[
              ["carousel", "Story carousel", "5–7 photographs", Images],
              ["reel", "Photo Reel", "Vertical motion video", Film],
              ["single", "Single image", "Focused moment", BookImage],
              ["story_pack", "Story pack", "Five ready-to-upload slides", Instagram],
            ].map(([value, title, note, Icon]: any) => {
              const active=formats.includes(value);
              return <button key={value} type="button" className={active?"format-card active":"format-card"} onClick={()=>setFormats(active?formats.filter(x=>x!==value):[...formats,value])}>
                <Icon size={18}/><span><b>{title}</b><small>{note}</small></span><i>{active?"Included":"Off"}</i>
              </button>
            })}
          </div>
        </Field>
        <div className="form-grid">
          <Field label="Number of posts">
            <input
              type="number"
              min="1"
              max="500"
              value={posts}
              onChange={(e) => setPosts(Number(e.target.value))}
            />
          </Field>
          <Field label="Posts per day">
            <select
              value={daily}
              onChange={(e) => setDaily(Number(e.target.value))}
            >
              <option value="1">1 per day</option>
              <option value="2">2 per day</option>
              <option value="3">3 per day</option>
              <option value="4">4 per day</option>
              <option value="5">5 per day</option>
            </select>
          </Field>
        </div>
        <p className="hint">
          At this pace, {Math.ceil(posts / daily)} days of mixed content will be
          prepared. Reels are rendered locally and every item still requires approval.
        </p>
        <div className="consent-note">
          <Check /> Permission: {wedding.consent_level.replace("_", " ")}.
          Embargo: {wedding.embargo_until || "none"}.
        </div>
        <div className="modal-actions">
          <button onClick={close}>Cancel</button>
          <button className="primary" onClick={build} disabled={!formats.length}>
            Build Content Studio campaign
          </button>
        </div>
      </div>
    </Modal>
  );
}
function PostEditor({ post, close, run }: any) {
  const [caption, setCaption] = useState(post.caption),
    [tags, setTags] = useState(post.hashtags.join(" ")),
    [date, setDate] = useState(post.scheduled_at?.slice(0, 16) || ""),
    [ordered, setOrdered] = useState<ImageRecord[]>(post.images);
  const move = (index: number, direction: number) => {
    const next = [...ordered], target = index + direction;
    if (target < 0 || target >= next.length) return;
    [next[index], next[target]] = [next[target], next[index]];
    setOrdered(next);
  };
  const save = () =>
    run(
      "Saving post…",
      async () => {
        if (post.post_type === "carousel")
          await command("reorder_post_images", {
            postId: post.id,
            imageIds: ordered.map((image) => image.id),
          });
        await command("update_post", {
          postId: post.id,
          caption,
          hashtags: parseHashtags(tags),
          scheduledAt: date ? new Date(date).toISOString() : null,
        });
      },
      "Post saved",
    ).then(close);
  const publishFacebook = () =>
    run(
      "Publishing to your Facebook Page…",
      async () => {
        await command("update_post", {
          postId: post.id,
          caption,
          hashtags: parseHashtags(tags),
          scheduledAt: date ? new Date(date).toISOString() : null,
        });
        await command("publish_post_to_facebook", { postId: post.id });
      },
      "Published to Facebook",
    ).then(close);
  return (
    <Modal close={close}>
      <div className="post-editor">
        <div className="editor-photo">
          {post.post_type === "reel" && post.asset_path ? (
            <video src={photoSrc(post.asset_path)} controls playsInline />
          ) : ordered[0]?.thumbnail_path ? (
            <img src={photoSrc(ordered[0].thumbnail_path)} />
          ) : null}
        </div>
        <div>
          <span className="eyebrow">{platformName(post.platform).toUpperCase()} · {post.post_type.replace("_", " ").toUpperCase()} REVIEW</span>
          <h2>Preview, arrange and make it sound like you.</h2>
          {ordered.length > 1 && (
            <div className="sequence-strip">
              {ordered.map((image, index) => (
                <figure key={image.id} className={index === 0 ? "lead" : ""}>
                  <img src={photoSrc(image.thumbnail_path)} />
                  <figcaption>{index + 1}</figcaption>
                  {post.post_type === "carousel" && <div><button onClick={() => move(index, -1)}>←</button><button onClick={() => move(index, 1)}>→</button></div>}
                </figure>
              ))}
            </div>
          )}
          {post.asset_path && (
            <button onClick={() => command("reveal_post_asset", { postId: post.id })}>
              <Download size={14} /> {post.post_type === "story_pack" ? "Open exported Story pack" : "Show generated Reel"}
            </button>
          )}
          <Field label="Caption">
            <textarea
              value={caption}
              onChange={(e) => setCaption(e.target.value)}
            />
          </Field>
          <div className="rewrite">
            <button onClick={() => setCaption(caption.split("\n")[0])}>
              Shorter
            </button>
            <button
              onClick={() =>
                setCaption(caption.replace(/beautiful|magical|perfect/gi, ""))
              }
            >
              Less salesy
            </button>
          </div>
          <Field label="Five hashtags">
            <input value={tags} onChange={(e) => setTags(e.target.value)} />
          </Field>
          <Field label="Publishing time">
            <input
              type="datetime-local"
              value={date}
              onChange={(e) => setDate(e.target.value)}
            />
          </Field>
          <div className="modal-actions">
            <button onClick={close}>Cancel</button>
            {post.platform === "facebook" && post.status !== "published" && (
              <button onClick={publishFacebook}><Facebook size={15} /> Publish now</button>
            )}
            <button className="primary" onClick={save}>
              Save changes
            </button>
          </div>
        </div>
      </div>
    </Modal>
  );
}
function ImageDetail({ image, close }: any) {
  return (
    <Modal close={close}>
      <div className="preview">
        <div className="preview-photo">
          {image.thumbnail_path && <img src={photoSrc(image.thumbnail_path)} />}
        </div>
        <aside>
          <button className="close" onClick={close}>
            <X />
          </button>
          <span className="eyebrow">IMAGE DETAIL</span>
          <h2>{image.filename}</h2>
          <p className="path">{image.source_path}</p>
          <dl>
            <dt>Dimensions</dt>
            <dd>
              {image.width} × {image.height}
            </dd>
            <dt>Organisation</dt>
            <dd>{image.analysis_status}</dd>
            <dt>Wedding-day section</dt>
            <dd>{momentLabel(image.category) || "—"}</dd>
            <dt>Marketing score</dt>
            <dd>{image.social_score || "—"}</dd>
            <dt>Used</dt>
            <dd>{image.used_count} times</dd>
          </dl>
          <p>{image.description}</p>
        </aside>
      </div>
    </Modal>
  );
}
function SettingsView({ data, refresh, error }: any) {
  const [section, setSection] = useState("Brand voice"),
    [form, setForm] = useState(data.settings);
  useEffect(() => setForm(data.settings), [data]);
  const save = async () => {
    try {
      for (const [k, v] of Object.entries(form))
        await command("set_setting", { key: k, value: String(v) });
      await refresh();
    } catch (e) {
      error(String(e));
    }
  };
  return (
    <section className="settings">
      <div className="settings-nav">
        {[
          "Brand voice",
          "AI providers",
          "Instagram",
          "Facebook",
          "TikTok",
          "Posting",
          "Hashtags",
          "Storage",
          "Privacy",
          "About",
        ].map((x) => (
          <button
            className={section === x ? "active" : ""}
            onClick={() => setSection(x)}
          >
            {x}
          </button>
        ))}
      </div>
      <div className="settings-panel">
        <h2>{section}</h2>
        {section === "Brand voice" ? (
          <Setting title="Wayne's voice">
            <p>
              Natural British documentary storytelling: dry humour, specific
              human observations, no sentimentality and no generic wedding
              clichés.
            </p>
            <label>
              Banned phrases
              <textarea
                value={
                  form.banned_phrases ||
                  "capturing memories, magical moment, love was in the air, picture perfect, a day to remember, beautiful couple"
                }
                onChange={(e) =>
                  setForm({ ...form, banned_phrases: e.target.value })
                }
              />
            </label>
            <label>
              Default CTA
              <input
                value={
                  form.default_cta ||
                  "If your date is free, it’s yours — check your date."
                }
                onChange={(e) =>
                  setForm({ ...form, default_cta: e.target.value })
                }
              />
            </label>
          </Setting>
        ) : section === "AI providers" ? (
          <Setting title="Automatic visual analysis">
            <div className="connection">
              <Sparkles />
              <div>
                <b>
                  {data.settings.claude_installed === "true"
                    ? "Claude Code installed"
                    : "Claude Code not detected"}
                </b>
                <small>Official subscription route only</small>
              </div>
            </div>
            <div className="connection">
              <Sparkles />
              <div>
                <b>
                  {data.settings.codex_installed === "true"
                    ? "ChatGPT fallback ready"
                    : "Codex not detected"}
                </b>
                <small>Codex uses your signed-in ChatGPT subscription</small>
              </div>
            </div>
            <label className="switch">
              <input
                type="checkbox"
                checked={form.allow_cloud_ai === "true"}
                onChange={(e) =>
                  setForm({ ...form, allow_cloud_ai: String(e.target.checked) })
                }
              />
              Analyse resized previews with Claude, then ChatGPT automatically
              if Claude is unavailable or reaches its limit
            </label>
            <p>
              Originals never leave your Mac; only cached previews are supplied.
              Results must contain a real wedding section, a unique caption of
              at least three lines and exactly five hashtags.
            </p>
          </Setting>
        ) : section === "Instagram" ? (
          <InstagramConnection data={data} refresh={refresh} error={error} />
        ) : section === "Facebook" ? (
          <FacebookConnection data={data} refresh={refresh} error={error} />
        ) : section === "TikTok" ? (
          <TikTokConnection data={data} refresh={refresh} error={error} />
        ) : section === "Posting" ? (
          <Setting title="Local scheduler">
            <label className="switch">
              <input
                type="checkbox"
                checked={form.scheduler_enabled !== "false"}
                onChange={(e) =>
                  setForm({
                    ...form,
                    scheduler_enabled: String(e.target.checked),
                  })
                }
              />
              Publish scheduled posts while SocialFlow is running
            </label>
            <p>Your Mac must be awake at publishing time.</p>
          </Setting>
        ) : section === "Storage" ? (
          <Setting title="Local-first storage">
            <p>{data.settings.data_path}</p>
            <p>Originals are never renamed, moved, overwritten or deleted.</p>
          </Setting>
        ) : section === "Privacy" ? (
          <Setting title="Consent before convenience">
            <p>
              Every wedding has explicit marketing permission and an optional
              embargo. “No marketing use” disables campaign generation.
            </p>
          </Setting>
        ) : (
          <Setting title={section}>
            <p>SocialFlow 0.2 · The Bearded Wedding Photographer workflow.</p>
          </Setting>
        )}
        <button className="primary save" onClick={save}>
          Save settings
        </button>
      </div>
    </section>
  );
}
function TikTokConnection({ data, refresh, error }: any) {
  const [clientKey,setClientKey]=useState(data.settings.tiktok_client_key||"");
  const [clientSecret,setClientSecret]=useState("");
  const [openId,setOpenId]=useState(data.settings.tiktok_open_id||"");
  const [token,setToken]=useState("");
  const [saving,setSaving]=useState(false);
  const connected=data.settings.tiktok_connected==="true";
  const copies=data.settings.tiktok_reel_copies==="true";
  const mode=data.settings.tiktok_publish_mode||"draft";
  const connect=async()=>{try{setSaving(true);await command("save_tiktok_connection",{clientKey,openId,accessToken:token});setToken("");await refresh();}catch(e){error(String(e));}finally{setSaving(false)}};
  const oauth=async()=>{try{setSaving(true);await command("connect_tiktok_oauth",{clientKey,clientSecret});setClientSecret("");await refresh();}catch(e){error(String(e));}finally{setSaving(false)}};
  const setMode=async(value:string)=>{try{await command("set_setting",{key:"tiktok_publish_mode",value});await refresh()}catch(e){error(String(e))}};
  const toggle=async(checked:boolean)=>{try{if(checked){await command("backfill_tiktok_reels")}else{await command("set_setting",{key:"tiktok_reel_copies",value:"false"})}await refresh()}catch(e){error(String(e))}};
  return <Setting title="TikTok video publishing">
    <div className="connection"><Music2/><div><b>{connected?`Connected to ${data.settings.tiktok_display_name||"TikTok"}`:"TikTok not connected"}</b><small>{connected?"Token stored in macOS Keychain":"Content Posting API · Reel/video only"}</small></div></div>
    <label>TikTok Client Key<input value={clientKey} onChange={e=>setClientKey(e.target.value)} placeholder="From TikTok Developer Portal"/></label>
    <label>TikTok Client Secret<input type="password" autoComplete="off" value={clientSecret} onChange={e=>setClientSecret(e.target.value)} placeholder="Stored only in macOS Keychain"/></label>
    <button className="primary" disabled={!clientKey||!clientSecret||saving} onClick={oauth}>{saving?"Waiting for TikTok…":connected?"Reconnect with TikTok":"Connect with TikTok"}</button>
    <label>Open ID <small>(optional; verified automatically)</small><input value={openId} onChange={e=>setOpenId(e.target.value)} /></label>
    <label>{connected?"Paste replacement access token":"OAuth access token"}<input type="password" autoComplete="off" value={token} onChange={e=>setToken(e.target.value)} placeholder="Stored only in Keychain"/></label>
    <details><summary>Advanced: connect with an existing access token</summary><button disabled={!clientKey||!token||saving} onClick={connect}>{connected?"Replace access token":"Connect token"}</button></details>
    <label className="switch"><input type="checkbox" checked={copies} disabled={!connected} onChange={e=>toggle(e.target.checked)}/>Prepare separate TikTok versions of Reel posts, including existing scheduled Reels</label>
    <label>Publishing mode<select value={mode} disabled={!connected} onChange={e=>setMode(e.target.value)}><option value="draft">Send to TikTok inbox for final review</option><option value="direct">Publish directly after TikTok approval</option></select></label>
    <p className="hint">TikTok copies have separate approval and results. Direct mode requires TikTok to approve the app for <code>video.publish</code>; draft mode uses <code>video.upload</code>.</p>
  </Setting>;
}
function InstagramConnection({ data, refresh, error }: any) {
  const [appId, setAppId] = useState(
    data.settings.instagram_app_id || "27278259361853022",
  );
  const [accountId, setAccountId] = useState("17841473820152269");
  const [username, setUsername] = useState("bearded_wedding_photographer");
  const [token, setToken] = useState("");
  const [saving, setSaving] = useState(false);
  const connected = data.settings.mock_publish === "false";
  const connect = async () => {
    try {
      setSaving(true);
      await command("save_instagram_connection", {
        appId,
        accountId,
        username,
        accessToken: token,
      });
      setToken("");
      await refresh();
    } catch (e) {
      error(String(e));
    } finally {
      setSaving(false);
    }
  };
  return (
    <Setting title="Official Meta publishing">
      <div className="connection">
        <Instagram />
        <div>
          <b>{connected ? "Instagram connected" : "Ready to connect"}</b>
          <small>
            {connected
              ? `@${username} · token stored in macOS Keychain`
              : "Instagram Login API · Creator account"}
          </small>
        </div>
      </div>
      <label>
        Instagram App ID
        <input value={appId} onChange={(e) => setAppId(e.target.value)} />
      </label>
      <label>
        Instagram account ID
        <input
          value={accountId}
          onChange={(e) => setAccountId(e.target.value)}
        />
      </label>
      <label>
        Instagram username
        <input
          value={username}
          onChange={(e) => setUsername(e.target.value)}
        />
      </label>
      <label>
        {connected ? "Paste replacement access token" : "Access token"}
        <input
          type="password"
          value={token}
          autoComplete="off"
          placeholder={connected ? "Paste the newly generated Meta token here" : "Paste token here"}
          onChange={(e) => setToken(e.target.value)}
        />
      </label>
      <p className="hint">
        {connected && !token
          ? "Paste the new token into the field above. The verification button will then become active."
          : "The token is verified directly with Instagram and stored only in macOS Keychain. It is never written to SQLite or application logs."}
      </p>
      <button className="primary" disabled={!token || saving} onClick={connect}>
        {saving ? "Testing connection…" : connected ? "Verify & replace token" : "Connect & test"}
      </button>
    </Setting>
  );
}
function FacebookConnection({ data, refresh, error }: any) {
  const [pageId, setPageId] = useState(data.settings.facebook_page_id || "");
  const [pageName, setPageName] = useState(data.settings.facebook_page_name || "The Bearded Wedding Photographer");
  const [appId, setAppId] = useState(data.settings.facebook_app_id || "");
  const [appSecret, setAppSecret] = useState("");
  const [token, setToken] = useState("");
  const [saving, setSaving] = useState(false);
  const connected = data.settings.facebook_connected === "true";
  const separate = data.settings.facebook_separate_posts === "true";
  const connect = async () => {
    try {
      setSaving(true);
      await command("save_facebook_connection", { pageId, pageName, appId, appSecret, userAccessToken: token });
      await command("set_setting", { key: "facebook_separate_posts", value: "true" });
      setToken("");
      await refresh();
    } catch (e) { error(String(e)); } finally { setSaving(false); }
  };
  const toggleSeparate = async (checked: boolean) => {
    try {
      await command("set_setting", { key: "facebook_separate_posts", value: String(checked) });
      await refresh();
    } catch (e) { error(String(e)); }
  };
  return (
    <Setting title="Separate Facebook Page publishing">
      <div className="connection">
        <Facebook />
        <div>
          <b>{connected ? `Connected to ${data.settings.facebook_page_name || pageName}` : "Facebook Page not connected"}</b>
          <small>{connected ? (data.settings.facebook_token_kind === "non_expiring_page" ? "Non-expiring Page token · stored in macOS Keychain" : "Long-lived Page token · stored in macOS Keychain") : "Uses Meta's long-lived Pages token flow"}</small>
        </div>
      </div>
      <label>Facebook Page ID<input value={pageId} onChange={(e) => setPageId(e.target.value)} placeholder="Numeric Page ID" /></label>
      <label>Facebook Page name<input value={pageName} onChange={(e) => setPageName(e.target.value)} /></label>
      <label>Meta App ID<input value={appId} onChange={(e) => setAppId(e.target.value)} placeholder="From Meta App settings" /></label>
      <label>Meta App Secret<input type="password" value={appSecret} autoComplete="off" onChange={(e) => setAppSecret(e.target.value)} placeholder="Protected in macOS Keychain" /></label>
      <label>Short-lived User access token<input type="password" value={token} autoComplete="off" onChange={(e) => setToken(e.target.value)} placeholder="Include pages_show_list, pages_read_engagement and pages_manage_posts" /></label>
      <button className="primary" disabled={!pageId || !appId || !appSecret || !token || saving} onClick={connect}>{saving ? "Creating durable Page connection…" : connected ? "Replace with durable token" : "Create durable connection"}</button>
      <label className="switch">
        <input type="checkbox" checked={separate} disabled={!connected} onChange={(e) => toggleSeparate(e.target.checked)} />
        Prepare a separate Facebook version of every new campaign post
      </label>
      <p className="hint">SocialFlow exchanges the temporary User token for a long-lived token, derives the correct Page token, verifies it with Meta, and stores only protected credentials. Facebook versions retain their own caption, schedule, approval and publishing result.</p>
    </Setting>
  );
}
function Setting({ title, children }: any) {
  return (
    <div className="setting">
      <h3>{title}</h3>
      {children}
    </div>
  );
}
function Field({ label, children }: any) {
  return (
    <label className="field">
      <span>{label}</span>
      {children}
    </label>
  );
}
function Modal({ children, close }: any) {
  return (
    <div className="modal" onClick={close}>
      <div onClick={(e) => e.stopPropagation()}>{children}</div>
    </div>
  );
}
