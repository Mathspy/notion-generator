use either::Either;
use itertools::Itertools;
use serde::{Deserialize, Serialize};
use std::{fmt, str::FromStr};
use time::{Date, OffsetDateTime};
use uuid::Uuid;

#[derive(Debug, Deserialize, Clone, Copy, Eq, PartialEq, Hash)]
#[repr(transparent)]
#[serde(transparent)]
pub struct NotionId(Uuid);

impl FromStr for NotionId {
    type Err = uuid::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(NotionId(Uuid::parse_str(s)?))
    }
}
impl fmt::Display for NotionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.to_simple_ref().fmt(f)
    }
}

// ------------------ NOTION ERROR OBJECT ------------------
// As defined in https://developers.notion.com/reference/errors
#[derive(Debug, Deserialize, PartialEq, Eq)]
pub struct Error {
    pub code: ErrorCode,
    pub message: String,
}

#[derive(Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all(deserialize = "snake_case", serialize = "SCREAMING_SNAKE_CASE"))]
pub enum ErrorCode {
    InvalidJson,
    InvalidRequestUrl,
    InvalidRequest,
    ValidationError,
    MissingVersion,
    Unauthorized,
    RestrictedResource,
    ObjectNotFound,
    ConflictError,
    RateLimited,
    InternalServerError,
    ServiceUnavailable,
    DatabaseConnectionUnavailable,
    #[serde(other)]
    Unknown,
}

// ------------------ NOTION LIST OBJECT ------------------
// As defined in https://developers.notion.com/reference/pagination
#[derive(Debug, Deserialize, PartialEq, Eq)]
pub struct List<T> {
    // TODO: assert!(list.object == "list");
    pub object: String,
    pub results: Vec<T>,
    pub next_cursor: Option<String>,
    pub has_more: bool,
}

// ------------------ NOTION RICH TEXT OBJECT ------------------
// As defined in https://developers.notion.com/reference/rich-text
#[derive(Debug, Deserialize, PartialEq, Eq)]
pub struct RichText {
    pub plain_text: String,
    pub href: Option<String>,
    pub annotations: Annotations,
    #[serde(flatten)]
    pub ty: RichTextType,
}

pub trait PlainText {
    fn plain_text(&self) -> String;
}

impl PlainText for &[RichText] {
    fn plain_text(&self) -> String {
        self.iter().map(|rich_text| &rich_text.plain_text).join("")
    }
}

impl PlainText for Vec<RichText> {
    fn plain_text(&self) -> String {
        self.iter().map(|rich_text| &rich_text.plain_text).join("")
    }
}

impl PlainText for Vec<&RichText> {
    fn plain_text(&self) -> String {
        self.iter().map(|rich_text| &rich_text.plain_text).join("")
    }
}

mod deserializers {
    use super::{File, PageParent, RichTextLink, Time};
    use serde::{
        de::{Error, Unexpected},
        Deserialize, Deserializer,
    };
    use std::borrow::Cow;

    fn time_inner<'a, D: Deserializer<'a>>(input: &'a str) -> Result<Time, D::Error> {
        input.parse().map_err(|_| {
            D::Error::custom("data matched neither a date (YYYY-MM-DD) nor a datetime (RFC3339)")
        })
    }

    pub fn time<'a, D: Deserializer<'a>>(deserializer: D) -> Result<Time, D::Error> {
        time_inner::<D>(<_>::deserialize(deserializer)?)
    }

    pub fn optional_time<'a, D: Deserializer<'a>>(
        deserializer: D,
    ) -> Result<Option<Time>, D::Error> {
        Option::deserialize(deserializer)?
            .map(time_inner::<D>)
            .transpose()
    }

    pub fn optional_rich_text_link<'a, D: Deserializer<'a>>(
        deserializer: D,
    ) -> Result<Option<RichTextLink>, D::Error> {
        // We use Cow below because MOST of the time the string we are passed can be reused
        // and we only have to allocate once at the end when returning RichTextLink BUT sometimes
        // we will be passed strings that has escape sequences AKA "\"love\"" and to be able to
        // return `"love"` without the backslashes Serde needs to allocate! Thus Cow to the rescue
        #[derive(Deserialize)]
        struct Link<'a> {
            // TODO(NOTION): Notion docs say type: "url" should be returned but it's not
            // type: "url",
            url: Cow<'a, str>,
        }

        let link = Option::<Link>::deserialize(deserializer)?;
        let link = match link {
            Some(link) => link,
            None => return Ok(None),
        };

        if link.url.starts_with('/') {
            let mut parts = link.url[1..].split('#');

            let page = match parts.next() {
                Some(page) => page,
                None => {
                    return Err(D::Error::invalid_value(
                        Unexpected::Str(&link.url),
                        &"a valid internal Notion url",
                    ))
                }
            };

            Ok(Some(RichTextLink::Internal {
                page: page.parse().map_err(|_| {
                    D::Error::invalid_value(Unexpected::Str(page), &"a Notion page UUID")
                })?,
                block: parts.next().map(str::to_string),
            }))
        } else {
            Ok(Some(RichTextLink::External {
                url: link.url.to_string(),
            }))
        }
    }

    pub fn page_parent<'a, D: Deserializer<'a>>(deserializer: D) -> Result<PageParent, D::Error> {
        #[derive(Deserialize)]
        struct Parent<'a> {
            #[serde(rename = "type")]
            ty: &'a str,
            database_id: Option<String>,
            page_id: Option<String>,
            block_id: Option<String>,
            workspace: Option<bool>,
        }

        let parent = Parent::deserialize(deserializer)?;
        match parent.ty {
            "database_id" => {
                if let Some(id) = parent.database_id {
                    Ok(PageParent::Database { id })
                } else {
                    Err(D::Error::missing_field("database_id"))
                }
            }
            "page_id" => {
                if let Some(id) = parent.page_id {
                    Ok(PageParent::Page { id })
                } else {
                    Err(D::Error::missing_field("page_id"))
                }
            }
            "workspace" => {
                if let Some(should_be_true) = parent.workspace {
                    if !should_be_true {
                        Err(D::Error::invalid_value(
                            Unexpected::Bool(false),
                            &"expected workspace to be `true` since parent type workspace",
                        ))
                    } else {
                        Ok(PageParent::Workspace)
                    }
                } else {
                    Err(D::Error::missing_field("workspace"))
                }
            }
            "block_id" => {
                if let Some(id) = parent.block_id {
                    Ok(PageParent::Block {
                        id: id.parse().map_err(|_| {
                            D::Error::invalid_value(
                                Unexpected::Str(&id),
                                &"expected `block_id` to be a valid notion id since parent type is block_id",
                            )
                        })?,
                    })
                } else {
                    Err(D::Error::missing_field("database_id"))
                }
            }
            ty => Err(D::Error::invalid_value(
                Unexpected::Str(ty),
                &r#"expected `type` to be one of "database_id", "page_id", "workspace""#,
            )),
        }
    }

    pub fn optional_file<'a, D: Deserializer<'a>>(
        deserializer: D,
    ) -> Result<Option<File>, D::Error> {
        #[derive(Deserialize)]
        struct RawFile {
            #[serde(flatten)]
            file: File,
        }

        <Option<RawFile>>::deserialize(deserializer)
            .map(|option| option.map(|RawFile { file }| file))
    }
}

#[derive(Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RichTextType {
    Text {
        content: String,
        #[serde(deserialize_with = "deserializers::optional_rich_text_link")]
        link: Option<RichTextLink>,
    },
    Equation {
        expression: String,
    },
    Mention {
        #[serde(flatten)]
        mention: RichTextMentionType,
    },
}

#[derive(Debug, Deserialize, PartialEq, Eq)]
pub enum RichTextLink {
    Internal {
        page: NotionId,
        block: Option<String>,
    },
    External {
        url: String,
    },
}

// TODO: The original and parsed shouldn't really be pub and instead should use getter methods to
// ensure they stay in sync and can't be changed in an invalid way
#[derive(Debug, Eq, Clone)]
pub struct Time {
    // We keep the original to avoid needing to recreate it if we need an ISO 8601 formatted
    // date(time) later
    pub original: String,
    pub parsed: Either<Date, OffsetDateTime>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InvalidTime;

impl std::fmt::Display for InvalidTime {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("data matched neither a date (YYYY-MM-DD) nor a datetime (RFC3339)")?;

        Ok(())
    }
}

impl FromStr for Time {
    type Err = InvalidTime;

    fn from_str(input: &str) -> Result<Self, Self::Err> {
        use time::{
            format_description::{well_known::Rfc3339, FormatItem},
            macros::format_description,
        };

        const DATE_FORMAT: &[FormatItem<'_>] = format_description!("[year]-[month]-[day]");

        if let Ok(date) = Date::parse(input, DATE_FORMAT) {
            return Ok(Time {
                original: input.to_owned(),
                parsed: Either::Left(date),
            });
        }

        if let Ok(datetime) = OffsetDateTime::parse(input, &Rfc3339) {
            return Ok(Time {
                original: input.to_owned(),
                parsed: Either::Right(datetime),
            });
        }

        Err(InvalidTime)
    }
}

impl PartialEq<Date> for Time {
    fn eq(&self, other: &Date) -> bool {
        match self.parsed {
            Either::Left(date) => date.eq(other),
            Either::Right(datetime) => datetime.date().eq(other),
        }
    }
}

impl PartialOrd<Date> for Time {
    fn partial_cmp(&self, other: &Date) -> Option<std::cmp::Ordering> {
        match self.parsed {
            Either::Left(date) => date.partial_cmp(other),
            Either::Right(datetime) => datetime.date().partial_cmp(other),
        }
    }
}

impl PartialEq<Time> for Time {
    fn eq(&self, other: &Time) -> bool {
        self.parsed == other.parsed
    }
}

impl PartialOrd<Time> for Time {
    fn partial_cmp(&self, other: &Time) -> Option<std::cmp::Ordering> {
        let this = match self.parsed {
            Either::Left(date) => date.with_time(time::Time::MIDNIGHT).assume_utc(),
            Either::Right(datetime) => datetime,
        };

        let other = match other.parsed {
            Either::Left(date) => date.with_time(time::Time::MIDNIGHT).assume_utc(),
            Either::Right(datetime) => datetime,
        };

        this.partial_cmp(&other)
    }
}

impl Ord for Time {
    fn cmp(&self, other: &Time) -> std::cmp::Ordering {
        let this = match self.parsed {
            Either::Left(date) => date.with_time(time::Time::MIDNIGHT).assume_utc(),
            Either::Right(datetime) => datetime,
        };

        let other = match other.parsed {
            Either::Left(date) => date.with_time(time::Time::MIDNIGHT).assume_utc(),
            Either::Right(datetime) => datetime,
        };

        this.cmp(&other)
    }
}

#[derive(Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RichTextMentionType {
    User {
        // TODO: assert!(object == "user");
        id: NotionId,
        name: String,
        avatar_url: String,
        #[serde(flatten)]
        ty: UserType,
    },
    Page {
        id: NotionId,
    },
    Database {
        id: NotionId,
    },
    Date(NotionDate),
    // TODO(NOTION): link_preview has absolutely no documentation
}

