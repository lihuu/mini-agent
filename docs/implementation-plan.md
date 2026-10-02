# Minimal Agent V0 Implementation Plan

**Goal:** 实现已确认设计中的一次性 `ma` executable。

**Architecture:** 单 Rust 源文件，普通函数依次处理参数、模型请求、Guard、shell 和循环。无异步运行时、SDK 或插件抽象。

**Tech Stack:** Rust 2024、ureq（同步 HTTP + rustls）、serde_json、libc（Unix 进程控制）。

**Spec:** [design.md](design.md)。由当前会话直接执行；用户已授权需求明确后开工，无需再次选择执行方式。

## Global Constraints

- 唯一 `shell` 工具与 OpenAI-compatible Chat Completions；权限在启动时确定。
- `--write` 仅启动 cwd 及其子目录，Guard 为 best-effort。
- 产品代码 `src/main.rs`；release 硬限制 <10 MB，目标 <5 MB。

## Review Focus

- 管道输入和错误不得污染 stdout。
- 工具输出超限或后代保持管道开放不得造成永久等待。
- 重定向、父目录路径和 symlink 不得通过明显越界写检查。
- 参数格式错误、畸形响应或模型返回未知工具不得意外执行。
- 步数耗尽不得再执行无法反馈的副作用。

## 执行步骤

- [x] 创建真实 CLI / mock HTTP 集成测试；空实现运行，确认失败来自行为缺失。
- [x] 实现参数解析、输入边界、HTTP / JSON 与唯一工具定义。
- [x] 为 Guard 与 shell 写失败测试，再实现共享 Policy、明显违规拒绝、cwd 路径检查、进程组超时及限量输出。
- [x] 接通多轮循环；集成测试检查 assistant/tool 配对、多工具执行、拒绝反馈、错误退出和 max_steps。
- [x] 补充用户说明及 macOS/Linux CI，运行 fmt、clippy、测试、release 构建和体积检查。
- [x] 复查实现与原始需求，明确本机、Linux CI 与私人模型 endpoint 的验证边界。

## 2026-10-02 验收结果

| 环境 | 测试 | Release 体积 | 运行 |
| --- | --- | --- | --- |
| macOS ARM64，Rust 1.91.1 | 20 / 20 通过；fmt、clippy 通过 | 1,287,664 字节 | help / version 通过 |
| 一次性 Alpine Linux ARM64 容器，Rust 1.96.1 | 同一源码快照，20 / 20 通过 | 1,446,624 字节 | version 通过 |

独立审查发现的 cd 子 shell、CDPATH、下载写路径、新 symlink 空窗、中断后残留工具均已修复，并用实际 CLI / shell 回归验证。审查者只读复查确认修复无明显问题。

Linux 容器已自动删除，源码挂载只读。GitHub CI 配置已添加，尚未在远端执行；Linux x86_64、其他发行版、Windows、Rust 1.85 下的实际构建和私人模型 endpoint 均未验证。V0 Guard 的已知非沙箱边界见 README。没有执行安装、发布、Git 提交或宿主配置修改。

## 2026-10-02 流式与 verbose 增量验收

- 上游请求默认 `stream: true`，同步 SSE 聚合；直接 JSON 响应仍兼容。工具参数完整接收后执行，断流不执行部分工具。
- `--verbose` / `-v` 默认关闭；开启后实时向 stderr 打印模型文本、轮次、耗时、shell 命令、输出及结果，stdout 仍只输出完整最终回答。
- 兼容 BOM、LF/CRLF/CR 和不冲突的扩展元数据回放；满 stderr 管道可丢弃可选日志，保持工具超时生效。
- 保留用户调整的默认 `max_steps = 200`；没有新增依赖。

| 环境 | 测试 | Release 体积 | 运行 |
| --- | --- | --- | --- |
| macOS ARM64，Rust 1.91.1 | 31 / 31 通过；fmt、clippy 通过 | 1,320,784 字节 | help / version 通过 |
| 一次性 Alpine Linux ARM64，Rust 1.96.1 | 与当前源码一致的只读快照，31 / 31 通过 | 1,512,160 字节 | version 通过 |

测试包含上游结束前可见的模型文本、shell 退出前可见的工具输出、满 stderr 管道下仍按时终止工具，以及扩展元数据回放和冲突拒绝。独立只读复查确认上述三个边界问题已修复，无新增实质阻断发现。容器自动删除，源码快照已清理；真实模型 endpoint 和远端 CI 仍未验证。

## 2026-10-02 上下文过长裁剪兜底

用户确认先做裁剪，不引入模型摘要。正常运行只追加历史；明确上下文错误时按完整工具轮次删除最旧的一批历史，始终保留系统指令、原始任务和最新轮次，插入一条可替换的裁剪提示，然后重试模型一次。重试计入原有请求上限，不重放已处理的 shell；无可裁剪历史或重试仍失败则退出。HTTP 错误体限制 8 KiB，普通 400、401、429、5xx 和超时不触发恢复；JSON/SSE 的明确 error 同样识别。

本机 macOS ARM64：40 / 40 CLI 集成测试通过，fmt、clippy、release build 和文档链接/命令语法检查通过；release 为 1,320,800 字节。新增测试验证整轮配对、多工具最新轮次保留、再次恢复不累积提示、写操作不重复、错误识别、一次重试边界、请求上限和 SSE 部分工具不执行。没有新增依赖或配置项。

此增量尚未重新执行 Linux、远端 CI 或真实模型 endpoint；此前 Linux 验收仅覆盖当时的源码快照。

独立只读审查未发现实质阻断，并额外用现有二进制验证 200 JSON 上下文错误、422 上下文错误、超过 8 KiB 的 HTTP 错误体、普通 JSON 错误和恢复重试后 500 退出五种 mock 场景。
