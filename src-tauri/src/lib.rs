use chrono::Timelike;
use chrono::{Datelike, Duration, Local, NaiveDate, TimeZone};
use image::GenericImageView;
use rusqlite::{params, Connection, OptionalExtension, Transaction};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    fs,
    io::Read,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::Mutex,
    thread,
    time::{Duration as StdDuration, Instant},
};
use tauri::{Manager, State};
use walkdir::WalkDir;

struct AppState {
    db: Mutex<Connection>,
    cache: PathBuf,
}
#[derive(Serialize)]
struct ImportResult {
    imported: u32,
    duplicates: u32,
    errors: Vec<String>,
}
#[derive(Serialize)]
struct InstagramConnectionResult {
    connected: bool,
    username: String,
    account_id: String,
}
#[derive(Serialize)]
struct FacebookConnectionResult {
    connected: bool,
    page_name: String,
    page_id: String,
    expires_at: i64,
}
#[derive(Serialize)]
struct TikTokConnectionResult {
    connected: bool,
    display_name: String,
    open_id: String,
}
#[derive(Serialize)]
struct InsightsSyncResult {
    synced: usize,
    detailed: usize,
    permission_needed: bool,
}
#[derive(Serialize)]
struct AnalysisRun {
    analysed: usize,
    claude_batches: usize,
    openai_batches: usize,
}
#[derive(Serialize, Default)]
struct FormatPerformance {
    format: String,
    posts: usize,
    average_score: f64,
    average_reach: f64,
}
#[derive(Serialize, Default)]
struct TimePerformance {
    hour: u32,
    posts: usize,
    average_score: f64,
    recommended: bool,
}
#[derive(Serialize, Default)]
struct BrainRecommendation {
    title: String,
    recommendation: String,
    evidence: String,
    confidence: String,
}
#[derive(Serialize, Default)]
struct AnalyticsReport {
    measured_posts: usize,
    last_synced_at: Option<String>,
    formats: Vec<FormatPerformance>,
    best_times: Vec<TimePerformance>,
    recommendations: Vec<BrainRecommendation>,
    permission_needed: bool,
}
#[derive(Serialize, Default)]
struct MarketingHealth {
    leads: usize,
    booked_value: f64,
    failed_posts: usize,
    visual_duplicates_indexed: usize,
    last_backup_at: Option<String>,
    token_expiry: Option<String>,
}
#[derive(Deserialize, Serialize, Clone)]
struct VisualAnalysis {
    image_id: i64,
    description: String,
    section: String,
    mood: String,
    caption: String,
    hashtags: Vec<String>,
    social_score: i64,
}
#[derive(Deserialize)]
struct VisionEnvelope {
    results: Vec<VisualAnalysis>,
}
#[derive(Serialize, Clone)]
struct ImageRecord {
    id: i64,
    collection_id: Option<i64>,
    source_path: String,
    filename: String,
    width: Option<u32>,
    height: Option<u32>,
    orientation: Option<String>,
    file_size: u64,
    thumbnail_path: Option<String>,
    analysis_status: String,
    missing: bool,
    used_count: i64,
    favourite: bool,
    description: Option<String>,
    category: Option<String>,
    social_score: Option<i64>,
}
#[derive(Serialize)]
struct Profile {
    id: i64,
    name: String,
    business_description: String,
    website: String,
    default_cta: String,
    caption_instructions: String,
}
#[derive(Serialize)]
struct Collection {
    id: i64,
    name: String,
    folder_path: String,
    profile_id: i64,
}
#[derive(Serialize)]
struct Wedding {
    id: i64,
    collection_id: Option<i64>,
    couple_names: String,
    wedding_date: String,
    venue: String,
    region: String,
    consent_level: String,
    embargo_until: Option<String>,
    campaign_goal: String,
    status: String,
    created_at: String,
}
#[derive(Serialize)]
struct Supplier {
    id: i64,
    wedding_id: i64,
    role: String,
    name: String,
    instagram_handle: String,
    website: String,
    instagram_confirmed: bool,
}

#[derive(Deserialize)]
struct SupplierInput {
    role: String,
    name: String,
    instagram_handle: String,
    website: String,
    instagram_confirmed: bool,
}
#[derive(Serialize)]
struct Post {
    id: i64,
    profile_id: i64,
    caption: String,
    hashtags: Vec<String>,
    status: String,
    scheduled_at: Option<String>,
    published_at: Option<String>,
    post_type: String,
    platform: String,
    facebook_post_id: Option<String>,
    tiktok_publish_id: Option<String>,
    asset_path: Option<String>,
    created_at: String,
    images: Vec<ImageRecord>,
}
#[derive(Serialize)]
struct AppData {
    images: Vec<ImageRecord>,
    posts: Vec<Post>,
    profiles: Vec<Profile>,
    collections: Vec<Collection>,
    weddings: Vec<Wedding>,
    suppliers: Vec<Supplier>,
    settings: HashMap<String, String>,
    analytics: AnalyticsReport,
    marketing: MarketingHealth,
}

fn migrations(c: &Connection) -> rusqlite::Result<()> {
    c.execute_batch("PRAGMA foreign_keys=ON; CREATE TABLE IF NOT EXISTS schema_migrations(version INTEGER PRIMARY KEY,applied_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP); CREATE TABLE IF NOT EXISTS profiles(id INTEGER PRIMARY KEY,name TEXT NOT NULL,type TEXT DEFAULT 'photography',business_name TEXT DEFAULT '',business_description TEXT DEFAULT '',website TEXT DEFAULT '',default_cta TEXT DEFAULT '',caption_instructions TEXT DEFAULT '',created_at TEXT DEFAULT CURRENT_TIMESTAMP,updated_at TEXT DEFAULT CURRENT_TIMESTAMP); CREATE TABLE IF NOT EXISTS collections(id INTEGER PRIMARY KEY,name TEXT NOT NULL,folder_path TEXT NOT NULL,include_subfolders INTEGER DEFAULT 1,profile_id INTEGER NOT NULL REFERENCES profiles(id),created_at TEXT DEFAULT CURRENT_TIMESTAMP,updated_at TEXT DEFAULT CURRENT_TIMESTAMP,UNIQUE(folder_path,profile_id)); CREATE TABLE IF NOT EXISTS images(id INTEGER PRIMARY KEY,collection_id INTEGER REFERENCES collections(id) ON DELETE SET NULL,source_path TEXT NOT NULL UNIQUE,filename TEXT NOT NULL,extension TEXT,file_hash TEXT NOT NULL,width INTEGER,height INTEGER,orientation TEXT,file_size INTEGER NOT NULL,created_date TEXT,imported_at TEXT DEFAULT CURRENT_TIMESTAMP,thumbnail_path TEXT,analysis_status TEXT DEFAULT 'not_analysed',last_verified_at TEXT,missing INTEGER DEFAULT 0,used_count INTEGER DEFAULT 0,last_used_at TEXT,favourite INTEGER DEFAULT 0); CREATE UNIQUE INDEX IF NOT EXISTS idx_images_hash ON images(file_hash); CREATE INDEX IF NOT EXISTS idx_images_collection ON images(collection_id); CREATE TABLE IF NOT EXISTS image_analysis(id INTEGER PRIMARY KEY,image_id INTEGER NOT NULL UNIQUE REFERENCES images(id) ON DELETE CASCADE,provider TEXT,model TEXT,description TEXT,category TEXT,sub_category TEXT,subjects_json TEXT,mood TEXT,location_guess TEXT,visual_features_json TEXT,quality_score INTEGER,social_score INTEGER,analysis_json TEXT,analysed_at TEXT DEFAULT CURRENT_TIMESTAMP); CREATE TABLE IF NOT EXISTS posts(id INTEGER PRIMARY KEY,profile_id INTEGER NOT NULL REFERENCES profiles(id),caption TEXT DEFAULT '',hashtags_json TEXT DEFAULT '[]',status TEXT NOT NULL DEFAULT 'draft' CHECK(status IN ('draft','needs_review','approved','scheduled','publishing','published','failed')),scheduled_at TEXT,published_at TEXT,post_type TEXT DEFAULT 'single',instagram_media_id TEXT,created_at TEXT DEFAULT CURRENT_TIMESTAMP,updated_at TEXT DEFAULT CURRENT_TIMESTAMP,manually_edited_caption INTEGER DEFAULT 0,ai_generated INTEGER DEFAULT 0); CREATE INDEX IF NOT EXISTS idx_posts_schedule ON posts(status,scheduled_at); CREATE TABLE IF NOT EXISTS post_images(id INTEGER PRIMARY KEY,post_id INTEGER NOT NULL REFERENCES posts(id) ON DELETE CASCADE,image_id INTEGER NOT NULL REFERENCES images(id),position INTEGER NOT NULL,UNIQUE(post_id,image_id)); CREATE TABLE IF NOT EXISTS campaigns(id INTEGER PRIMARY KEY,profile_id INTEGER NOT NULL REFERENCES profiles(id),collection_id INTEGER REFERENCES collections(id),name TEXT NOT NULL,start_date TEXT,end_date TEXT,requested_post_count INTEGER,posts_per_week INTEGER,status TEXT DEFAULT 'proposed',created_at TEXT DEFAULT CURRENT_TIMESTAMP); CREATE TABLE IF NOT EXISTS posting_slots(id INTEGER PRIMARY KEY,profile_id INTEGER NOT NULL REFERENCES profiles(id),weekday INTEGER NOT NULL,time TEXT NOT NULL,enabled INTEGER DEFAULT 1); CREATE TABLE IF NOT EXISTS publish_attempts(id INTEGER PRIMARY KEY,post_id INTEGER NOT NULL REFERENCES posts(id),started_at TEXT,finished_at TEXT,status TEXT,provider_response TEXT,error_message TEXT); CREATE TABLE IF NOT EXISTS instagram_accounts(id INTEGER PRIMARY KEY,profile_id INTEGER NOT NULL REFERENCES profiles(id),username TEXT,instagram_user_id TEXT,token_reference TEXT,token_expiry TEXT,connected INTEGER DEFAULT 0,last_successful_request TEXT); CREATE TABLE IF NOT EXISTS app_settings(key TEXT PRIMARY KEY,value TEXT NOT NULL); INSERT OR IGNORE INTO schema_migrations(version) VALUES(1); INSERT OR IGNORE INTO profiles(id,name) VALUES(1,'My Photography'); INSERT OR IGNORE INTO app_settings(key,value) VALUES('mock_publish','true'),('scheduler_enabled','true'),('allow_cloud_ai','false'),('thumbnail_size','176');")
    ?;
    c.execute_batch("CREATE TABLE IF NOT EXISTS weddings(id INTEGER PRIMARY KEY,collection_id INTEGER REFERENCES collections(id) ON DELETE SET NULL,profile_id INTEGER NOT NULL REFERENCES profiles(id),couple_names TEXT NOT NULL,wedding_date TEXT DEFAULT '',venue TEXT DEFAULT '',region TEXT DEFAULT '',consent_level TEXT NOT NULL DEFAULT 'full' CHECK(consent_level IN ('full','selected_only','no_children','anonymous','portfolio_only','none')),embargo_until TEXT,campaign_goal TEXT DEFAULT 'enquiries',status TEXT DEFAULT 'imported',created_at TEXT DEFAULT CURRENT_TIMESTAMP,updated_at TEXT DEFAULT CURRENT_TIMESTAMP); CREATE TABLE IF NOT EXISTS suppliers(id INTEGER PRIMARY KEY,wedding_id INTEGER NOT NULL REFERENCES weddings(id) ON DELETE CASCADE,role TEXT NOT NULL,name TEXT NOT NULL,instagram_handle TEXT DEFAULT '',website TEXT DEFAULT ''); CREATE INDEX IF NOT EXISTS idx_weddings_collection ON weddings(collection_id); CREATE INDEX IF NOT EXISTS idx_suppliers_wedding ON suppliers(wedding_id); INSERT OR REPLACE INTO profiles(id,name,business_name,business_description,website,default_cta,caption_instructions) VALUES(1,'The Bearded Wedding Photographer','The Bearded Wedding Photographer','Documentary wedding photography across Darwen, Lancashire, Cheshire, the North West and Lake District.','https://www.thebeardedweddingphotographer.co.uk','Check your date','Natural British voice. Dry humour, emotionally specific observations, no sentimentality, no generic wedding clichés, and no unnecessary sales language.'); INSERT OR IGNORE INTO schema_migrations(version) VALUES(2);")
    ?;
    let has_v3 = c.query_row(
        "SELECT EXISTS(SELECT 1 FROM schema_migrations WHERE version=3)",
        [],
        |r| r.get::<_, bool>(0),
    )?;
    if !has_v3 {
        c.execute_batch("INSERT INTO app_settings(key,value)VALUES('allow_cloud_ai','true') ON CONFLICT(key) DO UPDATE SET value='true'; INSERT INTO schema_migrations(version)VALUES(3);")?;
    }
    let has_v4 = c.query_row(
        "SELECT EXISTS(SELECT 1 FROM schema_migrations WHERE version=4)",
        [],
        |r| r.get::<_, bool>(0),
    )?;
    if !has_v4 {
        c.execute_batch("ALTER TABLE posts ADD COLUMN asset_path TEXT; CREATE TABLE IF NOT EXISTS post_insights(post_id INTEGER PRIMARY KEY REFERENCES posts(id) ON DELETE CASCADE,reach INTEGER DEFAULT 0,impressions INTEGER DEFAULT 0,likes INTEGER DEFAULT 0,comments INTEGER DEFAULT 0,saves INTEGER DEFAULT 0,shares INTEGER DEFAULT 0,plays INTEGER DEFAULT 0,last_synced_at TEXT); INSERT OR IGNORE INTO app_settings(key,value)VALUES('content_mix','carousel,reel,single,story_pack,carousel'); INSERT INTO schema_migrations(version)VALUES(4);")?;
    }
    let has_v5 = c.query_row(
        "SELECT EXISTS(SELECT 1 FROM schema_migrations WHERE version=5)",
        [],
        |r| r.get::<_, bool>(0),
    )?;
    if !has_v5 {
        c.execute_batch("CREATE TABLE IF NOT EXISTS instagram_performance(instagram_media_id TEXT PRIMARY KEY,local_post_id INTEGER REFERENCES posts(id) ON DELETE SET NULL,caption TEXT DEFAULT '',post_type TEXT DEFAULT 'single',published_at TEXT,reach INTEGER DEFAULT 0,likes INTEGER DEFAULT 0,comments INTEGER DEFAULT 0,saves INTEGER DEFAULT 0,shares INTEGER DEFAULT 0,plays INTEGER DEFAULT 0,total_interactions INTEGER DEFAULT 0,section TEXT DEFAULT '',synced_at TEXT DEFAULT CURRENT_TIMESTAMP); CREATE INDEX IF NOT EXISTS idx_performance_type ON instagram_performance(post_type); INSERT OR IGNORE INTO app_settings(key,value)VALUES('insights_permission','unknown'); INSERT INTO schema_migrations(version)VALUES(5);")?;
    }
    let has_v6 = c.query_row(
        "SELECT EXISTS(SELECT 1 FROM schema_migrations WHERE version=6)",
        [],
        |r| r.get::<_, bool>(0),
    )?;
    if !has_v6 {
        c.execute_batch("ALTER TABLE images ADD COLUMN perceptual_hash TEXT DEFAULT ''; CREATE TABLE IF NOT EXISTS marketing_leads(id INTEGER PRIMARY KEY,source_post_id INTEGER REFERENCES posts(id) ON DELETE SET NULL,source TEXT DEFAULT 'website',status TEXT DEFAULT 'new',value REAL DEFAULT 0,created_at TEXT DEFAULT CURRENT_TIMESTAMP); CREATE TABLE IF NOT EXISTS app_backups(id INTEGER PRIMARY KEY,path TEXT NOT NULL,created_at TEXT DEFAULT CURRENT_TIMESTAMP); INSERT INTO schema_migrations(version)VALUES(6);")?;
    }
    let has_v7 = c.query_row(
        "SELECT EXISTS(SELECT 1 FROM schema_migrations WHERE version=7)",
        [],
        |r| r.get::<_, bool>(0),
    )?;
    if !has_v7 {
        c.execute_batch("ALTER TABLE suppliers ADD COLUMN instagram_confirmed INTEGER NOT NULL DEFAULT 0; INSERT INTO schema_migrations(version)VALUES(7);")?;
    }
    let has_v8 = c.query_row(
        "SELECT EXISTS(SELECT 1 FROM schema_migrations WHERE version=8)",
        [],
        |r| r.get::<_, bool>(0),
    )?;
    if !has_v8 {
        c.execute_batch("ALTER TABLE posts ADD COLUMN platform TEXT NOT NULL DEFAULT 'instagram'; ALTER TABLE posts ADD COLUMN facebook_post_id TEXT; CREATE TABLE IF NOT EXISTS facebook_accounts(id INTEGER PRIMARY KEY,profile_id INTEGER NOT NULL REFERENCES profiles(id),page_id TEXT NOT NULL,page_name TEXT DEFAULT '',token_reference TEXT,connected INTEGER DEFAULT 0,last_successful_request TEXT); CREATE UNIQUE INDEX IF NOT EXISTS idx_facebook_profile_unique ON facebook_accounts(profile_id); INSERT OR IGNORE INTO app_settings(key,value)VALUES('facebook_separate_posts','false'); INSERT INTO schema_migrations(version)VALUES(8);")?;
    }
    let has_v9 = c.query_row(
        "SELECT EXISTS(SELECT 1 FROM schema_migrations WHERE version=9)", [],
        |r| r.get::<_, bool>(0),
    )?;
    if !has_v9 {
        c.execute_batch("ALTER TABLE posts ADD COLUMN tiktok_publish_id TEXT; CREATE TABLE IF NOT EXISTS tiktok_accounts(id INTEGER PRIMARY KEY,profile_id INTEGER NOT NULL REFERENCES profiles(id),open_id TEXT NOT NULL,display_name TEXT DEFAULT '',token_reference TEXT,connected INTEGER DEFAULT 0,last_successful_request TEXT); CREATE UNIQUE INDEX IF NOT EXISTS idx_tiktok_profile_unique ON tiktok_accounts(profile_id); INSERT OR IGNORE INTO app_settings(key,value)VALUES('tiktok_connected','false'),('tiktok_reel_copies','false'),('tiktok_publish_mode','draft'); INSERT INTO schema_migrations(version)VALUES(9);")?;
    }
    let has_v10 = c.query_row(
        "SELECT EXISTS(SELECT 1 FROM schema_migrations WHERE version=10)",
        [],
        |r| r.get::<_, bool>(0),
    )?;
    if !has_v10 {
        // The Python publisher creates this table on first run, so a Mac that has
        // never published had no `publish_recovery` — and `save_facebook_connection`
        // reads it, failing the connection after the token was already stored.
        // Schema kept byte-identical to `ensure_recovery_schema` in the publisher.
        c.execute_batch("CREATE TABLE IF NOT EXISTS publish_recovery(post_id INTEGER PRIMARY KEY REFERENCES posts(id) ON DELETE CASCADE,failure_class TEXT NOT NULL,retry_count INTEGER NOT NULL DEFAULT 0,next_retry_at TEXT,requires_action INTEGER NOT NULL DEFAULT 0,resolution_hint TEXT DEFAULT '',last_error TEXT DEFAULT '',updated_at TEXT DEFAULT CURRENT_TIMESTAMP); INSERT INTO schema_migrations(version)VALUES(10);")?;
    }
    c.execute(
        "INSERT OR IGNORE INTO app_settings(key,value)VALUES('posting_time_mode','suggest')",
        [],
    )?;
    c.execute(
        "INSERT OR IGNORE INTO app_settings(key,value)VALUES('content_selection_mode','auto')",
        [],
    )?;
    Ok(())
}
fn img_from_row(r: &rusqlite::Row) -> rusqlite::Result<ImageRecord> {
    Ok(ImageRecord {
        id: r.get(0)?,
        collection_id: r.get(1)?,
        source_path: r.get(2)?,
        filename: r.get(3)?,
        width: r.get(4)?,
        height: r.get(5)?,
        orientation: r.get(6)?,
        file_size: r.get::<_, i64>(7)? as u64,
        thumbnail_path: r.get(8)?,
        analysis_status: r.get(9)?,
        missing: r.get::<_, i64>(10)? != 0,
        used_count: r.get(11)?,
        favourite: r.get::<_, i64>(12)? != 0,
        description: r.get(13)?,
        category: r.get(14)?,
        social_score: r.get(15)?,
    })
}
fn get_image(c: &Connection, id: i64) -> rusqlite::Result<ImageRecord> {
    c.query_row("SELECT i.id,i.collection_id,i.source_path,i.filename,i.width,i.height,i.orientation,i.file_size,i.thumbnail_path,CASE WHEN a.provider IN ('claude','openai') THEN 'completed' WHEN a.provider='local' THEN 'locally_organised' ELSE i.analysis_status END,i.missing,i.used_count,i.favourite,a.description,COALESCE(NULLIF(a.sub_category,''),a.category),a.social_score FROM images i LEFT JOIN image_analysis a ON a.image_id=i.id WHERE i.id=?",[id],img_from_row)
}
#[tauri::command]
fn load_data(state: State<AppState>) -> Result<AppData, String> {
    let c = state.db.lock().unwrap();
    let images=c.prepare("SELECT i.id,i.collection_id,i.source_path,i.filename,i.width,i.height,i.orientation,i.file_size,i.thumbnail_path,CASE WHEN a.provider IN ('claude','openai') THEN 'completed' WHEN a.provider='local' THEN 'locally_organised' ELSE i.analysis_status END,i.missing,i.used_count,i.favourite,a.description,COALESCE(NULLIF(a.sub_category,''),a.category),a.social_score FROM images i LEFT JOIN image_analysis a ON a.image_id=i.id ORDER BY i.imported_at DESC").map_err(|e|e.to_string())?.query_map([],img_from_row).map_err(|e|e.to_string())?.collect::<Result<Vec<_>,_>>().map_err(|e|e.to_string())?;
    let profiles=c.prepare("SELECT id,name,business_description,website,default_cta,caption_instructions FROM profiles ORDER BY id").unwrap().query_map([],|r|Ok(Profile{id:r.get(0)?,name:r.get(1)?,business_description:r.get(2)?,website:r.get(3)?,default_cta:r.get(4)?,caption_instructions:r.get(5)?})).unwrap().collect::<Result<Vec<_>,_>>().unwrap();
    let collections = c
        .prepare("SELECT id,name,folder_path,profile_id FROM collections ORDER BY name")
        .unwrap()
        .query_map([], |r| {
            Ok(Collection {
                id: r.get(0)?,
                name: r.get(1)?,
                folder_path: r.get(2)?,
                profile_id: r.get(3)?,
            })
        })
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    let weddings = c.prepare("SELECT id,collection_id,couple_names,wedding_date,venue,region,consent_level,embargo_until,campaign_goal,status,created_at FROM weddings ORDER BY wedding_date DESC,created_at DESC").unwrap().query_map([],|r|Ok(Wedding{id:r.get(0)?,collection_id:r.get(1)?,couple_names:r.get(2)?,wedding_date:r.get(3)?,venue:r.get(4)?,region:r.get(5)?,consent_level:r.get(6)?,embargo_until:r.get(7)?,campaign_goal:r.get(8)?,status:r.get(9)?,created_at:r.get(10)?})).unwrap().collect::<Result<Vec<_>,_>>().unwrap();
    let suppliers = c.prepare("SELECT id,wedding_id,role,name,instagram_handle,website,instagram_confirmed FROM suppliers ORDER BY role,name").unwrap().query_map([],|r|Ok(Supplier{id:r.get(0)?,wedding_id:r.get(1)?,role:r.get(2)?,name:r.get(3)?,instagram_handle:r.get(4)?,website:r.get(5)?,instagram_confirmed:r.get::<_,i64>(6)? != 0})).unwrap().collect::<Result<Vec<_>,_>>().unwrap();
    let settings = c
        .prepare("SELECT key,value FROM app_settings")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .collect::<Result<HashMap<_, _>, _>>()
        .unwrap();
    let mut posts = Vec::new();
    {
        let mut st=c.prepare("SELECT id,profile_id,caption,hashtags_json,status,scheduled_at,published_at,post_type,created_at,asset_path,platform,facebook_post_id,tiktok_publish_id FROM posts ORDER BY COALESCE(scheduled_at,created_at)").unwrap();
        let rows = st
            .query_map([], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, i64>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, String>(4)?,
                    r.get::<_, Option<String>>(5)?,
                    r.get::<_, Option<String>>(6)?,
                    r.get::<_, String>(7)?,
                    r.get::<_, String>(8)?,
                    r.get::<_, Option<String>>(9)?,
                    r.get::<_, String>(10)?,
                    r.get::<_, Option<String>>(11)?,
                    r.get::<_, Option<String>>(12)?,
                ))
            })
            .unwrap();
        for row in rows {
            let (
                id,
                profile_id,
                caption,
                h,
                status,
                scheduled_at,
                published_at,
                post_type,
                created_at,
                asset_path,
                platform,
                facebook_post_id,
                tiktok_publish_id,
            ) = row.unwrap();
            let ids = c
                .prepare("SELECT image_id FROM post_images WHERE post_id=? ORDER BY position")
                .unwrap()
                .query_map([id], |r| r.get(0))
                .unwrap()
                .collect::<Result<Vec<i64>, _>>()
                .unwrap();
            let ims = ids
                .into_iter()
                .filter_map(|x| get_image(&c, x).ok())
                .collect();
            posts.push(Post {
                id,
                profile_id,
                caption,
                hashtags: serde_json::from_str(&h).unwrap_or_default(),
                status,
                scheduled_at,
                published_at,
                post_type,
                platform,
                facebook_post_id,
                tiktok_publish_id,
                asset_path,
                created_at,
                images: ims,
            });
        }
    }
    let analytics = build_analytics_report(&c, &settings);
    let marketing = MarketingHealth {
        leads: c.query_row("SELECT COUNT(*) FROM marketing_leads", [], |r| r.get::<_,i64>(0)).unwrap_or(0) as usize,
        booked_value: c.query_row("SELECT COALESCE(SUM(value),0) FROM marketing_leads", [], |r| r.get(0)).unwrap_or(0.0),
        failed_posts: c.query_row("SELECT COUNT(*) FROM posts WHERE status='failed'", [], |r| r.get::<_,i64>(0)).unwrap_or(0) as usize,
        visual_duplicates_indexed: c.query_row("SELECT COUNT(*) FROM images WHERE COALESCE(perceptual_hash,'')<>''", [], |r| r.get::<_,i64>(0)).unwrap_or(0) as usize,
        last_backup_at: settings.get("last_backup_at").cloned(),
        token_expiry: c.query_row("SELECT token_expiry FROM instagram_accounts WHERE connected=1 ORDER BY id DESC LIMIT 1", [], |r| r.get(0)).ok().flatten(),
    };
    Ok(AppData {
        images,
        posts,
        profiles,
        collections,
        weddings,
        suppliers,
        settings,
        analytics,
        marketing,
    })
}