#[derive(Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum UserType {
    Person { email: String },
    // TODO: Add UserType::Bot
    // Bot
}

#[derive(Debug, Deserialize, PartialEq, Eq)]
pub struct NotionDate {
    #[serde(deserialize_with = "deserializers::time")]
    pub start: Time,
    #[serde(deserialize_with = "deserializers::optional_time")]
    pub end: Option<Time>,
    // TODO(NOTION): This just got added to the API recently but it seems to never be returned
    // as a response, it's possible to set it if you're using the API to add/modify Notion though
    // Would be really useful if it was there
    // It's also tied to both `start` and `end` in a whacky way, it's never specified if there's
    // no time component, we might want to encode that somehow in the type system
    pub time_zone: Option<String>,
}

#[derive(Debug, Default, Deserialize, PartialEq, Eq)]
pub struct Annotations {
    pub bold: bool,
    pub italic: bool,
    pub strikethrough: bool,
    pub underline: bool,
    pub code: bool,
    pub color: Color,
}

#[derive(Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Color {
    Default,
    Gray,
    Brown,
    Orange,
    Yellow,
    Green,
    Blue,
    Purple,
    Pink,
    Red,
    GrayBackground,
    BrownBackground,
    OrangeBackground,
    YellowBackground,
    GreenBackground,
    BlueBackground,
    PurpleBackground,
    PinkBackground,
    RedBackground,
}

impl Default for Color {
    fn default() -> Self {
        Color::Default
    }
}

// ------------------ NOTION PAGE OBJECT -------------------
// As defined in https://developers.notion.com/reference/page
#[derive(Debug, Deserialize, PartialEq, Eq)]
pub struct Page<P> {
    // TODO: assert!(page.object == "page");
    pub object: String,
    pub id: NotionId,
    pub created_time: String,
    pub last_edited_time: String,
    pub archived: bool,
    pub icon: Option<EmojiOrFile>,
    #[serde(deserialize_with = "deserializers::optional_file")]
    pub cover: Option<File>,
    pub properties: P,
    #[serde(deserialize_with = "deserializers::page_parent")]
    pub parent: PageParent,
    pub url: String,
    /// All the blocks inside of the page
    ///
    /// This isn't technically part of the Notion spec but it makes sense here so!
    #[serde(default)]
    pub children: Vec<Block>,
}

impl<P> Page<P> {
    pub fn replace_children(self, updated_children: Vec<Block>) -> Self {
        Self {
            children: updated_children,
            ..self
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum PageParent {
    Database { id: String },
    Page { id: String },
    Workspace,
    Block { id: NotionId },
}

pub mod properties {
    use super::{NotionDate, RichText};
    use serde::Deserialize;

    #[derive(Debug, Deserialize, PartialEq, Eq)]
    pub struct TitleProperty {
        pub id: String,
        pub title: Vec<RichText>,
    }

    #[derive(Debug, Deserialize, PartialEq, Eq)]
    pub struct RichTextProperty {
        pub id: String,
        pub rich_text: Vec<RichText>,
    }

    #[derive(Debug, Deserialize, PartialEq)]
    pub struct NumberProperty {
        pub id: String,
        pub number: Option<f64>,
    }

    #[derive(Debug, Deserialize, PartialEq, Eq)]
    pub struct DateProperty {
        pub id: String,
        pub date: Option<NotionDate>,
    }

    // TODO: Rest of properties:
    // - select
    // - multi_select
    // - formula
    // - relation
    // - rollup
    // - people
    // - files
    // - checkbox
    // - url
    // - email
    // - phone_number
    // - created_time
    // - created_by
    // - last_edited_time
    // - last_edited_by
}

// ------------------ NOTION BLOCK OBJECT ------------------
// As defined in https://developers.notion.com/reference/block
#[derive(Debug, Deserialize, PartialEq, Eq)]
pub struct Block {
    // TODO: assert!(list.object == "list");
    pub object: String,
    pub id: NotionId,
    pub created_time: String,
    pub last_edited_time: String,
    pub has_children: bool,
    pub archived: bool,
    #[serde(flatten)]
    pub ty: BlockType,
}

impl Block {
    pub fn replace_children(self, updated_children: Vec<Block>) -> Block {
        if !self.has_children {
            return self;
        }

        match self.ty {
            BlockType::Paragraph { text, .. } => Block {
                ty: BlockType::Paragraph {
                    text,
                    children: updated_children,
                },
                ..self
            },
            BlockType::Callout { text, icon, .. } => Block {
                ty: BlockType::Callout {
                    text,
                    icon,
                    children: updated_children,
                },
                ..self
            },
            BlockType::Quote { text, .. } => Block {
                ty: BlockType::Quote {
                    text,
                    children: updated_children,
                },
                ..self
            },
            BlockType::BulletedListItem { text, .. } => Block {
                ty: BlockType::BulletedListItem {
                    text,
                    children: updated_children,
                },
                ..self
            },
            BlockType::NumberedListItem { text, .. } => Block {
                ty: BlockType::NumberedListItem {
                    text,
                    children: updated_children,
                },
                ..self
            },
            BlockType::ToDo { checked, text, .. } => Block {
                ty: BlockType::ToDo {
                    checked,
                    text,
                    children: updated_children,
                },
                ..self
            },
            ty => Block { ty, ..self },
        }
    }

    pub fn name(&self) -> &'static str {
        match self.ty {
            BlockType::Paragraph { .. } => "paragraph",
            BlockType::HeadingOne { .. } => "heading_1",
            BlockType::HeadingTwo { .. } => "heading_2",
            BlockType::HeadingThree { .. } => "heading_3",
            BlockType::Callout { .. } => "callout",
            BlockType::Quote { .. } => "quote",
            BlockType::BulletedListItem { .. } => "bulleted_list_item",
            BlockType::NumberedListItem { .. } => "numbered_list_item",
            BlockType::ToDo { .. } => "todo",
            BlockType::Code { .. } => "code",
            BlockType::Image { .. } => "image",
            BlockType::Video { .. } => "video",
            BlockType::Divider { .. } => "divider",
            BlockType::TableOfContents { .. } => "table_of_contents",
        }
    }

    pub fn get_text(&self) -> Option<&[RichText]> {
        match &self.ty {
            BlockType::Paragraph { text, .. } => Some(text),
            BlockType::HeadingOne { text, .. } => Some(text),
            BlockType::HeadingTwo { text, .. } => Some(text),
            BlockType::HeadingThree { text, .. } => Some(text),
            BlockType::Callout { text, .. } => Some(text),
            BlockType::Quote { text, .. } => Some(text),
            BlockType::BulletedListItem { text, .. } => Some(text),
            BlockType::NumberedListItem { text, .. } => Some(text),
            BlockType::ToDo { text, .. } => Some(text),
            BlockType::Code { text, .. } => Some(text),
            _ => None,
        }
    }

    pub fn get_children(&self) -> Option<&[Block]> {
        match &self.ty {
            BlockType::Paragraph { children, .. } => Some(children),
            BlockType::Callout { children, .. } => Some(children),
            BlockType::Quote { children, .. } => Some(children),
            BlockType::BulletedListItem { children, .. } => Some(children),
            BlockType::NumberedListItem { children, .. } => Some(children),
            BlockType::ToDo { children, .. } => Some(children),
            _ => None,
        }
    }

    pub fn list_type(&self) -> Option<ListType> {
        match self.ty {
            BlockType::BulletedListItem { .. } => Some(ListType::Bulleted),
            BlockType::NumberedListItem { .. } => Some(ListType::Numbered),
            BlockType::ToDo { .. } => Some(ListType::ToDo),
            _ => None,
        }
    }
}

// Not a Notion type but a useful utility type
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ListType {
    Bulleted,
    Numbered,
    ToDo,
}

// TODO: This only supports the types I think I will need for now
#[derive(Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BlockType {
    Paragraph {
        #[serde(rename = "rich_text")]
        text: Vec<RichText>,
        #[serde(default)]
        children: Vec<Block>,
    },
    #[serde(rename = "heading_1")]
    HeadingOne {
        #[serde(rename = "rich_text")]
        text: Vec<RichText>,
    },
    #[serde(rename = "heading_2")]
    HeadingTwo {
        #[serde(rename = "rich_text")]
        text: Vec<RichText>,
    },
    #[serde(rename = "heading_3")]
    HeadingThree {
        #[serde(rename = "rich_text")]
        text: Vec<RichText>,
    },
    Callout {
        #[serde(rename = "rich_text")]
        text: Vec<RichText>,
        icon: EmojiOrFile,
        #[serde(default)]
        children: Vec<Block>,
    },
    Quote {
        #[serde(rename = "rich_text")]
        text: Vec<RichText>,
        #[serde(default)]
        children: Vec<Block>,
    },
    BulletedListItem {
        #[serde(rename = "rich_text")]
        text: Vec<RichText>,
        #[serde(default)]
        children: Vec<Block>,
    },
    NumberedListItem {
        #[serde(rename = "rich_text")]
        text: Vec<RichText>,
        #[serde(default)]
        children: Vec<Block>,
    },
    ToDo {
        checked: bool,
        #[serde(rename = "rich_text")]
        text: Vec<RichText>,
        #[serde(default)]
        children: Vec<Block>,
    },
    // Toggle
    Code {
        language: Language,
        #[serde(rename = "rich_text")]
        text: Vec<RichText>,
        // TODO(NOTION): Notion docs say text should be a string but it's a rich text instead
        // text: String,
    },
    // ChildPage
    // ChildDatabase
    // Embed
    Image {
        #[serde(flatten)]
        image: File,
        caption: Vec<RichText>,
    },
    Video {
        #[serde(flatten)]
        video: File,
        caption: Vec<RichText>,
    },
    // PDF
    // Bookmark
    // Equation
    Divider {},
    TableOfContents {},
    // Breadcrumb
    // ColumnList
    // Column
    // LinkPreview
    // Unsupported
}

#[derive(Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all(deserialize = "lowercase", serialize = "snake_case"))]
pub enum Language {
    Abap,
    Arduino,
    Bash,
    Basic,
    C,
    Clojure,
    CoffeeScript,
    #[serde(rename(deserialize = "c++"))]
    CPlusPlus,
    #[serde(rename(deserialize = "c#"))]
    CSharp,
    Css,
    Dart,
    Diff,
    Docker,
    Elixir,
    Elm,
    Erlang,
    Flow,
    Fortran,
    #[serde(rename(deserialize = "f#"))]
    FSharp,
    Gherkin,
    Glsl,
    Go,
    GraphQL,
    Groovy,
    Haskell,
    Html,
    Java,
    // TODO(NOTION): It says `javaSsript` in the docs but it sends back `javascript`
    JavaScript,
    Json,
    Julia,
    Kotlin,
    Latex,
    Less,
    Lisp,
    LiveScript,
    Lua,
    Makefile,
    Markdown,
    Markup,
    Matlab,
    Mermaid,
    Nix,
    #[serde(rename(deserialize = "objective-c"))]
    ObjectiveC,
    Ocaml,
    Pascal,
    Perl,
    Php,
    #[serde(rename(deserialize = "plain text"))]
    PlainText,
    Powershell,
    Prolog,
    Protobuf,
    Python,
    R,
    Reason,
    Ruby,
    Rust,
    Sass,
    Scala,
    Scheme,
    Scss,
    Shell,
    Sql,
    Swift,
    TypeScript,
    #[serde(rename(deserialize = "vb.net"))]
    VbNet,
    Verilog,
    Vhdl,
    #[serde(rename(deserialize = "visual basic"))]
    VisualBasic,
    WebAssembly,
    Xml,
    Yaml,
    #[serde(rename(deserialize = "java/c/c++/c#"))]
    CLike,
}

// // ------------------ NOTION EMOJI OBJECT ------------------
// // As defined in https://developers.notion.com/reference/emoji-object
#[derive(Debug, Deserialize, PartialEq, Eq)]
pub struct Emoji {
    pub emoji: String,
}

// // ------------------ NOTION EMOJI OBJECT ------------------
// // As defined in https://developers.notion.com/reference/file-object
#[derive(Debug, Deserialize, PartialEq, Eq)]
pub enum File {
    #[serde(rename = "file")]
    Internal { url: String, expiry_time: String },
    #[serde(rename = "external")]
    External { url: String },
}

impl File {
    pub fn as_downloadable(
        &self,
        block_id: NotionId,
    ) -> Result<crate::download::Downloadable, anyhow::Error> {
        use crate::download::{Downloadable, FILES_DIR};
        use anyhow::Context;
        use reqwest::Url;
        use std::path::{Path, PathBuf};

        let url = match self {
            File::Internal { url, .. } => url,
            File::External { url } => url,
        };

        let parsed_url = Url::parse(url).context("Failed to parse image URL")?;
        let ext = parsed_url
            .path_segments()
            .and_then(|segments| segments.last().map(Path::new).and_then(Path::extension));
        // A path is the media directory + UUID + ext
        // i.e media/eb39a20e10364469b750a9df8f4f18df.png
        let block_id = block_id.to_string();
        let mut path = PathBuf::with_capacity(
            FILES_DIR.len() + block_id.len() + ext.map(|ext| ext.len()).unwrap_or(0),
        );
        path.push(FILES_DIR);
        path.push(block_id);
        if let Some(ext) = ext {
            path.set_extension(ext);
        };

        Downloadable::new(parsed_url, path)
    }
}

#[derive(Debug, Deserialize, PartialEq, Eq)]
#[serde(tag = "type")]
pub enum EmojiOrFile {
    #[serde(rename = "file", alias = "external")]
    File(File),
    #[serde(rename = "emoji")]
    Emoji(Emoji),
}

// TODO(NOTION): The field `caption` is missing from Notion docs but available in responses for
// internal files (type == "file")
// {
//   "type": "file",
//   "file": {
//     "url": string,
//     "expiry_time": string,
//   },
//   "caption": Array<string>,
// };

#[cfg(test)]
mod tests {
    use super::{
        properties::{DateProperty, RichTextProperty},
        Block, BlockType, Emoji, EmojiOrFile, Error, ErrorCode, File, Language, List, NotionDate,
        Page, PageParent, RichText, RichTextLink, RichTextMentionType, RichTextType, Time,
        UserType,
    };
    use either::Either;
    use pretty_assertions::assert_eq;
    use serde::Deserialize;
    use time::macros::{date, datetime};

