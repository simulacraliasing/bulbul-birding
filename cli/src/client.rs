use crate::{
    Result, ToolError,
    account::{Account, CredentialStore, Environment, OsCredentialStore, http_json},
};
use percent_encoding::{NON_ALPHANUMERIC, utf8_percent_encode};
use reqwest::{Client, Method, redirect::Policy};
use serde::Deserialize;
use serde_json::{Map, Value, json};
use std::{
    sync::{Arc, LazyLock},
    time::Duration,
};

#[derive(Deserialize)]
pub struct ToolSpec {
    pub name: String,
    pub description: String,
    #[serde(default)]
    pub method: String,
    #[serde(default)]
    pub path: String,
    pub mutation: bool,
    #[serde(default)]
    pub path_keys: Vec<String>,
    #[serde(default)]
    pub query_keys: Vec<String>,
    #[serde(default)]
    pub body_keys: Vec<String>,
    #[serde(default)]
    pub has_body: bool,
    pub result_limit: Option<String>,
    pub input_schema: Value,
}

// These immutable definitions are embedded in the binary, independent of cwd.
pub static TOOLS: LazyLock<Vec<ToolSpec>> = LazyLock::new(|| {
    serde_json::from_str(include_str!("../../assets/tools.json"))
        .expect("invalid embedded tool contract")
});

static VALIDATORS: LazyLock<Vec<jsonschema::Validator>> = LazyLock::new(|| {
    TOOLS
        .iter()
        .map(|tool| {
            jsonschema::options()
                .should_validate_formats(true)
                .build(&tool.input_schema)
                .expect("invalid embedded tool schema")
        })
        .collect()
});

pub fn find_tool(name: &str) -> Result<(usize, &'static ToolSpec)> {
    TOOLS
        .iter()
        .enumerate()
        .find(|(_, tool)| tool.name == name)
        .ok_or_else(|| ToolError::new("没有这个 Bulbul 工具，请运行 list 查看可用工具。"))
}

pub fn list_tools(name: Option<&str>) -> Result<Value> {
    let tools: Vec<&ToolSpec> = match name {
        Some(name) => vec![find_tool(name)?.1],
        None => TOOLS.iter().collect(),
    };
    Ok(Value::Array(tools.iter().map(|tool| json!({"name":tool.name, "description":tool.description, "input_schema":tool.input_schema})).collect()))
}

pub struct BulbulClient {
    pub account: Account,
    tianditu_key: Option<String>,
    geocoder_url: String,
}

impl BulbulClient {
    pub fn from_environment(api_override: Option<&str>) -> Result<Self> {
        Self::new(
            Arc::new(OsCredentialStore::new()?),
            Environment::read(),
            api_override,
        )
    }