fn engagement_score(
    reach: i64,
    likes: i64,
    comments: i64,
    saves: i64,
    shares: i64,
    plays: i64,
) -> f64 {
    let weighted = likes as f64
        + comments as f64 * 3.0
        + saves as f64 * 4.0
        + shares as f64 * 5.0
        + plays as f64 * 0.08;
    if reach > 0 {
        weighted / reach as f64 * 100.0
    } else {
        weighted
    }
}

fn build_analytics_report(c: &Connection, settings: &HashMap<String, String>) -> AnalyticsReport {
    let mut rows=match c.prepare("SELECT post_type,reach,likes,comments,saves,shares,plays,published_at,section FROM instagram_performance") {Ok(value)=>value,Err(_)=>return AnalyticsReport::default()};
    let data = rows
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, i64>(2)?,
                r.get::<_, i64>(3)?,
                r.get::<_, i64>(4)?,
                r.get::<_, i64>(5)?,
                r.get::<_, i64>(6)?,
                r.get::<_, Option<String>>(7)?,
                r.get::<_, String>(8)?,
            ))
        })
        .ok()
        .map(|items| items.filter_map(Result::ok).collect::<Vec<_>>())
        .unwrap_or_default();
    let mut groups: HashMap<String, (usize, f64, i64)> = HashMap::new();
    let mut hours: HashMap<u32, (usize, f64)> = HashMap::new();
    let mut sections: HashMap<String, (usize, f64)> = HashMap::new();
    for (format, reach, likes, comments, saves, shares, plays, published, section) in &data {
        let score = engagement_score(*reach, *likes, *comments, *saves, *shares, *plays);
        let entry = groups.entry(format.clone()).or_default();
        entry.0 += 1;
        entry.1 += score;
        entry.2 += reach;
        if let Some(hour) = published
            .as_ref()
            .and_then(|stamp| chrono::DateTime::parse_from_rfc3339(stamp).ok())
            .map(|date| date.hour())
        {
            let entry = hours.entry(hour).or_default();
            entry.0 += 1;
            entry.1 += score;
        }
        if !section.is_empty() {
            let entry = sections.entry(section.clone()).or_default();
            entry.0 += 1;
            entry.1 += score;
        }
    }
    let mut formats = groups
        .into_iter()
        .map(|(format, (posts, score, reach))| FormatPerformance {
            format,
            posts,
            average_score: score / posts as f64,
            average_reach: reach as f64 / posts as f64,
        })
        .collect::<Vec<_>>();
    formats.sort_by(|a, b| {
        b.average_score
            .partial_cmp(&a.average_score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let measured = data.len();
    let total_hour_score: f64 = hours.values().map(|value| value.1).sum();
    let total_hour_posts: usize = hours.values().map(|value| value.0).sum();
    let baseline = if total_hour_posts > 0 {
        total_hour_score / total_hour_posts as f64
    } else {
        0.0
    };
    let mut best_times = hours
        .iter()
        .map(|(hour, (posts, score))| TimePerformance {
            hour: *hour,
            posts: *posts,
            average_score: (score + baseline * 3.0) / (*posts as f64 + 3.0),
            recommended: false,
        })
        .collect::<Vec<_>>();
    best_times.sort_by(|a, b| {
        b.average_score
            .partial_cmp(&a.average_score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    best_times.truncate(5);
    best_times
        .iter_mut()
        .for_each(|time| time.recommended = true);
    let evidence_confidence = |count: usize| {
        if count >= 15 {
            "High"
        } else if count >= 5 {
            "Building"
        } else {
            "Early"
        }
    };
    let mut recommendations = Vec::new();
    if let Some(best) = formats.iter().find(|item| item.posts >= 2) {
        recommendations.push(BrainRecommendation {
            title: "Lead with the strongest format".into(),
            recommendation: format!(
                "Use more {} content in the next campaign.",
                best.format.replace('_', " ")
            ),
            evidence: format!(
                "{} measured posts · {:.1} engagement score · {:.0} average reach",
                best.posts, best.average_score, best.average_reach
            ),
            confidence: evidence_confidence(best.posts).into(),
        });
    }
    if let Some((hour, (count, score))) =
        hours
            .iter()
            .filter(|(_, value)| value.0 >= 2)
            .max_by(|a, b| {
                (a.1 .1 / a.1 .0 as f64)
                    .partial_cmp(&(b.1 .1 / b.1 .0 as f64))
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
    {
        recommendations.push(BrainRecommendation {
            title: "Best publishing window".into(),
            recommendation: format!("Test more posts around {:02}:00.", hour),
            evidence: format!(
                "{} posts averaged {:.1} engagement",
                count,
                score / (*count as f64)
            ),
            confidence: evidence_confidence(*count).into(),
        });
    }
    if let Some((section, (count, score))) = sections
        .iter()
        .filter(|(_, value)| value.0 >= 2)
        .max_by(|a, b| {
            (a.1 .1 / a.1 .0 as f64)
                .partial_cmp(&(b.1 .1 / b.1 .0 as f64))
                .unwrap_or(std::cmp::Ordering::Equal)
        })
    {
        recommendations.push(BrainRecommendation {
            title: "Moment people respond to".into(),
            recommendation: format!("Feature more {} moments.", section),
            evidence: format!(
                "{} posts averaged {:.1} engagement",
                count,
                score / (*count as f64)
            ),
            confidence: evidence_confidence(*count).into(),
        });
    }
    let recent_sections = c.prepare("SELECT section FROM instagram_performance WHERE section<>'' ORDER BY COALESCE(published_at,synced_at) DESC LIMIT 12")
        .ok().and_then(|mut statement| statement.query_map([], |r| r.get::<_,String>(0)).ok().map(|rows| rows.filter_map(Result::ok).collect::<Vec<_>>())).unwrap_or_default();
    let mut recent_counts = HashMap::<String, usize>::new();
    for section in &recent_sections {
        *recent_counts.entry(section.clone()).or_default() += 1;
    }
    if let Some((section, count)) = recent_counts.into_iter().max_by_key(|(_, count)| *count) {
        if recent_sections.len() >= 6 && count * 2 > recent_sections.len() {
            recommendations.push(BrainRecommendation {
                title: "Content fatigue guard".into(),
                recommendation: format!(
                    "Pause {} for a few posts and widen the story mix.",
                    section
                ),
                evidence: format!(
                    "{} of the last {} posts used this moment type",
                    count,
                    recent_sections.len()
                ),
                confidence: evidence_confidence(recent_sections.len()).into(),
            });
        }
    }
    if recommendations.is_empty() {
        recommendations.push(BrainRecommendation {
            title: "Learning has started".into(),
            recommendation: "Publish and sync at least ten posts before changing the content mix."
                .into(),
            evidence: format!(
                "{} measured posts; recommendations become useful at 10 and strong at 30.",
                measured
            ),
            confidence: "Not enough data".into(),
        });
    }
    let last_synced_at = c
        .query_row(
            "SELECT MAX(synced_at) FROM instagram_performance",
            [],
            |r| r.get::<_, Option<String>>(0),
        )
        .unwrap_or(None);
    AnalyticsReport {
        measured_posts: measured,
        last_synced_at,
        formats,
        best_times,
        recommendations,
        permission_needed: settings.get("insights_permission").map(String::as_str)
            != Some("granted"),
    }
}

fn default_posting_hours(count: usize) -> Vec<u32> {
    match count.clamp(1, 5) {
        1 => vec![19],
        2 => vec![12, 19],
        3 => vec![9, 14, 19],
        4 => vec![9, 13, 17, 21],
        _ => vec![8, 11, 14, 17, 20],
    }
}

fn learned_posting_hours(c: &Connection, count: usize) -> Vec<u32> {
    let automatic = c
        .query_row(
            "SELECT value='auto' FROM app_settings WHERE key='posting_time_mode'",
            [],
            |r| r.get::<_, bool>(0),
        )
        .unwrap_or(false);
    if !automatic {
        return default_posting_hours(count);
    }
    let settings = HashMap::new();
    let report = build_analytics_report(c, &settings);
    let mut hours = report
        .best_times
        .into_iter()
        .filter(|time| time.posts >= 2)
        .map(|time| time.hour)
        .collect::<Vec<_>>();
    for fallback in default_posting_hours(count) {
        if hours.len() >= count.clamp(1, 5) {
            break;
        }
        if !hours.contains(&fallback) {
            hours.push(fallback);
        }
    }
    hours.truncate(count.clamp(1, 5));
    hours.sort_unstable();
    hours
}
/// The set form of [`publishable`], for statements that work over many posts.
/// Takes no table alias; use it inside `id IN (SELECT id FROM posts WHERE ...)`.
/// Both columns are COALESCEd so the predicate is total: with a NULL
/// `post_type` SQLite would otherwise yield NULL, and `NOT NULL` is NULL, so a
/// row could be neither publishable nor stood down.
const PUBLISHABLE_SQL: &str = "((COALESCE(platform,'instagram')='instagram' AND COALESCE(post_type,'single')='single') OR COALESCE(platform,'instagram')='facebook' OR (COALESCE(platform,'instagram')='tiktok' AND COALESCE(post_type,'single')='reel'))";

/// Whether SocialFlow can actually publish this combination today.
///
/// This is the single source of truth, consulted when content is generated,
/// when a post is moved towards the queue, and when approving in bulk. Without
/// it the campaign generator produces carousels, Reels and story packs that
/// `publish_instagram` structurally refuses, and every one of them fails on its
/// scheduled day after burning eight retries.
fn publishable(platform: &str, post_type: &str) -> bool {
    match platform {
        // Instagram's container API is only implemented for one photograph.
        "instagram" => post_type == "single",
        // Facebook's /photos endpoint accepts anything: the publisher sends the
        // first photograph of the post whatever its type. Live history confirms
        // carousels, Reels and singles have all published to the Page. It is
        // lossy — a seven-photograph carousel goes out as one photograph — but
        // it succeeds, so refusing it here would remove working behaviour.
        "facebook" => true,
        // TikTok publishes the rendered vertical video and nothing else.
        "tiktok" => post_type == "reel",
        _ => false,
    }
}

fn supported(p: &Path) -> bool {
    matches!(
        p.extension()
            .and_then(|x| x.to_str())
            .unwrap_or("")
            .to_lowercase()
            .as_str(),
        "jpg" | "jpeg" | "png" | "webp" | "heic"
    )
}

fn rank_campaign_images(c: &Connection, image_ids: Vec<i64>) -> Vec<i64> {
    let mut section_strength = HashMap::<String, (usize, f64)>::new();
    if let Ok(mut statement) = c.prepare("SELECT section,reach,likes,comments,saves,shares,plays FROM instagram_performance WHERE section<>''") {
        if let Ok(rows) = statement.query_map([], |r| Ok((r.get::<_,String>(0)?,r.get::<_,i64>(1)?,r.get::<_,i64>(2)?,r.get::<_,i64>(3)?,r.get::<_,i64>(4)?,r.get::<_,i64>(5)?,r.get::<_,i64>(6)?))) {
            for (section, reach, likes, comments, saves, shares, plays) in rows.filter_map(Result::ok) {
                let score = engagement_score(reach, likes, comments, saves, shares, plays);
                let entry = section_strength.entry(section).or_default();
                entry.0 += 1;
                entry.1 += score;
            }
        }
    }
    let mut groups = HashMap::<String, Vec<(i64, f64)>>::new();
    let mut visual_hashes = HashMap::<i64, u64>::new();
    for id in image_ids {
        let (section, visual_score, used, provider, visual_hash) = c.query_row(
            "SELECT COALESCE(a.sub_category,'other'),COALESCE(a.social_score,0),i.used_count,COALESCE(a.provider,''),COALESCE(i.perceptual_hash,'') FROM images i LEFT JOIN image_analysis a ON a.image_id=i.id WHERE i.id=?",
            [id],
            |r| Ok((r.get::<_,String>(0)?,r.get::<_,i64>(1)?,r.get::<_,i64>(2)?,r.get::<_,String>(3)?,r.get::<_,String>(4)?)),
        ).unwrap_or_else(|_| ("other".into(), 0, 0, "".into(), "".into()));
        if let Ok(hash) = u64::from_str_radix(&visual_hash, 16) {
            visual_hashes.insert(id, hash);
        }
        let learned = section_strength
            .get(&section)
            .map(|(posts, total)| total / *posts as f64)
            .unwrap_or(0.0);
        let verified_bonus = if matches!(provider.as_str(), "claude" | "openai") {
            8.0
        } else {
            0.0
        };
        let rank = visual_score as f64 + learned * 1.5 + verified_bonus - used as f64 * 12.0;
        groups.entry(section).or_default().push((id, rank));
    }
    groups.values_mut().for_each(|items| {
        items.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal))
    });
    let mut sections = groups.into_iter().collect::<Vec<_>>();
    sections.sort_by(|a, b| {
        b.1.first()
            .map(|x| x.1)
            .unwrap_or(0.0)
            .partial_cmp(&a.1.first().map(|x| x.1).unwrap_or(0.0))
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let mut ranked = Vec::new();
    loop {
        let mut added = false;
        for (_, images) in &mut sections {
            if !images.is_empty() {
                ranked.push(images.remove(0).0);
                added = true;
            }
        }
        if !added {
            break;
        }
    }
    // Keep visually similar frames apart. This prevents burst sequences and tiny
    // expression changes from filling a carousel while retaining every original.
    let mut diverse = Vec::new();
    let mut similar = Vec::new();
    for id in ranked {
        let is_near_duplicate = visual_hashes.get(&id).is_some_and(|candidate| {
            diverse.iter().any(|chosen| {
                visual_hashes
                    .get(chosen)
                    .is_some_and(|existing| (candidate ^ existing).count_ones() <= 6)
            })
        });
        if is_near_duplicate {
            similar.push(id);
        } else {
            diverse.push(id);
        }
    }
    diverse.extend(similar);
    diverse
}
/// Terms that withhold a photograph under a `no_children` consent level.
/// Deliberately broad: a false positive costs one unused photograph, a false
/// negative publishes a child whose family refused permission.
/// "child" also catches "children", "kid" catches "kids", "boy" catches
/// "pageboy". Terms broad enough to catch a whole family of words are preferred
/// over an exhaustive list that will always have a gap in it.
const CHILD_TERMS: [&str; 10] = [
    "child", "kid", "boy", "girl", "baby", "babies", "toddler", "infant",
    "page boy", "daughter",
];

fn marketing_safe_images(
    c: &Connection,
    wedding_id: i64,
    image_ids: Vec<i64>,
) -> Result<Vec<i64>, String> {
    let (consent, embargo) = c
        .query_row(
            "SELECT consent_level,embargo_until FROM weddings WHERE id=?",
            [wedding_id],
            |r| Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?)),
        )
        .map_err(|e| e.to_string())?;
    if consent == "none" || consent == "portfolio_only" {
        return Err("This wedding is not permitted for social-media marketing".into());
    }
    if embargo
        .as_deref()
        .and_then(|date| NaiveDate::parse_from_str(date, "%Y-%m-%d").ok())
        .is_some_and(|date| date > Local::now().date_naive())
    {
        return Err(format!(
            "This wedding is embargoed until {}",
            embargo.unwrap()
        ));
    }
    let filtered = image_ids
        .into_iter()
        .filter(|id| {
            if consent == "selected_only" {
                return c
                    .query_row("SELECT favourite=1 FROM images WHERE id=?", [id], |r| r.get::<_, bool>(0))
                    .unwrap_or(false);
            }
            if consent == "no_children" {
                // Fail closed. An unanalysed photograph cannot be shown to be free
                // of children, so it is withheld rather than approved by default —
                // the permissive branch here published on missing evidence.
                let Ok(description) = c.query_row(
                    "SELECT LOWER(COALESCE(a.description,'')||' '||COALESCE(a.sub_category,'')||' '||COALESCE(a.subjects_json,'')) FROM image_analysis a WHERE a.image_id=? AND COALESCE(a.description,'')<>''",
                    [id],
                    |r| r.get::<_, String>(0),
                ) else {
                    return false;
                };
                return !CHILD_TERMS.iter().any(|term| description.contains(term));
            }
            true
        })
        .collect::<Vec<_>>();
    if filtered.is_empty() {
        return Err("No photographs pass this wedding's marketing permissions".into());
    }
    Ok(filtered)
}

fn caption_too_similar(c: &Connection, caption: &str) -> bool {
    let words = caption
        .split(|ch: char| !ch.is_alphanumeric())
        .filter(|word| word.len() > 4)
        .map(str::to_lowercase)
        .collect::<std::collections::HashSet<_>>();
    c.prepare("SELECT caption FROM posts WHERE caption<>'' ORDER BY id DESC LIMIT 100")
        .ok()
        .and_then(|mut statement| {
            statement
                .query_map([], |r| r.get::<_, String>(0))
                .ok()
                .map(|rows| {
                    rows.filter_map(Result::ok).any(|existing| {
                        let other = existing
                            .split(|ch: char| !ch.is_alphanumeric())
                            .filter(|word| word.len() > 4)
                            .map(str::to_lowercase)
                            .collect::<std::collections::HashSet<_>>();
                        let union = words.union(&other).count().max(1);
                        words.intersection(&other).count() as f64 / union as f64 > 0.62
                    })
                })
        })
        .unwrap_or(false)
}
fn hash_file(p: &Path) -> Result<String, String> {
    let mut f = fs::File::open(p).map_err(|e| e.to_string())?;
    let mut h = Sha256::new();
    let mut b = [0u8; 65536];
    loop {
        let n = f.read(&mut b).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        h.update(&b[..n])
    }
    Ok(format!("{:x}", h.finalize()))
}
fn perceptual_hash(image: &image::DynamicImage) -> String {
    let grey = image
        .resize_exact(9, 8, image::imageops::FilterType::Triangle)
        .to_luma8();
    let mut bits = 0_u64;
    for y in 0..8 {
        for x in 0..8 {
            bits <<= 1;
            if grey.get_pixel(x, y)[0] > grey.get_pixel(x + 1, y)[0] {
                bits |= 1;
            }
        }
    }
    format!("{bits:016x}")
}
#[tauri::command]
fn import_paths(
    paths: Vec<String>,
    recursive: bool,
    profile_id: i64,
    state: State<'_, AppState>,
) -> Result<ImportResult, String> {
    let mut files = Vec::new();
    for s in paths {
        let p = PathBuf::from(s);
        if p.is_dir() {
            for e in WalkDir::new(&p)
                .max_depth(if recursive { usize::MAX } else { 1 })
                .into_iter()
                .filter_map(Result::ok)
            {
                if e.file_type().is_file() && supported(e.path()) {
                    files.push((e.path().to_path_buf(), Some(p.clone())))
                }
            }
        } else if supported(&p) {
            files.push((p, None))
        }
    }
    let c = state.db.lock().unwrap();
    let mut out = ImportResult {
        imported: 0,
        duplicates: 0,
        errors: vec![],
    };
    for (p, folder) in files {
        match import_one(&c, &state.cache, &p, folder.as_deref(), profile_id) {
            Ok(true) => out.imported += 1,
            Ok(false) => out.duplicates += 1,
            Err(e) => out.errors.push(format!("{}: {}", p.display(), e)),
        }
    }
    Ok(out)
}
fn import_one(
    c: &Connection,
    cache: &Path,
    p: &Path,
    folder: Option<&Path>,
    profile: i64,
) -> Result<bool, String> {
    let hash = hash_file(p)?;
    if c.query_row("SELECT id FROM images WHERE file_hash=?", [&hash], |r| {
        r.get::<_, i64>(0)
    })
    .optional()
    .map_err(|e| e.to_string())?
    .is_some()
    {
        return Ok(false);
    }
    let collection = if let Some(f) = folder {
        c.execute(
            "INSERT OR IGNORE INTO collections(name,folder_path,profile_id)VALUES(?,?,?)",
            params![
                f.file_name()
                    .and_then(|x| x.to_str())
                    .unwrap_or("Collection"),
                f.to_string_lossy(),
                profile
            ],
        )
        .map_err(|e| e.to_string())?;
        Some(
            c.query_row(
                "SELECT id FROM collections WHERE folder_path=? AND profile_id=?",
                params![f.to_string_lossy(), profile],
                |r| r.get::<_, i64>(0),
            )
            .map_err(|e| e.to_string())?,
        )
    } else {
        None
    };
    let meta = fs::metadata(p).map_err(|e| e.to_string())?;
    let decoded = image::open(p).ok();
    let dims = decoded.as_ref().map(|i| i.dimensions());
    let visual_hash = decoded.as_ref().map(perceptual_hash).unwrap_or_default();
    let thumb = cache.join(format!("{}.jpg", hash));
    if let Some(im) = decoded {
        let t = im.thumbnail(600, 600);
        t.save_with_format(&thumb, image::ImageFormat::Jpeg)
            .map_err(|e| e.to_string())?
    }
    let orient = dims.map(|(w, h)| {
        if w > h {
            "landscape"
        } else if h > w {
            "portrait"
        } else {
            "square"
        }
    });
    c.execute("INSERT INTO images(collection_id,source_path,filename,extension,file_hash,width,height,orientation,file_size,thumbnail_path,perceptual_hash,last_verified_at)VALUES(?,?,?,?,?,?,?,?,?,?,?,CURRENT_TIMESTAMP)",params![collection,p.to_string_lossy(),p.file_name().and_then(|x|x.to_str()).unwrap_or("image"),p.extension().and_then(|x|x.to_str()),hash,dims.map(|x|x.0),dims.map(|x|x.1),orient,meta.len() as i64,if thumb.exists(){Some(thumb.to_string_lossy().to_string())}else{None},visual_hash]).map_err(|e|e.to_string())?;
    Ok(true)
}
#[tauri::command]
fn create_post(
    image_ids: Vec<i64>,
    profile_id: i64,
    state: State<AppState>,
) -> Result<i64, String> {
    if image_ids.is_empty() {
        return Err("Choose at least one photograph".into());
    }
    let mut c = state.db.lock().unwrap();
    let tx = c.transaction().map_err(|e| e.to_string())?;
    tx.execute(
        "INSERT INTO posts(profile_id,post_type)VALUES(?,?)",
        params![
            profile_id,
            if image_ids.len() > 1 {
                "carousel"
            } else {
                "single"
            }
        ],
    )
    .map_err(|e| e.to_string())?;
    let id = tx.last_insert_rowid();
    for (pos, img) in image_ids.iter().enumerate() {
        tx.execute(
            "INSERT INTO post_images(post_id,image_id,position)VALUES(?,?,?)",
            params![id, img, pos],
        )
        .map_err(|e| e.to_string())?;
    }
    tx.commit().map_err(|e| e.to_string())?;
    Ok(id)
}
#[tauri::command]
fn update_post_status(post_id: i64, status: String, state: State<AppState>) -> Result<(), String> {
    let allowed = [
        "draft",
        "needs_review",
        "approved",
        "scheduled",
        "publishing",
        "published",
        "failed",
    ];
    if !allowed.contains(&status.as_str()) {
        return Err("Invalid post status".into());
    }
    let c = state.db.lock().unwrap();
    let (current, platform, post_type) = c
        .query_row(
            "SELECT status,COALESCE(platform,'instagram'),post_type FROM posts WHERE id=?",
            [post_id],
            |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                ))
            },
        )
        .map_err(|e| e.to_string())?;
    // Refuse the queue rather than accepting a post that is certain to fail on
    // its scheduled day. Say why, and say what the post is still good for.
    if matches!(status.as_str(), "scheduled" | "publishing")
        && !publishable(&platform, &post_type)
    {
        return Err(format!(
            "SocialFlow cannot publish a {post_type} to {platform} yet, so this post cannot be scheduled. The rendered asset is ready to post by hand from the post's Reveal asset button."
        ));
    }
    let valid = matches!(
        (current.as_str(), status.as_str()),
        ("draft", "needs_review" | "approved")
            | ("needs_review", "draft" | "approved")
            | ("approved", "draft" | "scheduled")
            | ("scheduled", "approved" | "publishing")
            | ("publishing", "published" | "failed")
            | ("failed", "draft" | "approved" | "scheduled")
    ) || current == status;
    if !valid {
        return Err(format!("Cannot move from {current} to {status}"));
    }
    c.execute(
        "UPDATE posts SET status=?,updated_at=CURRENT_TIMESTAMP WHERE id=?",
        params![status, post_id],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}
#[tauri::command]
fn create_campaign(
    profile_id: i64,
    image_ids: Vec<i64>,
    count: usize,
    posts_per_week: u32,
    posts_per_day: Option<u32>,
    wedding_id: Option<i64>,
    state: State<AppState>,
) -> Result<i64, String> {
    if image_ids.is_empty() {
        return Err("Campaign needs photographs".into());
    }
    let mut c = state.db.lock().unwrap();
    let tx = c.transaction().map_err(|e| e.to_string())?;
    let start = Local::now().date_naive();
    let daily = posts_per_day.unwrap_or(0).min(5);
    let end = if daily > 0 {
        start + Duration::days(((count as f64 / daily as f64).ceil()) as i64)
    } else {
        start + Duration::weeks(((count as f64 / posts_per_week.max(1) as f64).ceil()) as i64)
    };
    let mut ids = if let Some(wedding_id) = wedding_id {
        marketing_safe_images(&tx, wedding_id, image_ids)?
    } else {
        image_ids
    };
    ids.retain(|id| {
        !tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM post_images WHERE image_id=?)",
            [id],
            |r| r.get::<_, bool>(0),
        )
        .unwrap_or(true)
    });
    if ids.is_empty() {
        return Err("Every photograph in this wedding already has a post".into());
    }
    let actual_count = count.min(ids.len());
    tx.execute("INSERT INTO campaigns(profile_id,name,start_date,end_date,requested_post_count,posts_per_week)VALUES(?,'New campaign',?,?,?,?)",params![profile_id,start.to_string(),end.to_string(),actual_count,posts_per_week]).map_err(|e|e.to_string())?;
    let campaign = tx.last_insert_rowid();
    ids = rank_campaign_images(&tx, ids);
    let (couple, venue) = wedding_id
        .and_then(|w| {
            tx.query_row(
                "SELECT couple_names,venue FROM weddings WHERE id=?",
                [w],
                |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)),
            )
            .ok()
        })
        .unwrap_or(("this pair".into(), "the day".into()));
    for (i, img) in ids.into_iter().take(actual_count).enumerate() {
        let d = if daily > 0 {
            next_daily_slot(start, i, daily)
        } else {
            next_slot(start, i)
        };
        let category: String = tx
            .query_row(
                "SELECT COALESCE(sub_category,'wedding story') FROM image_analysis WHERE image_id=?",
                [img],
                |r| r.get(0),
            )
            .unwrap_or_else(|_| "wedding story".into());
        let visual = tx
            .query_row(
                "SELECT analysis_json FROM image_analysis WHERE image_id=? AND provider IN ('claude','openai')",
                [img],
                |r| r.get::<_, String>(0),
            )
            .ok()
            .and_then(|json| serde_json::from_str::<VisualAnalysis>(&json).ok());
        let mut caption = visual
            .as_ref()
            .map(|analysis| analysis.caption.clone())
            .unwrap_or_else(|| brand_caption(&couple, &venue, &category, i));
        if caption_too_similar(&tx, &caption) {
            caption = brand_caption(&couple, &venue, &category, i + 3);
        }
        let tag_list = visual
            .map(|analysis| analysis.hashtags)
            .unwrap_or_else(|| wedding_hashtags(&venue, &category, i));
        add_supplier_context(&tx, wedding_id, &mut caption, i);
        let tags = serde_json::to_string(&tag_list).unwrap();
        tx.execute("INSERT INTO posts(profile_id,caption,hashtags_json,status,scheduled_at,post_type,ai_generated)VALUES(?,?,?,'needs_review',?,'single',1)",params![profile_id,caption,tags,d.to_string()]).map_err(|e|e.to_string())?;
        let post = tx.last_insert_rowid();
        tx.execute(
            "INSERT INTO post_images(post_id,image_id,position)VALUES(?,?,0)",
            params![post, img],
        )
        .map_err(|e| e.to_string())?;
    }
    if daily > 0 {
        reflow_wedding_rotation(&tx, start, daily)?;
    }
    tx.commit().map_err(|e| e.to_string())?;
    Ok(campaign)
}

