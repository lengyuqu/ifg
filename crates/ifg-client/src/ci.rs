//! CI/CD pipelines resource group (design doc §4.5). Log streaming is
//! polling-based (§7.1 option A): the WebSocket endpoint only accepts
//! session JWTs, not PATs.

use serde::Deserialize;
use serde_json::Value;

use crate::error::Error;
use crate::http::Client;
use crate::pagination::Paginator;
use crate::repos::repo_path;

/// A pipeline row in `GET …/pipelines` and the detail wrapper.
#[derive(Debug, Clone, Deserialize)]
pub struct Pipeline {
    pub id: i64,
    pub repo_id: i64,
    #[serde(default)]
    pub commit_sha: Option<String>,
    pub ref_name: String,
    pub status: String,
    pub trigger_type: String,
    #[serde(default)]
    pub triggered_by: Option<String>,
    #[serde(default)]
    pub started_at: Option<String>,
    #[serde(default)]
    pub finished_at: Option<String>,
    #[serde(default)]
    pub created_at: Option<String>,
}

/// A stage inside a pipeline detail.
#[derive(Debug, Clone, Deserialize)]
pub struct Stage {
    pub id: i64,
    pub pipeline_id: i64,
    pub name: String,
    pub stage_order: i64,
    pub status: String,
    #[serde(default)]
    pub started_at: Option<String>,
    #[serde(default)]
    pub finished_at: Option<String>,
}

/// A job inside a stage. `log` carries the accumulated output (§7.1-A).
#[derive(Debug, Clone, Deserialize)]
pub struct Job {
    pub id: i64,
    pub stage_id: i64,
    pub name: String,
    pub status: String,
    #[serde(default)]
    pub exit_code: Option<i64>,
    #[serde(default)]
    pub environment_name: Option<String>,
    #[serde(default)]
    pub allow_failure: Option<bool>,
    #[serde(default)]
    pub log: Option<String>,
    #[serde(default)]
    pub started_at: Option<String>,
    #[serde(default)]
    pub finished_at: Option<String>,
}

impl Job {
    /// Terminal job states: polling stops on these (§7.1-A). `error` is what
    /// the runner records when a job could not run at all.
    pub fn is_terminal(&self) -> bool {
        matches!(
            self.status.as_str(),
            "success"
                | "succeeded"
                | "failed"
                | "failure"
                | "error"
                | "canceled"
                | "cancelled"
                | "skipped"
        )
    }

    /// States that never progress on their own: a `manual` job waits to be
    /// played, a `waiting_approval` job for an environment approval.
    /// Polling them would never end.
    pub fn is_blocked(&self) -> bool {
        matches!(self.status.as_str(), "manual" | "waiting_approval")
    }
}

/// `GET …/pipelines/{id}`: pipeline plus stages with their jobs.
#[derive(Debug, Clone, Deserialize)]
pub struct PipelineDetail {
    pub pipeline: Pipeline,
    pub stages: Vec<StageWithJobs>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct StageWithJobs {
    pub stage: Stage,
    pub jobs: Vec<Job>,
}

impl PipelineDetail {
    /// All jobs, stage order preserved.
    pub fn jobs(&self) -> Vec<&Job> {
        self.stages
            .iter()
            .flat_map(|stage| stage.jobs.iter())
            .collect()
    }

    /// Find a job by id or by name (first match).
    pub fn find_job(&self, selector: &str) -> Option<&Job> {
        self.jobs()
            .into_iter()
            .find(|job| job.id.to_string() == selector || job.name == selector)
    }
}

/// Accessor for CI endpoints of one repository.
pub struct Ci<'a> {
    client: &'a Client,
    owner: String,
    repo: String,
}

impl Client {
    /// CI pipeline endpoints of `owner/repo`.
    pub fn ci(&self, owner: &str, repo: &str) -> Ci<'_> {
        Ci {
            client: self,
            owner: owner.to_string(),
            repo: repo.to_string(),
        }
    }
}

