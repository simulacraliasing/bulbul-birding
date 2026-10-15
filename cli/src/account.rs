use crate::{Result, ToolError};
use directories::BaseDirs;
use reqwest::{Client, Method, StatusCode};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{path::PathBuf, process::Stdio, sync::Arc, time::Duration};
use tokio::{process::Command, sync::Mutex, time::Instant};
use url::Url;

pub const DEFAULT_API_URL: &str = "https://bulbulbirding.cn";
const KEYRING_ERROR: &str = "系统凭据库不可用。Windows 使用凭据管理器，macOS 使用钥匙串，Linux 需要启用 Secret Service（支持提供该服务的 KWallet）。";

#[derive(Default)]
pub struct Environment {
    pub api_url: Option<String>,
    pub session_token: Option<String>,
}

impl Environment {
    pub fn read() -> Self {
        fn value(name: &str) -> Option<String> {
            std::env::var(name)
                .ok()
                .map(|v| v.trim().to_owned())
                .filter(|v| !v.is_empty())
        }
        Self {
            api_url: value("BULBUL_API_URL"),
            session_token: value("BULBUL_SESSION_TOKEN"),
        }
    }
}

// No Debug implementation: session tokens must never appear in diagnostics.
#[derive(Serialize, Deserialize)]
pub struct Session {
    pub session_token: String,
    pub expires_at: Option<String>,
}

pub trait CredentialStore: Send + Sync {
    fn api_url(&self) -> Result<Option<String>>;
    fn ensure_available(&self, api_url: &str) -> Result<()>;
    fn read(&self, api_url: &str) -> Result<Option<Session>>;
    fn save(&self, api_url: &str, session: &Session) -> Result<()>;
    fn clear(&self, api_url: &str) -> Result<()>;
}

pub struct OsCredentialStore {
    path: PathBuf,
}

impl OsCredentialStore {
    pub fn new() -> Result<Self> {
        let dirs = BaseDirs::new().ok_or_else(|| ToolError::new("无法找到本机设置目录。"))?;
        // Match platformdirs("bulbul-birding") used by the previous Python client.
        #[cfg(windows)]
        let path = dirs
            .data_local_dir()
            .join("bulbul-birding/bulbul-birding/settings.json");
        #[cfg(not(windows))]
        let path = dirs.config_dir().join("bulbul-birding/settings.json");
        Ok(Self { path })
    }

    fn entry(api_url: &str) -> Result<keyring::Entry> {
        let username = format!("{:x}", Sha256::digest(api_url.as_bytes()));
        keyring::Entry::new("bulbul-birding", &username).map_err(|_| ToolError::new(KEYRING_ERROR))
    }
}

impl CredentialStore for OsCredentialStore {
    fn api_url(&self) -> Result<Option<String>> {
        if !self.path.exists() {
            return Ok(None);
        }
        let error =
            || ToolError::new("无法读取 Bulbul 本机设置，请重新运行 login --api-url 指定站点。");
        let value: Value = serde_json::from_slice(&std::fs::read(&self.path).map_err(|_| error())?)
            .map_err(|_| error())?;
        let api = value
            .get("api_url")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .ok_or_else(error)?;
        Ok(Some(api.to_owned()))
    }

    fn ensure_available(&self, api_url: &str) -> Result<()> {
        self.read(api_url).map(|_| ())
    }

    fn read(&self, api_url: &str) -> Result<Option<Session>> {
        match Self::entry(api_url)?.get_password() {
            Ok(raw) => {
                let session: Session = serde_json::from_str(&raw).map_err(|_| {
                    ToolError::new("无法读取保存的 Bulbul 登录，请重新运行 login。")
                })?;
                if session.session_token.is_empty() {
                    return Err(ToolError::new("保存的 Bulbul 登录无效，请重新运行 login。"));
                }
                Ok(Some(session))
            }
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(_) => Err(ToolError::new(KEYRING_ERROR)),
        }
    }

    fn save(&self, api_url: &str, session: &Session) -> Result<()> {
        let error = || ToolError::new("无法保存 Bulbul 登录，请检查系统凭据库和本机设置目录。");
        let parent = self.path.parent().ok_or_else(error)?;
        std::fs::create_dir_all(parent).map_err(|_| error())?;
        // This file contains only an API address. Credentials stay in the OS vault.
        let temporary = self
            .path
            .with_extension(format!("{}.tmp", std::process::id()));
        std::fs::write(
            &temporary,
            serde_json::to_vec(&json!({"api_url": api_url})).map_err(|_| error())?,
        )
        .map_err(|_| error())?;
        std::fs::rename(&temporary, &self.path).map_err(|_| error())?;
        Self::entry(api_url)?
            .set_password(&serde_json::to_string(session).map_err(|_| error())?)
            .map_err(|_| error())
    }

