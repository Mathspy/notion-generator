use crate::response::{Language, NotionId};
use anyhow::{bail, Context, Result};
use maud::{html, Markup, PreEscaped};
use tree_sitter_highlight::{HighlightConfiguration, Highlighter, HtmlRenderer};

const RUST_HIGHLIGHTS: &str = include_str!("./rust.scm");

const HIGHLIGHTS: [&str; 18] = [
    "attribute",
    "comment",
    "constant",
    "constant.numeric",
    "constructor",
    "keyword",
    "function",
    "function.macro",
    "label",
    "namespace",
    "operator",
    "punctuation",
    "string",
    "turbofish",
    "type.builtin",
    "type",
    "variable.builtin",
    "variable",
];

pub fn highlight(lang: &Language, code: &str, id: NotionId) -> Result<Markup> {
    let (tree_sitter_lang, highlights, code, lang_name) = match (lang, code) {
        (Language::PlainText, code) => {
            if let Some(code) = code.strip_prefix("%$NOTION-HACK$%toml\n") {
                (
                    tree_sitter_toml::language(),
                    tree_sitter_toml::HIGHLIGHT_QUERY,
                    code,
                    "toml",
                )
            } else {
                return Ok(html! {
                    pre id=(id) class="plain_text" {
                        code class="plain_text" {
                            (code)
                        }
                    }
                });
            }
        }
        (Language::Rust, code) => (tree_sitter_rust::language(), RUST_HIGHLIGHTS, code, "rust"),
        _ => bail!(
            "Unsupported language {}",
            serde_json::to_value(lang)
                .ok()
                .and_then(
                    |value| if let serde_json::Value::String(lang_name) = value {
                        Some(lang_name)
                    } else {
                        None
                    }
                )
                .context("Unsupported language with unserializable name")?
        ),
    };

    let mut config = HighlightConfiguration::new(tree_sitter_lang, highlights, "", "")
        .context("Failed to parse tree_sitter config")?;
    config.configure(&HIGHLIGHTS);
    let mut highlighter = Highlighter::new();
    let mut renderer = HtmlRenderer::new();

    let events = highlighter
        .highlight(&config, code.as_bytes(), None, |_| None)
        .unwrap();

    let classes = HIGHLIGHTS.map(|highlight| {
        format!(
            r#"class="{}""#,
            highlight
                .replace('.', " ")
                .replace("turbofish", "punctuation turbofish")
        )
        .into_bytes()
    });

    renderer
        .render(events, code.as_bytes(), &|highlight| &classes[highlight.0])
        .context("Failed to render code")?;

    Ok(html! {
        pre id=(id) class=(lang_name) {
            code class=(lang_name) {
                @for line in renderer.lines() {
                    // TreeSitter HtmlRenderer already handles escaping
                    (PreEscaped(line))
                }
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use std::{fs, path::Path};

    use super::highlight;
    use crate::response::Language;
    use pretty_assertions::assert_eq;

    #[test]
    fn plain_text() {
        assert_eq!(
            highlight(
                &Language::PlainText,
                "Hey there, lovely friend!\nI hope you have a great day!",
                "5e845049255f423296fd6f20449be0bc".parse().unwrap()
            )
            .unwrap()
            .into_string(),
            r#"<pre id="5e845049255f423296fd6f20449be0bc" class="plain_text"><code class="plain_text">Hey there, lovely friend!
I hope you have a great day!</code></pre>"#
        );
    }

    #[test]
    fn rust_highlighting() {
        let tests_dir = Path::new(file!())
            .parent()
            .unwrap()
            .join("tests/rust")
            .canonicalize()
            .unwrap();

        fs::read_dir(&tests_dir)
            .unwrap()
            .filter_map(|file| {
                let path = file.as_ref().unwrap().path();
                let extension = path.extension().unwrap().to_str().unwrap();
                if extension == "rs" {
                    Some(path)
                } else {
                    None
                }
            })
            .for_each(|path| {
                let mut html_file = path.clone();
                html_file.set_extension("html");
                let code = fs::read_to_string(tests_dir.join(&path)).unwrap();
                let html = fs::read_to_string(tests_dir.join(html_file)).unwrap();

                assert_eq!(
                    highlight(
                        &Language::Rust,
                        &code,
                        "5e845049255f423296fd6f20449be0bc".parse().unwrap()
                    )
                    .unwrap()
                    .into_string(),
                    html.trim(),
                    "generated html didn't match output for: {}",
                    path.display()
                );
            });
    }

    #[test]
    fn toml_via_hack() {
        assert_eq!(
            highlight(
                &Language::PlainText,
                r#"%$NOTION-HACK$%toml
[package]
name = "cargo"
version = "0.1.0"
edition = "2021""#,
                "5e845049255f423296fd6f20449be0bc".parse().unwrap()
            )
            .unwrap()
            .into_string(),
            r#"<pre id="5e845049255f423296fd6f20449be0bc" class="toml"><code class="toml"><span class="punctuation">[</span>package<span class="punctuation">]</span>
name <span class="operator">=</span> <span class="string">&quot;cargo&quot;</span>
version <span class="operator">=</span> <span class="string">&quot;0.1.0&quot;</span>
edition <span class="operator">=</span> <span class="string">&quot;2021&quot;</span>
</code></pre>"#
        );
    }
}
