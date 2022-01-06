use crate::response::{Language, NotionId};
use anyhow::{bail, Context, Result};
use maud::{html, Markup, PreEscaped};
use tree_sitter_highlight::{HighlightConfiguration, Highlighter, HtmlRenderer};

const RUST_HIGHLIGHTS: &str = include_str!("./rust.scm");

const HIGHLIGHTS: [&str; 17] = [
    "attribute",
    "comment",
    "constant",
    "constant.numeric",
    "constructor",
    "function",
    "function.macro",
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

    let classes = HIGHLIGHTS
        .map(|highlight| format!(r#"class="{}""#, highlight.replace(".", " ")).into_bytes());

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
            r#"<pre id="5e845049255f423296fd6f20449be0bc" class="rust"><code class="rust"><span class="keyword">const</span> <span class="variable">x</span>: <span class="operator">&amp;</span><span class="type builtin">str</span> <span class="operator">=</span> <span class="string">&quot;abc&quot;</span><span class="punctuation">;</span>
</code></pre>"#
        );
    }

    #[test]
    fn rust_attributes() {
        assert_eq!(
            highlight(
                &Language::Rust,
                r#"#[derive(Parser, Serialize)]
struct Opts {
    #[clap(short, long, default_value = "partials/head.html")]
    head: PathBuf,
}"#,
                "5e845049255f423296fd6f20449be0bc".parse().unwrap()
            )
            .unwrap()
            .into_string(),
            r#"<pre id="5e845049255f423296fd6f20449be0bc" class="rust"><code class="rust"><span class="attribute">#<span class="punctuation">[</span><span class="variable">derive</span><span class="punctuation">(</span><span class="type">Parser</span>, <span class="type">Serialize</span><span class="punctuation">)</span><span class="punctuation">]</span></span>
<span class="keyword">struct</span> <span class="type">Opts</span> <span class="punctuation">{</span>
    <span class="attribute">#<span class="punctuation">[</span><span class="variable">clap</span><span class="punctuation">(</span><span class="variable">short</span>, <span class="variable">long</span>, <span class="variable">default_value</span> <span class="operator">=</span> <span class="string">&quot;partials/head.html&quot;</span><span class="punctuation">)</span><span class="punctuation">]</span></span>
    <span class="variable">head</span>: <span class="type">PathBuf</span>,
<span class="punctuation">}</span>
</code></pre>"#
        )
    }

    #[test]
    fn rust_functions_and_macros() {
        assert_eq!(
            highlight(
                &Language::Rust,
                r#"fn rust_attributes() {
    assert!(cool_function());
    println!()
}"#,
                "5e845049255f423296fd6f20449be0bc".parse().unwrap()
            )
            .unwrap()
            .into_string(),
            r#"<pre id="5e845049255f423296fd6f20449be0bc" class="rust"><code class="rust"><span class="keyword">fn</span> <span class="function">rust_attributes</span><span class="punctuation">(</span><span class="punctuation">)</span> <span class="punctuation">{</span>
    <span class="function macro">assert</span><span class="function macro">!</span><span class="punctuation">(</span><span class="variable">cool_function</span><span class="punctuation">(</span><span class="punctuation">)</span><span class="punctuation">)</span><span class="punctuation">;</span>
    <span class="function macro">println</span><span class="function macro">!</span><span class="punctuation">(</span><span class="punctuation">)</span>
<span class="punctuation">}</span>
</code></pre>"#
        )
    }

    #[test]
    fn rust_constants() {
        assert_eq!(
            highlight(
                &Language::Rust,
                r#"const x: &str = "abc";
const y: u32 = 123;
const z: f32 = 1.0;"#,
                "5e845049255f423296fd6f20449be0bc".parse().unwrap()
            )
            .unwrap()
            .into_string(),
            r#"<pre id="5e845049255f423296fd6f20449be0bc" class="rust"><code class="rust"><span class="keyword">const</span> <span class="variable">x</span>: <span class="operator">&amp;</span><span class="type builtin">str</span> <span class="operator">=</span> <span class="string">&quot;abc&quot;</span><span class="punctuation">;</span>
<span class="keyword">const</span> <span class="variable">y</span>: <span class="type builtin">u32</span> <span class="operator">=</span> <span class="constant numeric">123</span><span class="punctuation">;</span>
<span class="keyword">const</span> <span class="variable">z</span>: <span class="type builtin">f32</span> <span class="operator">=</span> <span class="constant numeric">1.0</span><span class="punctuation">;</span>
</code></pre>"#
        )
    }

    #[test]
    fn rust_all() {
        assert_eq!(
            highlight(
                &Language::Rust,
                r#"use std::iter::{self, Map};

const COOL: &str = "abc";

#[derive(Debug)]
struct Abc {
    field: i32,
}

/// Doc comment!
enum Xyz {
    XVariant { field: u32 },
    YVariant(f32),
    ZVariant,
}

#[some_attr_macro]
fn other_fn<'a, T>(
    arg1: &'a mut T,
    arg2: String,
    arg3: &'static str,
) -> impl Iterator<Item = String>
where
    T: Debug,
{
}

// This is the main function
fn main() {
    // Statements here are executed when the compiled binary is called
    // Print text to the console
    println!("Hello World!");

    let logical: bool = true || false && true;
    let a_float: f64 = 1.0 + 2.0 * 3.0; // Regular annotation
    let mut integer = 5i32 as f32;
    let mut boolean: bool = a_float as i32 > 5;

    let (x, y, z) = ([1, 2, 3], [4, 5], [6]);

    match x {
        [1, ..] => {
            println!("{}", 1);
        }
        [2 | 3, ..] => {}
        [4, x, y] if x == y => {}
        n @ [10, ..] => {}
        _ => {}
    };

    if logical {
        for something in x {
            loop {
                break;
            }
        }
    }

    (1..10).map(|x| x * 3).collect::<Vec<_>>();

    match Xyz {
        XVariant { field } => {}
        YVariant(whatever) => {}
        ZVariant => {}
        fallback => {}
    };
}

macro_rules! print_result {
    ($expression:expr) => {
        println!("{:?} = {:?}", stringify!($expression), $expression);
    };
}

#[cfg(test)]
mod tests {
    use super::other_fn;

    #[test]
    fn welp() {}
}
"#,
                "5e845049255f423296fd6f20449be0bc".parse().unwrap(),
            )
            .unwrap()
            .into_string(),
            r#"<pre id="5e845049255f423296fd6f20449be0bc" class="rust"><code class="rust"><span class="keyword">use</span> <span class="namespace"><span class="variable">std</span><span class="punctuation">::</span><span class="variable">iter</span></span><span class="punctuation">::</span><span class="punctuation">{</span><span class="variable builtin">self</span>, <span class="type">Map</span><span class="punctuation">}</span><span class="punctuation">;</span>

<span class="keyword">const</span> <span class="constant">COOL</span>: <span class="operator">&amp;</span><span class="type builtin">str</span> <span class="operator">=</span> <span class="string">&quot;abc&quot;</span><span class="punctuation">;</span>

<span class="attribute">#<span class="punctuation">[</span><span class="variable">derive</span><span class="punctuation">(</span><span class="type">Debug</span><span class="punctuation">)</span><span class="punctuation">]</span></span>
<span class="keyword">struct</span> <span class="type">Abc</span> <span class="punctuation">{</span>
    <span class="variable">field</span>: <span class="type builtin">i32</span>,
<span class="punctuation">}</span>

<span class="comment">/// Doc comment!</span>
<span class="keyword">enum</span> <span class="type">Xyz</span> <span class="punctuation">{</span>
    <span class="type">XVariant</span> <span class="punctuation">{</span> <span class="variable">field</span>: <span class="type builtin">u32</span> <span class="punctuation">}</span>,
    <span class="type">YVariant</span><span class="punctuation">(</span><span class="type builtin">f32</span><span class="punctuation">)</span>,
    <span class="type">ZVariant</span>,
<span class="punctuation">}</span>

<span class="attribute">#<span class="punctuation">[</span><span class="variable">some_attr_macro</span><span class="punctuation">]</span></span>
<span class="keyword">fn</span> <span class="function">other_fn</span><span class="punctuation">&lt;</span><span class="label"><span class="operator">&#39;</span><span class="variable">a</span></span>, <span class="type">T</span><span class="punctuation">&gt;</span><span class="punctuation">(</span>
    <span class="variable">arg1</span>: <span class="operator">&amp;</span><span class="label"><span class="operator">&#39;</span><span class="variable">a</span></span> <span class="keyword">mut</span> <span class="type">T</span>,
    <span class="variable">arg2</span>: <span class="type">String</span>,
    <span class="variable">arg3</span>: <span class="operator">&amp;</span><span class="label"><span class="operator">&#39;</span><span class="variable">static</span></span> <span class="type builtin">str</span>,
<span class="punctuation">)</span> <span class="operator">-&gt;</span> <span class="keyword">impl</span> <span class="type">Iterator</span><span class="punctuation">&lt;</span><span class="type">Item</span> <span class="operator">=</span> <span class="type">String</span><span class="punctuation">&gt;</span>
<span class="keyword">where</span>
    <span class="type">T</span>: <span class="type">Debug</span>,
