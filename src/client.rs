use crate::response::{Block, Error, List, NotionId, Page};
use anyhow::{bail, format_err, Context, Result};
use futures_util::stream::{FuturesOrdered, TryStreamExt};
use reqwest::{Client, Method, Request};
use serde::{Deserialize, Serialize};
use std::{future::Future, ops::Not, pin::Pin};
use tower::{buffer::Buffer, limit::RateLimit, Service, ServiceExt};

pub struct NotionClient {
    svc: Buffer<RateLimit<Client>, Request>,
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
use request::RequestBuilder;

impl NotionClient {
    fn make_service(client: Client) -> Buffer<RateLimit<Client>, Request> {
        use std::time::Duration;
        use tower::Layer;

        tower::buffer::BufferLayer::new(16).layer(
            // The current Notion rate limit is 3 requests per second
            // Reference: https://developers.notion.com/reference/errors#rate-limits
            tower::limit::RateLimitLayer::new(3, Duration::new(1, 0)).layer(client),
        )
    }

    pub fn new(auth_token: String) -> Self {
        NotionClient {
            svc: Self::make_service(Client::new()),
            auth_token,
        }
    }

    pub fn with_client(client: Client, auth_token: String) -> Self {
        NotionClient {
            svc: Self::make_service(client),
            auth_token,
        }
    }

    fn build_request(&self, method: Method, url: &str) -> Result<RequestBuilder> {
        RequestBuilder::new(method, url)?
            .header("Notion-Version", "2022-06-28")?
            .bearer_auth(&self.auth_token)
    }

    async fn send_request<R>(&self, url: &str, request: RequestBuilder) -> Result<R>
    where
        R: for<'de> Deserialize<'de>,
    {
        let response = self
            .svc
            .clone()
            .ready()
            .await
            .map_err(|error| format_err!(error))?
            .call(request.build())
            .await
            .map_err(|error| format_err!(error))
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

    pub fn get_block_children<'a>(
        &'a self,
        id: NotionId,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<Block>>> + 'a>> {
        let future = async move {
            let mut cursor = None;
            let mut output = FuturesOrdered::new();

            loop {
                let url = format!("https://api.notion.com/v1/blocks/{}/children", id);
                let list = self
                    .send_request::<List<Block>>(
                        &url,
                        self.build_request(Method::GET, &url)?.query(&[
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
        };

        Box::pin(future)
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
                    self.build_request(Method::POST, &url)?
                        .json(&QueryDatabaseRequestBody {
                            start_cursor: cursor.as_deref(),
                            page_size: 100,
                        })?,
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
}
