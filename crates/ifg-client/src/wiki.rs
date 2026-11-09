//! Wiki operations on a single repository. Historical revision lookup is
//! deliberately omitted until the server verifies revision ownership.
use crate::repos::repo_path;
use crate::{Client, Error, Method};
use serde_json::{json, Value};

pub struct Wiki<'a> {
    client: &'a Client,
    path: String,
}

impl Client {
    pub fn wiki(&self, owner: &str, repo: &str) -> Wiki<'_> {
        Wiki {
            client: self,
            path: format!("{}/wiki", repo_path(owner, repo)),
        }
    }
}

impl Wiki<'_> {
    fn page(&self, title: &str) -> String {
        format!("{}/{}", self.path, crate::collab_segment(title))
    }
    pub async fn list(&self) -> Result<Value, Error> {
        self.client.send(Method::GET, &self.path, None).await
    }
    pub async fn view(&self, title: &str) -> Result<Value, Error> {
        self.client.send(Method::GET, &self.page(title), None).await
    }
    pub async fn history(&self, title: &str) -> Result<Value, Error> {
        self.client
            .send(Method::GET, &format!("{}/history", self.page(title)), None)
            .await
    }
    pub async fn create(
        &self,
        title: &str,
        content: &str,
        message: Option<&str>,
    ) -> Result<Value, Error> {
        self.client
            .send(
                Method::POST,
                &self.path,
                Some(&json!({"title":title,"content":content,"message":message})),
            )
            .await
    }
    pub async fn update(
        &self,
        title: &str,
        content: &str,
        expected_version: &str,
        message: Option<&str>,
    ) -> Result<Value, Error> {
        let repo_path = self.path.strip_suffix("/wiki").expect("Wiki path suffix");
        let capabilities: Value = self
            .client
            .send(
                Method::GET,
                &format!("{repo_path}/collaboration/capabilities"),
                None,
            )
            .await?;
        let supported = capabilities["api_capabilities"]
            .as_array()
            .is_some_and(|items| {
                items
                    .iter()
                    .any(|item| item.as_str() == Some("wiki.concurrent_update.v1"))
            });
        if !supported {
            return Err(Error::Config(
                "server does not advertise protected Wiki updates; refusing an unprotected edit"
                    .into(),
            ));
        }
        self.client
            .send(
                Method::PATCH,
                &self.page(title),
                Some(&json!({"content":content,"message":message,"expected_version":expected_version})),
            )
            .await
    }
    pub async fn delete(&self, title: &str) -> Result<(), Error> {
        self.client
            .raw(Method::DELETE, &self.page(title), None)
            .await?;
        Ok(())
    }
}