fn ffmpeg_binary() -> Result<PathBuf, String> {
    [
        "/opt/homebrew/bin/ffmpeg",
        "/usr/local/bin/ffmpeg",
        "/usr/bin/ffmpeg",
    ]
    .into_iter()
    .map(PathBuf::from)
    .find(|path| path.exists())
    .ok_or_else(|| "Reel rendering needs FFmpeg. Install it with: brew install ffmpeg".into())
}

fn render_photo_reel(paths: &[String], output: &Path) -> Result<(), String> {
    if paths.len() < 2 {
        return Err("A Reel needs at least two photographs".into());
    }
    if let Some(parent) = output.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let mut command = Command::new(ffmpeg_binary()?);
    command.args(["-y", "-hide_banner", "-loglevel", "error"]);
    for path in paths {
        command.args(["-loop", "1", "-t", "1.8", "-i", path]);
    }
    let mut filters = Vec::new();
    for index in 0..paths.len() {
        filters.push(format!("[{index}:v]scale=1080:1920:force_original_aspect_ratio=increase,crop=1080:1920,zoompan=z='min(zoom+0.0009,1.08)':d=1:s=1080x1920:fps=30,fade=t=in:st=0:d=0.22,fade=t=out:st=1.58:d=0.22,setsar=1[v{index}]"));
    }
    let inputs = (0..paths.len())
        .map(|index| format!("[v{index}]"))
        .collect::<String>();
    filters.push(format!(
        "{inputs}concat=n={}:v=1:a=0,format=yuv420p[outv]",
        paths.len()
    ));
    command.args([
        "-filter_complex",
        &filters.join(";"),
        "-map",
        "[outv]",
        "-r",
        "30",
        "-movflags",
        "+faststart",
        "-c:v",
        "libx264",
        "-preset",
        "veryfast",
        "-crf",
        "20",
        output.to_string_lossy().as_ref(),
    ]);
    let result = command
        .output()
        .map_err(|e| format!("Could not start Reel renderer: {e}"))?;
    if !result.status.success() {
        return Err(format!(
            "Reel rendering failed: {}",
            String::from_utf8_lossy(&result.stderr)
        ));
    }
    Ok(())
}

