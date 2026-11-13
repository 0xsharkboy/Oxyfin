//! HTTP transport shared by [`crate::Server`] and [`crate::Session`].

use crate::client_info::{ClientInfo, authorization_header};
use crate::{Error, Result, json};
use reqwest::header::{ACCEPT, AUTHORIZATION};
use reqwest::{Client, Method};
use serde::Serialize;
use serde::de::DeserializeOwned;
use std::time::Duration;
use url::Url;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

/// Sends requests relative to a base URL, with the Jellyfin headers.
///
/// Routes are relative and have no leading `/`. The token is passed per call and never stored here.
/// JSON responses go through [`json::decode`]; an empty body decodes as `null`, so `()` or
/// [`serde::de::IgnoredAny`] accept a `204 No Content`.
#[derive(Debug)]
pub(crate) struct Http {
    client: Client,
    base_url: Url,
    info: ClientInfo,
}

impl Http {
    /// Builds the transport; `base_url` must already be validated and end with `/`.
    pub(crate) fn new(base_url: Url, info: ClientInfo) -> Result<Self> {
        let client = Client::builder()
            .connect_timeout(CONNECT_TIMEOUT)
            .timeout(REQUEST_TIMEOUT)
            .build()
            .map_err(|source| Error::Transport {
                route: String::new(),
                source,
            })?;
        Ok(Self {
            client,
            base_url,
            info,
        })
    }

    /// Base URL of the server, always ending with `/`.
    pub(crate) fn base_url(&self) -> &Url {
        &self.base_url
    }

    /// Full `Authorization` header value, with the token when given.
    pub(crate) fn authorization(&self, token: Option<&str>) -> String {
        authorization_header(&self.info, token)
    }

    /// Absolute URL of `route` with `query` appended in order; no `?` when `query` is empty.
    pub(crate) fn url(&self, route: &str, query: &[(&str, String)]) -> Url {
        debug_assert!(!route.starts_with('/'), "route must be relative: {route}");
        let mut url = self.base_url.clone();
        if let Ok(mut segments) = url.path_segments_mut() {
            segments.pop_if_empty().extend(route.split('/'));
        }
        if !query.is_empty() {
            url.query_pairs_mut().extend_pairs(query);
        }
        url
    }

    /// `GET route`, decoding the JSON response.
    pub(crate) async fn get<T: DeserializeOwned>(
        &self,
        token: Option<&str>,
        route: &str,
        query: &[(&str, String)],
    ) -> Result<T> {
        let bytes = self
            .send(Method::GET, token, route, query, None::<&()>)
            .await?;
        decode(route, &bytes)
    }

    /// `GET route`, returning the raw response body.
    pub(crate) async fn get_bytes(
        &self,
        token: Option<&str>,
        route: &str,
        query: &[(&str, String)],
    ) -> Result<Vec<u8>> {
        self.send(Method::GET, token, route, query, None::<&()>)
            .await
    }

    /// `POST route` with `body` as JSON, decoding the JSON response.
    pub(crate) async fn post_json<B, T>(
        &self,
        token: Option<&str>,
        route: &str,
        query: &[(&str, String)],
        body: &B,
    ) -> Result<T>
    where
        B: Serialize + ?Sized,
        T: DeserializeOwned,
    {
        let bytes = self
            .send(Method::POST, token, route, query, Some(body))
            .await?;
        decode(route, &bytes)
    }

    /// `POST route` without a request body, decoding the JSON response.
    pub(crate) async fn post_empty<T: DeserializeOwned>(
        &self,
        token: Option<&str>,
        route: &str,
        query: &[(&str, String)],
    ) -> Result<T> {
        let bytes = self
            .send(Method::POST, token, route, query, None::<&()>)
            .await?;
        decode(route, &bytes)
    }

    /// `DELETE route` without a request body, decoding the JSON response.
    pub(crate) async fn delete_empty<T: DeserializeOwned>(
        &self,
        token: Option<&str>,
        route: &str,
        query: &[(&str, String)],
    ) -> Result<T> {
        let bytes = self
            .send(Method::DELETE, token, route, query, None::<&()>)
            .await?;
        decode(route, &bytes)
    }

