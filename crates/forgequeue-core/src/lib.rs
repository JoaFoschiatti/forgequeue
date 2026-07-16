use std::{fmt, str::FromStr, time::Duration};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use utoipa::ToSchema;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum JobKind {
    Image,
    Pdf,
}

impl JobKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Image => "image",
            Self::Pdf => "pdf",
        }
    }
}

impl fmt::Display for JobKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for JobKind {
    type Err = ParseDomainValueError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "image" => Ok(Self::Image),
            "pdf" => Ok(Self::Pdf),
            _ => Err(ParseDomainValueError::new("job kind", value)),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum JobStatus {
    Queued,
    Running,
    RetryScheduled,
    Succeeded,
    CancelRequested,
    Cancelled,
    DeadLettered,
    Expired,
}

impl JobStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Running => "running",
            Self::RetryScheduled => "retry_scheduled",
            Self::Succeeded => "succeeded",
            Self::CancelRequested => "cancel_requested",
            Self::Cancelled => "cancelled",
            Self::DeadLettered => "dead_lettered",
            Self::Expired => "expired",
        }
    }

    pub const fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Succeeded | Self::Cancelled | Self::DeadLettered | Self::Expired
        )
    }

    pub const fn can_cancel(self) -> bool {
        matches!(self, Self::Queued | Self::RetryScheduled | Self::Running)
    }
}

impl fmt::Display for JobStatus {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for JobStatus {
    type Err = ParseDomainValueError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "queued" => Ok(Self::Queued),
            "running" => Ok(Self::Running),
            "retry_scheduled" => Ok(Self::RetryScheduled),
            "succeeded" => Ok(Self::Succeeded),
            "cancel_requested" => Ok(Self::CancelRequested),
            "cancelled" => Ok(Self::Cancelled),
            "dead_lettered" => Ok(Self::DeadLettered),
            "expired" => Ok(Self::Expired),
            _ => Err(ParseDomainValueError::new("job status", value)),
        }
    }
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
#[error("invalid {field}: {value}")]
pub struct ParseDomainValueError {
    field: &'static str,
    value: String,
}

impl ParseDomainValueError {
    fn new(field: &'static str, value: impl Into<String>) -> Self {
        Self {
            field,
            value: value.into(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct Job {
    pub id: Uuid,
    pub kind: JobKind,
    pub status: JobStatus,
    pub progress: u8,
    pub stage: String,
    pub original_name: String,
    pub input_content_type: String,
    pub input_size: i64,
    pub attempt_count: i32,
    pub max_attempts: i32,
    pub last_error_code: Option<String>,
    pub last_error_detail: Option<String>,
    pub retry_of_job_id: Option<Uuid>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
    pub artifacts_expire_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct JobAttempt {
    pub id: Uuid,
    pub number: i32,
    pub worker_id: String,
    pub status: String,
    pub error_code: Option<String>,
    pub error_detail: Option<String>,
    pub started_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct JobOutput {
    pub id: Uuid,
    pub name: String,
    pub content_type: String,
    pub size: i64,
    pub width: Option<i32>,
    pub height: Option<i32>,
    pub page_number: Option<i32>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct JobDetail {
    #[serde(flatten)]
    pub job: Job,
    pub attempts: Vec<JobAttempt>,
    pub outputs: Vec<JobOutput>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct JobPage {
    pub items: Vec<Job>,
    pub next_cursor: Option<Uuid>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct SessionResponse {
    pub token: String,
    pub expires_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ProblemDetails {
    #[serde(rename = "type")]
    pub problem_type: String,
    pub title: String,
    pub status: u16,
    pub code: String,
    pub detail: String,
    pub request_id: String,
}

pub fn retry_delay(completed_attempts: i32) -> Option<Duration> {
    match completed_attempts {
        1 => Some(Duration::from_secs(5)),
        2 => Some(Duration::from_secs(30)),
        _ => None,
    }
}

pub fn valid_transition(from: JobStatus, to: JobStatus) -> bool {
    use JobStatus as S;

    matches!(
        (from, to),
        (S::Queued, S::Running)
            | (S::Queued, S::Cancelled)
            | (S::Running, S::Succeeded)
            | (S::Running, S::RetryScheduled)
            | (S::Running, S::CancelRequested)
            | (S::Running, S::DeadLettered)
            | (S::CancelRequested, S::Cancelled)
            | (S::CancelRequested, S::RetryScheduled)
            | (S::RetryScheduled, S::Running)
            | (S::RetryScheduled, S::Cancelled)
            | (S::DeadLettered, S::Expired)
            | (S::Succeeded, S::Expired)
            | (S::Cancelled, S::Expired)
    )
}

#[cfg(test)]
mod tests {
    use super::{JobStatus, retry_delay, valid_transition};

    #[test]
    fn retry_schedule_is_bounded() {
        assert_eq!(retry_delay(1).map(|delay| delay.as_secs()), Some(5));
        assert_eq!(retry_delay(2).map(|delay| delay.as_secs()), Some(30));
        assert_eq!(retry_delay(3), None);
    }

    #[test]
    fn terminal_jobs_cannot_restart_in_place() {
        assert!(!valid_transition(JobStatus::Succeeded, JobStatus::Running));
        assert!(!valid_transition(
            JobStatus::DeadLettered,
            JobStatus::Queued
        ));
    }

    #[test]
    fn running_job_can_request_cancellation() {
        assert!(valid_transition(
            JobStatus::Running,
            JobStatus::CancelRequested
        ));
    }
}
