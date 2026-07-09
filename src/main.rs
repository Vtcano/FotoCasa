use axum::{
    body::Bytes,
    extract::{Path, State},
    http::{header, StatusCode},
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use exif::{In, Reader, Tag, Value};
use image::{
    codecs::jpeg::JpegEncoder, DynamicImage, ExtendedColorType, ImageDecoder, ImageEncoder,
};
use serde::{Deserialize, Serialize};
use sqlx::{sqlite::SqlitePoolOptions, FromRow, SqlitePool};
use std::{
    fs,
    io::{BufReader, Read},
    net::SocketAddr,
    path::{Component, Path as FsPath, PathBuf},
    sync::Arc,
    time::UNIX_EPOCH,
};
use tokio::{
    fs as async_fs,
    sync::Mutex,
    task,
    time::{sleep, Duration},
};
use tower_http::{services::ServeDir, trace::TraceLayer};

const DEFAULT_PHOTO_ROOT: &str = r"Z:\Fotos_iphone";
const DB_URL: &str = "sqlite://data/fotos.sqlite?mode=rwc";
const THUMB_DIR: &str = "data/thumbs";

#[derive(Clone)]
struct AppState {
    photo_root: PathBuf,
    db: SqlitePool,
    index_lock: Arc<Mutex<()>>,
    geo_lock: Arc<Mutex<()>>,
    geo_job: Arc<Mutex<GeocodeJob>>,
    favorite_job: Arc<Mutex<FavoriteScanJob>>,
    thumbnail_job: Arc<Mutex<ThumbnailJob>>,
}

#[derive(Debug, Serialize, FromRow)]
struct Photo {
    id: String,
    name: String,
    relative_path: String,
    media_url: String,
    thumb_url: String,
    extension: String,
    kind: String,
    size_bytes: i64,
    modified_at: Option<i64>,
    captured_at: Option<String>,
    date_bucket: Option<String>,
    country: Option<String>,
    city: Option<String>,
    latitude: Option<f64>,
    longitude: Option<f64>,
    geo_status: Option<String>,
    favorite: i64,
    people: Option<String>,
}

#[derive(Debug)]
struct IndexedPhoto {
    id: String,
    name: String,
    relative_path: String,
    media_url: String,
    extension: String,
    kind: String,
    size_bytes: i64,
    modified_at: Option<i64>,
    captured_at: Option<String>,
    date_bucket: Option<String>,
    country: Option<String>,
    city: Option<String>,
    latitude: Option<f64>,
    longitude: Option<f64>,
    geo_status: Option<String>,
    favorite: i64,
    people: Option<String>,
}

#[derive(Debug, Serialize)]
struct Filters {
    dates: Vec<String>,
    countries: Vec<String>,
    cities: Vec<String>,
    people: Vec<String>,
}

#[derive(Debug, Serialize)]
struct RefreshResult {
    indexed: usize,
}

#[derive(Debug, Deserialize)]
struct FavoritePayload {
    favorite: bool,
}

#[derive(Debug, Deserialize)]
struct BulkLocationPayload {
    ids: Vec<String>,
    country: Option<String>,
    city: Option<String>,
}

#[derive(Debug, Serialize)]
struct BulkLocationResult {
    updated: u64,
}

#[derive(Debug, Deserialize)]
struct RotatePayload {
    direction: String,
}

#[derive(Debug, Serialize)]
struct SettingsStats {
    thumbnails_loaded: usize,
    total_photos: i64,
    total_videos: i64,
    thumbnail_job: ThumbnailJob,
}

#[derive(Debug, Clone, Serialize)]
struct ThumbnailJob {
    running: bool,
    stop_requested: bool,
    total: usize,
    processed: usize,
    created: usize,
    skipped: usize,
    failed: usize,
    message: String,
}

impl Default for ThumbnailJob {
    fn default() -> Self {
        Self {
            running: false,
            stop_requested: false,
            total: 0,
            processed: 0,
            created: 0,
            skipped: 0,
            failed: 0,
            message: "Sin generacion activa".to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
struct FavoriteScanJob {
    running: bool,
    total: usize,
    processed: usize,
    found: usize,
    message: String,
}

impl Default for FavoriteScanJob {
    fn default() -> Self {
        Self {
            running: false,
            total: 0,
            processed: 0,
            found: 0,
            message: "Sin barrido activo".to_string(),
        }
    }
}

#[derive(Debug, Serialize)]
struct GeocodeResult {
    processed: usize,
    located: usize,
    unknown: usize,
}

#[derive(Debug, Clone, Serialize)]
struct GeocodeJob {
    running: bool,
    total: usize,
    processed: usize,
    located: usize,
    unknown: usize,
    message: String,
}

impl Default for GeocodeJob {
    fn default() -> Self {
        Self {
            running: false,
            total: 0,
            processed: 0,
            located: 0,
            unknown: 0,
            message: "Sin barrido activo".to_string(),
        }
    }
}

#[derive(Debug, Deserialize)]
struct NominatimResponse {
    address: Option<NominatimAddress>,
}

#[derive(Debug, Deserialize)]
struct NominatimAddress {
    country: Option<String>,
    city: Option<String>,
    town: Option<String>,
    village: Option<String>,
    municipality: Option<String>,
    county: Option<String>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter("gestor_mvp=debug,tower_http=debug")
        .init();

    ensure_data_dir()?;

    let photo_root = std::env::var("PHOTO_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(DEFAULT_PHOTO_ROOT));

    let db = SqlitePoolOptions::new()
        .max_connections(5)
        .connect(DB_URL)
        .await?;
    init_db(&db).await?;

    let state = AppState {
        photo_root,
        db,
        index_lock: Arc::new(Mutex::new(())),
        geo_lock: Arc::new(Mutex::new(())),
        geo_job: Arc::new(Mutex::new(GeocodeJob::default())),
        favorite_job: Arc::new(Mutex::new(FavoriteScanJob::default())),
        thumbnail_job: Arc::new(Mutex::new(ThumbnailJob::default())),
    };

    let api = Router::new()
        .route("/photos", get(list_photos))
        .route(
            "/photos/bulk-location",
            axum::routing::put(update_bulk_location),
        )
        .route("/photos/:id/rotate", axum::routing::put(rotate_photo))
        .route("/photos/:id/favorite", axum::routing::put(update_favorite))
        .route("/favorites/scan", post(start_favorite_scan))
        .route("/favorites/scan/status", get(favorite_scan_status))
        .route("/filters", get(list_filters))
        .route("/settings/stats", get(settings_stats))
        .route("/thumbnails/toggle", post(toggle_thumbnail_generation))
        .route("/thumbnails/status", get(thumbnail_status))
        .route("/refresh", post(refresh_index))
        .route("/normalize-countries", post(normalize_countries))
        .route("/normalize-cities", post(normalize_cities))
        .route("/geocode", post(start_geocode_locations))
        .route("/geocode/status", get(geocode_status));

    let app = Router::new()
        .nest("/api", api)
        .route("/media/:id", get(serve_media))
        .route("/thumb/:id", get(serve_thumbnail))
        .fallback_service(ServeDir::new("static").append_index_html_on_directories(true))
        .layer(TraceLayer::new_for_http())
        .with_state(state.clone());

    let addr = SocketAddr::from(([0, 0, 0, 0], 3000));
    println!("Servidor listo en http://localhost:3000");
    println!("Carpeta de fotos: {}", state.photo_root.display());
    println!("Indice SQLite: data/fotos.sqlite");

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}

fn ensure_data_dir() -> anyhow::Result<()> {
    if !FsPath::new("data").exists() {
        fs::create_dir_all("data")?;
    }
    if !FsPath::new(THUMB_DIR).exists() {
        fs::create_dir_all(THUMB_DIR)?;
    }

    Ok(())
}

async fn init_db(db: &SqlitePool) -> anyhow::Result<()> {
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS geocode_cache (
            coord_key TEXT PRIMARY KEY,
            latitude REAL NOT NULL,
            longitude REAL NOT NULL,
            country TEXT NOT NULL,
            city TEXT NOT NULL,
            status TEXT NOT NULL,
            indexed_at INTEGER NOT NULL
        );
        "#,
    )
    .execute(db)
    .await?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS photos (
            id TEXT PRIMARY KEY,
            name TEXT NOT NULL,
            relative_path TEXT NOT NULL UNIQUE,
            media_url TEXT NOT NULL,
            extension TEXT NOT NULL,
            kind TEXT NOT NULL,
            size_bytes INTEGER NOT NULL,
            modified_at INTEGER,
            captured_at TEXT,
            date_bucket TEXT,
            country TEXT,
            city TEXT,
            latitude REAL,
            longitude REAL,
            geo_status TEXT,
            favorite INTEGER NOT NULL DEFAULT 0,
            people TEXT,
            indexed_at INTEGER NOT NULL
        );
        "#,
    )
    .execute(db)
    .await?;

    add_column_if_missing(db, "photos", "latitude", "REAL").await?;
    add_column_if_missing(db, "photos", "longitude", "REAL").await?;
    add_column_if_missing(db, "photos", "geo_status", "TEXT").await?;
    add_column_if_missing(db, "photos", "favorite", "INTEGER NOT NULL DEFAULT 0").await?;
    add_column_if_missing(db, "photos", "people", "TEXT").await?;

    Ok(())
}

async fn add_column_if_missing(
    db: &SqlitePool,
    table: &str,
    column: &str,
    definition: &str,
) -> anyhow::Result<()> {
    let rows: Vec<(String,)> =
        sqlx::query_as(&format!("SELECT name FROM pragma_table_info('{table}')"))
            .fetch_all(db)
            .await?;

    if rows.iter().any(|(name,)| name == column) {
        return Ok(());
    }

    sqlx::query(&format!(
        "ALTER TABLE {table} ADD COLUMN {column} {definition}"
    ))
    .execute(db)
    .await?;
    Ok(())
}

async fn list_photos(State(state): State<AppState>) -> Result<Json<Vec<Photo>>, AppError> {
    ensure_index_if_empty(&state).await?;

    let photos = sqlx::query_as::<_, Photo>(
        r#"
        SELECT id, name, relative_path, media_url, '/thumb/' || id AS thumb_url,
               extension, kind, size_bytes,
               modified_at, captured_at, date_bucket, country, city,
               latitude, longitude, geo_status, favorite, people
        FROM photos
        ORDER BY COALESCE(captured_at, datetime(modified_at, 'unixepoch'), date_bucket, name) ASC
        "#,
    )
    .fetch_all(&state.db)
    .await?;

    Ok(Json(photos))
}

async fn update_favorite(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(payload): Json<FavoritePayload>,
) -> Result<Json<Photo>, AppError> {
    let favorite = if payload.favorite { 1 } else { 0 };

    let photo = sqlx::query_as::<_, Photo>(
        r#"
        UPDATE photos
        SET favorite = ?
        WHERE id = ?
        RETURNING id, name, relative_path, media_url, '/thumb/' || id AS thumb_url,
                  extension, kind, size_bytes,
                  modified_at, captured_at, date_bucket, country, city,
                  latitude, longitude, geo_status, favorite, people
        "#,
    )
    .bind(favorite)
    .bind(id)
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(|| AppError::not_found("Foto no encontrada"))?;

    Ok(Json(photo))
}

async fn update_bulk_location(
    State(state): State<AppState>,
    Json(payload): Json<BulkLocationPayload>,
) -> Result<Json<BulkLocationResult>, AppError> {
    if payload.ids.is_empty() {
        return Err(AppError::bad_request("Selecciona al menos una foto"));
    }

    let country = payload
        .country
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    let city = payload
        .city
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());

    if country.is_none() && city.is_none() {
        return Err(AppError::bad_request("Escribe un pais, una ciudad o ambos"));
    }

    let mut transaction = state.db.begin().await?;
    let mut updated = 0;

    for id in payload.ids {
        let result =
            match (&country, &city) {
                (Some(country), Some(city)) => sqlx::query(
                    "UPDATE photos SET country = ?, city = ?, geo_status = 'manual' WHERE id = ?",
                )
                .bind(country)
                .bind(city)
                .bind(id)
                .execute(&mut *transaction)
                .await?,
                (Some(country), None) => {
                    sqlx::query("UPDATE photos SET country = ?, geo_status = 'manual' WHERE id = ?")
                        .bind(country)
                        .bind(id)
                        .execute(&mut *transaction)
                        .await?
                }
                (None, Some(city)) => {
                    sqlx::query("UPDATE photos SET city = ?, geo_status = 'manual' WHERE id = ?")
                        .bind(city)
                        .bind(id)
                        .execute(&mut *transaction)
                        .await?
                }
                (None, None) => unreachable!(),
            };

        updated += result.rows_affected();
    }

    transaction.commit().await?;

    Ok(Json(BulkLocationResult { updated }))
}

async fn rotate_photo(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(payload): Json<RotatePayload>,
) -> Result<Json<Photo>, AppError> {
    let relative = decode_id(&id)?;
    let source_path = safe_media_path(&state.photo_root, &relative)?;
    let extension = source_path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();

    if !matches!(extension.as_str(), "jpg" | "jpeg") {
        return Err(AppError::bad_request(
            "Por ahora solo puedo girar originales JPG/JPEG sin perder los metadatos",
        ));
    }

    let direction = payload.direction;
    let rotate_path = source_path.clone();
    task::spawn_blocking(move || rotate_jpeg_original(&rotate_path, &direction))
        .await
        .map_err(|_| AppError::internal("No se pudo girar la foto"))??;

    let thumb_path = thumbnail_path(&id);
    if thumb_path.exists() {
        fs::remove_file(thumb_path)?;
    }

    let metadata = fs::metadata(&source_path)?;
    let modified_at = metadata
        .modified()
        .ok()
        .and_then(|value| value.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_secs() as i64);

    let size_bytes = metadata.len() as i64;
    let photo = sqlx::query_as::<_, Photo>(
        r#"
        UPDATE photos
        SET size_bytes = ?, modified_at = ?
        WHERE id = ?
        RETURNING id, name, relative_path, media_url, '/thumb/' || id AS thumb_url,
                  extension, kind, size_bytes,
                  modified_at, captured_at, date_bucket, country, city,
                  latitude, longitude, geo_status, favorite, people
        "#,
    )
    .bind(size_bytes)
    .bind(modified_at)
    .bind(id)
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(|| AppError::not_found("Foto no encontrada"))?;

    Ok(Json(photo))
}

async fn start_favorite_scan(
    State(state): State<AppState>,
) -> Result<Json<FavoriteScanJob>, AppError> {
    {
        let job = state.favorite_job.lock().await;
        if job.running {
            return Ok(Json(job.clone()));
        }
    }

    {
        let mut job = state.favorite_job.lock().await;
        *job = FavoriteScanJob {
            running: true,
            total: 0,
            processed: 0,
            found: 0,
            message: "Preparando barrido de favoritos".to_string(),
        };
    }

    let state_for_job = state.clone();
    tokio::spawn(async move {
        if let Err(error) = run_favorite_scan(state_for_job.clone()).await {
            let mut job = state_for_job.favorite_job.lock().await;
            job.running = false;
            job.message = error.message;
        }
    });

    Ok(Json(state.favorite_job.lock().await.clone()))
}

async fn favorite_scan_status(State(state): State<AppState>) -> Json<FavoriteScanJob> {
    Json(state.favorite_job.lock().await.clone())
}

async fn run_favorite_scan(state: AppState) -> Result<(), AppError> {
    let rows = sqlx::query_as::<_, (String, String)>(
        "SELECT id, relative_path FROM photos WHERE kind = 'image' AND favorite = 0",
    )
    .fetch_all(&state.db)
    .await?;

    {
        let mut job = state.favorite_job.lock().await;
        job.total = rows.len();
        job.message = format!("0 de {} fotos revisadas", rows.len());
    }

    let mut processed = 0;
    let mut found = 0;

    for (id, relative_path) in rows {
        processed += 1;
        let path = safe_media_path(&state.photo_root, &relative_path)?;
        let is_favorite = task::spawn_blocking(move || photo_looks_favorite(&path))
            .await
            .map_err(|_| AppError::internal("No se pudo leer favoritos"))?;

        if is_favorite {
            sqlx::query("UPDATE photos SET favorite = 1 WHERE id = ?")
                .bind(id)
                .execute(&state.db)
                .await?;
            found += 1;
        }

        if processed % 100 == 0 || processed == found || processed == 1 {
            let mut job = state.favorite_job.lock().await;
            job.processed = processed;
            job.found = found;
            job.message = format!(
                "{processed} de {} fotos revisadas, {found} favoritas",
                job.total
            );
        }
    }

    let mut job = state.favorite_job.lock().await;
    job.running = false;
    job.processed = processed;
    job.found = found;
    job.message = format!("Barrido terminado: {found} favoritas encontradas");

    Ok(())
}

async fn list_filters(State(state): State<AppState>) -> Result<Json<Filters>, AppError> {
    ensure_index_if_empty(&state).await?;

    let dates = distinct_values(&state.db, "date_bucket").await?;
    let countries = distinct_values(&state.db, "country").await?;
    let cities = distinct_values(&state.db, "city").await?;
    let people = distinct_people(&state.db).await?;

    Ok(Json(Filters {
        dates,
        countries,
        cities,
        people,
    }))
}

async fn settings_stats(State(state): State<AppState>) -> Result<Json<SettingsStats>, AppError> {
    ensure_index_if_empty(&state).await?;

    let total_photos: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM photos WHERE kind = 'image'")
        .fetch_one(&state.db)
        .await?;
    let total_videos: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM photos WHERE kind = 'video'")
        .fetch_one(&state.db)
        .await?;
    let image_ids = sqlx::query_scalar::<_, String>("SELECT id FROM photos WHERE kind = 'image'")
        .fetch_all(&state.db)
        .await?;
    let thumbnails_loaded = image_ids
        .iter()
        .filter(|id| thumbnail_path(id).exists())
        .count();
    let thumbnail_job = state.thumbnail_job.lock().await.clone();

    Ok(Json(SettingsStats {
        thumbnails_loaded,
        total_photos,
        total_videos,
        thumbnail_job,
    }))
}

async fn toggle_thumbnail_generation(
    State(state): State<AppState>,
) -> Result<Json<ThumbnailJob>, AppError> {
    ensure_index_if_empty(&state).await?;

    {
        let mut job = state.thumbnail_job.lock().await;
        if job.running {
            job.stop_requested = true;
            job.message = "Parando generacion de mini fotos...".to_string();
            return Ok(Json(job.clone()));
        }

        *job = ThumbnailJob {
            running: true,
            stop_requested: false,
            total: 0,
            processed: 0,
            created: 0,
            skipped: 0,
            failed: 0,
            message: "Preparando mini fotos...".to_string(),
        };
    }

    let state_for_job = state.clone();
    tokio::spawn(async move {
        if let Err(error) = run_thumbnail_generation(state_for_job.clone()).await {
            let mut job = state_for_job.thumbnail_job.lock().await;
            job.running = false;
            job.stop_requested = false;
            job.message = error.message;
        }
    });

    Ok(Json(state.thumbnail_job.lock().await.clone()))
}

async fn thumbnail_status(State(state): State<AppState>) -> Json<ThumbnailJob> {
    Json(state.thumbnail_job.lock().await.clone())
}

async fn run_thumbnail_generation(state: AppState) -> Result<(), AppError> {
    let rows = sqlx::query_as::<_, (String, String, String)>(
        "SELECT id, relative_path, extension FROM photos WHERE kind = 'image' ORDER BY COALESCE(captured_at, date_bucket, name) ASC",
    )
    .fetch_all(&state.db)
    .await?;

    let pending: Vec<(String, String)> = rows
        .into_iter()
        .filter(|(id, _, extension)| {
            is_thumbnail_supported_extension(extension) && !thumbnail_path(id).exists()
        })
        .map(|(id, relative_path, _)| (id, relative_path))
        .collect();

    {
        let mut job = state.thumbnail_job.lock().await;
        job.total = pending.len();
        job.message = if pending.is_empty() {
            "Todas las mini fotos soportadas estan generadas".to_string()
        } else {
            format!("0/{} mini fotos", pending.len())
        };
    }

    for (id, relative_path) in pending {
        {
            let job = state.thumbnail_job.lock().await;
            if job.stop_requested {
                drop(job);
                let mut job = state.thumbnail_job.lock().await;
                job.running = false;
                job.stop_requested = false;
                job.message = format!("Generacion parada en {}/{}", job.processed, job.total);
                return Ok(());
            }
        }

        let source_path = match safe_media_path(&state.photo_root, &relative_path) {
            Ok(path) => path,
            Err(_) => {
                let mut job = state.thumbnail_job.lock().await;
                job.processed += 1;
                job.failed += 1;
                job.message = format!("{}/{} mini fotos", job.processed, job.total);
                continue;
            }
        };
        let target_path = thumbnail_path(&id);
        let result = task::spawn_blocking(move || create_thumbnail(&source_path, &target_path))
            .await
            .map_err(|_| AppError::internal("No se pudo crear miniatura"));

        let mut job = state.thumbnail_job.lock().await;
        job.processed += 1;
        match result {
            Ok(Ok(())) => job.created += 1,
            Ok(Err(_)) => job.failed += 1,
            Err(_) => job.failed += 1,
        }
        job.message = format!("{}/{} mini fotos", job.processed, job.total);
    }

    let mut job = state.thumbnail_job.lock().await;
    job.running = false;
    job.stop_requested = false;
    job.message = format!(
        "Mini fotos listas: {} creadas, {} fallidas",
        job.created, job.failed
    );

    Ok(())
}

async fn refresh_index(State(state): State<AppState>) -> Result<Json<RefreshResult>, AppError> {
    let indexed = rebuild_index(&state).await?;
    Ok(Json(RefreshResult { indexed }))
}

async fn normalize_countries(
    State(state): State<AppState>,
) -> Result<Json<RefreshResult>, AppError> {
    let mut updated = 0;
    for (from, to) in country_normalization_pairs() {
        let result = sqlx::query("UPDATE photos SET country = ? WHERE country = ?")
            .bind(to)
            .bind(from)
            .execute(&state.db)
            .await?;
        updated += result.rows_affected() as usize;

        sqlx::query("UPDATE geocode_cache SET country = ? WHERE country = ?")
            .bind(to)
            .bind(from)
            .execute(&state.db)
            .await?;
    }

    Ok(Json(RefreshResult { indexed: updated }))
}

async fn normalize_cities(State(state): State<AppState>) -> Result<Json<RefreshResult>, AppError> {
    let mut updated = 0;
    for (from, to) in city_normalization_pairs() {
        let result = sqlx::query("UPDATE photos SET city = ? WHERE city = ?")
            .bind(to)
            .bind(from)
            .execute(&state.db)
            .await?;
        updated += result.rows_affected() as usize;

        sqlx::query("UPDATE geocode_cache SET city = ? WHERE city = ?")
            .bind(to)
            .bind(from)
            .execute(&state.db)
            .await?;
    }

    Ok(Json(RefreshResult { indexed: updated }))
}

async fn start_geocode_locations(
    State(state): State<AppState>,
) -> Result<Json<GeocodeJob>, AppError> {
    {
        let job = state.geo_job.lock().await;
        if job.running {
            return Ok(Json(job.clone()));
        }
    }

    {
        let mut job = state.geo_job.lock().await;
        *job = GeocodeJob {
            running: true,
            total: 0,
            processed: 0,
            located: 0,
            unknown: 0,
            message: "Preparando barrido de ubicaciones".to_string(),
        };
    }

    let state_for_job = state.clone();
    tokio::spawn(async move {
        if let Err(error) = run_geocode_locations(state_for_job.clone()).await {
            let mut job = state_for_job.geo_job.lock().await;
            job.running = false;
            job.message = error.message;
        }
    });

    let job = state.geo_job.lock().await.clone();
    Ok(Json(job))
}

async fn geocode_status(State(state): State<AppState>) -> Json<GeocodeJob> {
    Json(state.geo_job.lock().await.clone())
}

async fn run_geocode_locations(state: AppState) -> Result<GeocodeResult, AppError> {
    let _guard = state.geo_lock.lock().await;

    let coords = sqlx::query_as::<_, (f64, f64)>(
        r#"
        SELECT DISTINCT ROUND(latitude, 3), ROUND(longitude, 3)
        FROM photos
        WHERE latitude IS NOT NULL
          AND longitude IS NOT NULL
          AND (
            geo_status IS NULL
            OR geo_status <> 'located'
            OR country IS NULL
            OR city IS NULL
          )
        ORDER BY ROUND(latitude, 3), ROUND(longitude, 3)
        "#,
    )
    .fetch_all(&state.db)
    .await?;

    {
        let mut job = state.geo_job.lock().await;
        job.total = coords.len();
        job.message = format!("0 de {} ubicaciones", coords.len());
    }

    let client = reqwest::Client::builder()
        .user_agent("fotocasa-local-photo-viewer/0.1")
        .build()
        .map_err(|_| AppError::internal("No se pudo preparar geocodificacion"))?;

    let mut processed = 0;
    let mut located = 0;
    let mut unknown = 0;

    for (latitude, longitude) in coords {
        processed += 1;
        let coord_key = coord_key(latitude, longitude);
        let cached = cached_location(&state.db, &coord_key).await?;

        let (country, city, status) = match cached {
            Some(location) => location,
            None => {
                let location = match reverse_geocode(&client, latitude, longitude).await {
                    Ok(Some((country, city))) => (country, city, "located".to_string()),
                    _ => (
                        "Unknown".to_string(),
                        "Unknown".to_string(),
                        "unknown".to_string(),
                    ),
                };

                sqlx::query(
                    r#"
                    INSERT OR REPLACE INTO geocode_cache
                        (coord_key, latitude, longitude, country, city, status, indexed_at)
                    VALUES (?, ?, ?, ?, ?, ?, ?)
                    "#,
                )
                .bind(&coord_key)
                .bind(latitude)
                .bind(longitude)
                .bind(&location.0)
                .bind(&location.1)
                .bind(&location.2)
                .bind(now_unix())
                .execute(&state.db)
                .await?;

                sleep(Duration::from_millis(1100)).await;
                location
            }
        };

        sqlx::query(
            r#"
            UPDATE photos
            SET country = ?, city = ?, geo_status = ?
            WHERE ROUND(latitude, 3) = ? AND ROUND(longitude, 3) = ?
            "#,
        )
        .bind(&country)
        .bind(&city)
        .bind(&status)
        .bind(latitude)
        .bind(longitude)
        .execute(&state.db)
        .await?;

        if status == "located" {
            located += 1;
        } else {
            unknown += 1;
        }

        {
            let mut job = state.geo_job.lock().await;
            job.processed = processed;
            job.located = located;
            job.unknown = unknown;
            job.message = format!("{processed} de {} ubicaciones", job.total);
        }
    }

    let without_gps: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM photos WHERE latitude IS NULL OR longitude IS NULL",
    )
    .fetch_one(&state.db)
    .await?;

    sqlx::query(
        "UPDATE photos SET country = 'Unknown', city = 'Unknown', geo_status = 'unknown'
         WHERE latitude IS NULL OR longitude IS NULL",
    )
    .execute(&state.db)
    .await?;
    unknown += without_gps as usize;

    {
        let mut job = state.geo_job.lock().await;
        job.running = false;
        job.processed = processed;
        job.located = located;
        job.unknown = unknown;
        job.message =
            format!("Barrido terminado: {located} ubicaciones resueltas, {unknown} sin datos");
    }

    Ok(GeocodeResult {
        processed,
        located,
        unknown,
    })
}

async fn cached_location(
    db: &SqlitePool,
    coord_key: &str,
) -> Result<Option<(String, String, String)>, AppError> {
    let location = sqlx::query_as::<_, (String, String, String)>(
        "SELECT country, city, status FROM geocode_cache WHERE coord_key = ?",
    )
    .bind(coord_key)
    .fetch_optional(db)
    .await?;

    Ok(location)
}

fn coord_key(latitude: f64, longitude: f64) -> String {
    format!("{latitude:.3},{longitude:.3}")
}

async fn reverse_geocode(
    client: &reqwest::Client,
    latitude: f64,
    longitude: f64,
) -> Result<Option<(String, String)>, AppError> {
    let response = client
        .get("https://nominatim.openstreetmap.org/reverse")
        .query(&[
            ("format", "jsonv2"),
            ("lat", &latitude.to_string()),
            ("lon", &longitude.to_string()),
            ("zoom", "10"),
            ("addressdetails", "1"),
            ("accept-language", "es"),
        ])
        .send()
        .await
        .map_err(|_| AppError::internal("Error consultando pais/ciudad"))?;

    if !response.status().is_success() {
        return Ok(None);
    }

    let data = response
        .json::<NominatimResponse>()
        .await
        .map_err(|_| AppError::internal("Error leyendo pais/ciudad"))?;

    let Some(address) = data.address else {
        return Ok(None);
    };

    let country = normalize_country_name(&address.country.unwrap_or_else(|| "Unknown".to_string()));
    let city = address
        .city
        .or(address.town)
        .or(address.village)
        .or(address.municipality)
        .or(address.county)
        .unwrap_or_else(|| "Unknown".to_string());
    let city = normalize_city_name(&city);

    Ok(Some((country, city)))
}

fn normalize_city_name(city: &str) -> String {
    let clean = city.trim();

    for (from, to) in city_normalization_pairs() {
        if clean == *from {
            return (*to).to_string();
        }
    }

    clean.to_string()
}

fn normalize_country_name(country: &str) -> String {
    let clean = country.trim();

    for (from, to) in country_normalization_pairs() {
        if clean == *from {
            return (*to).to_string();
        }
    }

    clean.to_string()
}

fn country_normalization_pairs() -> &'static [(&'static str, &'static str)] {
    &[
        ("België / Belgique / Belgien", "Bélgica"),
        ("BelgiÃ« / Belgique / Belgien", "Bélgica"),
        ("Belgium", "Bélgica"),
        ("Deutschland", "Alemania"),
        ("Germany", "Alemania"),
        ("España", "España"),
        ("EspaÃ±a", "España"),
        ("Spain", "España"),
        ("France", "Francia"),
        ("Indonesia", "Indonesia"),
        ("Liechtenstein", "Liechtenstein"),
        ("Maroc ⵍⵎⵖⵔⵉⴱ المغرب", "Marruecos"),
        ("Maroc âµâµâµâµâµâ´± Ø§ÙÙØºØ±Ø¨", "Marruecos"),
        ("Morocco", "Marruecos"),
        ("România", "Rumanía"),
        ("RomÃ¢nia", "Rumanía"),
        ("Romania", "Rumanía"),
        ("Schweiz/Suisse/Svizzera/Svizra", "Suiza"),
        ("Switzerland", "Suiza"),
        ("Türkiye", "Turquía"),
        ("TÃ¼rkiye", "Turquía"),
        ("Turkey", "Turquía"),
        ("United Kingdom", "Reino Unido"),
        ("Ελλάς", "Grecia"),
        ("ÎÎ»Î»Î¬Ï", "Grecia"),
        ("Greece", "Grecia"),
        ("България", "Bulgaria"),
        ("ÐÑÐ»Ð³Ð°ÑÐ¸Ñ", "Bulgaria"),
        ("Bulgaria", "Bulgaria"),
    ]
}

