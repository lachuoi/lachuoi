use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use spin_sdk::{
    http::{IntoResponse, Method, Params, Request, Response},
    http_component,
    sqlite::{Connection, QueryResult, Value as SqlValue},
};
use std::collections::HashMap;
use std::{env, str};

/// A simple Spin HTTP component.
#[http_component]
async fn handle_root(req: Request) -> Result<impl IntoResponse> {
    let out = random_location().await?;
    Ok(Response::builder()
        .status(200)
        .header("content-type", "application/json")
        .body(out)
        .build())
}

async fn random_location() -> Result<String> {
    let connection =
        Connection::open("geoname").expect("geoname libsql connection error");

    let execute_params = [SqlValue::Integer(50000)];
    let rowset = connection.execute(
        "SELECT alternatenames, asciiname, country, elevation, fclass, latitude, longitude, moddate, name, population, timezone FROM cities15000 WHERE population >= ? ORDER BY RANDOM() LIMIT 1",
        execute_params.as_slice(),
    )?;

    Ok(query_result_to_json(&rowset))
}

fn query_result_to_json(query_result: &QueryResult) -> String {
    let rows_json: Vec<Value> = query_result
        .rows
        .iter()
        .map(|row| {
            let obj = query_result
                .columns
                .iter()
                .zip(&row.values)
                .map(|(col, val)| {
                    let json_val = match val {
                        SqlValue::Integer(i) => json!(i),
                        SqlValue::Real(f) => json!(f),
                        SqlValue::Text(s) => json!(s),
                        SqlValue::Blob(_) => json!(null), // Blob not supported here
                        SqlValue::Null => json!(null),
                    };
                    (col.clone(), json_val)
                })
                .collect::<serde_json::Map<_, _>>();
            Value::Object(obj)
        })
        .collect();

    let result = json!(rows_json);
    serde_json::to_string_pretty(&result).unwrap()
}
