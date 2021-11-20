use crate::highlight::highlight;
use crate::response::{Block, BlockType, RichText, RichTextType};
use anyhow::{Context, Result};
use maud::{html, Escaper, Markup, Render};
use std::fmt::Write;

fn render_block(block: &Block, class: Option<&str>) -> Result<Markup> {
    match &block.ty {
        BlockType::HeadingOne { text } => Ok(html! {
            h1 class=[class] {
                (render_rich_text(text))
            }
        }),
        BlockType::HeadingTwo { text } => Ok(html! {
            h2 class=[class] {
                (render_rich_text(text))
            }
        }),
        BlockType::HeadingThree { text } => Ok(html! {
            h3 class=[class] {
                (render_rich_text(text))
            }
        }),
        BlockType::Paragraph { text, children } => {
            if children.is_empty() {
                Ok(html! {
                    p class=[class] {
                        (render_rich_text(text))
                    }
                })
            } else {
                eprintln!("WARNING: Rendering a paragraph with children doesn't make sense as far as I am aware at least for the English language.\nThe HTML spec is strictly against it (rendering a <p> inside of a <p> is forbidden) but it's part of Notion's spec so we support it but emit this warning.\n\nRendering a paragraph with children doesn't give any indication to accessibility tools that anything about the children of this paragraph are special so it causes accessibility information loss.\n\nIf you have an actual use case for paragraphs inside of paragraphs please open an issue, I would love to be convinced of reasons to remove this warning or of good HTML ways to render paragraphs inside of paragraphs!");

                Ok(html! {
                    div class=[class] {
                        p {
                            (render_rich_text(text))
                        }
                        @for child in children {
                            (render_block(child, Some("indent"))?)
                        }
                    }
                })
            }
        }
        BlockType::Quote { text, children } => Ok(html! {
            blockquote {
                (render_rich_text(text))
                @for child in children {
                    (render_block(child, Some("indent"))?)
                }
            }
        }),
        BlockType::Code { language, text } => highlight(
            language,
            &text
                .get(0)
                .context("Code block's RichText is empty")?
                .plain_text,
        ),
        _ => Ok(html! {
            h4 style="color: red;" class=[class] {
                "UNSUPPORTED FEATURE: " (block.name())
            }
        }),
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
        }
    }
}

