use anyhow::{Context, Result};
use flurry::HashSet;
use futures_util::stream::{FuturesUnordered, TryStreamExt};
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
        tokio::fs::create_dir(output.join(FILES_DIR))
            .await
            .with_context(|| format!("Failed to create dir {}", output.display()))?;

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
                    let response = client_ref.get(&downloadable.url).send().await?;
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
    use std::{
        collections::HashSet,
        path::{Path, PathBuf},
    };

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
                    format!("https://gamediary.dev/{}.png", i),
                    path,
                ))
            } else {
                None
            }
        });

        iterator.for_each(|downloadable| {
            if let Some(downloadable) = downloadable {
                downloadables.insert(downloadable);
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
                    url: "https://gamediary.dev/0.png".to_string(),
                    path: PathBuf::from("media/A.png"),
                },
                &Downloadable {
                    url: "https://gamediary.dev/3.png".to_string(),
                    path: PathBuf::from("media/D.png"),
                },
                &Downloadable {
                    url: "https://gamediary.dev/6.png".to_string(),
                    path: PathBuf::from("media/G.png"),
                },
                &Downloadable {
                    url: "https://gamediary.dev/9.png".to_string(),
                    path: PathBuf::from("media/J.png"),
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
