import { invoke } from "@tauri-apps/api/core";
import type { Analysis, AppData } from "./types";
export const isTauri = () => Boolean((window as any).__TAURI_INTERNALS__);
const demo: AppData = {
  images: [],
  posts: [],
  profiles: [
    {
      id: 1,
      name: "The Bearded Wedding Photographer",
      business_description:
        "Documentary wedding photography across the North West and Lake District.",
      website: "https://www.thebeardedweddingphotographer.co.uk",
      default_cta: "Check your date",
      caption_instructions:
        "Dry, observant, natural British voice. Never sentimental or generic.",
    },
  ],
  collections: [],
  weddings: [],
  suppliers: [],
  settings: {
    mock_publish: "true",
    timezone: Intl.DateTimeFormat().resolvedOptions().timeZone,
    thumbnail_size: "176",
  },
  analytics: {
    measured_posts: 0,
    last_synced_at: null,
    formats: [],
    best_times: [],
    recommendations: [],
    permission_needed: true,
  },
  marketing: {
    leads: 0,
    booked_value: 0,
    failed_posts: 0,
    visual_duplicates_indexed: 0,
    last_backup_at: null,
    token_expiry: null,
  },
};
export async function command<T>(
  name: string,
  args: Record<string, unknown> = {},
): Promise<T> {
  if (!isTauri()) {
    if (name === "load_data") return structuredClone(demo) as T;
    throw new Error(
      "This action needs the macOS desktop app. Run npm run tauri dev.",
    );
  }
  return invoke<T>(name, args);
}
export function parseHashtags(s: string) {
  return [
    ...new Set(
      (s.match(/#[\p{L}\p{N}_]+/gu) || []).map((x) => x.toLowerCase()),
    ),
  ].slice(0, 30);
}
export function validateAnalysis(v: unknown): Analysis {
  if (!v || typeof v !== "object") throw Error("AI returned no object");
  const x = v as any;
  if (
    typeof x.description !== "string" ||
    typeof x.category !== "string" ||
    !Number.isFinite(x.socialScore)
  )
    throw Error("AI response is missing required fields");
  return {
    description: x.description,
    category: x.category,
    subCategory: String(x.subCategory || ""),
    subjects: Array.isArray(x.subjects) ? x.subjects.map(String) : [],
    mood: Array.isArray(x.mood) ? x.mood.map(String) : [],
    socialScore: Math.max(0, Math.min(100, x.socialScore)),
    visualNotes: Array.isArray(x.visualNotes) ? x.visualNotes.map(String) : [],
    captionAngles: Array.isArray(x.captionAngles)
      ? x.captionAngles.map(String)
      : [],
  };
}
export function allocateSlots(
  start: Date,
  count: number,
  weekdays = [1, 3, 5],
  hour = 19,
) {
  const out: Date[] = [];
  const d = new Date(start);
  d.setHours(hour, 0, 0, 0);
  while (out.length < count) {
    if (weekdays.includes(d.getDay()) && d >= start) out.push(new Date(d));
    d.setDate(d.getDate() + 1);
  }
  return out;
}
export const formatDate = (s: string | null) => {
  if (!s) return "Unscheduled";
  const normalized = s.replace(
    /^(\d{4}-\d{2}-\d{2}) (\d{2}:\d{2}:\d{2}) ([+-]\d{2}:\d{2})$/,
    "$1T$2$3",
  );
  const value = new Date(normalized);
  if (!Number.isFinite(value.getTime())) return s;
  return new Intl.DateTimeFormat(undefined, {
    dateStyle: "medium",
    timeStyle: "short",
  }).format(value);
};
