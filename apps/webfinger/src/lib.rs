use anyhow::Result;
use spin_sdk::{
    http::{IntoResponse, Params, Request, Response, Router},
    http_component,
};

/// A simple Spin HTTP component.
#[http_component]
async fn handle_root(req: Request) -> Result<impl IntoResponse> {
    println!("{:?}", req.query());
    println!("???");

    Ok(Response::builder()
        .status(200)
        .header("content-type", "text/plain")
        .body("webfinger")
        .build())
}
