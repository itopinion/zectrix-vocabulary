# ZECTRIX Vocabulary

[简体中文](README.md) | **English**

A local vocabulary app for ZECTRIX Hub. Manage words by textbook and unit in a browser, send Chinese prompts to the device, practise letter spelling and whole-word reading separately, then listen to a spoken answer.

Hub owns the hardware connection, app switching and shared speech environment. The current app package supports **Apple Silicon Macs**. Windows, Linux and Synology NAS speech ports are not yet implemented or verified.

## Device requirements and installation

**Requires ZECTRIX Note 4 with official device firmware 3.7.0 or later (≥3.7.0), in local push mode.** Follow the Note 4 section of the [official firmware upgrade guide](https://wiki.zectrix.com/zh/software/firmware) (Chinese). Use Chrome or Edge on a computer, sign in to the cloud platform, open **Local Upgrade / 本地升级**, connect with a USB data cable and select the correct firmware version. Note 4 and Note 4C firmware are not interchangeable. This app does not automatically support Note 4C and contains no firmware or flashing tools.

1. [Download and start ZECTRIX Hub](https://github.com/itopinion/zectrix-hub/releases), then connect the device through its local management page.
2. Download `zectrix-vocabulary-VERSION-macos-aarch64.zip` from [Releases](https://github.com/itopinion/zectrix-vocabulary/releases).
3. Select that ZIP in Hub, install it and activate the vocabulary app. Users do not need to configure Rust or Python themselves.
4. Install the local speech components through Hub, then reopen the app.

Allow macOS local network access for `zectrix-hub` when prompted. If the web interface opens but the device cannot connect, check **System Settings → Privacy & Security → Local Network**.

## Practice

Create textbooks and units, add words, meanings and phonetic symbols, or import your own word list using the format shown in the interface. The initial entries are explicitly labelled examples, not a verified textbook list. Check edition and unit assignments when importing.

Select the practice scope and start. The device shows a Chinese prompt. Hold the voice key to spell the letters and release to finish recording. After the spelling result, briefly press confirm to enter reading, then hold the key and say only the word. The app displays the answer and phonetic symbols and plays a Kokoro example. The web interface provides volume, retry, skip and continue controls.

Correct and incorrect spelling have different sound effects. Unclear letters are not automatically completed using the expected answer. Whole-word recognition uses a prompt without the target answer; known homophones can be accepted for reading while spelling remains strict. An unconfirmed reading is not treated as a definite pronunciation error or added as a spelling mistake.

Recognition uses Qwen3-ASR-1.7B-8bit. Example speech uses Kokoro-82M with `af_heart`. **Text recognition is not professional acoustic pronunciation scoring.** Fast spelling, repeated letters and noise may require a retry.

## Data and privacy

Vocabulary, progress, generated audio caches and logs live under the persistent `app-data/zectrix-vocabulary/` directory assigned by Hub. Updates do not replace this data. Recognition and synthesis run locally; initial runtime/model installation requires network access.

The repository does not include personal learning records, real diagnostic recordings, device addresses, model weights or a full textbook word list. Diagnostic recording is not continuously retained by default. Back up old experimental data before migration and import through the web interface rather than overwriting the data directory.

## Windows and NAS ports

The Rust app logic and Hub protocol can be reused, but the Mac package is not portable as a binary. Build for the target platform and adapt the manifest, packaging, browser access, speech setup and inference backend. Current Qwen recognition uses Apple Silicon MLX; changing the platform label alone will not make it work elsewhere.

See Hub's [English porting guide](https://github.com/itopinion/zectrix-hub/blob/main/docs/PORTING.en.md) / [中文适配指南](https://github.com/itopinion/zectrix-hub/blob/main/docs/PORTING.md). It identifies changes in this repository's `src/hub.rs`, `src/main.rs`, `src/speech.rs`, `workers/speech.py` and packaging scripts.

## Development and packaging

This repository builds independently: a neighbouring Hub or old dashboard checkout is not required. Install Rust stable; developer packaging also uses Python 3's standard library. Hub handles device communication and rendering.

```sh
cargo test --locked
cargo build --release --locked
./scripts/package-app.sh
```

The packaging script currently requires Apple Silicon macOS. It copies an allowlist of runtime files, hashes them and creates a complete `app.json`. The source manifest contains an empty hash map and cannot be installed directly. Outputs and SHA-256 files go to `dist/`.

The [GitHub Actions template](docs/github-actions/README.md) is inactive because current publishing authorization lacks workflow permission. When enabled, it builds candidate artifacts without automatically publishing a Release. Do not commit local `data/`, `dist/`, `target/`, models or personal paths.

- [Release notes and verification scope](docs/RELEASE.md) (Chinese)
- [Third-party notices](THIRD_PARTY_NOTICES.md)

Project code is MIT-licensed. Third-party components, fonts and models retain their own licenses.
