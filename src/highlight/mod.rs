use crate::response::{Language, NotionId};
use anyhow::{bail, Context, Result};
use maud::{html, Markup, PreEscaped};
use tree_sitter_highlight::{HighlightConfiguration, Highlighter, HtmlRenderer};

const RUST_HIGHLIGHTS: &str = include_str!("./rust.scm");

const HIGHLIGHTS: [&str; 14] = [
    "attribute",
    "comment",
    "constant",
    "function",
    "keyword",
    "label",
    "namespace",
    "operator",
    "punctuation",
    "string",
    "type.builtin",
    "type",
    "variable.builtin",
    "variable",
];

pub fn highlight(lang: &Language, code: &str, id: NotionId) -> Result<Markup> {
    // This converts the language because to serde_json::Value and since we are confident
    // that it's a Value of variant `Value::String` we call .as_str to get the content of
    // the string.
    // Attempting to use serde_json::to_string results in language names being wrapped in
    // unnecessary quotes that we have to manually remove
    let lang_name =
        serde_json::to_value(lang).context("Failed to convert Language back to string")?;
    let lang_name = lang_name
        .as_str()
        .context("Language type was not a JSON string? This should be unreachable")?;

    let (tree_sitter_lang, highlights) = match lang {
        Language::PlainText => {
            return Ok(html! {
                pre id=(id) class=(lang_name) {
                    // TODO: I might remove this code wrapper IF Notion's language support improves
                    // even TOML is currently not supported :<
                    code class=(lang_name) {
                        (code)
                    }
                }
            });
        }
        Language::Rust => (tree_sitter_rust::language(), RUST_HIGHLIGHTS),
        _ => bail!("Unsupported language {}", lang_name),
    };

    let mut config = HighlightConfiguration::new(tree_sitter_lang, highlights, "", "")
        .context("Failed to parse tree_sitter config")?;
    config.configure(&HIGHLIGHTS);
    let mut highlighter = Highlighter::new();
    let mut renderer = HtmlRenderer::new();

    let events = highlighter
        .highlight(&config, code.as_bytes(), None, |_| None)
        .unwrap();

    let classes = HIGHLIGHTS
        .map(|highlight| format!(r#"class="{}""#, highlight.replace(".", "-")).into_bytes());

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
    fn rust_type_builtin() {
        assert_eq!(
            highlight(
                &Language::Rust,
                r#"const x: &str = "abc";"#,
                "5e845049255f423296fd6f20449be0bc".parse().unwrap()
            )
            .unwrap()
            .into_string(),
            r#"<pre id="5e845049255f423296fd6f20449be0bc" class="rust"><code class="rust"><span class="keyword">const</span> <span class="variable">x</span>: <span class="operator">&amp;</span><span class="type-builtin">str</span> <span class="operator">=</span> <span class="string">&quot;abc&quot;</span><span class="punctuation">;</span>
</code></pre>"#
        );
    }
}