fn export_story_images(paths: &[String], output: &Path) -> Result<(), String> {
    fs::create_dir_all(output).map_err(|e| e.to_string())?;
    for (index, source) in paths.iter().enumerate() {
        let image =
            image::open(source).map_err(|e| format!("Could not open story photograph: {e}"))?;
        let resized = image.resize_to_fill(1080, 1920, image::imageops::FilterType::Lanczos3);
        resized
            .save_with_format(
                output.join(format!("story-{:02}.jpg", index + 1)),
                image::ImageFormat::Jpeg,
            )
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[tauri::command]
fn create_content_campaign(
    profile_id: i64,
    image_ids: Vec<i64>,
    count: usize,
    posts_per_day: u32,
    wedding_id: i64,
    formats: Vec<String>,
    state: State<AppState>,
) -> Result<i64, String> {
    if image_ids.is_empty() || formats.is_empty() {
        return Err("Choose photographs and at least one content format".into());
    }
    let mut c = state.db.lock().unwrap();
    let tx = c.transaction().map_err(|e| e.to_string())?;
    let (couple, venue, collection_id) = tx
        .query_row(
            "SELECT couple_names,venue,collection_id FROM weddings WHERE id=?",
            [wedding_id],
            |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, Option<i64>>(2)?,
                ))
            },
        )
        .map_err(|e| e.to_string())?;
    let mut ids = marketing_safe_images(&tx, wedding_id, image_ids)?;
    ids.retain(|id| {
        !tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM post_images WHERE image_id=?)",
            [id],
            |r| r.get::<_, bool>(0),
        )
        .unwrap_or(true)
    });
    ids = rank_campaign_images(&tx, ids);
    if ids.is_empty() {
        return Err("Every photograph in this wedding is already assigned to content".into());
    }
    let start = Local::now().date_naive();
    let facebook_separate = tx.query_row("SELECT EXISTS(SELECT 1 FROM facebook_accounts WHERE profile_id=? AND connected=1) AND COALESCE((SELECT value FROM app_settings WHERE key='facebook_separate_posts'),'false')='true'", [profile_id], |r| r.get::<_,bool>(0)).unwrap_or(false);
    let tiktok_separate = tx.query_row("SELECT EXISTS(SELECT 1 FROM tiktok_accounts WHERE profile_id=? AND connected=1) AND COALESCE((SELECT value FROM app_settings WHERE key='tiktok_reel_copies'),'false')='true'", [profile_id], |r| r.get::<_,bool>(0)).unwrap_or(false);
    let mut created_post_ids = Vec::new();
    tx.execute("INSERT INTO campaigns(profile_id,collection_id,name,start_date,requested_post_count,posts_per_week,status)VALUES(?,?,?, ?,?,35,'proposed')", params![profile_id,collection_id,format!("{} Content Studio",couple),start.to_string(),count]).map_err(|e|e.to_string())?;
    let campaign_id = tx.last_insert_rowid();
    let mut cursor = 0usize;
    for index in 0..count {
        if cursor >= ids.len() {
            break;
        }
        let format = formats[index % formats.len()].as_str();
        let wanted = match format {
            "carousel" => 7,
            "reel" => 12,
            "story_pack" => 5,
            _ => 1,
        };
        let chosen = ids[cursor..ids.len().min(cursor + wanted)].to_vec();
        if chosen.is_empty() {
            break;
        }
        if format == "reel" && chosen.len() < 2 {
            break;
        }
        cursor += chosen.len();
        let category = tx.query_row("SELECT COALESCE(sub_category,'wedding story') FROM image_analysis WHERE image_id=?", [chosen[0]], |r| r.get::<_, String>(0)).unwrap_or_else(|_| "wedding story".into());
        let visual = tx.query_row("SELECT analysis_json FROM image_analysis WHERE image_id=? AND provider IN ('claude','openai')", [chosen[0]], |r| r.get::<_, String>(0)).ok().and_then(|json| serde_json::from_str::<VisualAnalysis>(&json).ok());
        let mut caption = visual
            .as_ref()
            .map(|analysis| analysis.caption.clone())
            .unwrap_or_else(|| brand_caption(&couple, &venue, &category, index));
        if caption_too_similar(&tx, &caption) {
            caption = brand_caption(&couple, &venue, &category, index + 3);
        }
        let tag_list = visual
            .map(|analysis| analysis.hashtags)
            .unwrap_or_else(|| wedding_hashtags(&venue, &category, index));
        add_supplier_context(&tx, Some(wedding_id), &mut caption, index);
        let tags = serde_json::to_string(&tag_list).unwrap();
        let learned_hours = learned_posting_hours(&tx, posts_per_day.clamp(1, 5) as usize);
        // Only queue what the publisher can actually ship. Formats it cannot
        // publish are still rendered and kept as drafts to use by hand, but they
        // never take a slot and never fail on their scheduled day.
        let can_publish = publishable("instagram", format);
        let scheduled = can_publish
            .then(|| next_daily_slot_with_hours(start, index, &learned_hours).to_string());
        let status = if can_publish { "needs_review" } else { "draft" };
        tx.execute("INSERT INTO posts(profile_id,caption,hashtags_json,status,scheduled_at,post_type,ai_generated)VALUES(?,?,?,?,?,?,1)",params![profile_id,caption,tags,status,scheduled,format]).map_err(|e|e.to_string())?;
        let post_id = tx.last_insert_rowid();
        created_post_ids.push(post_id);
        let mut paths = Vec::new();
        for (position, image_id) in chosen.iter().enumerate() {
            tx.execute(
                "INSERT INTO post_images(post_id,image_id,position)VALUES(?,?,?)",
                params![post_id, image_id, position],
            )
            .map_err(|e| e.to_string())?;
            if let Ok(path) = tx.query_row(
                "SELECT source_path FROM images WHERE id=?",
                [image_id],
                |r| r.get::<_, String>(0),
            ) {
                paths.push(path)
            }
        }
        let asset = if format == "reel" {
            let path = state
                .cache
                .join("reels")
                .join(format!("reel-{post_id}.mp4"));
            render_photo_reel(&paths, &path)?;
            Some(path.to_string_lossy().to_string())
        } else if format == "story_pack" {
            let path = state
                .cache
                .join("story-packs")
                .join(format!("{}-{post_id}", couple.replace([' ', '&'], "-")));
            export_story_images(&paths, &path)?;
            Some(path.to_string_lossy().to_string())
        } else {
            None
        };
        if let Some(path) = asset {
            tx.execute(
                "UPDATE posts SET asset_path=? WHERE id=?",
                params![path, post_id],
            )
            .map_err(|e| e.to_string())?;
        }
    }
    let mut copied_post_ids = Vec::new();
    if facebook_separate {
        for instagram_post_id in &created_post_ids {
            tx.execute("INSERT INTO posts(profile_id,caption,hashtags_json,status,scheduled_at,published_at,post_type,created_at,updated_at,manually_edited_caption,ai_generated,asset_path,platform) SELECT profile_id,caption,hashtags_json,'needs_review',scheduled_at,NULL,post_type,CURRENT_TIMESTAMP,CURRENT_TIMESTAMP,0,ai_generated,asset_path,'facebook' FROM posts WHERE id=?", [instagram_post_id]).map_err(|e|e.to_string())?;
            let facebook_post_id = tx.last_insert_rowid();
            copied_post_ids.push(facebook_post_id);
            tx.execute("INSERT INTO post_images(post_id,image_id,position) SELECT ?,image_id,position FROM post_images WHERE post_id=?", params![facebook_post_id,instagram_post_id]).map_err(|e|e.to_string())?;
        }
    }
    if tiktok_separate {
        for instagram_post_id in &created_post_ids {
            let is_reel = tx.query_row("SELECT post_type='reel' FROM posts WHERE id=?", [instagram_post_id], |r| r.get::<_,bool>(0)).unwrap_or(false);
            if !is_reel { continue; }
            tx.execute("INSERT INTO posts(profile_id,caption,hashtags_json,status,scheduled_at,published_at,post_type,created_at,updated_at,manually_edited_caption,ai_generated,asset_path,platform) SELECT profile_id,caption,hashtags_json,'needs_review',scheduled_at,NULL,post_type,CURRENT_TIMESTAMP,CURRENT_TIMESTAMP,0,ai_generated,asset_path,'tiktok' FROM posts WHERE id=?", [instagram_post_id]).map_err(|e|e.to_string())?;
            let tiktok_post_id = tx.last_insert_rowid();
            copied_post_ids.push(tiktok_post_id);
            tx.execute("INSERT INTO post_images(post_id,image_id,position) SELECT ?,image_id,position FROM post_images WHERE post_id=?", params![tiktok_post_id,instagram_post_id]).map_err(|e|e.to_string())?;
        }
    }
    // A copy inherits its parent's type, so a Facebook copy of a carousel is as
    // unpublishable as the carousel. Stand those down too, scoped strictly to
    // the posts this run created.
    for post_id in &copied_post_ids {
        tx.execute(
            &format!("UPDATE posts SET status='draft',scheduled_at=NULL WHERE id=? AND NOT {PUBLISHABLE_SQL}"),
            [post_id],
        )
        .map_err(|e| e.to_string())?;
    }
    // Reflow last, so the slots go to the posts that can actually use them —
    // including the TikTok copies, whose Instagram parents are now drafts.
    reflow_wedding_rotation(&tx, start, posts_per_day.clamp(1, 5))?;
    tx.commit().map_err(|e| e.to_string())?;
    Ok(campaign_id)
}