fn city_normalization_pairs() -> &'static [(&'static str, &'static str)] {
    &[
        ("Αθήνα", "Atenas"),
        ("ÎÎ¸Î®Î½Î±", "Atenas"),
        ("Πειραιάς", "El Pireo"),
        ("Î ÎµÎ¹ÏÎ±Î¹Î¬Ï", "El Pireo"),
        ("Δήμος Μήλου", "Milos"),
        ("ÎÎ®Î¼Î¿Ï ÎÎ®Î»Î¿Ï", "Milos"),
        ("Περιφερειακή Ενότητα Μήλου", "Milos"),
        (
            "Î ÎµÏÎ¹ÏÎµÏÎµÎ¹Î±ÎºÎ® ÎÎ½ÏÏÎ·ÏÎ± ÎÎ®Î»Î¿Ï",
            "Milos",
        ),
        ("Δήμος Πάρου", "Paros"),
        ("ÎÎ®Î¼Î¿Ï Î Î¬ÏÎ¿Ï", "Paros"),
        ("Περιφερειακή Ενότητα Πάρου", "Paros"),
        (
            "Î ÎµÏÎ¹ÏÎµÏÎµÎ¹Î±ÎºÎ® ÎÎ½ÏÏÎ·ÏÎ± Î Î¬ÏÎ¿Ï",
            "Paros",
        ),
        ("Δημοτική Ενότητα Θήρας", "Santorini"),
        ("ÎÎ·Î¼Î¿ÏÎ¹ÎºÎ® ÎÎ½ÏÏÎ·ÏÎ± ÎÎ®ÏÎ±Ï", "Santorini"),
        ("Περιφερειακή Ενότητα Θήρας", "Santorini"),
        (
            "Î ÎµÏÎ¹ÏÎµÏÎµÎ¹Î±ÎºÎ® ÎÎ½ÏÏÎ·ÏÎ± ÎÎ®ÏÎ±Ï",
            "Santorini",
        ),
        ("Δημοτική Ενότητα Οίας", "Oia"),
        ("ÎÎ·Î¼Î¿ÏÎ¹ÎºÎ® ÎÎ½ÏÏÎ·ÏÎ± ÎÎ¯Î±Ï", "Oia"),
    ]
}

