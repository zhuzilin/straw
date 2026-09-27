# 二进制 wheel 与源码构建

[English](BUILDING.md)

## 使用者：安装二进制包

从 PyPI 安装 `straw-queue`。在支持的平台上，pip 会选择包含 Rust 扩展的 wheel；安装 wheel 无需 Cargo、rustc、maturin 或源码树。Python 导入名为 `straw`。

```sh
pip install straw-queue
python -c 'import straw; from straw.tensor import TensorRef'
```

发布工作流目标为 Linux x86_64、CPython 3.10–3.13、glibc 2.28+。其他平台需要使用下文的工具链从源码构建。NumPy/PyTorch 仍是运行时依赖；需要时先安装适合应用的 CPU/CUDA PyTorch。

离线安装或验证 CI 产物时，下载兼容的 wheel：

```sh
python -m pip install ./straw_queue-*.whl
```

运行命令的目录中只应有兼容的 wheel。例如 `cp312-cp312` 代表 CPython 3.12，不能用在 CPython 3.10，以文件名中的实际平台 tag 为准。推送版本 tag 时，工作流还会将已验证产物发布到 PyPI；其他触发方式只构建与测试。

## 维护者：本地构建

需要 Rust 1.89+、C linker/toolchain 和 CPython 3.10+。只有构建机器需要 Rust。构建依赖声明在 `pyproject.toml`，Cargo 依赖由 `Cargo.lock` 固定。

```sh
python -m pip install build maturin twine
python -m build --wheel --sdist
python -m twine check dist/*
```

`dist/` 包含 `.whl` 和源码 `.tar.gz`。本地构建获得其实际满足的平台 tag；在较新 Linux 发行版构建，不会自动得到兼容更旧 glibc 的 wheel。不要手动改 tag。

在 Linux 构建机上生成可移植的 Linux x86_64 wheel，可使用 maturin 与 Zig：

```sh
python -m pip install 'maturin[zig,patchelf]>=1.12,<2'
maturin build --release --locked --strip --zig \
  --compatibility manylinux_2_28 --out dist
```

也可使用 [wheel 工作流](../.github/workflows/wheels.yml)中的 manylinux 容器构建，遵循 [maturin 分发指南](https://www.maturin.rs/distribution.html)。绑定使用 Python buffer API，所以 wheel 区分解释器版本；项目不宣称用一个 ABI3 wheel 支持所有解释器。

## 检查可分发产物

先在用于检查的解释器中安装兼容 NumPy/PyTorch，再运行：

```sh
python tools/check_wheel.py dist/<the-compatible-wheel>.whl \
  --report /tmp/straw-wheel-check.json
```

把占位文件名替换为实际文件。检查脚本创建临时 venv，复用已经安装的运行时依赖，在不访问包索引的情况下**只安装 wheel**，并从测试 PATH 中移除 Rust/maturin。它拒绝导入源码树中的 straw，覆盖记录、张量/COW、任务/GC、多进程崩溃恢复、README 示例、本地 benchmark 和 trace profile，并检查记录确实打包、载荷 root 已删除。

安装 pytest 后，加 `--tests` 可进一步针对已安装 wheel 运行全部 Python 核心测试、独立模型与 native 故障注入。

还应验证仅用源码包就能重新构建：

```sh
python -m pip wheel --no-deps dist/straw_queue-*.tar.gz --wheel-dir rebuilt
```

这一步在装有 Rust 的构建机器上执行；二进制使用者不需要源码包。生成的 wheel、本地实验日志、数据集和训练输出不应进入源码版本管理或源码发行包。

自动生成 Rust SBOM 会带入本地 crate 的绝对路径，因此已关闭。`check_wheel.py` 会拒绝包含本地 crate 路径引用的生成式 JSON 元数据。Rust 依赖版本仍由 `Cargo.lock` 记录。

## CI 与发布步骤

[测试工作流](../.github/workflows/tests.yml)运行格式检查、启用/禁用 Python 绑定的 Clippy 和 Rust 测试。CPython 3.10–3.13 矩阵构建并安装 wheel，通过 `tools/check_wheel.py --tests` 运行全部 Python 回归、独立所有权模型、native 故障注入、公开示例、有界 I/O/GC benchmark 和 trace 检查。结果上传为 `test-report-py*` 产物。支持 push、pull request 和手动触发。

[Wheel 工作流](../.github/workflows/wheels.yml)构建 CPython 3.10–3.13 的 manylinux 2.28 x86_64 wheel，并逐个运行同样的安装后检查。它还会生成源码包，从源码包重新构建 wheel，再验证重新构建的 wheel。支持 pull request、手动触发和 `v*` tag。Wheel 位于 `wheel-cp*-linux-x86_64` 产物中，源码包位于 `source-distribution`，验证报告也会上传。

推送 `v*` tag 时，独立的 `publish` job 等待全部 wheel 测试和源码重建检查通过，确认包版本与 tag 一致，再将同一批产物上传到 PyPI。Pull request 和手动触发不会发布。

手动触发时，打开仓库的 **Actions**，选择 **tests** 或 **wheels**，点击 **Run workflow** 并选择分支。工作流进入默认分支后，GitHub 才会提供手动触发入口，详见 [GitHub 工作流触发文档](https://docs.github.com/en/actions/how-tos/write-workflows/choose-when-workflows-run/trigger-a-workflow)。构建和下载产物无需配置包仓库凭据。

### 配置 PyPI Trusted Publishing

在 GitHub 仓库的 **Settings → Environments** 创建名为 `pypi` 的 environment。首次发布时，在 [PyPI publishing 页面](https://pypi.org/manage/account/publishing/)添加 pending publisher，填写：

| 字段 | 内容 |
|---|---|
| PyPI project name | `straw-queue` |
| GitHub owner | `zhuzilin` |
| Repository name | `straw` |
| Workflow filename | `wheels.yml` |
| Environment name | `pypi` |

Workflow filename 只填 `wheels.yml`，不带 `.github/workflows/`。发布前，将该文件提交并推送到配置的仓库。发布 job 通过 GitHub OIDC 和 `id-token: write` 认证，无需配置 PyPI API token 或 GitHub secret。参见 [PyPI 配置指南](https://docs.pypi.org/trusted-publishers/creating-a-project-through-oidc/)和[发布指南](https://docs.pypi.org/trusted-publishers/using-a-publisher/)。

发布 commit 准备好、包版本均为 `0.1.0` 后，推送版本 tag 触发首次发布：

```sh
git tag v0.1.0
git push origin v0.1.0
```

首次上传成功时才创建 PyPI 项目；仅登记 pending publisher 不会保留包名。后续版本使用新的匹配版本号和 tag。构建或测试失败时，应先修复；失败的 job 会阻止发布 job 运行。

托管 CI 使用本地 Linux 文件系统和 CPU PyTorch。多客户端 JuiceFS 验证、存储服务故障及 GPU 训练属于独立的[部署与应用检查](VERIFICATION_zh.md)。托管结果需要单独检查；本地构建通过不代表矩阵已经执行。

分发版本前，确认 `Cargo.toml` 与 `pyproject.toml` 版本一致，运行[验证检查](VERIFICATION_zh.md)，检查 wheel/sdist 内容，运行 `check_wheel.py`，保存 wheel SHA-256 与精确源码版本。更新日志应说明协议/API 兼容性及已知限制。上传已经测试过的那个产物；之后重新构建会产生新产物，需要重新检查。
