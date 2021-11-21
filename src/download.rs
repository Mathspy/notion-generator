use anyhow::Result;
use itertools::Itertools;
use maud::Markup;
use std::path::PathBuf;

pub const FILES_DIR: &str = "media";

#[derive(Debug, PartialEq)]
pub struct Downloadable {
    url: String,
    path: PathBuf,
}

impl Downloadable {
    pub fn new(url: String, path: PathBuf) -> Self {
        Downloadable { url, path }
    }
}

/// A list of things that needs downloading
/// Their URL and the relative path they need to be downloaded to
pub struct Downloadables {
    pub list: Vec<Downloadable>,
}

impl Downloadables {
    pub fn new() -> Self {
        Downloadables { list: Vec::new() }
    }

    pub fn extract<'a, I>(&'a mut self, iter: I) -> impl Iterator<Item = Result<Markup>> + 'a
    where
        I: Iterator<Item = Result<(Markup, Self)>> + 'a,
    {
        iter.map_ok(|(markup, downloadables)| {
            self.list.extend(downloadables.list);
            markup
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{Downloadable, Downloadables, FILES_DIR};
    use maud::html;
    use std::path::{Path, PathBuf};

    #[test]
    fn can_extract() {
        let mut downloadables = Downloadables::new();
        let iterator = (0..10).map(|i| {
            Ok::<_, anyhow::Error>((
                html! {
                    (i)
                },
                if i % 3 == 0 {
                    let id = char::from_u32(65 + i).unwrap();
                    let mut path = Path::new(FILES_DIR).to_owned();
                    path.push(String::from(id));
                    path.set_extension("png");

                    Downloadables {
                        list: vec![Downloadable::new(
                            format!("https://gamediary.dev/{}.png", i),
                            path,
                        )],
                    }
                } else {
                    Downloadables { list: Vec::new() }
                },
            ))
        });

        downloadables.extract(iterator).for_each(|result| {
            drop(result.unwrap());
        });

        assert_eq!(
            downloadables.list,
            vec![
                Downloadable {
                    url: "https://gamediary.dev/0.png".to_string(),
                    path: PathBuf::from("media/A.png"),
                },
                Downloadable {
                    url: "https://gamediary.dev/3.png".to_string(),
                    path: PathBuf::from("media/D.png"),
                },
                Downloadable {
                    url: "https://gamediary.dev/6.png".to_string(),
                    path: PathBuf::from("media/G.png"),
                },
                Downloadable {
                    url: "https://gamediary.dev/9.png".to_string(),
                    path: PathBuf::from("media/J.png"),
                },
            ]
        );
    }
}
