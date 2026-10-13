# 启用 GitHub Actions

此目录保存已准备的自动测试和 macOS Apple Silicon 打包配置。首次公开上传所用 GitHub 授权没有 `workflow` 权限，因此配置作为模板保留，当前不会自动运行。源码、安装包及本地测试不受影响。

维护者取得工作流写入权限后，将 `build.yml` 移到仓库根目录下的 `.github/workflows/build.yml` 并提交到 `main`，即可启用。也可以由有权限的维护者在 GitHub 网页中创建该文件。

工作流只运行测试、生成安装包并上传 Actions artifact，不自动公开发布 Release，也不访问真实设备或下载语音模型。远端 CI 是否通过，以启用后的实际结果为准。
