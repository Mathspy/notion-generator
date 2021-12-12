use anyhow::{Context, Result};
use clap::Parser;
use notion_generator::{
    client::NotionClient, options::HeadingAnchors, response::NotionId, HtmlRenderer,
};
use std::{
    collections::{HashMap, HashSet},
    fmt,
    path::PathBuf,
    str::FromStr,
};

struct LinkMap(HashMap<NotionId, String>);

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

                let page_id = page_id.parse().map_err(|_| LinkMapError::InvalidPageId)?;

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

    let client = NotionClient::new(auth_token);

    let document_id = opts
        .document_id
        .parse()
        .with_context(|| format!("{} is not a valid Notion document ID", opts.document_id))?;

    // TODO: We still need to get the title from requesting the page's own block
    let blocks = client
        .get_block_children(document_id)
        .await
        .context("Failed to get block children")?;

    let head = String::from_utf8(
        tokio::fs::read(opts.head)
            .await
            .context("Failed to read head partial")?,
    )
    .context("Failed to parse head partial as utf8")?;
    let mut current_pages = HashSet::new();
    current_pages.insert(document_id);
    for current_page in opts.current_pages {
        current_pages.insert(current_page.parse()?);
    }
    let renderer = HtmlRenderer {
        heading_anchors: opts.heading_anchors,
        current_pages,
        link_map: &opts.link_map.0,
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
        downloadables.download_all(client.client(), &opts.output)
    )?;

    Ok(())
}
