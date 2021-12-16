use crate::response::{Block, Error, List, NotionId};
use anyhow::{bail, Context, Result};
use async_recursion::async_recursion;
use futures_util::stream::{FuturesOrdered, TryStreamExt};
use reqwest::Client;
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

    #[async_recursion]
    pub async fn get_block_children(&self, id: NotionId) -> Result<Vec<Block>> {
        let mut cursor = None;
        let mut output = FuturesOrdered::new();

        loop {
            let response = self
                .client
                .get(format!("https://api.notion.com/v1/blocks/{}/children", id))
                .header("Notion-Version", "2021-08-16")
                .bearer_auth(&self.auth_token)
                .query(&[
                    ("page_size", Some("100")),
                    ("start_cursor", cursor.as_deref()),
                ])
                .send()
                .await
                .context("Failed to get data for block children")?;

            if response.status().is_success().not() {
                let error = response.json::<Error>().await.with_context(|| {
                    format!("Failed to parse ERROR JSON for block {} children", id)
                })?;

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

            let list = response
                .json::<List<Block>>()
                .await
                .with_context(|| format!("Failed to parse JSON for block {} children", id))?;

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

    pub fn client(&self) -> &Client {
        &self.client
    }
}
