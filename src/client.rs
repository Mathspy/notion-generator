use crate::response::{Block, Error, List, NotionId, Page};
use anyhow::{bail, Context, Result};
use async_recursion::async_recursion;
use futures_util::stream::{FuturesOrdered, TryStreamExt};
use reqwest::{Client, Method, RequestBuilder};
use serde::{Deserialize, Serialize};
use std::ops::Not;

pub struct NotionClient {
    client: Client,
    auth_token: String,
}

impl NotionClient {
    pub fn new(auth_token: String) -> Self {
        NotionClient {
            client: Client::new(),
            auth_token,
        }
    }

    pub fn with_client(client: Client, auth_token: String) -> Self {
        NotionClient { client, auth_token }
    }

    fn build_request(&self, method: Method, url: &str) -> RequestBuilder {
        self.client
            .request(method, url)
            .header("Notion-Version", "2021-08-16")
            .bearer_auth(&self.auth_token)
    }

    async fn send_request<R>(&self, url: &str, request: RequestBuilder) -> Result<R>
    where
        R: for<'de> Deserialize<'de>,
    {
        let response = request
            .send()
            .await
            .with_context(|| format!("Failed to get data for request {}", url))?;

        if response.status().is_success().not() {
            let error = response
                .json::<Error>()
                .await
                .with_context(|| format!("Failed to parse error for request {}", url))?;

            bail!(
                "{}: {}",
                serde_json::to_value(error.code)
                    .expect("unreachable")
                    .as_str()
                    .context(
                        "Error code should was not a JSON string? This should be unreachable"
                    )?,
                error.message
            );
        }

        let parsed = response
            .json::<R>()
            .await
            .with_context(|| format!("Failed to parse JSON for request {}", url))?;

        Ok(parsed)
    }

    #[async_recursion]
    pub async fn get_block_children(&self, id: NotionId) -> Result<Vec<Block>> {
        let mut cursor = None;
        let mut output = FuturesOrdered::new();

        loop {
            let url = format!("https://api.notion.com/v1/blocks/{}/children", id);
            let list = self
                .send_request::<List<Block>>(
                    &url,
                    self.build_request(Method::GET, &url).query(&[
                        ("page_size", Some("100")),
                        ("start_cursor", cursor.as_deref()),
                    ]),
                )
                .await?;

            let requests = list
                .results
                .into_iter()
                .map(|block| async {
                    if !block.has_children {
                        return Ok::<Block, anyhow::Error>(block);
                    }

                    let children = self.get_block_children(block.id).await?;

                    Ok(block.replace_children(children))
                })
                .collect::<Vec<_>>();

            output.extend(requests);

            if list.has_more {
                cursor = list.next_cursor;
            } else {
                return output.try_collect().await;
            }
        }
    }

    pub async fn get_database_pages<P>(&self, id: &str) -> Result<Vec<Page<P>>>
    where
        P: for<'de> Deserialize<'de>,
    {
        #[derive(Serialize)]
        struct QueryDatabaseRequestBody<'a> {
            #[serde(skip_serializing_if = "Option::is_none")]
            start_cursor: Option<&'a str>,
            page_size: u8,
        }

        let mut cursor = None;
        let mut output = FuturesOrdered::new();

        loop {
            let url = format!("https://api.notion.com/v1/databases/{}/query", id);
            let list = self
                .send_request::<List<Page<P>>>(
                    &url,
                    self.build_request(Method::POST, &url)
                        .json(&QueryDatabaseRequestBody {
                            start_cursor: cursor.as_deref(),
                            page_size: 100,
                        }),
                )
                .await?;

            let requests = list
                .results
                .into_iter()
                .map(|page| async {
                    let children = self.get_block_children(page.id).await?;

                    Ok(page.replace_children(children))
                })
                .collect::<Vec<_>>();

            output.extend(requests);

            if list.has_more {
                cursor = list.next_cursor;
            } else {
                return output.try_collect().await;
            }
        }
    }

    pub fn client(&self) -> &Client {
        &self.client
    }
}
