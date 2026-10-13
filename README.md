# Bulbul 观鸟 skill

在 Codex、Claude Code 和 pi 中使用 Bulbul 的鸟种、地点预测、观鸟热点、百科、清单和练习数据，准备行程和学习识别鸟类。

## 安装

```bash
npx skills add simulacraliasing/bulbul-birding --skill bulbul-birding -g
```

按提示选择使用的工具，或追加 `-a codex claude-code pi`。已有 Git 的用户也可以直接克隆到对应工具的 skill 目录。

首次调用时，skill 的启动脚本会从本仓库的 GitHub Releases 下载对应平台的 Rust 程序，校验 SHA-256 后保存到本机。日常使用不需要 Node.js、Python 或 Rust，也不需要手动填写令牌。支持 Windows x64、macOS Intel / Apple Silicon、Linux x64 / ARM64。

登录通过 Bulbul 网页完成，凭据保存在系统凭据库。工具使用普通用户的权限和额度。Skill 可以直接调用命令行；需要 MCP 时，同一个程序也提供 stdio MCP 服务。

完整的登录、更新和 MCP 配置见 [接入说明](references/setup.md)，具体使用规则见 [SKILL.md](SKILL.md)。下载失败时可以查看 [Releases](https://github.com/simulacraliasing/bulbul-birding/releases)。

## 开发与发布

Rust 源码在 `cli/`，使用 Rust 1.95 或更高版本：

```bash
cd cli
cargo fmt --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
cargo build --release --locked
```

客户端在这个仓库独立维护，版本与 Bulbul 服务端分别管理。`VERSION` 是客户端版本的唯一来源，`assets/runtime.json`、Cargo manifest 和 lockfile 通过版本脚本同步。维护脚本需要 Python 3.12 或更高版本，使用项目虚拟环境：

```bash
uv venv .venv
# Windows 使用 .venv/Scripts/python.exe，macOS / Linux 使用 .venv/bin/python
.venv/bin/python scripts/manage_version.py check
.venv/bin/python scripts/manage_version.py sync
# 准备客户端发版时才更新版本
.venv/bin/python scripts/manage_version.py set 0.39.0
.venv/bin/python tests/test_maintenance.py
```

主分支和 pull request 会检查并构建五个平台；`v<version>` tag 在全部验证通过后发布程序和 SHA-256 文件。启动脚本固定使用 skill 对应版本，更新 skill 后才会切换程序版本。

工具参数来自 Bulbul 的 public v1 OpenAPI 契约。服务端更新接口后，先导出并提供这份公开契约文件，再在此仓库生成工具定义；传入路径即可，不需要检出 Bulbul 源码：

```bash
.venv/bin/python scripts/export_birding_tools.py --openapi /path/to/public-v1.json
.venv/bin/python scripts/export_birding_tools.py --openapi /path/to/public-v1.json --check
```

审阅 `assets/tools.json` 的变化，再运行 Rust 测试和构建。添加工具时在 `scripts/export_birding_tools.py` 的明确列表中选择公开业务接口，并更新 skill 说明和行为测试。业务 API、浏览器授权页面、服务端鉴权及数据库仍由 Bulbul 项目维护。

`tests/test_maintenance.py` 不需要服务端目录；启动脚本测试需要先编译程序，并通过 `BULBUL_TEST_BINARY` 指定程序路径。CI 在五个平台运行维护脚本、Rust 和启动脚本测试。