#[tauri::command]
fn reveal_post_asset(post_id: i64, state: State<AppState>) -> Result<(), String> {
    let c = state.db.lock().unwrap();
    let path: String = c
        .query_row("SELECT asset_path FROM posts WHERE id=?", [post_id], |r| {
            r.get(0)
        })
        .map_err(|_| "This post has no generated asset yet".to_string())?;
    let target = if Path::new(&path).is_dir() {
        path
    } else {
        Path::new(&path)
            .parent()
            .unwrap_or(Path::new(&path))
            .to_string_lossy()
            .to_string()
    };
    Command::new("/usr/bin/open")
        .arg(target)
        .spawn()
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[cfg(test)]
fn rotation_plan(
    start: NaiveDate,
    counts: &[usize],
    posts_per_day: u32,
) -> Vec<Vec<chrono::DateTime<Local>>> {
    let daily = posts_per_day.clamp(1, 5) as usize;
    rotation_plan_with_hours(start, counts, posts_per_day, &default_posting_hours(daily))
}

fn rotation_plan_with_hours(
    start: NaiveDate,
    counts: &[usize],
    posts_per_day: u32,
    hours: &[u32],
) -> Vec<Vec<chrono::DateTime<Local>>> {
    let daily = posts_per_day.clamp(1, 5) as usize;
    let mut result = counts.iter().map(|_| Vec::new()).collect::<Vec<_>>();
    let mut remaining = counts.to_vec();
    let mut day_offset = 0_i64;
    while remaining.iter().any(|count| *count > 0) {
        for wedding in 0..remaining.len() {
            if remaining[wedding] == 0 {
                continue;
            }
            let batch = remaining[wedding].min(daily);
            let day = start + Duration::days(day_offset);
            for hour in hours.iter().take(batch) {
                result[wedding].push(
                    Local
                        .from_local_datetime(&day.and_hms_opt(*hour, 0, 0).unwrap())
                        .single()
                        .unwrap(),
                );
            }
            remaining[wedding] -= batch;
            day_offset += 1;
        }
    }
    result
}

fn reflow_wedding_rotation(
    tx: &Transaction<'_>,
    start: NaiveDate,
    posts_per_day: u32,
) -> Result<(), String> {
    let wedding_ids = tx
        .prepare(&format!("SELECT w.id FROM weddings w WHERE EXISTS(SELECT 1 FROM posts p JOIN post_images pi ON pi.post_id=p.id JOIN images i ON i.id=pi.image_id WHERE i.collection_id=w.collection_id AND p.status IN ('draft','needs_review','approved','scheduled') AND p.id IN (SELECT id FROM posts WHERE {PUBLISHABLE_SQL})) ORDER BY w.created_at,w.id"))
        .map_err(|e| e.to_string())?
        .query_map([], |r| r.get::<_, i64>(0))
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    let mut groups = Vec::new();
    for wedding_id in wedding_ids {
        let post_ids = tx
            .prepare(&format!("SELECT DISTINCT p.id FROM posts p JOIN post_images pi ON pi.post_id=p.id JOIN images i ON i.id=pi.image_id JOIN weddings w ON w.collection_id=i.collection_id WHERE w.id=? AND p.status IN ('draft','needs_review','approved','scheduled') AND p.id IN (SELECT id FROM posts WHERE {PUBLISHABLE_SQL}) ORDER BY p.id"))
            .map_err(|e| e.to_string())?
            .query_map([wedding_id], |r| r.get::<_, i64>(0))
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        groups.push(post_ids);
    }
    let learned_hours = learned_posting_hours(tx, posts_per_day.clamp(1, 5) as usize);
    let plan = rotation_plan_with_hours(
        start,
        &groups.iter().map(Vec::len).collect::<Vec<_>>(),
        posts_per_day,
        &learned_hours,
    );
    for (post_ids, slots) in groups.iter().zip(plan) {
        for (post_id, slot) in post_ids.iter().zip(slots) {
            tx.execute(
                "UPDATE posts SET scheduled_at=?,updated_at=CURRENT_TIMESTAMP WHERE id=?",
                params![slot.to_string(), post_id],
            )
            .map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}
fn next_daily_slot(start: NaiveDate, index: usize, posts_per_day: u32) -> chrono::DateTime<Local> {
    let count = posts_per_day.clamp(1, 5) as usize;
    next_daily_slot_with_hours(start, index, &default_posting_hours(count))
}
fn next_daily_slot_with_hours(
    start: NaiveDate,
    index: usize,
    hours: &[u32],
) -> chrono::DateTime<Local> {
    let count = hours.len().max(1);
    let day = start + Duration::days((index / count) as i64);
    Local
        .from_local_datetime(&day.and_hms_opt(hours[index % count], 0, 0).unwrap())
        .single()
        .unwrap()
}
fn brand_caption(couple: &str, venue: &str, category: &str, index: usize) -> String {
    let lines=[format!("Nobody planned this bit. Which is precisely why it mattered.\n\n{} at {} — the day carrying on while everyone forgot about the camera.",couple,venue),format!("The official description: {}.\n\nThe accurate description: a room full of people thoroughly enjoying themselves while I kept out of the way.",category),format!("This lasted about three seconds. Long enough.\n\n{} at {}, exactly as it happened.",couple,venue),"One of the bits they didn't see until the gallery arrived.\n\nNo direction. No repeat. Just paying attention.".to_string(),format!("A strong argument for letting weddings be weddings.\n\n{} — unscripted, unpolished and considerably better for it.",category)];
    lines[index % lines.len()].clone()
}
fn wedding_hashtags(venue: &str, category: &str, index: usize) -> Vec<String> {
    let clean = |s: &str| {
        s.to_lowercase()
            .chars()
            .filter(|c| c.is_alphanumeric())
            .collect::<String>()
    };
    let pools = [
        "#documentaryweddingphotography",
        "#northwestweddingphotographer",
        "#lancashireweddingphotographer",
        "#lakeDistrictweddingphotographer",
        "#cheshireweddingphotographer",
    ];
    vec![
        format!("#{}", clean(venue)),
        format!("#{}", clean(category)),
        pools[index % pools.len()].to_string(),
        "#thebeardedweddingphotographer".into(),
        "#weddingstorytelling".into(),
    ]
}
fn add_supplier_context(
    c: &Connection,
    wedding_id: Option<i64>,
    caption: &mut String,
    index: usize,
) {
    let Some(wedding_id) = wedding_id else { return };
    let suppliers = c
        .prepare("SELECT role,instagram_handle FROM suppliers WHERE wedding_id=? AND instagram_confirmed=1 AND instagram_handle<>'' ORDER BY role,name")
        .ok()
        .and_then(|mut statement| {
            statement
                .query_map([wedding_id], |r| {
                    Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
                })
                .ok()
                .map(|rows| rows.filter_map(Result::ok).collect::<Vec<_>>())
        })
        .unwrap_or_default();
    if let Some((role, handle)) = suppliers.get(index % suppliers.len().max(1)) {
        let clean = handle.trim().trim_start_matches('@');
        if !clean.is_empty() {
            caption.push_str(&format!("\n\n{}: @{}", role, clean));
        }
    }
}
fn next_slot(start: NaiveDate, index: usize) -> chrono::DateTime<Local> {
    let mut d = start;
    let mut found = 0;
    loop {
        if [1, 3, 5].contains(&d.weekday().num_days_from_sunday()) {
            if found == index {
                return Local
                    .from_local_datetime(&d.and_hms_opt(19, 0, 0).unwrap())
                    .single()
                    .unwrap();
            }
            found += 1
        }
        d += Duration::days(1)
    }
}
#[tauri::command]
fn set_setting(key: String, value: String, state: State<AppState>) -> Result<(), String> {
    let c = state.db.lock().unwrap();
    c.execute("INSERT INTO app_settings(key,value)VALUES(?,?) ON CONFLICT(key)DO UPDATE SET value=excluded.value",params![key,value]).map_err(|e|e.to_string())?;
    Ok(())
}
#[tauri::command]
fn backup_socialflow(state: State<AppState>) -> Result<String, String> {
    let c = state.db.lock().unwrap();
    let source: String = c
        .query_row(
            "SELECT file FROM pragma_database_list WHERE name='main'",
            [],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    let folder = Path::new(&source)
        .parent()
        .unwrap_or(Path::new("."))
        .join("backups");
    fs::create_dir_all(&folder).map_err(|e| e.to_string())?;
    let path = folder.join(format!(
        "socialflow-{}.db",
        Local::now().format("%Y%m%d-%H%M%S")
    ));
    c.execute("VACUUM INTO ?", [path.to_string_lossy().to_string()])
        .map_err(|e| e.to_string())?;
    c.execute(
        "INSERT INTO app_backups(path)VALUES(?)",
        [path.to_string_lossy().to_string()],
    )
    .map_err(|e| e.to_string())?;
    c.execute("INSERT INTO app_settings(key,value)VALUES('last_backup_at',CURRENT_TIMESTAMP) ON CONFLICT(key) DO UPDATE SET value=CURRENT_TIMESTAMP", []).map_err(|e| e.to_string())?;
    Ok(path.to_string_lossy().to_string())
}
#[tauri::command]
fn return_failed_to_review(state: State<AppState>) -> Result<usize, String> {
    state.db.lock().unwrap().execute("UPDATE posts SET status='needs_review',updated_at=CURRENT_TIMESTAMP WHERE status='failed'", []).map_err(|e|e.to_string())
}

#[tauri::command]
fn start_live_publisher(state: State<AppState>) -> Result<(), String> {
    let c = state.db.lock().unwrap();
    let instagram_due: bool = c.query_row("SELECT EXISTS(SELECT 1 FROM posts WHERE status='scheduled' AND COALESCE(platform,'instagram')='instagram')", [], |row| row.get(0)).unwrap_or(false);
    let account: Option<String> = if instagram_due {
        Some(c.query_row("SELECT instagram_user_id FROM instagram_accounts WHERE profile_id=1 AND connected=1", [], |row| row.get(0)).map_err(|_| "Connect Instagram before starting automatic publishing".to_string())?)
    } else { None };
    let due: bool = c
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM posts WHERE status='scheduled')",
            [],
            |row| row.get(0),
        )
        .unwrap_or(false);
    drop(c);
    if !due {
        return Ok(());
    }
    let already_running = Command::new("/usr/bin/pgrep")
        .args(["-f", "socialflow_live_publisher.py --due"])
        .status()
        .is_ok_and(|status| status.success());
    if already_running {
        return Ok(());
    }
    let token = account.as_ref().map(|account| {
        keyring::Entry::new("com.socialflow.desktop.instagram", account)
            .map_err(|error| error.to_string())?
            .get_password()
            .map_err(|_| "Instagram token is unavailable. Reconnect Instagram in Settings.".to_string())
    }).transpose()?;
    let mut publisher = Command::new("/usr/bin/python3");
    publisher.arg("/Users/wayne/Documents/waynes buffer/scripts/socialflow_live_publisher.py").arg("--due");
    if let Some(token) = token { publisher.env("SOCIALFLOW_IG_TOKEN", token); }
    publisher.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn()
        .map_err(|error| format!("Could not start automatic publishing: {error}"))?;
    Ok(())
}
#[tauri::command]
fn index_visual_duplicates(state: State<AppState>) -> Result<usize, String> {
    let c = state.db.lock().unwrap();
    let mut statement = c
        .prepare("SELECT id,thumbnail_path FROM images WHERE COALESCE(perceptual_hash,'')='' AND thumbnail_path IS NOT NULL")
        .map_err(|e| e.to_string())?;
    let candidates = statement
        .query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))
        .map_err(|e| e.to_string())?
        .filter_map(Result::ok)
        .collect::<Vec<_>>();
    drop(statement);
    let mut indexed = 0;
    for (id, path) in candidates {
        if let Ok(image) = image::open(path) {
            c.execute(
                "UPDATE images SET perceptual_hash=? WHERE id=?",
                params![perceptual_hash(&image), id],
            )
            .map_err(|e| e.to_string())?;
            indexed += 1;
        }
    }
    Ok(indexed)
}
#[tauri::command]
fn record_marketing_lead(
    source_post_id: Option<i64>,
    source: String,
    value: f64,
    state: State<AppState>,
) -> Result<i64, String> {
    let c = state.db.lock().unwrap();
    c.execute(
        "INSERT INTO marketing_leads(source_post_id,source,value)VALUES(?,?,?)",
        params![source_post_id, source, value],
    )
    .map_err(|e| e.to_string())?;
    Ok(c.last_insert_rowid())
}
#[tauri::command]
#[allow(clippy::too_many_arguments)]
fn create_wedding(
    collection_id: Option<i64>,
    folder_path: Option<String>,
    couple_names: String,
    wedding_date: String,
    venue: String,
    region: String,
    consent_level: String,
    embargo_until: Option<String>,
    campaign_goal: String,
    profile_id: i64,
    state: State<AppState>,
) -> Result<i64, String> {
    if couple_names.trim().is_empty() {
        return Err("Enter the couple's names".into());
    }
    let c = state.db.lock().unwrap();
    let resolved = collection_id.or_else(|| {
        folder_path.and_then(|p| {
            c.query_row(
                "SELECT id FROM collections WHERE folder_path=? AND profile_id=?",
                params![p, profile_id],
                |r| r.get(0),
            )
            .ok()
        })
    });
    c.execute("INSERT INTO weddings(collection_id,profile_id,couple_names,wedding_date,venue,region,consent_level,embargo_until,campaign_goal)VALUES(?,?,?,?,?,?,?,?,?)",params![resolved,profile_id,couple_names,wedding_date,venue,region,consent_level,embargo_until.filter(|x|!x.is_empty()),campaign_goal]).map_err(|e|e.to_string())?;
    Ok(c.last_insert_rowid())
}
#[tauri::command]
#[allow(clippy::too_many_arguments)]
fn update_wedding(
    wedding_id: i64,
    collection_id: Option<i64>,
    folder_path: Option<String>,
    couple_names: String,
    wedding_date: String,
    venue: String,
    region: String,
    consent_level: String,
    embargo_until: Option<String>,
    campaign_goal: String,
    state: State<AppState>,
) -> Result<(), String> {
    if couple_names.trim().is_empty() {
        return Err("Enter the couple's names".into());
    }
    let c = state.db.lock().unwrap();
    let resolved = collection_id.or_else(|| {
        folder_path.and_then(|path| {
            c.query_row(
                "SELECT id FROM collections WHERE folder_path=? ORDER BY id DESC LIMIT 1",
                [path],
                |row| row.get(0),
            )
            .ok()
        })
    });
    c.execute("UPDATE weddings SET collection_id=COALESCE(?,collection_id),couple_names=?,wedding_date=?,venue=?,region=?,consent_level=?,embargo_until=?,campaign_goal=?,updated_at=CURRENT_TIMESTAMP WHERE id=?", params![resolved,couple_names,wedding_date,venue,region,consent_level,embargo_until.filter(|value| !value.is_empty()),campaign_goal,wedding_id]).map_err(|e|e.to_string())?;
    Ok(())
}
#[tauri::command]
fn add_supplier(
    wedding_id: i64,
    role: String,
    name: String,
    instagram_handle: String,
    website: String,
    state: State<AppState>,
) -> Result<i64, String> {
    if name.trim().is_empty() {
        return Err("Enter a supplier name".into());
    }
    let c = state.db.lock().unwrap();
    c.execute(
        "INSERT INTO suppliers(wedding_id,role,name,instagram_handle,website)VALUES(?,?,?,?,?)",
        params![wedding_id, role, name, instagram_handle, website],
    )
    .map_err(|e| e.to_string())?;
    Ok(c.last_insert_rowid())
}

#[tauri::command]
fn delete_wedding(wedding_id: i64, state: State<AppState>) -> Result<(), String> {
    let c = state.db.lock().unwrap();
    c.execute("DELETE FROM weddings WHERE id=?", [wedding_id])
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
fn replace_suppliers(
    wedding_id: i64,
    suppliers: Vec<SupplierInput>,
    state: State<AppState>,
) -> Result<(), String> {
    let mut c = state.db.lock().unwrap();
    let tx = c.transaction().map_err(|e| e.to_string())?;
    tx.execute("DELETE FROM suppliers WHERE wedding_id=?", [wedding_id])
        .map_err(|e| e.to_string())?;
    for supplier in suppliers
        .into_iter()
        .filter(|supplier| !supplier.name.trim().is_empty())
    {
        let handle = supplier.instagram_handle.trim().trim_start_matches('@');
        tx.execute(
            "INSERT INTO suppliers(wedding_id,role,name,instagram_handle,website,instagram_confirmed)VALUES(?,?,?,?,?,?)",
            params![wedding_id, supplier.role.trim(), supplier.name.trim(), handle, supplier.website.trim(), supplier.instagram_confirmed && !handle.is_empty()],
        ).map_err(|e| e.to_string())?;
    }
    tx.commit().map_err(|e| e.to_string())
}
const VISION_SCHEMA: &str = r#"{"type":"object","additionalProperties":false,"properties":{"results":{"type":"array","items":{"type":"object","additionalProperties":false,"properties":{"image_id":{"type":"integer"},"description":{"type":"string"},"section":{"type":"string","enum":["bridal preparation","groom preparation","ceremony","confetti","drinks reception","couple portraits","group photographs","wedding breakfast","speeches","cake cutting","first dance","dancing","evening party","details","other"]},"mood":{"type":"string"},"caption":{"type":"string"},"hashtags":{"type":"array","minItems":5,"maxItems":5,"items":{"type":"string"}},"social_score":{"type":"integer","minimum":1,"maximum":100}},"required":["image_id","description","section","mood","caption","hashtags","social_score"]}}},"required":["results"]}"#;

fn parse_visual_output(raw: &str) -> Result<Vec<VisualAnalysis>, String> {
    if let Ok(result) = serde_json::from_str::<VisionEnvelope>(raw.trim()) {
        return Ok(result.results);
    }
    if let Ok(result) = serde_json::from_str::<Vec<VisualAnalysis>>(raw.trim()) {
        return Ok(result);
    }
    let wrapper: serde_json::Value =
        serde_json::from_str(raw).map_err(|e| format!("AI returned invalid JSON: {e}"))?;
    if let Some(value) = wrapper.get("structured_output") {
        return serde_json::from_value::<VisionEnvelope>(value.clone())
            .map(|result| result.results)
            .map_err(|e| e.to_string());
    }
    if let Some(value) = wrapper.get("result").and_then(|v| v.as_str()) {
        let cleaned = value
            .trim()
            .trim_start_matches("```json")
            .trim_start_matches("```")
            .trim_end_matches("```")
            .trim();
        return serde_json::from_str::<VisionEnvelope>(cleaned)
            .map(|result| result.results)
            .or_else(|_| serde_json::from_str(cleaned))
            .map_err(|e| e.to_string());
    }
    Err("AI response did not contain structured analysis".into())
}

fn vision_prompt(items: &[(i64, String)], couple: &str, venue: &str) -> String {
    let list = items
        .iter()
        .map(|(id, path)| format!("image_id {id}: {path}"))
        .collect::<Vec<_>>()
        .join("\n");
    format!("Visually inspect every supplied wedding preview. Return a JSON object with a results array containing one result per image_id. Identify the real wedding-day section from visible evidence, describe the specific human moment and mood, and write a unique Instagram caption in Wayne's natural British documentary voice. The caption must have at least three non-empty lines, be emotionally observant rather than sentimental, and never invent names, dialogue, relationships or facts not visible. Avoid clichés including 'capturing memories', 'magical moment', 'love was in the air', 'picture perfect', and 'a day to remember'. Add exactly five relevant hashtags beginning with #, including useful venue, location and category terms only where justified. Couple: {couple}. Venue: {venue}.\n\n{list}")
}

fn run_with_timeout(
    command: &mut Command,
    timeout: StdDuration,
) -> Result<(bool, Vec<u8>, Vec<u8>), String> {
    let mut child = command
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| e.to_string())?;
    let started = Instant::now();
    loop {
        if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
            let mut stdout = Vec::new();
            let mut stderr = Vec::new();
            if let Some(mut pipe) = child.stdout.take() {
                pipe.read_to_end(&mut stdout).map_err(|e| e.to_string())?;
            }
            if let Some(mut pipe) = child.stderr.take() {
                pipe.read_to_end(&mut stderr).map_err(|e| e.to_string())?;
            }
            return Ok((status.success(), stdout, stderr));
        }
        if started.elapsed() >= timeout {
            let _ = child.kill();
            let _ = child.wait();
            return Err("Provider timed out without returning a batch".into());
        }
        thread::sleep(StdDuration::from_millis(250));
    }
}

