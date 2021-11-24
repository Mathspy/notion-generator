mod download;
mod highlight;
mod render;
mod response;

use anyhow::{bail, Context, Result};
use async_recursion::async_recursion;
use clap::Parser;
use futures_util::stream::{self, FuturesOrdered, StreamExt};
use reqwest::Client;
use response::{Block, Error, List};
use std::{ops::Not, path::PathBuf};

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

/// Generate an HTML page from a Notion document
#[derive(Parser)]
struct Opts {
    /// The id of the Notion document to generate an HTML page from
    document_id: String,
    /// A partial HTML file to append to the bottom of the head
    #[clap(short, long, default_value = "partials/head.html")]
    head: PathBuf,
    /// The directory to output generated files into, defaults to current directory
    #[clap(short, long, default_value = ".")]
    output: PathBuf,
    // TODO: Add actual verbose logs lol
    /// A level of verbosity, and can be used multiple times
    #[clap(short, long, parse(from_occurrences))]
    verbose: u8,
}

#[tokio::main]
async fn main() -> Result<()> {
    let opts: Opts = Opts::parse();
    let auth_token = std::env::var("NOTION_TOKEN").context("Missing NOTION_TOKEN env variable")?;

    let client = Client::builder()
        .build()
        .context("Failed to build HTTP client")?;

    // TODO: We still need to get the title from requesting the page's own block
    let blocks = get_block_children(&client, &opts.document_id, None, &auth_token)
        .await
        .context("Failed to get block children")?
        .into_iter()
        .collect::<Result<Vec<_>>>()?;

    let head = String::from_utf8(
        tokio::fs::read(opts.head)
            .await
            .context("Failed to read head partial")?,
    )
    .context("Failed to parse head partial as utf8")?;
    let (markup, downloadables) =
        render::render_page(blocks, head).context("Failed to render page")?;

    let write_markup = async {
        tokio::fs::write(opts.output.join("index.html"), markup.0)
            .await
            .context("Failed to write index.html file")?;

        Ok::<_, anyhow::Error>(())
    };

    tokio::try_join!(
        write_markup,
        downloadables.download_all(&client, &opts.output)
    )?;

    Ok(())
}
