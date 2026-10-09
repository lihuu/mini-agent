# Security Policy

[中文](SECURITY.md) · [English](SECURITY.en.md)

## `ma` 执行模型生成的 shell 命令

`ma` 会把模型生成的命令交给 `/bin/sh -c` 执行。内置的 Guard 只做**启动时固定的 best-effort 检查**,用于拦截常见误操作:

- 它**不是安全沙箱**,不提供任何 OS 级强制隔离。
- 未知 CLI、程序内部写入/联网、构建脚本、Git hooks、解释器(`python`/`node`/`perl`/`ruby`)、引号内的重定向、检查后发生的路径变化,都可能绕过 Guard。
- `--write` 只表示"意图上允许在启动 cwd 内写入",不代表越界写入被可靠阻止。
- `--net` 只按命令名拦截明显的网络命令;模型 API 请求始终放行。
- `--skills` 显式选中的 skill 指令会加入系统提示词,实际 skill 目录额外允许只读访问;不扩大写入或网络权限。请只选择信任的 skills,其指令和脚本仍受上述 Guard 局限影响。

**在不受信任的环境中使用时,请务必把 `ma` 放进容器或虚拟机,配合只读挂载与网络策略做真正的隔离。** 不要把它指向会被 prompt injection 影响的内容后仍期待 Guard 保护宿主。

## 支持的版本

只有 `main` 分支的最新提交在维护范围内。

## 报告漏洞

请使用 GitHub 的私下漏洞报告功能:仓库页面 → **Security** → **Report a vulnerability**(Private vulnerability reporting)。请不要在公开 issue 中披露未修复的绕过方式。

报告时请尽量包含:

- `ma` 的版本/提交与运行平台;
- 完整的 `shell(command)` 参数或触发 Prompt;
- 启动参数(`--write` / `--net`);
- 实际发生的越界写入/网络访问与预期行为。

## 非目标

以下不属于安全漏洞,不要报告:

- Guard 未拦截某个未列出的 CLI(已知的 best-effort 边界,见 README 与本节);
- 在 `--write --net` 下模型执行了破坏性命令(权限设计如此);
- 模型输出错误结论或声称成功(契约只保证 `final` 已取得,不保证任务成功)。