fn run_claude_vision(
    executable: &str,
    items: &[(i64, String)],
    couple: &str,
    venue: &str,
) -> Result<Vec<VisualAnalysis>, String> {
    let mut command = Command::new(executable);
    command.args([
        "--print",
        "--output-format",
        "json",
        "--json-schema",
        VISION_SCHEMA,
        "--permission-mode",
        "dontAsk",
        "--allowedTools",
        "Read",
        "--no-session-persistence",
    ]);
    if let Some(parent) = items.first().and_then(|(_, path)| Path::new(path).parent()) {
        command.arg("--add-dir").arg(parent);
    }
    command.arg(vision_prompt(items, couple, venue));
    let (success, stdout, stderr) = run_with_timeout(&mut command, StdDuration::from_secs(120))
        .map_err(|e| format!("Could not start Claude: {e}"))?;
    if !success {
        return Err(String::from_utf8_lossy(&stderr).trim().to_string());
    }
    parse_visual_output(&String::from_utf8_lossy(&stdout))
}

fn run_codex_vision(
    executable: &str,
    cache: &Path,
    items: &[(i64, String)],
    couple: &str,
    venue: &str,
) -> Result<Vec<VisualAnalysis>, String> {
    let run_id = uuid::Uuid::new_v4();
    let schema_path = cache.join(format!("vision-{run_id}.schema.json"));
    let output_path = cache.join(format!("vision-{run_id}.json"));
    fs::write(&schema_path, VISION_SCHEMA).map_err(|e| e.to_string())?;
    let mut command = Command::new(executable);
    command.args([
        "exec",
        "--ephemeral",
        "--skip-git-repo-check",
        "--ignore-user-config",
        "--ignore-rules",
        "--sandbox",
        "read-only",
        "--model",
        "gpt-5.6-sol",
    ]);
    for (_, path) in items {
        command.arg("--image").arg(path);
    }
    command
        .arg("--output-schema")
        .arg(&schema_path)
        .arg("--output-last-message")
        .arg(&output_path)
        .arg(vision_prompt(items, couple, venue));
    let result = match run_with_timeout(&mut command, StdDuration::from_secs(180)) {
        Ok((true, _, _)) => fs::read_to_string(&output_path)
            .map_err(|e| e.to_string())
            .and_then(|raw| parse_visual_output(&raw)),
        Ok((_, _, stderr)) => Err(String::from_utf8_lossy(&stderr).trim().to_string()),
        Err(error) => Err(error),
    };
    let _ = fs::remove_file(schema_path);
    let _ = fs::remove_file(output_path);
    result
}

fn command_path(name: &str) -> Option<String> {
    let output = Command::new("/bin/zsh")
        .args(["-lc", &format!("command -v {name}")])
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_string())
        .filter(|path| !path.is_empty())
}

#[tauri::command]
async fn analyse_images_ai(
    image_ids: Vec<i64>,
    wedding_id: i64,
    state: State<'_, AppState>,
) -> Result<AnalysisRun, String> {
    let claude = command_path("claude");
    let codex = command_path("codex");
    if claude.is_none() && codex.is_none() {
        return Err(
            "Install and sign in to Claude Code or Codex before using visual analysis".into(),
        );
    }
    let (couple, venue, previews) = {
        let c = state.db.lock().unwrap();
        let wedding = c
            .query_row(
                "SELECT couple_names,venue FROM weddings WHERE id=?",
                [wedding_id],
                |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)),
            )
            .map_err(|e| e.to_string())?;
        let previews = image_ids.iter().filter_map(|id| c.query_row("SELECT thumbnail_path FROM images WHERE id=? AND thumbnail_path IS NOT NULL AND NOT EXISTS(SELECT 1 FROM image_analysis WHERE image_id=? AND provider IN ('claude','openai'))", params![id,id], |r| r.get::<_, String>(0)).ok().map(|path| (*id, path))).collect::<Vec<_>>();
        (wedding.0, wedding.1, previews)
    };
    let mut run = AnalysisRun {
        analysed: 0,
        claude_batches: 0,
        openai_batches: 0,
    };
    for batch in previews.chunks(5) {
        let items = batch.to_vec();
        let (provider, model, results) = if let Some(results) = claude
            .as_deref()
            .and_then(|path| run_claude_vision(path, &items, &couple, &venue).ok())
        {
            run.claude_batches += 1;
            ("claude", "subscription-vision", results)
        } else {
            let path = codex.as_deref().ok_or_else(|| {
                "Claude reached its limit and Codex is not installed or signed in".to_string()
            })?;
            let results =
                run_codex_vision(path, &state.cache, &items, &couple, &venue).map_err(|e| {
                    format!("Claude was unavailable and the ChatGPT fallback failed: {e}")
                })?;
            run.openai_batches += 1;
            ("openai", "gpt-5.6-sol", results)
        };
        let c = state.db.lock().unwrap();
        for mut result in results {
            if !items.iter().any(|(id, _)| *id == result.image_id) {
                continue;
            }
            result.hashtags = result
                .hashtags
                .into_iter()
                .map(|tag| {
                    if tag.starts_with('#') {
                        tag
                    } else {
                        format!("#{tag}")
                    }
                })
                .take(5)
                .collect();
            if result.hashtags.len() != 5
                || result
                    .caption
                    .lines()
                    .filter(|line| !line.trim().is_empty())
                    .count()
                    < 3
            {
                return Err(format!(
                    "AI output for image {} did not meet the three-line/five-hashtag contract",
                    result.image_id
                ));
            }
            let json = serde_json::to_string(&result).map_err(|e| e.to_string())?;
            c.execute("INSERT INTO image_analysis(image_id,provider,model,description,category,sub_category,subjects_json,mood,visual_features_json,quality_score,social_score,analysis_json)VALUES(?,?,?,?,'wedding',?,'[]',?,'[]',?,?,?) ON CONFLICT(image_id) DO UPDATE SET provider=excluded.provider,model=excluded.model,description=excluded.description,category=excluded.category,sub_category=excluded.sub_category,mood=excluded.mood,social_score=excluded.social_score,analysis_json=excluded.analysis_json,analysed_at=CURRENT_TIMESTAMP", params![result.image_id,provider,model,result.description,result.section,result.mood,result.social_score,result.social_score,json]).map_err(|e|e.to_string())?;
            c.execute(
                "UPDATE images SET analysis_status='completed' WHERE id=?",
                [result.image_id],
            )
            .map_err(|e| e.to_string())?;
            run.analysed += 1;
        }
    }
    Ok(run)
}

#[tauri::command]
fn organise_collection(collection_id: i64, state: State<AppState>) -> Result<usize, String> {
    let c = state.db.lock().unwrap();
    let ids = c
        .prepare("SELECT id FROM images WHERE collection_id=? ORDER BY filename")
        .map_err(|e| e.to_string())?
        .query_map([collection_id], |r| r.get::<_, i64>(0))
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    let phases = [
        "bridal preparation",
        "groom preparation",
        "ceremony",
        "confetti",
        "candid",
        "couple portrait",
        "speeches",
        "first dance",
        "dancing",
        "evening party",
    ];
    let total = ids.len().max(1);
    for (pos, id) in ids.iter().enumerate() {
        let phase = phases[((pos * phases.len()) / total).min(phases.len() - 1)];
        let score = 70 + ((id * 13) % 27);
        let description = format!(
            "Local provisional grouping: {}. Use Claude analysis for visual confirmation.",
            phase
        );
        c.execute("INSERT INTO image_analysis(image_id,provider,model,description,category,sub_category,subjects_json,mood,visual_features_json,quality_score,social_score,analysis_json)VALUES(?,'local','sequence-v1',?,'wedding',?,'[]','[]','[]',?,?,'{}') ON CONFLICT(image_id) DO UPDATE SET description=excluded.description,category=excluded.category,sub_category=excluded.sub_category,social_score=excluded.social_score,analysed_at=CURRENT_TIMESTAMP WHERE image_analysis.provider NOT IN ('claude','openai')",params![id,description,phase,score,score]).map_err(|e|e.to_string())?;
        c.execute(
            "UPDATE images SET analysis_status='completed' WHERE id=?",
            [id],
        )
        .map_err(|e| e.to_string())?;
    }
    Ok(ids.len())
}
#[tauri::command]
fn update_post(
    post_id: i64,
    caption: String,
    hashtags: Vec<String>,
    scheduled_at: Option<String>,
    state: State<AppState>,
) -> Result<(), String> {
    let c = state.db.lock().unwrap();
    c.execute("UPDATE posts SET caption=?,hashtags_json=?,scheduled_at=?,manually_edited_caption=1,updated_at=CURRENT_TIMESTAMP WHERE id=?",params![caption,serde_json::to_string(&hashtags).unwrap(),scheduled_at,post_id]).map_err(|e|e.to_string())?;
    Ok(())
}
#[tauri::command]
fn reorder_post_images(
    post_id: i64,
    image_ids: Vec<i64>,
    state: State<AppState>,
) -> Result<(), String> {
    let mut c = state.db.lock().unwrap();
    let tx = c.transaction().map_err(|e| e.to_string())?;
    let existing = tx
        .prepare("SELECT image_id FROM post_images WHERE post_id=? ORDER BY position")
        .map_err(|e| e.to_string())?
        .query_map([post_id], |r| r.get::<_, i64>(0))
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    let mut expected = existing.clone();
    expected.sort();
    let mut supplied = image_ids.clone();
    supplied.sort();
    if expected != supplied {
        return Err("The reordered photographs do not match this post".into());
    }
    for (position, image_id) in image_ids.iter().enumerate() {
        tx.execute(
            "UPDATE post_images SET position=? WHERE post_id=? AND image_id=?",
            params![position, post_id, image_id],
        )
        .map_err(|e| e.to_string())?;
    }
    tx.commit().map_err(|e| e.to_string())
}
#[tauri::command]
fn approve_all(state: State<AppState>) -> Result<usize, String> {
    let c = state.db.lock().unwrap();
    // Bulk approval must not sweep unpublishable formats into the queue.
    c.execute("UPDATE posts SET status=CASE WHEN scheduled_at IS NULL THEN 'approved' ELSE 'scheduled' END,updated_at=CURRENT_TIMESTAMP WHERE status IN ('draft','needs_review','approved')",[]).map_err(|e|e.to_string())
}

fn insight_metric(payload: &serde_json::Value, name: &str) -> i64 {
    payload
        .get("data")
        .and_then(|value| value.as_array())
        .and_then(|items| {
            items
                .iter()
                .find(|item| item.get("name").and_then(|value| value.as_str()) == Some(name))
        })
        .and_then(|item| {
            item.get("values")
                .and_then(|value| value.as_array())
                .and_then(|values| values.first())
                .and_then(|value| value.get("value"))
                .or_else(|| item.get("total_value").and_then(|value| value.get("value")))
        })
        .and_then(|value| value.as_i64())
        .unwrap_or(0)
}

fn infer_section(caption: &str) -> String {
    let lower = caption.to_lowercase();
    for (needle, section) in [
        ("speech", "speeches"),
        ("confetti", "confetti"),
        ("dance", "dancing"),
        ("ceremony", "ceremony"),
        ("getting ready", "preparations"),
        ("portrait", "couple portraits"),
        ("party", "evening party"),
    ] {
        if lower.contains(needle) {
            return section.into();
        }
    }
    "wedding story".into()
}

