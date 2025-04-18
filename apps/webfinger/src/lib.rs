use anyhow::Result;
use serde_json::json;
use spin_sdk::{
    http::{IntoResponse, Request, Response},
    http_component,
    sqlite::{Connection, Value as SqlValue},
};
use std::collections::HashMap;
use url::form_urlencoded;

/// A simple Spin HTTP component.
#[http_component]
async fn handle_root(req: Request) -> Result<impl IntoResponse> {
    let hashed_query: HashMap<String, String> =
        form_urlencoded::parse(req.query().as_bytes())
            .map(|(k, v)| (k.into_owned(), v.into_owned()))
            .collect();

    if !hashed_query.contains_key("resource") {
        return Ok(Response::builder().status(404).build());
    }

    let resource_name = hashed_query.get("resource").unwrap();

    let connection =
        Connection::open("lachuoi").expect("lachuoi db connection error");
    let execute_params = [SqlValue::Text(resource_name.to_owned())];
    let rowset = connection.execute(
        "SELECT value FROM webfinger WHERE resource = ?",
        execute_params.as_slice(),
    );

    let rows = match rowset {
        Ok(rq) => rq,
        Err(_) => {
            return Ok(Response::builder().status(400).build());
        }
    };

    let row = rows.rows().last().unwrap();
    let v = row.get::<&str>("value").unwrap();

    // println!("{:?}", v.to_string());
    //    let r = serde_json::to_string(&value).unwrap();
    Ok(Response::builder()
        .status(200)
        .header("content-type", "application/json")
        .body(v)
        .build())
}
