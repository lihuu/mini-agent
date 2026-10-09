# 发版流程

发版由 tag 触发，`.github/workflows/release.yml` 自动完成校验、构建和发布。人工只做三件事：改版本号、提交、打 tag 并推送。

## 步骤

```sh
# 1. 改版本号（唯一来源是 Cargo.toml）
$EDITOR Cargo.toml              # version = "0.3.4"
cargo build --offline           # 同步 Cargo.lock 里的 mini-agent 版本

# 2. 提交（与功能提交分开，保持历史可读）
git add Cargo.toml Cargo.lock
git commit -m "Release v0.3.4"

# 3. 附注 tag + 推送
git tag -a v0.3.4 -m "v0.3.4 — <一句话说明这次发布的核心变化>"
git push origin main
git push origin v0.3.4
```

推送 tag 后工作流自动运行，约两分钟出结果。

## 推送前必须本地验证

CI 只在 ubuntu 上跑 verify，**有些问题本地 macOS 看不出来**。发版前的固定动作：

```sh
cargo fmt --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked --release
```

历史教训：0.3.2 的 tag 推送后 verify 失败，因为单元测试把发布产物名硬编码成了 `aarch64-apple-darwin`，而 CI 在 Linux 上运行时 `release_target()` 返回 `x86_64-unknown-linux-gnu`。**凡是与平台相关的断言都要用 `release_target()` 推导，不要写死。** 这次失败也证明了 verify 这道门有效：它拦下了构建，没有产出任何产物。

## 工作流做了什么

| Job | 内容 |
| --- | --- |
| `verify` | ubuntu-latest：fmt、clippy（`-D warnings`）、`cargo test --release`，并校验 **tag 与 `Cargo.toml` 版本一致** |
| `build` | 矩阵构建 macos-latest（`aarch64-apple-darwin`）与 ubuntu-latest（`x86_64-unknown-linux-gnu`），各打成 `ma-<版本>-<target>.tar.gz`，内含 `ma`、`LICENSE`、`README.md`、`SECURITY.md`，并检查二进制 < 10 MB |
| `release` | 汇总产物，用 `softprops/action-gh-release` 直接发布并生成 changelog |

`verify` 失败则 `build` 不运行（`needs: verify`），因此版本号写错或测试挂掉时不会发出任何产物。

发布是**直接公开**（非草稿）。质量由 `verify` 兜底，而不是靠人工点发布。

### tag 必须与 Cargo.toml 一致

`verify` 里这一步是有意加的：

```sh
version=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
test "v$version" = "$GITHUB_REF_NAME"
```

它把「记得改版本号」从人的习惯变成机器的检查。否则会出现 tag 写 `v0.4.0`、二进制却自称 `0.3.1` 的情况。

### tag 触发读的是被 tag 的那个 commit

`on: push: tags` 触发时，GitHub 读的是**被 tag 的那个 commit 里的**工作流文件，不是分支最新版。所以给一个早于 `release.yml` 的 commit 补打 tag 不会触发构建，也不能靠重推 tag 解决（除非强推改写 tag，不该做）。**只能发新版本。**

## 用户如何更新

三种途径，产物相同：

```sh
./scripts/install.sh        # 源码安装到 ~/.local/bin
ma --update                 # 已安装的 ma 自我更新到最新发布版
# 或直接从 Releases 页面下载 tarball
```

`ma --update` 的可靠性来自「用产物本身验证」：下载后先执行候选二进制的 `--version`，只有版本与 API 公布的一致才替换，任何一步失败都清理临时文件并保留原二进制。详见[使用说明](usage.md#更新)。

## 平台支持范围（实测）

### macOS

产物为 `minos 11.0`（Big Sur 及以上），只链接 `/usr/lib` 下的系统库（`libSystem`、`libiconv`）。macOS 不使用符号版本机制，因此**不存在 Linux 那种 glibc 版本墙**：只要系统版本达标即可运行。

### Linux

产物为 `x86_64-unknown-linux-gnu`（动态链接 glibc），在以下环境实测：

| 系统 | glibc | 结果 |
| --- | --- | --- |
| Ubuntu 24.04 | 2.39 | 正常运行 |
| Debian 12 | 2.36 | 正常运行 |
| Ubuntu 22.04 | 2.35 | 正常运行 |
| Ubuntu 20.04 | 2.31 | 无法运行 |
| Debian 11 | 2.31 | 无法运行 |
| Alpine | musl | 无法运行（非 glibc） |

**下界是 glibc 2.34。** Ubuntu 22.04 及更新的 glibc 发行版可用；20.04、Debian 11 及更老的版本不行。

关于构建机上较高的 glibc（当前 CI 用 ubuntu-latest，即 24.04）：

- 产物里确实带有 `GLIBC_2.39` 的符号引用（`pidfd_spawnp`、`pidfd_getpid`，glibc 2.39 新增的进程创建接口），但它被记为 **WEAK**（弱符号），ld.so 找不到时只绑成 NULL，不会中止加载。
- 真正导致 20.04 失败的是 `GLIBC_2.32` / `2.33` / `2.34` 这几个**强需求**，与 2.39 无关。
- 因此 2.35/2.36 的系统会出现一行 `weak version GLIBC_2.39 not found` 警告，但**功能正常**。这行警告来自 ld.so，程序自身无从抑制。

当前不打算降低 glibc 下界（例如改用 `ubuntu-22.04` 构建）。若日后需要支持 Ubuntu 20.04 / Debian 11，把 `build` 矩阵里的 `ubuntu-latest` 换成 `ubuntu-22.04` 即可，产物下界随之下移。

### `--version` 是机器契约，不要加注解

`--version` 的 **stdout 必须恰好是 `ma <版本>` 一行**，不能加括号、后缀或第二行。

原因是升级链路依赖它：`ma --update` 用候选二进制的 `--version` 输出做严格相等校验，而**已发布的旧版本用的是全行比较**。只要这行长大一点点，所有旧版本的 `ma --update` 就会永久拒绝新版本 —— 用户再也升不上来。

平台信息因此走 **stderr**，与项目其它部分一致（stdout 给机器、stderr 给人）：

```
$ ma --version
ma 0.3.4                                              ← stdout，机器读
ma: built for aarch64-apple-darwin, requires macOS 11+  ← stderr，人读
```

`ma --update` 只读 stdout，所以两不干扰。修改 `--version` 前先看 `version_line_is_exactly_the_bare_form_older_binaries_compare_against` 这个测试，它把这条约束固定住了。

### 没有产物的平台

`ma --update` 依赖 `release_target()` 的映射，目前只覆盖 macOS（arm64 / x86_64）与 Linux（x86_64 / aarch64）。其他平台会明确报「no prebuilt binary is published for this platform」，而不是下载到错误架构的产物。

## 本地打包（不使用 CI 时）

`dist/` 已被 `.gitignore` 忽略。手工复现 CI 的打包：

```sh
version=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
triple=$(rustc -vV | sed -n 's/^host: //p')
name="ma-$version-$triple"
mkdir -p "dist/$name"
cp target/release/ma LICENSE README.md SECURITY.md "dist/$name/"
chmod +x "dist/$name/ma"
(cd dist && tar -czf "$name.tar.gz" "$name")
```

归档内的路径形态（`ma` 还是 `<目录>/ma`）两种都被 `ma --update` 的提取逻辑接受。
