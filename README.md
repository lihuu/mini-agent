# Minimal Agent

**One model. One tool. One loop.**

`ma` 是一个一次性运行的命令行 Agent：接收任务，调用模型，使用宿主已有的 shell 工具完成工作，输出回答后退出。支持 macOS / Linux，运行时只需要一个二进制文件。

## 安装

预编译二进制在 [Releases](https://github.com/lihuu/mini-agent/releases) 页面，下载对应平台后解压即可，无需 Rust：

| 平台 | 产物 |
| --- | --- |
| macOS (Apple Silicon) | `ma-<版本>-aarch64-apple-darwin.tar.gz` |
| Linux (x86_64) | `ma-<版本>-x86_64-unknown-linux-gnu.tar.gz` |

```sh
tar -xzf ma-<版本>-aarch64-apple-darwin.tar.gz
mv ma-<版本>-aarch64-apple-darwin/ma ~/.local/bin/
ma --help
```

从源码安装需要 Rust 1.85+ 和 C 编译器：

```sh
git clone https://github.com/lihuu/mini-agent.git
cd mini-agent
./scripts/install.sh
ma --help
```

脚本构建 release 版本并安装到 `~/.local/bin`。选这个目录是因为它在 XDG 用户 bin 约定内，且在 `PATH` 上通常排在 `~/.cargo/bin` 之前，后来的同名命令不会静默遮蔽它。若旧副本由 cargo 管理，脚本会一并移除；不是 cargo 管理的文件绝不触碰。

`--prefix DIR` 改安装根（二进制落在 `DIR/bin`），`--uninstall` 卸载，`--offline` 禁止 cargo 联网。等价的手工命令是 `cargo install --path . --locked --root ~/.local`。

运行 `ma` 无需安装 Rust；它使用任务所需的宿主 CLI。

## 配置模型

使用支持工具调用的 OpenAI-compatible Chat Completions API，将下面的地址、模型和密钥替换为你的实际配置：

```sh
export BASE_URL='https://your-endpoint.example/v1'
export MODEL='your-model'
export API_KEY='your-key'
```

`BASE_URL` 是 API 根地址，程序会追加 `/chat/completions`。也可以将连接设置保存到配置文件，见[使用说明](docs/usage.md#配置与输入)。

## 使用

先进入要处理的项目目录。`ma` 以**启动时的当前目录**为工作目录：

```sh
cd /path/to/your/project
ma -v '解释这个项目的结构'
```

`-v` 实时显示模型文本和命令执行过程；最终回答写入 stdout，过程写入 stderr。

```sh
# 从 stdin 接收补充上下文
git diff | ma '总结这些修改'

# 允许修改当前目录及其子目录
ma -v --write '修复配置文件'

# 同时允许 shell 联网
ma -v --write --net '升级项目依赖'

# 按名称加载指定 skills；多个名称用英文逗号分隔
ma --skills code-review,explain '评审并解释这些修改'
```

管道输入只存在于对话里，不落盘。上下文超限时程序按代价从小到大恢复：先压缩旧轮次、再截断最新结果、最后才截断原始输入；最后一步会无条件在 stderr 告警，因为被省略的部分无法再读回来。详见[使用说明](docs/usage.md#上下文过长恢复)。

默认拒绝常见写操作和 shell 网络命令，模型 API 请求始终允许。内置 Guard 是尽力而为的命令检查，**不是安全沙箱**；需要强隔离时使用容器或虚拟机。详见[安全说明](SECURITY.md)。

完整参数、配置文件和输出约定见[使用说明](docs/usage.md)，或运行 `ma --help`。

`--skills` 按目录名从当前项目的 `.agents/skills/`、然后 `~/.agents/skills/` 查找 `SKILL.md`；项目优先，找不到或无法读取的跳过。不传此参数就不加载任何 skill。指定 skills 的完整指令会加入提示词，引用文件按需读取；自定义目录参数尚未支持。详见[skills 使用说明](docs/usage.md#skills)。

## License

[MIT](LICENSE)。