    async fn send<B: Serialize + ?Sized>(
        &self,
        method: Method,
        token: Option<&str>,
        route: &str,
        query: &[(&str, String)],
        body: Option<&B>,
    ) -> Result<Vec<u8>> {
        // The URL can carry secrets in its query, such as a Quick Connect secret.
        let transport = |source: reqwest::Error| Error::Transport {
            route: route.to_owned(),
            source: source.without_url(),
        };
        let mut request = self
            .client
            .request(method, self.url(route, query))
            .header(ACCEPT, "application/json")
            .header(AUTHORIZATION, self.authorization(token));
        if let Some(body) = body {
            request = request.json(body);
        }
        let response = request.send().await.map_err(transport)?;
        let status = response.status();
        if !status.is_success() {
            let body = response.bytes().await.unwrap_or_default();
            return Err(Error::status(
                route,
                status.as_u16(),
                &String::from_utf8_lossy(&body),
            ));
        }
        let bytes = response.bytes().await.map_err(transport)?;
        Ok(bytes.to_vec())
    }
}

fn decode<T: DeserializeOwned>(route: &str, bytes: &[u8]) -> Result<T> {
    if bytes.iter().all(u8::is_ascii_whitespace) {
        return json::decode(route, b"null");
    }
    json::decode(route, bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};
    use wiremock::matchers::{body_json, method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn http(base: &str) -> Http {
        Http::new(
            Url::parse(base).unwrap(),
            ClientInfo::new("Test Device", "id"),
        )
        .unwrap()
    }

    #[test]
    fn url_joins_route_and_query() {
        let http = http("https://h/jellyfin/");
        assert_eq!(
            http.url("System/Info/Public", &[]).as_str(),
            "https://h/jellyfin/System/Info/Public"
        );
        let query = [
            ("userId", "u1".to_owned()),
            ("searchTerm", "a&b c".to_owned()),
        ];
        assert_eq!(
            http.url("Items/abc", &query).as_str(),
            "https://h/jellyfin/Items/abc?userId=u1&searchTerm=a%26b+c"
        );
    }

    #[test]
    fn url_on_root_base() {
        assert_eq!(
            http("http://h:8096/").url("Users/Me", &[]).as_str(),
            "http://h:8096/Users/Me"
        );
    }

    #[tokio::test]
    async fn verbs_and_bodies() {
        let mock = MockServer::start().await;
        Mock::given(method("DELETE"))
            .and(path("/UserPlayedItems/abc"))
            .and(query_param("userId", "u1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"playedPercentage": 1})))
            .expect(1)
            .mount(&mock)
            .await;
        Mock::given(method("GET"))
            .and(path("/Items/abc/Images/Primary"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(vec![0xFF, 0xD8], "image/jpeg"))
            .expect(1)
            .mount(&mock)
            .await;
        Mock::given(method("POST"))
            .and(path("/Sessions/Playing"))
            .and(body_json(json!({"ItemId": "abc"})))
            .respond_with(ResponseTemplate::new(204))
            .expect(1)
            .mount(&mock)
            .await;

        let http = http(&format!("{}/", mock.uri()));
        let value: Value = http
            .delete_empty(
                Some("t"),
                "UserPlayedItems/abc",
                &[("userId", "u1".to_owned())],
            )
            .await
            .unwrap();
        assert_eq!(value, json!({"PlayedPercentage": 1}));
        let bytes = http
            .get_bytes(Some("t"), "Items/abc/Images/Primary", &[])
            .await
            .unwrap();
        assert_eq!(bytes, [0xFF, 0xD8]);
        let () = http
            .post_json(
                Some("t"),
                "Sessions/Playing",
                &[],
                &json!({"ItemId": "abc"}),
            )
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn transport_errors_hide_the_url() {
        let port = std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let err = http(&format!("http://127.0.0.1:{port}/"))
            .get::<Value>(
                None,
                "QuickConnect/Connect",
                &[("secret", "s3cr3t".to_owned())],
            )
            .await
            .unwrap_err();
        assert!(matches!(&err, Error::Transport { route, .. } if route == "QuickConnect/Connect"));
        let shown = format!("{err} {err:?}");
        assert!(!shown.contains("s3cr3t"), "{shown}");
    }

    #[tokio::test]
    async fn empty_body_is_a_decode_error_for_structured_types() {
        let mock = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/Users/Me"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&mock)
            .await;
        let result: Result<Vec<String>> = http(&format!("{}/", mock.uri()))
            .get(Some("t"), "Users/Me", &[])
            .await;
        assert!(matches!(result, Err(Error::Decode { route, .. }) if route == "Users/Me"));
    }
}
