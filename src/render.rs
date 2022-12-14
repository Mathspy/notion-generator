use crate::download::Downloadables;
use crate::highlight::highlight;
use crate::options::HeadingAnchors;
use crate::response::{
    Block, BlockType, EmojiOrFile, ListType, NotionId, Page, PlainText, RichText, RichTextLink,
    RichTextMentionType, RichTextType, Time,
};
use anyhow::Result;
use itertools::Itertools;
use maud::{html, Escaper, Markup, PreEscaped, Render, DOCTYPE};
use std::collections::HashMap;
use std::{
    collections::HashSet,
    fmt::{self, Write},
};

pub struct HtmlRenderer<'html> {
    pub heading_anchors: HeadingAnchors<'html>,
    /// A list of pages that will be rendered together, used to figure out whether to use fragment
    /// part of links alone (#block_id) or to use the full canonical link (/page_id#block_id)
    ///
    /// If you're rendering each page independently this should be a set with only the page id
    ///
    /// If you're rendering multiple pages together into the same HTML page this should be a set
    /// of all those pages ids
    pub current_pages: HashSet<NotionId>,
    /// A map from page ids to URL paths to replace page ids in links with the corresponding URL
    /// path
    pub link_map: &'html HashMap<NotionId, String>,
    /// A list of media to download for rendering
    pub downloadables: &'html Downloadables,
}

enum BlockCoalition<'a> {
    List(ListType, Vec<&'a Block>),
    Solo(&'a Block),
}

impl<'a> BlockCoalition<'a> {
    fn list_type(&self) -> Option<ListType> {
        match self {
            BlockCoalition::List(ty, _) => Some(*ty),
            BlockCoalition::Solo(block) => block.list_type(),
        }
    }
}

impl<'a> std::ops::Add for BlockCoalition<'a> {
    type Output = Result<BlockCoalition<'a>, (BlockCoalition<'a>, BlockCoalition<'a>)>;

    fn add(self, rhs: Self) -> Self::Output {
        match (self.list_type(), rhs.list_type()) {
            (Some(self_type), Some(rhs_type)) if self_type == rhs_type => match (self, rhs) {
                (BlockCoalition::Solo(first), BlockCoalition::Solo(second)) => {
                    Ok(BlockCoalition::List(self_type, vec![first, second]))
                }
                (BlockCoalition::List(_, mut list), BlockCoalition::Solo(second)) => {
                    list.push(second);
                    Ok(BlockCoalition::List(self_type, list))
                }
                (BlockCoalition::Solo(first), BlockCoalition::List(_, mut list)) => {
                    list.push(first);
                    Ok(BlockCoalition::List(self_type, list))
                }
                (BlockCoalition::List(_, mut first_list), BlockCoalition::List(_, second_list)) => {
                    first_list.extend(second_list);
                    Ok(BlockCoalition::List(self_type, first_list))
                }
            },
            _ => Err((self, rhs)),
        }
    }
}

/// Trait to retrieve the title from arbitrary Properties structs
pub trait Title {
    /// Return the title property's `title` field
    ///
    /// If you're curious why title is returned from API as Vec<RichText> it's because titles can
    /// contain mentions and other types of rich text besides text. And although Notion doesn't
    /// currently display any annotations (bold, italic, etc) on titles in their desktop app, the
    /// information actually gets saved and will be included in the [RichText] objects
    fn title(&self) -> &[RichText];
}

#[derive(Debug, Clone, Copy)]
pub enum Heading {
    H1,
    H2,
    H3,
    H4,
    H5,
    H6,
}

impl From<Heading> for u8 {
    fn from(value: Heading) -> Self {
        match value {
            Heading::H1 => 1,
            Heading::H2 => 2,
            Heading::H3 => 3,
            Heading::H4 => 4,
            Heading::H5 => 5,
            Heading::H6 => 6,
        }
    }
}

#[derive(Debug)]
pub struct InvalidHtmlHeading(u16);

impl fmt::Display for InvalidHtmlHeading {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("h")?;
        self.0.fmt(f)?;
        f.write_str(" is not a valid HTML heading")?;

        Ok(())
    }
}

impl std::error::Error for InvalidHtmlHeading {}

impl TryFrom<u16> for Heading {
    type Error = InvalidHtmlHeading;

    fn try_from(value: u16) -> Result<Self, Self::Error> {
        match value {
            1 => Ok(Heading::H1),
            2 => Ok(Heading::H2),
            3 => Ok(Heading::H3),
            4 => Ok(Heading::H4),
            5 => Ok(Heading::H5),
            6 => Ok(Heading::H6),
            value => Err(InvalidHtmlHeading(value)),
        }
    }
}

impl Heading {
    fn downgrade(&self, amount: u8) -> Result<Self, InvalidHtmlHeading> {
        let current = u8::from(*self);
        let downgraded = u16::from(current) + u16::from(amount);
        Heading::try_from(downgraded)
    }
}

impl<'html> HtmlRenderer<'html> {
    pub fn render_html(&self, blocks: Vec<Block>, head: String) -> Result<Markup> {
        let rendered_blocks = self.render_blocks(&blocks, None, 0);

        let markup = html! {
            (DOCTYPE)
            html lang="en" {
                head {
                    meta charset="utf-8";
                    meta name="viewport" content="width=device-width, initial-scale=1";
                    link rel="stylesheet" href="styles/katex.css";

                    (PreEscaped(head))
                }
                body {
                    main {
                        @for block in rendered_blocks {
                            (block?)
                        }
                    }
                }
            }
        };

        Ok(markup)
    }

    pub fn render_page<P: Title>(&self, page: &Page<P>) -> Result<Markup> {
        let rendered_blocks = self.render_blocks(&page.children, None, 1);

        Ok(html! {
            (self.render_heading(page.id, None, Heading::H1, page.properties.title()))
            @for block in rendered_blocks {
                (block?)
            }
        })
    }

