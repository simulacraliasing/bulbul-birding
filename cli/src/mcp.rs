use crate::{
    Result, ToolError,
    client::{BulbulClient, TOOLS},
};
use rmcp::{
    ErrorData, RoleServer, ServerHandler, ServiceExt, model::*, service::RequestContext,
    transport::stdio,
};
use serde_json::{Value, json};
use std::sync::Arc;

const AUTH_TOOLS: [(&str, &str); 4] = [
    ("bulbul_auth_status", "查看 Bulbul 当前登录状态和账号。"),
    (
        "bulbul_login",
        "开始 Bulbul 浏览器登录，返回授权码和登录链接。用户确认后调用 bulbul_login_complete。",
    ),
    (
        "bulbul_login_complete",
        "用户在浏览器确认后，完成登录并保存到系统凭据库。尚未确认时返回 pending。",
    ),
    (
        "bulbul_logout",
        "退出 Bulbul 工具登录，撤销当前工具会话并删除本机凭据。",
    ),
];

#[derive(Clone)]
pub struct BirdingServer(pub Arc<BulbulClient>);

impl BirdingServer {
    pub fn tools() -> Vec<Tool> {
        let mut tools = TOOLS
            .iter()
            .map(|tool| {
                let read_only = !tool.mutation && tool.name != "species_predict";
                Tool::new(
                    tool.name.clone(),
                    tool.description.clone(),
                    tool.input_schema
                        .as_object()
                        .expect("tool object schema")
                        .clone(),
                )
                .with_annotations(
                    ToolAnnotations::new()
                        .read_only(read_only)
                        .destructive(tool.name == "species_list_replace_items")
                        .idempotent(read_only)
                        .open_world(true),
                )
            })
            .collect::<Vec<_>>();
        tools.extend(AUTH_TOOLS.iter().map(|(name, description)| {
            Tool::new(
                *name,
                *description,
                json!({"type":"object","properties":{},"additionalProperties":false})
                    .as_object()
                    .unwrap()
                    .clone(),
            )
            .with_annotations(
                ToolAnnotations::new()
                    .read_only(*name == "bulbul_auth_status")
                    .destructive(*name == "bulbul_logout")
                    .idempotent(*name == "bulbul_auth_status")
                    .open_world(true),
            )
        }));
        tools
    }

    pub async fn dispatch(&self, name: &str, arguments: Value) -> Result<Value> {
        if AUTH_TOOLS.iter().any(|(tool, _)| *tool == name) {
            if arguments.as_object().is_none_or(|map| !map.is_empty()) {
                return Err(ToolError::new("登录工具不接收密码、令牌或其他参数。"));
            }
            match name {
                "bulbul_auth_status" => self.0.account.status().await,
                "bulbul_login" => self.0.account.begin_login(true).await,
                "bulbul_login_complete" => self.0.account.complete_login().await,
                "bulbul_logout" => self.0.account.logout().await,
                _ => unreachable!(),
            }
        } else {
            self.0.call(name, arguments).await
        }
    }
}

impl ServerHandler for BirdingServer {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new(
                "bulbul-birding",
                env!("CARGO_PKG_VERSION"),
            ))
            .with_instructions(
                "使用普通用户登录查询观鸟数据；写入先预览，取得用户授权后传 confirm=true。",
            )
    }

    async fn list_tools(
        &self,
        _: Option<PaginatedRequestParams>,
        _: RequestContext<RoleServer>,
    ) -> std::result::Result<ListToolsResult, ErrorData> {
        Ok(ListToolsResult::with_all_items(Self::tools()))
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        _: RequestContext<RoleServer>,
    ) -> std::result::Result<CallToolResponse, ErrorData> {
        match self
            .dispatch(
                &request.name,
                Value::Object(request.arguments.unwrap_or_default()),
            )
            .await
        {
            Ok(result) => {
                let structured = if result.is_object() {
                    result
                } else {
                    json!({"items":result})
                };
                Ok(CallToolResult::structured(structured).into())
            }
            Err(error) => {
                Ok(CallToolResult::error(vec![ContentBlock::text(error.to_string())]).into())
            }
        }
    }
}

pub async fn run(client: BulbulClient) -> Result<()> {
    let service = BirdingServer(Arc::new(client))
        .serve(stdio())
        .await
        .map_err(|_| ToolError::new("MCP 初始化失败，请检查 harness 的 stdio 配置。"))?;
    service
        .waiting()
        .await
        .map_err(|_| ToolError::new("MCP 连接已中断。"))?;
    Ok(())
}