<span class="punctuation">{</span>
<span class="punctuation">}</span>

<span class="comment">// This is the main function</span>
<span class="keyword">fn</span> <span class="function">main</span><span class="punctuation">(</span><span class="punctuation">)</span> <span class="punctuation">{</span>
    <span class="comment">// Statements here are executed when the compiled binary is called</span>
    <span class="comment">// Print text to the console</span>
    <span class="function macro">println</span><span class="function macro">!</span><span class="punctuation">(</span><span class="string">&quot;Hello World!&quot;</span><span class="punctuation">)</span><span class="punctuation">;</span>

    <span class="keyword">let</span> <span class="variable">logical</span>: <span class="type builtin">bool</span> <span class="operator">=</span> <span class="constant">true</span> <span class="operator">||</span> <span class="constant">false</span> <span class="operator">&amp;&amp;</span> <span class="constant">true</span><span class="punctuation">;</span>
    <span class="keyword">let</span> <span class="variable">a_float</span>: <span class="type builtin">f64</span> <span class="operator">=</span> <span class="constant numeric">1.0</span> <span class="operator">+</span> <span class="constant numeric">2.0</span> <span class="operator">*</span> <span class="constant numeric">3.0</span><span class="punctuation">;</span> <span class="comment">// Regular annotation</span>
    <span class="keyword">let</span> <span class="keyword">mut</span> <span class="variable">integer</span> <span class="operator">=</span> <span class="constant numeric">5i32</span> <span class="keyword">as</span> <span class="type builtin">f32</span><span class="punctuation">;</span>
    <span class="keyword">let</span> <span class="keyword">mut</span> <span class="variable">boolean</span>: <span class="type builtin">bool</span> <span class="operator">=</span> <span class="variable">a_float</span> <span class="keyword">as</span> <span class="type builtin">i32</span> <span class="operator">&gt;</span> <span class="constant numeric">5</span><span class="punctuation">;</span>

    <span class="keyword">let</span> <span class="punctuation">(</span><span class="variable">x</span>, <span class="variable">y</span>, <span class="variable">z</span><span class="punctuation">)</span> <span class="operator">=</span> <span class="punctuation">(</span><span class="punctuation">[</span><span class="constant numeric">1</span>, <span class="constant numeric">2</span>, <span class="constant numeric">3</span><span class="punctuation">]</span>, <span class="punctuation">[</span><span class="constant numeric">4</span>, <span class="constant numeric">5</span><span class="punctuation">]</span>, <span class="punctuation">[</span><span class="constant numeric">6</span><span class="punctuation">]</span><span class="punctuation">)</span><span class="punctuation">;</span>

    <span class="keyword">match</span> <span class="variable">x</span> <span class="punctuation">{</span>
        <span class="punctuation">[</span><span class="constant numeric">1</span>, <span class="operator">..</span><span class="punctuation">]</span> <span class="operator">=&gt;</span> <span class="punctuation">{</span>
            <span class="function macro">println</span><span class="function macro">!</span><span class="punctuation">(</span><span class="string">&quot;{}&quot;</span>, <span class="constant numeric">1</span><span class="punctuation">)</span><span class="punctuation">;</span>
        <span class="punctuation">}</span>
        <span class="punctuation">[</span><span class="constant numeric">2</span> <span class="operator">|</span> <span class="constant numeric">3</span>, <span class="operator">..</span><span class="punctuation">]</span> <span class="operator">=&gt;</span> <span class="punctuation">{</span><span class="punctuation">}</span>
        <span class="punctuation">[</span><span class="constant numeric">4</span>, <span class="variable">x</span>, <span class="variable">y</span><span class="punctuation">]</span> <span class="keyword">if</span> <span class="variable">x</span> <span class="operator">==</span> <span class="variable">y</span> <span class="operator">=&gt;</span> <span class="punctuation">{</span><span class="punctuation">}</span>
        <span class="variable">n</span> <span class="operator">@</span> <span class="punctuation">[</span><span class="constant numeric">10</span>, <span class="operator">..</span><span class="punctuation">]</span> <span class="operator">=&gt;</span> <span class="punctuation">{</span><span class="punctuation">}</span>
        _ <span class="operator">=&gt;</span> <span class="punctuation">{</span><span class="punctuation">}</span>
    <span class="punctuation">}</span><span class="punctuation">;</span>

    <span class="keyword">if</span> <span class="variable">logical</span> <span class="punctuation">{</span>
        <span class="keyword">for</span> <span class="variable">something</span> <span class="keyword">in</span> <span class="variable">x</span> <span class="punctuation">{</span>
            <span class="keyword">loop</span> <span class="punctuation">{</span>
                <span class="keyword">break</span><span class="punctuation">;</span>
            <span class="punctuation">}</span>
        <span class="punctuation">}</span>
    <span class="punctuation">}</span>

    <span class="punctuation">(</span><span class="constant numeric">1</span><span class="operator">..</span><span class="constant numeric">10</span><span class="punctuation">)</span><span class="punctuation">.</span><span class="function">map</span><span class="punctuation">(</span><span class="operator">|</span><span class="variable">x</span><span class="operator">|</span> <span class="variable">x</span> <span class="operator">*</span> <span class="constant numeric">3</span><span class="punctuation">)</span><span class="punctuation">.</span><span class="function">collect</span><span class="punctuation">::</span><span class="punctuation">&lt;</span><span class="type">Vec</span><span class="punctuation">&lt;</span><span class="type">_</span><span class="punctuation">&gt;</span><span class="punctuation">&gt;</span><span class="punctuation">(</span><span class="punctuation">)</span><span class="punctuation">;</span>

    <span class="keyword">match</span> <span class="type">Xyz</span> <span class="punctuation">{</span>
        <span class="constructor">XVariant</span> <span class="punctuation">{</span> <span class="variable">field</span> <span class="punctuation">}</span> <span class="operator">=&gt;</span> <span class="punctuation">{</span><span class="punctuation">}</span>
        <span class="constructor">YVariant</span><span class="punctuation">(</span><span class="variable">whatever</span><span class="punctuation">)</span> <span class="operator">=&gt;</span> <span class="punctuation">{</span><span class="punctuation">}</span>
        <span class="type">ZVariant</span> <span class="operator">=&gt;</span> <span class="punctuation">{</span><span class="punctuation">}</span>
        <span class="variable">fallback</span> <span class="operator">=&gt;</span> <span class="punctuation">{</span><span class="punctuation">}</span>
    <span class="punctuation">}</span><span class="punctuation">;</span>
<span class="punctuation">}</span>

