---
name: bulbul-birding
description: 使用 Bulbul 的真实鸟种、观鸟热点、地点预测、百科、用户清单和练习数据，帮助用户准备观鸟行程、学习和练习。适用于行前预习、按目标鸟种找地点、读取最近查询或保存鸟种清单。
---

# Bulbul 观鸟准备

使用 Bulbul 工具查询实际数据，再根据用户的地点、日期和目标鸟种整理建议。模型、会话历史和推理由当前 harness 提供。

## 使用工具

优先使用已连接的 `bulbul-birding` MCP 工具。没有 MCP 时，使用本 skill 目录中的命令行入口，两种方式具有相同的工具和参数。

```bash
bash <skill绝对路径>/scripts/bulbul.sh list species_predict
bash <skill绝对路径>/scripts/bulbul.sh call species_predict --arguments-file <参数文件.json>
```

把路径替换为本 skill 的实际目录；Windows 使用 `powershell.exe -NoProfile -ExecutionPolicy Bypass -File <skill绝对路径>/scripts/bulbul.ps1`，后面接相同参数。启动脚本会按本 skill 的版本从公开仓库下载对应平台的 Rust 程序，核对 SHA-256 后运行，后续直接使用本机缓存。所有提示输出到 stderr，stdout 保留给业务 JSON 或 MCP。业务程序是 Rust 单文件，不需要 Python、Node.js、uv 或 Rust 编译环境，也不依赖当前工作目录。首次安装、登录或配置 MCP 时读取 [接入说明](references/setup.md)。普通用户通过浏览器登录，工具自动从系统凭据库读取账号；无需填写会话令牌。密码和令牌不放进聊天、参数文件或 Git，也不请求用户把登录响应发送给模型。

尚未登录或遇到 401 时，使用 `bulbul_login` 开始登录，把授权码和链接告诉用户；用户在浏览器确认后再调用 `bulbul_login_complete`。返回 `pending` 时等待用户完成确认，不反复轮询。没有 MCP 时，请用户在自己的终端运行 `bulbul login`，可以用 `--no-browser` 在另一台设备上打开登录链接。`bulbul_auth_status` / `bulbul status` 查看当前账号；用户要求退出工具账号时使用 `bulbul_logout` / `bulbul logout`。登录、状态查询和退出都不返回令牌，不用服务令牌替代用户登录。

参数以 MCP 的 input schema 或 `list <工具名>` 输出为准。参数文件必须是 JSON object；命令行的 `--arguments-file` 可以避免 PowerShell、Bash 对 JSON 引号的不同处理。

## 整理观鸟建议

- 用户只给地名时，先用 `location_search` 查询候选。它使用天地图，需要 `BULBUL_TIANDITU_KEY`。地点重名或有多个合理候选时，请用户选择；没有地理编码配置时请用户提供坐标，不猜经纬度。天地图默认搜索范围为中国大陆，境外地点使用用户提供或其他可用工具查到的坐标。
- 根据用户日期传入 `target_date`，格式为 `YYYY-MM-DD`；“明天”等相对日期按用户时区解释。
- 用 `species_catalog` 把名称对齐到项目内 `taxon_id`，后续鸟种查询和清单关联都使用这个 ID。学名、中文名和外部代码用于展示或查找。
- 地点预习用 `species_predict`，附近地点用 `hotspots_nearby`，按目标鸟种寻找目的地用 `hotspots_target_destinations`。预测会像网页一样保存当前用户的最近查询。
- 用 `species_wiki`、`species_detail` 读取识别特征和相似种。返回 `pending` 时说明资料尚未就绪，不补造百科或分布结果。
- 用户提到最近查询或已有清单时，使用 `saved_queries`、`species_lists`、`species_list_detail`。最近查询使用返回的 `snapshots[].id`，作为后续请求的 `snapshot_id` 或 `saved_query_id`；清单使用返回的 `list_id`。结果含 `truncated` 或分页参数时，说明覆盖范围，需要时继续读取。
- 建议区分预测、历史记录和实际观察，不把概率当作保证能看到。保留工具返回的来源和日期，地点坐标、鸟种 ID 和清单 ID 都来自真实结果。

## 清单和练习

`quiz_route_book_create`、`quiz_custom_batch_create` 和 `species_list_*` 写入工具默认返回 `confirmation_required` 及操作参数，不执行请求。

用户已经明确要求创建练习、保存清单或替换指定清单时，沿用已有授权，用相同参数加 `confirm: true` 执行；命令行也可以使用 `--confirm`。只有目标清单、内容或覆盖范围尚不清楚时，才先展示具体方案并确认。仅询问推荐或预习建议不表示授权替换清单。

替换清单前读取 `species_list_detail`，说明会替换全部鸟种，保留原有 taxon_ids 以便按需恢复。创建后使用返回的 ID，并根据需要再读取清单确认内容。写入请求失败或超时后先检查是否已完成，不自动重试。当前用户的权限、额度和数据归属由 Bulbul API 校验。