    #[test]
    fn test_errors() {
        let json: &str = r#"
          {
            "code": "invalid_json",
            "message": "Oh no the JSON you sent is invalid :<"
          }
        "#;

        assert_eq!(
            serde_json::from_str::<Error>(json).unwrap(),
            Error {
                code: ErrorCode::InvalidJson,
                message: "Oh no the JSON you sent is invalid :<".to_string()
            }
        );

        let json: &str = r#""invalid_request_url""#;
        assert_eq!(
            serde_json::from_str::<ErrorCode>(json).unwrap(),
            ErrorCode::InvalidRequestUrl
        );

        let json: &str = r#""invalid_request""#;
        assert_eq!(
            serde_json::from_str::<ErrorCode>(json).unwrap(),
            ErrorCode::InvalidRequest
        );

        let json: &str = r#""validation_error""#;
        assert_eq!(
            serde_json::from_str::<ErrorCode>(json).unwrap(),
            ErrorCode::ValidationError
        );

        let json: &str = r#""missing_version""#;
        assert_eq!(
            serde_json::from_str::<ErrorCode>(json).unwrap(),
            ErrorCode::MissingVersion
        );

        let json: &str = r#""unauthorized""#;
        assert_eq!(
            serde_json::from_str::<ErrorCode>(json).unwrap(),
            ErrorCode::Unauthorized
        );

        let json: &str = r#""restricted_resource""#;
        assert_eq!(
            serde_json::from_str::<ErrorCode>(json).unwrap(),
            ErrorCode::RestrictedResource
        );

        let json: &str = r#""object_not_found""#;
        assert_eq!(
            serde_json::from_str::<ErrorCode>(json).unwrap(),
            ErrorCode::ObjectNotFound
        );

        let json: &str = r#""conflict_error""#;
        assert_eq!(
            serde_json::from_str::<ErrorCode>(json).unwrap(),
            ErrorCode::ConflictError
        );

        let json: &str = r#""rate_limited""#;
        assert_eq!(
            serde_json::from_str::<ErrorCode>(json).unwrap(),
            ErrorCode::RateLimited
        );

        let json: &str = r#""internal_server_error""#;
        assert_eq!(
            serde_json::from_str::<ErrorCode>(json).unwrap(),
            ErrorCode::InternalServerError
        );

        let json: &str = r#""service_unavailable""#;
        assert_eq!(
            serde_json::from_str::<ErrorCode>(json).unwrap(),
            ErrorCode::ServiceUnavailable
        );

        let json: &str = r#""database_connection_unavailable""#;
        assert_eq!(
            serde_json::from_str::<ErrorCode>(json).unwrap(),
            ErrorCode::DatabaseConnectionUnavailable
        );

        let json: &str = r#""some_new_error_notion_just_added""#;
        assert_eq!(
            serde_json::from_str::<ErrorCode>(json).unwrap(),
            ErrorCode::Unknown
        );
    }