impl Ci<'_> {
    fn pipelines_path(&self) -> String {
        format!("{}/pipelines", repo_path(&self.owner, &self.repo))
    }

    fn pipeline_path(&self, id: i64) -> String {
        format!("{}/{id}", self.pipelines_path())
    }

    /// List pipelines (paginated; no server-side filters — filter locally).
    pub fn list(&self) -> Paginator<Pipeline> {
        self.client.paginate(self.pipelines_path())
    }

    /// Raw JSON twin of [`Ci::list`].
    pub fn list_raw(&self) -> Paginator<Value> {
        self.client.paginate(self.pipelines_path())
    }

    /// `GET …/pipelines/{id}` — pipeline with stages and jobs.
    pub async fn get(&self, id: i64) -> Result<PipelineDetail, Error> {
        self.client
            .send(crate::Method::GET, &self.pipeline_path(id), None)
            .await
    }

    /// Raw JSON twin of [`Ci::get`].
    pub async fn get_raw(&self, id: i64) -> Result<Value, Error> {
        self.client
            .send(crate::Method::GET, &self.pipeline_path(id), None)
            .await
    }

    /// `GET …/pipelines/{id}/jobs/{job_id}` — job detail with the log.
    pub async fn job(&self, pipeline_id: i64, job_id: i64) -> Result<Job, Error> {
        self.client
            .send(
                crate::Method::GET,
                &format!("{}/jobs/{job_id}", self.pipeline_path(pipeline_id)),
                None,
            )
            .await
    }

    /// Raw JSON twin of [`Ci::job`].
    pub async fn job_raw(&self, pipeline_id: i64, job_id: i64) -> Result<Value, Error> {
        self.client
            .send(
                crate::Method::GET,
                &format!("{}/jobs/{job_id}", self.pipeline_path(pipeline_id)),
                None,
            )
            .await
    }

    /// `POST …/pipelines/{id}/retry`.
    pub async fn retry(&self, id: i64) -> Result<Value, Error> {
        self.action(id, "retry").await
    }

    /// `POST …/pipelines/{id}/cancel`.
    pub async fn cancel(&self, id: i64) -> Result<Value, Error> {
        self.action(id, "cancel").await
    }

    /// `POST …/jobs/{job_id}/play` — release a manual job.
    pub async fn play_job(&self, pipeline_id: i64, job_id: i64) -> Result<Value, Error> {
        self.job_action(pipeline_id, job_id, "play").await
    }

    /// `POST …/jobs/{job_id}/rerun`.
    pub async fn rerun_job(&self, pipeline_id: i64, job_id: i64) -> Result<Value, Error> {
        self.job_action(pipeline_id, job_id, "rerun").await
    }

    /// `POST …/jobs/{job_id}/approve` — approve an environment-gated job.
    pub async fn approve_job(&self, pipeline_id: i64, job_id: i64) -> Result<Value, Error> {
        self.job_action(pipeline_id, job_id, "approve").await
    }

    async fn action(&self, id: i64, action: &str) -> Result<Value, Error> {
        self.client
            .send(
                crate::Method::POST,
                &format!("{}/{action}", self.pipeline_path(id)),
                None,
            )
            .await
    }

    async fn job_action(
        &self,
        pipeline_id: i64,
        job_id: i64,
        action: &str,
    ) -> Result<Value, Error> {
        self.client
            .send(
                crate::Method::POST,
                &format!("{}/jobs/{job_id}/{action}", self.pipeline_path(pipeline_id)),
                None,
            )
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn job(status: &str) -> Job {
        serde_json::from_value(serde_json::json!({
            "id": 1, "stage_id": 1, "name": "build", "status": status
        }))
        .unwrap()
    }

    #[test]
    fn error_is_terminal() {
        assert!(job("error").is_terminal());
        assert!(job("failed").is_terminal());
        assert!(!job("running").is_terminal());
        assert!(!job("pending").is_terminal());
    }

    #[test]
    fn manual_and_waiting_approval_are_blocked_not_terminal() {
        for status in ["manual", "waiting_approval"] {
            assert!(job(status).is_blocked(), "{status}");
            assert!(!job(status).is_terminal(), "{status}");
        }
        assert!(!job("running").is_blocked());
    }
}