    fn clear(&self, api_url: &str) -> Result<()> {
        match Self::entry(api_url)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(_) => Err(ToolError::new(
                "无法删除本机保存的 Bulbul 登录，请检查系统凭据库。",
            )),
        }
    }
}

pub fn normalize_api_url(value: &str) -> Result<String> {
    let value = value.trim().trim_end_matches('/');
    let invalid = || {
        ToolError::new(
            "Bulbul 地址需要是 HTTP(S) 站点地址，或以 /api/v1 结尾，不能包含账号、密码或查询参数。",
        )
    };
    let url = Url::parse(value).map_err(|_| invalid())?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || !matches!(url.path(), "" | "/" | "/api/v1")
    {
        return Err(invalid());
    }
    if url.scheme() == "http"
        && !matches!(
            url.host_str(),
            Some("localhost" | "127.0.0.1" | "[::1]" | "::1")
        )
    {
        return Err(ToolError::new("远程 Bulbul 站点需要使用 HTTPS。"));
    }
    Ok(format!("{}/api/v1", url.origin().ascii_serialization()))
}

pub async fn http_json(
    http: &Client,
    method: Method,
    url: &str,
    body: Option<&Value>,
    token: Option<&str>,
    query: &[(String, String)],
) -> Result<Value> {
    let mut request = http.request(method, url).query(query);
    if let Some(body) = body {
        request = request.json(body);
    }
    if let Some(token) = token {
        request = request.bearer_auth(token);
    }
    read_json_response(request).await
}

pub async fn read_json_response(request: reqwest::RequestBuilder) -> Result<Value> {
    let response = request
        .send()
        .await
        .map_err(|_| ToolError::new("暂时无法连接 Bulbul，请检查网络后重试。"))?;
    decode_json_response(response).await
}

// Retry only admission rejections, never a request with an uncertain processing outcome.
pub async fn read_json_response_with_overload_retry<F>(make_request: F) -> Result<Value>
where
    F: Fn() -> Result<reqwest::RequestBuilder>,
{
    let started = std::time::Instant::now();
    let mut retries = 0;
    loop {
        let response = make_request()?
            .send()
            .await
            .map_err(|_| ToolError::new("暂时无法连接 Bulbul，没有自动重新上传文件。"))?;
        let delay = if response.status() == StatusCode::TOO_MANY_REQUESTS
            && response
                .headers()
                .get("x-bulbul-request-accepted")
                .and_then(|v| v.to_str().ok())
                == Some("false")
        {
            response
                .headers()
                .get("retry-after")
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.parse::<u64>().ok())
                .filter(|seconds| *seconds <= 10)
                .map(Duration::from_secs)
        } else {
            None
        };
        if let Some(delay) = delay
            && retries < 2
            && started.elapsed().saturating_add(delay) <= Duration::from_secs(20)
        {
            retries += 1;
            drop(response);
            eprintln!(
                "识别请求尚未受理，等待 {} 秒后重试（{}/2）。",
                delay.as_secs(),
                retries
            );
            tokio::time::sleep(delay).await;
            continue;
        }
        return decode_json_response(response).await;
    }
}

async fn decode_json_response(mut response: reqwest::Response) -> Result<Value> {
    match response.status() {
        StatusCode::UNAUTHORIZED => {
            return Err(ToolError::new(
                "Bulbul 登录已过期，请重新运行 bulbul login 登录。",
            ));
        }
        StatusCode::TOO_MANY_REQUESTS => {
            let seconds = response
                .headers()
                .get("retry-after")
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.parse::<u64>().ok());
            return Err(ToolError::new(seconds.map_or_else(
                || "请求较频繁或服务繁忙，请稍后重试。".to_owned(),
                |seconds| format!("请求较频繁或服务繁忙，请等待 {seconds} 秒后重试。"),
            )));
        }
        StatusCode::NO_CONTENT => return Ok(json!({})),
        status if !status.is_success() => {
            return Err(ToolError::new(format!(
                "服务返回 HTTP {}，没有自动重试本次操作。",
                status.as_u16()
            )));
        }
        _ => {}
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| ToolError::new("读取服务响应失败，没有自动重试本次操作。"))?
    {
        if bytes.len() + chunk.len() > 16 * 1024 * 1024 {
            return Err(ToolError::new("服务响应过大，请缩小查询范围。"));
        }
        bytes.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&bytes).map_err(|_| ToolError::new("服务返回了无法识别的 JSON 响应。"))
}