<span class="keyword">macro_rules!</span> <span class="function macro">print_result</span> <span class="punctuation">{</span>
    <span class="punctuation">(</span><span class="variable">$expression</span>:<span class="variable">expr</span><span class="punctuation">)</span> <span class="operator">=&gt;</span> <span class="punctuation">{</span>
        <span class="variable">println</span>!<span class="punctuation">(</span><span class="string">&quot;{:?} = {:?}&quot;</span>, <span class="variable">stringify</span>!<span class="punctuation">(</span><span class="variable">$expression</span><span class="punctuation">)</span>, <span class="variable">$expression</span><span class="punctuation">)</span>;
    <span class="punctuation">}</span><span class="punctuation">;</span>
<span class="punctuation">}</span>

<span class="attribute">#<span class="punctuation">[</span><span class="variable">cfg</span><span class="punctuation">(</span><span class="variable">test</span><span class="punctuation">)</span><span class="punctuation">]</span></span>
<span class="keyword">mod</span> <span class="namespace">tests</span> <span class="punctuation">{</span>
    <span class="keyword">use</span> <span class="keyword"><span class="namespace">super<span class="punctuation">::</span><span class="variable">other_fn</span></span></span><span class="punctuation">;</span>

    <span class="attribute">#<span class="punctuation">[</span><span class="variable">test</span><span class="punctuation">]</span></span>
    <span class="keyword">fn</span> <span class="function">welp</span><span class="punctuation">(</span><span class="punctuation">)</span> <span class="punctuation">{</span><span class="punctuation">}</span>
<span class="punctuation">}</span>
</code></pre>"#
        );
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
