//! Backend for <https://pka.github.io/shell-compose/>

use crate::backend::{
    Execute, Job, ProcessingBackend, ProcessingExecute, ProcessingProcessMeta, ProcessingResults,
};
use crate::config::ShellBackendCfg;
use crate::endpoints::JobResult;
use crate::error::{self, Result};
use crate::models::{StatusCode, StatusInfo};
use async_trait::async_trait;
use log::{error, info};
use serde_json::json;
use shell_compose::*;
use std::path::PathBuf;
use std::process::{self, Child, Stdio};
use std::time::Duration;
use std::{env, thread};

#[derive(Clone)]
pub struct ShellBackend {
    config: ShellBackendCfg,
}

impl ShellBackend {
    pub fn new(config: ShellBackendCfg) -> Self {
        Self { config }
    }
    async fn get_job_infos(&self) -> Result<Vec<StatusInfo>> {
        let cmd = CliCommand::Ps;
        let mut stream = send_command(cmd.into())?;
        let response = stream.receive_message();
        match response {
            Ok(Message::PsInfo(proc_infos)) => {
                let job_infos = proc_infos
                    .into_iter()
                    .map(|proc| {
                        let status = match proc.state {
                            ProcStatus::ExitOk => StatusCode::SUCCESSFUL,
                            ProcStatus::ExitErr(_) => StatusCode::FAILED,
                            ProcStatus::Running => StatusCode::RUNNING,
                            _ => StatusCode::ACCEPTED,
                        };
                        StatusInfo::new("process".to_string(), proc.job_id.to_string(), status)
                    })
                    .collect::<Vec<_>>();
                return Ok(job_infos);
            }
            Ok(Message::Err(msg)) => {
                error!("Error message from backend: {msg}");
            }
            Err(msg) => {
                error!("Backend error: {msg}");
            }
            Ok(_) => {
                error!("Unexpected response from backend");
            }
        }
        Err(error::Error::BackendExecutionError(
            "Job information not available".to_owned(),
        ))
    }
}

#[async_trait]
impl ProcessingBackend for ShellBackend {}

#[async_trait]
impl ProcessingProcessMeta for ShellBackend {
    async fn process_list(&self) -> Result<Vec<Job>> {
        env::set_current_dir(&self.config.base_path).ok();
        if let Ok(dir) = env::current_dir() {
            info!("Shell backend running from directory: {}", dir.display());
        }
        let justfile =
            Justfile::parse().map_err(|e| error::Error::BackendExecutionError(e.to_string()))?;
        let recipes = justfile.group_recipes("processes");
        let jobs = recipes
            .into_iter()
            .map(|name| Job {
                name,
                description: None,
            })
            .collect();
        Ok(jobs)
    }
    async fn get_process_description(&self, process_id: &str) -> Result<serde_json::Value> {
        let jobs = self.process_list().await?;
        let descr = jobs
            .into_iter()
            .find(|job| job.name == process_id)
            .map(|job| {
                json!({
                    "processes": [
                        {
                            "id": job.name,
                            "version": "0.0.1"
                        }
                    ],
                    "links": [],
                })
            });
        descr.ok_or(error::Error::NotFound(process_id.to_string()))
    }
}

#[async_trait]
impl ProcessingExecute for ShellBackend {
    async fn execute(&self, process_id: &str, params: &Execute) -> Result<StatusInfo> {
        let noargsjson = json!([]);
        let noargs = vec![];
        let args = params
            .inputs
            .as_ref()
            .unwrap_or(&noargsjson)
            .as_array()
            .unwrap_or(&noargs)
            .iter()
            .map(|param| param.as_str().unwrap_or("").to_string());
        let cmd = ExecCommand::Run {
            args: vec!["just".to_string(), process_id.to_string()]
                .into_iter()
                .chain(args.clone())
                .collect(),
            restart: Some(RestartPolicy::Never),
        };
        let mut stream = send_command(Message::ExecCommand(
            cmd,
            PathBuf::from(&self.config.base_path),
        ))?;
        let response = stream.receive_message();
        match response {
            Ok(Message::JobsStarted(job_ids)) => {
                if let Some(job_id) = match job_ids.len() {
                    1 => {
                        info!("Job {job_ids:?} started");
                        let id = job_ids.first().unwrap_or(&0).to_string();
                        Some(id)
                    }
                    n => {
                        error!("{n} jobs started {job_ids:?}");
                        None
                    }
                } {
                    return Ok(StatusInfo::new(
                        "process".to_string(),
                        job_id,
                        StatusCode::ACCEPTED,
                    ));
                }
            }
            Ok(Message::Err(msg)) => {
                error!("{msg}");
            }
            Err(msg) => {
                error!("{msg}");
            }
            Ok(_) => {
                error!("Ignoring unexpected response from backend");
            }
        }
        return Ok(StatusInfo::new(
            "process".to_string(),
            "".to_string(),
            StatusCode::FAILED,
        ));
    }

    async fn execute_sync(&self, process_id: &str, params: &Execute) -> Result<JobResult> {
        let mut info = self.execute(process_id, params).await?;
        while info.status == StatusCode::ACCEPTED || info.status == StatusCode::RUNNING {
            tokio::time::sleep(Duration::from_millis(100)).await;
            info = self.get_status(&info.job_id).await?;
        }
        if info.status == StatusCode::SUCCESSFUL {
            self.get_result(&info.job_id).await
        } else {
            Err(error::Error::BackendExecutionFailed(info.status))
        }
    }

    async fn get_jobs(&self) -> Result<serde_json::Value> {
        let job_infos = self.get_job_infos().await?;
        Ok(serde_json::to_value(job_infos)?)
    }
}

