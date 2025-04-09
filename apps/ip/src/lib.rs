use anyhow::Result;
use spin_sdk::{
    http::{IntoResponse, Params, Request, Response, Router},
    http_component,
};

/// A simple Spin HTTP component.
#[http_component]
async fn handle_root(req: Request) -> Result<impl IntoResponse> {
    whoyouare(req).await?;
    Ok(Response::builder()
        .status(200)
        .header("content-type", "text/plain")
        .body("Hello World!")
        .build())
}

async fn whoyouare(req: Request) -> Result<()> {
    let headers = req.headers();
    let method = req.method().to_string();

    for header in headers {
        println!("{}: {}", header.0, header.1.as_str().unwrap());
    }
    println!("{:?}", method);
    Ok(())
}