    pub fn new(
        store: Arc<dyn CredentialStore>,
        env: Environment,
        api_override: Option<&str>,
    ) -> Result<Self> {
        let http = Client::builder()
            .timeout(Duration::from_secs(90))
            .connect_timeout(Duration::from_secs(15))
            .redirect(Policy::none())
            .retry(reqwest::retry::never())
            .user_agent(concat!("bulbul-birding/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|_| ToolError::new("无法启动 Bulbul HTTP 客户端。"))?;
        Self::with_http(
            http,
            store,
            env,
            api_override,
            "https://api.tianditu.gov.cn/v2/search",
        )
    }

    pub fn with_http(
        http: Client,
        store: Arc<dyn CredentialStore>,
        env: Environment,
        api_override: Option<&str>,
        geocoder_url: &str,
    ) -> Result<Self> {
        let account = Account::new(http, store, &env, api_override)?;
        Ok(Self {
            account,
            tianditu_key: env.tianditu_key,
            geocoder_url: geocoder_url.to_owned(),
        })
    }

    pub async fn call(&self, name: &str, arguments: Value) -> Result<Value> {
        let (index, tool) = find_tool(name)?;
        if let Some(error) = VALIDATORS[index].iter_errors(&arguments).next() {
            // Validator descriptions include argument values; expose only the field.
            return Err(ToolError::new(format!(
                "参数 {} 不符合工具 schema，请运行 list 查看参数要求。",
                error.instance_path()
            )));
        }
        let mut values = Map::new();
        if let Some(properties) = tool.input_schema["properties"].as_object() {
            for (key, schema) in properties {
                if let Some(default) = schema.get("default") {
                    values.insert(key.clone(), default.clone());
                }
            }
        }
        values.extend(
            arguments
                .as_object()
                .ok_or_else(|| ToolError::new("参数必须是 JSON object。"))?
                .clone(),
        );
        if name == "location_search" {
            return self.search_locations(&values).await;
        }
        let mut path = tool.path.clone();
        for key in &tool.path_keys {
            let value = scalar(&values[key]);
            if matches!(value.as_str(), "." | "..") || value.contains(['/', '\\']) {
                return Err(ToolError::new(format!("路径参数 {key} 无效。")));
            }
            path = path.replace(
                &format!("{{{key}}}"),
                &utf8_percent_encode(&value, NON_ALPHANUMERIC).to_string(),
            );
        }
        let select = |keys: &[String]| -> Map<String, Value> {
            keys.iter()
                .filter_map(|key| {
                    values
                        .get(key)
                        .filter(|v| !v.is_null())
                        .map(|v| (key.clone(), v.clone()))
                })
                .collect()
        };
        let query = select(&tool.query_keys);
        let body = Value::Object(select(&tool.body_keys));
        if tool.mutation && values.get("confirm").and_then(Value::as_bool) != Some(true) {
            return Ok(
                json!({"status":"confirmation_required", "tool":name, "method":tool.method, "path":path, "query":query, "body":body}),
            );
        }
        let token = self.account.token()?.ok_or_else(|| {
            ToolError::new("尚未登录 Bulbul，请运行 login 或使用 bulbul_login 登录。")
        })?;
        let method = Method::from_bytes(tool.method.as_bytes())
            .map_err(|_| ToolError::new("工具 HTTP 方法无效。"))?;
        let params = query
            .iter()
            .map(|(key, value)| (key.clone(), scalar(value)))
            .collect::<Vec<_>>();
        let result = http_json(
            &self.account.http,
            method,
            &format!("{}{path}", self.account.api_url),
            tool.has_body.then_some(&body),
            Some(&token),
            &params,
        )
        .await?;
        Ok(limit_result(
            result,
            tool.result_limit.as_deref(),
            values.get("limit").and_then(Value::as_u64).unwrap_or(20) as usize,
        ))
    }

    async fn search_locations(&self, values: &Map<String, Value>) -> Result<Value> {
        let key = self.tianditu_key.as_ref().ok_or_else(|| {
            ToolError::new("地点搜索需要 BULBUL_TIANDITU_KEY，也可以请用户提供坐标。")
        })?;
        let query = values["query"].as_str().unwrap_or_default().trim();
        if query.is_empty() {
            return Err(ToolError::new("地点名称不能为空。"));
        }
        let limit = values.get("limit").and_then(Value::as_u64).unwrap_or(5) as usize;
        let post = json!({"keyWord":query,"level":"15","mapBound":"73.0,18.0,135.0,53.0","queryType":"1","count":limit.to_string()});
        let result = http_json(
            &self.account.http,
            Method::GET,
            &self.geocoder_url,
            None,
            None,
            &[
                ("postStr".into(), post.to_string()),
                ("type".into(), "search".into()),
                ("tk".into(), key.clone()),
            ],
        )
        .await?;
        if !result.is_object()
            || result.get("status").is_some_and(|status| {
                !status.is_object()
                    || status
                        .get("infocode")
                        .is_some_and(|code| scalar(code) != "1000")
            })
        {
            return Err(ToolError::new(
                "地点搜索服务返回无效结果，请检查天地图配置。",
            ));
        }
        let mut rows = Vec::new();
        if let Some(area) = result.get("area").filter(|v| v.is_object()) {
            rows.push(area);
        }
        if let Some(pois) = result.get("pois") {
            rows.extend(
                pois.as_array()
                    .ok_or_else(|| ToolError::new("地点搜索服务返回无效候选。"))?,
            );
        }
        let candidates = rows.into_iter().filter_map(|row| {
            let name = row.get("name")?.as_str().filter(|s| !s.is_empty())?;
            let (lon,lat) = row.get("lonlat")?.as_str()?.split_once(',')?;
            let lon: f64 = lon.parse().ok()?;
            let lat: f64 = lat.parse().ok()?;
            if !lon.is_finite() || !lat.is_finite() || !(-180.0..=180.0).contains(&lon) || !(-90.0..=90.0).contains(&lat) { return None; }
            Some(json!({"name":name,"lon":lon,"lat":lat,"address":row.get("address").cloned().unwrap_or(json!(""))}))
        }).take(limit).collect::<Vec<_>>();
        Ok(json!({"source":"tianditu","query":query,"candidates":candidates}))
    }
}

fn scalar(value: &Value) -> String {
    value
        .as_str()
        .map(str::to_owned)
        .unwrap_or_else(|| value.to_string())
}

fn limit_result(mut result: Value, key: Option<&str>, limit: usize) -> Value {
    let Some(key) = key else {
        return result;
    };
    if let Some(rows) = result.as_array() {
        return json!({"items":rows.iter().take(limit).collect::<Vec<_>>(), "total":rows.len(), "truncated":rows.len()>limit});
    }
    if let Some(object) = result.as_object_mut()
        && let Some(rows) = object.get_mut(key).and_then(Value::as_array_mut)
    {
        let total = rows.len();
        rows.truncate(limit);
        object.insert("total".into(), json!(total));
        object.insert("truncated".into(), json!(total > limit));
    }
    result
}