async fn ensure_index_if_empty(state: &AppState) -> Result<(), AppError> {
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM photos")
        .fetch_one(&state.db)
        .await?;

    if count == 0 {
        let _guard = state.index_lock.lock().await;
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM photos")
            .fetch_one(&state.db)
            .await?;
        if count > 0 {
            return Ok(());
        }

        rebuild_index_unlocked(state).await?;
    }

    Ok(())
}

async fn rebuild_index(state: &AppState) -> Result<usize, AppError> {
    let _guard = state.index_lock.lock().await;
    rebuild_index_unlocked(state).await
}

async fn rebuild_index_unlocked(state: &AppState) -> Result<usize, AppError> {
    let root = state.photo_root.clone();
    let photos = task::spawn_blocking(move || scan_photos(&root))
        .await
        .map_err(|_| AppError::internal("No se pudo escanear la carpeta"))??;

    let existing_favorites = sqlx::query_as::<_, (String, i64)>(
        "SELECT relative_path, favorite FROM photos WHERE favorite = 1",
    )
    .fetch_all(&state.db)
    .await?;

    let mut tx = state.db.begin().await?;
    sqlx::query("DELETE FROM photos").execute(&mut *tx).await?;

    let indexed_at = now_unix();
    for photo in &photos {
        sqlx::query(
            r#"
            INSERT INTO photos (
                id, name, relative_path, media_url, extension, kind, size_bytes,
                modified_at, captured_at, date_bucket, country, city,
                latitude, longitude, geo_status, favorite, people, indexed_at
            )
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(&photo.id)
        .bind(&photo.name)
        .bind(&photo.relative_path)
        .bind(&photo.media_url)
        .bind(&photo.extension)
        .bind(&photo.kind)
        .bind(photo.size_bytes)
        .bind(photo.modified_at)
        .bind(&photo.captured_at)
        .bind(&photo.date_bucket)
        .bind(&photo.country)
        .bind(&photo.city)
        .bind(photo.latitude)
        .bind(photo.longitude)
        .bind(&photo.geo_status)
        .bind(
            if photo.favorite == 1
                || existing_favorites
                    .iter()
                    .any(|(path, favorite)| path == &photo.relative_path && *favorite == 1)
            {
                1
            } else {
                0
            },
        )
        .bind(&photo.people)
        .bind(indexed_at)
        .execute(&mut *tx)
        .await?;
    }

    tx.commit().await?;

    Ok(photos.len())
}

async fn distinct_values(db: &SqlitePool, column: &str) -> Result<Vec<String>, AppError> {
    let sql = format!(
        "SELECT DISTINCT {column} FROM photos WHERE {column} IS NOT NULL AND TRIM({column}) <> '' ORDER BY {column}"
    );

    let values = sqlx::query_scalar::<_, String>(&sql).fetch_all(db).await?;
    Ok(values)
}

async fn distinct_people(db: &SqlitePool) -> Result<Vec<String>, AppError> {
    let rows = sqlx::query_scalar::<_, String>(
        "SELECT people FROM photos WHERE people IS NOT NULL AND TRIM(people) <> ''",
    )
    .fetch_all(db)
    .await?;

    let mut people = Vec::new();
    for row in rows {
        for name in row.split(',') {
            let name = name.trim();
            if !name.is_empty() && !people.iter().any(|value| value == name) {
                people.push(name.to_string());
            }
        }
    }

    people.sort();
    Ok(people)
}

async fn serve_media(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, AppError> {
    let relative = decode_id(&id)?;
    let path = safe_media_path(&state.photo_root, &relative)?;

    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();

    if !is_supported_extension(&extension) {
        return Err(AppError::not_found("Formato no soportado"));
    }

    let bytes = async_fs::read(path).await?;
    let content_type = content_type_for(&extension);

    Ok((
        StatusCode::OK,
        [(header::CONTENT_TYPE, content_type)],
        Bytes::from(bytes),
    ))
}

async fn serve_thumbnail(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, AppError> {
    let relative = decode_id(&id)?;
    let source_path = safe_media_path(&state.photo_root, &relative)?;
    let thumb_path = thumbnail_path(&id);

    if !thumb_path.exists() {
        let source = source_path.clone();
        let target = thumb_path.clone();
        task::spawn_blocking(move || create_thumbnail(&source, &target))
            .await
            .map_err(|_| AppError::internal("No se pudo crear miniatura"))??;
    }

    let bytes = async_fs::read(thumb_path).await?;

    Ok((
        StatusCode::OK,
        [(header::CONTENT_TYPE, "image/jpeg")],
        Bytes::from(bytes),
    ))
}

fn create_thumbnail(source: &FsPath, target: &FsPath) -> Result<(), AppError> {
    let extension = source
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();

    if !matches!(
        extension.as_str(),
        "jpg" | "jpeg" | "png" | "gif" | "webp" | "bmp"
    ) {
        return Err(AppError::bad_request(
            "Miniatura no soportada para este formato",
        ));
    }

    let (image, _) = decode_oriented_image(source)?;
    let thumb = image.thumbnail(360, 360);

    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent)?;
    }

    thumb
        .save_with_format(target, image::ImageFormat::Jpeg)
        .map_err(|_| AppError::internal("No se pudo guardar la miniatura"))?;

    Ok(())
}

fn thumbnail_path(id: &str) -> PathBuf {
    PathBuf::from(THUMB_DIR).join(format!("oriented-{id}.jpg"))
}

fn is_thumbnail_supported_extension(extension: &str) -> bool {
    matches!(
        extension.to_ascii_lowercase().as_str(),
        "jpg" | "jpeg" | "png" | "gif" | "webp" | "bmp"
    )
}

fn decode_oriented_image(source: &FsPath) -> Result<(DynamicImage, Option<Vec<u8>>), AppError> {
    let reader = image::ImageReader::open(source)
        .map_err(|_| AppError::internal("No se pudo abrir la imagen"))?
        .with_guessed_format()
        .map_err(|_| AppError::internal("No se pudo detectar el formato"))?;

    let mut decoder = reader
        .into_decoder()
        .map_err(|_| AppError::internal("No se pudo leer la imagen"))?;
    let exif = decoder
        .exif_metadata()
        .map_err(|_| AppError::internal("No se pudo leer el EXIF"))?;
    let orientation = decoder
        .orientation()
        .map_err(|_| AppError::internal("No se pudo leer la orientacion"))?;
    let mut image = DynamicImage::from_decoder(decoder)
        .map_err(|_| AppError::internal("No se pudo decodificar la imagen"))?;

    image.apply_orientation(orientation);

    Ok((image, exif))
}

fn rotate_jpeg_original(source: &FsPath, direction: &str) -> Result<(), AppError> {
    let (image, mut exif) = decode_oriented_image(source)?;
    let rotated = match direction {
        "left" => image.rotate270(),
        "right" => image.rotate90(),
        _ => return Err(AppError::bad_request("Direccion de giro no valida")),
    };

    if let Some(exif) = exif.as_mut() {
        let _ = image::metadata::Orientation::remove_from_exif_chunk(exif);
    }

    let parent = source
        .parent()
        .ok_or_else(|| AppError::internal("Ruta de foto no valida"))?;
    let file_name = source
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| AppError::internal("Nombre de foto no valido"))?;
    let temp_path = parent.join(format!(".rotating-{file_name}"));
    let backup_path = parent.join(format!(".rotating-backup-{file_name}"));
    if temp_path.exists() || backup_path.exists() {
        return Err(AppError::internal(
            "Hay una rotacion temporal pendiente para esta foto",
        ));
    }

    let rgb = rotated.to_rgb8();
    let (width, height) = rgb.dimensions();

    {
        let file = fs::File::create(&temp_path)?;
        let mut encoder = JpegEncoder::new_with_quality(file, 92);
        if let Some(exif) = exif {
            encoder
                .set_exif_metadata(exif)
                .map_err(|_| AppError::internal("No se pudo conservar el EXIF"))?;
        }
        encoder
            .write_image(rgb.as_raw(), width, height, ExtendedColorType::Rgb8)
            .map_err(|_| AppError::internal("No se pudo guardar la foto girada"))?;
    }

    fs::rename(source, &backup_path)?;
    if let Err(error) = fs::rename(&temp_path, source) {
        let _ = fs::rename(&backup_path, source);
        let _ = fs::remove_file(&temp_path);
        return Err(error.into());
    }
    fs::remove_file(backup_path)?;

    Ok(())
}

