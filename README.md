# ZECTRIX 背单词

**简体中文** | [English](README.en.md)

运行于 ZECTRIX Hub 的本地背单词应用：在网页按教材和单元管理词库，把中文提示发送到设备，分别练习字母拼写和整词朗读，再播放标准答案示范。

本应用不直接连接硬件；设备连接、应用切换和语音环境由独立的 **ZECTRIX Hub** 管理。首版应用包支持 Apple Silicon Mac，Windows、Linux 和群晖上的本地语音尚未适配。

## 安装

1. 先[下载并启动 ZECTRIX Hub](https://github.com/itopinion/zectrix-hub/releases)，打开其本地管理界面，连接设备。
2. 从 [GitHub Releases](https://github.com/itopinion/zectrix-vocabulary/releases) 下载 `zectrix-vocabulary-版本-macos-aarch64.zip`。没有 Release 时，可从源码生成应用包。
3. 在 Hub 的应用安装入口选择此 ZIP，安装后打开“背单词”。无需用户自行配置 Rust 或 Python。
4. 首次使用在 Hub 安装本地语音组件，完成后重新打开背单词。

macOS 首次询问 `zectrix-hub` 的本地网络访问时，需要允许后才能连接设备。若网页可打开但设备连接失败，在“系统设置 → 隐私与安全性 → 本地网络”检查该权限，再回 Hub 重试。

**设备要求：ZECTRIX Note 4，官方设备固件 3.7.0 及以上（≥3.7.0），并进入本地推送模式。** 升级请参考 [官方固件升级说明](https://wiki.zectrix.com/zh/software/firmware) 的 Note 4 部分，在电脑用 Chrome 或 Edge 登录极趣云的“本地升级”，通过 USB 连接并选择对应固件版本。Note 4 与 Note 4C 固件不可混刷，本应用不自动兼容 Note 4C。固件不在本仓库或应用包内。

## 使用

网页中创建教材与单元，添加单词、中文和音标，或按导入入口给出的格式导入自有词表。首次运行仅提供明确标注的演示词条，不把演示词表当作已核对教材。请使用有权使用的词库，并核对课本版次和单元归属。

选择练习范围后启动抽测。设备先显示中文，长按语音键报字母、松开结束；查看拼写结果后，短按确认进入整词朗读，长按只念单词。系统随后显示答案、音标并播放 Kokoro 示范。网页可调节音量，也可以重试、跳过或继续。

拼写正确和错误各有提示音。未可靠分清字母时不会自动补齐答案。整词朗读采用不含目标答案的识别提示；已知同音词可按朗读规则接受，但拼写仍严格区分。朗读未确认不等于发音错误，也不会因此加入拼写错词。

本地识别使用 Qwen3-ASR-1.7B-8bit，示范声音使用 Kokoro-82M / `af_heart`。**识别文字不是专业发音精度评分。** 连续快读、重复字母和噪音仍可能影响结果，应用保留重试入口。

## 数据与隐私

词库、进度、合成缓存和日志位于 Hub 分配的 `app-data/zectrix-vocabulary/`，不会写回应用包。更新应用保留这些数据。识别和语音合成在本机执行；首次安装需要联网下载环境和模型。

仓库不包含个人学习记录、真实排障录音、设备地址、模型权重或完整教材词库。诊断录音默认不持续留存；请勿将任何个人数据或录音提交到公开仓库。迁移旧实验版本前先备份，再通过网页导入词表，不覆盖现有数据目录。

## Windows 与 NAS 开发适配

应用的 Rust 业务逻辑和 Hub 接口可以复用，但当前 Mac 应用包不能直接跨平台运行。需为目标平台编译应用、生成对应 manifest，适配网页访问、语音安装和识别后端。当前 Qwen 识别依赖 Apple Silicon 的 MLX；仅更改应用包的平台名称无法完成移植。

参阅 Hub 的 [中文适配指南](https://github.com/itopinion/zectrix-hub/blob/main/docs/PORTING.md) / [English porting guide](https://github.com/itopinion/zectrix-hub/blob/main/docs/PORTING.en.md)，其中标出了本仓库需修改的 `src/hub.rs`、`src/main.rs`、`src/speech.rs`、`workers/speech.py` 和打包脚本。

## 开发与打包

应用可以在单独的仓库目录构建，不需要旁边存在旧墨卡项目或 Hub 源码。需要 Rust stable，打包还使用 Python 3 标准库。设备和字体渲染在 Hub 进行。

```sh
cargo test --locked
cargo build --release --locked
./scripts/package-app.sh
```

包和校验文件生成于 `dist/`。打包脚本复制明确允许的运行文件，为所有文件计算哈希并生成完整 `app.json`。源码里的 `app.json` 是空哈希模板，不是可直接安装的包。

[GitHub Actions 配置模板](docs/github-actions/README.md)可运行构建与测试并上传待检查的应用包。首次公开上传的授权不含工作流权限，因此模板暂未启用；不自动公开发布 Release。不要上传开发机上的 `data/`、`dist/`、`target/`、模型或个人路径。

- [发布说明](docs/RELEASE.md)
- [第三方组件](THIRD_PARTY_NOTICES.md)

程序代码使用 MIT 许可；第三方组件、字体及模型各自遵循原许可。
