use anyhow::Result;
use futures_util::stream::{FuturesUnordered, TryStreamExt};
use itertools::Itertools;
use maud::Markup;
use reqwest::Client;
use std::{
    hash::{Hash, Hasher},
    path::{Path, PathBuf},
};

pub const FILES_DIR: &str = "media";

#[derive(Clone, Debug, Eq)]
pub struct Downloadable {
    url: String,
    path: PathBuf,
}

impl PartialEq for Downloadable {
    fn eq(&self, other: &Self) -> bool {
        self.path.eq(&other.path)
    }
}
impl PartialOrd for Downloadable {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        self.path.partial_cmp(&other.path)
    }
}
impl Ord for Downloadable {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.path.cmp(&other.path)
    }
}
impl Hash for Downloadable {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.path.hash(state);
    }
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

    pub async fn download_all(self, client: &Client, output: &Path) -> Result<()> {
        tokio::fs::create_dir(output.join(FILES_DIR)).await?;

        let write_operations = self
            .list
            .into_iter()
            .map(|downloadable| async {
                let response = client.get(downloadable.url).send().await?;
                let bytes = response.bytes().await?;
                tokio::fs::write(output.join(downloadable.path), bytes.as_ref()).await?;
                Ok(())
            })
            .collect::<FuturesUnordered<_>>();

        write_operations.try_collect::<()>().await
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

impl Default for Downloadables {
    fn default() -> Self {
        Self::new()
    }
}
