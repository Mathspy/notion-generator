use crate::response::{Block, Error, List, NotionId, Page};
use anyhow::{bail, Context, Result};
use async_recursion::async_recursion;
use futures_util::stream::{FuturesOrdered, TryStreamExt};
use reqwest::{Client, Method, Request, RequestBuilder};
use serde::{Deserialize, Serialize};
use std::{future::Future, ops::Not, pin::Pin, task};
use tower::Service;

struct NotionService {
    client: Client,
}

impl Service<Request> for NotionService {
    type Response = reqwest::Response;
    type Error = reqwest::Error;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send + Sync>>;

    fn poll_ready(&mut self, _cx: &mut task::Context<'_>) -> task::Poll<Result<(), Self::Error>> {
        task::Poll::Ready(Ok(()))
    }

    fn call(&mut self, req: Request) -> Self::Future {
        Box::pin(self.client.execute(req))
    }
}
pub struct NotionClient {
    client: Client,
    auth_token: String,
}

mod request {
    use anyhow::Result;
    use reqwest::{
        header::{HeaderName, HeaderValue, InvalidHeaderName, InvalidHeaderValue},
        Body, Method, Request,
    };
    use serde::Serialize;
    use std::fmt::Display;

    pub(crate) struct RequestBuilder {
        request: Request,
    }

    impl RequestBuilder {
        pub(crate) fn new(method: Method, url: &str) -> Result<Self> {
            Ok(Self {
                request: Request::new(method, url.parse()?),
            })
        }

        pub(crate) fn header<K, V>(self, key: K, value: V) -> Result<Self>
        where
            K: TryInto<HeaderName, Error = InvalidHeaderName>,
            V: TryInto<HeaderValue, Error = InvalidHeaderValue>,
        {
            let mut request = self.request;

            let headers = request.headers_mut();
            headers.insert(key.try_into()?, value.try_into()?);

            Ok(Self { request })
        }

        pub(crate) fn bearer_auth<T>(self, token: T) -> Result<Self>
        where
            T: Display,
        {
            let mut request = self.request;

            let headers = request.headers_mut();
            headers.insert("Authorization", format!("Bearer {}", token).try_into()?);

            Ok(Self { request })
        }

        pub(crate) fn query(self, query: &[(&'static str, Option<&str>)]) -> Self {
            let mut request = self.request;

            {
                let url = request.url_mut();
                let mut current = url.query_pairs_mut();
                query.iter().for_each(|(name, value)| {
                    if let Some(value) = value {
                        current.append_pair(name, value);
                    }
                });
            }

            Self { request }
        }

        pub(crate) fn json<T>(self, json: &T) -> Result<Self>
        where
            T: Serialize,
        {
            let mut request = self.request;

            let body = request.body_mut();

            *body = Some(Body::from(serde_json::to_vec(json)?));

            Ok(Self { request })
        }

        pub(crate) fn build(self) -> Request {
            self.request
        }
    }
}

impl NotionClient {
    pub fn new(auth_token: String) -> Self {
        NotionClient {
            client: Client::new(),
            auth_token,
        }
    }

    pub fn with_client(client: Client, auth_token: String) -> Self {
        NotionClient { client, auth_token }
    }

    fn build_request(&self, method: Method, url: &str) -> RequestBuilder {
        self.client
            .request(method, url)
            .header("Notion-Version", "2021-08-16")
            .bearer_auth(&self.auth_token)
    }

    async fn send_request<R>(&self, url: &str, request: RequestBuilder) -> Result<R>
    where
        R: for<'de> Deserialize<'de>,
    {
        let response = request
            .send()
            .await
            .with_context(|| format!("Failed to get data for request {}", url))?;

        if response.status().is_success().not() {
            let error = response
                .json::<Error>()
                .await
                .with_context(|| format!("Failed to parse error for request {}", url))?;

            bail!(
                "{}: {}",
                serde_json::to_value(error.code)
                    .expect("unreachable")
                    .as_str()
                    .context(
                        "Error code should was not a JSON string? This should be unreachable"
                    )?,
                error.message
            );
        }

        let parsed = response
            .json::<R>()
            .await
            .with_context(|| format!("Failed to parse JSON for request {}", url))?;

        Ok(parsed)
    }

    #[async_recursion]
    pub async fn get_block_children(&self, id: NotionId) -> Result<Vec<Block>> {
        let mut cursor = None;
        let mut output = FuturesOrdered::new();

        loop {
            let url = format!("https://api.notion.com/v1/blocks/{}/children", id);
            let list = self
                .send_request::<List<Block>>(
                    &url,
                    self.build_request(Method::GET, &url).query(&[
                        ("page_size", Some("100")),
                        ("start_cursor", cursor.as_deref()),
                    ]),
                )
                .await?;

            let requests = list
                .results
                .into_iter()
                .map(|block| async {
                    if !block.has_children {
                        return Ok::<Block, anyhow::Error>(block);
                    }

                    let children = self.get_block_children(block.id).await?;

                    Ok(block.replace_children(children))
                })
                .collect::<Vec<_>>();

            output.extend(requests);

            if list.has_more {
                cursor = list.next_cursor;
            } else {
                return output.try_collect().await;
            }
        }
    }

    pub async fn get_database_pages<P>(&self, id: &str) -> Result<Vec<Page<P>>>
    where
        P: for<'de> Deserialize<'de>,
    {
        #[derive(Serialize)]
        struct QueryDatabaseRequestBody<'a> {
            #[serde(skip_serializing_if = "Option::is_none")]
            start_cursor: Option<&'a str>,
            page_size: u8,
        }

        let mut cursor = None;
        let mut output = FuturesOrdered::new();

        loop {
            let url = format!("https://api.notion.com/v1/databases/{}/query", id);
            let list = self
                .send_request::<List<Page<P>>>(
                    &url,
                    self.build_request(Method::POST, &url)
                        .json(&QueryDatabaseRequestBody {
                            start_cursor: cursor.as_deref(),
                            page_size: 100,
                        }),
                )
                .await?;

            let requests = list
                .results
                .into_iter()
                .map(|page| async {
                    let children = self.get_block_children(page.id).await?;

                    Ok(page.replace_children(children))
                })
                .collect::<Vec<_>>();

            output.extend(requests);

            if list.has_more {
                cursor = list.next_cursor;
            } else {
                return output.try_collect().await;
            }
        }
    }

    pub fn client(&self) -> &Client {
        &self.client
    }
}