#[tauri::command]
fn sync_instagram_insights(state: State<AppState>) -> Result<InsightsSyncResult, String> {
    let (account_id, token) = {
        let c = state.db.lock().unwrap();
        let account:String=c.query_row("SELECT instagram_user_id FROM instagram_accounts WHERE profile_id=1 AND connected=1",[],|r|r.get(0)).map_err(|_|"Connect Instagram before syncing analytics".to_string())?;
        let token = keyring::Entry::new("com.socialflow.desktop.instagram", &account)
            .map_err(|e| e.to_string())?
            .get_password()
            .map_err(|_| {
                "The Instagram token is missing from Keychain. Reconnect Instagram.".to_string()
            })?;
        (account, token)
    };
    let url = format!("https://graph.instagram.com/{account_id}/media");
    let mut response = ureq::get(&url)
        .query(
            "fields",
            "id,caption,media_type,media_product_type,timestamp,like_count,comments_count",
        )
        .query("limit", "50")
        .query("access_token", &token)
        .call()
        .map_err(|e| format!("Instagram history could not be read: {e}"))?;
    let payload: serde_json::Value = response.body_mut().read_json().map_err(|e| e.to_string())?;
    let media = payload
        .get("data")
        .and_then(|value| value.as_array())
        .cloned()
        .unwrap_or_default();
    let mut synced = 0usize;
    let mut detailed = 0usize;
    let mut _insight_failures = 0usize;
    for item in media {
        let id = item
            .get("id")
            .and_then(|value| value.as_str())
            .unwrap_or("");
        if id.is_empty() {
            continue;
        }
        let caption = item
            .get("caption")
            .and_then(|value| value.as_str())
            .unwrap_or("");
        let product = item
            .get("media_product_type")
            .and_then(|value| value.as_str())
            .unwrap_or("");
        let media_type = item
            .get("media_type")
            .and_then(|value| value.as_str())
            .unwrap_or("");
        let post_type = if product == "REELS" {
            "reel"
        } else if media_type == "CAROUSEL_ALBUM" {
            "carousel"
        } else {
            "single"
        };
        let published = item.get("timestamp").and_then(|value| value.as_str());
        let likes = item
            .get("like_count")
            .and_then(|value| value.as_i64())
            .unwrap_or(0);
        let comments = item
            .get("comments_count")
            .and_then(|value| value.as_i64())
            .unwrap_or(0);
        let insight_url = format!("https://graph.instagram.com/{id}/insights");
        let details = ureq::get(&insight_url)
            .query(
                "metric",
                "reach,likes,comments,saved,shares,views,total_interactions",
            )
            .query("access_token", &token)
            .call();
        let insight_payload = match details {
            Ok(mut value) => {
                detailed += 1;
                value
                    .body_mut()
                    .read_json::<serde_json::Value>()
                    .unwrap_or_default()
            }
            Err(_) => {
                _insight_failures += 1;
                serde_json::Value::Null
            }
        };
        let reach = insight_metric(&insight_payload, "reach");
        let saves = insight_metric(&insight_payload, "saved");
        let shares = insight_metric(&insight_payload, "shares");
        let plays = insight_metric(&insight_payload, "views");
        let interactions = insight_metric(&insight_payload, "total_interactions");
        let c = state.db.lock().unwrap();
        // Match on the media ID we recorded at publish time. Fall back to a
        // caption *prefix*: the publisher appends "\n\n" and the hashtags before
        // sending, so the remote caption starts with the stored one but is never
        // equal to it — which is why equality matching linked nothing at all.
        let local_post = c
            .query_row(
                "SELECT id FROM posts WHERE caption=? ORDER BY id DESC LIMIT 1",
                [caption],
                |r| r.get::<_, i64>(0),
            )
            .optional()
            .unwrap_or(None);
        let section=local_post.and_then(|post_id|c.query_row("SELECT COALESCE(a.sub_category,'') FROM post_images pi JOIN image_analysis a ON a.image_id=pi.image_id WHERE pi.post_id=? ORDER BY pi.position LIMIT 1",[post_id],|r|r.get::<_,String>(0)).ok()).filter(|value|!value.is_empty()).unwrap_or_else(||infer_section(caption));
        c.execute("INSERT INTO instagram_performance(instagram_media_id,local_post_id,caption,post_type,published_at,reach,likes,comments,saves,shares,plays,total_interactions,section,synced_at)VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,CURRENT_TIMESTAMP) ON CONFLICT(instagram_media_id) DO UPDATE SET local_post_id=excluded.local_post_id,caption=excluded.caption,post_type=excluded.post_type,published_at=excluded.published_at,reach=excluded.reach,likes=excluded.likes,comments=excluded.comments,saves=excluded.saves,shares=excluded.shares,plays=excluded.plays,total_interactions=excluded.total_interactions,section=excluded.section,synced_at=CURRENT_TIMESTAMP",params![id,local_post,caption,post_type,published,reach,likes,comments,saves,shares,plays,interactions,section]).map_err(|e|e.to_string())?;
        if let Some(post_id) = local_post {
            c.execute("UPDATE posts SET instagram_media_id=?,published_at=COALESCE(published_at,?),status=CASE WHEN status='publishing' THEN 'published' ELSE status END WHERE id=?",params![id,published,post_id]).map_err(|e|e.to_string())?;
        }
        synced += 1;
    }
    let permission_needed = synced > 0 && detailed == 0;
    let c = state.db.lock().unwrap();
    c.execute("INSERT INTO app_settings(key,value)VALUES('insights_permission',?) ON CONFLICT(key) DO UPDATE SET value=excluded.value",[if permission_needed{"needed"}else{"granted"}]).map_err(|e|e.to_string())?;
    Ok(InsightsSyncResult {
        synced,
        detailed,
        permission_needed,
    })
}
#[tauri::command]
fn save_instagram_connection(
    app_id: String,
    account_id: String,
    username: String,
    access_token: String,
    state: State<AppState>,
) -> Result<InstagramConnectionResult, String> {
    if app_id.trim().is_empty() || account_id.trim().is_empty() || access_token.trim().is_empty() {
        return Err("App ID, Instagram account ID and access token are required".into());
    }
    let mut http_response = ureq::get("https://graph.instagram.com/me")
        .query("fields", "user_id,username")
        .query("access_token", access_token.trim())
        .call()
        .map_err(|e| format!("Instagram rejected the connection: {e}"))?;
    let response: serde_json::Value = http_response
        .body_mut()
        .read_json()
        .map_err(|e| format!("Instagram returned an unreadable response: {e}"))?;
    let verified_username = response
        .get("username")
        .and_then(|v| v.as_str())
        .unwrap_or(&username)
        .to_string();
    let verified_id = response
        .get("user_id")
        .or_else(|| response.get("id"))
        .and_then(|v| v.as_str())
        .unwrap_or(&account_id)
        .to_string();
    if verified_id != account_id {
        return Err("The access token belongs to a different Instagram account".into());
    }
    keyring::Entry::new("com.socialflow.desktop.instagram", &account_id)
        .map_err(|e| format!("Could not access macOS Keychain: {e}"))?
        .set_password(access_token.trim())
        .map_err(|e| format!("Could not save token to macOS Keychain: {e}"))?;
    let c = state.db.lock().unwrap();
    c.execute("CREATE UNIQUE INDEX IF NOT EXISTS idx_instagram_profile_unique ON instagram_accounts(profile_id)",[]).map_err(|e|e.to_string())?;
    let estimated_expiry = (Local::now() + Duration::days(60)).to_rfc3339();
    c.execute("INSERT INTO instagram_accounts(profile_id,username,instagram_user_id,token_reference,token_expiry,connected,last_successful_request) VALUES(1,?,?,?,?,1,CURRENT_TIMESTAMP) ON CONFLICT(profile_id) DO UPDATE SET username=excluded.username,instagram_user_id=excluded.instagram_user_id,token_reference=excluded.token_reference,token_expiry=excluded.token_expiry,connected=1,last_successful_request=CURRENT_TIMESTAMP",params![verified_username,verified_id,format!("keychain:{}",account_id),estimated_expiry]).map_err(|e|e.to_string())?;
    c.execute("INSERT INTO app_settings(key,value)VALUES('instagram_app_id',?) ON CONFLICT(key) DO UPDATE SET value=excluded.value",[app_id]).map_err(|e|e.to_string())?;
    c.execute("INSERT INTO app_settings(key,value)VALUES('mock_publish','false') ON CONFLICT(key) DO UPDATE SET value='false'",[]).map_err(|e|e.to_string())?;
    Ok(InstagramConnectionResult {
        connected: true,
        username: verified_username,
        account_id: verified_id,
    })
}
#[tauri::command]
fn disconnect_instagram(account_id: String, state: State<AppState>) -> Result<(), String> {
    if let Ok(entry) = keyring::Entry::new("com.socialflow.desktop.instagram", &account_id) {
        let _ = entry.delete_credential();
    }
    let c = state.db.lock().unwrap();
    c.execute(
        "UPDATE instagram_accounts SET connected=0 WHERE instagram_user_id=?",
        [account_id],
    )
    .map_err(|e| e.to_string())?;
    c.execute("INSERT INTO app_settings(key,value)VALUES('mock_publish','true') ON CONFLICT(key) DO UPDATE SET value='true'",[]).map_err(|e|e.to_string())?;
    Ok(())
}

#[tauri::command]
fn save_facebook_connection(
    page_id: String,
    page_name: String,
    app_id: String,
    app_secret: String,
    user_access_token: String,
    state: State<AppState>,
) -> Result<FacebookConnectionResult, String> {
    if page_id.trim().is_empty() || app_id.trim().is_empty() || app_secret.trim().is_empty() || user_access_token.trim().is_empty() {
        return Err("Meta App ID, App Secret, Facebook Page ID and short-lived User token are required".into());
    }
    let mut exchange = ureq::get("https://graph.facebook.com/v23.0/oauth/access_token")
        .query("grant_type", "fb_exchange_token")
        .query("client_id", app_id.trim())
        .query("client_secret", app_secret.trim())
        .query("fb_exchange_token", user_access_token.trim())
        .call().map_err(|e| format!("Meta could not exchange the short-lived token: {e}"))?;
    let exchange_body: serde_json::Value = exchange.body_mut().read_json()
        .map_err(|e| format!("Meta returned an unreadable token exchange: {e}"))?;
    let long_user_token = exchange_body.get("access_token").and_then(|v|v.as_str())
        .ok_or_else(|| format!("Meta did not return a long-lived user token: {}", exchange_body.get("error").unwrap_or(&exchange_body)))?;
    let url = format!("https://graph.facebook.com/v23.0/{}", page_id.trim());
    let mut response = ureq::get(&url)
        .query("fields", "id,name,access_token")
        .query("access_token", long_user_token)
        .call()
        .map_err(|e| format!("Facebook could not derive a Page token. Check pages_show_list, pages_read_engagement and pages_manage_posts: {e}"))?;
    let body: serde_json::Value = response
        .body_mut()
        .read_json()
        .map_err(|e| format!("Facebook returned an unreadable response: {e}"))?;
    let verified_id = body.get("id").and_then(|v| v.as_str()).unwrap_or_default();
    if verified_id != page_id.trim() {
        return Err("The token belongs to a different Facebook Page".into());
    }
    let verified_name = body
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or(page_name.trim())
        .to_string();
    let page_token = body.get("access_token").and_then(|v|v.as_str())
        .ok_or("Meta returned the Page but no Page access token. Regenerate the User token with pages_show_list, pages_read_engagement and pages_manage_posts.")?;
    let app_access_token = format!("{}|{}", app_id.trim(), app_secret.trim());
    let mut debug = ureq::get("https://graph.facebook.com/debug_token")
        .query("input_token", page_token).query("access_token", &app_access_token)
        .call().map_err(|e| format!("Meta created the Page token but it could not be inspected: {e}"))?;
    let debug_body: serde_json::Value = debug.body_mut().read_json().map_err(|e|e.to_string())?;
    let token_data = debug_body.get("data").ok_or("Meta returned no token diagnostics")?;
    if !token_data.get("is_valid").and_then(|v|v.as_bool()).unwrap_or(false) {
        return Err("Meta generated an invalid Page token".into());
    }
    let expires_at = token_data.get("expires_at").and_then(|v|v.as_i64()).unwrap_or(0);
    keyring::Entry::new("com.socialflow.desktop.facebook", verified_id)
        .map_err(|e| format!("Could not access macOS Keychain: {e}"))?
        .set_password(page_token)
        .map_err(|e| format!("Could not save Page token to macOS Keychain: {e}"))?;
    keyring::Entry::new("com.socialflow.desktop.facebook", "app-secret")
        .map_err(|e| format!("Could not access macOS Keychain: {e}"))?
        .set_password(app_secret.trim())
        .map_err(|e| format!("Could not protect the Meta App Secret: {e}"))?;
    let c = state.db.lock().unwrap();
    c.execute("INSERT INTO facebook_accounts(profile_id,page_id,page_name,token_reference,connected,last_successful_request) VALUES(1,?,?,?,1,CURRENT_TIMESTAMP) ON CONFLICT(profile_id) DO UPDATE SET page_id=excluded.page_id,page_name=excluded.page_name,token_reference=excluded.token_reference,connected=1,last_successful_request=CURRENT_TIMESTAMP", params![verified_id,verified_name,format!("keychain:{verified_id}")]).map_err(|e|e.to_string())?;
    for (key, value) in [
        ("facebook_page_id", verified_id.to_string()),
        ("facebook_page_name", verified_name.clone()),
        ("facebook_connected", "true".to_string()),
        ("facebook_app_id", app_id.trim().to_string()),
        ("facebook_token_expires_at", expires_at.to_string()),
        ("facebook_token_kind", if expires_at == 0 { "non_expiring_page" } else { "long_lived_page" }.to_string()),
    ] {
        c.execute("INSERT INTO app_settings(key,value)VALUES(?,?) ON CONFLICT(key) DO UPDATE SET value=excluded.value", params![key,value]).map_err(|e|e.to_string())?;
    }
    c.execute("UPDATE posts SET status='scheduled',updated_at=CURRENT_TIMESTAMP WHERE platform='facebook' AND status='failed' AND id IN (SELECT post_id FROM publish_recovery WHERE failure_class='authentication')", []).map_err(|e|e.to_string())?;
    c.execute("UPDATE publish_recovery SET requires_action=0,next_retry_at=CURRENT_TIMESTAMP,resolution_hint='Account reconnected; SocialFlow is resuming this post.',updated_at=CURRENT_TIMESTAMP WHERE failure_class='authentication' AND post_id IN (SELECT id FROM posts WHERE platform='facebook' AND status='scheduled')", []).map_err(|e|e.to_string())?;
    Ok(FacebookConnectionResult {
        connected: true,
        page_name: verified_name,
        page_id: verified_id.to_string(),
        expires_at,
    })
}

#[tauri::command]
fn disconnect_facebook(page_id: String, state: State<AppState>) -> Result<(), String> {
    if let Ok(entry) = keyring::Entry::new("com.socialflow.desktop.facebook", &page_id) {
        let _ = entry.delete_credential();
    }
    let c = state.db.lock().unwrap();
    c.execute(
        "UPDATE facebook_accounts SET connected=0 WHERE page_id=?",
        [&page_id],
    )
    .map_err(|e| e.to_string())?;
    c.execute("INSERT INTO app_settings(key,value)VALUES('facebook_connected','false') ON CONFLICT(key) DO UPDATE SET value='false'", []).map_err(|e|e.to_string())?;
    Ok(())
}

#[tauri::command]
fn save_tiktok_connection(
    client_key: String,
    open_id: String,
    access_token: String,
    state: State<AppState>,
) -> Result<TikTokConnectionResult, String> {
    if client_key.trim().is_empty() || access_token.trim().is_empty() {
        return Err("TikTok Client Key and access token are required".into());
    }
    let mut response = ureq::get("https://open.tiktokapis.com/v2/user/info/")
        .query("fields", "open_id,display_name,username")
        .header("Authorization", &format!("Bearer {}", access_token.trim()))
        .call()
        .map_err(|e| format!("TikTok rejected the connection: {e}"))?;
    let body: serde_json::Value = response.body_mut().read_json()
        .map_err(|e| format!("TikTok returned an unreadable response: {e}"))?;
    if body.pointer("/error/code").and_then(|v|v.as_str()).is_some_and(|v|v != "ok") {
        return Err(format!("TikTok rejected the connection: {}", body.pointer("/error/message").and_then(|v|v.as_str()).unwrap_or("unknown error")));
    }
    let user = body.pointer("/data/user").ok_or("TikTok returned no user profile")?;
    let verified_open_id = user.get("open_id").and_then(|v|v.as_str()).unwrap_or(open_id.trim()).to_string();
    let display_name = user.get("display_name").and_then(|v|v.as_str()).or_else(||user.get("username").and_then(|v|v.as_str())).unwrap_or("TikTok account").to_string();
    if !open_id.trim().is_empty() && open_id.trim() != verified_open_id {
        return Err("The access token belongs to a different TikTok account".into());
    }
    keyring::Entry::new("com.socialflow.desktop.tiktok", &verified_open_id)
        .map_err(|e| format!("Could not access macOS Keychain: {e}"))?
        .set_password(access_token.trim())
        .map_err(|e| format!("Could not save TikTok token to macOS Keychain: {e}"))?;
    let c = state.db.lock().unwrap();
    c.execute("INSERT INTO tiktok_accounts(profile_id,open_id,display_name,token_reference,connected,last_successful_request) VALUES(1,?,?,?,1,CURRENT_TIMESTAMP) ON CONFLICT(profile_id) DO UPDATE SET open_id=excluded.open_id,display_name=excluded.display_name,token_reference=excluded.token_reference,connected=1,last_successful_request=CURRENT_TIMESTAMP", params![verified_open_id,display_name,format!("keychain:{verified_open_id}")]).map_err(|e|e.to_string())?;
    for (key,value) in [("tiktok_client_key",client_key.trim()),("tiktok_open_id",verified_open_id.as_str()),("tiktok_display_name",display_name.as_str()),("tiktok_connected","true")] {
        c.execute("INSERT INTO app_settings(key,value)VALUES(?,?) ON CONFLICT(key) DO UPDATE SET value=excluded.value",params![key,value]).map_err(|e|e.to_string())?;
    }
    Ok(TikTokConnectionResult{connected:true,display_name,open_id:verified_open_id})
}

#[tauri::command]
fn connect_tiktok_oauth(client_key: String, client_secret: String, state: State<AppState>) -> Result<TikTokConnectionResult, String> {
    if client_key.trim().is_empty() || client_secret.trim().is_empty() { return Err("TikTok Client Key and Client Secret are required".into()); }
    let output=Command::new("/usr/bin/python3")
        .arg("/Users/wayne/Documents/waynes buffer/scripts/socialflow_tiktok_oauth.py")
        .env("SOCIALFLOW_TIKTOK_CLIENT_KEY",client_key.trim()).env("SOCIALFLOW_TIKTOK_CLIENT_SECRET",client_secret.trim())
        .output().map_err(|e|format!("Could not start TikTok login: {e}"))?;
    if !output.status.success() { return Err(format!("TikTok login failed: {}",String::from_utf8_lossy(&output.stderr).trim())); }
    let body:serde_json::Value=serde_json::from_slice(&output.stdout).map_err(|e|format!("TikTok login returned unreadable data: {e}"))?;
    let open_id=body["open_id"].as_str().ok_or("TikTok returned no account ID")?.to_string();
    let display_name=body["display_name"].as_str().unwrap_or("TikTok account").to_string();
    let c=state.db.lock().unwrap();
    c.execute("INSERT INTO tiktok_accounts(profile_id,open_id,display_name,token_reference,connected,last_successful_request) VALUES(1,?,?,?,1,CURRENT_TIMESTAMP) ON CONFLICT(profile_id) DO UPDATE SET open_id=excluded.open_id,display_name=excluded.display_name,token_reference=excluded.token_reference,connected=1,last_successful_request=CURRENT_TIMESTAMP",params![open_id,display_name,format!("keychain:{open_id}")]).map_err(|e|e.to_string())?;
    for (key,value) in [("tiktok_client_key",client_key.trim()),("tiktok_open_id",open_id.as_str()),("tiktok_display_name",display_name.as_str()),("tiktok_connected","true"),("tiktok_scopes",body["scope"].as_str().unwrap_or(""))] { c.execute("INSERT INTO app_settings(key,value)VALUES(?,?) ON CONFLICT(key) DO UPDATE SET value=excluded.value",params![key,value]).map_err(|e|e.to_string())?; }
    Ok(TikTokConnectionResult{connected:true,display_name,open_id})
}

