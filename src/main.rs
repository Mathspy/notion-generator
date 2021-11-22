mod download;
mod highlight;
mod render;
mod response;

use anyhow::{bail, Context, Result};
use async_recursion::async_recursion;
use futures_util::stream::{self, FuturesOrdered, StreamExt};
use reqwest::Client;
use response::{Block, Error, List};
use std::ops::Not;

#[async_recursion]
async fn get_block_children(
    client: &Client,
    id: &str,
    cursor: Option<String>,
    auth_token: &str,
) -> Result<Vec<Result<Block>>> {
    let mut cursor = cursor;
    let mut output = vec![];

    loop {
        let response = client
            .get(format!("https://api.notion.com/v1/blocks/{}/children", id))
            .header("Notion-Version", "2021-08-16")
            .bearer_auth(auth_token)
            .query(&[
                ("page_size", Some("100")),
                ("start_cursor", cursor.as_deref()),
            ])
            .send()
            .await
            .context("Failed to get data for block children")?;

        if response.status().is_success().not() {
            let error = response
                .json::<Error>()
                .await
                .with_context(|| format!("Failed to parse ERROR JSON for block {} children", id))?;

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

                let children = get_block_children(client, &block.id, None, auth_token).await?;

                Ok(block.replace_children(
                    children
                        .into_iter()
                        .collect::<Result<Vec<_>>>()
                        .context("Failed to get sub-block's children")?,
                ))
            })
            .collect::<FuturesOrdered<_>>();

        output.push(requests);

        if list.has_more {
            cursor = list.next_cursor;
        } else {
            return Ok(stream::iter(output).flatten().collect().await);
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let auth_token = std::env::var("NOTION_TOKEN").context("Missing NOTION_TOKEN env variable")?;

    let client = Client::builder()
        .build()
        .context("Failed to build HTTP client")?;

    // TODO: We still need to get the title from requesting the page's own block
    let blocks = get_block_children(
        &client,
        args.get(1).context("Missing page id as first argument")?,
        None,
        &auth_token,
    )
    .await
    .context("Failed to get block children")?;

    Ok(())
}