struct LoginAttempt {
    device_code: String,
    deadline: Instant,
    interval: Duration,
    next_poll: Instant,
}

pub struct Account {
    pub api_url: String,
    pub http: Client,
    store: Arc<dyn CredentialStore>,
    token_override: Option<String>,
    attempt: Mutex<Option<LoginAttempt>>,
}

impl Account {
    pub fn new(
        http: Client,
        store: Arc<dyn CredentialStore>,
        env: &Environment,
        api_override: Option<&str>,
    ) -> Result<Self> {
        let selected = api_override.or(env.api_url.as_deref());
        let api_url = match selected {
            Some(value) => normalize_api_url(value)?,
            None => normalize_api_url(store.api_url()?.as_deref().unwrap_or(DEFAULT_API_URL))?,
        };
        Ok(Self {
            api_url,
            http,
            store,
            token_override: env.session_token.clone(),
            attempt: Mutex::new(None),
        })
    }

    pub fn token(&self) -> Result<Option<String>> {
        if let Some(token) = &self.token_override {
            return Ok(Some(token.clone()));
        }
        Ok(self.store.read(&self.api_url)?.map(|s| s.session_token))
    }

    async fn request(
        &self,
        method: Method,
        path: &str,
        body: Option<&Value>,
        token: Option<&str>,
    ) -> Result<Value> {
        http_json(
            &self.http,
            method,
            &format!("{}{path}", self.api_url),
            body,
            token,
            &[],
        )
        .await
    }

    pub async fn begin_login(&self, open_browser: bool) -> Result<Value> {
        if self.token_override.is_some() {
            return Err(ToolError::new(
                "当前设置了 BULBUL_SESSION_TOKEN；请先移除它，再使用浏览器登录。",
            ));
        }
        self.store.ensure_available(&self.api_url)?;
        let mut attempt = self.attempt.lock().await;
        let result = self
            .request(Method::POST, "/auth/device/start", None, None)
            .await?;
        let invalid = || ToolError::new("Bulbul 登录服务返回了无效的授权信息。");
        let uri = result["verification_uri"].as_str().ok_or_else(invalid)?;
        let verification = Url::parse(uri).map_err(|_| invalid())?;
        // The API and web app may use different domains or localhost ports.
        normalize_api_url(&verification.origin().ascii_serialization()).map_err(|_| invalid())?;
        if verification.path() != "/auth/device"
            || !verification.username().is_empty()
            || verification.password().is_some()
            || verification.fragment().is_some()
        {
            return Err(invalid());
        }
        let device_code = result["device_code"]
            .as_str()
            .filter(|s| {
                s.len() == 43
                    && s.bytes()
                        .all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-')
            })
            .ok_or_else(invalid)?;
        let user_code = result["user_code"]
            .as_str()
            .filter(|s| {
                !s.is_empty()
                    && s.len() <= 32
                    && s.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-')
            })
            .ok_or_else(invalid)?;
        let query = verification.query_pairs().collect::<Vec<_>>();
        if query.len() != 1 || query[0].0 != "code" || query[0].1 != user_code {
            return Err(invalid());
        }
        let expires = result["expires_in"]
            .as_u64()
            .filter(|n| (60..=1800).contains(n))
            .ok_or_else(invalid)?;
        let interval = result["interval"]
            .as_u64()
            .filter(|n| (3..=30).contains(n))
            .ok_or_else(invalid)?;
        *attempt = Some(LoginAttempt {
            device_code: device_code.to_owned(),
            deadline: Instant::now() + Duration::from_secs(expires),
            interval: Duration::from_secs(interval),
            next_poll: Instant::now(),
        });
        let opened = open_browser && launch_browser(uri).await;
        Ok(
            json!({"status":"pending", "verification_url":uri, "user_code":user_code, "expires_in":expires, "browser_opened":opened, "message":"请在浏览器登录，对照授权码后确认。完成后返回这里继续。"}),
        )
    }

