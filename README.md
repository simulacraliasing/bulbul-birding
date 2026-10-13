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

`VERSION`、`assets/runtime.json`、Cargo manifest 和 lockfile 的版本需要一致。主分支和 pull request 会检查并构建五个平台；`v<version>` tag 在全部验证通过后发布程序和 SHA-256 文件。启动脚本固定使用 skill 对应版本，更新 skill 后才会切换程序版本。

维护者从 Bulbul 项目通过 `tools/birding/export_public_skill.py` 导出明确列出的文件，审阅差异后提交到这里。这个仓库具有独立的 Git 历史，包含观鸟工具所需的源码和数据结构。
