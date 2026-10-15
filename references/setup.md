# 接入 Codex、Claude Code 和 pi

Skill 从公开仓库 [simulacraliasing/bulbul-skills](https://github.com/simulacraliasing/bulbul-skills) 安装。查看参数的 `list` 命令直接读取随 skill 分发的定义，无需下载、联网或登录。其他命令首次使用时，启动脚本下载同版本的 Rust 程序，校验 SHA-256 后保存在本机缓存，不需要编译，也不写入 skill 安装目录。

## 从 Git 安装

在终端执行：

```bash
npx skills add simulacraliasing/bulbul-skills --skill bulbul-birding -g
```

按提示选择使用的工具，也可以明确指定：

```bash
npx skills add simulacraliasing/bulbul-skills --skill bulbul-birding -g -a codex claude-code pi
```

`-g` 表示用户级安装，所有项目都能使用；去掉它则安装到当前项目。安装器负责选择 skill 目录，无需自己复制文件。这个安装方式需要 Node.js；日常使用的观鸟程序不需要 Node.js、Python 或 Rust。

已有 Git 时也可以直接克隆到 skill 目录。Codex 的用户级目录为 `~/.agents/skills/bulbul-birding`，Claude Code 为 `~/.claude/skills/bulbul-birding`，pi 为 `~/.pi/agent/skills/bulbul-birding`。下面以 Codex 为例：

```bash
git clone https://github.com/simulacraliasing/bulbul-skills.git ~/.agents/skills/bulbul-birding
```

更新安装器安装的 skill 使用 `npx skills update bulbul-birding -g`；克隆安装的目录使用 `git pull --ff-only`。新版本首次调用时会获取对应程序。

## 首次运行和登录

找到已安装的 skill 目录，在任意工作目录使用它的绝对路径。Windows 示例：

```powershell
powershell.exe -NoProfile -ExecutionPolicy Bypass -File C:/skills/bulbul-birding/scripts/bulbul.ps1 --version
powershell.exe -NoProfile -ExecutionPolicy Bypass -File C:/skills/bulbul-birding/scripts/bulbul.ps1 login
```

macOS / Linux 示例：

```bash
bash /absolute/path/bulbul-birding/scripts/bulbul.sh --version
bash /absolute/path/bulbul-birding/scripts/bulbul.sh login
```

把示例路径替换为实际安装目录。支持 Windows x64、macOS Intel / Apple Silicon、Linux x64 / ARM64。Linux x64 需要 Ubuntu 22.04 或兼容的系统环境，ARM64 需要 Ubuntu 24.04 或兼容的系统环境。下载需要连接 GitHub Releases；未发布的版本会给出提示。离线使用可以先在有网络时完成 `--version` 检查，或把同版本、同平台的程序放进 skill 的 `bin/`，命名为 `bulbul` 或 `bulbul.exe`。

登录会打开 Bulbul 网页。使用已有账号登录，对照终端显示的授权码，确认允许观鸟工具使用这个账号。默认连接 `https://bulbulbirding.cn`，普通用户不需要配置环境变量。密码由网页接收，不经过模型或命令行参数；浏览器无法打开时可以手动打开链接，或加 `--no-browser`。

下面用 `bulbul` 代表上述启动命令：

```bash
bulbul status
bulbul doctor
bulbul logout
bulbul list species_catalog location_search identify_photo
bulbul call species_catalog --arguments-file /absolute/path/arguments.json
```

参数文件是 UTF-8 JSON object，例如 `{ "q": "白头鹎", "limit": 5 }`。使用参数文件可以避免各个 shell 的 JSON 引号差异。写入工具默认返回预览，沿用用户已有授权后加 `--confirm` 执行；失败的写入不会自动重试。

`list` 可一次查看多个工具，输出仍为 JSON 数组；不带工具名时列出全部工具。命令失败会使用非零退出码，并在 stderr 返回包含 `error`、`code` 的 JSON；需要详情时读取 `details`。422 会提供安全的字段位置和错误类型，不回传输入值和原始服务错误。调用脚本后应保留 `$LASTEXITCODE`（PowerShell）或 `$?`（Bash），不能只凭有 JSON 输出判断成功。

登录令牌保存在 Windows 凭据管理器、macOS 钥匙串或 Linux Secret Service 中。KWallet 需要提供 Secret Service 接口；Linux 没有该服务时需先启用。后续 CLI 调用自动读取保存的登录。`logout` 撤销工具自己的会话，网页仍保持登录；会话过期后重新 `login`。

## 缓存和本机诊断

从 Git 项目目录调用时，程序优先保存在最近项目根目录的 `.tmp/bulbul/<版本>/<平台>/`。其他目录使用 Windows 的 `%LOCALAPPDATA%/bulbul-birding/cache/`、macOS 的 `~/Library/Caches/bulbul-birding/` 或 Linux 的 `$XDG_CACHE_HOME/bulbul-birding/`（未设置时为 `~/.cache/bulbul-birding/`）。同版本的 `bin/bulbul`、`bin/bulbul.exe` 和经过校验的旧安装缓存仍可读取使用。

AI 工具的权限限制仍适用；更换缓存位置不会自动取得访问权限。目录不可写时，先在可写的项目目录运行，也可以仅为当前进程指定项目内缓存：

```powershell
$env:BULBUL_CACHE_DIR = Join-Path (Get-Location) '.tmp/bulbul'
```

```bash
export BULBUL_CACHE_DIR="$PWD/.tmp/bulbul"
```

无需把这个设置写进全局配置。项目缓存可重建，不包含登录令牌。

`doctor` 只检查本机版本、站点、保存的会话是否存在和系统凭据库是否可访问，不连接 Bulbul 服务端、不改变登录，也不输出令牌。输出 `attention_required` 时查看 `issue`；`session_present: true` 不代表会话仍然有效，用 `status` 验证。

凭据库访问失败时，在自己的终端运行同一个 `doctor` 命令。如果终端正常而 AI 工具失败，检查 AI 工具的执行权限；不要因此重新安装凭据管理器或把令牌复制到聊天。两个环境都失败时，再检查系统凭据库是否启用、解锁及登录会话是否可用。

## 在 AI 工具中使用

安装 skill 后，Codex 可通过 `$bulbul-birding` 使用，pi 可通过 `/skill:bulbul-birding` 加载；Claude Code 可根据请求自动使用。Agent 按 `SKILL.md` 的说明调用 CLI，查询鸟种、地点、清单和练习数据。

## 从旧的 MCP 接入迁移

如果之前配置过 `bulbul-birding` MCP 服务，在当前 AI 工具的设置中移除这一项，保留已安装的 skill。系统凭据库中已有的账号登录可继续用于 CLI。

## 上传照片或声音

使用 `list identify_photo`、`list identify_audio` 查看参数，文件路径传入 `file`。文件会上传到 Bulbul 云端，识别不会下载本地模型。已知地点和日期时提供经纬度及 `captured_at`，无需配置模型路径或额外令牌。

声音支持 WAV、FLAC、MP3、M4A（AAC/ALAC）、AIFF 和 Ogg Vorbis；Opus 需先转成 WAV。照片限制 20 MB，声音完整文件限制 50 MB、0.5–300 秒。选区不能绕过时长上限，超过 5 分钟时先截取或拆分文件。CLI 仅对明确未受理的 429 按 Retry-After 最多重试 2 次，重试前累计自动等待最多 20 秒；超时或网络错误不重新上传。

## 按地名搜索

登录后可以直接使用 `location_search`，无需申请天地图账号或设置地图 API Key。先运行 `bulbul list location_search` 查看参数，参数文件示例：

```json
{"query":"昆明翠湖","limit":5}
```

地点搜索通过 Bulbul 服务端完成，返回候选名称、地址和经纬度。同名地点需要确认后再使用坐标，默认搜索范围为中国大陆。服务繁忙时按提示稍后再试，或使用已知坐标。

## 可选配置

| 变量                   | 用途                                                                        |
| ---------------------- | --------------------------------------------------------------------------- |
| `BULBUL_API_URL`       | 覆盖记住的站点地址；远程站点需要 HTTPS。也可在 `login` 时使用 `--api-url`。 |
| `BULBUL_SESSION_TOKEN` | 使用已有普通用户会话，优先于系统中保存的登录；浏览器登录前先移除。          |
| `BULBUL_CACHE_DIR`     | 仅在默认位置不可写等情况下，指定当前进程可访问的程序缓存目录。              |

这些变量需要由运行 CLI 的进程提供。Agent 调用 CLI 时会使用终端的环境变量，普通用户无需配置账号变量；不要把令牌写进共享配置、参数、聊天或 Git。

工具沿用普通用户权限和额度，不需要 `agent_preview` 角色、后端 LLM key 或数据库连接。

## 从源码构建

开发者使用 Rust 1.95 或更高版本，在 skill 的 `cli/` 目录执行：

```bash
cargo test --locked
cargo build --release --locked
```

将 `target/release/bulbul` 或 `bulbul.exe` 放进 skill 的 `bin/`。Windows 使用静态 CRT。工具定义从 `assets/tools.json` 编译到程序中；启动器的离线参数查询使用同源生成的 `assets/tools.jsonl`。更新契约时同时生成两者，重新构建程序。

公开仓库通过版本 tag 构建各平台程序，并上传到对应 GitHub Release。启动脚本按 `assets/runtime.json` 中的版本和仓库下载，不自动切换到 `latest`。客户端版本由本仓库的 `VERSION` 和 `scripts/manage_version.py` 管理，与 Bulbul 服务端分别维护。
