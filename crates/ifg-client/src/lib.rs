//! Typed REST client for the IronForge API (`/api/v1/`).
//!
//! Layout (design doc §4): `http` holds the transport (auth header, retries,
//! error-body parsing), `pagination` the page iteration, and the resource
//! groups (`users`, `repos`, `issues`, `pulls`) the typed endpoints with
//! `*_raw` JSON twins for `--json` (D5). DTOs are hand-written and only
//! declare fields the client uses; unknown fields are ignored (D4).

mod agents;
mod ai;
mod ci;
mod collab;
mod commits;
mod contract;
mod drafts;
mod error;
mod events;
mod http;
mod idempotency;
mod issues;
mod labels;
mod milestones;
mod notifications;
mod orgs;
mod pagination;
mod pulls;
mod releases;
mod repos;
mod search;
mod secret;
mod text;
mod users;
mod wiki;

pub use agents::{AgentTokenRequest, Agents};
pub use ai::{
    Ai, AiListQuery, CodeSearchPage, CodeSearchQuery, MapQuery, PrContextQuery, SymbolsQuery,
    TreeQuery,
};
pub use ci::{Ci, Job, Pipeline, PipelineDetail, Stage, StageWithJobs};
/// Path-segment encoder for resource routes (`repos/{owner}/{name}`, `orgs/{name}`).
/// Untrusted segments must never be able to change the route, so every
/// user-supplied segment goes through the same encoder as collaboration IDs.
pub(crate) use collab::segment as path_segment;
pub use collab::{segment as collab_segment, Collaboration, SessionCredentials, StartedSession};
pub use commits::{Branches, Commits, PickKind, PickRequest};
pub use contract::USED_PATHS;
pub use drafts::Drafts;
pub use error::Error;
pub use events::Events;
pub use http::{Client, ClientBuilder, DEFAULT_BACKOFF, DEFAULT_MAX_RETRIES, DEFAULT_TIMEOUT};
pub use idempotency::{IdempotencyKey, InvalidIdempotencyKey};
pub use issues::{Comment, CreateIssueOptions, Issue, IssueListFilter, Issues, UpdateIssueOptions};
pub use labels::{Label, LabelOptions, Labels};
pub use milestones::{Milestone, MilestoneOptions, Milestones};
pub use notifications::{Notification, Notifications};
pub use orgs::{CreateOrgOptions, OrgMember, Organization, Orgs};
pub use pagination::{Page, PaginationMeta, Paginator, PER_PAGE};
pub use pulls::{
    CreatePrOptions, MergeOutcome, MergeStrategy, PullRequest, Pulls, ReviewAction, ReviewComment,
    UpdatePrOptions,
};
pub use releases::{Asset, CreateReleaseOptions, Release, Releases, UpdateReleaseOptions};
pub use repos::{Collaborator, CreateRepoOptions, Repo, Repos};
pub use reqwest::Method;
pub use search::{Search, SearchHit, SearchPage};
pub use secret::Secret;
pub use text::sanitize;
pub use users::{has_scope, TokenInfo, UserProfile, Users};
pub use wiki::Wiki;
