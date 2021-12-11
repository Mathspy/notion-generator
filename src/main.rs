mod download;
mod highlight;
mod render;
mod response;

use anyhow::{bail, Context, Result};
use async_recursion::async_recursion;
use clap::Parser;
use futures_util::stream::{self, FuturesOrdered, StreamExt};
use render::HtmlRenderer;
use reqwest::Client;
use response::{Block, Error, List};
use std::{
    collections::{HashMap, HashSet},
    fmt,
    ops::Not,
    path::PathBuf,
    str::FromStr,
};

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

#[derive(Clone, Copy)]
pub enum HeadingAnchors {
    None,
    Icon,
}

#[derive(Debug)]
pub struct HeadingAnchorsParseError;
impl fmt::Display for HeadingAnchorsParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Expected either `none` or `icon`")?;

        Ok(())
    }
}
impl std::error::Error for HeadingAnchorsParseError {}

impl FromStr for HeadingAnchors {
    type Err = HeadingAnchorsParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "none" => Ok(HeadingAnchors::None),
            "icon" => Ok(HeadingAnchors::Icon),
            _ => Err(HeadingAnchorsParseError),
        }
    }
}

struct LinkMap(HashMap<String, String>);

#[derive(Debug)]
enum LinkMapError {
    MissingPageId,
    InvalidPageId,
    MissingPath,
}

impl fmt::Display for LinkMapError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LinkMapError::MissingPageId => f.write_str("Missing page id"),
            LinkMapError::InvalidPageId => f.write_str("Invalid page id, must be a UUIDv4"),
            LinkMapError::MissingPath => f.write_str("Missing path"),
        }
    }
}

impl std::error::Error for LinkMapError {}

impl FromStr for LinkMap {
    type Err = LinkMapError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        s.split(',')
            .map(|entry| {
                let mut entry = entry.trim().split(':');
                let page_id = match entry.next() {
                    Some(page_id) => page_id,
                    None => return Err(LinkMapError::MissingPageId),
                };

                let page_id = page_id.replace("-", "");
                if page_id.len() != 32 {
                    return Err(LinkMapError::InvalidPageId);
                }

                let path = match entry.next() {
                    Some(path) => path,
                    None => return Err(LinkMapError::MissingPath),
                };

                Ok((page_id, path.to_string()))
            })
            .collect::<Result<HashMap<_, _>, _>>()
            .map(LinkMap)
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
    /// Whether to include a link icon next to headings or not
    #[clap(long, default_value = "none")]
    heading_anchors: HeadingAnchors,
    /// In case of rendering multiple notion pages into the same HTML page, this should
    /// contain the id of those other pages. Comma delimited list
    #[clap(long, require_delimiter = true, use_delimiter = true)]
    current_pages: Vec<String>,
    /// A map from page ids to URL paths, used to replace page ids in links with the URL path
    /// Example usage:
    /// 46ce88507ab748c78f92024dc1190ca7:/path/to/page,9b4d1ba2963e4dd885fc9c3c4284fc74:/path/to/other/page
    #[clap(long)]
    link_map: LinkMap,
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
    let mut current_pages = HashSet::new();
    current_pages.extend(
        std::iter::once(opts.document_id.replace("-", "")).chain(
            opts.current_pages
                .into_iter()
                .map(|page_id| page_id.replace("-", "")),
        ),
    );
    let renderer = HtmlRenderer {
        heading_anchors: opts.heading_anchors,
        current_pages,
        link_map: opts.link_map.0,
    };
    let (markup, downloadables) = renderer
        .render_page(blocks, head)
        .context("Failed to render page")?;

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
