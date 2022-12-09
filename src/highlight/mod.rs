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
            return Ok(html! {
                pre id=(id) class="plain_text" {
                    code class="plain_text" {
                        (code)
                    }
                }
            });
        }
        (Language::Rust, code) => (tree_sitter_rust::language(), RUST_HIGHLIGHTS, code, "rust"),
        (Language::Toml, code) => (
            tree_sitter_toml::language(),
            tree_sitter_toml::HIGHLIGHT_QUERY,
            code,
            "toml",
        ),
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
    use insta::Settings;

    #[test]
    fn highlighting_tests() {
        let mut settings = Settings::new();
        settings.set_prepend_module_to_snapshot(false);
        settings.set_snapshot_path("tests");
        settings.set_omit_expression(true);

        let tests_dir = Path::new(file!())
            .parent()
            .unwrap()
            .join("tests")
            .canonicalize()
            .unwrap();

        fs::read_dir(&tests_dir)
            .unwrap()
            .flat_map(|folder| {
                let path = folder.as_ref().unwrap().path();
                let lang = path
                    .file_name()
                    .unwrap()
                    .to_os_string()
                    .to_string_lossy()
                    .into_owned();

                fs::read_dir(&path)
                    .unwrap()
                    .map(move |file| (lang.clone(), file))
            })
            .filter_map(|(lang, file)| {
                let path = file.as_ref().unwrap().path();
                let extension = path.extension().unwrap().to_str().unwrap();
                if extension != "snap" {
                    Some((lang, path))
                } else {
                    None
                }
            })
            .for_each(|(lang, path)| {
                let code = fs::read_to_string(tests_dir.join(&path)).unwrap();
                let snap_name = path.file_name().unwrap().to_str().unwrap();

                settings.set_snapshot_path(tests_dir.join(&lang));
                settings.bind(|| {
                    insta::assert_snapshot!(
                        snap_name,
                        highlight(
                            &serde_json::from_str::<Language>(&format!("\"{lang}\""))
                                .expect(&format!("unexpected language {lang}")),
                            &code,
                            "5e845049255f423296fd6f20449be0bc".parse().unwrap()
                        )
                        .unwrap()
                        .into_string()
                    )
                })
            });
    }
}
