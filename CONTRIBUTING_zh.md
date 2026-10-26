# 贡献指南

[English](CONTRIBUTING.md)

修改持久化顺序、恢复、所有权或认证前，阅读[设计与 spec](docs/DESIGN_zh.md)。使用 [straw-protocol-review skill](.agents/skills/straw-protocol-review/SKILL.md) 先核对规则与 TLA+，再沿实际调用链核对实现。

协议改动说明成功、报错但已生效、崩溃／重试及兼容性。针对真实故障边界添加复现，并保留合法成功路径；同步规则、模型、调用者与回归。普通局部函数使用编译器、Clippy 和有意义的测试，不重建逐函数证明。未知 wire 版本明确拒绝，framing 变化需要格式版本。

修改环境前提时，同步 [environment_contracts.json](verification/environment_contracts.json) 的稳定 ID、[spec](docs/DESIGN_zh.md#environment)、模型与敏感性负例，不把 straw 调用者义务改成文件系统假设。用户／贡献者文档有中英文两版，链接、API 与命令保持一致。应用 codec 与调度留在使用方。项目使用 [MIT 许可证](LICENSE)。

<a id="checks"></a>

## 开发与检查

需要 Rust 1.89+、C linker、CPython 3.10+；TLC 需要 Java 17+。安装适合环境的 CPU/CUDA PyTorch，再安装开发包：

```sh
python -m pip install -e '.[test,build,dev]'
python -m black --check src tests examples tools verification
python -m isort --check-only src tests examples tools verification
cargo fmt --check
cargo clippy --locked --all-targets -- -D warnings
cargo clippy --locked --all-targets --features python -- -D warnings
cargo test --locked
python -m pytest
python verification/runners/environment_manifest.py --check
python verification/runners/check_tla.py
python verification/protocol/checks/ownership_model.py --output /tmp/straw-ownership.json
python verification/wal/checks/crash_campaign.py --output /tmp/straw-crashes.json
```

Rust 修改后先重新构建／安装 native 扩展，再跑 Python 测试。局部模型工作可用 `check_tla.py --area wal --area gc` 或 `--case NAME`；发布验收运行全部保留场景。`--jar /path/to/tla2tools.jar` 指定 TLC，否则 runner 下载配置的发行包；记录实际 jar 哈希，不强制 checksum。日志、配置和反例默认在 `target/tla/`。正例必须通过，负例必须违反指定不变量／时序性质；解析错误、超时和启动失败不算检出。

环境登记检查核对声明的 ID、路径与模块导入。独立所有权模型探索有限抽象；crash campaign 检查逐字节前缀／损坏和进程退出，都不认证存储服务故障或掉电行为。多客户端验证见[部署指南](docs/FILESYSTEM_zh.md#qualification)。

<a id="build"></a>

## 构建与检查发行包

只有构建机需要 Rust。依赖由 `pyproject.toml` 声明、`Cargo.lock` 固定：

```sh
python -m pip install build maturin twine
python tools/release_build.py python -m build --wheel --sdist
python -m twine check dist/*
python tools/check_artifact_privacy.py dist/*
python tools/check_wheel.py dist/<compatible-wheel>.whl --tests --report /tmp/straw-wheel-check.json
python tools/release_build.py python -m pip wheel --no-deps dist/straw_queue-*.tar.gz --wheel-dir rebuilt
```

使用实际兼容 wheel 的文件名，并检查从 sdist 重建的 wheel。`check_wheel.py` 创建临时环境，复用已安装 NumPy/PyTorch，不访问索引，只安装 wheel，拒绝源码导入，并从测试 PATH 移除 Rust。它运行示例、多进程恢复、benchmark／GC 和 trace 检查；`--tests` 加上全部 Python 回归、所有权模型和 crash campaign。

发布包装器移除 Rust 产物中的本地源码／工具链路径。隐私检查扫描 archive 内可识别的路径／凭据，不打印匹配值。自动 Rust SBOM 因嵌入本地 crate 路径而关闭；依赖版本仍保存在 `Cargo.lock`。

发布工作流面向 Linux x86_64、CPython 3.10–3.13、manylinux/glibc 2.28+。绑定使用区分解释器版本的 buffer API，不是 ABI3。本地构建只获得实际满足的平台 tag，不得改名冒充兼容性。使用 [manylinux 工作流](.github/workflows/wheels.yml)，或在 Linux 上运行：

```sh
python -m pip install 'maturin[zig,patchelf]>=1.12,<2'
python tools/release_build.py maturin build --release --locked --strip --zig \
  --compatibility manylinux_2_28 --out dist
```

<a id="release"></a>

## CI 与发布

[测试工作流](.github/workflows/tests.yml)运行全部 TLA 场景的 16 个独立分片、格式、两种 Clippy 配置、Rust 测试，以及 CPython 3.10–3.13 的已安装 wheel 检查。[Wheel 工作流](.github/workflows/wheels.yml)先要求同一提交通过上述工作流，再验证每个可移植 wheel 和从 sdist 重建的 wheel，之后上传产物。失败、取消、跳过或超时阻止发布。托管 CI 使用本地 Linux 文件系统和 CPU PyTorch；本地验收、托管结果与分布式部署测试分别确认。

发布前同步 `Cargo.toml`、`pyproject.toml` 和 `Cargo.lock` 中 `straw` 的版本，并在 changelog 写明兼容性与限制。新的匹配 `v*` tag 触发发布已经测试过的产物；PR 和手动触发只构建／测试。源码 revision 与 wheel 哈希随发布产物保存，重新构建会改变产物身份。不要覆盖已发布版本／tag。

发布使用 GitHub OIDC 和 `pypi` environment。PyPI publisher 配置为项目 `straw-queue`、仓库 `zhuzilin/straw`、workflow `wheels.yml`、environment `pypi`；workflow 文件名不带目录。本工作流不需要 PyPI token，服务端配置见 [PyPI 指南](https://docs.pypi.org/trusted-publishers/creating-a-project-through-oidc/)。两条工作流都支持从 Actions 手动触发。

<a id="evidence"></a>

## 报告与公开基准

逐轮验证报告、尝试过程、wheel、凭据、机器地址、载荷和原始日志保存在 CI artifacts 或忽略的本地目录。使用全新、有界的测试根，仅在自有进程停止后清理。

`verification/measurements/` 只保留[性能文档](docs/BENCHMARKS_zh.md#published-results)引用的精选公开基准：说明负载／方法、构建／探针身份、单位、样本和限制。比较时保持载荷大小、发布粒度、fanout、持久化与缓存条件可比，不把旧构建结果当成当前代码性能。单次任务验收写入 PR 或 CI 结果，不再新增逐日 JSON 流水账。
