use crate::{
    AppError, AppResult, AppState, access, commerce,
    models::{self, User},
    now, safety, user_id,
};
use axum::{
    Json, Router,
    extract::{Multipart, Path, Query, State},
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
    routing::{delete, get, patch, post},
};
use base64::{Engine, engine::general_purpose::STANDARD};
use rusqlite::{OptionalExtension, params};
use serde::Deserialize;
use serde_json::{Value, json};

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/config", get(config))
        .route("/api/ai/eligibility", get(ai_eligibility))
        .route("/api/profile", get(profile).put(update_profile))
        .route("/api/profile/avatar", post(upload_avatar).put(svg_avatar))
        .route("/api/avatars/{id}", get(avatar))
        .route("/api/role", post(role))
        .route("/api/jobs", get(jobs).post(create_job))
        .route("/api/jobs/mine", get(my_jobs))
        .route("/api/my/jobs/trash", get(trash))
        .route("/api/jobs/{id}", get(job).delete(delete_job))
        .route("/api/jobs/{id}/edit", post(edit_job))
        .route("/api/jobs/{id}/headcount", patch(update_headcount))
        .route("/api/jobs/{id}/close", post(close_job))
        .route("/api/jobs/{id}/restore", post(restore_job))
        .route("/api/jobs/{id}/permanent", delete(permanently_delete_job))
        .route("/api/people", get(people))
        .route("/api/people/{id}", get(person))
        .route("/api/orders", get(orders))
        .route("/api/orders/{id}", get(order))
        .route("/api/orders/{id}/checkout", post(checkout))
        .route("/api/tokens", get(tokens).post(create_token))
        .route("/api/tokens/{id}", delete(revoke_token))
        .route("/api/markdown", post(markdown))
        .route("/api/matches", get(matches))
}
async fn config(State(s): State<AppState>) -> Json<Value> {
    Json(public_config(&s))
}
pub fn public_config(s: &AppState) -> Value {
    let mut providers = vec![];
    if s.cfg.payments.stripe.enabled {
        providers.push("stripe");
    }
    if s.cfg.payments.waffo.enabled {
        providers.push("waffo");
    }
    json!({"name":s.cfg.site.name,"public_origin":s.cfg.public_url,"default_language":s.cfg.site.default_language,"support_email":s.cfg.site.support_email,"source_url":s.cfg.site.source_url,"google_enabled":s.cfg.google.enabled,"humangrant_enabled":s.cfg.humangrant.enabled,"email_enabled":s.cfg.smtp.enabled||s.cfg.development,"development":s.cfg.development,"providers":providers,"payment_currencies":s.cfg.payments.public_currencies(),"payment_modes":{"waffo":s.cfg.payments.waffo.mode,"stripe":if s.cfg.payments.stripe.live_mode {"prod"} else {"test"}},"waffo_regions":s.cfg.payments.waffo.checkout_regions,"pricing":s.cfg.pricing.public(),"limits":s.cfg.limits})
}
async fn profile(State(s): State<AppState>, h: HeaderMap) -> AppResult<Json<User>> {
    let id = user_id(&s, &h, false)?;
    Ok(Json(models::load_user(&*s.conn()?, &id)?))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProfileInput {
    pub name: String,
    pub public_email: String,
    pub resume_md: String,
    #[serde(default)]
    pub visible: bool,
}
pub fn save_profile(s: &AppState, id: &str, input: ProfileInput) -> AppResult<User> {
    safety::bounded_text(&input.name, 120, "Name")?;
    safety::bounded_text(&input.resume_md, 80_000, "Resume")?;
    let email = if input.public_email.trim().is_empty() {
        String::new()
    } else {
        safety::email(&input.public_email)?
    };
    let db = s.conn()?;
    let user = models::load_user(&db, id)?;
    if user.banned {
        return Err(AppError::forbidden("Account is suspended"));
    }
    if user.role == "unassigned" {
        return Err(AppError::conflict("Choose your account type first"));
    }
    if user.role != "seeker" && !input.resume_md.is_empty() {
        return Err(AppError::bad_request("Only seekers have a resume"));
    }
    // Retain the legacy input field for compatibility, but it never grants
    // discovery access. Only the platform and explicit matches see a profile.
    db.execute(
        "UPDATE users SET name=?2,public_email=?3,resume_md=?4,visible=0,updated_at=?5 WHERE id=?1",
        params![id, input.name.trim(), email, input.resume_md, now()],
    )?;
    crate::db::audit(&db, id, "profile_update", id, "")?;
    models::load_user(&db, id)
}
async fn update_profile(
    State(s): State<AppState>,
    h: HeaderMap,
    Json(input): Json<ProfileInput>,
) -> AppResult<Json<User>> {
    let id = user_id(&s, &h, true)?;
    Ok(Json(save_profile(&s, &id, input)?))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SvgInput {
    svg: String,
}
pub const MAX_AVATAR_IMAGE_BYTES: usize = 2 * 1024 * 1024;
pub const MAX_AVATAR_BASE64_LEN: usize = MAX_AVATAR_IMAGE_BYTES.div_ceil(3) * 4;
fn save_avatar(s: &AppState, id: &str, raw: &str) -> AppResult<User> {
    // Image decoding and multipart reads happen before this transaction. Recheck the
    // account while holding the write lock so a concurrent ban cannot be bypassed.
    let mut db = s.conn()?;
    let tx = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    let user = models::load_user(&tx, id)?;
    if user.banned {
        return Err(AppError::forbidden("Account is suspended"));
    }
    tx.execute(
        "UPDATE users SET avatar=?2,updated_at=?3 WHERE id=?1 AND banned=0",
        params![id, raw, now()],
    )?;
    crate::db::audit(&tx, id, "avatar_update", id, "")?;
    let user = models::load_user(&tx, id)?;
    tx.commit()?;
    Ok(user)
}
pub fn save_svg_avatar(s: &AppState, id: &str, svg: &str) -> AppResult<User> {
    let svg = safety::safe_svg(svg)?;
    save_avatar(s, id, &format!("svg:{svg}"))
}
pub fn save_image_avatar(s: &AppState, id: &str, bytes: &[u8]) -> AppResult<User> {
    if bytes.len() > MAX_AVATAR_IMAGE_BYTES {
        return Err(AppError::bad_request("Image must be at most 2 MiB"));
    }
    let reader = image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .map_err(AppError::bad_request)?;
    if !matches!(
        reader.format(),
        Some(image::ImageFormat::Png | image::ImageFormat::Jpeg | image::ImageFormat::WebP)
    ) {
        return Err(AppError::bad_request("Use a PNG, JPEG or WebP image"));
    }
    let (w, h) = reader
        .into_dimensions()
        .map_err(|_| AppError::bad_request("Use a PNG, JPEG or WebP image"))?;
    if w == 0 || h == 0 || w > 2048 || h > 2048 {
        return Err(AppError::bad_request(
            "Image dimensions must be 1–2048 pixels",
        ));
    }
    // Decode into pixels and re-encode to remove metadata, appended content and any
    // source encoding that would otherwise reach another user's browser.
    let image = image::load_from_memory(bytes)
        .map_err(|_| AppError::bad_request("Could not decode image"))?
        .thumbnail(256, 256);
    let mut output = std::io::Cursor::new(Vec::new());
    image
        .write_to(&mut output, image::ImageFormat::Png)
        .map_err(AppError::internal)?;
    save_avatar(
        s,
        id,
        &format!("png:{}", STANDARD.encode(output.into_inner())),
    )
}
async fn svg_avatar(
    State(s): State<AppState>,
    h: HeaderMap,
    Json(input): Json<SvgInput>,
) -> AppResult<Json<User>> {
    let id = user_id(&s, &h, true)?;
    Ok(Json(save_svg_avatar(&s, &id, &input.svg)?))
}
async fn upload_avatar(
    State(s): State<AppState>,
    h: HeaderMap,
    mut multipart: Multipart,
) -> AppResult<Json<User>> {
    user_id(&s, &h, true)?;
    let field = multipart
        .next_field()
        .await
        .map_err(|_| AppError::bad_request("Invalid upload"))?
        .ok_or_else(|| AppError::bad_request("Select an image"))?;
    let bytes = field
        .bytes()
        .await
        .map_err(|_| AppError::bad_request("Could not read image"))?;
    let id = user_id(&s, &h, true)?;
    Ok(Json(save_image_avatar(&s, &id, &bytes)?))
}
async fn avatar(
    State(s): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> AppResult<Response> {
    let principal = access::principal(&s, &headers, false).ok();
    let db = s.conn()?;
    let user = models::load_user(&db, &id).map_err(|e| {
        if e.status == StatusCode::NOT_FOUND {
            AppError::not_found("Avatar not found")
        } else {
            e
        }
    })?;
    let allowed = if let Some(p) = principal {
        access::ensure_active(&db, &p)?;
        p.admin
            || (!user.banned && p.user_id.as_deref().is_some_and(|v| v == id))
            || (!user.banned
                && p.user_id
                    .as_deref()
                    .map(|v| access::matched_pair(&db, v, &id))
                    .transpose()?
                    .unwrap_or(false))
    } else {
        false
    };
    if !allowed {
        return Err(AppError::not_found("Avatar not found"));
    }
    let raw: String = db.query_row("SELECT avatar FROM users WHERE id=?1", [id], |r| r.get(0))?;
    if let Some(svg) = raw.strip_prefix("svg:") {
        Ok((
            [
                (header::CONTENT_TYPE, "image/svg+xml"),
                (header::CACHE_CONTROL, "private, no-store"),
            ],
            svg.to_string(),
        )
            .into_response())
    } else if let Some(png) = raw.strip_prefix("png:") {
        Ok((
            [
                (header::CONTENT_TYPE, "image/png"),
                (header::CACHE_CONTROL, "private, no-store"),
            ],
            STANDARD.decode(png).map_err(AppError::internal)?,
        )
            .into_response())
    } else {
        Err(AppError::not_found("Avatar not found"))
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RoleInput {
    pub role: String,
    pub provider: Option<String>,
    pub currency: Option<String>,
}
async fn role(
    State(s): State<AppState>,
    h: HeaderMap,
    Json(i): Json<RoleInput>,
) -> AppResult<Json<Value>> {
    let uid = user_id(&s, &h, true)?;
    let (user, order) = commerce::choose_role_with_currency(
        &s,
        &uid,
        &i.role,
        i.provider.as_deref(),
        i.currency.as_deref(),
    )?;
    Ok(Json(json!({"user":user,"order":order})))
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct ListQuery {
    pub q: Option<String>,
    pub page: Option<i64>,
}
pub fn pagination(s: &AppState, q: &ListQuery) -> (i64, i64) {
    let limit = s.cfg.limits.list_page_size;
    (
        limit,
        (q.page.unwrap_or(1).clamp(1, s.cfg.limits.list_max_page) - 1) * limit,
    )
}
pub fn list_jobs(s: &AppState, q: &ListQuery, owner: Option<&str>) -> AppResult<Value> {
    let owner = owner.ok_or_else(|| AppError::unauthorized("Sign in required"))?;
    query_jobs(s, q, owner, true)
}
pub fn matched_jobs(s: &AppState, q: &ListQuery, viewer: &str) -> AppResult<Value> {
    query_jobs(s, q, viewer, false)
}
fn query_jobs(s: &AppState, q: &ListQuery, viewer: &str, mine: bool) -> AppResult<Value> {
    let db = s.conn()?;
    let user = models::load_user(&db, viewer)?;
    if user.banned {
        return Err(AppError::forbidden("Account is suspended"));
    }
    let (limit, offset) = pagination(s, q);
    let search = format!(
        "%{}%",
        q.q.as_deref()
            .unwrap_or("")
            .chars()
            .take(100)
            .collect::<String>()
    );
    let clause = if mine {
        "j.owner_id=?1 AND j.status<>'deleted' AND u.banned=0 AND (j.title LIKE ?2 OR j.jd_md LIKE ?2)"
    } else {
        "j.status<>'deleted' AND u.banned=0 AND EXISTS(SELECT 1 FROM matches m WHERE m.job_id=j.id AND m.seeker_id=?1 AND m.status IN ('introduced','interview','hired')) AND (j.title LIKE ?2 OR j.jd_md LIKE ?2)"
    };
    let sql = format!(
        "SELECT j.* FROM jobs j JOIN users u ON j.owner_id=u.id WHERE {clause} ORDER BY j.created_at DESC,j.id LIMIT ?3 OFFSET ?4"
    );
    let items = db
        .prepare(&sql)?
        .query_map(params![viewer, search, limit, offset], models::job_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let total: i64 = db.query_row(
        &format!("SELECT count(*) FROM jobs j JOIN users u ON j.owner_id=u.id WHERE {clause}"),
        params![viewer, search],
        |r| r.get(0),
    )?;
    Ok(json!({"items":items,"total":total,"limit":limit,"page":offset/limit+1}))
}
async fn jobs(
    State(s): State<AppState>,
    h: HeaderMap,
    Query(q): Query<ListQuery>,
) -> AppResult<Json<Value>> {
    let uid = user_id(&s, &h, false)?;
    Ok(Json(matched_jobs(&s, &q, &uid)?))
}
async fn my_jobs(
    State(s): State<AppState>,
    h: HeaderMap,
    Query(q): Query<ListQuery>,
) -> AppResult<Json<Value>> {
    let uid = user_id(&s, &h, false)?;
    Ok(Json(list_jobs(&s, &q, Some(&uid))?))
}
pub fn deleted_jobs(s: &AppState, uid: &str) -> AppResult<Value> {
    let db = s.conn()?;
    let user = models::load_user(&db, uid)?;
    if user.banned || user.role != "employer" {
        return Err(AppError::forbidden(
            "An active employer account is required",
        ));
    }
    let items = db
        .prepare("SELECT * FROM jobs WHERE owner_id=?1 AND status='deleted' AND delete_count=1 ORDER BY updated_at DESC,id LIMIT ?2")?
        .query_map(params![uid, s.cfg.limits.recent_list_limit], models::job_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(json!({"items":items}))
}
async fn trash(State(s): State<AppState>, h: HeaderMap) -> AppResult<Json<Value>> {
    let uid = user_id(&s, &h, false)?;
    Ok(Json(deleted_jobs(&s, &uid)?))
}
pub fn job_detail(s: &AppState, id: &str, viewer: Option<&str>) -> AppResult<Value> {
    let db = s.conn()?;
    let job = models::load_job(&db, id)?;
    if job.status == "deleted" && job.delete_count >= 2 {
        return Err(AppError::not_found("Job not found"));
    }
    let owner = models::load_user(&db, &job.owner_id)?;
    if !viewer
        .map(|v| access::can_read_job(&db, v, id))
        .transpose()?
        .unwrap_or(false)
    {
        return Err(AppError::not_found("Job not found"));
    }
    let terms:Option<String>=db.query_row("SELECT snapshot_json FROM orders WHERE job_id=?1 AND kind='publish' ORDER BY created_at LIMIT 1",[&job.id],|r|r.get(0)).optional()?;
    let snapshot: Value = terms
        .as_deref()
        .and_then(|v| serde_json::from_str(v).ok())
        .unwrap_or(json!({}));
    Ok(
        json!({"jd_html":safety::markdown(&job.jd_md),"job":job,"terms":{"edit_bps":snapshot["edit_bps"].as_i64().unwrap_or(100),"max_edits":snapshot["max_edits"].as_i64().unwrap_or(3),"edit_mode":snapshot["edit_mode"].as_str().unwrap_or("salary_percent"),"edit_fixed_minor":snapshot["edit_fixed_minor"].as_i64().unwrap_or(0),"currency":job.currency,"currency_decimals":job.currency_decimals,"employer_ai_min_salary_minor":job.ai_min_salary_cents},"employer":{"id":owner.id,"name":owner.name,"public_email":owner.public_email,"avatar":owner.avatar}}),
    )
}
async fn job(
    State(s): State<AppState>,
    h: HeaderMap,
    Path(id): Path<String>,
) -> AppResult<Json<Value>> {
    let viewer = user_id(&s, &h, false).ok();
    Ok(Json(job_detail(&s, &id, viewer.as_deref())?))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JobInput {
    pub title: String,
    pub jd_md: String,
    pub salary_cents: i64,
    #[serde(default)]
    pub provider: String,
    #[serde(default)]
    pub currency: Option<String>,
    #[serde(default = "default_headcount")]
    pub headcount: i64,
}
fn default_headcount() -> i64 {
    1
}
async fn create_job(
    State(s): State<AppState>,
    h: HeaderMap,
    Json(i): Json<JobInput>,
) -> AppResult<Json<Value>> {
    let uid = user_id(&s, &h, true)?;
    let (job, order) = commerce::new_job_with_currency(
        &s,
        &uid,
        &i.title,
        &i.jd_md,
        i.salary_cents,
        &i.provider,
        i.headcount,
        i.currency.as_deref(),
    )?;
    Ok(Json(json!({"job":job,"order":order})))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct HeadcountInput {
    headcount: i64,
}
async fn update_headcount(
    State(s): State<AppState>,
    h: HeaderMap,
    Path(id): Path<String>,
    Json(input): Json<HeadcountInput>,
) -> AppResult<Json<models::Job>> {
    let uid = user_id(&s, &h, true)?;
    Ok(Json(commerce::set_headcount(
        &s,
        &uid,
        &id,
        input.headcount,
    )?))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EditInput {
    pub jd_md: String,
    #[serde(default)]
    pub provider: String,
}
async fn edit_job(
    State(s): State<AppState>,
    h: HeaderMap,
    Path(id): Path<String>,
    Json(i): Json<EditInput>,
) -> AppResult<Json<Value>> {
    let uid = user_id(&s, &h, true)?;
    let (job, order) = commerce::edit_job(&s, &uid, &id, &i.jd_md, &i.provider)?;
    Ok(Json(json!({"job":job,"order":order})))
}
async fn close_job(
    State(s): State<AppState>,
    h: HeaderMap,
    Path(id): Path<String>,
) -> AppResult<Json<models::Job>> {
    let uid = user_id(&s, &h, true)?;
    Ok(Json(commerce::close_job(&s, &uid, &id)?))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JobVersionInput {
    pub expected_version: i64,
}
async fn delete_job(
    State(s): State<AppState>,
    h: HeaderMap,
    Path(id): Path<String>,
    Json(input): Json<JobVersionInput>,
) -> AppResult<Json<models::Job>> {
    let uid = user_id(&s, &h, true)?;
    Ok(Json(commerce::delete_job(
        &s,
        &uid,
        &id,
        input.expected_version,
    )?))
}
async fn restore_job(
    State(s): State<AppState>,
    h: HeaderMap,
    Path(id): Path<String>,
    Json(input): Json<JobVersionInput>,
) -> AppResult<Json<models::Job>> {
    let uid = user_id(&s, &h, true)?;
    Ok(Json(commerce::restore_job(
        &s,
        &uid,
        &id,
        input.expected_version,
    )?))
}
async fn permanently_delete_job(
    State(s): State<AppState>,
    h: HeaderMap,
    Path(id): Path<String>,
    Json(input): Json<JobVersionInput>,
) -> AppResult<Json<models::Job>> {
    let uid = user_id(&s, &h, true)?;
    Ok(Json(commerce::permanently_delete_job(
        &s,
        &uid,
        &id,
        input.expected_version,
    )?))
}

pub fn public_user(user: User) -> Value {
    json!({"id":user.id,"name":user.name,"avatar":user.avatar,"public_email":user.public_email,"resume_md":user.resume_md,"created_at":user.created_at})
}
pub fn list_people(s: &AppState, q: &ListQuery, viewer: &str) -> AppResult<Value> {
    let db = s.conn()?;
    if models::load_user(&db, viewer)?.banned {
        return Err(AppError::forbidden("Account is suspended"));
    }
    let (limit, offset) = pagination(s, q);
    let search = format!(
        "%{}%",
        q.q.as_deref()
            .unwrap_or("")
            .chars()
            .take(100)
            .collect::<String>()
    );
    let condition = "role='seeker' AND banned=0 AND (name LIKE ?1 OR resume_md LIKE ?1) AND EXISTS(SELECT 1 FROM matches m JOIN jobs j ON j.id=m.job_id JOIN users e ON e.id=j.owner_id WHERE m.seeker_id=users.id AND j.owner_id=?4 AND j.status<>'deleted' AND e.banned=0 AND m.status IN ('introduced','interview','hired'))";
    let users = db
        .prepare(&format!(
            "SELECT * FROM users WHERE {condition} ORDER BY updated_at DESC,id LIMIT ?2 OFFSET ?3"
        ))?
        .query_map(params![search, limit, offset, viewer], models::user_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let total: i64 = db.query_row(
        &format!("SELECT count(*) FROM users WHERE {condition}"),
        params![search, limit, offset, viewer],
        |r| r.get(0),
    )?;
    Ok(
        json!({"items":users.into_iter().map(public_user).collect::<Vec<_>>(),"total":total,"limit":limit,"page":offset/limit+1}),
    )
}
async fn people(
    State(s): State<AppState>,
    h: HeaderMap,
    Query(q): Query<ListQuery>,
) -> AppResult<Json<Value>> {
    let uid = user_id(&s, &h, false)?;
    Ok(Json(list_people(&s, &q, &uid)?))
}
pub fn person_detail(s: &AppState, id: &str, viewer: Option<&str>) -> AppResult<Value> {
    let db = s.conn()?;
    let u = models::load_user(&db, id).map_err(|e| {
        if e.status == StatusCode::NOT_FOUND {
            AppError::not_found("Profile not found")
        } else {
            e
        }
    })?;
    let allowed = viewer
        .map(|v| {
            if v == id {
                Ok(true)
            } else {
                access::matched_pair(&db, v, id)
            }
        })
        .transpose()?
        .unwrap_or(false);
    if !allowed || u.banned || u.role != "seeker" {
        return Err(AppError::not_found("Profile not found"));
    }
    Ok(json!({"resume_html":safety::markdown(&u.resume_md),"user":public_user(u)}))
}
async fn person(
    State(s): State<AppState>,
    h: HeaderMap,
    Path(id): Path<String>,
) -> AppResult<Json<Value>> {
    let viewer = user_id(&s, &h, false).ok();
    Ok(Json(person_detail(&s, &id, viewer.as_deref())?))
}
pub fn user_orders(s: &AppState, id: &str) -> AppResult<Vec<models::Order>> {
    let db = s.conn()?;
    let mut orders = db
        .prepare("SELECT * FROM orders WHERE user_id=?1 ORDER BY created_at DESC,id LIMIT ?2")?
        .query_map(
            params![id, s.cfg.limits.recent_list_limit],
            models::order_row,
        )?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    for order in &mut orders {
        hide_unavailable_checkout(s, &db, order)?;
    }
    Ok(orders)
}
fn hide_unavailable_checkout(
    s: &AppState,
    db: &rusqlite::Connection,
    order: &mut models::Order,
) -> AppResult<()> {
    if order.status != "pending"
        || !s
            .cfg
            .payments
            .permits_currency(&order.provider, &order.currency)
        || !commerce::job_checkout_available(db, order)?
    {
        order.checkout_url = None;
    }
    Ok(())
}
pub fn user_order(s: &AppState, uid: &str, id: &str) -> AppResult<models::Order> {
    let db = s.conn()?;
    let mut order = models::load_order(&db, id)?;
    if order.user_id != uid {
        return Err(AppError::not_found("Order not found"));
    }
    hide_unavailable_checkout(s, &db, &mut order)?;
    Ok(order)
}
async fn orders(State(s): State<AppState>, h: HeaderMap) -> AppResult<Json<Value>> {
    let uid = user_id(&s, &h, false)?;
    Ok(Json(json!({"items":user_orders(&s,&uid)?})))
}
async fn order(
    State(s): State<AppState>,
    h: HeaderMap,
    Path(id): Path<String>,
) -> AppResult<Json<models::Order>> {
    let uid = user_id(&s, &h, false)?;
    Ok(Json(user_order(&s, &uid, &id)?))
}
#[derive(Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
struct CheckoutInput {
    payment_region: Option<String>,
}
async fn checkout(
    State(s): State<AppState>,
    h: HeaderMap,
    Path(id): Path<String>,
    input: Option<Json<CheckoutInput>>,
) -> AppResult<Json<models::Order>> {
    let uid = user_id(&s, &h, true)?;
    let input = input.map(|Json(value)| value).unwrap_or_default();
    Ok(Json(
        commerce::checkout_with_region(&s, &uid, &id, input.payment_region.as_deref()).await?,
    ))
}

async fn tokens(State(s): State<AppState>, h: HeaderMap) -> AppResult<Json<Value>> {
    let uid = user_id(&s, &h, false)?;
    let db = s.conn()?;
    let items=db.prepare("SELECT id,name,created_at,expires_at,last_used_at FROM api_tokens WHERE user_id=?1 AND revoked=0 ORDER BY created_at DESC")?.query_map([uid],|r|Ok(json!({"id":r.get::<_,String>(0)?,"name":r.get::<_,String>(1)?,"created_at":r.get::<_,i64>(2)?,"expires_at":r.get::<_,i64>(3)?,"last_used_at":r.get::<_,Option<i64>>(4)?})))?.collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(Json(json!({"items":items})))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TokenInput {
    name: String,
}
async fn create_token(
    State(s): State<AppState>,
    h: HeaderMap,
    Json(i): Json<TokenInput>,
) -> AppResult<Json<Value>> {
    if h.contains_key("authorization") {
        return Err(AppError::forbidden(
            "Create agent tokens from an interactive signed-in session",
        ));
    }
    let uid = user_id(&s, &h, true)?;
    safety::bounded_text(&i.name, 80, "Token name")?;
    if i.name.trim().is_empty() {
        return Err(AppError::bad_request("Token name is required"));
    }
    use rand::RngCore;
    let mut bytes = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    let token = format!("hf_{}", hex::encode(bytes));
    let id = uuid::Uuid::new_v4().to_string();
    let expires = now() + 30 * 24 * 3600;
    let db = s.conn()?;
    access::require_user_ai(&db, &uid)?;
    let count: i64 = db.query_row(
        "SELECT count(*) FROM api_tokens WHERE user_id=?1 AND revoked=0 AND expires_at>?2",
        params![uid, now()],
        |r| r.get(0),
    )?;
    if count >= 10 {
        return Err(AppError::conflict("At most 10 active agent tokens"));
    }
    db.execute("INSERT INTO api_tokens(id,user_id,name,token_hash,created_at,expires_at) VALUES(?1,?2,?3,?4,?5,?6)",params![id,uid,i.name,crate::auth::hash_token(&token),now(),expires])?;
    crate::db::audit(&db, &uid, "token_create", &id, "")?;
    Ok(Json(json!({"id":id,"token":token,"expires_at":expires})))
}
async fn revoke_token(
    State(s): State<AppState>,
    h: HeaderMap,
    Path(id): Path<String>,
) -> AppResult<StatusCode> {
    let uid = user_id(&s, &h, true)?;
    let mut db = s.conn()?;
    let tx = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    let changed = tx.execute(
        "UPDATE api_tokens SET revoked=1 WHERE id=?1 AND user_id=?2",
        params![id, uid],
    )?;
    if changed == 1 {
        crate::cli_auth::revoke_token_family_by_id(&tx, &id, false)?;
    }
    crate::db::audit(&tx, &uid, "token_revoke", &id, "")?;
    tx.commit()?;
    Ok(StatusCode::NO_CONTENT)
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MarkdownInput {
    text: String,
}
async fn markdown(
    State(s): State<AppState>,
    h: HeaderMap,
    Json(i): Json<MarkdownInput>,
) -> AppResult<Json<Value>> {
    access::principal(&s, &h, true)?;
    safety::bounded_text(&i.text, 80_000, "Markdown")?;
    Ok(Json(json!({"html":safety::markdown(&i.text)})))
}
pub fn list_matches(s: &AppState, uid: Option<&str>) -> AppResult<Value> {
    let db = s.conn()?;
    let uid = uid.ok_or_else(|| AppError::unauthorized("Sign in required"))?;
    if models::load_user(&db, uid)?.banned {
        return Err(AppError::forbidden("Account is suspended"));
    }
    let sql = "SELECT m.*,s.name AS seeker_name,j.title AS job_title,j.owner_id AS employer_id,e.name AS employer_name FROM matches m JOIN users s ON s.id=m.seeker_id JOIN jobs j ON j.id=m.job_id JOIN users e ON e.id=j.owner_id WHERE (m.seeker_id=?1 OR j.owner_id=?1) AND m.status IN ('introduced','interview','hired') AND s.banned=0 AND e.banned=0 AND j.status<>'deleted' ORDER BY m.updated_at DESC LIMIT ?2";
    let items=db.prepare(sql)?.query_map(params![uid,s.cfg.limits.recent_list_limit],|r|Ok(json!({"id":r.get::<_,String>("id")?,"seeker_id":r.get::<_,String>("seeker_id")?,"seeker_name":r.get::<_,String>("seeker_name")?,"job_id":r.get::<_,String>("job_id")?,"job_title":r.get::<_,String>("job_title")?,"employer_id":r.get::<_,String>("employer_id")?,"employer_name":r.get::<_,String>("employer_name")?,"status":r.get::<_,String>("status")?,"note":r.get::<_,String>("note")?,"created_at":r.get::<_,i64>("created_at")?,"updated_at":r.get::<_,i64>("updated_at")?})))?.collect::<rusqlite::Result<Vec<_>>>()?;
    let items = items
        .into_iter()
        .map(|mut m| {
            m["conversation_id"] = json!(format!("match_{}", m["id"].as_str().unwrap_or("")));
            m
        })
        .collect::<Vec<_>>();
    Ok(json!({"items":items}))
}
async fn matches(State(s): State<AppState>, h: HeaderMap) -> AppResult<Json<Value>> {
    let uid = user_id(&s, &h, false)?;
    Ok(Json(list_matches(&s, Some(&uid))?))
}

async fn ai_eligibility(State(s): State<AppState>, h: HeaderMap) -> AppResult<Json<Value>> {
    let p = access::principal(&s, &h, false)?;
    if p.admin {
        return Ok(Json(
            json!({"eligible":true,"reason":"platform_admin","role":"admin","min_salary_cents":s.cfg.pricing.employer_ai_min_salary_minor,"currency":s.cfg.pricing.currency,"currency_decimals":crate::config::currency_decimals(&s.cfg.pricing.currency)}),
        ));
    }
    let mut value = access::eligibility(&*s.conn()?, p.user_id.as_deref().unwrap())?;
    value["min_salary_cents"] = json!(s.cfg.pricing.employer_ai_min_salary_minor);
    value["currency"] = json!(s.cfg.pricing.currency);
    value["currency_decimals"] = json!(crate::config::currency_decimals(&s.cfg.pricing.currency));
    value["currency_thresholds"] = s.cfg.pricing.public()["currency_options"].clone();
    Ok(Json(value))
}