    /// Render a group of blocks into HTML
    pub fn render_blocks<'a, I>(
        &'a self,
        blocks: I,
        class: Option<&'a str>,
        downgrade_headings: u8,
    ) -> impl Iterator<Item = Result<Markup>> + 'a
    where
        I: IntoIterator<Item = &'a Block> + 'a,
    {
        blocks
            .into_iter()
            .map(BlockCoalition::Solo)
            .coalesce(|a, b| a + b)
            .map(move |coalition| match coalition {
                BlockCoalition::List(ty, list) => {
                    self.render_list(ty, list, class, downgrade_headings)
                }
                BlockCoalition::Solo(block) => self.render_block(block, class, downgrade_headings),
            })
    }

    fn render_list(
        &self,
        ty: ListType,
        list: Vec<&Block>,
        class: Option<&str>,
        downgrade_headings: u8,
    ) -> Result<Markup> {
        let list = list.into_iter().map(|item| {
            if let (Some(text), Some(children)) = (item.get_text(), item.get_children()) {
                Ok::<_, anyhow::Error>(html! {
                    li id=(item.id) {
                        (self.render_rich_text(text))
                        @for block in self.render_blocks(children, class, downgrade_headings) {
                            (block?)
                        }
                    }
                })
            } else {
                unreachable!()
            }
        });

        match ty {
            ListType::Bulleted => Ok(html! {
                ul class=[class] {
                    @for item in list {
                        (item?)
                    }
                }
            }),
            ListType::Numbered => Ok(html! {
                ol class=[class] {
                    @for item in list {
                        (item?)
                    }
                }
            }),
            _ => todo!(),
        }
    }

    fn render_block(
        &self,
        block: &Block,
        class: Option<&str>,
        downgrade_headings: u8,
    ) -> Result<Markup> {
        match &block.ty {
            BlockType::HeadingOne { text } => Ok(self.render_heading(
                block.id,
                class,
                Heading::H1.downgrade(downgrade_headings)?,
                text,
            )),
            BlockType::HeadingTwo { text } => Ok(self.render_heading(
                block.id,
                class,
                Heading::H2.downgrade(downgrade_headings)?,
                text,
            )),
            BlockType::HeadingThree { text } => Ok(self.render_heading(
                block.id,
                class,
                Heading::H3.downgrade(downgrade_headings)?,
                text,
            )),
            BlockType::Divider {} => Ok(html! {
                hr id=(block.id);
            }),
            BlockType::Paragraph { text, children } => {
                if children.is_empty() {
                    Ok(html! {
                        p id=(block.id) class=[class] {
                            (self.render_rich_text(text))
                        }
                    })
                } else {
                    eprintln!("WARNING: Rendering a paragraph with children doesn't make sense as far as I am aware at least for the English language.\nThe HTML spec is strictly against it (rendering a <p> inside of a <p> is forbidden) but it's part of Notion's spec so we support it but emit this warning.\n\nRendering a paragraph with children doesn't give any indication to accessibility tools that anything about the children of this paragraph are special so it causes accessibility information loss.\n\nIf you have an actual use case for paragraphs inside of paragraphs please open an issue, I would love to be convinced of reasons to remove this warning or of good HTML ways to render paragraphs inside of paragraphs!");

                    Ok(html! {
                        div id=(block.id) class=[class] {
                            p {
                                (self.render_rich_text(text))
                            }
                            @for child in self.render_blocks(children, Some("indent"), downgrade_headings) {
                                (child?)
                            }
                        }
                    })
                }
            }
            BlockType::Quote { text, children } => Ok(html! {
                blockquote id=(block.id) {
                    (self.render_rich_text(text))
                    @for child in self.render_blocks(children, Some("indent"), downgrade_headings) {
                        (child?)
                    }
                }
            }),
            // TODO: We don't currently handle the possibility of rich text inside of code blocks
            // this is complex because we need to create an HTML highlight renderer besides the one
            // built into tree-sitter that knows how to render both rich text and highlights at the
            // same time. Can likely reuse a lot of the code from RichTextRenderer
            BlockType::Code { language, text } => highlight(language, &text.plain_text(), block.id),
            // The list items should only be reachable below if a block wasn't coalesced, thus it's
            // a list made of one item so we can safely render a list of one item
            BlockType::BulletedListItem { text, children } => Ok(html! {
                ul {
                    li id=(block.id) {
                        (self.render_rich_text(text))
                        @for child in self.render_blocks(children, Some("indent"), downgrade_headings) {
                            (child?)
                        }
                    }
                }
            }),
            BlockType::NumberedListItem { text, children } => Ok(html! {
                ol {
                    li id=(block.id) {
                        (self.render_rich_text(text))
                        @for child in self.render_blocks(children, Some("indent"), downgrade_headings) {
                            (child?)
                        }
                    }
                }
            }),
            BlockType::Image { image, caption } => {
                let downloadable = image.as_downloadable(block.id)?;

                let markup = if !caption.is_empty() {
                    // Lack of alt text can be explained here
                    // https://stackoverflow.com/a/58468470/3018913
                    html! {
                        figure id=(block.id) {
                            img src=(downloadable.src_path());
                            figcaption {
                                (self.render_rich_text(caption))
                            }
                        }
                    }
                } else {
                    eprintln!("WARNING: Rendering image without caption text is not accessibility friendly for users who use screen readers");

                    html! {
                        img id=(block.id) src=(downloadable.src_path());
                    }
                };

                self.downloadables.insert(downloadable);

                Ok(markup)
            }
            BlockType::Video { video, caption } => {
                let downloadable = video.as_downloadable(block.id)?;

                let markup = if !caption.is_empty() {
                    // Lack of alt text can be explained here
                    // https://stackoverflow.com/a/58468470/3018913
                    html! {
                        figure id=(block.id) {
                            video controls src=(downloadable.src_path()) {
                                p {
                                    "Unfortunately looks like your browser doesn't support videos."
                                    a href=(downloadable.src_path()) {
                                        "But no worries you can click me to download the video!"
                                    }
                                }
                            }
                            figcaption {
                                (self.render_rich_text(caption))
                            }
                        }
                    }
                } else {
                    eprintln!("WARNING: Rendering image without caption text is not accessibility friendly for users who use screen readers");

                    html! {
                        video id=(block.id) src=(downloadable.src_path());
                    }
                };

                self.downloadables.insert(downloadable);

                Ok(markup)
            }
            BlockType::Callout {
                text,
                children,
                icon,
            } => {
                let icon = match icon {
                    // Accessible emojis:
                    // https://adrianroselli.com/2016/12/accessible-emoji-tweaked.html
                    EmojiOrFile::Emoji(emoji) => {
                        let label =
                            emoji::lookup_by_glyph::lookup(&emoji.emoji).map(|emoji| emoji.name);

                        html! {
                            span role="img" aria-label=[label] {
                                (emoji.emoji)
                            }
                        }
                    }
                    EmojiOrFile::File(file) => {
                        eprintln!("WARNING: Using images as callout icon results in images that don't have accessible alt text");

                        let downloadable = file.as_downloadable(block.id)?;

                        let markup = html! {
                            img src=(downloadable.src_path());
                        };

                        self.downloadables.insert(downloadable);

                        markup
                    }
                };

                Ok(html! {
                    aside id=(block.id) {
                        div {
                            (icon)
                        }
                        div {
                            p {
                                (self.render_rich_text(text))
                            }
                            @for child in self.render_blocks(children, Some("indent"), downgrade_headings) {
                                (child?)
                            }
                        }
                    }
                })
            }
            _ => Ok(html! {
                h4 id=(block.id) style="color: red;" class=[class] {
                    "UNSUPPORTED FEATURE: " (block.name())
                }
            }),
        }
    }

    pub fn render_heading(
        &self,
        id: NotionId,
        class: Option<&str>,
        heading: Heading,
        text: &[RichText],
    ) -> Markup {
        let content = match self.heading_anchors {
            HeadingAnchors::Before(icon) => html! {
                (render_heading_icon(id, icon))
                (" ")
                (self.render_rich_text(text))
            },
            HeadingAnchors::After(icon) => html! {
                (self.render_rich_text(text))
                (" ")
                (render_heading_icon(id, icon))
            },
            HeadingAnchors::None => html! {
                (self.render_rich_text(text))
            },
        };

        match heading {
            Heading::H1 => html! {
                h1 id=(id) class=[class] {
                    (content)
                }
            },
            Heading::H2 => html! {
                h2 id=(id) class=[class] {
                    (content)
                }
            },
            Heading::H3 => html! {
                h3 id=(id) class=[class] {
                    (content)
                }
            },
            Heading::H4 => html! {
                h4 id=(id) class=[class] {
                    (content)
                }
            },
            Heading::H5 => html! {
                h5 id=(id) class=[class] {
                    (content)
                }
            },
            Heading::H6 => html! {
                h6 id=(id) class=[class] {
                    (content)
                }
            },
        }
    }

    pub fn render_rich_text(&self, rich_text: &[RichText]) -> Markup {
        html! {
            @for segment in rich_text {
                (RichTextRenderer::new(segment, self))
            }
        }
    }
}

struct RichTextRenderer<'a> {
    rich_text: &'a RichText,
    current_pages: &'a HashSet<NotionId>,
    link_map: &'a HashMap<NotionId, String>,
}

impl<'a> RichTextRenderer<'a> {
    fn new(rich_text: &'a RichText, renderer: &'a HtmlRenderer) -> Self {
        Self {
            rich_text,
            current_pages: &renderer.current_pages,
            link_map: renderer.link_map,
        }
    }