    pub async fn complete_login(&self) -> Result<Value> {
        let mut guard = self.attempt.lock().await;
        let attempt = guard
            .as_mut()
            .ok_or_else(|| ToolError::new("请先运行 bulbul login，开始一次登录。"))?;
        if Instant::now() >= attempt.deadline {
            *guard = None;
            return Err(ToolError::new("本次授权已过期，请重新登录。"));
        }
        if Instant::now() < attempt.next_poll {
            return Ok(json!({"status":"pending", "message":"还在等待浏览器确认，请稍后继续。"}));
        }
        attempt.next_poll = Instant::now() + attempt.interval;
        let result = self
            .request(
                Method::POST,
                "/auth/device/poll",
                Some(&json!({"device_code":attempt.device_code})),
                None,
            )
            .await?;
        match result["status"].as_str() {
            Some("pending") => {
                return Ok(json!({"status":"pending", "message":"还在等待浏览器确认。"}));
            }
            Some("denied") => {
                *guard = None;
                return Ok(json!({"status":"denied", "message":"已取消本次登录，没有保存账号。"}));
            }
            Some("expired") => {
                *guard = None;
                return Err(ToolError::new("本次授权已过期，请重新登录。"));
            }
            Some("authorized") => {}
            _ => {
                *guard = None;
                return Err(ToolError::new("Bulbul 登录服务返回了无效的会话。"));
            }
        }
        *guard = None;
        let token = result["session_token"]
            .as_str()
            .filter(|s| !s.is_empty())
            .ok_or_else(|| ToolError::new("Bulbul 登录服务返回了无效的会话。"))?;
        let session = Session {
            session_token: token.to_owned(),
            expires_at: result["expires_at"].as_str().map(str::to_owned),
        };
        if let Err(error) = self.store.save(&self.api_url, &session) {
            let _ = self
                .request(Method::POST, "/auth/logout", None, Some(token))
                .await;
            return Err(error);
        }
        Ok(
            json!({"status":"logged_in", "message":"登录完成，后续查询会自动使用这个账号。", "user":result.get("user"), "expires_at":session.expires_at}),
        )
    }

    pub async fn login(&self, open_browser: bool) -> Result<Value> {
        let info = self.begin_login(open_browser).await?;
        eprintln!(
            "请打开 {}\n授权码：{}\n在浏览器确认后，这里会自动完成登录。",
            info["verification_url"].as_str().unwrap_or_default(),
            info["user_code"].as_str().unwrap_or_default()
        );
        loop {
            let interval = self
                .attempt
                .lock()
                .await
                .as_ref()
                .map(|a| a.interval)
                .ok_or_else(|| ToolError::new("本次授权已过期，请重新登录。"))?;
            tokio::time::sleep(interval).await;
            let result = self.complete_login().await?;
            if result["status"] != "pending" {
                return Ok(result);
            }
        }
    }

    pub async fn status(&self) -> Result<Value> {
        let Some(token) = self.token()? else {
            return Ok(json!({"status":"signed_out", "message":"请运行 bulbul login 登录。"}));
        };
        let result = self
            .request(Method::GET, "/auth/me", None, Some(&token))
            .await?;
        Ok(json!({"status":"logged_in", "user":result.get("user"), "api_url":self.api_url}))
    }

    pub async fn logout(&self) -> Result<Value> {
        if let Some(token) = self.token()? {
            // Keep the credential if revocation failed so users can retry logout.
            self.request(Method::POST, "/auth/logout", None, Some(&token))
                .await?;
        }
        if self.token_override.is_none() {
            self.store.clear(&self.api_url)?;
        }
        *self.attempt.lock().await = None;
        Ok(json!({"status":"signed_out", "message":"已退出工具登录。网页的登录状态保持不变。"}))
    }
}

async fn launch_browser(uri: &str) -> bool {
    #[cfg(windows)]
    let mut command = {
        let mut c = Command::new("rundll32.exe");
        c.arg("url.dll,FileProtocolHandler");
        c.creation_flags(0x08000000);
        c
    };
    #[cfg(target_os = "macos")]
    let mut command = Command::new("open");
    #[cfg(target_os = "linux")]
    let mut command = Command::new("xdg-open");
    // Launch directly, never through a shell, and keep CLI stdout clean.
    command
        .arg(uri)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    matches!(tokio::time::timeout(Duration::from_secs(10), command.status()).await, Ok(Ok(status)) if status.success())
}
