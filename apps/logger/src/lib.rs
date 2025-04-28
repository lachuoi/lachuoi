use serde_json::Value;
use spin_sdk::{
    http::{
        IntoResponse,
        Method::{Get, Post},
        Request, Response,
    },
    http_component,
    sqlite::{Connection, Value as SqlValue},
    variables,
};
use std::{collections::hash_map::IntoKeys, str};

/*

curl "https://logger.seungjin.net/$(hostname -f | tr -d ' \n')/hourly_chores/start" \
  -H 'AUTHKEY: '${LOGGER_AUTHKEY}'' \
  -H 'Content-Type: application/json' \
  -d "{\"message\": \"Hello\"}"


*/

/// Source: https://github.com/seungjin/logger/blob/2e0fd3aef32470186990d53811e8a4d014fe7961/src/main.rs
#[http_component]
async fn handle_http(req: Request) -> anyhow::Result<impl IntoResponse> {
    let method = req.method();

    if method == &Get {
        return Ok(Response::builder()
            .status(200)
            .header("content-type", "text/plain")
            .body("Hello World!")
            .build());
    }

    if method == &Post {
        return handle_post(req).await;
    }

    Ok(Response::builder()
        .status(405)
        .header("content-type", "text/plain")
        .body("Method not allowed")
        .build())
}

async fn handle_post(req: Request) -> anyhow::Result<Response> {
    println!("{:?}", req.uri());
    println!("{:?}", req.path());

    let key = req.path();
    let val = str::from_utf8(req.body()).unwrap();

    let sender = match req.header("X-Forwarded-For") {
        Some(ip) => ip.as_str().unwrap(),
        None => match req.header("spin-client-addr") {
            Some(val) => val.as_str().unwrap(),
            None => "",
        },
    };

    // Todo: Check Auth http header
    let who = match req.header("AUTHKEY") {
        Some(k) => {
            let a = check_auth(k.as_str().unwrap().to_string()).await.unwrap();
            if a.is_none() {
                return Ok(Response::builder()
                    .status(401)
                    .header("content-type", "text/plain")
                    .body("Now Allowed")
                    .build());
            } else {
                a.unwrap()
            }
        }
        _ => {
            return Ok(Response::builder()
                .status(401)
                .header("content-type", "text/plain")
                .body("Unauthorized")
                .build());
        }
    };

    // Todo: Store into turso sql
    match record(sender.to_string(), who, key.to_string(), val.to_string())
        .await
    {
        Ok(_) => Ok(Response::builder()
            .status(200)
            .header("content-type", "text/plain")
            .body("Thank you. Come again!")
            .build()),
        Err(e) => {
            eprintln!("{:?}", e);
            Ok(Response::builder()
                .status(500)
                .header("content-type", "text/plain")
                .body("Something is wrong.")
                .build())
        }
    }
}

async fn check_auth(auth_key_string: String) -> anyhow::Result<Option<String>> {
    let auth_json = variables::get("auth_table").unwrap();
    let auth_table = json5::from_str::<Value>(&auth_json)
        .expect("auth_tabln json5 parse error");
    for auth in auth_table.as_array().unwrap() {
        if let Some(object) = auth.as_object() {
            for (key, value) in object {
                if value.as_str().unwrap() == auth_key_string {
                    return Ok(Some(key.to_string()));
                }
            }
        }
    }
    Ok(None)
}

async fn record(
    sender: String,
    who: String,
    key: String,
    val: String,
) -> anyhow::Result<()> {
    let connection =
        Connection::open("log").expect("log libsql connection error");
    let execute_params = [
        SqlValue::Text(sender),
        SqlValue::Text(who),
        SqlValue::Text(key),
        SqlValue::Text(val),
    ];
    connection.execute(
        "INSERT INTO message (sender, who, key, value) VALUES (?, ?, ?, ?)",
        execute_params.as_slice(),
    )?;
    Ok(())
}