    #[test]
    fn test_rich_text_links() {
        let json = r#"
            {
              "type": "text",
              "text": {
                "content": "A cool website",
                "link": {
                  "url": "https://mathspy.me/"
                }
              },
              "annotations": {
                "bold": false,
                "italic": false,
                "strikethrough": false,
                "underline": false,
                "code": false,
                "color": "default"
              },
              "plain_text": "A cool website",
              "href": "https://mathspy.me/"
            }
        "#;

        assert_eq!(
            serde_json::from_str::<RichText>(json).unwrap(),
            RichText {
                plain_text: "A cool website".to_string(),
                href: Some("https://mathspy.me/".to_string()),
                annotations: Default::default(),
                ty: RichTextType::Text {
                    content: "A cool website".to_string(),
                    link: Some(RichTextLink::External {
                        url: "https://mathspy.me/".to_string()
                    })
                },
            }
        );

        let json = r#"
            {
            "type": "text",
            "text": {
                "content": "Test characters that need escaping",
                "link": {
                "url": "http://mathspy.me/\"hello\"/"
                }
            },
            "annotations": {
                "bold": false,
                "italic": false,
                "strikethrough": false,
                "underline": false,
                "code": false,
                "color": "default"
            },
            "plain_text": "Test characters that need escaping",
            "href": "http://mathspy.me/\"hello\"/"
            }
        "#;

        assert_eq!(
            serde_json::from_str::<RichText>(json).unwrap(),
            RichText {
                plain_text: "Test characters that need escaping".to_string(),
                href: Some("http://mathspy.me/\"hello\"/".to_string()),
                annotations: Default::default(),
                ty: RichTextType::Text {
                    content: "Test characters that need escaping".to_string(),
                    link: Some(RichTextLink::External {
                        url: "http://mathspy.me/\"hello\"/".to_string()
                    })
                },
            }
        );

        let json = r#"
            {
              "type": "text",
              "text": {
                "content": "¹",
                "link": {
                  "url": "/46f8638c25a84ccd9d926e42bdb5535e#48cb69650f584e60be8159e9f8e07a8a"
                }
              },
              "annotations": {
                "bold": false,
                "italic": false,
                "strikethrough": false,
                "underline": false,
                "code": false,
                "color": "default"
              },
              "plain_text": "¹",
              "href": "/46f8638c25a84ccd9d926e42bdb5535e#48cb69650f584e60be8159e9f8e07a8a"
            }
        "#;

        assert_eq!(
            serde_json::from_str::<RichText>(json).unwrap(),
            RichText {
                plain_text: "¹".to_string(),
                href: Some(
                    "/46f8638c25a84ccd9d926e42bdb5535e#48cb69650f584e60be8159e9f8e07a8a"
                        .to_string()
                ),
                annotations: Default::default(),
                ty: RichTextType::Text {
                    content: "¹".to_string(),
                    link: Some(RichTextLink::Internal {
                        page: "46f8638c25a84ccd9d926e42bdb5535e".parse().unwrap(),
                        block: Some("48cb69650f584e60be8159e9f8e07a8a".to_string()),
                    })
                },
            }
        );

        let json = r#"
            {
              "type": "text",
              "text": {
                "content": "A less watered down test",
                "link": {
                  "url": "/46f8638c25a84ccd9d926e42bdb5535e"
                }
              },
              "annotations": {
                "bold": false,
                "italic": false,
                "strikethrough": false,
                "underline": false,
                "code": false,
                "color": "default"
              },
              "plain_text": "A less watered down test",
              "href": "/46f8638c25a84ccd9d926e42bdb5535e"
            }
        "#;

        assert_eq!(
            serde_json::from_str::<RichText>(json).unwrap(),
            RichText {
                plain_text: "A less watered down test".to_string(),
                href: Some("/46f8638c25a84ccd9d926e42bdb5535e".to_string()),
                annotations: Default::default(),
                ty: RichTextType::Text {
                    content: "A less watered down test".to_string(),
                    link: Some(RichTextLink::Internal {
                        page: "46f8638c25a84ccd9d926e42bdb5535e".parse().unwrap(),
                        block: None,
                    })
                },
            }
        );
    }

    #[test]
    fn test_rich_text_mentions() {
        let json = r#"
            {
              "type": "mention",
              "mention": {
                "type": "date",
                "date": {
                  "start": "2021-11-07T02:59:00.000-08:00",
                  "end": null
                }
              },
              "annotations": {
                "bold": false,
                "italic": false,
                "strikethrough": false,
                "underline": false,
                "code": false,
                "color": "default"
              },
              "plain_text": "2021-11-07T02:59:00.000-08:00 → ",
              "href": null
            }
        "#;

        assert_eq!(
            serde_json::from_str::<RichText>(json).unwrap(),
            RichText {
                plain_text: "2021-11-07T02:59:00.000-08:00 → ".to_string(),
                href: None,
                annotations: Default::default(),
                ty: RichTextType::Mention {
                    mention: RichTextMentionType::Date(NotionDate {
                        start: Time {
                            original: "2021-11-07T02:59:00.000-08:00".to_string(),
                            parsed: Either::Right(datetime!(2021-11-07 10:59 UTC))
                        },
                        end: None,
                        time_zone: None,
                    }),
                },
            }
        );

        let json = r#"
            {
              "type": "mention",
              "mention": {
                "type": "date",
                "date": {
                  "start": "2021-12-05",
                  "end": "2021-12-06"
                }
              },
              "annotations": {
                "bold": false,
                "italic": false,
                "strikethrough": false,
                "underline": false,
                "code": false,
                "color": "default"
              },
              "plain_text": "2021-12-05 → 2021-12-06",
              "href": null
            }
        "#;

        assert_eq!(
            serde_json::from_str::<RichText>(json).unwrap(),
            RichText {
                plain_text: "2021-12-05 → 2021-12-06".to_string(),
                href: None,
                annotations: Default::default(),
                ty: RichTextType::Mention {
                    mention: RichTextMentionType::Date(NotionDate {
                        start: Time {
                            original: "2021-12-05".to_string(),
                            parsed: Either::Left(date!(2021 - 12 - 05))
                        },
                        end: Some(Time {
                            original: "2021-12-06".to_string(),
                            parsed: Either::Left(date!(2021 - 12 - 06))
                        }),
                        time_zone: None,
                    }),
                },
            }
        );

        let json = r#"
            {
              "type": "mention",
              "mention": {
                "type": "user",
                "user": {
                  "object": "user",
                  "id": "8cac60c2-74b9-408c-acbd-08fd7f8b795c",
                  "name": "Mathy",
                  "avatar_url": "https://mathspy.me/mathy.png",
                  "type": "person",
                  "person": {
                    "email": "spam@example.com"
                  }
                }
              },
              "annotations": {
                "bold": false,
                "italic": false,
                "strikethrough": false,
                "underline": false,
                "code": false,
                "color": "default"
              },
              "plain_text": "@Mathy",
              "href": null
            }
        "#;

        assert_eq!(
            serde_json::from_str::<RichText>(json).unwrap(),
            RichText {
                plain_text: "@Mathy".to_string(),
                href: None,
                annotations: Default::default(),
                ty: RichTextType::Mention {
                    mention: RichTextMentionType::User {
                        id: "8cac60c274b9408cacbd08fd7f8b795c".parse().unwrap(),
                        avatar_url: "https://mathspy.me/mathy.png".to_string(),
                        name: "Mathy".to_string(),
                        ty: UserType::Person {
                            email: "spam@example.com".to_string(),
                        }
                    },
                },
            }
        );

        let json = r#"
                        {
              "type": "mention",
              "mention": {
                "type": "page",
                "page": {
                  "id": "6e0eb85f-6047-4efb-a130-4f92d2abfa2c"
                }
              },
              "annotations": {
                "bold": false,
                "italic": false,
                "strikethrough": false,
                "underline": false,
                "code": false,
                "color": "default"
              },
              "plain_text": "watereddown-test",
              "href": "https://www.notion.so/6e0eb85f60474efba1304f92d2abfa2c"
            }
        "#;

        assert_eq!(
            serde_json::from_str::<RichText>(json).unwrap(),
            RichText {
                plain_text: "watereddown-test".to_string(),
                href: Some("https://www.notion.so/6e0eb85f60474efba1304f92d2abfa2c".to_string()),
                annotations: Default::default(),
                ty: RichTextType::Mention {
                    mention: RichTextMentionType::Page {
                        id: "6e0eb85f60474efba1304f92d2abfa2c".parse().unwrap(),
                    },
                },
            }
        );

        let json = r#"
            {
              "type": "mention",
              "mention": {
                "type": "database",
                "database": {
                  "id": "332b7b05-2ded-4955-bc77-60851242836a"
                }
              },
              "annotations": {
                "bold": false,
                "italic": false,
                "strikethrough": false,
                "underline": false,
                "code": false,
                "color": "default"
              },
              "plain_text": "Some whacky database",
              "href": "https://www.notion.so/332b7b052ded4955bc7760851242836a"
            }
        "#;

        assert_eq!(
            serde_json::from_str::<RichText>(json).unwrap(),
            RichText {
                plain_text: "Some whacky database".to_string(),
                href: Some("https://www.notion.so/332b7b052ded4955bc7760851242836a".to_string()),
                annotations: Default::default(),
                ty: RichTextType::Mention {
                    mention: RichTextMentionType::Database {
                        id: "332b7b052ded4955bc7760851242836a".parse().unwrap(),
                    },
                },
            }
        );
    }

