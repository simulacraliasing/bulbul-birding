# Changelog

## 0.39.0 - 2026-10-13

观鸟 skill 统一通过 CLI 使用。保留 `login`、`status`、`logout`、`list` 和 `call`，以及原有的 21 项业务工具。

- 移除 stdio MCP 服务和 `bulbul mcp` 子命令，删除协议依赖及相关测试。
- 更新 skill、README 安装提示词和接入说明，使用 CLI 的参数 schema 和 JSON 结果。
- 浏览器登录和系统凭据库继续使用。旧 MCP 用户移除对应的服务配置后，可以通过 skill 调用 CLI，无需重新登录。

从 Git 更新 skill 后，启动脚本会下载并校验 v0.39.0 程序。迁移方法见 [接入说明](references/setup.md#从旧的-mcp-接入迁移)。
