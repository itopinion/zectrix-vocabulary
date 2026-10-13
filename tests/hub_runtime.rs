//! Runs the packaged application against a loopback Hub stub. No device or model needed.
use axum::{
    extract::{Query, State},
    http::{HeaderMap, StatusCode},
    routing::{get, post},
    Json, Router,
};
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    process::{Child, Command, Stdio},
    sync::Arc,
    time::Duration,
};
use tokio::sync::Mutex;
struct Process(Child);
impl Drop for Process {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
#[derive(Default)]
struct Stub {
    active: bool,
    lease: u64,
    events: Vec<Value>,
    frames: usize,
}
type Shared = Arc<Mutex<Stub>>;
fn authorized(headers: &HeaderMap) -> bool {
    headers.get("authorization").and_then(|x| x.to_str().ok()) == Some("Bearer private-test-secret")
}
async fn status(State(s): State<Shared>, h: HeaderMap) -> Result<Json<Value>, StatusCode> {
    if !authorized(&h) {
        return Err(StatusCode::UNAUTHORIZED);
    }
    let s = s.lock().await;
    Ok(Json(
        json!({"active":s.active,"lease":s.lease,"device":{"connected":false}}),
    ))
}
async fn events(
    State(s): State<Shared>,
    Query(q): Query<HashMap<String, String>>,
    h: HeaderMap,
) -> Result<Json<Value>, StatusCode> {
    if !authorized(&h) {
        return Err(StatusCode::UNAUTHORIZED);
    }
    tokio::time::sleep(Duration::from_millis(30)).await;
    let s = s.lock().await;
    let after = q
        .get("after")
        .and_then(|x| x.parse::<u64>().ok())
        .unwrap_or(0);
    Ok(Json(
        json!({"events":s.events.iter().filter(|e|e["seq"].as_u64().unwrap()>after).collect::<Vec<_>>(),"cursor":s.events.len(),"active":s.active,"lease":s.lease}),
    ))
}
async fn render(
    State(s): State<Shared>,
    h: HeaderMap,
    Json(body): Json<Value>,
) -> Result<Vec<u8>, StatusCode> {
    if !authorized(&h) {
        return Err(StatusCode::UNAUTHORIZED);
    }
    let mut s = s.lock().await;
    if body["present"] == true && (!s.active || body["lease"] != s.lease) {
        return Err(StatusCode::CONFLICT);
    }
    s.frames += 1;
    Ok(vec![137, 80, 78, 71, 13, 10, 26, 10])
}
async fn wait_status(
    client: &reqwest::Client,
    url: &str,
    predicate: impl Fn(&Value) -> bool,
) -> Value {
    for _ in 0..150 {
        if let Ok(r) = client.get(format!("{url}/api/status")).send().await {
            if let Ok(v) = r.json::<Value>().await {
                if predicate(&v) {
                    return v;
                }
            }
        }
        tokio::time::sleep(Duration::from_millis(30)).await;
    }
    panic!("application did not reach requested state")
}
#[tokio::test]
async fn plugin_lifecycle_and_recording_are_isolated_by_lease() {
    let root = std::env::temp_dir().join(format!("zectrix-plugin-test-{}", uuid::Uuid::new_v4()));
    let data = root.join("user-data");
    std::fs::create_dir_all(&data).unwrap();
    let shared = Arc::new(Mutex::new(Stub::default()));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let hub_url = format!("http://{}", listener.local_addr().unwrap());
    let routes = Router::new()
        .route("/runtime/status", get(status))
        .route("/runtime/events", get(events))
        .route("/runtime/render", post(render))
        .with_state(shared.clone());
    let server = tokio::spawn(async move { axum::serve(listener, routes).await.unwrap() });
    let port_listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = port_listener.local_addr().unwrap().port();
    drop(port_listener);
    let url = format!("http://127.0.0.1:{port}");
    let app = Process(
        Command::new(env!("CARGO_BIN_EXE_zectrix-vocabulary"))
            .env("ZECTRIX_HUB_URL", &hub_url)
            .env("ZECTRIX_APP_TOKEN", "private-test-secret")
            .env("ZECTRIX_APP_ROOT", env!("CARGO_MANIFEST_DIR"))
            .env("ZECTRIX_APP_DATA", &data)
            .env("ZECTRIX_VOCAB_PORT", port.to_string())
            .env_remove("ZECTRIX_ASR_PYTHON")
            .env_remove("ZECTRIX_TTS_PYTHON")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    let s = wait_status(&client, &url, |s| {
        s["models"]["asr"]
            .as_str()
            .unwrap_or("")
            .starts_with("error:")
    })
    .await;
    assert_eq!(s["runtime"]["active"], false);
    let boot: Value = client
        .get(format!("{url}/api/boot"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(!boot.to_string().contains("private-test-secret"));
    let token = boot["token"].as_str().unwrap();
    let db: Value = client
        .get(format!("{url}/api/library"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let unit = &db["books"][1]["units"][0];
    let start = json!({"unit_id":unit["id"],"shuffle":false});
    let response = client
        .post(format!("{url}/api/quiz/start"))
        .header("X-App-Token", token)
        .json(&start)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 400);
    {
        let mut s = shared.lock().await;
        s.active = true;
        s.lease = 1;
    }
    wait_status(&client, &url, |s| s["runtime"]["active"] == true).await;
    assert!(client
        .post(format!("{url}/api/quiz/start"))
        .header("X-App-Token", token)
        .json(&start)
        .send()
        .await
        .unwrap()
        .status()
        .is_success());
    let s = wait_status(&client, &url, |s| s["quiz"]["stage"] == "spell").await;
    let epoch = s["quiz"]["epoch"].as_u64().unwrap();
    shared
        .lock()
        .await
        .events
        .push(json!({"seq":1,"lease":1,"event":{"type":"recording_started"}}));
    wait_status(&client, &url, |s| s["quiz"]["recording"] == true).await;
    {
        let mut s = shared.lock().await;
        s.active = false;
        s.lease = 2;
    }
    let s = wait_status(&client, &url, |s| {
        s["runtime"]["active"] == false && s["quiz"]["recording"] == false
    })
    .await;
    assert!(s["quiz"]["epoch"].as_u64().unwrap() > epoch);
    assert_eq!(s["quiz"]["index"], 0);
    {
        let mut s = shared.lock().await;
        s.active = true;
        s.lease = 3;
        s.events
            .push(json!({"seq":2,"lease":1,"event":{"type":"key","key":"Down","action":"Short"}}));
        s.events.push(json!({"seq":3,"lease":1,"event":{"type":"recording_finished","reason":"Completed","wav_base64":"invalid"}}));
    }
    let s = wait_status(&client, &url, |s| s["runtime"]["lease"] == 3).await;
    assert_eq!(s["quiz"]["index"], 0);
    assert_eq!(s["quiz"]["notice"], "");
    shared
        .lock()
        .await
        .events
        .push(json!({"seq":4,"lease":3,"event":{"type":"key","key":"Down","action":"Short"}}));
    wait_status(&client, &url, |s| s["quiz"]["index"] == 1).await;
    assert!(shared.lock().await.frames > 1);
    assert!(data.join("data/library.json").exists());
    assert_eq!(
        client
            .post(format!("{url}/api/device/connect"))
            .header("X-App-Token", token)
            .json(&json!({}))
            .send()
            .await
            .unwrap()
            .status(),
        404
    );
    drop(app);
    server.abort();
    std::fs::remove_dir_all(root).unwrap();
}