fn scan_photos(root: &FsPath) -> Result<Vec<IndexedPhoto>, AppError> {
    if !root.exists() {
        return Err(AppError::not_found(format!(
            "No existe la carpeta {}",
            root.display()
        )));
    }

    let mut photos = Vec::new();
    scan_dir(root, root, &mut photos)?;
    photos.sort_by(|a, b| {
        b.captured_at
            .cmp(&a.captured_at)
            .then_with(|| b.modified_at.cmp(&a.modified_at))
    });
    Ok(photos)
}

fn scan_dir(
    root: &FsPath,
    current: &FsPath,
    photos: &mut Vec<IndexedPhoto>,
) -> Result<(), AppError> {
    for entry in fs::read_dir(current)? {
        let entry = entry?;
        let path = entry.path();
        let file_type = entry.file_type()?;

        if file_type.is_dir() {
            scan_dir(root, &path, photos)?;
            continue;
        }

        if !file_type.is_file() {
            continue;
        }

        let extension = path
            .extension()
            .and_then(|value| value.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase();

        if !is_supported_extension(&extension) {
            continue;
        }

        let relative = path
            .strip_prefix(root)
            .map_err(|_| AppError::internal("Ruta fuera de la carpeta de fotos"))?;

        let metadata = entry.metadata()?;
        let relative_path = relative.to_string_lossy().replace('\\', "/");
        let id = encode_id(&relative_path);
        let meta = read_photo_metadata(&path, &relative_path);

        photos.push(IndexedPhoto {
            media_url: format!("/media/{id}"),
            id,
            name: entry.file_name().to_string_lossy().to_string(),
            relative_path,
            extension,
            kind: media_kind(&path),
            size_bytes: metadata.len() as i64,
            modified_at: metadata
                .modified()
                .ok()
                .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
                .map(|duration| duration.as_secs() as i64),
            captured_at: meta.captured_at,
            date_bucket: meta.date_bucket,
            country: meta.country,
            city: meta.city,
            latitude: meta.latitude,
            longitude: meta.longitude,
            geo_status: meta.geo_status,
            favorite: meta.favorite,
            people: meta.people,
        });
    }

    Ok(())
}

struct PhotoMetadata {
    captured_at: Option<String>,
    date_bucket: Option<String>,
    country: Option<String>,
    city: Option<String>,
    latitude: Option<f64>,
    longitude: Option<f64>,
    geo_status: Option<String>,
    favorite: i64,
    people: Option<String>,
}

fn read_photo_metadata(path: &FsPath, relative_path: &str) -> PhotoMetadata {
    let mut captured_at = None;
    let mut latitude = None;
    let mut longitude = None;
    let mut favorite = false;

    if is_exif_readable(path) {
        if let Ok(file) = fs::File::open(path) {
            let mut reader = BufReader::new(file);
            if let Ok(exif) = Reader::new().read_from_container(&mut reader) {
                if let Some(field) = exif.get_field(Tag::DateTimeOriginal, In::PRIMARY) {
                    let value = field.display_value().with_unit(&exif).to_string();
                    captured_at = normalize_exif_datetime(&value);
                }

                if let Some((lat, lon)) = gps_from_exif(&exif) {
                    latitude = Some(lat);
                    longitude = Some(lon);
                }

                favorite = favorite || exif_looks_favorite(&exif);
            }
        }
    }

    favorite = favorite || metadata_text_looks_favorite(path);
    let people = metadata_text_people(path);

    PhotoMetadata {
        captured_at: captured_at.clone(),
        date_bucket: captured_at
            .as_ref()
            .map(|value| value.chars().take(7).collect())
            .or_else(|| date_bucket_from_path(relative_path)),
        country: None,
        city: None,
        latitude,
        longitude,
        geo_status: None,
        favorite: if favorite { 1 } else { 0 },
        people,
    }
}

fn photo_looks_favorite(path: &FsPath) -> bool {
    let mut favorite = false;

    if is_exif_readable(path) {
        if let Ok(file) = fs::File::open(path) {
            let mut reader = BufReader::new(file);
            if let Ok(exif) = Reader::new().read_from_container(&mut reader) {
                favorite = favorite || exif_looks_favorite(&exif);
            }
        }
    }

    favorite || metadata_text_looks_favorite(path)
}

fn exif_looks_favorite(exif: &exif::Exif) -> bool {
    for field in exif.fields() {
        let name = format!("{:?}", field.tag).to_ascii_lowercase();
        let value = field
            .display_value()
            .with_unit(exif)
            .to_string()
            .to_ascii_lowercase();

        if name.contains("rating") && !value.contains('0') {
            return true;
        }

        if name.contains("favorite") && (value.contains("true") || value.contains('1')) {
            return true;
        }
    }

    false
}

fn metadata_text_looks_favorite(path: &FsPath) -> bool {
    let Ok(file) = fs::File::open(path) else {
        return false;
    };

    let mut reader = BufReader::new(file);
    let mut buffer = Vec::new();
    if std::io::Read::by_ref(&mut reader)
        .take(1_000_000)
        .read_to_end(&mut buffer)
        .is_err()
    {
        return false;
    }

    let text = String::from_utf8_lossy(&buffer).to_ascii_lowercase();
    let positive_flags = [
        "favorite=\"1\"",
        "favorite=\"true\"",
        "favourite=\"1\"",
        "favourite=\"true\"",
        "rating=\"1\"",
        "rating=\"2\"",
        "rating=\"3\"",
        "rating=\"4\"",
        "rating=\"5\"",
        "<xmp:rating>1</xmp:rating>",
        "<xmp:rating>2</xmp:rating>",
        "<xmp:rating>3</xmp:rating>",
        "<xmp:rating>4</xmp:rating>",
        "<xmp:rating>5</xmp:rating>",
    ];

    positive_flags.iter().any(|flag| text.contains(flag))
}

fn metadata_text_people(path: &FsPath) -> Option<String> {
    let Ok(file) = fs::File::open(path) else {
        return None;
    };

    let mut reader = BufReader::new(file);
    let mut buffer = Vec::new();
    if reader
        .by_ref()
        .take(1_000_000)
        .read_to_end(&mut buffer)
        .is_err()
    {
        return None;
    }

    let text = String::from_utf8_lossy(&buffer);
    let mut names = Vec::new();

    for key in [
        "PersonInImage",
        "PersonInImageName",
        "RegionPersonDisplayName",
        "mwg-rs:Name",
        "MicrosoftPhoto:LastKeywordXMP",
        "dc:subject",
        "subject",
    ] {
        collect_metadata_values(&text, key, &mut names);
    }

    if names.is_empty() {
        None
    } else {
        Some(names.join(", "))
    }
}

fn collect_metadata_values(text: &str, key: &str, values: &mut Vec<String>) {
    for quote in ['"', '\''] {
        let pattern = format!("{key}={quote}");
        let mut offset = 0;
        while let Some(start) = text[offset..].find(&pattern) {
            let value_start = offset + start + pattern.len();
            let Some(end) = text[value_start..].find(quote) else {
                break;
            };
            add_metadata_name(&text[value_start..value_start + end], values);
            offset = value_start + end + 1;
        }
    }

    let open = format!("<{key}>");
    let close = format!("</{key}>");
    let mut offset = 0;
    while let Some(start) = text[offset..].find(&open) {
        let value_start = offset + start + open.len();
        let Some(end) = text[value_start..].find(&close) else {
            break;
        };
        add_metadata_name(&text[value_start..value_start + end], values);
        offset = value_start + end + close.len();
    }
}

fn add_metadata_name(value: &str, values: &mut Vec<String>) {
    let value = value
        .replace("&amp;", "&")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .trim()
        .to_string();

    if value.is_empty() || value.len() > 80 {
        return;
    }

    if !values
        .iter()
        .any(|existing| existing.eq_ignore_ascii_case(&value))
    {
        values.push(value);
    }
}

fn is_exif_readable(path: &FsPath) -> bool {
    matches!(
        path.extension()
            .and_then(|value| value.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase()
            .as_str(),
        "jpg" | "jpeg" | "tif" | "tiff"
    )
}

fn gps_from_exif(exif: &exif::Exif) -> Option<(f64, f64)> {
    let lat = exif.get_field(Tag::GPSLatitude, In::PRIMARY)?;
    let lat_ref = exif.get_field(Tag::GPSLatitudeRef, In::PRIMARY)?;
    let lon = exif.get_field(Tag::GPSLongitude, In::PRIMARY)?;
    let lon_ref = exif.get_field(Tag::GPSLongitudeRef, In::PRIMARY)?;

    let mut latitude = gps_value_to_decimal(&lat.value)?;
    let mut longitude = gps_value_to_decimal(&lon.value)?;

    if ascii_value(&lat_ref.value)?.eq_ignore_ascii_case("S") {
        latitude = -latitude;
    }

    if ascii_value(&lon_ref.value)?.eq_ignore_ascii_case("W") {
        longitude = -longitude;
    }

    Some((latitude, longitude))
}

fn gps_value_to_decimal(value: &Value) -> Option<f64> {
    let Value::Rational(values) = value else {
        return None;
    };

    if values.len() < 3 {
        return None;
    }

    Some(
        rational_to_f64(values[0])
            + rational_to_f64(values[1]) / 60.0
            + rational_to_f64(values[2]) / 3600.0,
    )
}

fn rational_to_f64(value: exif::Rational) -> f64 {
    if value.denom == 0 {
        return 0.0;
    }

    value.num as f64 / value.denom as f64
}

fn ascii_value(value: &Value) -> Option<String> {
    let Value::Ascii(values) = value else {
        return None;
    };

    values
        .first()
        .map(|bytes| String::from_utf8_lossy(bytes).trim().to_string())
}

fn normalize_exif_datetime(value: &str) -> Option<String> {
    let clean = value.trim().trim_matches('"');
    if clean.len() < 19 {
        return None;
    }

    let date = &clean[..10];
    let time = &clean[11..19];
    let mut date_parts = date.split(':');
    let year = date_parts.next()?;
    let month = date_parts.next()?;
    let day = date_parts.next()?;

    Some(format!("{year}-{month}-{day} {time}"))
}

fn date_bucket_from_path(relative_path: &str) -> Option<String> {
    let digits: String = relative_path
        .chars()
        .filter(|char| char.is_ascii_digit())
        .take(6)
        .collect();

    if digits.len() == 6 {
        Some(format!("{}-{}", &digits[0..4], &digits[4..6]))
    } else {
        None
    }
}

fn safe_media_path(root: &FsPath, relative: &str) -> Result<PathBuf, AppError> {
    let relative_path = PathBuf::from(relative);

    for component in relative_path.components() {
        if !matches!(component, Component::Normal(_)) {
            return Err(AppError::bad_request("Ruta no valida"));
        }
    }

    Ok(root.join(relative_path))
}

fn is_supported_extension(extension: &str) -> bool {
    matches!(
        extension,
        "jpg"
            | "jpeg"
            | "png"
            | "gif"
            | "webp"
            | "bmp"
            | "avif"
            | "heic"
            | "heif"
            | "mp4"
            | "mov"
            | "m4v"
    )
}

fn media_kind(path: &FsPath) -> String {
    match path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase()
        .as_str()
    {
        "mp4" | "mov" | "m4v" => "video".to_string(),
        "jpg" | "jpeg" | "png" | "gif" | "webp" | "bmp" | "avif" | "heic" | "heif" => {
            "image".to_string()
        }
        _ => "other".to_string(),
    }
}

fn content_type_for(extension: &str) -> &'static str {
    match extension {
        "jpg" | "jpeg" => "image/jpeg",
        "png" => "image/png",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "bmp" => "image/bmp",
        "avif" => "image/avif",
        "heic" => "image/heic",
        "heif" => "image/heif",
        "mp4" | "m4v" => "video/mp4",
        "mov" => "video/quicktime",
        _ => "application/octet-stream",
    }
}

fn encode_id(value: &str) -> String {
    value
        .as_bytes()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn decode_id(value: &str) -> Result<String, AppError> {
    if value.len() % 2 != 0 {
        return Err(AppError::bad_request("Identificador no valido"));
    }

    let mut bytes = Vec::with_capacity(value.len() / 2);
    for index in (0..value.len()).step_by(2) {
        let byte = u8::from_str_radix(&value[index..index + 2], 16)
            .map_err(|_| AppError::bad_request("Identificador no valido"))?;
        bytes.push(byte);
    }

    String::from_utf8(bytes).map_err(|_| AppError::bad_request("Identificador no valido"))
}

fn now_unix() -> i64 {
    UNIX_EPOCH
        .elapsed()
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or(0)
}

struct AppError {
    status: StatusCode,
    message: String,
}

impl AppError {
    fn bad_request(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            message: message.into(),
        }
    }

    fn not_found(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::NOT_FOUND,
            message: message.into(),
        }
    }

    fn internal(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            message: message.into(),
        }
    }
}

impl From<std::io::Error> for AppError {
    fn from(error: std::io::Error) -> Self {
        eprintln!("Error de archivos: {error}");
        Self::internal("Error leyendo las fotos")
    }
}

impl From<sqlx::Error> for AppError {
    fn from(error: sqlx::Error) -> Self {
        eprintln!("Error de base de datos: {error}");
        Self::internal("Error leyendo el indice")
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> axum::response::Response {
        (
            self.status,
            Json(serde_json::json!({ "error": self.message })),
        )
            .into_response()
    }
}