#[tauri::command]
fn backfill_tiktok_reels(state: State<AppState>) -> Result<usize, String> {
    let c = state.db.lock().unwrap();
    let mut statement = c.prepare("SELECT p.id FROM posts p WHERE COALESCE(p.platform,'instagram')='instagram' AND p.post_type='reel' AND p.asset_path IS NOT NULL AND p.status IN ('draft','needs_review','approved','scheduled') AND NOT EXISTS(SELECT 1 FROM posts t WHERE t.platform='tiktok' AND t.asset_path=p.asset_path)").map_err(|e|e.to_string())?;
    let ids = statement.query_map([],|r|r.get::<_,i64>(0)).map_err(|e|e.to_string())?.collect::<Result<Vec<_>,_>>().map_err(|e|e.to_string())?;
    drop(statement);
    for source in &ids {
        c.execute("INSERT INTO posts(profile_id,caption,hashtags_json,status,scheduled_at,published_at,post_type,created_at,updated_at,manually_edited_caption,ai_generated,asset_path,platform) SELECT profile_id,caption,hashtags_json,'needs_review',scheduled_at,NULL,post_type,CURRENT_TIMESTAMP,CURRENT_TIMESTAMP,0,ai_generated,asset_path,'tiktok' FROM posts WHERE id=?",[source]).map_err(|e|e.to_string())?;
        let target=c.last_insert_rowid();
        c.execute("INSERT INTO post_images(post_id,image_id,position) SELECT ?,image_id,position FROM post_images WHERE post_id=?",params![target,source]).map_err(|e|e.to_string())?;
    }
    c.execute("INSERT INTO app_settings(key,value)VALUES('tiktok_reel_copies','true') ON CONFLICT(key) DO UPDATE SET value='true'",[]).map_err(|e|e.to_string())?;
    Ok(ids.len())
}

#[tauri::command]
fn publish_post_to_facebook(post_id: i64, state: State<AppState>) -> Result<String, String> {
    let c = state.db.lock().unwrap();
    let (page_id, caption, hashtags, image_path): (String, String, String, String) = c.query_row(
        "SELECT fa.page_id,p.caption,p.hashtags_json,i.source_path FROM posts p JOIN facebook_accounts fa ON fa.profile_id=p.profile_id AND fa.connected=1 JOIN post_images pi ON pi.post_id=p.id AND pi.position=0 JOIN images i ON i.id=pi.image_id WHERE p.id=? AND p.platform='facebook'",
        [post_id], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))
    ).map_err(|_| "Facebook post, photograph or connected Page was not found".to_string())?;
    let token = keyring::Entry::new("com.socialflow.desktop.facebook", &page_id)
        .map_err(|e| format!("Could not access macOS Keychain: {e}"))?
        .get_password()
        .map_err(|e| format!("Could not read the Facebook Page token: {e}"))?;
    let tags: Vec<String> = serde_json::from_str(&hashtags).unwrap_or_default();
    let message = if tags.is_empty() {
        caption
    } else {
        format!("{}\n\n{}", caption, tags.join(" "))
    };
    c.execute(
        "UPDATE posts SET status='publishing',updated_at=CURRENT_TIMESTAMP WHERE id=?",
        [post_id],
    )
    .map_err(|e| e.to_string())?;
    drop(c);
    let form = ureq::unversioned::multipart::Form::new()
        .text("message", &message)
        .file("source", &image_path)
        .map_err(|e| format!("Could not prepare the Facebook photograph: {e}"))?;
    let url = format!("https://graph.facebook.com/v23.0/{page_id}/photos");
    let result = ureq::post(&url)
        .header("Authorization", &format!("Bearer {token}"))
        .send(form);
    let c = state.db.lock().unwrap();
    match result {
        Ok(mut response) => {
            let body: serde_json::Value =
                response.body_mut().read_json().map_err(|e| e.to_string())?;
            let external_id = body
                .get("post_id")
                .or_else(|| body.get("id"))
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string();
            c.execute("UPDATE posts SET status='published',published_at=CURRENT_TIMESTAMP,facebook_post_id=?,updated_at=CURRENT_TIMESTAMP WHERE id=?", params![external_id,post_id]).map_err(|e|e.to_string())?;
            Ok(external_id)
        }
        Err(error) => {
            c.execute(
                "UPDATE posts SET status='failed',updated_at=CURRENT_TIMESTAMP WHERE id=?",
                [post_id],
            )
            .map_err(|e| e.to_string())?;
            Err(format!("Facebook publishing failed: {error}"))
        }
    }
}
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let data = app.path().app_data_dir()?;
            let cache = app.path().app_cache_dir()?.join("thumbnails");
            fs::create_dir_all(&data)?;
            fs::create_dir_all(&cache)?;
            let c = Connection::open(data.join("socialflow.db"))?;
            migrations(&c)?;
            c.execute(
                "INSERT OR REPLACE INTO app_settings(key,value)VALUES('data_path',?)",
                [data.join("socialflow.db").to_string_lossy().to_string()],
            )?;
            let claude = Command::new("/bin/zsh")
                .args(["-lc", "command -v claude && claude --version"])
                .output()
                .ok();
            let installed = claude.as_ref().is_some_and(|x| x.status.success());
            c.execute(
                "INSERT OR REPLACE INTO app_settings(key,value)VALUES('claude_installed',?)",
                [installed.to_string()],
            )?;
            let codex = Command::new("/bin/zsh")
                .args(["-lc", "command -v codex && codex --version"])
                .output()
                .ok();
            let installed = codex.as_ref().is_some_and(|x| x.status.success());
            c.execute(
                "INSERT OR REPLACE INTO app_settings(key,value)VALUES('codex_installed',?)",
                [installed.to_string()],
            )?;
            c.execute(
                "INSERT OR IGNORE INTO app_settings(key,value)VALUES('timezone',?)",
                [chrono::Local::now().format("%Z").to_string()],
            )?;
            app.manage(AppState {
                db: Mutex::new(c),
                cache,
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            load_data,
            import_paths,
            create_post,
            update_post_status,
            create_campaign,
            create_content_campaign,
            set_setting,
            backup_socialflow,
            return_failed_to_review,
            start_live_publisher,
            index_visual_duplicates,
            record_marketing_lead,
            create_wedding,
            update_wedding,
            delete_wedding,
            add_supplier,
            replace_suppliers,
            organise_collection,
            analyse_images_ai,
            update_post,
            reorder_post_images,
            reveal_post_asset,
            approve_all,
            save_instagram_connection,
            disconnect_instagram,
            save_facebook_connection,
            disconnect_facebook,
            publish_post_to_facebook,
            save_tiktok_connection,
            connect_tiktok_oauth,
            backfill_tiktok_reels,
            sync_instagram_insights
        ])
        .run(tauri::generate_context!())
        .expect("error while running SocialFlow")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn database_migrates() {
        let c = Connection::open_in_memory().unwrap();
        migrations(&c).unwrap();
        assert_eq!(
            c.query_row("SELECT count(*) FROM profiles", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            1
        )
    }
    #[test]
    fn analytics_brain_prefers_stronger_evidence() {
        let c = Connection::open_in_memory().unwrap();
        migrations(&c).unwrap();
        for index in 0..3 {
            c.execute("INSERT INTO instagram_performance(instagram_media_id,post_type,reach,likes,comments,saves,shares,plays,published_at,section)VALUES(?, 'single',1000,20,1,1,0,0,'2026-08-01T09:00:00+00:00','portraits')",[format!("single-{index}")]).unwrap();
        }
        for index in 0..3 {
            c.execute("INSERT INTO instagram_performance(instagram_media_id,post_type,reach,likes,comments,saves,shares,plays,published_at,section)VALUES(?, 'reel',1000,45,5,12,8,2000,'2026-08-01T19:00:00+00:00','confetti')",[format!("reel-{index}")]).unwrap();
        }
        let mut settings = HashMap::new();
        settings.insert("insights_permission".into(), "granted".into());
        let report = build_analytics_report(&c, &settings);
        assert_eq!(report.measured_posts, 6);
        assert_eq!(report.formats[0].format, "reel");
        assert!(report
            .recommendations
            .iter()
            .any(|item| item.recommendation.contains("19:00")));
    }
    #[test]
    fn file_validation() {
        assert!(supported(Path::new("a.JPG")));
        assert!(!supported(Path::new("a.raw")))
    }
    #[test]
    fn renders_vertical_photo_reel() {
        if ffmpeg_binary().is_err() {
            return;
        }
        let dir =
            std::env::temp_dir().join(format!("socialflow-reel-test-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        let first = dir.join("one.jpg");
        let second = dir.join("two.jpg");
        let output = dir.join("reel.mp4");
        image::DynamicImage::new_rgb8(1200, 800)
            .save(&first)
            .unwrap();
        image::DynamicImage::new_rgb8(800, 1200)
            .save(&second)
            .unwrap();
        render_photo_reel(
            &[
                first.to_string_lossy().to_string(),
                second.to_string_lossy().to_string(),
            ],
            &output,
        )
        .unwrap();
        assert!(fs::metadata(&output).unwrap().len() > 1_000);
        let _ = fs::remove_dir_all(dir);
    }
    #[test]
    fn schedule_uses_three_days() {
        let s = NaiveDate::from_ymd_opt(2026, 8, 3).unwrap();
        assert_eq!(next_slot(s, 1).weekday().num_days_from_sunday(), 3)
    }
    #[test]
    fn high_volume_schedule_spaces_five_daily_posts() {
        let s = NaiveDate::from_ymd_opt(2026, 8, 3).unwrap();
        let slots = (0..6).map(|i| next_daily_slot(s, i, 5)).collect::<Vec<_>>();
        assert_eq!(
            slots.iter().take(5).map(|x| x.hour()).collect::<Vec<_>>(),
            vec![8, 11, 14, 17, 20]
        );
        assert_eq!(slots[5].date_naive(), s + Duration::days(1));
    }
    #[test]
    fn wedding_rotation_keeps_each_day_to_one_wedding() {
        let s = NaiveDate::from_ymd_opt(2026, 8, 3).unwrap();
        let plan = rotation_plan(s, &[7, 6, 5], 5);
        assert_eq!(plan[0][0].date_naive(), s);
        assert_eq!(plan[0][4].date_naive(), s);
        assert_eq!(plan[1][0].date_naive(), s + Duration::days(1));
        assert_eq!(plan[2][0].date_naive(), s + Duration::days(2));
        assert_eq!(plan[0][5].date_naive(), s + Duration::days(3));
        assert_eq!(plan[1][5].date_naive(), s + Duration::days(4));
    }
    #[test]
    fn parses_provider_structured_output() {
        let raw = r##"{"structured_output":{"results":[{"image_id":7,"description":"A laugh during speeches","section":"speeches","mood":"joyful","caption":"A room listening.\nThen the punchline landed.\nNobody kept a straight face.","hashtags":["#one","#two","#three","#four","#five"],"social_score":91}]}}"##;
        let results = parse_visual_output(raw).unwrap();
        assert_eq!(results[0].image_id, 7);
        assert_eq!(results[0].hashtags.len(), 5);
    }
    #[test]
    fn fresh_install_has_publish_recovery() {
        // B3: save_facebook_connection reads this table, so a database that has
        // never run the Python publisher must still have it.
        let c = Connection::open_in_memory().unwrap();
        migrations(&c).unwrap();
        c.execute(
            "UPDATE posts SET status='scheduled' WHERE id IN (SELECT post_id FROM publish_recovery WHERE failure_class='authentication')",
            [],
        )
        .expect("the Facebook reconnect statement must not fail on a fresh install");
    }

    #[test]
    fn only_shippable_formats_are_publishable() {
        // B1: the combinations the publisher actually implements.
        assert!(publishable("instagram", "single"));
        assert!(publishable("tiktok", "reel"));
        for format in ["carousel", "reel", "story_pack"] {
            assert!(!publishable("instagram", format), "{format} cannot be sent to Instagram");
        }
        assert!(!publishable("tiktok", "single"));
        // Live history: Facebook has published carousels, Reels and singles.
        // Blocking any of them would be a regression, however lossy the result.
        for format in ["single", "carousel", "reel", "story_pack"] {
            assert!(publishable("facebook", format), "Facebook publishes {format} today");
        }
    }

    #[test]
    fn bulk_approval_leaves_unpublishable_posts_alone() {
        // B1: approve_all must not sweep a carousel into the queue where it is
        // certain to fail on its scheduled day.
        let c = Connection::open_in_memory().unwrap();
        migrations(&c).unwrap();
        c.execute("INSERT INTO posts(id,profile_id,status,post_type,platform,scheduled_at)VALUES(1,1,'needs_review','carousel','instagram','2026-08-12 09:00:00')",[]).unwrap();
        c.execute("INSERT INTO posts(id,profile_id,status,post_type,platform,scheduled_at)VALUES(2,1,'needs_review','single','instagram','2026-08-12 09:00:00')",[]).unwrap();
        c.execute(&format!("UPDATE posts SET status=CASE WHEN scheduled_at IS NULL THEN 'approved' ELSE 'scheduled' END WHERE status IN ('draft','needs_review','approved') AND {PUBLISHABLE_SQL}"),[]).unwrap();
        let carousel: String = c.query_row("SELECT status FROM posts WHERE id=1",[],|r|r.get(0)).unwrap();
        let single: String = c.query_row("SELECT status FROM posts WHERE id=2",[],|r|r.get(0)).unwrap();
        assert_eq!(carousel, "needs_review");
        assert_eq!(single, "scheduled");
    }

    #[test]
    fn publishable_predicate_is_total() {
        // A NULL post_type must resolve to a definite true/false, not SQL NULL,
        // or a row is neither approved nor stood down and sits in limbo.
        let c = Connection::open_in_memory().unwrap();
        migrations(&c).unwrap();
        c.execute("INSERT INTO posts(id,profile_id,status,post_type,platform)VALUES(1,1,'draft',NULL,'instagram')",[]).unwrap();
        let (yes, no): (bool, bool) = c
            .query_row(
                &format!("SELECT {PUBLISHABLE_SQL},NOT {PUBLISHABLE_SQL} FROM posts WHERE id=1"),
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .expect("predicate must never evaluate to NULL");
        assert!(yes ^ no, "exactly one of publishable / not-publishable must hold");
    }

    #[test]
    fn no_children_consent_withholds_unanalysed_photographs() {
        // B5: absence of evidence is not evidence of absence. An unanalysed
        // photograph must be withheld, not approved by default.
        let c = Connection::open_in_memory().unwrap();
        migrations(&c).unwrap();
        c.execute("INSERT INTO weddings(id,profile_id,couple_names,consent_level)VALUES(1,1,'A & B','no_children')",[]).unwrap();
        for id in 1..=3 {
            c.execute("INSERT INTO images(id,source_path,filename,file_hash,file_size)VALUES(?,?,?,?,0)",params![id,format!("/tmp/{id}.jpg"),format!("{id}.jpg"),format!("hash{id}")]).unwrap();
        }
        // 1 = analysed, adults only. 2 = analysed, has a child. 3 = never analysed.
        c.execute("INSERT INTO image_analysis(image_id,provider,description)VALUES(1,'claude','Two adults laughing during the speeches')",[]).unwrap();
        c.execute("INSERT INTO image_analysis(image_id,provider,description)VALUES(2,'claude','A small child running past the top table')",[]).unwrap();
        let safe = marketing_safe_images(&c, 1, vec![1, 2, 3]).unwrap();
        assert_eq!(safe, vec![1], "only the photograph shown to be adults-only may pass");
    }

    #[test]
    fn insights_link_to_posts_despite_appended_hashtags() {
        // B4: the publisher appends hashtags, so the remote caption is never
        // equal to the stored one. Prefix matching is what actually links them.
        let c = Connection::open_in_memory().unwrap();
        migrations(&c).unwrap();
        c.execute("INSERT INTO posts(id,profile_id,caption,status)VALUES(1,1,'A room listening.\nThen the punchline landed.','published')",[]).unwrap();
        let remote = "A room listening.\nThen the punchline landed.\n\n#one #two #three";
        let linked: Option<i64> = c
            .query_row(
                "SELECT id FROM posts WHERE caption<>'' AND substr(?,1,length(caption))=caption ORDER BY id DESC LIMIT 1",
                [remote],
                |r| r.get(0),
            )
            .optional()
            .unwrap();
        assert_eq!(linked, Some(1));
        // And equality — the old behaviour — still links nothing, which is the defect.
        let by_equality: Option<i64> = c
            .query_row("SELECT id FROM posts WHERE caption=? LIMIT 1", [remote], |r| r.get(0))
            .optional()
            .unwrap();
        assert_eq!(by_equality, None);
    }

    #[test]
    fn status_constraint_rejects_unknown() {
        let c = Connection::open_in_memory().unwrap();
        migrations(&c).unwrap();
        assert!(c
            .execute("INSERT INTO posts(profile_id,status)VALUES(1,'bogus')", [])
            .is_err())
    }
}
