use anyhow::Result;
use spin_sdk::{
    http::{IntoResponse, Request, Response},
    http_component,
};
use std::collections::HashMap;
use url::form_urlencoded;

/// A simple Spin HTTP component.
#[http_component]
async fn handle_root(req: Request) -> Result<impl IntoResponse> {
    println!("Handling request to {:?}", req.header("spin-full-url"));

    let _url_query: HashMap<String, String> =
        form_urlencoded::parse(req.query().as_bytes())
            .map(|(k, v)| (k.into_owned(), v.into_owned()))
            .collect();

    Ok(Response::builder()
        .status(200)
        .header("content-type", "text/plain")
        .body("Hello World!")
        .build())
}
