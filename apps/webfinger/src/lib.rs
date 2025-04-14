use anyhow::Result;
use serde_json::json;
use spin_sdk::{
    http::{IntoResponse, Params, Request, Response, Router},
    http_component,
};

/// A simple Spin HTTP component.
#[http_component]
async fn handle_root(req: Request) -> Result<impl IntoResponse> {
    println!("{:?}", req.query());

    let response = json!(
        {
            "subject":"acct:seungjin@mstd.seungjin.net",
            "aliases":["https://mstd.seungjin.net/@seungjin","https://mstd.seungjin.net/users/seungjin"],
            "links":[
                {"rel":"http://webfinger.net/rel/profile-page","type":"text/html","href":"https://mstd.seungjin.net/@seungjin"},
                {"rel":"self","type":"application/activity+json","href":"https://mstd.seungjin.net/users/seungjin"},
                {"rel":"http://ostatus.org/schema/1.0/subscribe","template":"https://mstd.seungjin.net/authorize_interaction?uri={uri}"},
                {"rel":"http://webfinger.net/rel/avatar","type":"image/jpeg","href":"https://media-mstd.seungjin.net/accounts/avatars/109/737/937/659/013/254/original/626c9187e341632b.jpg"}
            ]
        }
    );

    Ok(Response::builder()
        .status(200)
        .header("content-type", "application/json")
        .body(serde_json::to_string(&response).unwrap())
        .build())
}
