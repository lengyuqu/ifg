//! Releases resource group (design doc §4.6). There is no server-side
//! by-tag route (§7.3): [`Releases::find_by_tag`] paginates and matches
//! `tag_name`. Uploads stream the file body; downloads expose the raw
//! `reqwest::Response` so the caller can hash and write incrementally.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::Error;
use crate::http::Client;
use crate::pagination::Paginator;
use crate::repos::{repo_path, to_body};

/// A release row (server serializes the entity model directly).
#[derive(Debug, Clone, Deserialize)]
pub struct Release {
    pub id: i64,
    pub repo_id: i64,
    pub tag_name: String,
    pub target_commitish: String,
    pub title: String,
    #[serde(default)]
    pub body: Option<String>,
    pub is_draft: bool,
    pub is_prerelease: bool,
    pub author_id: i64,
    #[serde(default)]
    pub created_at: Option<String>,
    #[serde(default)]
    pub updated_at: Option<String>,
}

/// `POST …/releases` request body.
#[derive(Debug, Default, Serialize)]
pub struct CreateReleaseOptions {
    pub tag_name: String,
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_commitish: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_draft: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_prerelease: Option<bool>,
}

/// A release asset (plain JSON array endpoint, not paginated).
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Asset {
    pub id: i64,
    pub release_id: i64,
    pub filename: String,
    pub size: i64,
    pub content_type: String,
    pub download_count: i64,
    /// Hex SHA-256 recorded at upload time; `None` for legacy assets
    /// (unverifiable, not a failure).
    #[serde(default)]
    pub sha256: Option<String>,
    #[serde(default)]
    pub created_at: Option<String>,
}

/// Accessor for release endpoints of one repository.
pub struct Releases<'a> {
    client: &'a Client,
    owner: String,
    repo: String,
}

impl Client {
    /// Release endpoints of `owner/repo`.
    pub fn releases(&self, owner: &str, repo: &str) -> Releases<'_> {
        Releases {
            client: self,
            owner: owner.to_string(),
            repo: repo.to_string(),
        }
    }
}

impl Releases<'_> {
    fn releases_path(&self) -> String {
        format!("{}/releases", repo_path(&self.owner, &self.repo))
    }

    fn release_path(&self, id: i64) -> String {
        format!("{}/{id}", self.releases_path())
    }

    fn assets_path(&self, release_id: i64) -> String {
        format!("{}/assets", self.release_path(release_id))
    }

    fn asset_path(&self, asset_id: i64) -> String {
        format!(
            "{}/releases/assets/{asset_id}",
            repo_path(&self.owner, &self.repo)
        )
    }

    /// List releases (paginated, newest first server-side).
    pub fn list(&self) -> Paginator<Release> {
        self.client.paginate(self.releases_path())
    }

    /// Raw JSON twin of [`Releases::list`].
    pub fn list_raw(&self) -> Paginator<Value> {
        self.client.paginate(self.releases_path())
    }

    /// `GET …/releases/{id}`.
    pub async fn get(&self, id: i64) -> Result<Release, Error> {
        self.client
            .send(crate::Method::GET, &self.release_path(id), None)
            .await
    }

    /// Raw JSON twin of [`Releases::get`].
    pub async fn get_raw(&self, id: i64) -> Result<Value, Error> {
        self.client
            .send(crate::Method::GET, &self.release_path(id), None)
            .await
    }

    /// Walk all pages and match `tag_name` (no by-tag route on the server,
    /// design doc §7.3).
    pub async fn find_by_tag(&self, tag: &str) -> Result<Option<Release>, Error> {
        let releases = self.list().collect().await?;
        Ok(releases.into_iter().find(|release| release.tag_name == tag))
    }

    /// `POST …/releases`.
    pub async fn create(&self, options: &CreateReleaseOptions) -> Result<Release, Error> {
        self.client
            .send(
                crate::Method::POST,
                &self.releases_path(),
                Some(&to_body(options)?),
            )
            .await
    }

    /// Raw JSON twin of [`Releases::create`].
    pub async fn create_raw(&self, options: &CreateReleaseOptions) -> Result<Value, Error> {
        self.client
            .send(
                crate::Method::POST,
                &self.releases_path(),
                Some(&to_body(options)?),
            )
            .await
    }

    /// `GET …/releases/{release_id}/assets` — plain array.
    pub async fn assets(&self, release_id: i64) -> Result<Vec<Asset>, Error> {
        self.client
            .send(crate::Method::GET, &self.assets_path(release_id), None)
            .await
    }

    /// Raw JSON twin of [`Releases::assets`].
    pub async fn assets_raw(&self, release_id: i64) -> Result<Value, Error> {
        self.client
            .send(crate::Method::GET, &self.assets_path(release_id), None)
            .await
    }

    /// `POST …/releases/{release_id}/assets` with a streaming body. The
    /// filename rides the `x-asset-filename` header (server also parses
    /// `Content-Disposition`, but the plain header avoids quoting rules).
    /// The file must exist; reading is streamed, never buffered whole.
    pub async fn upload_file(
        &self,
        release_id: i64,
        path: &std::path::Path,
        content_type: &str,
    ) -> Result<Asset, Error> {
        let filename = path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| Error::Config(format!("no usable filename in {}", path.display())))?;
        let value = self.upload_file_raw(release_id, path, content_type).await?;
        serde_json::from_value(value).map_err(|source| Error::Decode {
            body_snippet: format!("asset response for {filename}"),
            source,
        })
    }

    /// Raw JSON twin of [`Releases::upload_file`].
    pub async fn upload_file_raw(
        &self,
        release_id: i64,
        path: &std::path::Path,
        content_type: &str,
    ) -> Result<Value, Error> {
        use tokio_util::io::ReaderStream;

        let file = tokio::fs::File::open(path)
            .await
            .map_err(|e| Error::Config(format!("could not open {}: {e}", path.display())))?;
        let body = reqwest::Body::wrap_stream(ReaderStream::new(file));
        let filename = path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| Error::Config(format!("no usable filename in {}", path.display())))?;
        let headers = [(
            reqwest::header::HeaderName::from_static("x-asset-filename"),
            reqwest::header::HeaderValue::from_str(filename)
                .map_err(|e| Error::Config(format!("invalid filename {filename}: {e}")))?,
        )];
        let response = self
            .client
            .raw_with_stream_body(
                crate::Method::POST,
                &self.assets_path(release_id),
                body,
                content_type,
                &headers,
            )
            .await?;
        let bytes = response.bytes().await.map_err(Error::Transport)?;
        serde_json::from_slice(&bytes).map_err(|source| Error::Decode {
            body_snippet: crate::error::snippet(&bytes),
            source,
        })
    }

    /// `GET …/releases/assets/{asset_id}/download` — returns the un-read
    /// response: the caller streams, hashes against `x-checksum-sha256`
    /// (also exposed via [`Asset::sha256`]) and writes a `.part` file before
    /// renaming (design doc §4.5).
    pub async fn download(&self, asset_id: i64) -> Result<reqwest::Response, Error> {
        let path = format!("{}/download", self.asset_path(asset_id));
        self.client
            .raw_response(crate::Method::GET, &path, None)
            .await
    }
}
