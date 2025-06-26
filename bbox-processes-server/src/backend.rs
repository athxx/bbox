use crate::endpoints::JobResult;
use crate::error::Result;
use crate::models;
use async_trait::async_trait;
use dyn_clone::{clone_trait_object, DynClone};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug)]
pub struct Job {
    pub name: String,
    pub description: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Execute {
    pub inputs: Option<serde_json::Value>,
    pub outputs: Option<serde_json::Value>,
    pub response: Option<String>,
}

#[async_trait]
pub trait ProcessingBackend:
    ProcessingProcessMeta + ProcessingExecute + ProcessingResults + DynClone + Sync + Send
{
}
clone_trait_object!(ProcessingBackend);

#[async_trait]
pub trait ProcessingProcessMeta: DynClone + Sync + Send {
    async fn process_list(&self) -> Result<Vec<Job>>;
    async fn get_process_description(&self, process_id: &str) -> Result<serde_json::Value>;
}
clone_trait_object!(ProcessingProcessMeta);

#[async_trait]
pub trait ProcessingExecute: DynClone + Sync + Send {
    async fn execute(&self, process_id: &str, params: &Execute) -> Result<models::StatusInfo>;
    async fn execute_sync(&self, process_id: &str, params: &Execute) -> Result<JobResult>;
    async fn get_jobs(&self) -> Result<serde_json::Value>;
}
clone_trait_object!(ProcessingExecute);

#[async_trait]
pub trait ProcessingResults: DynClone + Sync + Send {
    async fn get_status(&self, job_id: &str) -> Result<models::StatusInfo>;
    async fn get_result(&self, job_id: &str) -> Result<JobResult>;
}
clone_trait_object!(ProcessingResults);
