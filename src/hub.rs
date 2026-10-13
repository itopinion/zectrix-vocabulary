//! Private, loopback-only application protocol. This credential never reaches the browser.
use anyhow::{bail, Context, Result};
use serde::Deserialize;
use serde_json::{json, Value};
use std::time::Duration;

#[derive(Clone, Debug, Deserialize)]
pub struct Runtime {
    pub active: bool,
    pub lease: u64,
    #[serde(default)]
    pub device: Value,
}
#[derive(Deserialize)]
pub struct Event {
    pub lease: u64,
    pub event: Value,
}
#[derive(Deserialize)]
pub struct Events {
    pub events: Vec<Event>,
    pub cursor: u64,
}
pub struct Hub {
    client: reqwest::Client,
    url: String,
    token: String,
}
impl Hub {
    pub fn from_env() -> Result<Self> {
        let raw = std::env::var("ZECTRIX_HUB_URL")
            .context("请从 ZECTRIX 本地助手打开本应用（缺少 ZECTRIX_HUB_URL）")?;
        let url = reqwest::Url::parse(&raw)?;
        if url.scheme() != "http"
            || !matches!(url.host_str(), Some("127.0.0.1") | Some("localhost"))
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
            || url.path() != "/"
        {
            bail!("本地助手地址必须是本机 HTTP 地址")
        }
        let token = std::env::var("ZECTRIX_APP_TOKEN")
            .context("请从 ZECTRIX 本地助手打开本应用（缺少私有凭证）")?;
        if token.is_empty() {
            bail!("本地助手凭证为空")
        }
        let client = reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(40))
            .build()?;
        Ok(Self {
            client,
            url: raw.trim_end_matches('/').to_owned(),
            token,
        })
    }
    pub fn url(&self) -> &str {
        &self.url
    }
    async fn checked(&self, request: reqwest::RequestBuilder) -> Result<reqwest::Response> {
        let response = request.bearer_auth(&self.token).send().await?;
        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().await.unwrap_or_default();
            bail!(
                "本地助手拒绝请求 ({status})：{}",
                text.chars().take(300).collect::<String>()
            )
        }
        Ok(response)
    }
    pub async fn status(&self) -> Result<Runtime> {
        Ok(self
            .checked(self.client.get(format!("{}/runtime/status", self.url)))
            .await?
            .json()
            .await?)
    }
    pub async fn events(&self, after: u64) -> Result<Events> {
        Ok(self
            .checked(
                self.client
                    .get(format!("{}/runtime/events", self.url))
                    .query(&[("after", after)])
                    .timeout(Duration::from_secs(6)),
            )
            .await?
            .json()
            .await?)
    }
    pub async fn render(&self, layout: Value, lease: u64, present: bool) -> Result<Vec<u8>> {
        Ok(self
            .checked(
                self.client
                    .post(format!("{}/runtime/render", self.url))
                    .json(&json!({"layout":layout,"lease":lease,"present":present})),
            )
            .await?
            .bytes()
            .await?
            .to_vec())
    }
    pub async fn audio(&self, wav: Vec<u8>, volume: u8, lease: u64) -> Result<Value> {
        let result: Value = self
            .checked(
                self.client
                    .post(format!("{}/runtime/audio", self.url))
                    .header("Content-Type", "audio/wav")
                    .header("X-Lease", lease.to_string())
                    .header("X-Volume", volume.to_string())
                    .body(wav),
            )
            .await?
            .json()
            .await?;
        if result["result"] != "Ok(Completed)" {
            bail!("设备音频未完成：{}", result)
        }
        Ok(result)
    }
    pub async fn stop(&self, lease: u64) -> Result<()> {
        self.checked(
            self.client
                .post(format!("{}/runtime/stop", self.url))
                .json(&json!({"lease":lease})),
        )
        .await?;
        Ok(())
    }
}