    #[test]
    fn test_pages() {
        let json = r#"
            {
              "object": "page",
              "id": "ac3fb543-001f-4be5-a25e-4978abd05b1d",
              "created_time": "2021-11-29T18:20:00.000Z",
              "last_edited_time": "2021-12-06T09:25:00.000Z",
              "cover": {
                "type": "file",
                "file": {
                  "url": "https://s3.us-west-2.amazonaws.com/secure.notion-static.com/2044fff8-de9e-418b-a7c5-6f0075c0564f/entity_component_system_better.png?X-Amz-Algorithm=AWS4-HMAC-SHA256&X-Amz-Content-Sha256=UNSIGNED-PAYLOAD&X-Amz-Credential=AKIAT73L2G45EIPT3X45%2F20211213%2Fus-west-2%2Fs3%2Faws4_request&X-Amz-Date=20211213T134329Z&X-Amz-Expires=3600&X-Amz-Signature=ab03b81b8492949ad6b0357cfcabbb1fd3990b061084f8b4b385695613a6548f&X-Amz-SignedHeaders=host&x-id=GetObject",
                  "expiry_time": "2021-12-13T14:43:29.267Z"
                }
              },
              "icon": null,
              "parent": {
                "type": "database_id",
                "database_id": "4045404e-233a-4278-84f0-b3389887b315"
              },
              "archived": false,
              "properties": {
                "published": {
                  "id": "Fpr%3E",
                  "type": "date",
                  "date": null
                }
              },
              "url": "https://www.notion.so/ac3fb543001f4be5a25e4978abd05b1d"
            }
        "#;

        #[derive(Debug, Deserialize, PartialEq)]
        struct Properties {
            published: DateProperty,
        }

        assert_eq!(
            serde_json::from_str::<Page<Properties>>(json).unwrap(),
            Page {
                object: "page".to_string(),
                id: "ac3fb543-001f-4be5-a25e-4978abd05b1d".parse().unwrap(),
                created_time: "2021-11-29T18:20:00.000Z".to_string(),
                last_edited_time: "2021-12-06T09:25:00.000Z".to_string(),
                cover: Some(File::Internal {
                    url: "https://s3.us-west-2.amazonaws.com/secure.notion-static.com/2044fff8-de9e-418b-a7c5-6f0075c0564f/entity_component_system_better.png?X-Amz-Algorithm=AWS4-HMAC-SHA256&X-Amz-Content-Sha256=UNSIGNED-PAYLOAD&X-Amz-Credential=AKIAT73L2G45EIPT3X45%2F20211213%2Fus-west-2%2Fs3%2Faws4_request&X-Amz-Date=20211213T134329Z&X-Amz-Expires=3600&X-Amz-Signature=ab03b81b8492949ad6b0357cfcabbb1fd3990b061084f8b4b385695613a6548f&X-Amz-SignedHeaders=host&x-id=GetObject".to_string(),
                    expiry_time: "2021-12-13T14:43:29.267Z".to_string(),
                }),
                icon: None,
                archived: false,
                properties: Properties {
                    published: DateProperty {
                        id: "Fpr%3E".to_string(),
                        date: None
                    }
                },
                parent: PageParent::Database {
                    id: "4045404e-233a-4278-84f0-b3389887b315".to_string()
                },
                url: "https://www.notion.so/ac3fb543001f4be5a25e4978abd05b1d".to_string(),
                children: vec![]
            }
        );

        let json = r#"
            {
              "object": "page",
              "id": "ac3fb543-001f-4be5-a25e-4978abd05b1d",
              "created_time": "2021-11-29T18:20:00.000Z",
              "last_edited_time": "2021-12-06T09:25:00.000Z",
              "cover": null,
              "icon": null,
              "parent": {
                "type": "page_id",
                "page_id": "4045404e-233a-4278-84f0-b3389887b315"
              },
              "archived": false,
              "properties": {
                "description": {
                  "id": "QPqF",
                  "type": "rich_text",
                  "rich_text": []
                }
              },
              "url": "https://www.notion.so/ac3fb543001f4be5a25e4978abd05b1d"
            }
        "#;

        #[derive(Debug, Deserialize, PartialEq)]
        struct Properties2 {
            description: RichTextProperty,
        }

        assert_eq!(
            serde_json::from_str::<Page<Properties2>>(json).unwrap(),
            Page {
                object: "page".to_string(),
                id: "ac3fb543-001f-4be5-a25e-4978abd05b1d".parse().unwrap(),
                created_time: "2021-11-29T18:20:00.000Z".to_string(),
                last_edited_time: "2021-12-06T09:25:00.000Z".to_string(),
                cover: None,
                icon: None,
                archived: false,
                properties: Properties2 {
                    description: RichTextProperty {
                        id: "QPqF".to_string(),
                        rich_text: vec![]
                    }
                },
                parent: PageParent::Page {
                    id: "4045404e-233a-4278-84f0-b3389887b315".to_string()
                },
                url: "https://www.notion.so/ac3fb543001f4be5a25e4978abd05b1d".to_string(),
                children: vec![]
            }
        );

        let json = r#"
            {
              "object": "page",
              "id": "ac3fb543-001f-4be5-a25e-4978abd05b1d",
              "created_time": "2021-11-29T18:20:00.000Z",
              "last_edited_time": "2021-12-06T09:25:00.000Z",
              "cover": null,
              "icon": null,
              "parent": {
                "type": "workspace",
                "workspace": true
              },
              "archived": false,
              "properties": {
                "description": {
                  "id": "QPqF",
                  "type": "rich_text",
                  "rich_text": [{
                    "type": "text",
                    "text": {
                      "content": "Day 1: Down the rabbit hole we go",
                      "link": null
                    },
                    "annotations": {
                      "bold": false,
                      "italic": false,
                      "strikethrough": false,
                      "underline": false,
                      "code": false,
                      "color": "default"
                    },
                    "plain_text": "Day 1: Down the rabbit hole we go",
                    "href": null
                  }]
                }
              },
              "url": "https://www.notion.so/ac3fb543001f4be5a25e4978abd05b1d"
            }
        "#;

        assert_eq!(
            serde_json::from_str::<Page<Properties2>>(json).unwrap(),
            Page {
                object: "page".to_string(),
                id: "ac3fb543-001f-4be5-a25e-4978abd05b1d".parse().unwrap(),
                created_time: "2021-11-29T18:20:00.000Z".to_string(),
                last_edited_time: "2021-12-06T09:25:00.000Z".to_string(),
                cover: None,
                icon: None,
                archived: false,
                properties: Properties2 {
                    description: RichTextProperty {
                        id: "QPqF".to_string(),
                        rich_text: vec![RichText {
                            plain_text: "Day 1: Down the rabbit hole we go".to_string(),
                            href: None,
                            annotations: Default::default(),
                            ty: RichTextType::Text {
                                content: "Day 1: Down the rabbit hole we go".to_string(),
                                link: None,
                            },
                        }]
                    }
                },
                parent: PageParent::Workspace,
                url: "https://www.notion.so/ac3fb543001f4be5a25e4978abd05b1d".to_string(),
                children: vec![]
            }
        );
    }