#[cfg(test)]
mod tests {
    use super::render_block;
    use crate::response::{
        Annotations, Block, BlockType, Color, Language, RichText, RichTextLink, RichTextType,
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
            format!("{}", render_block(&block, None).unwrap().into_string()),
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
            format!("{}", render_block(&block, None).unwrap().into_string()),
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
            format!("{}", render_block(&block, None).unwrap().into_string()),
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
            format!("{}", render_block(&block, None).unwrap().into_string()),
            "<h3>Coolest test</h3>"
        );
    }

    #[test]
    fn render_paragraphs() {
        let block = Block {
            object: "block".to_string(),
            id: "64740ca6-3a06-4694-8845-401688334ef5".to_string(),
            created_time: "2021-11-13T17:35:00.000Z".to_string(),
            last_edited_time: "2021-11-13T19:02:00.000Z".to_string(),
            has_children: false,
            archived: false,
            ty: BlockType::Paragraph {
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
                children: vec![],
            },
        };
        assert_eq!(
            format!("{}", render_block(&block, None).unwrap().into_string()),
            "<p>Cool test</p>"
        );

        let block = Block {
            object: "block".to_string(),
            id: "4f2efd79-ae9a-4684-827c-6b69743d6c5d".to_string(),
            created_time: "2021-11-13T17:35:00.000Z".to_string(),
            last_edited_time: "2021-11-16T11:23:00.000Z".to_string(),
            has_children: true,
            archived: false,
            ty: BlockType::Paragraph {
                text: vec![
                    RichText {
                        plain_text: "Or you can just leave an empty line in between if you want it to leave extra breathing room.".to_string(),
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
                            content: "Or you can just leave an empty line in between if you want it to leave extra breathing room.".to_string(),
                            link: None,
                        },
                    },
                ],
                children: vec![
                    Block {
                        object: "block".to_string(),
                        id: "4fb9dd79-2fc7-45b1-b3a2-8efae49992ed".to_string(),
                        created_time: "2021-11-15T18:03:00.000Z".to_string(),
                        last_edited_time: "2021-11-16T11:23:00.000Z".to_string(),
                        has_children: true,
                        archived: false,
                        ty: BlockType::Paragraph {
                            text: vec![
                                RichText {
                                    plain_text: "You can also create these rather interesting nested paragraphs".to_string(),
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
                                        content: "You can also create these rather interesting nested paragraphs".to_string(),
                                        link: None,
                                    },
                                },
                            ],
                            children: vec![
                                Block {
                                    object: "block".to_string(),
                                    id: "817c0ca1-721a-4565-ac54-eedbbe471f0b".to_string(),
                                    created_time: "2021-11-16T11:23:00.000Z".to_string(),
                                    last_edited_time: "2021-11-16T11:23:00.000Z".to_string(),
                                    has_children: false,
                                    archived: false,
                                    ty: BlockType::Paragraph {
                                        text: vec![
                                            RichText {
                                                plain_text: "Possibly more than once too!".to_string(),
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

        assert_eq!(
            format!("{}", render_block(&block, None).unwrap().into_string()),
            r#"<div><p>Or you can just leave an empty line in between if you want it to leave extra breathing room.</p><div class="indent"><p>You can also create these rather interesting nested paragraphs</p><p class="indent">Possibly more than once too!</p></div></div>"#
        );
    }

    #[test]
    fn render_quote() {
        let block = Block {
            object: "block".to_string(),
            id: "191b3d44-a37f-40c4-bb4f-3477359022fd".to_string(),
            created_time: "2021-11-13T18:58:00.000Z".to_string(),
            last_edited_time: "2021-11-13T19:00:00.000Z".to_string(),
            has_children: false,
            archived: false,
            ty: BlockType::Quote {
                text: vec![
                    RichText {
                        plain_text: "If you think you can do a thing or think you can’t do a thing, you’re right.\n—Henry Ford".to_string(),
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
                            content: "If you think you can do a thing or think you can’t do a thing, you’re right.\n—Henry Ford".to_string(),
                            link: None,
                        },
                    },
                ],
                children: vec![],
            },
        };

        assert_eq!(
            format!("{}", render_block(&block, None).unwrap().into_string()),
            "<blockquote>If you think you can do a thing or think you can’t do a thing, you’re right.\n—Henry Ford</blockquote>"
        );
    }

    #[test]
    fn render_code() {
        let block = Block {
            object: "block".to_string(),
            id: "bf0128fd-3b85-4d85-aada-e500dcbcda35".to_string(),
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
                        annotations: Annotations {
                            bold: false,
                            italic: false,
                            strikethrough: false,
                            underline: false,
                            code: false,
                            color: Color::Default,
                        },
                        ty: RichTextType::Text {
                            content: "struct Magic<T> {\n    value: T\n}\n\nfn cool() -> Magic<T> {\n    return Magic {\n        value: 100\n    };\n}".to_string(),
                            link: None,
                        },
                    },
                ],
            },
        };

        assert_eq!(
            format!("{}", render_block(&block, None).unwrap().into_string()),
            r#"<pre class="rust"><code class="rust">"#.to_string()
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
                + r#"        <span class="variable">value</span>: <span class="constant">100</span>"#
                + "\n"
                + r#"    <span class="punctuation">}</span><span class="punctuation">;</span>"#
                + "\n"
                + r#"<span class="punctuation">}</span>"#
                + "\n"
                + r#"</code></pre>"#
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

    #[test]
    fn display_rich_text_type_equation() {
        let text = RichText {
            href: None,
            plain_text: "f(x)=y".to_string(),
            annotations: Annotations {
                bold: false,
                italic: false,
                strikethrough: false,
                underline: false,
                code: false,
                color: Color::Default,
            },
            ty: RichTextType::Equation {
                expression: "f(x)=y".to_string(),
            },
        };
        assert_eq!(
            text.render().into_string(),
            r#"<span class="katex"><span class="katex-mathml"><math xmlns="http://www.w3.org/1998/Math/MathML"><semantics><mrow><mi>f</mi><mo stretchy="false">(</mo><mi>x</mi><mo stretchy="false">)</mo><mo>=</mo><mi>y</mi></mrow><annotation encoding="application/x-tex">f(x)=y</annotation></semantics></math></span><span class="katex-html" aria-hidden="true"><span class="base"><span class="strut" style="height:1em;vertical-align:-0.25em;"></span><span class="mord mathnormal" style="margin-right:0.10764em;">f</span><span class="mopen">(</span><span class="mord mathnormal">x</span><span class="mclose">)</span><span class="mspace" style="margin-right:0.2778em;"></span><span class="mrel">=</span><span class="mspace" style="margin-right:0.2778em;"></span></span><span class="base"><span class="strut" style="height:0.625em;vertical-align:-0.1944em;"></span><span class="mord mathnormal" style="margin-right:0.03588em;">y</span></span></span></span>"#
        )
    }
}
