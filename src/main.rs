mod render;
mod response;

use anyhow::{Context, Result};
use async_recursion::async_recursion;
use futures_util::stream::{self, FuturesOrdered, StreamExt};
use reqwest::{
    header::{self, HeaderMap, HeaderValue},
    Client,
};
use response::{Block, List};

#[async_recursion]
async fn get_block_children(
    client: &Client,
    id: &str,
    cursor: Option<String>,
) -> Result<Vec<Result<Block>>> {
    let mut cursor = cursor;
    let mut output = vec![];

    loop {
        let list = client
            .get(format!("https://api.notion.com/v1/blocks/{}/children", id))
            .query(&[
                ("page_size", Some("100")),
                ("start_cursor", cursor.as_deref()),
            ])
            .send()
            .await
            .context("Failed to get data for block children")?
            .json::<List<Block>>()
            .await
            .context("Failed to parse JSON for block children")?;

        let requests = list
            .results
            .into_iter()
            .map(|block| async {
                if !block.has_children {
                    return Ok::<Block, anyhow::Error>(block);
                }

                let children = get_block_children(client, &block.id, None).await?;

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

    let mut headers = HeaderMap::new();
    headers.insert("Notion-Version", HeaderValue::from_static("2021-08-16"));
    headers.insert(
        header::AUTHORIZATION,
        HeaderValue::try_from(&format!("Bearer {}", auth_token))
            .context("Failed to set bearer token on client")?,
    );
    let client = Client::builder()
        .default_headers(headers)
        .build()
        .context("Failed to build HTTP client")?;

    // TODO: We still need to get the title from requesting the page's own block
    let blocks = get_block_children(
        &client,
        args.get(1).context("Missing page id as first argument")?,
        None,
    )
    .await
    .context("Failed to get block children")?;

    Ok(())
}