    #[test]
    fn test_paragraph() {
        let json = r#"
            {
              "object": "block",
              "id": "64740ca6-3a06-4694-8845-401688334ef5",
              "created_time": "2021-11-13T17:35:00.000Z",
              "last_edited_time": "2021-11-13T19:02:00.000Z",
              "has_children": false,
              "archived": false,
              "type": "paragraph",
              "paragraph": {
                "rich_text": [{
                  "type": "text",
                  "text": {
                    "content": "Cool test",
                    "link": null
                  },
                  "annotations": {
                    "bold": false,
                    "italic": false,
                    "strikethrough": false,
                    "underline": false,
                    "code": false,
                    "color": "default"
                  },
                  "plain_text": "Cool test",
                  "href": null
                }]
              }
            }
        "#;

        assert_eq!(
            serde_json::from_str::<Block>(json).unwrap(),
            Block {
                object: "block".to_string(),
                id: "64740ca63a0646948845401688334ef5".parse().unwrap(),
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
                            link: None
                        }
                    }],
                    children: vec![],
                }
            }
        )
    }

    #[test]
    fn test_headers() {
        let json = r#"
            {
              "object": "list",
              "has_more": false,
              "next_cursor": null,
              "results": [
                {
                  "object": "block",
                  "id": "8cac60c2-74b9-408c-acbd-0895cfd7b7f8",
                  "created_time": "2021-11-13T17:35:00.000Z",
                  "last_edited_time": "2021-11-13T19:02:00.000Z",
                  "has_children": false,
                  "archived": false,
                  "type": "heading_1",
                  "heading_1": {
                    "rich_text": [
                      {
                        "type": "text",
                        "text": {
                          "content": "Cool test",
                          "link": null
                        },
                        "annotations": {
                          "bold": false,
                          "italic": false,
                          "strikethrough": false,
                          "underline": false,
                          "code": false,
                          "color": "default"
                        },
                        "plain_text": "Cool test",
                        "href": null
                      }
                    ]
                  }
                },
                {
                  "object": "block",
                  "id": "8042c69c-49e7-420b-a498-39b9d61c43d0",
                  "created_time": "2021-11-13T17:35:00.000Z",
                  "last_edited_time": "2021-11-13T19:02:00.000Z",
                  "has_children": false,
                  "archived": false,
                  "type": "heading_2",
                  "heading_2": {
                    "rich_text": [
                      {
                        "type": "text",
                        "text": {
                          "content": "Cooler test",
                          "link": null
                        },
                        "annotations": {
                          "bold": false,
                          "italic": false,
                          "strikethrough": false,
                          "underline": false,
                          "code": false,
                          "color": "default"
                        },
                        "plain_text": "Cooler test",
                        "href": null
                      }
                    ]
                  }
                },
                {
                  "object": "block",
                  "id": "7f54fffa-6108-4a49-b8e9-587afe7ac08f",
                  "created_time": "2021-11-13T17:35:00.000Z",
                  "last_edited_time": "2021-11-13T19:02:00.000Z",
                  "has_children": false,
                  "archived": false,
                  "type": "heading_3",
                  "heading_3": {
                    "rich_text": [
                      {
                        "type": "text",
                        "text": {
                          "content": "Coolest test",
                          "link": null
                        },
                        "annotations": {
                          "bold": false,
                          "italic": false,
                          "strikethrough": false,
                          "underline": false,
                          "code": false,
                          "color": "default"
                        },
                        "plain_text": "Coolest test",
                        "href": null
                      }
                    ]
                  }
                }
              ]
            }
        "#;

        assert_eq!(
            serde_json::from_str::<List<Block>>(json).unwrap(),
            List {
                object: "list".to_string(),
                has_more: false,
                next_cursor: None,
                results: vec![
                    Block {
                        object: "block".to_string(),
                        id: "8cac60c274b9408cacbd0895cfd7b7f8".parse().unwrap(),
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
                                    link: None
                                }
                            }],
                        }
                    },
                    Block {
                        object: "block".to_string(),
                        id: "8042c69c49e7420ba49839b9d61c43d0".parse().unwrap(),
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
                                    link: None
                                }
                            }],
                        }
                    },
                    Block {
                        object: "block".to_string(),
                        id: "7f54fffa61084a49b8e9587afe7ac08f".parse().unwrap(),
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
                                    link: None
                                }
                            }],
                        }
                    },
                ]
            }
        )
    }

    #[test]
    fn test_callouts() {
        let json = r#"
            {
              "object": "list",
              "has_more": false,
              "next_cursor": null,
              "results": [
                {
                  "object": "block",
                  "id": "b7363fed-d7cd-4aba-a86f-f51763f4ce91",
                  "created_time": "2021-11-13T17:50:00.000Z",
                  "last_edited_time": "2021-11-13T17:50:00.000Z",
                  "has_children": false,
                  "archived": false,
                  "type": "callout",
                  "callout": {
                    "rich_text": [
                      {
                        "type": "text",
                        "text": {
                          "content": "Some really spooky callout.",
                          "link": null
                        },
                        "annotations": {
                          "bold": false,
                          "italic": false,
                          "strikethrough": false,
                          "underline": false,
                          "code": false,
                          "color": "default"
                        },
                        "plain_text": "Some really spooky callout.",
                        "href": null
                      }
                    ],
                    "icon": {
                      "type": "emoji",
                      "emoji": "💡"
                    }
                  }
                },
                {
                  "object": "block",
                  "id": "28c719a3-9845-4f08-9e87-1fe78e50e92b",
                  "created_time": "2021-11-13T17:50:00.000Z",
                  "last_edited_time": "2021-11-13T17:50:00.000Z",
                  "has_children": false,
                  "archived": false,
                  "type": "callout",
                  "callout": {
                    "rich_text": [
                      {
                        "type": "text",
                        "text": {
                          "content": "Some really spooky callout.",
                          "link": null
                        },
                        "annotations": {
                          "bold": false,
                          "italic": false,
                          "strikethrough": false,
                          "underline": false,
                          "code": false,
                          "color": "default"
                        },
                        "plain_text": "Some really spooky callout.",
                        "href": null
                      }
                    ],
                    "icon": {
                      "type": "file",
                      "file": {
                        "url": "https://example.com",
                        "expiry_time": "2021-11-13T17:50:00.000Z"
                      }
                    }
                  }
                },
                {
                  "object": "block",
                  "id": "66ea7370-1a3b-4f4e-ada5-3be2f7e6ef73",
                  "created_time": "2021-11-13T17:50:00.000Z",
                  "last_edited_time": "2021-11-13T17:50:00.000Z",
                  "has_children": false,
                  "archived": false,
                  "type": "callout",
                  "callout": {
                    "rich_text": [
                      {
                        "type": "text",
                        "text": {
                          "content": "Some really spooky callout.",
                          "link": null
                        },
                        "annotations": {
                          "bold": false,
                          "italic": false,
                          "strikethrough": false,
                          "underline": false,
                          "code": false,
                          "color": "default"
                        },
                        "plain_text": "Some really spooky callout.",
                        "href": null
                      }
                    ],
                    "icon": {
                      "type": "external",
                      "external": {
                        "url": "https://example.com"
                      }
                    }
                  }
                }
              ]
            }
        "#;

        assert_eq!(
            serde_json::from_str::<List<Block>>(json).unwrap(),
            List {
                object: "list".to_string(),
                results: vec![
                    Block {
                        object: "block".to_string(),
                        id: "b7363fedd7cd4abaa86ff51763f4ce91".parse().unwrap(),
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
                            },],
                            icon: EmojiOrFile::Emoji(Emoji {
                                emoji: "💡".to_string()
                            },),
                            children: vec![],
                        },
                    },
                    Block {
                        object: "block".to_string(),
                        id: "28c719a398454f089e871fe78e50e92b".parse().unwrap(),
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
                            },],
                            icon: EmojiOrFile::File(File::Internal {
                                url: "https://example.com".to_string(),
                                expiry_time: "2021-11-13T17:50:00.000Z".to_string(),
                            },),
                            children: vec![],
                        },
                    },
                    Block {
                        object: "block".to_string(),
                        id: "66ea73701a3b4f4eada53be2f7e6ef73".parse().unwrap(),
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
                            },],
                            icon: EmojiOrFile::File(File::External {
                                url: "https://example.com".to_string(),
                            },),
                            children: vec![],
                        },
                    },
                ],
                next_cursor: None,
                has_more: false,
            }
        )
    }

    #[test]
    fn test_quote() {
        let json = r#"
            {
              "object": "block",
              "id": "191b3d44-a37f-40c4-bb4f-3477359022fd",
              "created_time": "2021-11-13T18:58:00.000Z",
              "last_edited_time": "2021-11-13T19:00:00.000Z",
              "has_children": false,
              "archived": false,
              "type": "quote",
              "quote": {
                "rich_text": [
                  {
                    "type": "text",
                    "text": {
                      "content": "If you think you can do a thing or think you can’t do a thing, you’re right.\n—Henry Ford",
                      "link": null
                    },
                    "annotations": {
                      "bold": false,
                      "italic": false,
                      "strikethrough": false,
                      "underline": false,
                      "code": false,
                      "color": "default"
                    },
                    "plain_text": "If you think you can do a thing or think you can’t do a thing, you’re right.\n—Henry Ford",
                    "href": null
                  }
                ]
              }
            }
        "#;

        assert_eq!(
            serde_json::from_str::<Block>(json).unwrap(),
            Block {
                object: "block".to_string(),
                id: "191b3d44a37f40c4bb4f3477359022fd".parse().unwrap(),
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
            }
        )
    }

    #[test]
    fn test_numbered_and_bulleted_lists() {
        let json = r#"
            {
              "object": "block",
              "id": "844b3fdf-5688-4f6c-91e8-97b4f0e436cd",
              "created_time": "2021-11-13T19:02:00.000Z",
              "last_edited_time": "2021-11-13T19:03:00.000Z",
              "has_children": true,
              "archived": false,
              "type": "bulleted_list_item",
              "bulleted_list_item": {
                "rich_text": [
                  {
                    "type": "text",
                    "text": { "content": "This is some cool list", "link": null },
                    "annotations": {
                      "bold": false,
                      "italic": false,
                      "strikethrough": false,
                      "underline": false,
                      "code": false,
                      "color": "default"
                    },
                    "plain_text": "This is some cool list",
                    "href": null
                  }
                ],
                "children": [
                  {
                    "object": "block",
                    "id": "c3e9c471-d4b3-47dc-ab6a-6ecd4dda161a",
                    "created_time": "2021-11-13T19:02:00.000Z",
                    "last_edited_time": "2021-11-13T19:03:00.000Z",
                    "has_children": true,
                    "archived": false,
                    "type": "numbered_list_item",
                    "numbered_list_item": {
                      "rich_text": [
                        {
                          "type": "text",
                          "text": {
                            "content": "It can even contain other lists inside of it",
                            "link": null
                          },
                          "annotations": {
                            "bold": false,
                            "italic": false,
                            "strikethrough": false,
                            "underline": false,
                            "code": false,
                            "color": "default"
                          },
                          "plain_text": "It can even contain other lists inside of it",
                          "href": null
                        }
                      ],
                      "children": [
                        {
                          "object": "block",
                          "id": "55d72942-49f6-49f9-8ade-e3d049f682e5",
                          "created_time": "2021-11-13T19:03:00.000Z",
                          "last_edited_time": "2021-11-13T19:03:00.000Z",
                          "has_children": true,
                          "archived": false,
                          "type": "bulleted_list_item",
                          "bulleted_list_item": {
                            "rich_text": [
                              {
                                "type": "text",
                                "text": {
                                  "content": "And those lists can contain OTHER LISTS!",
                                  "link": null
                                },
                                "annotations": {
                                  "bold": false,
                                  "italic": false,
                                  "strikethrough": false,
                                  "underline": false,
                                  "code": false,
                                  "color": "default"
                                },
                                "plain_text": "And those lists can contain OTHER LISTS!",
                                "href": null
                              }
                            ],
                            "children": [
                              {
                                "object": "block",
                                "id": "100116e2-0a47-4903-8b79-4ac9cc3a7870",
                                "created_time": "2021-11-13T19:03:00.000Z",
                                "last_edited_time": "2021-11-13T19:03:00.000Z",
                                "has_children": false,
                                "archived": false,
                                "type": "numbered_list_item",
                                "numbered_list_item": {
                                  "rich_text": [
                                    {
                                      "type": "text",
                                      "text": { "content": "Listception", "link": null },
                                      "annotations": {
                                        "bold": false,
                                        "italic": false,
                                        "strikethrough": false,
                                        "underline": false,
                                        "code": false,
                                        "color": "default"
                                      },
                                      "plain_text": "Listception",
                                      "href": null
                                    }
                                  ],
                                  "children": []
                                }
                              },
                              {
                                "object": "block",
                                "id": "c1a5555a-8359-4999-80dc-10241d262071",
                                "created_time": "2021-11-13T19:03:00.000Z",
                                "last_edited_time": "2021-11-13T19:03:00.000Z",
                                "has_children": false,
                                "archived": false,
                                "type": "numbered_list_item",
                                "numbered_list_item": {
                                  "rich_text": [
                                    {
                                      "type": "text",
                                      "text": { "content": "Listception", "link": null },
                                      "annotations": {
                                        "bold": false,
                                        "italic": false,
                                        "strikethrough": false,
                                        "underline": false,
                                        "code": false,
                                        "color": "default"
                                      },
                                      "plain_text": "Listception",
                                      "href": null
                                    }
                                  ],
                                  "children": []
                                }
                              }
                            ]
                          }
                        }
                      ]
                    }
                  }
                ]
              }
            }
        "#;

        assert_eq!(
            serde_json::from_str::<Block>(json).unwrap(),
            Block {
                object: "block".to_string(),
                id: "844b3fdf56884f6c91e897b4f0e436cd".parse().unwrap(),
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
                    },],
                    children: vec![Block {
                        object: "block".to_string(),
                        id: "c3e9c471d4b347dcab6a6ecd4dda161a".parse().unwrap(),
                        created_time: "2021-11-13T19:02:00.000Z".to_string(),
                        last_edited_time: "2021-11-13T19:03:00.000Z".to_string(),
                        has_children: true,
                        archived: false,
                        ty: BlockType::NumberedListItem {
                            text: vec![RichText {
                                plain_text: "It can even contain other lists inside of it"
                                    .to_string(),
                                href: None,
                                annotations: Default::default(),
                                ty: RichTextType::Text {
                                    content: "It can even contain other lists inside of it"
                                        .to_string(),
                                    link: None,
                                },
                            },],
                            children: vec![Block {
                                object: "block".to_string(),
                                id: "55d7294249f649f98adee3d049f682e5".parse().unwrap(),
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
                                    },],
                                    children: vec![
                                        Block {
                                            object: "block".to_string(),
                                            id: "100116e20a4749038b794ac9cc3a7870".parse().unwrap(),
                                            created_time: "2021-11-13T19:03:00.000Z".to_string(),
                                            last_edited_time: "2021-11-13T19:03:00.000Z"
                                                .to_string(),
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
                                                },],
                                                children: vec![],
                                            },
                                        },
                                        Block {
                                            object: "block".to_string(),
                                            id: "c1a5555a8359499980dc10241d262071".parse().unwrap(),
                                            created_time: "2021-11-13T19:03:00.000Z".to_string(),
                                            last_edited_time: "2021-11-13T19:03:00.000Z"
                                                .to_string(),
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
                                                },],
                                                children: vec![],
                                            },
                                        },
                                    ],
                                },
                            },],
                        },
                    },],
                },
            }
        );
    }

    #[test]
    fn test_to_dos() {
        let json = r#"
            {
              "object": "block",
              "id": "099286a5-f878-4773-a402-98711effacf2",
              "created_time": "2021-11-13T19:01:00.000Z",
              "last_edited_time": "2021-11-13T19:01:00.000Z",
              "has_children": false,
              "archived": false,
              "type": "to_do",
              "to_do": {
                "rich_text": [
                  {
                    "type": "text",
                    "text": {
                      "content": "Checked",
                      "link": null
                    },
                    "annotations": {
                      "bold": false,
                      "italic": false,
                      "strikethrough": false,
                      "underline": false,
                      "code": false,
                      "color": "default"
                    },
                    "plain_text": "Checked",
                    "href": null
                  }
                ],
                "checked": true
              }
            }
        "#;

        assert_eq!(
            serde_json::from_str::<Block>(json).unwrap(),
            Block {
                object: "block".to_string(),
                id: "099286a5f8784773a40298711effacf2".parse().unwrap(),
                created_time: "2021-11-13T19:01:00.000Z".to_string(),
                last_edited_time: "2021-11-13T19:01:00.000Z".to_string(),
                has_children: false,
                archived: false,
                ty: BlockType::ToDo {
                    checked: true,
                    text: vec![RichText {
                        plain_text: "Checked".to_string(),
                        href: None,
                        annotations: Default::default(),
                        ty: RichTextType::Text {
                            content: "Checked".to_string(),
                            link: None,
                        },
                    },],
                    children: vec![],
                },
            }
        );
    }

    #[test]
    fn test_code() {
        let json = r#"
            {
              "object": "block",
              "id": "bf0128fd-3b85-4d85-aada-e500dcbcda35",
              "created_time": "2021-11-13T17:35:00.000Z",
              "last_edited_time": "2021-11-13T17:38:00.000Z",
              "has_children": false,
              "archived": false,
              "type": "code",
              "code": {
                "rich_text": [
                  {
                    "type": "text",
                    "text": {
                      "content": "struct Magic<T> {\n    value: T\n}\n\nfn cool() -> Magic<T> {\n    return Magic {\n        value: 100\n    };\n}",
                      "link": null
                    },
                    "annotations": {
                      "bold": false,
                      "italic": false,
                      "strikethrough": false,
                      "underline": false,
                      "code": false,
                      "color": "default"
                    },
                    "plain_text": "struct Magic<T> {\n    value: T\n}\n\nfn cool() -> Magic<T> {\n    return Magic {\n        value: 100\n    };\n}",
                    "href": null
                  }
                ],
                "language": "rust"
              }
            }
        "#;

        assert_eq!(
            serde_json::from_str::<Block>(json).unwrap(),
            Block {
                object: "block".to_string(),
                id: "bf0128fd3b854d85aadae500dcbcda35".parse().unwrap(),
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
            }
        );
    }

    #[test]
    fn test_languages() {
        assert_eq!(
            serde_json::from_str::<Language>(r#""abap""#).unwrap(),
            Language::Abap,
        );
        assert_eq!(
            serde_json::from_str::<Language>(r#""arduino""#).unwrap(),
            Language::Arduino,
        );
        assert_eq!(
            serde_json::from_str::<Language>(r#""bash""#).unwrap(),
            Language::Bash,
        );
        assert_eq!(
            serde_json::from_str::<Language>(r#""basic""#).unwrap(),
            Language::Basic,
        );
        assert_eq!(
            serde_json::from_str::<Language>(r#""c""#).unwrap(),
            Language::C,
        );
        assert_eq!(
            serde_json::from_str::<Language>(r#""clojure""#).unwrap(),
            Language::Clojure,
        );
        assert_eq!(
            serde_json::from_str::<Language>(r#""coffeescript""#).unwrap(),
            Language::CoffeeScript,
        );
        assert_eq!(
            serde_json::from_str::<Language>(r#""c++""#).unwrap(),
            Language::CPlusPlus,
        );
        assert_eq!(
            serde_json::from_str::<Language>(r#""c#""#).unwrap(),
            Language::CSharp,
        );
        assert_eq!(
            serde_json::from_str::<Language>(r#""css""#).unwrap(),
            Language::Css,
        );
        assert_eq!(
            serde_json::from_str::<Language>(r#""dart""#).unwrap(),
            Language::Dart,
        );
        assert_eq!(
            serde_json::from_str::<Language>(r#""diff""#).unwrap(),
            Language::Diff,
        );
        assert_eq!(
            serde_json::from_str::<Language>(r#""docker""#).unwrap(),
            Language::Docker,
        );
        assert_eq!(
            serde_json::from_str::<Language>(r#""elixir""#).unwrap(),
            Language::Elixir,
        );
        assert_eq!(
            serde_json::from_str::<Language>(r#""elm""#).unwrap(),
            Language::Elm,
        );
        assert_eq!(
            serde_json::from_str::<Language>(r#""erlang""#).unwrap(),
            Language::Erlang,
        );
        assert_eq!(
            serde_json::from_str::<Language>(r#""flow""#).unwrap(),
            Language::Flow,
        );
        assert_eq!(
            serde_json::from_str::<Language>(r#""fortran""#).unwrap(),
            Language::Fortran,
        );
        assert_eq!(
            serde_json::from_str::<Language>(r#""f#""#).unwrap(),
            Language::FSharp,
        );
        assert_eq!(
            serde_json::from_str::<Language>(r#""gherkin""#).unwrap(),
            Language::Gherkin,
        );
        assert_eq!(
            serde_json::from_str::<Language>(r#""glsl""#).unwrap(),
            Language::Glsl,
        );
        assert_eq!(
            serde_json::from_str::<Language>(r#""go""#).unwrap(),
            Language::Go,
        );
        assert_eq!(
            serde_json::from_str::<Language>(r#""graphql""#).unwrap(),
            Language::GraphQL,
        );
        assert_eq!(
            serde_json::from_str::<Language>(r#""groovy""#).unwrap(),
            Language::Groovy,
        );
        assert_eq!(
            serde_json::from_str::<Language>(r#""haskell""#).unwrap(),
            Language::Haskell,
        );
        assert_eq!(
            serde_json::from_str::<Language>(r#""html""#).unwrap(),
            Language::Html,
        );
        assert_eq!(
            serde_json::from_str::<Language>(r#""java""#).unwrap(),
            Language::Java,
        );
        assert_eq!(
            serde_json::from_str::<Language>(r#""javascript""#).unwrap(),
            Language::JavaScript,
        );
        assert_eq!(
            serde_json::from_str::<Language>(r#""json""#).unwrap(),
            Language::Json,
        );
        assert_eq!(
            serde_json::from_str::<Language>(r#""julia""#).unwrap(),
            Language::Julia,
        );
        assert_eq!(
            serde_json::from_str::<Language>(r#""kotlin""#).unwrap(),
            Language::Kotlin,
        );
        assert_eq!(
            serde_json::from_str::<Language>(r#""latex""#).unwrap(),
            Language::Latex,
        );
        assert_eq!(
            serde_json::from_str::<Language>(r#""less""#).unwrap(),
            Language::Less,
        );
        assert_eq!(
            serde_json::from_str::<Language>(r#""lisp""#).unwrap(),
            Language::Lisp,
        );
        assert_eq!(
            serde_json::from_str::<Language>(r#""livescript""#).unwrap(),
            Language::LiveScript,
        );
        assert_eq!(
            serde_json::from_str::<Language>(r#""lua""#).unwrap(),
            Language::Lua,
        );
        assert_eq!(
            serde_json::from_str::<Language>(r#""makefile""#).unwrap(),
            Language::Makefile,
        );
        assert_eq!(
            serde_json::from_str::<Language>(r#""markdown""#).unwrap(),
            Language::Markdown,
        );
        assert_eq!(
            serde_json::from_str::<Language>(r#""markup""#).unwrap(),
            Language::Markup,
        );
        assert_eq!(
            serde_json::from_str::<Language>(r#""matlab""#).unwrap(),
            Language::Matlab,
        );
        assert_eq!(
            serde_json::from_str::<Language>(r#""mermaid""#).unwrap(),
            Language::Mermaid,
        );
        assert_eq!(
            serde_json::from_str::<Language>(r#""nix""#).unwrap(),
            Language::Nix,
        );
        assert_eq!(
            serde_json::from_str::<Language>(r#""objective-c""#).unwrap(),
            Language::ObjectiveC,
        );
        assert_eq!(
            serde_json::from_str::<Language>(r#""ocaml""#).unwrap(),
            Language::Ocaml,
        );
        assert_eq!(
            serde_json::from_str::<Language>(r#""pascal""#).unwrap(),
            Language::Pascal,
        );
        assert_eq!(
            serde_json::from_str::<Language>(r#""perl""#).unwrap(),
            Language::Perl,
        );
        assert_eq!(
            serde_json::from_str::<Language>(r#""php""#).unwrap(),
            Language::Php,
        );
        assert_eq!(
            serde_json::from_str::<Language>(r#""plain text""#).unwrap(),
            Language::PlainText,
        );
        assert_eq!(
            serde_json::from_str::<Language>(r#""powershell""#).unwrap(),
            Language::Powershell,
        );
        assert_eq!(
            serde_json::from_str::<Language>(r#""prolog""#).unwrap(),
            Language::Prolog,
        );
        assert_eq!(
            serde_json::from_str::<Language>(r#""protobuf""#).unwrap(),
            Language::Protobuf,
        );
        assert_eq!(
            serde_json::from_str::<Language>(r#""python""#).unwrap(),
            Language::Python,
        );
        assert_eq!(
            serde_json::from_str::<Language>(r#""r""#).unwrap(),
            Language::R,
        );
        assert_eq!(
            serde_json::from_str::<Language>(r#""reason""#).unwrap(),
            Language::Reason,
        );
        assert_eq!(
            serde_json::from_str::<Language>(r#""ruby""#).unwrap(),
            Language::Ruby,
        );
        assert_eq!(
            serde_json::from_str::<Language>(r#""rust""#).unwrap(),
            Language::Rust,
        );
        assert_eq!(
            serde_json::from_str::<Language>(r#""sass""#).unwrap(),
            Language::Sass,
        );
        assert_eq!(
            serde_json::from_str::<Language>(r#""scala""#).unwrap(),
            Language::Scala,
        );
        assert_eq!(
            serde_json::from_str::<Language>(r#""scheme""#).unwrap(),
            Language::Scheme,
        );
        assert_eq!(
            serde_json::from_str::<Language>(r#""scss""#).unwrap(),
            Language::Scss,
        );
        assert_eq!(
            serde_json::from_str::<Language>(r#""shell""#).unwrap(),
            Language::Shell,
        );
        assert_eq!(
            serde_json::from_str::<Language>(r#""sql""#).unwrap(),
            Language::Sql,
        );
        assert_eq!(
            serde_json::from_str::<Language>(r#""swift""#).unwrap(),
            Language::Swift,
        );
        assert_eq!(
            serde_json::from_str::<Language>(r#""typescript""#).unwrap(),
            Language::TypeScript,
        );
        assert_eq!(
            serde_json::from_str::<Language>(r#""vb.net""#).unwrap(),
            Language::VbNet,
        );
        assert_eq!(
            serde_json::from_str::<Language>(r#""verilog""#).unwrap(),
            Language::Verilog,
        );
        assert_eq!(
            serde_json::from_str::<Language>(r#""vhdl""#).unwrap(),
            Language::Vhdl,
        );
        assert_eq!(
            serde_json::from_str::<Language>(r#""visual basic""#).unwrap(),
            Language::VisualBasic,
        );
        assert_eq!(
            serde_json::from_str::<Language>(r#""webassembly""#).unwrap(),
            Language::WebAssembly,
        );
        assert_eq!(
            serde_json::from_str::<Language>(r#""xml""#).unwrap(),
            Language::Xml,
        );
        assert_eq!(
            serde_json::from_str::<Language>(r#""yaml""#).unwrap(),
            Language::Yaml,
        );
        assert_eq!(
            serde_json::from_str::<Language>(r#""java/c/c++/c#""#).unwrap(),
            Language::CLike,
        );
    }

    #[test]
    fn test_image() {
        let json = r#"
            {
              "object": "list",
              "has_more": false,
              "next_cursor": null,
              "results": [
                {
                  "object": "block",
                  "id": "5ac94d7e-25de-4fa3-a781-0a43aac9d5c4",
                  "created_time": "2021-11-13T17:35:00.000Z",
                  "last_edited_time": "2021-11-21T13:39:00.000Z",
                  "has_children": false,
                  "archived": false,
                  "type": "image",
                  "image": {
                    "caption": [
                      {
                        "type": "text",
                        "text": {
                          "content": "Circle rendered in Bevy",
                          "link": null
                        },
                        "annotations": {
                          "bold": false,
                          "italic": false,
                          "strikethrough": false,
                          "underline": false,
                          "code": false,
                          "color": "default"
                        },
                        "plain_text": "Circle rendered in Bevy",
                        "href": null
                      }
                    ],
                    "type": "file",
                    "file": {
                      "url": "https://s3.us-west-2.amazonaws.com/secure.notion-static.com/efbb73c3-2df3-4365-bcf3-cc9ece431127/circle.png?X-Amz-Algorithm=AWS4-HMAC-SHA256&X-Amz-Content-Sha256=UNSIGNED-PAYLOAD&X-Amz-Credential=AKIAT73L2G45EIPT3X45%2F20211121%2Fus-west-2%2Fs3%2Faws4_request&X-Amz-Date=20211121T134120Z&X-Amz-Expires=3600&X-Amz-Signature=9ea689335e9054f55c794c7609f9c9c057c80484cd06eaf9dff9641d92e923c8&X-Amz-SignedHeaders=host&x-id=GetObject",
                      "expiry_time": "2021-11-21T14:41:20.026Z"
                    }
                  }
                },
                {
                  "object": "block",
                  "id": "d1e5e2c5-4351-4b8e-83a3-20ef532967a7",
                  "created_time": "2021-11-13T17:35:00.000Z",
                  "last_edited_time": "2021-11-13T17:35:00.000Z",
                  "has_children": false,
                  "archived": false,
                  "type": "image",
                  "image": {
                    "caption": [],
                    "type": "external",
                    "external": {
                      "url": "https://mathspy.me/random-file.png"
                    }
                  }
                }
              ]
            }
        "#;

        assert_eq!(
            serde_json::from_str::<List<Block>>(json).unwrap(),
            List {
                object: "list".to_string(),
                has_more: false,
                next_cursor: None,
                results: vec![
                    Block {
                        object: "block".to_string(),
                        id: "5ac94d7e25de4fa3a7810a43aac9d5c4".parse().unwrap(),
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
                                    plain_text: "Circle rendered in Bevy".to_string(),
                                    href: None,
                                    annotations: Default::default(),
                                    ty: RichTextType::Text {
                                        content: "Circle rendered in Bevy".to_string(),
                                        link: None,
                                    },
                                },
                            ],
                        },
                    },
                    Block {
                        object: "block".to_string(),
                        id: "d1e5e2c543514b8e83a320ef532967a7".parse().unwrap(),
                        created_time: "2021-11-13T17:35:00.000Z".to_string(),
                        last_edited_time: "2021-11-13T17:35:00.000Z".to_string(),
                        has_children: false,
                        archived: false,
                        ty: BlockType::Image {
                            image: File::External {
                                url: "https://mathspy.me/random-file.png".to_string(),
                            },
                            caption: vec![],
                        },
                    }
                ]
            }
        );
    }

    #[test]
    fn test_video() {
        let json = r#"
            {
              "object": "block",
              "id": "c180005a-d8de-4cd5-87ad-d47d8c2fb0f3",
              "parent": {
                "type": "page_id",
                "page_id": "72854c41-6a1d-4247-951f-de978d0422eb"
              },
              "created_time": "2022-12-06T00:21:00.000Z",
              "last_edited_time": "2022-12-06T00:24:00.000Z",
              "created_by": {
                "object": "user",
                "id": "3490e5d3-1f88-4d8b-9a83-6431ece037e0"
              },
              "last_edited_by": {
                "object": "user",
                "id": "3490e5d3-1f88-4d8b-9a83-6431ece037e0"
              },
              "has_children": false,
              "archived": false,
              "type": "video",
              "video": {
                "caption": [
                  {
                    "type": "text",
                    "text": {
                      "content": "A video of two circles, one pink and one red where the pink is moving and stopping based on user input while the red one is chasing it relentlessly at a fixed velocity",
                      "link": null
                    },
                    "annotations": {
                      "bold": false,
                      "italic": false,
                      "strikethrough": false,
                      "underline": false,
                      "code": false,
                      "color": "default"
                    },
                    "plain_text": "A video of two circles, one pink and one red where the pink is moving and stopping based on user input while the red one is chasing it relentlessly at a fixed velocity",
                    "href": null
                  }
                ],
                "type": "file",
                "file": {
                  "url": "https://s3.us-west-2.amazonaws.com/secure.notion-static.com/a8c4f962-7f7a-45cd-a2a5-ffe295afa355/moving_enemy.mp4?X-Amz-Algorithm=AWS4-HMAC-SHA256&X-Amz-Content-Sha256=UNSIGNED-PAYLOAD&X-Amz-Credential=AKIAT73L2G45EIPT3X45%2F20221206%2Fus-west-2%2Fs3%2Faws4_request&X-Amz-Date=20221206T002733Z&X-Amz-Expires=3600&X-Amz-Signature=039e735804116d7f8cad60c8719fd61ff58438f408c8cf1401a3fbd70939495b&X-Amz-SignedHeaders=host&x-id=GetObject",
                  "expiry_time": "2022-12-06T01:27:33.514Z"
                }
              }
            }
        "#;

        assert_eq!(
            serde_json::from_str::<Block>(json).unwrap(),
            Block {
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
            },
        );
    }
    #[test]
    fn test_table_of_contents() {
        let json = r#"
            {
              "object": "block",
              "id": "eb39a20e-1036-4469-b750-a9df8f4f18df",
              "created_time": "2021-11-13T17:37:00.000Z",
              "last_edited_time": "2021-11-13T17:37:00.000Z",
              "has_children": false,
              "archived": false,
              "type": "table_of_contents",
              "table_of_contents": {}
            }
        "#;

        assert_eq!(
            serde_json::from_str::<Block>(json).unwrap(),
            Block {
                object: "block".to_string(),
                id: "eb39a20e10364469b750a9df8f4f18df".parse().unwrap(),
                created_time: "2021-11-13T17:37:00.000Z".to_string(),
                last_edited_time: "2021-11-13T17:37:00.000Z".to_string(),
                has_children: false,
                archived: false,
                ty: BlockType::TableOfContents {},
            }
        );
    }

    #[test]
    fn test_divider() {
        let json = r#"
            {
              "object": "block",
              "id": "5e845049-255f-4232-96fd-6f20449be0bc",
              "created_time": "2021-11-15T21:56:00.000Z",
              "last_edited_time": "2021-11-15T21:56:00.000Z",
              "has_children": false,
              "archived": false,
              "type": "divider",
              "divider": {}
            }
        "#;

        assert_eq!(
            serde_json::from_str::<Block>(json).unwrap(),
            Block {
                object: "block".to_string(),
                id: "5e845049255f423296fd6f20449be0bc".parse().unwrap(),
                created_time: "2021-11-15T21:56:00.000Z".to_string(),
                last_edited_time: "2021-11-15T21:56:00.000Z".to_string(),
                has_children: false,
                archived: false,
                ty: BlockType::Divider {},
            }
        );
    }
}
