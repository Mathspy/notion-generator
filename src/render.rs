use crate::response::{Block, BlockType, RichText, RichTextType};
use maud::{html, Escaper, Markup, Render};
use std::fmt::Write;

impl Render for Block {
    fn render(&self) -> Markup {
        match &self.ty {
            BlockType::HeadingOne { text } => {
                html! {
                    h1  {
                        (render_rich_text(text))
                    }
                }
            }
            BlockType::HeadingTwo { text } => {
                html! {
                    h2  {
                        (render_rich_text(text))
                    }
                }
            }
            BlockType::HeadingThree { text } => {
                html! {
                    h3  {
                        (render_rich_text(text))
                    }
                }
            }
            _ => {
                html! {
                    h4 style="color: red;" {
                        "UNSUPPORTED FEATURE: " (self.name())
                    }
                }
            }
        }
    }
}

fn render_rich_text(rich_text: &[RichText]) -> Markup {
    html! {
        @for segment in rich_text {
            (*segment)
        }
    }
}

impl Render for RichText {
    fn render_to(&self, buffer: &mut String) {
        match &self.ty {
            RichTextType::Text { content, link } => {
                // TODO: Handle colors
                if self.annotations.bold {
                    buffer.push_str("<strong>");
                }
                if self.annotations.italic {
                    buffer.push_str("<em>");
                }
                if self.annotations.strikethrough {
                    buffer.push_str("<del>");
                }
                if self.annotations.underline {
                    buffer.push_str(r#"<span class="underline">"#);
                }
                if self.annotations.code {
                    buffer.push_str("<code>");
                }
                if let Some(link) = link {
                    buffer.push_str("<a href=\"");

                    let mut escaped_link = String::with_capacity(link.url.len());
                    let mut escaper = Escaper::new(&mut escaped_link);
                    escaper.write_str(&link.url).expect("unreachable");
                    buffer.push_str(&escaped_link);

                    buffer.push_str("\">");
                }

                let mut escaped_content = String::with_capacity(content.len());
                let mut escape = Escaper::new(&mut escaped_content);
                escape.write_str(content).expect("unreachable");
                buffer.push_str(&escaped_content);

                if link.is_some() {
                    buffer.push_str("</a>");
                }
                if self.annotations.code {
                    buffer.push_str("</code>");
                }
                if self.annotations.underline {
                    buffer.push_str("</span>");
                }
                if self.annotations.strikethrough {
                    buffer.push_str("</del>");
                }
                if self.annotations.italic {
                    buffer.push_str("</em>");
                }
                if self.annotations.bold {
                    buffer.push_str("</strong>");
                }
            }
            RichTextType::Equation { .. } => todo!(),
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::response::{
        Annotations, Block, BlockType, Color, RichText, RichTextLink, RichTextType,
    };
    use maud::Render;
    use pretty_assertions::assert_eq;

    #[test]
    fn render_unsupported() {
        let block = Block {
            object: "block".to_string(),
            id: "eb39a20e-1036-4469-b750-a9df8f4f18df".to_string(),
            created_time: "2021-11-13T17:37:00.000Z".to_string(),
            last_edited_time: "2021-11-13T17:37:00.000Z".to_string(),
            has_children: false,
            archived: false,
            ty: BlockType::TableOfContents {},
        };

        assert_eq!(
            format!("{}", block.render().into_string()),
            r#"<h4 style="color: red;">UNSUPPORTED FEATURE: table_of_contents</h4>"#
        );
    }

    #[test]
    fn render_headings() {
        let block = Block {
            object: "block".to_string(),
            id: "8cac60c2-74b9-408c-acbd-0895cfd7b7f8".to_string(),
            created_time: "2021-11-13T17:35:00.000Z".to_string(),
            last_edited_time: "2021-11-13T19:02:00.000Z".to_string(),
            has_children: false,
            archived: false,
            ty: BlockType::HeadingOne {
                text: vec![RichText {
                    plain_text: "Cool test".to_string(),
                    href: None,
                    annotations: Annotations {
                        bold: false,
                        italic: false,
                        strikethrough: false,
                        underline: false,
                        code: false,
                        color: Color::Default,
                    },
                    ty: RichTextType::Text {
                        content: "Cool test".to_string(),
                        link: None,
                    },
                }],
            },
        };
        assert_eq!(
            format!("{}", block.render().into_string()),
            "<h1>Cool test</h1>"
        );

        let block = Block {
            object: "block".to_string(),
            id: "8042c69c-49e7-420b-a498-39b9d61c43d0".to_string(),
            created_time: "2021-11-13T17:35:00.000Z".to_string(),
            last_edited_time: "2021-11-13T19:02:00.000Z".to_string(),
            has_children: false,
            archived: false,
            ty: BlockType::HeadingTwo {
                text: vec![RichText {
                    plain_text: "Cooler test".to_string(),
                    href: None,
                    annotations: Annotations {
                        bold: false,
                        italic: false,
                        strikethrough: false,
                        underline: false,
                        code: false,
                        color: Color::Default,
                    },
                    ty: RichTextType::Text {
                        content: "Cooler test".to_string(),
                        link: None,
                    },
                }],
            },
        };
        assert_eq!(
            format!("{}", block.render().into_string()),
            "<h2>Cooler test</h2>"
        );

        let block = Block {
            object: "block".to_string(),
            id: "7f54fffa-6108-4a49-b8e9-587afe7ac08f".to_string(),
            created_time: "2021-11-13T17:35:00.000Z".to_string(),
            last_edited_time: "2021-11-13T19:02:00.000Z".to_string(),
            has_children: false,
            archived: false,
            ty: BlockType::HeadingThree {
                text: vec![RichText {
                    plain_text: "Coolest test".to_string(),
                    href: None,
                    annotations: Annotations {
                        bold: false,
                        italic: false,
                        strikethrough: false,
                        underline: false,
                        code: false,
                        color: Color::Default,
                    },
                    ty: RichTextType::Text {
                        content: "Coolest test".to_string(),
                        link: None,
                    },
                }],
            },
        };
        assert_eq!(
            format!("{}", block.render().into_string()),
            "<h3>Coolest test</h3>"
        );
    }

    #[test]
    fn display_rich_text_type_text() {
        let text = RichText {
            href: None,
            plain_text: "I love you!".to_string(),
            annotations: Annotations {
                bold: false,
                italic: false,
                strikethrough: false,
                underline: false,
                code: false,
                color: Color::Default,
            },
            ty: RichTextType::Text {
                content: "I love you!".to_string(),
                link: None,
            },
        };
        assert_eq!(text.render().into_string(), "I love you!");

        let text = RichText {
            href: None,
            plain_text: "a > 5 but < 3 how?".to_string(),
            annotations: Annotations {
                bold: false,
                italic: false,
                strikethrough: false,
                underline: false,
                code: false,
                color: Color::Default,
            },
            ty: RichTextType::Text {
                content: "a > 5 but < 3 how?".to_string(),
                link: None,
            },
        };
        assert_eq!(text.render().into_string(), "a &gt; 5 but &lt; 3 how?");

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
                link: Some(RichTextLink {
                    url: "https://cool.website/".to_string(),
                }),
            },
        };
        assert_eq!(
            text.render().into_string(),
            r#"<span class="underline"><a href="https://cool.website/">boring text</a></span>"#
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
                link: Some(RichTextLink {
                    url: "https://very.angry/><".to_string(),
                }),
            },
        };
        assert_eq!(
            text.render().into_string(),
            r#"<strong><em><del><span class="underline"><code><a href="https://very.angry/&gt;&lt;">Thanks Notion &lt;:angry_face:&gt;</a></code></span></del></em></strong>"#,
        );
    }
}
