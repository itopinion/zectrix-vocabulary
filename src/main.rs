mod hub;
mod speech;
use anyhow::{bail, Context, Result};
use axum::{
    body::Bytes,
    extract::{DefaultBodyLimit, Path as AxPath, Request, State},
    http::{header, HeaderMap, StatusCode},
    middleware::{self, Next},
    response::{Html, IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    time::Duration,
};
use tokio::sync::{watch, Mutex, RwLock};
use zectrix_vocabulary::*;
struct Model {
    library: Library,
    quiz: Quiz,
    recording_epoch: Option<u64>,
}
#[derive(Clone, Serialize, Deserialize, Debug, PartialEq)]
struct Settings {
    volume: u8,
}
impl Default for Settings {
    fn default() -> Self {
        Self { volume: 65 }
    }
}
impl Settings {
    fn validate(&self) -> Result<()> {
        if self.volume > 100 {
            bail!("音量须为 0–100")
        }
        Ok(())
    }
    fn load(root: &Path) -> Result<Self> {
        let p = root.join("data/settings.json");
        let settings: Self = if p.exists() {
            serde_json::from_slice(&std::fs::read(p)?)?
        } else {
            Self::default()
        };
        settings.validate()?;
        Ok(settings)
    }
    fn save(&self, root: &Path) -> Result<()> {
        self.validate()?;
        use std::io::Write;
        let tmp = root.join("data/settings.pending");
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(&serde_json::to_vec_pretty(self)?)?;
        f.sync_all()?;
        std::fs::rename(tmp, root.join("data/settings.json"))?;
        Ok(())
    }
}
async fn settings(State(a): State<Shared>, Json(next): Json<Settings>) -> Api<Settings> {
    let mut current = a.settings.lock().await;
    next.save(&a.root)?;
    *current = next.clone();
    Ok(Json(next))
}
struct App {
    settings: Mutex<Settings>,
    root: PathBuf,
    token: String,
    port: u16,
    model: Mutex<Model>,
    hub: hub::Hub,
    runtime: RwLock<hub::Runtime>,
    asr: speech::Speech,
    tts: speech::Speech,
    models: RwLock<Value>,
    png: RwLock<Vec<u8>>,
    frame_revision: AtomicU64,
    draw: watch::Sender<u64>,
    audio_gate: Mutex<()>,
    synth_gate: Mutex<()>,
}
type Shared = Arc<App>;
struct ApiError(anyhow::Error);
impl<E: Into<anyhow::Error>> From<E> for ApiError {
    fn from(e: E) -> Self {
        Self(e.into())
    }
}
impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (
            StatusCode::BAD_REQUEST,
            Json(json!({"error":self.0.to_string()})),
        )
            .into_response()
    }
}
type Api<T> = std::result::Result<Json<T>, ApiError>;
fn save(root: &Path, db: &Library) -> Result<()> {
    let path = root.join("data/library.json");
    let temp = root.join("data/library.pending");
    let bytes = serde_json::to_vec_pretty(db)?;
    use std::io::Write;
    let mut f = std::fs::File::create(&temp)?;
    f.write_all(&bytes)?;
    f.sync_all()?;
    if path.exists() {
        std::fs::copy(&path, root.join("data/library.previous.json"))?;
    }
    std::fs::rename(temp, path)?;
    Ok(())
}
fn seed() -> Library {
    let words = [
        ("apple", "苹果"),
        ("friend", "朋友"),
        ("because", "因为"),
        ("beautiful", "美丽的"),
        ("Wednesday", "星期三"),
        ("vegetable", "蔬菜"),
    ]
    .into_iter()
    .map(|(en, zh)| Word {
        id: id(),
        english: en.into(),
        chinese: zh.into(),
        example: String::new(),
        phonetic: String::new(),
    })
    .collect();
    Library {
        revision: 1,
        books: vec![
            Book {
                id: id(),
                title: "人教版英语".into(),
                edition: "修订版待核对".into(),
                grade: "八年级".into(),
                volume: "上册".into(),
                source: "待导入本人教材词表；未预填未经核对的教材内容".into(),
                units: vec![Unit {
                    id: id(),
                    title: "Unit 1".into(),
                    words: vec![],
                }],
            },
            Book {
                id: id(),
                title: "设备体验词库".into(),
                edition: "演示词库 · 非教材词表".into(),
                grade: "通用".into(),
                volume: "体验".into(),
                source: "用于试听和体验拼写、朗读流程".into(),
                units: vec![Unit {
                    id: id(),
                    title: "开始体验 · 6 个单词".into(),
                    words,
                }],
            },
        ],
        history: vec![],
    }
}
fn changed(a: &App) {
    a.draw.send_modify(|n| *n += 1);
}
async fn secure(State(a): State<Shared>, req: Request, next: Next) -> Response {
    let expected = format!("127.0.0.1:{}", a.port);
    let local = format!("localhost:{}", a.port);
    let host = req
        .headers()
        .get("host")
        .and_then(|h| h.to_str().ok())
        .unwrap_or("");
    if host != expected && host != local {
        return (StatusCode::FORBIDDEN, "只允许本机访问").into_response();
    }
    if let Some(origin) = req.headers().get("origin").and_then(|h| h.to_str().ok()) {
        if origin != format!("http://{expected}") && origin != format!("http://{local}") {
            return (StatusCode::FORBIDDEN, "拒绝跨站请求").into_response();
        }
    }
    if req.method() != axum::http::Method::GET
        && req
            .headers()
            .get("x-app-token")
            .and_then(|h| h.to_str().ok())
            != Some(a.token.as_str())
    {
        return (StatusCode::FORBIDDEN, "请从本机应用页面操作").into_response();
    }
    let mut r = next.run(req).await;
    r.headers_mut()
        .insert(header::CACHE_CONTROL, "no-store".parse().unwrap());
    r.headers_mut()
        .insert("x-content-type-options", "nosniff".parse().unwrap());
    r.headers_mut().insert("content-security-policy","default-src 'self'; script-src 'self'; style-src 'self'; img-src 'self' data:; media-src 'self' blob:; connect-src 'self'; frame-ancestors 'none'; base-uri 'none'; form-action 'self'".parse().unwrap());
    r
}
async fn index() -> Html<&'static str> {
    Html(include_str!("../web/index.html"))
}
async fn boot(State(a): State<Shared>) -> Json<Value> {
    Json(json!({"token":a.token,"hub_url":a.hub.url()}))
}
async fn get_library(State(a): State<Shared>) -> Json<Library> {
    Json(a.model.lock().await.library.clone())
}
async fn status(State(a): State<Shared>) -> Json<Value> {
    let m = a.model.lock().await;
    let q = m.quiz.clone();
    let library_revision = m.library.revision;
    drop(m);
    let runtime = a.runtime.read().await.clone();
    Json(
        json!({"quiz":q,"device":runtime.device,"runtime":{"active":runtime.active,"lease":runtime.lease,"hub_url":a.hub.url()},"models":a.models.read().await.clone(),"revision":a.frame_revision.load(Ordering::Relaxed),"library_revision":library_revision,"settings":a.settings.lock().await.clone()}),
    )
}
async fn screen(State(a): State<Shared>) -> Response {
    (
        [(header::CONTENT_TYPE, "image/png")],
        a.png.read().await.clone(),
    )
        .into_response()
}
#[derive(Deserialize)]
struct Mutation {
    revision: u64,
    kind: String,
    #[serde(default)]
    book_id: String,
    #[serde(default)]
    unit_id: String,
    #[serde(default)]
    word_id: String,
    #[serde(default)]
    data: Value,
}
fn field(v: &Value, k: &str, max: usize) -> Result<String> {
    let s = v[k].as_str().unwrap_or("").trim();
    if s.is_empty() || s.chars().count() > max {
        bail!("{k} 不能为空，且不可超过 {max} 字")
    }
    Ok(s.into())
}
fn mutate(db: &mut Library, m: Mutation) -> Result<()> {
    if db.revision != m.revision {
        bail!("词库已更新，请刷新页面后重试，避免覆盖其他修改")
    }
    if m.kind == "book_save" {
        let b = Book {
            id: if m.book_id.is_empty() {
                id()
            } else {
                m.book_id.clone()
            },
            title: field(&m.data, "title", 80)?,
            edition: field(&m.data, "edition", 80)?,
            grade: field(&m.data, "grade", 20)?,
            volume: field(&m.data, "volume", 20)?,
            source: m.data["source"]
                .as_str()
                .unwrap_or("")
                .chars()
                .take(300)
                .collect(),
            units: vec![],
        };
        if m.book_id.is_empty() {
            if db.books.len() >= 100 {
                bail!("最多 100 本教材")
            }
            db.books.push(b)
        } else {
            let old = db
                .books
                .iter_mut()
                .find(|x| x.id == m.book_id)
                .context("教材不存在")?;
            let units = std::mem::take(&mut old.units);
            *old = Book { units, ..b };
        }
    } else if m.kind == "book_delete" {
        if !db.books.iter().any(|b| b.id == m.book_id) {
            bail!("教材不存在")
        }
        db.books.retain(|b| b.id != m.book_id);
    } else {
        let b = db
            .books
            .iter_mut()
            .find(|b| b.id == m.book_id)
            .context("教材不存在")?;
        match m.kind.as_str() {
            "unit_save" => {
                let title = field(&m.data, "title", 80)?;
                if m.unit_id.is_empty() {
                    if b.units.len() >= 200 {
                        bail!("最多 200 个单元")
                    }
                    b.units.push(Unit {
                        id: id(),
                        title,
                        words: vec![],
                    })
                } else {
                    b.units
                        .iter_mut()
                        .find(|u| u.id == m.unit_id)
                        .context("单元不存在")?
                        .title = title;
                }
            }
            "unit_delete" => {
                b.units.retain(|u| u.id != m.unit_id);
            }
            _ => {
                let u = b
                    .units
                    .iter_mut()
                    .find(|u| u.id == m.unit_id)
                    .context("单元不存在")?;
                match m.kind.as_str() {
                    "word_save" => {
                        let mut w = Word {
                            id: if m.word_id.is_empty() {
                                id()
                            } else {
                                m.word_id.clone()
                            },
                            english: field(&m.data, "english", 64)?,
                            chinese: field(&m.data, "chinese", 80)?,
                            example: m.data["example"].as_str().unwrap_or("").into(),
                            phonetic: m.data["phonetic"].as_str().unwrap_or("").into(),
                        };
                        validate_word(&mut w)?;
                        if u.words
                            .iter()
                            .any(|x| x.id != w.id && x.english.eq_ignore_ascii_case(&w.english))
                        {
                            bail!("本单元已经有这个单词")
                        };
                        if m.word_id.is_empty() {
                            u.words.push(w)
                        } else {
                            *u.words
                                .iter_mut()
                                .find(|w| w.id == m.word_id)
                                .context("单词不存在")? = w;
                        }
                    }
                    "word_delete" => u.words.retain(|w| w.id != m.word_id),
                    "import" => {
                        let text = m.data["text"].as_str().context("没有词表")?;
                        let mut reader = csv::ReaderBuilder::new()
                            .trim(csv::Trim::All)
                            .from_reader(text.trim_start_matches('\u{feff}').as_bytes());
                        let headers = reader.headers()?.clone();
                        if headers.get(0) != Some("english") || headers.get(1) != Some("chinese") {
                            bail!("CSV 首行须为 english,chinese,example；第三列可省略")
                        };
                        let mut added = vec![];
                        for row in reader.records() {
                            let row = row?;
                            let mut w = Word {
                                id: id(),
                                english: row.get(0).unwrap_or("").into(),
                                chinese: row.get(1).unwrap_or("").into(),
                                example: headers
                                    .iter()
                                    .position(|x| x == "example")
                                    .and_then(|i| row.get(i))
                                    .unwrap_or("")
                                    .into(),
                                phonetic: headers
                                    .iter()
                                    .position(|x| x == "phonetic")
                                    .and_then(|i| row.get(i))
                                    .unwrap_or("")
                                    .into(),
                            };
                            validate_word(&mut w)?;
                            if u.words
                                .iter()
                                .chain(added.iter())
                                .any(|x: &Word| x.english.eq_ignore_ascii_case(&w.english))
                            {
                                bail!("重复单词：{}。本次导入未保存，请去重后重试", w.english)
                            }
                            added.push(w);
                            if added.len() + u.words.len() > 2000 {
                                bail!("每单元最多 2000 词")
                            }
                        }
                        if added.is_empty() {
                            bail!("没有可导入的单词")
                        }
                        u.words.extend(added);
                    }
                    _ => bail!("未知操作"),
                };
                if u.words.len() > 2000 {
                    bail!("每单元最多 2000 词")
                }
            }
        }
    }
    db.revision += 1;
    Ok(())
}
async fn edit(State(a): State<Shared>, Json(m): Json<Mutation>) -> Api<Library> {
    let mut s = a.model.lock().await;
    let mut db = s.library.clone();
    mutate(&mut db, m)?;
    save(&a.root, &db)?;
    s.library = db.clone();
    Ok(Json(db))
}
#[derive(Deserialize)]
struct Start {
    unit_id: String,
    #[serde(default)]
    word_ids: Vec<String>,
    #[serde(default)]
    wrong_only: bool,
    #[serde(default)]
    shuffle: bool,
}
async fn start(State(a): State<Shared>, Json(req): Json<Start>) -> Api<Value> {
    require_active(&a).await?;
    let mut m = a.model.lock().await;
    if m.quiz.busy || m.quiz.recording {
        bail_api!("录音或识别中，请先结束当前练习")
    };
    let (b, u) = m
        .library
        .books
        .iter()
        .find_map(|b| b.units.iter().find(|u| u.id == req.unit_id).map(|u| (b, u)))
        .context("单元不存在")?;
    let title = format!("{} {} · {}", b.title, b.volume, u.title);
    let mut words: Vec<Word> = u
        .words
        .iter()
        .filter(|w| !w.is_pattern())
        .filter(|w| req.word_ids.is_empty() || req.word_ids.contains(&w.id))
        .filter(|w| !req.wrong_only || m.library.needs_spelling_review(&w.id))
        .cloned()
        .collect();
    if req.shuffle {
        words.sort_by_key(|_| id())
    }
    m.quiz.start(title, words)?;
    m.recording_epoch = None;
    drop(m);
    changed(&a);
    Ok(Json(json!({"ok":true})))
}
macro_rules! bail_api {
    ($s:expr) => {
        return Err(ApiError(anyhow::anyhow!($s)))
    };
}
use bail_api;
#[derive(Deserialize)]
struct Action {
    action: String,
}
async fn act(a: &Shared, action: &str, expected_lease: Option<u64>) -> Result<()> {
    let lease = require_active(a).await?;
    if expected_lease.is_some_and(|value| value != lease) {
        bail!("应用已切换，忽略旧按键")
    }
    let mut m = a.model.lock().await;
    match action {
        "continue" => m.quiz.advance()?,
        "skip" => {
            if m.quiz.busy || m.quiz.recording {
                bail!("录音或识别中，请稍等")
            }
            if m.quiz.current().is_none() {
                bail!("没有进行中的单词")
            }
            m.quiz.next();
        }
        "retry" => {
            if m.quiz.busy || m.quiz.recording {
                bail!("请等待当前录音或识别结束")
            }
            let stage = match m.quiz.stage.as_str() {
                "spell_result" => "spell",
                "read_result" => "read",
                "spell" => "spell",
                "read" => "read",
                _ => bail!("没有可重试的单词"),
            };
            m.quiz.stage = stage.into();
            m.quiz.feedback = None;
            m.quiz.notice.clear();
            m.quiz.epoch += 1;
        }
        "end" => {
            let epoch = m.quiz.epoch + 1;
            m.quiz = Quiz {
                epoch,
                ..Default::default()
            };
            m.recording_epoch = None;
        }
        _ => bail!("未知操作"),
    };
    drop(m);
    if action == "end" {
        let _ = a.hub.stop(lease).await;
    }
    changed(a);
    Ok(())
}
async fn action(State(a): State<Shared>, Json(req): Json<Action>) -> Api<Value> {
    act(&a, &req.action, None).await?;
    Ok(Json(json!({"ok":true})))
}
async fn recognize(
    a: Shared,
    samples: Vec<i16>,
    source: &str,
    expected_epoch: Option<u64>,
) -> Result<()> {
    require_active(&a).await?;
    if samples.len() < 3200 || samples.len() > 480000 {
        bail!("请录制 0.2–30 秒，长按确认键说完后松开")
    };
    let mut m = a.model.lock().await;
    if expected_epoch.is_some_and(|e| e != m.quiz.epoch) {
        bail!("录音所属题目已变化，请重新录音")
    };
    if m.quiz.busy || !matches!(m.quiz.stage.as_str(), "spell" | "read") {
        bail!("当前不在等待回答的步骤")
    };
    m.quiz.recording = false;
    m.quiz.busy = true;
    m.quiz.notice.clear();
    let epoch = m.quiz.epoch;
    let stage = m.quiz.stage.clone();
    drop(m);
    changed(&a);
    let source = source.to_string();
    tokio::spawn(async move {
        let path = a.root.join(format!("data/recordings/{}.wav", id()));
        let result = async {
            tokio::fs::write(&path, wav(&samples)).await?;
            a.asr.call(json!({"path":path,"mode":stage})).await
        }
        .await;
        let _ = tokio::fs::remove_file(path).await;
        // Re-check the lease after inference: an app switch invalidates this answer.
        if refresh_runtime(&a).await.is_err() {
            return;
        }
        let mut m = a.model.lock().await;
        if m.quiz.epoch != epoch {
            return;
        }
        match result {
            Ok(v) => {
                if let Some(attempt) = m.quiz.accept_recognized(
                    epoch,
                    v["text"].as_str().unwrap_or(""),
                    &source,
                    v["recognition"].clone(),
                ) {
                    let mut db = m.library.clone();
                    db.history.push(attempt);
                    if db.history.len() > 10000 {
                        db.history.drain(..1000);
                    }
                    db.revision += 1;
                    match save(&a.root, &db) {
                        Ok(()) => m.library = db,
                        Err(e) => m.quiz.notice = format!("记录保存失败：{e}"),
                    };
                }
            }
            Err(e) => {
                m.quiz.busy = false;
                m.quiz.notice = format!("识别失败：{e}");
            }
        }
        let verdict = m
            .quiz
            .feedback
            .as_ref()
            .filter(|f| f.stage == "spell")
            .and_then(|f| f.correct);
        let play = m.quiz.stage == "read_result";
        let playback_epoch = m.quiz.epoch;
        drop(m);
        changed(&a);
        if let Some(correct) = verdict {
            let _ = play_feedback(&a, playback_epoch, correct).await;
        }
        if play {
            let _ = replay(&a, Some(playback_epoch), None).await;
        }
    });
    Ok(())
}
async fn upload(State(a): State<Shared>, headers: HeaderMap, b: Bytes) -> Api<Value> {
    let epoch = headers
        .get("x-quiz-epoch")
        .and_then(|h| h.to_str().ok())
        .and_then(|x| x.parse::<u64>().ok())
        .context("缺少当前题目标识，请刷新网页")?;
    recognize(a, pcm(&b)?, "web", Some(epoch)).await?;
    Ok(Json(json!({"ok":true})))
}
async fn synth(a: &Shared, text: &str) -> Result<String> {
    let _lock = a.synth_gate.lock().await;
    let key = format!(
        "{:x}",
        Sha256::digest(format!("kokoro-af_heart-0.95-16k-v1:{text}"))
    );
    let path = a.root.join(format!("data/audio/{key}.wav"));
    if !path.exists() {
        a.tts
            .call(json!({"text":format!("{text}."),"path":path}))
            .await?;
        pcm(&tokio::fs::read(&path).await?)?;
    }
    a.models.write().await["tts"] = json!("ready");
    Ok(key)
}
async fn feedback_sound(AxPath(kind): AxPath<String>) -> Response {
    let correct = match kind.as_str() {
        "correct.wav" => true,
        "incorrect.wav" => false,
        _ => return StatusCode::NOT_FOUND.into_response(),
    };
    (
        [(header::CONTENT_TYPE, "audio/wav")],
        wav(&feedback_samples(correct)),
    )
        .into_response()
}
async fn play_feedback(a: &Shared, epoch: u64, correct: bool) -> Result<()> {
    let _gate = a.audio_gate.lock().await;
    let lease = require_active(a).await?;
    let m = a.model.lock().await;
    if m.quiz.epoch != epoch || m.quiz.recording || m.quiz.busy || m.quiz.stage != "spell_result" {
        return Ok(());
    }
    drop(m);
    let volume = a.settings.lock().await.volume;
    if a.runtime.read().await.device["connected"] == true {
        a.hub
            .audio(wav(&feedback_samples(correct)), volume, lease)
            .await?;
    }
    Ok(())
}
async fn replay(a: &Shared, expected: Option<u64>, expected_lease: Option<u64>) -> Result<String> {
    let lease = require_active(a).await?;
    if expected_lease.is_some_and(|value| value != lease) {
        bail!("应用已切换，忽略旧按键")
    }
    let _gate = a
        .audio_gate
        .try_lock()
        .map_err(|_| anyhow::anyhow!("示范音频正在准备或播放"))?;
    let m = a.model.lock().await;
    if expected.is_some_and(|e| e != m.quiz.epoch) {
        bail!("题目已变化")
    };
    if !matches!(
        m.quiz.stage.as_str(),
        "spell_result" | "read" | "read_result"
    ) {
        bail!("完成拼写后才可听答案")
    };
    if m.quiz.recording || m.quiz.busy {
        bail!("录音或识别中，请稍后试听")
    };
    let word = m
        .quiz
        .current()
        .context("没有当前单词")?
        .spoken()
        .to_owned();
    let epoch = m.quiz.epoch;
    drop(m);
    let key = synth(a, &word).await?;
    let m = a.model.lock().await;
    if m.quiz.epoch != epoch || m.quiz.recording || m.quiz.busy {
        bail!("题目或录音状态已变化，已取消示范播放")
    };
    drop(m);
    if a.runtime.read().await.device["connected"] == true {
        let b = tokio::fs::read(a.root.join(format!("data/audio/{key}.wav"))).await?;
        let volume = a.settings.lock().await.volume;
        a.hub.audio(b, volume, lease).await?;
    }
    Ok(key)
}
async fn play(State(a): State<Shared>) -> Api<Value> {
    let key = replay(&a, None, None).await?;
    Ok(Json(json!({"url":format!("/audio/{key}")})))
}
#[derive(Deserialize)]
struct Preview {
    word_id: String,
}
async fn preview_audio(State(a): State<Shared>, Json(p): Json<Preview>) -> Api<Value> {
    let m = a.model.lock().await;
    let word = m
        .library
        .books
        .iter()
        .flat_map(|b| &b.units)
        .flat_map(|u| &u.words)
        .find(|w| w.id == p.word_id)
        .context("单词不存在")?
        .spoken()
        .to_owned();
    drop(m);
    let key = synth(&a, &word).await?;
    Ok(Json(json!({"url":format!("/audio/{key}")})))
}
async fn audio(State(a): State<Shared>, AxPath(key): AxPath<String>) -> Response {
    if key.len() != 64 || !key.bytes().all(|c| c.is_ascii_hexdigit()) {
        return StatusCode::NOT_FOUND.into_response();
    }
    match tokio::fs::read(a.root.join(format!("data/audio/{key}.wav"))).await {
        Ok(b) => ([(header::CONTENT_TYPE, "audio/wav")], b).into_response(),
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}
async fn apply_runtime(a: &Shared, next: hub::Runtime) {
    let changed_lease = {
        let mut state = a.runtime.write().await;
        if next.lease < state.lease {
            return;
        } // Ignore an older concurrent status response.
        let changed = state.active != next.active || state.lease != next.lease;
        *state = next;
        changed
    };
    if changed_lease {
        let mut m = a.model.lock().await;
        m.quiz.epoch += 1;
        m.quiz.busy = false;
        m.quiz.recording = false;
        m.recording_epoch = None;
        m.quiz.notice.clear();
        drop(m);
        changed(a);
    }
}
async fn refresh_runtime(a: &Shared) -> Result<hub::Runtime> {
    let state = a.hub.status().await?;
    apply_runtime(a, state).await;
    Ok(a.runtime.read().await.clone())
}
async fn require_active(a: &Shared) -> Result<u64> {
    let state = refresh_runtime(a).await?;
    if !state.active {
        bail!("请先在 ZECTRIX 本地助手中切换到背单词应用")
    }
    Ok(state.lease)
}
async fn consume(a: Shared) {
    let mut cursor = 0;
    loop {
        let batch = match a.hub.events(cursor).await {
            Ok(batch) => batch,
            Err(e) => {
                let mut state = a.runtime.read().await.clone();
                state.active = false;
                state.device["connected"] = json!(false);
                state.device["error"] = json!(format!("本地助手连接中断：{e}"));
                apply_runtime(&a, state).await;
                tokio::time::sleep(Duration::from_secs(1)).await;
                continue;
            }
        };
        // Fetch device status too; this also invalidates any in-flight recognition.
        if refresh_runtime(&a).await.is_err() {
            continue;
        }
        cursor = batch.cursor;
        let state = a.runtime.read().await.clone();
        for item in batch.events {
            if !state.active || item.lease != state.lease {
                continue;
            }
            match item.event["type"].as_str().unwrap_or("") {
                "recording_started" => {
                    let mut m = a.model.lock().await;
                    if !m.quiz.busy && matches!(m.quiz.stage.as_str(), "spell" | "read") {
                        m.recording_epoch = Some(m.quiz.epoch);
                        m.quiz.recording = true;
                        m.quiz.notice.clear();
                    } else {
                        m.recording_epoch = None;
                        m.quiz.notice = "这一步不需要录音，请松开按键后继续".into();
                    }
                    drop(m);
                    changed(&a);
                }
                "recording_finished" => {
                    let mut m = a.model.lock().await;
                    let epoch = m.recording_epoch.take();
                    m.quiz.recording = false;
                    drop(m);
                    if item.event["reason"] == "Completed" {
                        let result: Result<()> = async {
                            let epoch = epoch.context("没有可用录音，请在回答步骤长按确认键")?;
                            let b = base64::engine::general_purpose::STANDARD
                                .decode(item.event["wav_base64"].as_str().context("录音为空")?)?;
                            recognize(a.clone(), pcm(&b)?, "device", Some(epoch)).await
                        }
                        .await;
                        if let Err(e) = result {
                            a.model.lock().await.quiz.notice = e.to_string();
                        }
                    } else {
                        a.model.lock().await.quiz.notice = "录音未完整结束，请重新录音".into();
                    }
                    changed(&a);
                }
                "key" if item.event["action"] == "Short" => {
                    let result = match item.event["key"].as_str().unwrap_or("") {
                        "Confirm" => act(&a, "continue", Some(item.lease)).await,
                        "Down" => act(&a, "skip", Some(item.lease)).await,
                        "Up" => replay(&a, None, Some(item.lease)).await.map(|_| ()),
                        _ => Ok(()),
                    };
                    if let Err(e) = result {
                        a.model.lock().await.quiz.notice = e.to_string();
                        changed(&a);
                    }
                }
                "disconnected" => {
                    let mut m = a.model.lock().await;
                    m.quiz.epoch += 1;
                    m.quiz.busy = false;
                    m.quiz.recording = false;
                    m.recording_epoch = None;
                    drop(m);
                    changed(&a);
                }
                _ => {}
            }
        }
    }
}
async fn render_loop(a: Shared) {
    let mut rx = a.draw.subscribe();
    loop {
        let state = a.runtime.read().await.clone();
        let q = a.model.lock().await.quiz.clone();
        let mut failed = false;
        match a
            .hub
            .render(layout(&q), state.lease, state.active && !q.recording)
            .await
        {
            Ok(png) => {
                *a.png.write().await = png;
                a.frame_revision.fetch_add(1, Ordering::Relaxed);
            }
            Err(e) => {
                eprintln!("render: {e}");
                failed = true;
            }
        }
        if failed {
            tokio::select! {
                result = rx.changed() => { if result.is_err() { break; } },
                _ = tokio::time::sleep(Duration::from_secs(1)) => {},
            }
        } else if rx.changed().await.is_err() {
            break;
        }
    }
}
async fn warm(a: Shared) {
    let asr = a.clone();
    let tts = a.clone();
    let asr = tokio::spawn(async move {
        let result = asr.asr.warm().await;
        asr.models.write().await["asr"] = json!(if result.is_ok() {
            "ready".to_string()
        } else {
            format!("error: {}", result.unwrap_err())
        });
    });
    let tts = tokio::spawn(async move {
        let result = tts.tts.warm().await;
        tts.models.write().await["tts"] = json!(if result.is_ok() {
            "ready".to_string()
        } else {
            format!("error: {}", result.unwrap_err())
        });
    });
    let _ = tokio::join!(asr, tts);
}
#[tokio::main]
async fn main() -> Result<()> {
    let hub = hub::Hub::from_env()?;
    let app_root = PathBuf::from(
        std::env::var("ZECTRIX_APP_ROOT")
            .context("请从 ZECTRIX 本地助手打开本应用（缺少 ZECTRIX_APP_ROOT）")?,
    )
    .canonicalize()?;
    let root = PathBuf::from(
        std::env::var("ZECTRIX_APP_DATA")
            .context("请从 ZECTRIX 本地助手打开本应用（缺少 ZECTRIX_APP_DATA）")?,
    );
    std::fs::create_dir_all(&root)?;
    let root = root.canonicalize()?;
    let port: u16 = std::env::var("ZECTRIX_VOCAB_PORT")
        .context("请从 ZECTRIX 本地助手打开本应用（缺少网页端口）")?
        .parse()
        .context("应用网页端口无效")?;
    if port == 0 {
        bail!("应用网页端口不能为 0")
    }
    for p in ["data/recordings", "data/audio"] {
        std::fs::create_dir_all(root.join(p))?;
    }
    let path = root.join("data/library.json");
    let library = if path.exists() {
        serde_json::from_slice(&std::fs::read(path)?)
            .context("词库文件损坏；已保留原文件，请检查 library.previous.json")?
    } else {
        let db = seed();
        save(&root, &db)?;
        db
    };
    let runtime = hub
        .status()
        .await
        .context("无法连接本地助手，请从助手重新打开本应用")?;
    let initial_png = hub
        .render(layout(&Quiz::default()), runtime.lease, false)
        .await?;
    let (draw, _) = watch::channel(0);
    let initial_settings = Settings::load(&root)?;
    let a = Arc::new(App {
        settings: Mutex::new(initial_settings),
        token: id(),
        port,
        asr: speech::Speech::new(&app_root, &root, "asr"),
        tts: speech::Speech::new(&app_root, &root, "tts"),
        root,
        model: Mutex::new(Model {
            library,
            quiz: Quiz::default(),
            recording_epoch: None,
        }),
        hub,
        runtime: RwLock::new(runtime),
        models: RwLock::new(json!({"asr":"loading","tts":"loading"})),
        png: RwLock::new(initial_png),
        frame_revision: AtomicU64::new(0),
        draw,
        audio_gate: Mutex::new(()),
        synth_gate: Mutex::new(()),
    });
    let app = Router::new()
        .route("/", get(index))
        .route(
            "/app.js",
            get(|| async {
                (
                    [(header::CONTENT_TYPE, "text/javascript; charset=utf-8")],
                    include_str!("../web/app.js"),
                )
            }),
        )
        .route(
            "/style.css",
            get(|| async {
                (
                    [(header::CONTENT_TYPE, "text/css; charset=utf-8")],
                    include_str!("../web/style.css"),
                )
            }),
        )
        .route(
            "/font/ipa.ttf",
            get(|| async {
                (
                    [(header::CONTENT_TYPE, "font/ttf")],
                    include_bytes!("../assets/fonts/Charis-Regular.ttf").as_slice(),
                )
            }),
        )
        .route("/api/boot", get(boot))
        .route("/api/library", get(get_library))
        .route("/api/status", get(status))
        .route("/api/settings", post(settings))
        .route("/api/library/edit", post(edit))
        .route("/api/quiz/start", post(start))
        .route("/api/quiz/action", post(action))
        .route("/api/quiz/audio", post(upload))
        .route("/api/quiz/play", post(play))
        .route("/api/word/audio", post(preview_audio))
        .route("/screen.png", get(screen))
        .route("/audio/{key}", get(audio))
        .route("/sounds/{kind}", get(feedback_sound))
        .layer(DefaultBodyLimit::max(1100000))
        .layer(middleware::from_fn_with_state(a.clone(), secure))
        .with_state(a.clone());
    let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, port)).await?;
    tokio::spawn(render_loop(a.clone()));
    tokio::spawn(consume(a.clone()));
    tokio::spawn(warm(a.clone()));
    println!("单词练习已启动：http://127.0.0.1:{port}");
    axum::serve(listener, app)
        .with_graceful_shutdown(async move {
            #[cfg(unix)]
            {
                let mut term =
                    tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
                        .unwrap();
                tokio::select! {_=tokio::signal::ctrl_c()=>{},_=term.recv()=>{}}
            }
            #[cfg(not(unix))]
            {
                let _ = tokio::signal::ctrl_c().await;
            }
        })
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn settings_persist_and_reject_invalid_volume() {
        let root = std::env::temp_dir().join(format!("vocab-settings-{}", id()));
        std::fs::create_dir_all(root.join("data")).unwrap();
        assert_eq!(Settings::load(&root).unwrap().volume, 65);
        for volume in [0, 40, 100] {
            Settings { volume }.save(&root).unwrap();
            assert_eq!(Settings::load(&root).unwrap().volume, volume);
        }
        assert!(Settings { volume: 101 }.save(&root).is_err());
        assert_eq!(Settings::load(&root).unwrap().volume, 100);
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn import_atomic_and_revision() {
        let mut db = seed();
        let b = &db.books[1];
        let u = &b.units[0];
        let req = Mutation {
            revision: db.revision,
            kind: "import".into(),
            book_id: b.id.clone(),
            unit_id: u.id.clone(),
            word_id: String::new(),
            data: json!({"text":"english,chinese\nhello,你好\napple,苹果"}),
        };
        assert!(mutate(&mut db.clone(), req).is_err());
        assert_eq!(db.books[1].units[0].words.len(), 6);
        let mut request = Mutation {
            revision: 0,
            kind: "book_delete".into(),
            book_id: db.books[0].id.clone(),
            unit_id: String::new(),
            word_id: String::new(),
            data: Value::Null,
        };
        assert!(mutate(&mut db.clone(), request).is_err());
        request = Mutation {
            revision: db.revision,
            kind: "import".into(),
            book_id: db.books[0].id.clone(),
            unit_id: db.books[0].units[0].id.clone(),
            word_id: String::new(),
            data: json!({"text":"english,chinese,example\nhello,你好,Hello!"}),
        };
        mutate(&mut db, request).unwrap();
        assert_eq!(db.books[0].units[0].words[0].english, "hello");
    }
}
