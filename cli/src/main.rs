use bulbul_birding::{
    Result, ToolError,
    client::{BulbulClient, find_tool, list_tools},
    mcp,
};
use clap::{Parser, Subcommand};
use serde_json::{Value, json};
use std::{
    io::{self, Write},
    path::PathBuf,
    process::ExitCode,
};

#[derive(Parser)]
#[command(
    name = "bulbul",
    version,
    about = "Bulbul 观鸟工具：浏览器登录、查询、清单和 MCP"
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// 在浏览器中登录，自动保存到系统凭据库
    Login {
        /// 指定其他 Bulbul 站点，登录成功后记住该地址
        #[arg(long)]
        api_url: Option<String>,
        /// 显示登录链接，由用户自行打开
        #[arg(long)]
        no_browser: bool,
    },
    /// 查看当前登录账号
    Status,
    /// 撤销工具会话并删除本机凭据
    Logout,
    /// 查看可用工具和参数 schema
    List { tool: Option<String> },
    /// 调用工具，结果输出为 JSON
    Call {
        tool: String,
        /// JSON object；跨平台调用推荐使用 --arguments-file
        #[arg(long, conflicts_with = "arguments_file")]
        arguments: Option<String>,
        /// UTF-8 JSON 参数文件
        #[arg(long)]
        arguments_file: Option<PathBuf>,
        /// 用户已经授权本次写入或练习创建
        #[arg(long)]
        confirm: bool,
    },
    /// 启动 stdio MCP 服务，stdout 仅传输协议消息
    Mcp,
}

async fn run(cli: Cli) -> Result<Option<Value>> {
    match cli.command {
        Commands::List { tool } => Ok(Some(list_tools(tool.as_deref())?)),
        Commands::Login {
            api_url,
            no_browser,
        } => Ok(Some(
            BulbulClient::from_environment(api_url.as_deref())?
                .account
                .login(!no_browser)
                .await?,
        )),
        Commands::Status => Ok(Some(
            BulbulClient::from_environment(None)?
                .account
                .status()
                .await?,
        )),
        Commands::Logout => Ok(Some(
            BulbulClient::from_environment(None)?
                .account
                .logout()
                .await?,
        )),
        Commands::Mcp => {
            mcp::run(BulbulClient::from_environment(None)?).await?;
            Ok(None)
        }
        Commands::Call {
            tool,
            arguments,
            arguments_file,
            confirm,
        } => {
            let raw = match arguments_file {
                Some(path) => std::fs::read_to_string(path)
                    .map_err(|_| ToolError::new("无法读取参数文件，请提供 UTF-8 JSON object。"))?,
                None => arguments.unwrap_or_else(|| "{}".into()),
            };
            let mut arguments: Value = serde_json::from_str(raw.trim_start_matches('\u{feff}'))
                .map_err(|_| ToolError::new("参数必须是有效的 JSON object。"))?;
            let map = arguments
                .as_object_mut()
                .ok_or_else(|| ToolError::new("参数必须是 JSON object。"))?;
            if confirm {
                if !find_tool(&tool)?.1.mutation {
                    return Err(ToolError::new("--confirm 只用于写入或练习创建工具。"));
                }
                map.insert("confirm".into(), json!(true));
            }
            Ok(Some(
                BulbulClient::from_environment(None)?
                    .call(&tool, arguments)
                    .await?,
            ))
        }
    }
}

#[tokio::main]
async fn main() -> ExitCode {
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(error)
            if matches!(
                error.kind(),
                clap::error::ErrorKind::DisplayHelp | clap::error::ErrorKind::DisplayVersion
            ) =>
        {
            let _ = error.print();
            return ExitCode::SUCCESS;
        }
        Err(_) => {
            eprintln!(
                "{}",
                json!({"error":"命令参数无效，请运行 bulbul --help 查看用法。"})
            );
            return ExitCode::from(2);
        }
    };
    tokio::select! {
        result = run(cli) => match result {
            Ok(Some(value)) => {
                if writeln!(io::stdout().lock(), "{value}").is_err() { return ExitCode::from(1); }
                ExitCode::SUCCESS
            }
            Ok(None) => ExitCode::SUCCESS,
            Err(error) => { eprintln!("{}", json!({"error":error.to_string()})); ExitCode::from(1) }
        },
        _ = tokio::signal::ctrl_c() => { eprintln!("{}", json!({"status":"cancelled", "message":"已取消登录或操作。"})); ExitCode::from(130) }
    }
}
