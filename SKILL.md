---
name: bulbul-birding
description: 识别用户提供的鸟类照片或鸟声文件，安排观鸟行程、按目标鸟种找地点、学习鸟种，或读取最近查询、保存鸟种清单和创建练习时使用。通过 Bulbul CLI 查询真实的鸟种、热点、地点预测、百科和用户数据。
---

# Bulbul 观鸟工具

使用 Bulbul 的真实结果回答用户。照片和声音识别读取 [识别说明](references/recognition.md)，行程和地点推荐读取 [行程说明](references/trip-planning.md)。首次安装、登录或启动失败时读取 [接入说明](references/setup.md)。

## 调用 CLI

使用本 skill 的绝对路径。先批量查看本次需要的工具参数，按返回的 `input_schema` 调用；同一会话复用已经读过的 schema，无需每次调用前重新查询。

```bash
bash <skill绝对路径>/scripts/bulbul.sh list identify_photo species_wiki
bash <skill绝对路径>/scripts/bulbul.sh call identify_photo --arguments-file <参数文件绝对路径.json>
```

Windows 使用 `powershell.exe -NoProfile -ExecutionPolicy Bypass -File <skill绝对路径>/scripts/bulbul.ps1`，后面接相同参数。路径含空格时加引号。`list` 直接读取随 skill 分发的定义，无需下载、联网或访问凭据库。其他命令首次运行会下载并校验同版本的 Rust 程序，优先缓存在当前 Git 项目的 `.tmp/bulbul/`；不在 Git 项目时使用系统用户缓存。日常使用无需 Python、Node.js、uv 或 Rust。

参数文件使用 UTF-8 JSON object，保存在当前项目可写的临时目录。`--arguments-file` 避免不同 shell 的 JSON 引号问题。stdout 为业务 JSON，stderr 为提示或错误；需要限制输出时保留真实退出码，例如 PowerShell：

```powershell
$result = & powershell.exe -NoProfile -ExecutionPolicy Bypass -File "<skill绝对路径>/scripts/bulbul.ps1" list location_search hotspots_nearby species_predict hotspots_target_destinations 2>&1
$code = $LASTEXITCODE
$text = $result | Out-String
$text.Substring(0, [Math]::Min(12000, $text.Length))
exit $code
```

输出截断时按需缩小查询或读取参数定义，不能把截断的 JSON 当作完整结果。命令失败时检查退出码和错误 JSON；422 的 `details.validation` 给出安全的字段位置和错误类型，修正参数后再调用。

## 登录与恢复

普通用户无需配置环境变量、地图 Key 或模型。CLI 自动读取系统凭据库中的账号。直接执行需要的查询；提示 `login_required`、`session_expired` 或 401 时，引导用户在自己的终端运行启动脚本的 `login`，在浏览器完成授权。程序自动等待并保存登录；`--no-browser` 显示可手动打开的链接。`status` 验证当前账号，`logout` 用于用户要求退出工具账号时。

启动或凭据访问失败时先用 `doctor` 检查本机版本、站点和凭据访问；它不请求服务端、不改变登录，也不返回令牌。本机有会话不代表服务端仍有效。当前进程被拒绝访问凭据库时，让用户在自己的终端运行同一命令对照；终端正常则检查 harness 权限，不把沙箱限制解释成没有安装凭据管理器。

缓存不可写时使用项目内可写目录，必要时按接入说明指定缓存位置；访问权限仍由 harness 决定。密码和令牌不放进聊天、参数文件或 Git，不请求用户回传登录响应，不用服务令牌或明文文件代替系统凭据库。

## 数据依据

鸟种名称用 `species_catalog` 对齐到项目内 `taxon_id`，后续鸟种查询和清单关联使用这个 ID。坐标、鸟种 ID、查询 ID 和清单 ID 来自实际结果。区分预测、历史记录和实际观察；保留来源、日期和校准状态，不把分数当作保证能看到或已确认识别。

最近查询使用 `saved_queries` 返回的 `snapshots[].id`，作为 `snapshot_id` 或 `saved_query_id`；已有清单使用 `species_lists`、`species_list_detail` 返回的 `list_id`。结果含 `truncated` 或分页时说明覆盖范围，按需求继续读取。百科返回 `pending` 时说明资料尚未就绪，不补造百科或分布结果。

## 清单和练习

`quiz_route_book_create`、`quiz_custom_batch_create` 和 `species_list_*` 写入工具默认返回 `confirmation_required` 及具体操作参数，不执行请求。

用户已经明确要求创建练习、保存清单或替换指定清单时，沿用授权，用相同参数加 `confirm: true` 或 CLI 的 `--confirm` 执行。目标清单、内容或覆盖范围不清楚时，先整理具体方案再确认。仅询问推荐不表示授权替换清单。

替换前读取完整清单（包括分页），说明会替换全部鸟种，保留原有 `taxon_ids` 便于恢复。创建后使用返回的 ID，按需读取确认。写入失败或超时后先检查是否已完成，不自动重试。用户权限、额度和数据归属由 Bulbul API 校验。
