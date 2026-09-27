# 贡献指南

[English](CONTRIBUTING.md)

使用 Rust 1.89+、CPython 3.10+。先安装适合当前环境的 PyTorch，再安装开发包：

```sh
python -m pip install -e '.[test,build,dev]'
python -m black --check src tests examples tools verification
python -m isort --check-only src tests examples tools verification
cargo fmt --check
cargo clippy --all-targets --features python -- -D warnings
cargo test --locked
python -m pytest
python verification/ownership_model.py
python verification/crash_campaign.py --output /tmp/straw-crashes.json
```

二进制 wheel 见[构建文档](docs/BUILDING_zh.md)，每类检查的边界见[验证文档](docs/VERIFICATION_zh.md)。机器地址、凭据、训练脚本、载荷和原始实验日志应保存在项目外。每次文件系统测试使用独立私有临时目录，限制数据规模，并在自有进程停止后清理。

协议修改必须说明持久化顺序、重试行为、兼容性、reader 所有权和失败处理。对真实故障边界添加复现用例，而非仅断言实现自身的行为。格式和独立模型的抽象发生变化时，同步更新它们。未知版本必须明确报错。Framing 常量属于存储格式，修改时需要显式变更格式版本。

应用自定义 codec 和调度逻辑应放在 Rust 核心之外。使用清晰的引用和显式生命周期信号，避免引入应用专用依赖。性能修改需要可比的载荷大小、发布粒度、reader fanout、文件系统/缓存条件，以及实际测量的字节数。

项目使用 [MIT 许可证](LICENSE)。

用户与贡献者文档使用配套的英文 `.md` 和中文 `_zh.md`。行为或示例发生变化时同步更新两种语言，保留双向语言切换链接，并确保 API 名称、命令和协议常量一致。
