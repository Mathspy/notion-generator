use anyhow::{Context, Result};
use flurry::HashSet;
use futures_util::stream::{FuturesUnordered, TryStreamExt};
use reqwest::{Client, Url};
use std::{
    hash::{Hash, Hasher},
    path::{Path, PathBuf},
};

pub const FILES_DIR: &str = "media";

#[derive(Clone, Debug, Eq)]
pub struct Downloadable {
    url: Url,
    path: String,
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
    /// Create a new downloadable
    ///
    /// Errors if path is not valid UTF-8
    pub fn new(url: Url, path: PathBuf) -> Result<Self> {
        let path = path
            .into_os_string()
            .into_string()
            .map_err(|invalid_path| {
                anyhow::anyhow!(
                    "Passed invalid downloadable path {}. Downloadable paths must be valid UTF-8",
                    invalid_path.to_string_lossy()
                )
            })?;

        Ok(Downloadable { url, path })
    }

    /// Return the path from root to the downloadable content
    pub fn src_path(&self) -> String {
        format!("/{}", self.path)
    }
}

/// A list of things that needs downloading
/// Their URL and the relative path they need to be downloaded to
pub struct Downloadables {
    pub set: HashSet<Downloadable>,
}

impl Downloadables {
    pub fn new() -> Self {
        Downloadables {
            set: HashSet::new(),
        }
    }

    pub fn insert(&self, downloadable: Downloadable) -> bool {
        let guard = self.set.guard();
        self.set.insert(downloadable, &guard)
    }

    pub async fn download_all(self, client: Client, output: &Path) -> Result<()> {
        if self.set.is_empty() {
            return Ok(());
        }

        let downloads_dir = output.join(FILES_DIR);
        tokio::fs::create_dir_all(&downloads_dir)
            .await
            .with_context(|| format!("Failed to create dir {}", downloads_dir.display()))?;

        let client_ref = &client;

        // We need this block to drop guard at the end of it to ensure it won't be
        // held across .await points
        // Guard is not Send and it will make the whole function !Send if it's held
        // across .await points
        //
        // drop(guard) doesn't currently work
        let write_operations = {
            let guard = self.set.guard();

            self.set
                .iter(&guard)
                .map(Clone::clone)
                .map(|downloadable| async move {
                    let response = client_ref.get(downloadable.url).send().await?;
                    let bytes = response.bytes().await?;
                    let destination = output.join(&downloadable.path);
                    tokio::fs::write(&destination, bytes.as_ref())
                        .await
                        .with_context(|| {
                            format!("Failed to write file {}", destination.display())
                        })?;
                    Ok(())
                })
                .collect::<FuturesUnordered<_>>()
        };

        write_operations.try_collect::<()>().await
    }
}

#[cfg(test)]
mod tests {
    use super::{Downloadable, Downloadables, FILES_DIR};
    use reqwest::Url;
    use std::{collections::HashSet, path::Path};

    #[test]
    fn can_extract() {
        let downloadables = Downloadables::new();
        let iterator = (0..10).map(|i| {
            if i % 3 == 0 {
                let id = char::from_u32(65 + i).unwrap();
                let mut path = Path::new(FILES_DIR).to_owned();
                path.push(String::from(id));
                path.set_extension("png");

                Some(Downloadable::new(
                    Url::parse(&format!("https://gamediary.dev/{}.png", i)).unwrap(),
                    path,
                ))
            } else {
                None
            }
        });

        iterator.for_each(|downloadable| {
            if let Some(downloadable) = downloadable {
                downloadables.insert(downloadable.unwrap());
            }
        });

        let guard = downloadables.set.guard();
        assert_eq!(
            downloadables
                .set
                .iter(&guard)
                .collect::<HashSet<&Downloadable>>(),
            HashSet::from([
                &Downloadable {
                    url: Url::parse("https://gamediary.dev/0.png").unwrap(),
                    path: String::from("media/A.png"),
                },
                &Downloadable {
                    url: Url::parse("https://gamediary.dev/3.png").unwrap(),
                    path: String::from("media/D.png"),
                },
                &Downloadable {
                    url: Url::parse("https://gamediary.dev/6.png").unwrap(),
                    path: String::from("media/G.png"),
                },
                &Downloadable {
                    url: Url::parse("https://gamediary.dev/9.png").unwrap(),
                    path: String::from("media/J.png"),
                },
            ])
        );
    }
}

impl Default for Downloadables {
    fn default() -> Self {
        Self::new()
    }
}