#[async_trait]
impl ProcessingResults for ShellBackend {
    async fn get_status(&self, job_id: &str) -> Result<StatusInfo> {
        let job_infos = self.get_job_infos().await?;
        if let Some(status) = job_infos.into_iter().find(|proc| proc.job_id == job_id) {
            return Ok(status);
        }
        Err(error::Error::BackendExecutionError(
            "Job information not available".to_owned(),
        ))
    }

    async fn get_result(&self, job_id: &str) -> Result<JobResult> {
        let cmd = CliCommand::Logs {
            job_or_service: Some(job_id.to_string()),
        };
        let mut stream = send_command(cmd.into())?;
        let mut last_output = String::new();
        loop {
            let response = stream.receive_message();
            match response {
                Ok(Message::LogLine(log_line)) => {
                    if log_line.line != "<process terminated>" {
                        last_output = log_line.line;
                    }
                }
                Ok(Message::Ok) | Ok(Message::Connect) => {
                    break;
                }
                Ok(Message::Err(msg)) => {
                    error!("{msg}");
                    return Ok(JobResult::Json(json!({"error": msg})));
                }
                Err(msg) => {
                    error!("{msg}");
                    return Ok(JobResult::Json(json!({"error": format!("{msg}")})));
                }
                Ok(_) => {
                    error!("Unexpected response from backend");
                    return Ok(JobResult::Json(
                        json!({"error": "Unexpected response from backend"}),
                    ));
                }
            }
        }
        Ok(JobResult::Json(json!(last_output)))
    }
}

fn check_backend() -> Result<()> {
    if IpcStream::check_connection().is_err() {
        info!("Starting background process");
        let dispatcher = DispatcherProc::spawn()?;
        if let Err(e) = dispatcher.wait(2000) {
            dispatcher.kill()?;
            return Err(e);
        }
    }
    Ok(())
}

fn send_command(message: Message) -> Result<IpcStream> {
    check_backend()?;
    let mut stream = IpcStream::connect("processes-client")?;
    stream.send_message(&message)?;
    Ok(stream)
}

struct DispatcherProc(Child);

impl DispatcherProc {
    /// Spawn background process
    fn spawn() -> Result<Self> {
        let exe = "shell-composed";
        let mut proc = process::Command::new(exe);
        #[cfg(target_os = "windows")]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x08000000;
            // CREATE_NO_WINDOW causes all children to not show a visible console window,
            // but it also apparently has the effect of starting a new process group.
            //
            // https://learn.microsoft.com/en-us/windows/win32/procthread/process-creation-flags#flags
            // https://stackoverflow.com/a/71364777/9423933
            proc.creation_flags(CREATE_NO_WINDOW);

            // See https://stackoverflow.com/a/78989930 for a possible alternative.
        }
        // Propagate debug log level to background process
        if env::var("RUST_LOG").unwrap_or("".to_string()) == "debug" {
            proc.env("RUST_LOG", "debug")
        } else {
            proc.stdout(Stdio::null()).stderr(Stdio::null())
        };
        let child = proc
            .spawn()
            .map_err(DispatcherError::DispatcherSpawnError)?;
        Ok(DispatcherProc(child))
    }
    /// wait until communication with background process ready
    fn wait(&self, max_ms: u64) -> Result<()> {
        let mut wait_ms = 0;
        while IpcStream::check_connection().is_err() {
            if wait_ms >= max_ms {
                return Err(DispatcherError::DispatcherSpawnTimeoutError.into());
            }
            thread::sleep(Duration::from_millis(50));
            wait_ms += 50;
        }
        Ok(())
    }
    /// kill background process
    fn kill(mut self) -> Result<()> {
        self.0.kill().map_err(DispatcherError::KillError)?;
        self.0.wait().map_err(DispatcherError::KillError)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn backend() -> ShellBackend {
        let config = ShellBackendCfg {
            base_path: ".".to_string(),
        };
        ShellBackend { config }
    }

    #[actix_web::test]
    async fn metadata_test() {
        let backend = backend();
        let jobs = backend.process_list().await.unwrap();
        assert_eq!(jobs.len(), 3);
        assert!(jobs.iter().any(|job| job.name == "hello"));

        let descr = backend
            .get_process_description(&jobs[0].name)
            .await
            .unwrap();
        dbg!(&descr);
    }

    #[actix_web::test]
    async fn exec_sync() {
        let backend = backend();
        let execute = Execute {
            inputs: None,
            outputs: None,
            response: None,
        };
        let result = backend.execute_sync("hello", &execute).await.unwrap();
        if let JobResult::Json(v) = result {
            assert_eq!(v, json!("hello world"));
        } else {
            panic!("Unexpected result");
        }
    }

    #[actix_web::test]
    async fn exec_async() {
        bbox_core::logger::init(None);
        let backend = backend();
        let execute = Execute {
            inputs: Some(json!(["2"])),
            outputs: None,
            response: None,
        };
        let mut info = backend.execute("sleep", &execute).await.unwrap();
        assert_eq!(info.status, StatusCode::ACCEPTED);
        while info.status == StatusCode::ACCEPTED || info.status == StatusCode::RUNNING {
            tokio::time::sleep(Duration::from_millis(100)).await;
            info = backend.get_status(&info.job_id).await.unwrap();
        }
        assert!(info.status != StatusCode::FAILED);
        let result = backend.get_result(&info.job_id).await.unwrap();
        if let JobResult::Json(v) = result {
            assert_eq!(v, json!("Sleep 2"));
        } else {
            panic!("Unexpected result");
        }
    }
}