    fn render_link_opening(&self, buffer: &mut String, link: &RichTextLink) {
        buffer.push_str("<a href=\"");

        match link {
            RichTextLink::External { url } => {
                let mut escaped_link = String::with_capacity(url.len());
                let mut escaper = Escaper::new(&mut escaped_link);
                escaper.write_str(url).expect("unreachable");
                buffer.push_str(&escaped_link);

                // Ensure external links open in a new tab
                // We close href's string and then put target and rel. Rel's string
                // still needs to be closed and so that will happen below
                buffer.push_str(r#"" target="_blank" rel="noreferrer noopener"#);
            }
            RichTextLink::Internal { page, block } => {
                match (self.current_pages.contains(page), block) {
                    (true, Some(block)) => {
                        buffer.push('#');
                        buffer.push_str(block);
                    }
                    (true, None) => {
                        buffer.push('#');
                        buffer.push_str(&page.to_string());
                    }
                    (false, block) => {
                        if let Some(path) = self.link_map.get(page) {
                            buffer.push_str(path);
                        } else {
                            buffer.push('/');
                            buffer.push_str(&page.to_string());
                        }

                        if let Some(block) = block {
                            buffer.push('#');
                            buffer.push_str(block);
                        }
                    }
                }
            }
        }

        buffer.push_str("\">");
    }

    fn render_link_closing(&self, buffer: &mut String) {
        buffer.push_str("</a>");
    }
}

impl<'a> Render for RichTextRenderer<'a> {
    fn render_to(&self, buffer: &mut String) {
        match &self.rich_text.ty {
            RichTextType::Text { content, link } => {
                // TODO: Handle colors
                if self.rich_text.annotations.bold {
                    buffer.push_str("<strong>");
                }
                if self.rich_text.annotations.italic {
                    buffer.push_str("<em>");
                }
                if self.rich_text.annotations.strikethrough {
                    buffer.push_str("<del>");
                }
                if self.rich_text.annotations.underline {
                    buffer.push_str(r#"<span class="underline">"#);
                }
                if self.rich_text.annotations.code {
                    buffer.push_str("<code>");
                }
                if let Some(link) = link {
                    self.render_link_opening(buffer, link)
                }

                let mut escaped_content = String::with_capacity(content.len());
                let mut escape = Escaper::new(&mut escaped_content);
                escape.write_str(content).expect("unreachable");
                buffer.push_str(&escaped_content);

                if link.is_some() {
                    self.render_link_closing(buffer);
                }
                if self.rich_text.annotations.code {
                    buffer.push_str("</code>");
                }
                if self.rich_text.annotations.underline {
                    buffer.push_str("</span>");
                }
                if self.rich_text.annotations.strikethrough {
                    buffer.push_str("</del>");
                }
                if self.rich_text.annotations.italic {
                    buffer.push_str("</em>");
                }
                if self.rich_text.annotations.bold {
                    buffer.push_str("</strong>");
                }
            }
            RichTextType::Equation { expression } => match katex::render(expression) {
                Ok(rendered_expression) => {
                    // We don't skip KaTeX output because it returns actual HTML
                    // TODO: Should we enable anything special to make it so KaTeX sandboxes
                    // its parsing or is it already safe?
                    buffer.push_str(&rendered_expression);
                }
                Err(error) => {
                    eprintln!("{}", error);
                }
            },
            RichTextType::Mention { mention } => match mention {
                RichTextMentionType::Date(date) => {
                    append_html_datetime(buffer, &date.start);
                    if let Some(end) = &date.end {
                        buffer.push_str(" to ");
                        append_html_datetime(buffer, end);
                    }

                    fn append_html_datetime(buffer: &mut String, time: &Time) {
                        use time::{format_description::FormatItem, macros::format_description};

                        buffer.push_str("<time datetime=\"");

                        // We rely on Notion's timestamps to be HTML compliant
                        // They have two timestamp formats, one for dates only: 2021-12-06
                        // and one for datetime which seems to be Rfc3339 compliant but with
                        // only 3 subsecond places, which is exactly what we need
                        buffer.push_str(time.original());
                        buffer.push_str("\">");

                        const READABLE_DATE: &[FormatItem<'_>] =
                            format_description!("[month repr:long] [day], [year]");
                        const READABLE_DATETIME: &[FormatItem<'_>] = format_description!(
                            "[month repr:long] [day], [year] [hour repr:12]:[minute] [period case:lower]"
                        );

                        match time.get_date() {
                            Ok(date) => buffer.push_str(&date.format(READABLE_DATE).unwrap()),
                            // TODO: Either of the following
                            // 1) Support letting people customize the timezone for all blocks
                            // 2) Detect the timezone name and append it
                            // 3) Ask Notion devs to add timezone name to API response
                            Err(datetime) => buffer.push_str(
                                &datetime
                                    .to_offset(time::UtcOffset::UTC)
                                    .format(READABLE_DATETIME)
                                    .unwrap(),
                            ),
                        };

                        buffer.push_str("</time>");
                    }
                }
                &RichTextMentionType::Page { id } => {
                    self.render_link_opening(
                        buffer,
                        &RichTextLink::Internal {
                            page: id,
                            block: None,
                        },
                    );

                    buffer.push_str(&self.rich_text.plain_text);

                    self.render_link_closing(buffer);
                }
                // TODO: link_previews can be so much nicer if we actually query the link and get
                // the HTML data from it and look inside of it for the meta og:title with a fallback
                // to the <title> and the same for favicons so that we can render the icon next to
                // the link's title much like what Notion does
                RichTextMentionType::LinkPreview { url } => {
                    self.render_link_opening(
                        buffer,
                        &RichTextLink::External {
                            url: url.to_string(),
                        },
                    );

                    let mut escaped_content = String::with_capacity(url.len());
                    let mut escape = Escaper::new(&mut escaped_content);
                    escape.write_str(url).expect("unreachable");
                    buffer.push_str(&escaped_content);

                    self.render_link_closing(buffer);
                }
                _ => todo!(),
            },
        }
    }
}

const UUID_WITHOUT_DASHES_LENGTH: usize = 32;
const HEADING_LINK_ICON_LENGTH: usize = 1 + UUID_WITHOUT_DASHES_LENGTH;

fn render_heading_icon(id: NotionId, icon: &str) -> Markup {
    let mut link = String::with_capacity(HEADING_LINK_ICON_LENGTH);
    link.push('#');
    link.push_str(&id.to_string());

    html! {
        a href=(link) {
            (icon)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{HtmlRenderer, RichTextRenderer, Title};
    use crate::{
        download::{Downloadable, Downloadables},
        options::HeadingAnchors,
        response::{
            properties::TitleProperty, Annotations, Block, BlockType, Color, Emoji, EmojiOrFile,
            File, Language, NotionDate, Page, PageParent, RichText, RichTextLink,
            RichTextMentionType, RichTextType,
        },
    };
    use maud::Render;
    use pretty_assertions::assert_eq;
    use reqwest::Url;
    use std::{
        collections::{HashMap, HashSet},
        path::PathBuf,
    };

    #[test]
    fn render_unsupported() {
        let renderer = HtmlRenderer {
            heading_anchors: HeadingAnchors::None,
            current_pages: HashSet::from(["46f8638c25a84ccd9d926e42bdb5535e".parse().unwrap()]),
            link_map: &HashMap::new(),
            downloadables: &Downloadables::new(),
        };

        let block = Block {
            object: "block".to_string(),
            id: "eb39a20e-1036-4469-b750-a9df8f4f18df".parse().unwrap(),
            created_time: "2021-11-13T17:37:00.000Z".to_string(),
            last_edited_time: "2021-11-13T17:37:00.000Z".to_string(),
            has_children: false,
            archived: false,
            ty: BlockType::TableOfContents {},
        };

        let markup = renderer
            .render_block(&block, None, 0)
            .map(|markup| markup.into_string())
            .unwrap();
        assert_eq!(
            markup,
            r#"<h4 id="eb39a20e10364469b750a9df8f4f18df" style="color: red;">UNSUPPORTED FEATURE: table_of_contents</h4>"#
        );
        let guard = renderer.downloadables.set.guard();
        assert_eq!(
            renderer
                .downloadables
                .set
                .iter(&guard)
                .collect::<HashSet<&Downloadable>>(),
            HashSet::new()
        );
    }

    #[test]
    fn render_headings_without_anchors() {
        let renderer = HtmlRenderer {
            heading_anchors: HeadingAnchors::None,
            current_pages: HashSet::from(["46f8638c25a84ccd9d926e42bdb5535e".parse().unwrap()]),
            link_map: &HashMap::new(),
            downloadables: &Downloadables::new(),
        };

        let block = Block {
            object: "block".to_string(),
            id: "8cac60c2-74b9-408c-acbd-0895cfd7b7f8".parse().unwrap(),
            created_time: "2021-11-13T17:35:00.000Z".to_string(),
            last_edited_time: "2021-11-13T19:02:00.000Z".to_string(),
            has_children: false,
            archived: false,
            ty: BlockType::HeadingOne {
                text: vec![RichText {
                    plain_text: "Cool test".to_string(),
                    href: None,
                    annotations: Default::default(),
                    ty: RichTextType::Text {
                        content: "Cool test".to_string(),
                        link: None,
                    },
                }],
            },
        };
        let markup = renderer
            .render_block(&block, None, 0)
            .map(|markup| markup.into_string())
            .unwrap();
        assert_eq!(
            markup,
            r#"<h1 id="8cac60c274b9408cacbd0895cfd7b7f8">Cool test</h1>"#
        );
        let guard = renderer.downloadables.set.guard();
        assert_eq!(
            renderer
                .downloadables
                .set
                .iter(&guard)
                .collect::<HashSet<&Downloadable>>(),
            HashSet::new()
        );

        let block = Block {
            object: "block".to_string(),
            id: "8042c69c-49e7-420b-a498-39b9d61c43d0".parse().unwrap(),
            created_time: "2021-11-13T17:35:00.000Z".to_string(),
            last_edited_time: "2021-11-13T19:02:00.000Z".to_string(),
            has_children: false,
            archived: false,
            ty: BlockType::HeadingTwo {
                text: vec![RichText {
                    plain_text: "Cooler test".to_string(),
                    href: None,
                    annotations: Default::default(),
                    ty: RichTextType::Text {
                        content: "Cooler test".to_string(),
                        link: None,
                    },
                }],
            },
        };
        let markup = renderer
            .render_block(&block, None, 0)
            .map(|markup| markup.into_string())
            .unwrap();
        assert_eq!(
            markup,
            r#"<h2 id="8042c69c49e7420ba49839b9d61c43d0">Cooler test</h2>"#
        );
        let guard = renderer.downloadables.set.guard();
        assert_eq!(
            renderer
                .downloadables
                .set
                .iter(&guard)
                .collect::<HashSet<&Downloadable>>(),
            HashSet::new()
        );

        let block = Block {
            object: "block".to_string(),
            id: "7f54fffa-6108-4a49-b8e9-587afe7ac08f".parse().unwrap(),
            created_time: "2021-11-13T17:35:00.000Z".to_string(),
            last_edited_time: "2021-11-13T19:02:00.000Z".to_string(),
            has_children: false,
            archived: false,
            ty: BlockType::HeadingThree {
                text: vec![RichText {
                    plain_text: "Coolest test".to_string(),
                    href: None,
                    annotations: Default::default(),
                    ty: RichTextType::Text {
                        content: "Coolest test".to_string(),
                        link: None,
                    },
                }],
            },
        };
        let markup = renderer
            .render_block(&block, None, 0)
            .map(|markup| markup.into_string())
            .unwrap();
        assert_eq!(
            markup,
            r#"<h3 id="7f54fffa61084a49b8e9587afe7ac08f">Coolest test</h3>"#
        );
        let guard = renderer.downloadables.set.guard();
        assert_eq!(
            renderer
                .downloadables
                .set
                .iter(&guard)
                .collect::<HashSet<&Downloadable>>(),
            HashSet::new()
        );
    }

    #[test]
    fn render_headings_with_before_anchors() {
        let renderer = HtmlRenderer {
            heading_anchors: HeadingAnchors::Before("#"),
            current_pages: HashSet::from(["46f8638c25a84ccd9d926e42bdb5535e".parse().unwrap()]),
            link_map: &HashMap::new(),
            downloadables: &Downloadables::new(),
        };

        let block = Block {
            object: "block".to_string(),
            id: "8cac60c2-74b9-408c-acbd-0895cfd7b7f8".parse().unwrap(),
            created_time: "2021-11-13T17:35:00.000Z".to_string(),
            last_edited_time: "2021-11-13T19:02:00.000Z".to_string(),
            has_children: false,
            archived: false,
            ty: BlockType::HeadingOne {
                text: vec![RichText {
                    plain_text: "Cool test".to_string(),
                    href: None,
                    annotations: Default::default(),
                    ty: RichTextType::Text {
                        content: "Cool test".to_string(),
                        link: None,
                    },
                }],
            },
        };
        let markup = renderer
            .render_block(&block, None, 0)
            .map(|markup| markup.into_string())
            .unwrap();
        assert_eq!(
            markup,
            r##"<h1 id="8cac60c274b9408cacbd0895cfd7b7f8"><a href="#8cac60c274b9408cacbd0895cfd7b7f8">#</a> Cool test</h1>"##
        );
        let guard = renderer.downloadables.set.guard();
        assert_eq!(
            renderer
                .downloadables
                .set
                .iter(&guard)
                .collect::<HashSet<&Downloadable>>(),
            HashSet::new()
        );

        let block = Block {
            object: "block".to_string(),
            id: "8042c69c-49e7-420b-a498-39b9d61c43d0".parse().unwrap(),
            created_time: "2021-11-13T17:35:00.000Z".to_string(),
            last_edited_time: "2021-11-13T19:02:00.000Z".to_string(),
            has_children: false,
            archived: false,
            ty: BlockType::HeadingTwo {
                text: vec![RichText {
                    plain_text: "Cooler test".to_string(),
                    href: None,
                    annotations: Default::default(),
                    ty: RichTextType::Text {
                        content: "Cooler test".to_string(),
                        link: None,
                    },
                }],
            },
        };
        let markup = renderer
            .render_block(&block, None, 0)
            .map(|markup| markup.into_string())
            .unwrap();
        assert_eq!(
            markup,
            r##"<h2 id="8042c69c49e7420ba49839b9d61c43d0"><a href="#8042c69c49e7420ba49839b9d61c43d0">#</a> Cooler test</h2>"##
        );
        let guard = renderer.downloadables.set.guard();
        assert_eq!(
            renderer
                .downloadables
                .set
                .iter(&guard)
                .collect::<HashSet<&Downloadable>>(),
            HashSet::new()
        );

        let block = Block {
            object: "block".to_string(),
            id: "7f54fffa-6108-4a49-b8e9-587afe7ac08f".parse().unwrap(),
            created_time: "2021-11-13T17:35:00.000Z".to_string(),
            last_edited_time: "2021-11-13T19:02:00.000Z".to_string(),
            has_children: false,
            archived: false,
            ty: BlockType::HeadingThree {
                text: vec![RichText {
                    plain_text: "Coolest test".to_string(),
                    href: None,
                    annotations: Default::default(),
                    ty: RichTextType::Text {
                        content: "Coolest test".to_string(),
                        link: None,
                    },
                }],
            },
        };
        let markup = renderer
            .render_block(&block, None, 0)
            .map(|markup| markup.into_string())
            .unwrap();
        assert_eq!(
            markup,
            r##"<h3 id="7f54fffa61084a49b8e9587afe7ac08f"><a href="#7f54fffa61084a49b8e9587afe7ac08f">#</a> Coolest test</h3>"##
        );
        let guard = renderer.downloadables.set.guard();
        assert_eq!(
            renderer
                .downloadables
                .set
                .iter(&guard)
                .collect::<HashSet<&Downloadable>>(),
            HashSet::new()
        );
    }

    #[test]
    fn render_headings_with_after_anchors() {
        let renderer = HtmlRenderer {
            heading_anchors: HeadingAnchors::After("#"),
            current_pages: HashSet::from(["46f8638c25a84ccd9d926e42bdb5535e".parse().unwrap()]),
            link_map: &HashMap::new(),
            downloadables: &Downloadables::new(),
        };

        let block = Block {
            object: "block".to_string(),
            id: "8cac60c2-74b9-408c-acbd-0895cfd7b7f8".parse().unwrap(),
            created_time: "2021-11-13T17:35:00.000Z".to_string(),
            last_edited_time: "2021-11-13T19:02:00.000Z".to_string(),
            has_children: false,
            archived: false,
            ty: BlockType::HeadingOne {
                text: vec![RichText {
                    plain_text: "Cool test".to_string(),
                    href: None,
                    annotations: Default::default(),
                    ty: RichTextType::Text {
                        content: "Cool test".to_string(),
                        link: None,
                    },
                }],
            },
        };
        let markup = renderer
            .render_block(&block, None, 0)
            .map(|markup| markup.into_string())
            .unwrap();
        assert_eq!(
            markup,
            r##"<h1 id="8cac60c274b9408cacbd0895cfd7b7f8">Cool test <a href="#8cac60c274b9408cacbd0895cfd7b7f8">#</a></h1>"##
        );
        let guard = renderer.downloadables.set.guard();
        assert_eq!(
            renderer
                .downloadables
                .set
                .iter(&guard)
                .collect::<HashSet<&Downloadable>>(),
            HashSet::new()
        );
    }

    #[test]
    fn render_divider() {
        let renderer = HtmlRenderer {
            heading_anchors: HeadingAnchors::None,
            current_pages: HashSet::from(["46f8638c25a84ccd9d926e42bdb5535e".parse().unwrap()]),
            link_map: &HashMap::new(),
            downloadables: &Downloadables::new(),
        };

        let block = Block {
            object: "block".to_string(),
            id: "5e845049-255f-4232-96fd-6f20449be0bc".parse().unwrap(),
            created_time: "2021-11-15T21:56:00.000Z".to_string(),
            last_edited_time: "2021-11-15T21:56:00.000Z".to_string(),
            has_children: false,
            archived: false,
            ty: BlockType::Divider {},
        };

        let markup = renderer
            .render_block(&block, None, 0)
            .map(|markup| markup.into_string())
            .unwrap();
        assert_eq!(markup, r#"<hr id="5e845049255f423296fd6f20449be0bc">"#);
        let guard = renderer.downloadables.set.guard();
        assert_eq!(
            renderer
                .downloadables
                .set
                .iter(&guard)
                .collect::<HashSet<&Downloadable>>(),
            HashSet::new()
        );
    }

    #[test]
    fn render_paragraphs() {
        let renderer = HtmlRenderer {
            heading_anchors: HeadingAnchors::None,
            current_pages: HashSet::from(["46f8638c25a84ccd9d926e42bdb5535e".parse().unwrap()]),
            link_map: &HashMap::new(),
            downloadables: &Downloadables::new(),
        };

        let block = Block {
            object: "block".to_string(),
            id: "64740ca6-3a06-4694-8845-401688334ef5".parse().unwrap(),
            created_time: "2021-11-13T17:35:00.000Z".to_string(),
            last_edited_time: "2021-11-13T19:02:00.000Z".to_string(),
            has_children: false,
            archived: false,
            ty: BlockType::Paragraph {
                text: vec![RichText {
                    plain_text: "Cool test".to_string(),
                    href: None,
                    annotations: Default::default(),
                    ty: RichTextType::Text {
                        content: "Cool test".to_string(),
                        link: None,
                    },
                }],
                children: vec![],
            },
        };
        let markup = renderer
            .render_block(&block, None, 0)
            .map(|markup| markup.into_string())
            .unwrap();
        assert_eq!(
            markup,
            r#"<p id="64740ca63a0646948845401688334ef5">Cool test</p>"#
        );
        let guard = renderer.downloadables.set.guard();
        assert_eq!(
            renderer
                .downloadables
                .set
                .iter(&guard)
                .collect::<HashSet<&Downloadable>>(),
            HashSet::new()
        );

        let block = Block {
            object: "block".to_string(),
            id: "4f2efd79-ae9a-4684-827c-6b69743d6c5d".parse().unwrap(),
            created_time: "2021-11-13T17:35:00.000Z".to_string(),
            last_edited_time: "2021-11-16T11:23:00.000Z".to_string(),
            has_children: true,
            archived: false,
            ty: BlockType::Paragraph {
                text: vec![
                    RichText {
                        plain_text: "Or you can just leave an empty line in between if you want it to leave extra breathing room.".to_string(),
                        href: None,
                        annotations: Default::default(),
                        ty: RichTextType::Text {
                            content: "Or you can just leave an empty line in between if you want it to leave extra breathing room.".to_string(),
                            link: None,
                        },
                    },
                ],
                children: vec![
                    Block {
                        object: "block".to_string(),
                        id: "4fb9dd79-2fc7-45b1-b3a2-8efae49992ed".parse().unwrap(),
                        created_time: "2021-11-15T18:03:00.000Z".to_string(),
                        last_edited_time: "2021-11-16T11:23:00.000Z".to_string(),
                        has_children: true,
                        archived: false,
                        ty: BlockType::Paragraph {
                            text: vec![
                                RichText {
                                    plain_text: "You can also create these rather interesting nested paragraphs".to_string(),
                                    href: None,
                                    annotations: Default::default(),
                                    ty: RichTextType::Text {
                                        content: "You can also create these rather interesting nested paragraphs".to_string(),
                                        link: None,
                                    },
                                },
                            ],
                            children: vec![
                                Block {
                                    object: "block".to_string(),
                                    id: "817c0ca1-721a-4565-ac54-eedbbe471f0b".parse().unwrap(),
                                    created_time: "2021-11-16T11:23:00.000Z".to_string(),
                                    last_edited_time: "2021-11-16T11:23:00.000Z".to_string(),
                                    has_children: false,
                                    archived: false,
                                    ty: BlockType::Paragraph {
                                        text: vec![
                                            RichText {
                                                plain_text: "Possibly more than once too!".to_string(),
                                                href: None,
                                                annotations: Default::default(),
                                                ty: RichTextType::Text {
                                                    content: "Possibly more than once too!".to_string(),
                                                    link: None,
                                                },
                                            },
                                        ],
                                        children: vec![],
                                    },
                                },
                            ],
                        },
                    },
                ],
            },
        };

        let markup = renderer
            .render_block(&block, None, 0)
            .map(|markup| markup.into_string())
            .unwrap();
        assert_eq!(
            markup,
            r#"<div id="4f2efd79ae9a4684827c6b69743d6c5d"><p>Or you can just leave an empty line in between if you want it to leave extra breathing room.</p><div id="4fb9dd792fc745b1b3a28efae49992ed" class="indent"><p>You can also create these rather interesting nested paragraphs</p><p id="817c0ca1721a4565ac54eedbbe471f0b" class="indent">Possibly more than once too!</p></div></div>"#
        );
        let guard = renderer.downloadables.set.guard();
        assert_eq!(
            renderer
                .downloadables
                .set
                .iter(&guard)
                .collect::<HashSet<&Downloadable>>(),
            HashSet::new()
        );
    }

    #[test]
    fn render_quote() {
        let renderer = HtmlRenderer {
            heading_anchors: HeadingAnchors::None,
            current_pages: HashSet::from(["46f8638c25a84ccd9d926e42bdb5535e".parse().unwrap()]),
            link_map: &HashMap::new(),
            downloadables: &Downloadables::new(),
        };

        let block = Block {
            object: "block".to_string(),
            id: "191b3d44-a37f-40c4-bb4f-3477359022fd".parse().unwrap(),
            created_time: "2021-11-13T18:58:00.000Z".to_string(),
            last_edited_time: "2021-11-13T19:00:00.000Z".to_string(),
            has_children: false,
            archived: false,
            ty: BlockType::Quote {
                text: vec![
                    RichText {
                        plain_text: "If you think you can do a thing or think you can’t do a thing, you’re right.\n—Henry Ford".to_string(),
                        href: None,
                        annotations: Default::default(),
                        ty: RichTextType::Text {
                            content: "If you think you can do a thing or think you can’t do a thing, you’re right.\n—Henry Ford".to_string(),
                            link: None,
                        },
                    },
                ],
                children: vec![],
            },
        };

        let markup = renderer
            .render_block(&block, None, 0)
            .map(|markup| markup.into_string())
            .unwrap();
        assert_eq!(
            markup,
            r#"<blockquote id="191b3d44a37f40c4bb4f3477359022fd">If you think you can do a thing or think you can’t do a thing, you’re right.
—Henry Ford</blockquote>"#
        );
        let guard = renderer.downloadables.set.guard();
        assert_eq!(
            renderer
                .downloadables
                .set
                .iter(&guard)
                .collect::<HashSet<&Downloadable>>(),
            HashSet::new()
        );
    }

    #[test]
    fn render_code() {
        let renderer = HtmlRenderer {
            heading_anchors: HeadingAnchors::None,
            current_pages: HashSet::from(["46f8638c25a84ccd9d926e42bdb5535e".parse().unwrap()]),
            link_map: &HashMap::new(),
            downloadables: &Downloadables::new(),
        };

        let block = Block {
            object: "block".to_string(),
            id: "bf0128fd-3b85-4d85-aada-e500dcbcda35".parse().unwrap(),
            created_time: "2021-11-13T17:35:00.000Z".to_string(),
            last_edited_time: "2021-11-13T17:38:00.000Z".to_string(),
            has_children: false,
            archived: false,
            ty: BlockType::Code {
                language: Language::Rust,
                text: vec![
                    RichText {
                        plain_text: "struct Magic<T> {\n    value: T\n}\n\nfn cool() -> Magic<T> {\n    return Magic {\n        value: 100\n    };\n}".to_string(),
                        href: None,
                        annotations: Default::default(),
                        ty: RichTextType::Text {
                            content: "struct Magic<T> {\n    value: T\n}\n\nfn cool() -> Magic<T> {\n    return Magic {\n        value: 100\n    };\n}".to_string(),
                            link: None,
                        },
                    },
                ],
            },
        };

        let markup = renderer
            .render_block(&block, None, 0)
            .map(|markup| markup.into_string())
            .unwrap();
        assert_eq!(
            markup,
            r#"<pre id="bf0128fd3b854d85aadae500dcbcda35" class="rust"><code class="rust">"#
                .to_string()
                + r#"<span class="keyword">struct</span> <span class="type">Magic</span><span class="punctuation">&lt;</span><span class="type">T</span><span class="punctuation">&gt;</span> <span class="punctuation">{</span>"#
                + "\n"
                + r#"    <span class="variable">value</span>: <span class="type">T</span>"#
                + "\n"
                + r#"<span class="punctuation">}</span>"#
                + "\n"
                + "\n"
                + r#"<span class="keyword">fn</span> <span class="function">cool</span><span class="punctuation">(</span><span class="punctuation">)</span> <span class="operator">-&gt;</span> <span class="type">Magic</span><span class="punctuation">&lt;</span><span class="type">T</span><span class="punctuation">&gt;</span> <span class="punctuation">{</span>"#
                + "\n"
                + r#"    <span class="keyword">return</span> <span class="type">Magic</span> <span class="punctuation">{</span>"#
                + "\n"
                + r#"        <span class="variable">value</span>: <span class="constant numeric">100</span>"#
                + "\n"
                + r#"    <span class="punctuation">}</span><span class="punctuation">;</span>"#
                + "\n"
                + r#"<span class="punctuation">}</span>"#
                + "\n"
                + r#"</code></pre>"#
        );
        let guard = renderer.downloadables.set.guard();
        assert_eq!(
            renderer
                .downloadables
                .set
                .iter(&guard)
                .collect::<HashSet<&Downloadable>>(),
            HashSet::new()
        );
    }

    #[test]
    fn render_lists() {
        let renderer = HtmlRenderer {
            heading_anchors: HeadingAnchors::None,
            current_pages: HashSet::from(["46f8638c25a84ccd9d926e42bdb5535e".parse().unwrap()]),
            link_map: &HashMap::new(),
            downloadables: &Downloadables::new(),
        };

        let block = Block {
            object: "block".to_string(),
            id: "844b3fdf-5688-4f6c-91e8-97b4f0e436cd".parse().unwrap(),
            created_time: "2021-11-13T19:02:00.000Z".to_string(),
            last_edited_time: "2021-11-13T19:03:00.000Z".to_string(),
            has_children: true,
            archived: false,
            ty: BlockType::BulletedListItem {
                text: vec![RichText {
                    plain_text: "This is some cool list".to_string(),
                    href: None,
                    annotations: Default::default(),
                    ty: RichTextType::Text {
                        content: "This is some cool list".to_string(),
                        link: None,
                    },
                }],
                children: vec![Block {
                    object: "block".to_string(),
                    id: "c3e9c471-d4b3-47dc-ab6a-6ecd4dda161a".parse().unwrap(),
                    created_time: "2021-11-13T19:02:00.000Z".to_string(),
                    last_edited_time: "2021-11-13T19:03:00.000Z".to_string(),
                    has_children: true,
                    archived: false,
                    ty: BlockType::NumberedListItem {
                        text: vec![RichText {
                            plain_text: "It can even contain other lists inside of it".to_string(),
                            href: None,
                            annotations: Default::default(),
                            ty: RichTextType::Text {
                                content: "It can even contain other lists inside of it".to_string(),
                                link: None,
                            },
                        }],
                        children: vec![Block {
                            object: "block".to_string(),
                            id: "55d72942-49f6-49f9-8ade-e3d049f682e5".parse().unwrap(),
                            created_time: "2021-11-13T19:03:00.000Z".to_string(),
                            last_edited_time: "2021-11-13T19:03:00.000Z".to_string(),
                            has_children: true,
                            archived: false,
                            ty: BlockType::BulletedListItem {
                                text: vec![RichText {
                                    plain_text: "And those lists can contain OTHER LISTS!"
                                        .to_string(),
                                    href: None,
                                    annotations: Default::default(),
                                    ty: RichTextType::Text {
                                        content: "And those lists can contain OTHER LISTS!"
                                            .to_string(),
                                        link: None,
                                    },
                                }],
                                children: vec![
                                    Block {
                                        object: "block".to_string(),
                                        id: "100116e2-0a47-4903-8b79-4ac9cc3a7870".parse().unwrap(),
                                        created_time: "2021-11-13T19:03:00.000Z".to_string(),
                                        last_edited_time: "2021-11-13T19:03:00.000Z".to_string(),
                                        has_children: false,
                                        archived: false,
                                        ty: BlockType::NumberedListItem {
                                            text: vec![RichText {
                                                plain_text: "Listception".to_string(),
                                                href: None,
                                                annotations: Default::default(),
                                                ty: RichTextType::Text {
                                                    content: "Listception".to_string(),
                                                    link: None,
                                                },
                                            }],
                                            children: vec![],
                                        },
                                    },
                                    Block {
                                        object: "block".to_string(),
                                        id: "c1a5555a-8359-4999-80dc-10241d262071".parse().unwrap(),
                                        created_time: "2021-11-13T19:03:00.000Z".to_string(),
                                        last_edited_time: "2021-11-13T19:03:00.000Z".to_string(),
                                        has_children: false,
                                        archived: false,
                                        ty: BlockType::NumberedListItem {
                                            text: vec![RichText {
                                                plain_text: "Listception".to_string(),
                                                href: None,
                                                annotations: Default::default(),
                                                ty: RichTextType::Text {
                                                    content: "Listception".to_string(),
                                                    link: None,
                                                },
                                            }],
                                            children: vec![],
                                        },
                                    },
                                ],
                            },
                        }],
                    },
                }],
            },
        };

        let markup = renderer
            .render_block(&block, None, 0)
            .map(|markup| markup.into_string())
            .unwrap();
        assert_eq!(
            markup,
            r#"<ul><li id="844b3fdf56884f6c91e897b4f0e436cd">This is some cool list<ol><li id="c3e9c471d4b347dcab6a6ecd4dda161a">It can even contain other lists inside of it<ul><li id="55d7294249f649f98adee3d049f682e5">And those lists can contain OTHER LISTS!<ol class="indent"><li id="100116e20a4749038b794ac9cc3a7870">Listception</li><li id="c1a5555a8359499980dc10241d262071">Listception</li></ol></li></ul></li></ol></li></ul>"#
        );
        let guard = renderer.downloadables.set.guard();
        assert_eq!(
            renderer
                .downloadables
                .set
                .iter(&guard)
                .collect::<HashSet<&Downloadable>>(),
            HashSet::new()
        );
    }

    #[test]
    fn render_images() {
        let renderer = HtmlRenderer {
            heading_anchors: HeadingAnchors::None,
            current_pages: HashSet::from(["46f8638c25a84ccd9d926e42bdb5535e".parse().unwrap()]),
            link_map: &HashMap::new(),
            downloadables: &Downloadables::new(),
        };

        let blocks = [
                    Block {
                        object: "block".to_string(),
                        id: "5ac94d7e-25de-4fa3-a781-0a43aac9d5c4".parse().unwrap(),
                        created_time: "2021-11-13T17:35:00.000Z".to_string(),
                        last_edited_time: "2021-11-21T13:39:00.000Z".to_string(),
                        has_children: false,
                        archived: false,
                        ty: BlockType::Image {
                            image: File::Internal {
                                url: "https://s3.us-west-2.amazonaws.com/secure.notion-static.com/efbb73c3-2df3-4365-bcf3-cc9ece431127/circle.png?X-Amz-Algorithm=AWS4-HMAC-SHA256&X-Amz-Content-Sha256=UNSIGNED-PAYLOAD&X-Amz-Credential=AKIAT73L2G45EIPT3X45%2F20211121%2Fus-west-2%2Fs3%2Faws4_request&X-Amz-Date=20211121T134120Z&X-Amz-Expires=3600&X-Amz-Signature=9ea689335e9054f55c794c7609f9c9c057c80484cd06eaf9dff9641d92e923c8&X-Amz-SignedHeaders=host&x-id=GetObject".to_string(),
                                expiry_time: "2021-11-21T14:41:20.026Z".to_string(),
                            },
                            caption: vec![
                                RichText {
                                    plain_text: "Circle rendered in ".to_string(),
                                    href: None,
                                    annotations: Default::default(),
                                    ty: RichTextType::Text {
                                        content: "Circle rendered in ".to_string(),
                                        link: None,
                                    },
                                },
                                RichText {
                                    plain_text: "Bevy".to_string(),
                                    href: None,
                                    annotations: Annotations {
                                        bold: true,
                                        ..Default::default()
                                    },
                                    ty: RichTextType::Text {
                                        content: "Bevy".to_string(),
                                        link: None,
                                    },
                                },
                            ],
                        },
                    },
                    Block {
                        object: "block".to_string(),
                        id: "d1e5e2c5-4351-4b8e-83a3-20ef532967a7".parse().unwrap(),
                        created_time: "2021-11-13T17:35:00.000Z".to_string(),
                        last_edited_time: "2021-11-13T17:35:00.000Z".to_string(),
                        has_children: false,
                        archived: false,
                        ty: BlockType::Image {
                            image: File::External {
                                url: "https://mathspy.me/random-file".to_string(),
                            },
                            caption: vec![],
                        },
                    }
                ];

        let markup = renderer
            .render_blocks(&blocks, None, 0)
            .map(|result| result.unwrap())
            .map(|markup| markup.into_string())
            .collect::<Vec<_>>();
        assert_eq!(
            markup,
            vec![
                r#"<figure id="5ac94d7e25de4fa3a7810a43aac9d5c4"><img src="/media/5ac94d7e25de4fa3a7810a43aac9d5c4.png"><figcaption>Circle rendered in <strong>Bevy</strong></figcaption></figure>"#,
                r#"<img id="d1e5e2c543514b8e83a320ef532967a7" src="/media/d1e5e2c543514b8e83a320ef532967a7">"#
            ]
        );
        let guard = renderer.downloadables.set.guard();
        assert_eq!(
            renderer
                .downloadables
                .set
                .iter(&guard)
                .collect::<HashSet<&Downloadable>>(),
            HashSet::from([
                &Downloadable::new(
                    Url::parse(
                        "https://s3.us-west-2.amazonaws.com/secure.notion-static.com/efbb73c3-2df3-4365-bcf3-cc9ece431127/circle.png?X-Amz-Algorithm=AWS4-HMAC-SHA256&X-Amz-Content-Sha256=UNSIGNED-PAYLOAD&X-Amz-Credential=AKIAT73L2G45EIPT3X45%2F20211121%2Fus-west-2%2Fs3%2Faws4_request&X-Amz-Date=20211121T134120Z&X-Amz-Expires=3600&X-Amz-Signature=9ea689335e9054f55c794c7609f9c9c057c80484cd06eaf9dff9641d92e923c8&X-Amz-SignedHeaders=host&x-id=GetObject"
                    ).unwrap(),
                    PathBuf::from("media/5ac94d7e25de4fa3a7810a43aac9d5c4.png"),
                ).unwrap(),
                &Downloadable::new(
                    Url::parse("https://mathspy.me/random-file").unwrap(),
                    PathBuf::from("media/d1e5e2c543514b8e83a320ef532967a7"),
                ).unwrap(),
            ])
        );
    }

    #[test]
    fn render_videos() {
        let renderer = HtmlRenderer {
            heading_anchors: HeadingAnchors::None,
            current_pages: HashSet::from(["46f8638c25a84ccd9d926e42bdb5535e".parse().unwrap()]),
            link_map: &HashMap::new(),
            downloadables: &Downloadables::new(),
        };

        let block = Block {
            object: "block".to_string(),
            id: "c180005ad8de4cd587add47d8c2fb0f3".parse().unwrap(),
            created_time: "2022-12-06T00:21:00.000Z".to_string(),
            last_edited_time: "2022-12-06T00:24:00.000Z".to_string(),
            has_children: false,
            archived: false,
            ty: BlockType::Video {
                video: File::Internal {
                    url: "https://s3.us-west-2.amazonaws.com/secure.notion-static.com/a8c4f962-7f7a-45cd-a2a5-ffe295afa355/moving_enemy.mp4?X-Amz-Algorithm=AWS4-HMAC-SHA256&X-Amz-Content-Sha256=UNSIGNED-PAYLOAD&X-Amz-Credential=AKIAT73L2G45EIPT3X45%2F20221206%2Fus-west-2%2Fs3%2Faws4_request&X-Amz-Date=20221206T002733Z&X-Amz-Expires=3600&X-Amz-Signature=039e735804116d7f8cad60c8719fd61ff58438f408c8cf1401a3fbd70939495b&X-Amz-SignedHeaders=host&x-id=GetObject".to_string(),
                    expiry_time: "2022-12-06T01:27:33.514Z".to_string(),
                },
                caption: vec![
                    RichText {
                        plain_text: "A video of two circles, one pink and one red where the pink is moving and stopping based on user input while the red one is chasing it relentlessly at a fixed velocity".to_string(),
                        href: None,
                        annotations: Default::default(),
                        ty: RichTextType::Text {
                            content: "A video of two circles, one pink and one red where the pink is moving and stopping based on user input while the red one is chasing it relentlessly at a fixed velocity".to_string(),
                            link: None,
                        },
                    },
                ],
            },
        };

        let markup = renderer
            .render_block(&block, None, 0)
            .unwrap()
            .into_string();
        assert_eq!(
            markup,
            r#"<figure id="c180005ad8de4cd587add47d8c2fb0f3"><video controls src="/media/c180005ad8de4cd587add47d8c2fb0f3.mp4"><p>Unfortunately looks like your browser doesn't support videos.<a href="/media/c180005ad8de4cd587add47d8c2fb0f3.mp4">But no worries you can click me to download the video!</a></p></video><figcaption>A video of two circles, one pink and one red where the pink is moving and stopping based on user input while the red one is chasing it relentlessly at a fixed velocity</figcaption></figure>"#,
        );
        let guard = renderer.downloadables.set.guard();
        assert_eq!(
            renderer
                .downloadables
                .set
                .iter(&guard)
                .collect::<HashSet<&Downloadable>>(),
            HashSet::from([
                &Downloadable::new(
                    Url::parse(
                        "https://s3.us-west-2.amazonaws.com/secure.notion-static.com/a8c4f962-7f7a-45cd-a2a5-ffe295afa355/moving_enemy.mp4?X-Amz-Algorithm=AWS4-HMAC-SHA256&X-Amz-Content-Sha256=UNSIGNED-PAYLOAD&X-Amz-Credential=AKIAT73L2G45EIPT3X45%2F20221206%2Fus-west-2%2Fs3%2Faws4_request&X-Amz-Date=20221206T002733Z&X-Amz-Expires=3600&X-Amz-Signature=039e735804116d7f8cad60c8719fd61ff58438f408c8cf1401a3fbd70939495b&X-Amz-SignedHeaders=host&x-id=GetObject"
                    ).unwrap(),
                    PathBuf::from("media/c180005ad8de4cd587add47d8c2fb0f3.mp4"),
                ).unwrap(),
            ])
        );
    }

    #[test]
    fn render_callouts() {
        let renderer = HtmlRenderer {
            heading_anchors: HeadingAnchors::None,
            current_pages: HashSet::from(["46f8638c25a84ccd9d926e42bdb5535e".parse().unwrap()]),
            link_map: &HashMap::new(),
            downloadables: &Downloadables::new(),
        };

        let blocks = [
            Block {
                object: "block".to_string(),
                id: "b7363fed-d7cd-4aba-a86f-f51763f4ce91".parse().unwrap(),
                created_time: "2021-11-13T17:50:00.000Z".to_string(),
                last_edited_time: "2021-11-13T17:50:00.000Z".to_string(),
                has_children: false,
                archived: false,
                ty: BlockType::Callout {
                    text: vec![RichText {
                        plain_text: "Some really spooky callout.".to_string(),
                        href: None,
                        annotations: Default::default(),
                        ty: RichTextType::Text {
                            content: "Some really spooky callout.".to_string(),
                            link: None,
                        },
                    }],
                    icon: EmojiOrFile::Emoji(Emoji {
                        emoji: "⚠️".to_string(),
                    }),
                    children: vec![],
                },
            },
            Block {
                object: "block".to_string(),
                id: "28c719a3-9845-4f08-9e87-1fe78e50e92b".parse().unwrap(),
                created_time: "2021-11-13T17:50:00.000Z".to_string(),
                last_edited_time: "2021-11-13T17:50:00.000Z".to_string(),
                has_children: false,
                archived: false,
                ty: BlockType::Callout {
                    text: vec![RichText {
                        plain_text: "Some really spooky callout.".to_string(),
                        href: None,
                        annotations: Default::default(),
                        ty: RichTextType::Text {
                            content: "Some really spooky callout.".to_string(),
                            link: None,
                        },
                    }],
                    icon: EmojiOrFile::File(File::Internal {
                        url: "https://example.com/hehe.gif".to_string(),
                        expiry_time: "2021-11-13T17:50:00.000Z".to_string(),
                    }),
                    children: vec![],
                },
            },
            Block {
                object: "block".to_string(),
                id: "66ea7370-1a3b-4f4e-ada5-3be2f7e6ef73".parse().unwrap(),
                created_time: "2021-11-13T17:50:00.000Z".to_string(),
                last_edited_time: "2021-11-13T17:50:00.000Z".to_string(),
                has_children: false,
                archived: false,
                ty: BlockType::Callout {
                    text: vec![RichText {
                        plain_text: "Some really spooky callout.".to_string(),
                        href: None,
                        annotations: Default::default(),
                        ty: RichTextType::Text {
                            content: "Some really spooky callout.".to_string(),
                            link: None,
                        },
                    }],
                    icon: EmojiOrFile::File(File::External {
                        url: "https://example.com".to_string(),
                    }),
                    children: vec![],
                },
            },
        ];

        let markup = renderer
            .render_blocks(&blocks, None, 0)
            .map(|result| result.unwrap())
            .map(|markup| markup.into_string())
            .collect::<Vec<_>>();

        assert_eq!(
            markup,
            vec![
                r#"<aside id="b7363fedd7cd4abaa86ff51763f4ce91"><div><span role="img" aria-label="warning">⚠️</span></div><div><p>Some really spooky callout.</p></div></aside>"#,
                r#"<aside id="28c719a398454f089e871fe78e50e92b"><div><img src="/media/28c719a398454f089e871fe78e50e92b.gif"></div><div><p>Some really spooky callout.</p></div></aside>"#,
                r#"<aside id="66ea73701a3b4f4eada53be2f7e6ef73"><div><img src="/media/66ea73701a3b4f4eada53be2f7e6ef73"></div><div><p>Some really spooky callout.</p></div></aside>"#
            ]
        );
        let guard = renderer.downloadables.set.guard();
        assert_eq!(
            renderer
                .downloadables
                .set
                .iter(&guard)
                .collect::<HashSet<&Downloadable>>(),
            HashSet::from([
                &Downloadable::new(
                    Url::parse("https://example.com/hehe.gif").unwrap(),
                    PathBuf::from("media/28c719a398454f089e871fe78e50e92b.gif"),
                )
                .unwrap(),
                &Downloadable::new(
                    Url::parse("https://example.com").unwrap(),
                    PathBuf::from("media/66ea73701a3b4f4eada53be2f7e6ef73"),
                )
                .unwrap(),
            ])
        );
    }

    #[test]
    fn display_rich_text_type_text() {
        let renderer = HtmlRenderer {
            heading_anchors: HeadingAnchors::None,
            current_pages: HashSet::new(),
            link_map: &HashMap::new(),
            downloadables: &Downloadables::new(),
        };
        let renderer_with_link_map = HtmlRenderer {
            heading_anchors: HeadingAnchors::None,
            current_pages: HashSet::new(),
            link_map: &HashMap::from([(
                "46f8638c25a84ccd9d926e42bdb5535e".parse().unwrap(),
                "/path/to/page".to_string(),
            )]),
            downloadables: &Downloadables::new(),
        };
        let renderer_with_pages = HtmlRenderer {
            heading_anchors: HeadingAnchors::None,
            current_pages: HashSet::from(["46f8638c25a84ccd9d926e42bdb5535e".parse().unwrap()]),
            link_map: &HashMap::new(),
            downloadables: &Downloadables::new(),
        };
        let text = RichText {
            href: None,
            plain_text: "I love you!".to_string(),
            annotations: Default::default(),
            ty: RichTextType::Text {
                content: "I love you!".to_string(),
                link: None,
            },
        };
        assert_eq!(
            RichTextRenderer::new(&text, &renderer)
                .render()
                .into_string(),
            "I love you!"
        );

        let text = RichText {
            href: None,
            plain_text: "a > 5 but < 3 how?".to_string(),
            annotations: Default::default(),
            ty: RichTextType::Text {
                content: "a > 5 but < 3 how?".to_string(),
                link: None,
            },
        };
        assert_eq!(
            RichTextRenderer::new(&text, &renderer)
                .render()
                .into_string(),
            "a &gt; 5 but &lt; 3 how?"
        );

        let text = RichText {
            href: None,
            plain_text: "boring text".to_string(),
            annotations: Annotations {
                bold: false,
                italic: false,
                strikethrough: false,
                underline: true,
                code: false,
                color: Color::Default,
            },
            ty: RichTextType::Text {
                content: "boring text".to_string(),
                link: Some(RichTextLink::External {
                    url: "https://cool.website/".to_string(),
                }),
            },
        };
        assert_eq!(
            RichTextRenderer::new(&text, &renderer)
                .render()
                .into_string(),
            r#"<span class="underline"><a href="https://cool.website/" target="_blank" rel="noreferrer noopener">boring text</a></span>"#
        );

        let text = RichText {
            href: None,
            plain_text: "Thanks Notion <:angry_face:>".to_string(),
            annotations: Annotations {
                bold: true,
                italic: true,
                strikethrough: true,
                underline: true,
                code: true,
                color: Color::Default,
            },
            ty: RichTextType::Text {
                content: "Thanks Notion <:angry_face:>".to_string(),
                link: Some(RichTextLink::External {
                    url: "https://very.angry/><".to_string(),
                }),
            },
        };
        assert_eq!(
            RichTextRenderer::new(&text, &renderer)
                .render()
                .into_string(),
            r#"<strong><em><del><span class="underline"><code><a href="https://very.angry/&gt;&lt;" target="_blank" rel="noreferrer noopener">Thanks Notion &lt;:angry_face:&gt;</a></code></span></del></em></strong>"#,
        );

        let text = RichText {
            plain_text: "¹".to_string(),
            href: Some(
                "/46f8638c25a84ccd9d926e42bdb5535e#48cb69650f584e60be8159e9f8e07a8a".to_string(),
            ),
            annotations: Default::default(),
            ty: RichTextType::Text {
                content: "¹".to_string(),
                link: Some(RichTextLink::Internal {
                    page: "46f8638c25a84ccd9d926e42bdb5535e".parse().unwrap(),
                    block: Some("48cb69650f584e60be8159e9f8e07a8a".to_string()),
                }),
            },
        };
        assert_eq!(
            RichTextRenderer::new(&text, &renderer_with_pages)
                .render()
                .into_string(),
            r##"<a href="#48cb69650f584e60be8159e9f8e07a8a">¹</a>"##,
        );

        let text = RichText {
            plain_text: "¹".to_string(),
            href: Some(
                "/46f8638c25a84ccd9d926e42bdb5535e#48cb69650f584e60be8159e9f8e07a8a".to_string(),
            ),
            annotations: Default::default(),
            ty: RichTextType::Text {
                content: "¹".to_string(),
                link: Some(RichTextLink::Internal {
                    page: "46f8638c25a84ccd9d926e42bdb5535e".parse().unwrap(),
                    block: None,
                }),
            },
        };
        assert_eq!(
            RichTextRenderer::new(&text, &renderer_with_pages)
                .render()
                .into_string(),
            r##"<a href="#46f8638c25a84ccd9d926e42bdb5535e">¹</a>"##,
        );

        let text = RichText {
            plain_text: "A less watered down test".to_string(),
            href: Some("/46f8638c25a84ccd9d926e42bdb5535e".to_string()),
            annotations: Default::default(),
            ty: RichTextType::Text {
                content: "A less watered down test".to_string(),
                link: Some(RichTextLink::Internal {
                    page: "46f8638c25a84ccd9d926e42bdb5535e".parse().unwrap(),
                    block: None,
                }),
            },
        };
        assert_eq!(
            RichTextRenderer::new(&text, &renderer)
                .render()
                .into_string(),
            r##"<a href="/46f8638c25a84ccd9d926e42bdb5535e">A less watered down test</a>"##,
        );

        let text = RichText {
            plain_text: "A less watered down test".to_string(),
            href: Some("/46f8638c25a84ccd9d926e42bdb5535e".to_string()),
            annotations: Default::default(),
            ty: RichTextType::Text {
                content: "A less watered down test".to_string(),
                link: Some(RichTextLink::Internal {
                    page: "46f8638c25a84ccd9d926e42bdb5535e".parse().unwrap(),
                    block: Some("48cb69650f584e60be8159e9f8e07a8a".to_string()),
                }),
            },
        };
        assert_eq!(
            RichTextRenderer::new(&text, &renderer)
                .render()
                .into_string(),
            r##"<a href="/46f8638c25a84ccd9d926e42bdb5535e#48cb69650f584e60be8159e9f8e07a8a">A less watered down test</a>"##,
        );

        let text = RichText {
            plain_text: "A less watered down test".to_string(),
            href: Some("/46f8638c25a84ccd9d926e42bdb5535e".to_string()),
            annotations: Default::default(),
            ty: RichTextType::Text {
                content: "A less watered down test".to_string(),
                link: Some(RichTextLink::Internal {
                    page: "46f8638c25a84ccd9d926e42bdb5535e".parse().unwrap(),
                    block: None,
                }),
            },
        };
        assert_eq!(
            RichTextRenderer::new(&text, &renderer_with_link_map)
                .render()
                .into_string(),
            r##"<a href="/path/to/page">A less watered down test</a>"##,
        );

        let text = RichText {
            plain_text: "A less watered down test".to_string(),
            href: Some("/46f8638c25a84ccd9d926e42bdb5535e".to_string()),
            annotations: Default::default(),
            ty: RichTextType::Text {
                content: "A less watered down test".to_string(),
                link: Some(RichTextLink::Internal {
                    page: "46f8638c25a84ccd9d926e42bdb5535e".parse().unwrap(),
                    block: Some("48cb69650f584e60be8159e9f8e07a8a".to_string()),
                }),
            },
        };
        assert_eq!(
            RichTextRenderer::new(&text, &renderer_with_link_map)
                .render()
                .into_string(),
            r##"<a href="/path/to/page#48cb69650f584e60be8159e9f8e07a8a">A less watered down test</a>"##,
        );

        let text = RichText {
            plain_text: "2021-11-07T02:59:00.000-08:00 → ".to_string(),
            href: None,
            annotations: Default::default(),
            ty: RichTextType::Mention {
                mention: RichTextMentionType::Date(NotionDate {
                    start: "2021-11-07T02:59:00.000-08:00".parse().unwrap(),
                    end: None,
                    time_zone: None,
                }),
            },
        };

        assert_eq!(
            RichTextRenderer::new(&text, &renderer)
                .render()
                .into_string(),
            r#"<time datetime="2021-11-07T02:59:00.000-08:00">November 07, 2021 10:59 am</time>"#
        );

        let text = RichText {
            plain_text: "2021-12-05 → 2021-12-06".to_string(),
            href: None,
            annotations: Default::default(),
            ty: RichTextType::Mention {
                mention: RichTextMentionType::Date(NotionDate {
                    start: "2021-12-05".parse().unwrap(),
                    end: Some("2021-12-06".parse().unwrap()),
                    time_zone: None,
                }),
            },
        };

        assert_eq!(
            RichTextRenderer::new(&text, &renderer)
                .render()
                .into_string(),
            r#"<time datetime="2021-12-05">December 05, 2021</time> to <time datetime="2021-12-06">December 06, 2021</time>"#
        );

        let text = RichText {
            plain_text: "watereddown-test".to_string(),
            href: Some("https://www.notion.so/6e0eb85f60474efba1304f92d2abfa2c".to_string()),
            annotations: Default::default(),
            ty: RichTextType::Mention {
                mention: RichTextMentionType::Page {
                    id: "6e0eb85f60474efba1304f92d2abfa2c".parse().unwrap(),
                },
            },
        };

        assert_eq!(
            RichTextRenderer::new(&text, &renderer)
                .render()
                .into_string(),
            r#"<a href="/6e0eb85f60474efba1304f92d2abfa2c">watereddown-test</a>"#
        );

        let text = RichText {
            plain_text: "watereddown-test".to_string(),
            href: Some("https://www.notion.so/6e0eb85f60474efba1304f92d2abfa2c".to_string()),
            annotations: Default::default(),
            ty: RichTextType::Mention {
                mention: RichTextMentionType::LinkPreview {
                    url: "https://github.com/Mathspy/flocking_bevy/commit/21e0c3c1b0d198646b840038282c258318ac626e".to_string(),
                },
            },
        };

        assert_eq!(
            RichTextRenderer::new(&text, &renderer)
                .render()
                .into_string(),
            r#"<a href="https://github.com/Mathspy/flocking_bevy/commit/21e0c3c1b0d198646b840038282c258318ac626e" target="_blank" rel="noreferrer noopener">https://github.com/Mathspy/flocking_bevy/commit/21e0c3c1b0d198646b840038282c258318ac626e</a>"#
        );
    }

    #[test]
    fn display_rich_text_type_equation() {
        let renderer = HtmlRenderer {
            heading_anchors: HeadingAnchors::None,
            current_pages: HashSet::from(["46f8638c25a84ccd9d926e42bdb5535e".parse().unwrap()]),
            link_map: &HashMap::new(),
            downloadables: &Downloadables::new(),
        };
        let text = RichText {
            href: None,
            plain_text: "f(x)=y".to_string(),
            annotations: Default::default(),
            ty: RichTextType::Equation {
                expression: "f(x)=y".to_string(),
            },
        };
        assert_eq!(
            RichTextRenderer::new(&text, &renderer)
                .render()
                .into_string(),
            r#"<span class="katex"><span class="katex-mathml"><math xmlns="http://www.w3.org/1998/Math/MathML"><semantics><mrow><mi>f</mi><mo stretchy="false">(</mo><mi>x</mi><mo stretchy="false">)</mo><mo>=</mo><mi>y</mi></mrow><annotation encoding="application/x-tex">f(x)=y</annotation></semantics></math></span><span class="katex-html" aria-hidden="true"><span class="base"><span class="strut" style="height:1em;vertical-align:-0.25em;"></span><span class="mord mathnormal" style="margin-right:0.10764em;">f</span><span class="mopen">(</span><span class="mord mathnormal">x</span><span class="mclose">)</span><span class="mspace" style="margin-right:0.2778em;"></span><span class="mrel">=</span><span class="mspace" style="margin-right:0.2778em;"></span></span><span class="base"><span class="strut" style="height:0.625em;vertical-align:-0.1944em;"></span><span class="mord mathnormal" style="margin-right:0.03588em;">y</span></span></span></span>"#
        )
    }

    #[test]
    fn render_page() {
        struct Properties {
            name: TitleProperty,
        }
        impl Title for Properties {
            fn title(&self) -> &[RichText] {
                self.name.title.as_slice()
            }
        }

        let renderer = HtmlRenderer {
            heading_anchors: HeadingAnchors::None,
            current_pages: HashSet::from(["ac3fb543001f4be5a25e4978abd05b1d".parse().unwrap()]),
            link_map: &HashMap::new(),
            downloadables: &Downloadables::new(),
        };
        let page = Page {
            object: "page".to_string(),
            id: "ac3fb543-001f-4be5-a25e-4978abd05b1d".parse().unwrap(),
            created_time: "2021-11-29T18:20:00.000Z".to_string(),
            last_edited_time: "2021-12-06T09:25:00.000Z".to_string(),
            cover: None,
            icon: None,
            archived: false,
            properties: Properties {
                name: TitleProperty {
                    id: "QPqF".to_string(),
                    title: vec![RichText {
                        plain_text: "Day 1: Down the rabbit hole we go".to_string(),
                        href: None,
                        annotations: Default::default(),
                        ty: RichTextType::Text {
                            content: "Day 1: Down the rabbit hole we go".to_string(),
                            link: None,
                        },
                    }],
                },
            },
            parent: PageParent::Workspace,
            url: "https://www.notion.so/ac3fb543001f4be5a25e4978abd05b1d".to_string(),
            children: vec![Block {
                object: "block".to_string(),
                id: "8cac60c2-74b9-408c-acbd-0895cfd7b7f8".parse().unwrap(),
                created_time: "2021-11-13T17:35:00.000Z".to_string(),
                last_edited_time: "2021-11-13T19:02:00.000Z".to_string(),
                has_children: false,
                archived: false,
                ty: BlockType::HeadingOne {
                    text: vec![RichText {
                        plain_text: "Cool test".to_string(),
                        href: None,
                        annotations: Default::default(),
                        ty: RichTextType::Text {
                            content: "Cool test".to_string(),
                            link: None,
                        },
                    }],
                },
            }],
        };

        let markup = renderer
            .render_page(&page)
            .map(|markup| markup.into_string())
            .unwrap();

        assert_eq!(
            markup,
            r#"<h1 id="ac3fb543001f4be5a25e4978abd05b1d">Day 1: Down the rabbit hole we go</h1><h2 id="8cac60c274b9408cacbd0895cfd7b7f8">Cool test</h2>"#
        );
        let guard = renderer.downloadables.set.guard();
        assert_eq!(
            renderer
                .downloadables
                .set
                .iter(&guard)
                .collect::<HashSet<&Downloadable>>(),
            HashSet::new()
        );
    }
}
