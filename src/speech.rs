use anyhow::{bail, Context, Result};
use serde_json::Value;
use std::{
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader, Lines},
    process::{Child, ChildStdin, Command},
    sync::Mutex,
};
struct Worker {
    child: Child,
    input: ChildStdin,
    output: Lines<BufReader<tokio::process::ChildStdout>>,
}
pub struct Speech {
    worker: Mutex<Option<Worker>>,
    python: PathBuf,
    script: PathBuf,
    mode: String,
    log: PathBuf,
}
impl Speech {
    pub fn new(app_root: &Path, data_root: &Path, mode: &str) -> Self {
        let python = std::env::var(if mode == "asr" {
            "ZECTRIX_ASR_PYTHON"
        } else {
            "ZECTRIX_TTS_PYTHON"
        })
        .map(PathBuf::from)
        .unwrap_or_default();
        Self {
            worker: Mutex::new(None),
            python,
            script: app_root.join("workers/speech.py"),
            mode: mode.into(),
            log: data_root.join(format!("data/{}.log", mode)),
        }
    }
    async fn spawn(&self) -> Result<Worker> {
        if self.python.as_os_str().is_empty() {
            bail!("语音组件未安装，请在本地助手安装语音能力包；词库管理仍可使用")
        }
        let log = std::fs::OpenOptions::new()
            .append(true)
            .create(true)
            .open(&self.log)?;
        let mut child = Command::new(&self.python)
            .arg("-u")
            .arg(&self.script)
            .arg(&self.mode)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(log)
            .kill_on_drop(true)
            .spawn()
            .context("本地语音环境不可用，请检查模型路径")?;
        let input = child.stdin.take().unwrap();
        let mut output = BufReader::new(child.stdout.take().unwrap()).lines();
        let line = tokio::time::timeout(Duration::from_secs(120), output.next_line())
            .await??
            .context("模型启动失败，查看本地日志")?;
        let ready: Value = serde_json::from_str(&line)?;
        if ready["ready"] != true {
            bail!("模型没有就绪")
        }
        Ok(Worker {
            child,
            input,
            output,
        })
    }
    pub async fn call(&self, request: Value) -> Result<Value> {
        let mut guard = self.worker.lock().await;
        if guard.is_none() {
            *guard = Some(self.spawn().await?)
        }
        let w = guard.as_mut().unwrap();
        let result = tokio::time::timeout(Duration::from_secs(90), async {
            w.input
                .write_all(format!("{}\n", request).as_bytes())
                .await?;
            w.input.flush().await?;
            let line = w.output.next_line().await?.context("语音进程已退出")?;
            let value: Value = serde_json::from_str(&line)?;
            if value["ok"] != true {
                bail!("{}", value["error"].as_str().unwrap_or("语音处理失败"))
            }
            Ok(value)
        })
        .await;
        match result {
            Ok(Ok(v)) => Ok(v),
            other => {
                if let Some(mut w) = guard.take() {
                    let _ = w.child.kill().await;
                }
                match other {
                    Ok(Err(e)) => Err(e),
                    _ => bail!("本地模型超时，可重新录音"),
                }
            }
        }
    }
    pub async fn warm(&self) -> Result<()> {
        let mut w = self.worker.lock().await;
        if w.is_none() {
            *w = Some(self.spawn().await?)
        }
        Ok(())
    }
}
